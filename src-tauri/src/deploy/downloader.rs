//! Model downloader (DESIGN 5.1 Download): probe sources with a 2 MB Range request,
//! use the fastest, resume `.part` files with Range, retry and fail over between
//! sources, then verify SHA256. Direct connections only (no proxy handling).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, RANGE};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use super::hardware::disk_free;
use super::manifest::{ModelEntry, Source};

const PROBE_BYTES: u64 = 2 * 1024 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);
const PROBE_GRACE: Duration = Duration::from_secs(3);
const ATTEMPTS_PER_SOURCE: u32 = 3;
const ROUNDS: u32 = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum Progress {
    #[serde(rename_all = "camelCase")]
    Probe { source: String, speed_bps: Option<f64>, error: Option<String> },
    #[serde(rename_all = "camelCase")]
    Download { source: String, done: u64, total: u64, speed_bps: f64 },
    #[serde(rename_all = "camelCase")]
    Retry { source: String, message: String, wait_secs: u64 },
    #[serde(rename_all = "camelCase")]
    Verify { done: u64, total: u64 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum DownloadError {
    Paused,
    Cancelled,
    /// No source answered the probe.
    NoSource(String),
    /// Sources answered but the transfer kept failing.
    Network(String),
    DiskSpace(String),
    Checksum(String),
    Io(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Paused => write!(f, "已暂停"),
            Self::Cancelled => write!(f, "已取消"),
            Self::NoSource(m) => write!(f, "无法连接任何下载源：{m}"),
            Self::Network(m) => write!(f, "网络连接中断：{m}"),
            Self::DiskSpace(m) | Self::Checksum(m) | Self::Io(m) => f.write_str(m),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Pause,
    Cancel,
}

/// Pause / cancel handle for one running download.
#[derive(Clone, Default)]
pub struct Control {
    token: CancellationToken,
    reason: Arc<Mutex<Option<StopReason>>>,
}

impl Control {
    /// A cancel already asked for stays a cancel (closing the wizard pauses
    /// everything it started; that must not keep a cancelled `.part`).
    pub fn stop(&self, reason: StopReason) {
        let mut r = self.reason.lock().unwrap();
        if *r != Some(StopReason::Cancel) {
            *r = Some(reason);
        }
        drop(r);
        self.token.cancel();
    }

    fn stopped(&self) -> Option<DownloadError> {
        match *self.reason.lock().unwrap() {
            Some(StopReason::Pause) => Some(DownloadError::Paused),
            Some(StopReason::Cancel) => Some(DownloadError::Cancelled),
            None => None,
        }
    }
}

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .user_agent(concat!("LLMTranslate/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
}

fn io_err(e: impl std::fmt::Display) -> DownloadError {
    DownloadError::Io(format!("文件读写失败：{e}"))
}

pub fn part_path(final_path: &Path) -> PathBuf {
    let mut s = final_path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Bytes already on disk for this model (final file or partial).
pub fn downloaded_bytes(model: &ModelEntry, dir: &Path) -> u64 {
    let final_path = model.path_in(dir);
    if let Ok(m) = std::fs::metadata(&final_path) {
        return m.len();
    }
    std::fs::metadata(part_path(&final_path)).map(|m| m.len()).unwrap_or(0)
}

/// Total of the `Content-Range: bytes a-b/total` header.
fn content_range(resp: &reqwest::Response) -> Option<(u64, u64)> {
    let v = resp.headers().get(CONTENT_RANGE)?.to_str().ok()?;
    let v = v.strip_prefix("bytes ")?;
    let (range, total) = v.split_once('/')?;
    let start = range.split('-').next()?.parse().ok()?;
    Some((start, total.parse().ok()?))
}

/// Download the first 2 MB of each source in parallel; return the reachable
/// sources ordered by speed (fastest first).
pub async fn probe_sources(
    client: &reqwest::Client,
    model: &ModelEntry,
    progress: &ProgressFn,
) -> Result<Vec<Source>, DownloadError> {
    let tasks = model.sources.iter().cloned().map(|src| {
        let client = client.clone();
        let size = model.size;
        async move {
            let r = tokio::time::timeout(PROBE_TIMEOUT, async {
                let t0 = Instant::now();
                let resp = client
                    .get(&src.url)
                    .header(RANGE, format!("bytes=0-{}", PROBE_BYTES - 1))
                    .send()
                    .await
                    .map_err(|e| short_err(&e))?;
                // 206 = Range honoured; 200 = whole file (resume then restarts from 0).
                let total = match resp.status() {
                    StatusCode::PARTIAL_CONTENT => content_range(&resp).map(|(_, t)| t),
                    StatusCode::OK => resp.content_length(),
                    s => return Err(format!("HTTP {}", s.as_u16())),
                };
                if let Some(total) = total {
                    if total != size {
                        return Err(format!("文件大小不符（{total}）"));
                    }
                }
                let mut n = 0u64;
                let mut stream = resp.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    n += chunk.map_err(|e| short_err(&e))?.len() as u64;
                    if n >= PROBE_BYTES {
                        break;
                    }
                }
                Ok(n as f64 / t0.elapsed().as_secs_f64().max(1e-3))
            })
            .await
            .unwrap_or_else(|_| Err("连接超时".into()));
            (src, r)
        }
    });
    // Once one source answered, give the others PROBE_GRACE more, instead of
    // waiting the full timeout for a dead one.
    let mut pending: futures_util::stream::FuturesUnordered<_> = tasks.collect();
    let mut results = Vec::new();
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        let next = match deadline {
            Some(d) => match tokio::time::timeout_at(d, pending.next()).await {
                Ok(n) => n,
                Err(_) => break,
            },
            None => pending.next().await,
        };
        let Some((src, r)) = next else { break };
        if r.is_ok() && deadline.is_none() {
            deadline = Some(tokio::time::Instant::now() + PROBE_GRACE);
        }
        results.push((src, r));
    }
    let late: Vec<Source> = model
        .sources
        .iter()
        .filter(|s| !results.iter().any(|(r, _): &(Source, _)| r.name == s.name))
        .cloned()
        .collect();
    results.extend(late.into_iter().map(|src| (src, Err("响应过慢".to_string()))));
    let mut ok = Vec::new();
    let mut errors = Vec::new();
    for (src, r) in results {
        match &r {
            Ok(speed) => ok.push((src.clone(), *speed)),
            Err(e) => errors.push(format!("{} {e}", src.name)),
        }
        progress(Progress::Probe {
            source: src.name.clone(),
            speed_bps: r.as_ref().ok().copied(),
            error: r.err(),
        });
    }
    if ok.is_empty() {
        return Err(DownloadError::NoSource(errors.join("；")));
    }
    ok.sort_by(|a, b| b.1.total_cmp(&a.1));
    tracing::info!(
        "probe {}: {}",
        model.id,
        ok.iter().map(|(s, v)| format!("{} {:.1} MB/s", s.name, v / 1e6)).collect::<Vec<_>>().join(", ")
    );
    Ok(ok.into_iter().map(|(s, _)| s).collect())
}

fn short_err(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "连接超时".into()
    } else if e.is_connect() {
        "无法连接".into()
    } else {
        // By characters: OS messages are often Chinese, and cutting a
        // String inside a multi-byte char panics.
        e.to_string().chars().take(120).collect()
    }
}

enum FetchError {
    /// Worth retrying the same source.
    Transient(String),
    /// This source is unusable (404, wrong file...).
    BadSource(String),
    Fatal(DownloadError),
}

/// Fetch from `offset` to the end, appending to `part`.
async fn fetch_range(
    client: &reqwest::Client,
    src: &Source,
    part: &Path,
    offset: u64,
    total: u64,
    ctl: &Control,
    progress: &ProgressFn,
) -> Result<(), FetchError> {
    let req = client.get(&src.url).header(RANGE, format!("bytes={offset}-"));
    let resp = tokio::select! {
        r = req.send() => r.map_err(|e| FetchError::Transient(short_err(&e)))?,
        _ = ctl.token.cancelled() => return Err(FetchError::Fatal(ctl.stopped().unwrap())),
    };
    let status = resp.status();
    let mut offset = offset;
    let mut file_opts = tokio::fs::OpenOptions::new();
    file_opts.create(true);
    match status {
        StatusCode::PARTIAL_CONTENT => {
            match content_range(&resp) {
                Some((start, t)) if start == offset && t == total => {}
                Some((start, t)) => {
                    return Err(FetchError::BadSource(format!("Range 响应不符（{start}/{t}）")));
                }
                None => return Err(FetchError::BadSource("缺少 Content-Range".into())),
            }
            file_opts.append(true);
        }
        StatusCode::OK => {
            // Server ignored Range: start over from byte 0.
            offset = 0;
            file_opts.write(true).truncate(true);
        }
        StatusCode::RANGE_NOT_SATISFIABLE => {
            // Our .part is bigger than the file: discard it.
            let _ = tokio::fs::remove_file(part).await;
            return Err(FetchError::Transient("续传位置无效，重新下载".into()));
        }
        s if s.is_server_error() || s == StatusCode::TOO_MANY_REQUESTS => {
            return Err(FetchError::Transient(format!("HTTP {}", s.as_u16())));
        }
        s => return Err(FetchError::BadSource(format!("HTTP {}", s.as_u16()))),
    }

    let file = file_opts.open(part).await.map_err(|e| FetchError::Fatal(io_err(e)))?;
    let mut w = tokio::io::BufWriter::with_capacity(1 << 20, file);
    let mut stream = resp.bytes_stream();
    let mut done = offset;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    // Speed over a sliding window of ~3 s.
    let mut window: std::collections::VecDeque<(Instant, u64)> = Default::default();
    window.push_back((Instant::now(), done));

    let result = loop {
        let next = tokio::select! {
            n = stream.next() => n,
            _ = ctl.token.cancelled() => break Err(FetchError::Fatal(ctl.stopped().unwrap())),
        };
        match next {
            None => break Ok(()),
            Some(Err(e)) => break Err(FetchError::Transient(short_err(&e))),
            Some(Ok(chunk)) => {
                if let Err(e) = w.write_all(&chunk).await {
                    break Err(FetchError::Fatal(io_err(e)));
                }
                done += chunk.len() as u64;
                if last_emit.elapsed() >= Duration::from_millis(250) {
                    let now = Instant::now();
                    window.push_back((now, done));
                    while window.len() > 2 && now - window[0].0 > Duration::from_secs(3) {
                        window.pop_front();
                    }
                    let (t0, b0) = window[0];
                    let speed = (done - b0) as f64 / (now - t0).as_secs_f64().max(1e-3);
                    progress(Progress::Download { source: src.name.clone(), done, total, speed_bps: speed });
                    last_emit = now;
                }
            }
        }
    };
    if let Err(e) = w.flush().await {
        return Err(FetchError::Fatal(io_err(e)));
    }
    result?;
    if done < total {
        return Err(FetchError::Transient("连接提前结束".into()));
    }
    Ok(())
}

/// SHA256 of a file, reporting progress.
pub async fn sha256_file(path: &Path, progress: ProgressFn, ctl: Control) -> Result<String, DownloadError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut f = std::fs::File::open(&path).map_err(io_err)?;
        let total = f.metadata().map_err(io_err)?.len();
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 4 << 20];
        let mut done = 0u64;
        let mut last = Instant::now();
        loop {
            if let Some(e) = ctl.stopped() {
                return Err(e);
            }
            let n = f.read(&mut buf).map_err(io_err)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            done += n as u64;
            if last.elapsed() >= Duration::from_millis(250) {
                progress(Progress::Verify { done, total });
                last = Instant::now();
            }
        }
        progress(Progress::Verify { done: total, total });
        Ok(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(io_err)?
}

/// Download `model` into `dir`, verifying SHA256. Returns the final path.
pub async fn download(
    client: &reqwest::Client,
    model: &ModelEntry,
    dir: &Path,
    ctl: &Control,
    progress: ProgressFn,
) -> Result<PathBuf, DownloadError> {
    tokio::fs::create_dir_all(dir).await.map_err(io_err)?;
    let final_path = model.path_in(dir);
    let part = part_path(&final_path);

    // Already present: verify it instead of downloading again.
    if tokio::fs::metadata(&final_path).await.is_ok_and(|m| m.len() == model.size) {
        let hash = sha256_file(&final_path, progress.clone(), ctl.clone()).await?;
        if hash.eq_ignore_ascii_case(&model.sha256) {
            return Ok(final_path);
        }
        tracing::warn!("{} exists but hash mismatches; downloading again", model.file);
        let _ = tokio::fs::remove_file(&final_path).await;
    }

    let mut checksum_retries = 0;
    loop {
        let offset = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
        let needed = model.size.saturating_sub(offset) + model.size / 5;
        if let Some(free) = disk_free(dir) {
            if free < needed {
                return Err(DownloadError::DiskSpace(format!(
                    "磁盘空间不足：需要 {:.1} GB，剩余 {:.1} GB",
                    needed as f64 / 1e9,
                    free as f64 / 1e9
                )));
            }
        }

        if offset < model.size {
            transfer(client, model, &part, ctl, &progress).await?;
        }

        let hash = sha256_file(&part, progress.clone(), ctl.clone()).await?;
        if hash.eq_ignore_ascii_case(&model.sha256) {
            finalize(&part, &final_path).await?;
            return Ok(final_path);
        }
        let _ = tokio::fs::remove_file(&part).await;
        checksum_retries += 1;
        tracing::warn!("{} sha256 mismatch ({hash}), attempt {checksum_retries}", model.file);
        if checksum_retries >= 2 {
            return Err(DownloadError::Checksum("文件校验失败（SHA256 不一致），已删除，请重试".into()));
        }
    }
}

/// `.part` -> final name. Rename can fail across volumes (e.g. a redirected or
/// virtualized folder, os error 17), so fall back to copy + delete.
async fn finalize(part: &Path, final_path: &Path) -> Result<(), DownloadError> {
    match tokio::fs::rename(part, final_path).await {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!("rename failed ({e}); copying instead");
            tokio::fs::copy(part, final_path).await.map_err(io_err)?;
            let _ = tokio::fs::remove_file(part).await;
            Ok(())
        }
    }
}

