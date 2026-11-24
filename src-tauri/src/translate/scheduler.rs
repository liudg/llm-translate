//! Request scheduling (DESIGN 5.3): one queue; a new request cancels the one in
//! progress; segments are translated in order and streamed out.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::future::{BoxFuture, FutureExt, Shared};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::cache::Cache;
use super::clean::Cleaner;
use super::detect::{Direction, detect, detect_paragraph};
use super::prompt;
use super::segment::{Piece, segment};
use crate::deploy::manifest::{Manifest, ModelEntry};
use crate::inference::client::{ChatError, local_client, stream_chat};
use crate::inference::supervisor::Supervisor;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateRequest {
    pub text: String,
    /// `None` = detect automatically.
    pub direction: Option<Direction>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TranslateEvent {
    /// `mixed`: auto mode found paragraphs in both directions; `direction` is
    /// then only the majority.
    Start { direction: Direction, mixed: bool },
    Delta { text: String },
    #[serde(rename_all = "camelCase")]
    Done { elapsed_ms: u64 },
    Error { message: String },
    Cancelled,
}

/// Several independent texts (screenshot paragraphs) translated as one
/// request: one cancellation, results tagged with the item index.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRequest {
    pub items: Vec<String>,
    pub direction: Option<Direction>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BatchEvent {
    Start,
    Delta { index: usize, text: String },
    ItemDone { index: usize },
    #[serde(rename_all = "camelCase")]
    Done { elapsed_ms: u64 },
    Error { message: String },
    Cancelled,
}

pub struct Translator {
    sup: Arc<Supervisor>,
    manifest: Arc<Manifest>,
    http: reqwest::Client,
    cache: Cache,
    current: Mutex<CancellationToken>,
    serial: tokio::sync::Mutex<()>,
    /// The model start in progress, shared by every request that needs it.
    starting: Mutex<Option<Start>>,
}

/// Per request: the model's port once started, and whether the cache may
/// answer.
struct RunState {
    port: Option<u16>,
    use_cache: bool,
}

/// What a request talks to.
enum Target {
    /// The configured model, started on demand (`use_cache: false` always
    /// asks the model).
    Configured { use_cache: bool },
    /// One given process (deployment self-test): never the cache, never
    /// another process; if it dies, the request fails.
    Process { model: ModelEntry, port: u16 },
}

/// Port and generation of the started process (`Supervisor::is_live`).
type StartFuture = Shared<BoxFuture<'static, Result<(u16, u64), String>>>;

struct Start {
    fut: StartFuture,
    /// Set by the start task itself: `Shared` only learns the result when
    /// polled, and after a cancel nobody may poll it. A finished start's port
    /// may be stale (idle release, model switch), so it is never reused.
    done: Arc<AtomicBool>,
}

impl Translator {
    pub fn new(sup: Arc<Supervisor>, manifest: Arc<Manifest>) -> Self {
        Self {
            sup,
            manifest,
            http: local_client(),
            cache: Cache::new(500),
            current: Mutex::new(CancellationToken::new()),
            serial: tokio::sync::Mutex::new(()),
            starting: Mutex::new(None),
        }
    }

    /// Join the model start in progress, or begin one. It runs in its own
    /// task: a cancelled request only stops waiting (dropping a half-done
    /// start would leave llama-server running with the status stuck at
    /// Loading), and the next request waits for the same start instead of
    /// queueing a whole new round of retries behind it.
    fn start_model(&self) -> StartFuture {
        let mut slot = self.starting.lock().unwrap();
        if let Some(s) = slot.as_ref().filter(|s| !s.done.load(Ordering::SeqCst)) {
            return s.fut.clone();
        }
        let sup = self.sup.clone();
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let task = tauri::async_runtime::spawn(async move {
            let r = sup.ensure_ready_live().await;
            flag.store(true, Ordering::SeqCst);
            r
        });
        let fut = async move { task.await.map_err(|e| format!("启动推理失败：{e}"))? }.boxed().shared();
        *slot = Some(Start { fut: fut.clone(), done });
        fut
    }

    /// Cancel whatever is running.
    pub fn cancel(&self) {
        self.current.lock().unwrap().cancel();
    }

    /// Cancel the running request and hand out the token for a new one.
    fn begin(&self) -> CancellationToken {
        let mut cur = self.current.lock().unwrap();
        cur.cancel();
        *cur = CancellationToken::new();
        cur.clone()
    }

    /// Translate, streaming events to `sink`. Cancels any earlier request.
    pub async fn translate(&self, req: TranslateRequest, sink: impl Fn(TranslateEvent) + Send + Sync) {
        self.translate_with(req, Target::Configured { use_cache: true }, sink).await
    }

    /// The target is per request: a shared "no cache" switch would be reset
    /// by one self-test while another runs.
    async fn translate_with(&self, req: TranslateRequest, target: Target, sink: impl Fn(TranslateEvent) + Send + Sync) {
        let token = self.begin();
        let event = match self.run(&req, target, &token, &sink).await {
            Ok(ev) => ev,
            Err(ChatError::Cancelled) => TranslateEvent::Cancelled,
            Err(ChatError::Failed(m)) => TranslateEvent::Error { message: m },
        };
        sink(event);
    }

    /// Translate several texts in order. Cancels any earlier request; a later
    /// request cancels this whole batch.
    pub async fn translate_batch(&self, req: BatchRequest, sink: impl Fn(BatchEvent) + Send + Sync) {
        let token = self.begin();
        let event = match self.run_batch(&req, &token, &sink).await {
            Ok(ev) => ev,
            Err(ChatError::Cancelled) => BatchEvent::Cancelled,
            Err(ChatError::Failed(m)) => BatchEvent::Error { message: m },
        };
        sink(event);
    }

    /// Collect a whole translation from the configured model itself, never
    /// the cache. Cancels any earlier request.
    pub async fn translate_text(&self, text: &str, direction: Option<Direction>) -> Result<String, String> {
        self.collect(text, direction, Target::Configured { use_cache: false }).await
    }

    /// Deployment self-test: translate with the process the deployment just
    /// started on `port`, running `model`. Restarts and the CPU fallback are
    /// the deployment's business: if that process is gone, this fails.
    pub async fn self_test(&self, text: &str, model: ModelEntry, port: u16) -> Result<String, String> {
        self.collect(text, None, Target::Process { model, port }).await
    }

    async fn collect(&self, text: &str, direction: Option<Direction>, target: Target) -> Result<String, String> {
        let out = Mutex::new(String::new());
        let err = Mutex::new(None);
        self.translate_with(TranslateRequest { text: text.into(), direction }, target, |ev| match ev {
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

    async fn run(
        &self,
        req: &TranslateRequest,
        target: Target,
        token: &CancellationToken,
        sink: &(impl Fn(TranslateEvent) + Send + Sync),
    ) -> Result<TranslateEvent, ChatError> {
        // Wait for the previous request to wind down (it was just cancelled).
        let _guard = tokio::select! {
            g = self.serial.lock() => g,
            _ = token.cancelled() => return Err(ChatError::Cancelled),
        };
        let t0 = Instant::now();
        // Overall direction is only the label; in auto mode each paragraph is
        // detected on its own so mixed Chinese/English text works.
        let overall = req.direction.unwrap_or_else(|| detect(&req.text));

        // Paragraph direction; None = no letters, copied through unchanged.
        let para_dir = |chunks: &[String]| {
            let text = chunks.concat();
            detect_paragraph(&text).map(|d| req.direction.unwrap_or(d))
        };
        let pieces = segment(&req.text);
        let mixed = pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Text(c) => para_dir(c),
                Piece::Keep(_) => None,
            })
            .any(|d| d != overall);
        sink(TranslateEvent::Start { direction: overall, mixed });
        let needs_model = pieces.iter().any(|p| matches!(p, Piece::Text(c) if para_dir(c).is_some()));
        if !needs_model {
            for p in pieces {
                let text = match p {
                    Piece::Keep(s) => s,
                    Piece::Text(c) => c.join(" "),
                };
                sink(TranslateEvent::Delta { text });
            }
            return Ok(TranslateEvent::Done { elapsed_ms: 0 });
        }

        let (mut model, mut run) = match target {
            Target::Configured { use_cache } => (self.model()?, RunState { port: None, use_cache }),
            Target::Process { model, port } => (model, RunState { port: Some(port), use_cache: false }),
        };
        self.stream_pieces(&mut model, &mut run, token, pieces, para_dir, &|text| sink(TranslateEvent::Delta { text }))
            .await?;
        Ok(TranslateEvent::Done { elapsed_ms: t0.elapsed().as_millis() as u64 })
    }

    async fn run_batch(
        &self,
        req: &BatchRequest,
        token: &CancellationToken,
        sink: &(impl Fn(BatchEvent) + Send + Sync),
    ) -> Result<BatchEvent, ChatError> {
        let _guard = tokio::select! {
            g = self.serial.lock() => g,
            _ = token.cancelled() => return Err(ChatError::Cancelled),
        };
        let t0 = Instant::now();
        sink(BatchEvent::Start);
        let mut model = None;
        let mut run = RunState { port: None, use_cache: true };
        for (index, text) in req.items.iter().enumerate() {
            let para_dir = |chunks: &[String]| detect_paragraph(&chunks.concat()).map(|d| req.direction.unwrap_or(d));
            let pieces = segment(text);
            let needs_model = pieces.iter().any(|p| matches!(p, Piece::Text(c) if para_dir(c).is_some()));
            if needs_model && model.is_none() {
                model = Some(self.model()?);
            }
            match &mut model {
                Some(m) => {
                    self.stream_pieces(m, &mut run, token, pieces, para_dir, &|text| {
                        sink(BatchEvent::Delta { index, text })
                    })
                    .await?
                }
                None => {
                    for p in pieces {
                        let text = match p {
                            Piece::Keep(s) => s,
                            Piece::Text(c) => c.join(" "),
                        };
                        sink(BatchEvent::Delta { index, text });
                    }
                }
            }
            sink(BatchEvent::ItemDone { index });
        }
        Ok(BatchEvent::Done { elapsed_ms: t0.elapsed().as_millis() as u64 })
    }

    /// Port of a running model. A shared start may hand back a process that
    /// a model switch or idle release stopped while we waited for it: then
    /// start (or join) again.
    async fn live_port(&self, token: &CancellationToken) -> Result<u16, ChatError> {
        for _ in 0..3 {
            let (port, generation) = tokio::select! {
                r = self.start_model() => r.map_err(ChatError::Failed)?,
                _ = token.cancelled() => return Err(ChatError::Cancelled),
            };
            let live = tokio::select! {
                l = self.sup.is_live(generation) => l,
                _ = token.cancelled() => return Err(ChatError::Cancelled),
            };
            if live {
                return Ok(port);
            }
        }
        Err(ChatError::Failed("推理程序反复重启，请稍后重试".into()))
    }

    /// The model entry of the configured launch spec.
    fn model(&self) -> Result<ModelEntry, ChatError> {
        let spec = self.sup.spec().ok_or_else(|| ChatError::Failed("尚未部署模型".into()))?;
        self.manifest
            .get(&spec.model_id)
            .cloned()
            .ok_or_else(|| ChatError::Failed(format!("未知模型 {}", spec.model_id)))
    }

    /// Translate segmented pieces in order, streaming cleaned text to `emit`.
    /// `para_dir` gives a paragraph's direction (`None`: copy it unchanged);
    /// `run.port` starts the model on first use and is reused across calls.
    async fn stream_pieces(
        &self,
        model: &mut ModelEntry,
        run: &mut RunState,
        token: &CancellationToken,
        pieces: Vec<Piece>,
        para_dir: impl Fn(&[String]) -> Option<Direction>,
        emit: &(impl Fn(String) + Send + Sync),
    ) -> Result<(), ChatError> {
        for piece in pieces {
            let chunks = match piece {
                Piece::Keep(s) => {
                    emit(s);
                    continue;
                }
                Piece::Text(chunks) => chunks,
            };
            let Some(direction) = para_dir(&chunks) else {
                emit(chunks.join(" "));
                continue;
            };
            let joiner = if direction.target_is_zh() { "" } else { " " };
            for (i, chunk) in chunks.iter().enumerate() {
                if i > 0 {
                    emit(joiner.into());
                }
                let use_cache = run.use_cache;
                if let Some(hit) = use_cache.then(|| self.cache.get(chunk, direction, &model.id)).flatten() {
                    emit(hit);
                    continue;
                }
                let p = match run.port {
                    Some(p) => p,
                    None => {
                        let p = self.live_port(token).await?;
                        run.port = Some(p);
                        // A model switch may have finished while we waited: the
                        // prompt and cache key must belong to the model behind
                        // this port.
                        let running = self.model()?;
                        if running.id != model.id {
                            *model = running;
                            if let Some(hit) = use_cache.then(|| self.cache.get(chunk, direction, &model.id)).flatten() {
                                emit(hit);
                                continue;
                            }
                        }
                        p
                    }
                };
                self.sup.touch();
                let body = prompt::build(model.prompt_profile, direction, chunk);
                let mut cleaner = Cleaner::new(chunk);
                let mut shown = String::new();
                stream_chat(&self.http, p, body, token, |d| {
                    // A long CPU generation must not look idle.
                    self.sup.touch();
                    let out = cleaner.push(d);
                    if !out.is_empty() {
                        shown.push_str(&out);
                        emit(out);
                    }
                })
                .await?;
                let rest = cleaner.finish();
                if !rest.is_empty() {
                    shown.push_str(&rest);
                    emit(rest);
                }
                self.sup.touch();
                if !shown.trim().is_empty() {
                    self.cache.put(chunk, direction, &model.id, shown);
                }
            }
        }
        Ok(())
    }
}
