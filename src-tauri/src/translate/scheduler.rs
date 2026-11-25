//! Request scheduling (DESIGN 5.3): one queue; a new request cancels the one in
//! progress; segments are translated in order and streamed out.

use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    current: Mutex<(Option<String>, CancellationToken)>,
    serial: tokio::sync::Mutex<()>,
}

/// A paragraph's direction: the requested one, or detected per paragraph so
/// mixed Chinese/English text works. `None`: no letters, copied unchanged.
fn paragraph_direction(requested: Option<Direction>) -> impl Fn(&[String]) -> Option<Direction> {
    move |chunks| detect_paragraph(&chunks.concat()).map(|d| requested.unwrap_or(d))
}

impl Translator {
    pub fn new(sup: Arc<Supervisor>, manifest: Arc<Manifest>) -> Self {
        Self {
            sup,
            manifest,
            http: local_client(),
            cache: Cache::new(500),
            current: Mutex::new((None, CancellationToken::new())),
            serial: tokio::sync::Mutex::new(()),
        }
    }

    /// Cancel whatever is running, regardless of owner.
    pub fn cancel(&self) {
        let mut cur = self.current.lock().unwrap();
        cur.0 = None;
        cur.1.cancel();
    }

    /// Cancel the running request only if it belongs to `owner` (window label),
    /// so a hiding flyout cannot interrupt a newer translation in another window.
    pub fn cancel_for(&self, owner: &str) {
        let mut cur = self.current.lock().unwrap();
        if cur.0.as_deref() == Some(owner) {
            cur.0 = None;
            cur.1.cancel();
        }
    }

    /// Cancel the running request and hand out the token for a new one owned by `owner`.
    fn begin(&self, owner: &str) -> CancellationToken {
        let mut cur = self.current.lock().unwrap();
        cur.1.cancel();
        let token = CancellationToken::new();
        *cur = (Some(owner.to_string()), token.clone());
        token
    }