/// Bring `part` up to the full size, trying sources in speed order.
async fn transfer(
    client: &reqwest::Client,
    model: &ModelEntry,
    part: &Path,
    ctl: &Control,
    progress: &ProgressFn,
) -> Result<(), DownloadError> {
    let mut last_error = String::new();
    for round in 0..ROUNDS {
        if let Some(e) = ctl.stopped() {
            return Err(e);
        }
        let sources = match probe_sources(client, model, progress).await {
            Ok(s) => s,
            Err(e) if round + 1 < ROUNDS => {
                last_error = e.to_string();
                wait(ctl, progress, "全部", &last_error, 5).await?;
                continue;
            }
            Err(e) => return Err(e),
        };
        for src in &sources {
            let mut attempts = 0;
            while attempts < ATTEMPTS_PER_SOURCE {
                let offset = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
                if offset >= model.size {
                    return Ok(());
                }
                match fetch_range(client, src, part, offset, model.size, ctl, progress).await {
                    Ok(()) => return Ok(()),
                    Err(FetchError::Fatal(e)) => return Err(e),
                    Err(FetchError::BadSource(m)) => {
                        tracing::warn!("source {} unusable: {m}", src.name);
                        last_error = format!("{} {m}", src.name);
                        break;
                    }
                    Err(FetchError::Transient(m)) => {
                        let now = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
                        // A connection that made progress does not count as a failed attempt.
                        if now <= offset {
                            attempts += 1;
                        }
                        tracing::warn!("source {} failed at {offset}: {m}", src.name);
                        last_error = format!("{} {m}", src.name);
                        if attempts < ATTEMPTS_PER_SOURCE {
                            wait(ctl, progress, &src.name, &m, 1 << attempts).await?;
                        }
                    }
                }
            }
        }
        if round + 1 < ROUNDS {
            wait(ctl, progress, "全部", &last_error, 5).await?;
        }
    }
    Err(DownloadError::Network(last_error))
}

async fn wait(ctl: &Control, progress: &ProgressFn, source: &str, msg: &str, secs: u64) -> Result<(), DownloadError> {
    progress(Progress::Retry { source: source.into(), message: msg.into(), wait_secs: secs });
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(secs)) => Ok(()),
        _ = ctl.token.cancelled() => Err(ctl.stopped().unwrap()),
    }
}
