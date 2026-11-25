//! M1 end-to-end checks against real sources and a real llama-server.
//!
//! cargo run --example m1_check -- detect
//! cargo run --example m1_check -- download <dir>      pause/resume, failover, bad sources, checksum
//! cargo run --example m1_check -- infer <models_dir>  GPU failure fallback, crash restart, cancel

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use llm_translate_lib::app::settings::Backend;
use llm_translate_lib::deploy::downloader::{self, Control, DownloadError, Progress, StopReason};
use llm_translate_lib::deploy::hardware;
use llm_translate_lib::deploy::manifest::{Manifest, Source};
use llm_translate_lib::inference::supervisor::{LaunchSpec, Status, Supervisor};
use llm_translate_lib::translate::scheduler::{TranslateEvent, TranslateRequest, Translator};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn manifest() -> Manifest {
    Manifest::load(&root().join("resources/models.json")).unwrap()
}

fn llama() -> PathBuf {
    root().join("resources/llama/llama-server.exe")
}

fn quiet() -> downloader::ProgressFn {
    Arc::new(|p| match p {
        Progress::Probe { source, speed_bps, error } => {
            println!("    probe {source}: {:?} {:?}", speed_bps.map(|s| format!("{:.1} MB/s", s / 1e6)), error)
        }
        Progress::Retry { source, message, wait_secs } => println!("    retry {source}: {message} (wait {wait_secs}s)"),
        _ => {}
    })
}

async fn detect() {
    let hw = hardware::detect(&llama()).await;
    println!("{hw:#?}");
    println!("{:#?}", hardware::recommend(&hw));
}

async fn download(dir: &Path) {
    let client = downloader::client();
    let m = manifest();
    let mut model = m.get("index-translate-2b-q4").unwrap().clone();
    let _ = std::fs::remove_file(model.path_in(dir));
    let _ = std::fs::remove_file(downloader::part_path(&model.path_in(dir)));

    println!("[1] unreachable sources -> NoSource");
    let mut bad = model.clone();
    bad.sources = vec![
        Source { name: "dead1".into(), url: "https://127.0.0.1:9/x.gguf".into() },
        Source { name: "dead2".into(), url: "https://nonexistent.invalid/x.gguf".into() },
    ];
    let t0 = Instant::now();
    let r = downloader::download(&client, &bad, dir, &Control::default(), quiet()).await;
    println!("  -> {r:?} in {:.0}s", t0.elapsed().as_secs_f64());
    assert!(matches!(r, Err(DownloadError::NoSource(_))));

    println!("[2] pause after ~150 MB, then resume with a dead source first (failover)");
    let ctl = Control::default();
    let c2 = ctl.clone();
    let progress: downloader::ProgressFn = Arc::new(move |p| {
        if let Progress::Download { done, source, speed_bps, .. } = p {
            if done > 150_000_000 {
                println!("    pausing at {done} ({source}, {:.1} MB/s)", speed_bps / 1e6);
                c2.stop(StopReason::Pause);
            }
        }
    });
    let r = downloader::download(&client, &model, dir, &ctl, progress).await;
    let part = downloader::part_path(&model.path_in(dir));
    let paused_at = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    println!("  -> {r:?}, .part = {paused_at}");
    assert!(matches!(r, Err(DownloadError::Paused)) && paused_at > 100_000_000);

    model.sources.insert(0, Source { name: "dead".into(), url: "https://127.0.0.1:9/x.gguf".into() });
    let first_resume = Arc::new(std::sync::Mutex::new(None));
    let fr = first_resume.clone();
    let progress: downloader::ProgressFn = Arc::new(move |p| {
        if let Progress::Download { done, source, .. } = &p {
            fr.lock().unwrap().get_or_insert((*done, source.clone()));
        }
        if let Progress::Verify { done, total } = p {
            if done == total {
                println!("    verified {total}");
            }
        }
    });
    let t0 = Instant::now();
    let r = downloader::download(&client, &model, dir, &Control::default(), progress).await;
    println!("  -> {r:?} in {:.0}s, first progress after resume = {:?}", t0.elapsed().as_secs_f64(), first_resume.lock().unwrap());
    assert!(r.is_ok());
    assert!(first_resume.lock().unwrap().as_ref().unwrap().0 > paused_at, "resume did not continue from .part");

    println!("[3] already present -> verify only");
    let t0 = Instant::now();
    let r = downloader::download(&client, &model, dir, &Control::default(), quiet()).await;
    println!("  -> {r:?} in {:.1}s", t0.elapsed().as_secs_f64());
    assert!(r.is_ok());

    println!("[4] checksum mismatch -> retried once, then Checksum error (small file)");
    let mut tiny = model.clone();
    tiny.id = "tiny".into();
    tiny.file = "README-test.md".into();
    tiny.size = 6723;
    tiny.sha256 = "0".repeat(64);
    tiny.sources = vec![Source {
        name: "modelscope".into(),
        url: "https://www.modelscope.cn/models/IndexTeam/Index-Translate-2B-GGUF/resolve/master/README.md".into(),
    }];
    let r = downloader::download(&client, &tiny, dir, &Control::default(), quiet()).await;
    println!("  -> {r:?}");
    assert!(matches!(r, Err(DownloadError::Checksum(_))));
    assert!(!downloader::part_path(&tiny.path_in(dir)).exists());
    tiny.sha256 = "b7366cd52681819e5493803fbb40cff66728abf58b63b9e980fbb314a46460c0".into();
    let r = downloader::download(&client, &tiny, dir, &Control::default(), quiet()).await;
    println!("  -> with correct sha: {r:?}");
    assert!(r.is_ok());
    let _ = std::fs::remove_file(tiny.path_in(dir));
    println!("download checks passed");
}