    /// Wait for the previous request to wind down (it was just cancelled).
    async fn queue(&self, token: &CancellationToken) -> Result<tokio::sync::MutexGuard<'_, ()>, ChatError> {
        tokio::select! {
            g = self.serial.lock() => Ok(g),
            _ = token.cancelled() => Err(ChatError::Cancelled),
        }
    }

    /// Translate, streaming events to `sink`. Cancels any earlier request.
    pub async fn translate(&self, owner: &str, req: TranslateRequest, sink: impl Fn(TranslateEvent) + Send + Sync) {
        let token = self.begin(owner);
        let event = match self.run(&req, &token, &sink).await {
            Ok(ev) => ev,
            Err(ChatError::Cancelled) => TranslateEvent::Cancelled,
            Err(ChatError::Failed(m)) => TranslateEvent::Error { message: m },
        };
        sink(event);
    }

    /// Translate several texts in order. Cancels any earlier request; a later
    /// request cancels this whole batch.
    pub async fn translate_batch(&self, owner: &str, req: BatchRequest, sink: impl Fn(BatchEvent) + Send + Sync) {
        let token = self.begin(owner);
        let event = match self.run_batch(&req, &token, &sink).await {
            Ok(ev) => ev,
            Err(ChatError::Cancelled) => BatchEvent::Cancelled,
            Err(ChatError::Failed(m)) => BatchEvent::Error { message: m },
        };
        sink(event);
    }

    /// Deployment self-test: translate `text` with the process on `port`
    /// directly, bypassing the queue and the cache. If that process goes
    /// away meanwhile, this fails.
    pub async fn self_test(&self, text: &str, model: &ModelEntry, port: u16) -> Result<String, String> {
        let body = prompt::build(model.prompt_profile, detect(text), text);
        let mut cleaner = Cleaner::new(text);
        let mut out = String::new();
        stream_chat(&self.http, port, body, &CancellationToken::new(), |d| out.push_str(&cleaner.push(d)))
            .await
            .map_err(|e| e.to_string())?;
        out.push_str(&cleaner.finish());
        Ok(out)
    }

    async fn run(
        &self,
        req: &TranslateRequest,
        token: &CancellationToken,
        sink: &(impl Fn(TranslateEvent) + Send + Sync),
    ) -> Result<TranslateEvent, ChatError> {
        let _guard = self.queue(token).await?;
        let t0 = Instant::now();
        // The overall direction is only the label.
        let overall = req.direction.unwrap_or_else(|| detect(&req.text));
        let para_dir = paragraph_direction(req.direction);
        let pieces = segment(&req.text);
        let mixed = pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Text(c) => para_dir(c),
                Piece::Keep(_) => None,
            })
            .any(|d| d != overall);
        sink(TranslateEvent::Start { direction: overall, mixed });
        let mut port = None;
        self.stream_pieces(&mut port, token, pieces, para_dir, &|text| sink(TranslateEvent::Delta { text })).await?;
        Ok(TranslateEvent::Done { elapsed_ms: t0.elapsed().as_millis() as u64 })
    }

    async fn run_batch(
        &self,
        req: &BatchRequest,
        token: &CancellationToken,
        sink: &(impl Fn(BatchEvent) + Send + Sync),
    ) -> Result<BatchEvent, ChatError> {
        let _guard = self.queue(token).await?;
        let t0 = Instant::now();
        sink(BatchEvent::Start);
        let mut port = None;
        for (index, text) in req.items.iter().enumerate() {
            let para_dir = paragraph_direction(req.direction);
            self.stream_pieces(&mut port, token, segment(text), para_dir, &|text| sink(BatchEvent::Delta { index, text }))
                .await?;
            sink(BatchEvent::ItemDone { index });
        }
        Ok(BatchEvent::Done { elapsed_ms: t0.elapsed().as_millis() as u64 })
    }

    /// Port of a ready model. The start runs in its own task: a cancelled
    /// request only stops waiting (dropping a half-done start would leave
    /// llama-server running with the status stuck at Loading).
    async fn ready_port(&self, token: &CancellationToken) -> Result<u16, ChatError> {
        let sup = self.sup.clone();
        let start = tauri::async_runtime::spawn(async move { sup.ensure_ready().await });
        tokio::select! {
            r = start => r.map_err(|e| ChatError::Failed(format!("启动推理失败：{e}")))?.map_err(ChatError::Failed),
            _ = token.cancelled() => Err(ChatError::Cancelled),
        }
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
    /// `para_dir` gives a paragraph's direction (`None`: copy it unchanged).
    /// The model is started on first need; `port` keeps it across calls.
    async fn stream_pieces(
        &self,
        port: &mut Option<u16>,
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
                let mut model = self.model()?;
                if let Some(hit) = self.cache.get(chunk, direction, &model.id) {
                    emit(hit);
                    continue;
                }
                let p = match *port {
                    Some(p) => p,
                    None => {
                        let p = self.ready_port(token).await?;
                        *port = Some(p);
                        // A model switch may have finished while it started:
                        // prompt and cache key belong to the model behind `p`.
                        model = self.model()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_translator() -> Translator {
        let sup = Supervisor::new(Arc::new(|_| {}));
        let manifest = Arc::new(Manifest { version: 1, models: Vec::new() });
        Translator::new(sup, manifest)
    }

    #[test]
    fn cancel_for_only_cancels_matching_owner() {
        let t = dummy_translator();
        let popup_tok = t.begin("popup");
        assert!(!popup_tok.is_cancelled());

        // Main takes over: popup's token is cancelled immediately.
        let main_tok = t.begin("main");
        assert!(popup_tok.is_cancelled());
        assert!(!main_tok.is_cancelled());

        // Late cancel from hiding popup must NOT cancel main's active request.
        t.cancel_for("popup");
        assert!(!main_tok.is_cancelled());

        // Matching owner cancels cleanly.
        t.cancel_for("main");
        assert!(main_tok.is_cancelled());

        // Global cancel cancels regardless of owner.
        let mini_tok = t.begin("mini");
        assert!(!mini_tok.is_cancelled());
        t.cancel();
        assert!(mini_tok.is_cancelled());
    }
}
