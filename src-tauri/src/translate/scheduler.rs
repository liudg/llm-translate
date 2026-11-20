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
use crate::deploy::manifest::Manifest;
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
    Start { direction: Direction },
    Delta { text: String },
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
        }
    }

    /// Cancel whatever is running.
    pub fn cancel(&self) {
        self.current.lock().unwrap().cancel();
    }

    /// Translate, streaming events to `sink`. Cancels any earlier request.
    pub async fn translate(&self, req: TranslateRequest, sink: impl Fn(TranslateEvent) + Send + Sync) {
        let token = {
            let mut cur = self.current.lock().unwrap();
            cur.cancel();
            *cur = CancellationToken::new();
            cur.clone()
        };
        let event = match self.run(&req, &token, &sink).await {
            Ok(ev) => ev,
            Err(ChatError::Cancelled) => TranslateEvent::Cancelled,
            Err(ChatError::Failed(m)) => TranslateEvent::Error { message: m },
        };
        sink(event);
    }

    /// Collect a whole translation (self-test). Cancels any earlier request.
    pub async fn translate_text(&self, text: &str, direction: Option<Direction>) -> Result<String, String> {
        let out = Mutex::new(String::new());
        let err = Mutex::new(None);
        self.translate(TranslateRequest { text: text.into(), direction }, |ev| match ev {
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
        sink(TranslateEvent::Start { direction: overall });

        // Paragraph direction; None = no letters, copied through unchanged.
        let para_dir = |chunks: &[String]| {
            let text = chunks.concat();
            detect_paragraph(&text).map(|d| req.direction.unwrap_or(d))
        };
        let pieces = segment(&req.text);
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

        let spec = self.sup.spec().await.ok_or_else(|| ChatError::Failed("尚未部署模型".into()))?;
        let model = self
            .manifest
            .get(&spec.model_id)
            .ok_or_else(|| ChatError::Failed(format!("未知模型 {}", spec.model_id)))?
            .clone();

        let mut port = None;
        for piece in pieces {
            let chunks = match piece {
                Piece::Keep(s) => {
                    sink(TranslateEvent::Delta { text: s });
                    continue;
                }
                Piece::Text(chunks) => chunks,
            };
            let Some(direction) = para_dir(&chunks) else {
                sink(TranslateEvent::Delta { text: chunks.join(" ") });
                continue;
            };
            let joiner = if direction.target_is_zh() { "" } else { " " };
            for (i, chunk) in chunks.iter().enumerate() {
                if i > 0 {
                    sink(TranslateEvent::Delta { text: joiner.into() });
                }
                if let Some(hit) = self.cache.get(chunk, direction, &model.id) {
                    sink(TranslateEvent::Delta { text: hit });
                    continue;
                }
                let p = match port {
                    Some(p) => p,
                    None => {
                        let p = tokio::select! {
                            r = self.sup.ensure_ready() => r.map_err(ChatError::Failed)?,
                            _ = token.cancelled() => return Err(ChatError::Cancelled),
                        };
                        port = Some(p);
                        p
                    }
                };
                self.sup.touch();
                let body = prompt::build(model.prompt_profile, direction, chunk);
                let mut cleaner = Cleaner::new(chunk);
                let mut shown = String::new();
                stream_chat(&self.http, p, body, token, |d| {
                    let out = cleaner.push(d);
                    if !out.is_empty() {
                        shown.push_str(&out);
                        sink(TranslateEvent::Delta { text: out });
                    }
                })
                .await?;
                let rest = cleaner.finish();
                if !rest.is_empty() {
                    shown.push_str(&rest);
                    sink(TranslateEvent::Delta { text: rest });
                }
                self.sup.touch();
                if !shown.trim().is_empty() {
                    self.cache.put(chunk, direction, &model.id, shown);
                }
            }
        }
        Ok(TranslateEvent::Done { elapsed_ms: t0.elapsed().as_millis() as u64 })
    }
}
