//! llama-server supervisor (DESIGN 5.2): start with a random port, wait for
//! `/health`, restart on crash with exponential backoff, fall back to CPU after
//! repeated GPU failures, release the model after an idle period.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::process::{CREATE_NO_WINDOW, attach_to_job};
use crate::app::settings::Backend;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_RESTARTS: u32 = 3;
/// A server that ran this long resets the crash counter.
const STABLE_AFTER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub exe: PathBuf,
    pub model_path: PathBuf,
    pub model_id: String,
    pub backend: Backend,
    pub device: Option<String>,
    pub threads: u32,
    pub ctx: u32,
    pub log_path: PathBuf,
}

impl LaunchSpec {
    pub fn args(&self, port: u16) -> Vec<String> {
        let mut a: Vec<String> = vec![
            "-m".into(),
            self.model_path.to_string_lossy().into_owned(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
            "-c".into(),
            self.ctx.to_string(),
            "--parallel".into(),
            "1".into(),
            "-t".into(),
            self.threads.to_string(),
        ];
        match self.backend {
            Backend::Vulkan => {
                a.extend(["-ngl".into(), "99".into()]);
                if let Some(d) = &self.device {
                    a.extend(["--device".into(), d.clone()]);
                }
            }
            Backend::Cpu => a.extend(["--device".into(), "none".into(), "-ngl".into(), "0".into()]),
        }
        a
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    /// Not configured yet (no model deployed).
    Unconfigured,
    Stopped,
    #[serde(rename_all = "camelCase")]
    Loading { backend: Backend, model_id: String },
    #[serde(rename_all = "camelCase")]
    Ready { backend: Backend, model_id: String, port: u16 },
    /// Released after the idle period; next request starts it again.
    Idle,
    #[serde(rename_all = "camelCase")]
    Failed { message: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Notice {
    /// GPU backend kept failing; switched to CPU and saved that choice.
    FellBackToCpu { reason: String },
}

pub type NoticeFn = Arc<dyn Fn(Notice) + Send + Sync>;

struct Running {
    kill: CancellationToken,
    exited: watch::Receiver<bool>,
    started: Instant,
}

struct State {
    spec: Option<LaunchSpec>,
    running: Option<Running>,
    /// Bumped on every intentional stop/start so stale monitors can tell.
    generation: u64,
    crashes: u32,
}

pub struct Supervisor {
    state: tokio::sync::Mutex<State>,
    /// Copy of `State::spec` readable without waiting for a model load that
    /// holds `state` (translation requests must stay cancellable meanwhile).
    spec: Mutex<Option<LaunchSpec>>,
    status: watch::Sender<Status>,
    last_used: Mutex<Instant>,
    idle_minutes: AtomicU32,
    on_notice: NoticeFn,
}

fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Remove ANSI colour sequences (`ESC [ ... letter`) that llama-server writes
/// even to a file; the text ends up in UI error messages.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '' {
            if chars.clone().next() == Some('[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn log_tail(path: &PathBuf) -> String {
    let text = std::fs::read(path).map(|b| strip_ansi(&String::from_utf8_lossy(&b))).unwrap_or_default();
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("error") || l.contains("failed") || l.contains("exception")
        })
        .collect();
    let picked = if lines.is_empty() {
        text.lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>()
    } else {
        lines.into_iter().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect()
    };
    picked.join("\n")
}

impl Supervisor {
    pub fn new(on_notice: NoticeFn) -> Arc<Self> {
        let sup = Arc::new(Self {
            state: tokio::sync::Mutex::new(State { spec: None, running: None, generation: 0, crashes: 0 }),
            spec: Mutex::new(None),
            status: watch::channel(Status::Unconfigured).0,
            last_used: Mutex::new(Instant::now()),
            idle_minutes: AtomicU32::new(0),
            on_notice,
        });
        let weak = Arc::downgrade(&sup);
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(sup) = weak.upgrade() else { break };
                sup.check_idle().await;
            }
        });
        sup
    }

    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    pub fn set_idle_minutes(&self, m: u32) {
        self.idle_minutes.store(m, Ordering::Relaxed);
    }

    pub fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }

    /// The configured launch spec. Never waits for a running start.
    pub fn spec(&self) -> Option<LaunchSpec> {
        self.spec.lock().unwrap().clone()
    }

    fn fail(&self, msg: String) -> String {
        self.status.send_replace(Status::Failed { message: msg.clone() });
        msg
    }

    /// Set what to run. Stops a running server whose spec differs.
    pub async fn configure(&self, spec: LaunchSpec) {
        let mut st = self.state.lock().await;
        if st.spec.as_ref() == Some(&spec) {
            return;
        }
        Self::stop_locked(&mut st).await;
        *self.spec.lock().unwrap() = Some(spec.clone());
        st.spec = Some(spec);
        st.crashes = 0;
        self.status.send_replace(Status::Stopped);
    }

    pub async fn stop(&self) {
        let mut st = self.state.lock().await;
        Self::stop_locked(&mut st).await;
        if st.spec.is_some() {
            self.status.send_replace(Status::Stopped);
        }
    }

    async fn stop_locked(st: &mut State) {
        st.generation += 1;
        if let Some(r) = st.running.take() {
            r.kill.cancel();
            let mut exited = r.exited;
            let _ = tokio::time::timeout(Duration::from_secs(10), exited.wait_for(|e| *e)).await;
        }
    }

    /// Start (once) with the configured spec and wait until healthy. No fallback.
    pub async fn start(self: &Arc<Self>) -> Result<u16, String> {
        let mut st = self.state.lock().await;
        if let (Some(_), Status::Ready { port, .. }) = (&st.running, self.status()) {
            return Ok(port);
        }
        self.start_locked(&mut st).await
    }

    async fn start_locked(self: &Arc<Self>, st: &mut State) -> Result<u16, String> {
        Self::stop_locked(st).await;
        let spec = st.spec.clone().ok_or("尚未部署模型")?;
        if !spec.exe.exists() {
            return Err(self.fail(format!("找不到推理程序：{}", spec.exe.display())));
        }
        if !spec.model_path.exists() {
            return Err(self.fail(format!("找不到模型文件：{}", spec.model_path.display())));
        }
        self.status.send_replace(Status::Loading { backend: spec.backend, model_id: spec.model_id.clone() });

        let port = free_port().map_err(|e| self.fail(format!("无法分配端口：{e}")))?;
        if let Some(dir) = spec.log_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let log = std::fs::File::create(&spec.log_path).map_err(|e| self.fail(format!("无法写日志：{e}")))?;
        let mut cmd = tokio::process::Command::new(&spec.exe);
        cmd.args(spec.args(port))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .creation_flags(CREATE_NO_WINDOW)
            .kill_on_drop(true);
        if let Some(dir) = spec.exe.parent() {
            cmd.current_dir(dir);
        }
        tracing::info!("starting llama-server: {:?}", spec.args(port));
        let mut child = cmd.spawn().map_err(|e| self.fail(format!("无法启动推理程序：{e}")))?;
        attach_to_job(&child);

        st.generation += 1;
        let generation = st.generation;
        let kill = CancellationToken::new();
        let (exited_tx, exited_rx) = watch::channel(false);
        let started = Instant::now();
        {
            let kill = kill.clone();
            let weak = Arc::downgrade(self);
            tauri::async_runtime::spawn(async move {
                let code = tokio::select! {
                    r = child.wait() => r.ok().and_then(|s| s.code()),
                    _ = kill.cancelled() => {
                        let _ = child.kill().await;
                        None
                    }
                };
                let _ = exited_tx.send(true);
                if !kill.is_cancelled() {
                    tracing::warn!("llama-server exited unexpectedly (code {code:?})");
                    if let Some(sup) = weak.upgrade() {
                        sup.on_crash(generation).await;
                    }
                }
            });
        }
        st.running = Some(Running { kill, exited: exited_rx.clone(), started });

        // Wait for /health or an early exit.
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap();
        let url = format!("http://127.0.0.1:{port}/health");
        let deadline = Instant::now() + HEALTH_TIMEOUT;
        loop {
            if *exited_rx.borrow() {
                st.running = None;
                // The monitor saw this exit too; the caller handles a failed
                // start, so make the monitor's crash restart a no-op.
                st.generation += 1;
                let tail = log_tail(&spec.log_path);
                let msg = if tail.is_empty() { "推理程序启动后退出".to_string() } else { format!("推理程序启动失败：{tail}") };
                return Err(self.fail(msg));
            }
            if Instant::now() > deadline {
                Self::stop_locked(st).await;
                return Err(self.fail("模型加载超时".to_string()));
            }
            if let Ok(r) = client.get(&url).send().await {
                if r.status().is_success() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tracing::info!("llama-server ready on {port} after {:.1}s", started.elapsed().as_secs_f64());
        self.touch();
        self.status.send_replace(Status::Ready { backend: spec.backend, model_id: spec.model_id.clone(), port });
        Ok(port)
    }

    /// Port of a ready server, starting it (with retries and CPU fallback) if needed.
    pub async fn ensure_ready(self: &Arc<Self>) -> Result<u16, String> {
        self.touch();
        let mut st = self.state.lock().await;
        if let (Some(_), Status::Ready { port, .. }) = (&st.running, self.status()) {
            return Ok(port);
        }
        self.recover_locked(&mut st, 0).await
    }

    /// Start attempts `first..=MAX_RESTARTS` with backoff (none before the
    /// first: a crash handler has already waited); then GPU -> CPU fallback.
    /// `first > 0` continues after that many crashes, so every restart counts
    /// towards the same limit (DESIGN 5.2: at most 3, then CPU).
    async fn recover_locked(self: &Arc<Self>, st: &mut State, first: u32) -> Result<u16, String> {
        let mut last = String::new();
        for attempt in first..=MAX_RESTARTS {
            if attempt > first {
                tokio::time::sleep(Duration::from_secs(1 << (attempt - 1))).await;
            }
            if first > 0 {
                st.crashes = attempt;
            }
            match self.start_locked(st).await {
                Ok(p) => return Ok(p),
                Err(e) => {
                    tracing::warn!("start attempt {attempt} failed: {e}");
                    last = e;
                    // Missing files will not fix themselves.
                    if last.starts_with("找不到") {
                        return Err(last);
                    }
                }
            }
        }
        self.fallback_locked(st, last).await
    }

    async fn fallback_locked(self: &Arc<Self>, st: &mut State, reason: String) -> Result<u16, String> {
        let Some(gpu) = st.spec.clone() else { return Err(reason) };
        if gpu.backend != Backend::Vulkan {
            return Err(reason);
        }
        let cpu = LaunchSpec { backend: Backend::Cpu, device: None, ..gpu.clone() };
        st.spec = Some(cpu.clone());
        st.crashes = 0;
        tracing::warn!("falling back to CPU backend: {reason}");
        match self.start_locked(st).await {
            Ok(port) => {
                *self.spec.lock().unwrap() = Some(cpu);
                // Saved to settings only now: if CPU fails too the cause is
                // not the GPU (e.g. a broken model) and the choice stays.
                (self.on_notice)(Notice::FellBackToCpu { reason });
                Ok(port)
            }
            Err(e) => {
                st.spec = Some(gpu);
                Err(e)
            }
        }
    }

    /// Boxed to break the start_locked -> monitor -> on_crash -> start_locked type cycle.
    fn on_crash(self: Arc<Self>, generation: u64) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(self.on_crash_inner(generation))
    }

    async fn on_crash_inner(self: Arc<Self>, generation: u64) {
        let n = {
            let mut st = self.state.lock().await;
            if st.generation != generation {
                return; // superseded by an intentional stop/start
            }
            let ran = st.running.take().map(|r| r.started.elapsed()).unwrap_or_default();
            if ran >= STABLE_AFTER {
                st.crashes = 0;
            }
            st.crashes += 1;
            st.crashes
        };
        self.status.send_replace(Status::Failed { message: "推理程序意外退出，正在重启".into() });
        if n <= MAX_RESTARTS {
            tokio::time::sleep(Duration::from_secs(1 << (n - 1))).await;
        }
        let mut st = self.state.lock().await;
        if st.generation != generation || st.running.is_some() {
            return; // someone stopped or restarted it meanwhile
        }
        // A failed restart is reported by start_locked itself (its exit
        // monitor stays quiet), so the remaining restarts and the fallback
        // happen here.
        let result = if n <= MAX_RESTARTS {
            self.recover_locked(&mut st, n).await
        } else {
            self.fallback_locked(&mut st, format!("推理程序连续崩溃 {n} 次")).await
        };
        if let Err(e) = result {
            tracing::error!("restart after crash failed: {e}");
            // Not every failure path updates the status (e.g. no CPU fallback
            // left); never leave "正在重启" showing when nothing is restarting.
            self.fail(e);
        }
    }

    async fn check_idle(self: &Arc<Self>) {
        let minutes = self.idle_minutes.load(Ordering::Relaxed);
        if minutes == 0 || !matches!(self.status(), Status::Ready { .. }) {
            return;
        }
        if self.last_used.lock().unwrap().elapsed() < Duration::from_secs(minutes as u64 * 60) {
            return;
        }
        let mut st = self.state.lock().await;
        // A request may have started the model or used it while we waited.
        let minutes = self.idle_minutes.load(Ordering::Relaxed);
        if minutes == 0
            || !matches!(self.status(), Status::Ready { .. })
            || self.last_used.lock().unwrap().elapsed() < Duration::from_secs(minutes as u64 * 60)
        {
            return;
        }
        tracing::info!("releasing model after {minutes} idle minutes");
        Self::stop_locked(&mut st).await;
        self.status.send_replace(Status::Idle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server that exits during startup is reported by `start` alone: the
    /// exit monitor must not restart it or overwrite the failure message.
    #[tokio::test]
    async fn early_exit_is_not_restarted_by_monitor() {
        let tmp = std::env::temp_dir().join(format!("llmt-sup-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let model = tmp.join("model.gguf");
        std::fs::write(&model, b"x").unwrap();
        let sup = Supervisor::new(Arc::new(|_| {}));
        sup.configure(LaunchSpec {
            // Exits at once: stdin is null.
            exe: PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into())),
            model_path: model,
            model_id: "test".into(),
            backend: Backend::Cpu,
            device: None,
            threads: 1,
            ctx: 512,
            log_path: tmp.join("server.log"),
        })
        .await;
        let err = sup.start().await.unwrap_err();
        assert!(err.starts_with("推理程序启动"), "{err}");
        // The monitor's crash handler would restart after a 1 s backoff.
        tokio::time::sleep(Duration::from_millis(1800)).await;
        assert_eq!(sup.status(), Status::Failed { message: err });
        assert!(sup.state.lock().await.running.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn strips_ansi() {
        assert_eq!(
            super::strip_ansi("[34m0.00.356[0m [31mE failed to load[0m"),
            "0.00.356 E failed to load"
        );
    }
}