async fn wait_status(sup: &Supervisor, f: impl Fn(&Status) -> bool, secs: u64) -> Status {
    let mut rx = sup.subscribe();
    let r = tokio::time::timeout(Duration::from_secs(secs), rx.wait_for(|s| f(s))).await;
    match r {
        Ok(Ok(s)) => s.clone(),
        _ => sup.status(),
    }
}

/// Collect a whole translation (cancels any earlier request).
async fn translate_text(tr: &Translator, text: &str) -> Result<String, String> {
    let out = std::sync::Mutex::new(String::new());
    let err = std::sync::Mutex::new(None);
    tr.translate("m1_check", TranslateRequest { text: text.into(), direction: None }, |ev| match ev {
        TranslateEvent::Delta { text } => out.lock().unwrap().push_str(&text),
        TranslateEvent::Error { message } => *err.lock().unwrap() = Some(message),
        TranslateEvent::Cancelled => *err.lock().unwrap() = Some("已取消".into()),
        _ => {}
    })
    .await;
    match err.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(out.into_inner().unwrap()),
    }
}

async fn infer(models_dir: &Path) {
    let m = Arc::new(manifest());
    let model = m.get("index-translate-2b-q4").unwrap().clone();
    let notices = Arc::new(std::sync::Mutex::new(Vec::new()));
    let n2 = notices.clone();
    let sup = Supervisor::new(Arc::new(move |n| {
        println!("    notice: {n:?}");
        n2.lock().unwrap().push(n);
    }));
    let tr = Translator::new(sup.clone(), m.clone());
    let log = std::env::temp_dir().join("m1_check_llama.log");
    let spec = |backend, device: Option<&str>| LaunchSpec {
        exe: llama(),
        model_path: model.path_in(models_dir),
        model_id: model.id.clone(),
        backend,
        device: device.map(str::to_owned),
        threads: 8,
        ctx: model.ctx,
        log_path: log.clone(),
    };

    println!("[1] GPU start failure (bad device) -> retries, then CPU fallback");
    sup.configure(spec(Backend::Vulkan, Some("Vulkan9"))).await;
    let t0 = Instant::now();
    let r = sup.start().await;
    println!("  start(): {r:?}");
    assert!(r.is_err());
    let r = sup.ensure_ready().await;
    println!("  ensure_ready(): {r:?} in {:.1}s, status {:?}", t0.elapsed().as_secs_f64(), sup.status());
    assert!(r.is_ok());
    assert_eq!(sup.spec().unwrap().backend, Backend::Cpu);
    assert_eq!(notices.lock().unwrap().len(), 1);

    println!("[2] GPU translate, mixed-language paragraphs (direction per paragraph)");
    sup.configure(spec(Backend::Vulkan, Some("Vulkan0"))).await;
    let text = "本地翻译模型不依赖网络。\n\n  It has certain hardware requirements.\n----\n今天天气不错。";
    let out = std::sync::Mutex::new(String::new());
    tr.translate("m1_check", TranslateRequest { text: text.into(), direction: None }, |ev| {
        if let TranslateEvent::Delta { text } = &ev {
            out.lock().unwrap().push_str(text);
        } else {
            println!("    event {ev:?}");
        }
    })
    .await;
    println!("  -> {:?}", out.lock().unwrap());
    let out = out.into_inner().unwrap();
    let lines: Vec<&str> = out.split('\n').collect();
    assert_eq!(lines.len(), 5, "{lines:?}");
    let has_cjk = |s: &str| s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    assert!(!has_cjk(lines[0]) && has_cjk(lines[2]) && !has_cjk(lines[4]), "{lines:?}");
    assert_eq!(lines[3], "----");

    println!("[3] cancel: a second request cancels the first");
    let long = "Large language models have changed how software is built. ".repeat(20);
    let (a, b) = tokio::join!(
        async {
            let last = std::sync::Mutex::new(None);
            tr.translate("m1_check", TranslateRequest { text: long.clone(), direction: None }, |ev| {
                if !matches!(ev, TranslateEvent::Delta { .. }) {
                    *last.lock().unwrap() = Some(ev);
                }
            })
            .await;
            last.into_inner().unwrap()
        },
        async {
            tokio::time::sleep(Duration::from_millis(400)).await;
            translate_text(&tr, "再见").await
        }
    );
    println!("  first: {a:?}\n  second: {b:?}");
    assert!(matches!(a, Some(TranslateEvent::Cancelled)));
    assert!(b.unwrap().to_lowercase().contains("bye"));

    println!("[4] crash: kill llama-server -> automatic restart");
    let _ = std::process::Command::new("taskkill").args(["/F", "/IM", "llama-server.exe"]).output();
    let s = wait_status(&sup, |s| matches!(s, Status::Failed { .. }), 10).await;
    println!("  after kill: {s:?}");
    let t0 = Instant::now();
    let s = wait_status(&sup, |s| matches!(s, Status::Ready { .. }), 60).await;
    println!("  recovered: {s:?} in {:.1}s", t0.elapsed().as_secs_f64());
    assert!(matches!(s, Status::Ready { backend: Backend::Vulkan, .. }));
    let r = translate_text(&tr, "Thank you").await;
    println!("  translate after restart: {r:?}");
    assert!(r.is_ok());

    println!("[5] idle release -> Idle, next request reloads");
    sup.set_idle_minutes(1);
    let t0 = Instant::now();
    let s = wait_status(&sup, |s| matches!(s, Status::Idle), 120).await;
    println!("  {s:?} after {:.0}s", t0.elapsed().as_secs_f64());
    assert!(matches!(s, Status::Idle));
    let r = translate_text(&tr, "Good night").await;
    println!("  reload + translate: {r:?} status {:?}", sup.status());
    assert!(r.is_ok());
    sup.stop().await;
    println!("inference checks passed");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        match args.get(1).map(String::as_str) {
            Some("detect") => detect().await,
            Some("download") => download(Path::new(&args[2])).await,
            Some("infer") => infer(Path::new(&args[2])).await,
            _ => eprintln!("usage: m1_check detect | download <dir> | infer <models_dir>"),
        }
    });
}
