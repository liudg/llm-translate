//! Text-to-speech service backed by Microsoft Edge Read Aloud:
//! - Cross-window single active session (starting playback anywhere stops others)
//! - Bounded in-memory LRU MP3 cache (24 MiB) for instant replay
//! - Automatic language detection reusing `translate::detect`

pub mod drm;
pub mod edge;
pub mod voices;

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::Mutex;

use lru::LruCache;
use serde::{Deserialize, Serialize};
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, State, WebviewWindow};
use tokio_util::sync::CancellationToken;

use crate::state::AppState;
use crate::translate::detect::{self, Direction};
use voices::{TtsLang, VoiceCatalog, is_any_valid_voice, rate_to_percent, sanitize_voice};

const CACHE_MAX_BYTES: usize = 24 * 1024 * 1024;
const CACHE_MAX_ENTRY_BYTES: usize = 2 * 1024 * 1024;
const CACHE_MAX_ENTRIES: usize = 256;
const MAX_STOPPED_HISTORY: usize = 32;

/// Payload emitted with `tts-stop` so other windows stop playback immediately.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStopEvent {
    /// Window/session that triggered the stop stays playing; `None` stops all.
    pub except_session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsRequest {
    pub session_id: String,
    pub text: String,
    #[serde(default)]
    pub lang: Option<TtsLang>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub rate: Option<f32>,
}

type CacheKey = (String, String, i32);

struct AudioCache {
    lru: LruCache<CacheKey, Vec<u8>>,
    total_bytes: usize,
    max_bytes: usize,
}

impl AudioCache {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            lru: LruCache::new(NonZeroUsize::new(max_entries).unwrap()),
            total_bytes: 0,
            max_bytes,
        }
    }

    fn get(&mut self, key: &CacheKey) -> Option<Vec<u8>> {
        self.lru.get(key).cloned()
    }

    fn put(&mut self, key: CacheKey, audio: Vec<u8>) {
        if audio.len() > CACHE_MAX_ENTRY_BYTES || audio.len() > self.max_bytes {
            return;
        }
        if let Some(old) = self.lru.pop(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(old.len());
        }
        while self.total_bytes + audio.len() > self.max_bytes {
            let Some((_, evicted)) = self.lru.pop_lru() else { break };
            self.total_bytes = self.total_bytes.saturating_sub(evicted.len());
        }
        self.total_bytes += audio.len();
        if let Some((_, evicted)) = self.lru.push(key, audio) {
            self.total_bytes = self.total_bytes.saturating_sub(evicted.len());
        }
    }
}

struct SessionState {
    active: Option<ActiveSession>,
    stopped: VecDeque<String>,
}

struct ActiveSession {
    session_id: String,
    window: String,
    cancel: CancellationToken,
}

impl SessionState {
    fn new() -> Self {
        Self { active: None, stopped: VecDeque::with_capacity(MAX_STOPPED_HISTORY) }
    }

    fn mark_stopped(&mut self, session_id: String) {
        if self.stopped.contains(&session_id) {
            return;
        }
        if self.stopped.len() >= MAX_STOPPED_HISTORY {
            self.stopped.pop_front();
        }
        self.stopped.push_back(session_id);
    }

    fn is_stopped(&self, session_id: &str) -> bool {
        self.stopped.iter().any(|s| s == session_id)
    }
}

pub struct TtsService {
    sessions: Mutex<SessionState>,
    cache: Mutex<AudioCache>,
}

impl Default for TtsService {
    fn default() -> Self {
        Self::new()
    }
}

impl TtsService {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(SessionState::new()),
            cache: Mutex::new(AudioCache::new(CACHE_MAX_ENTRIES, CACHE_MAX_BYTES)),
        }
    }

    /// Acquire (or join) the active session for `session_id`.
    /// Returns `(cancel_token, broadcast_stop_to_others)` or `None` if already stopped.
    fn acquire(&self, window: &str, session_id: &str) -> Option<(CancellationToken, bool)> {
        let mut guard = self.sessions.lock().unwrap();
        if guard.is_stopped(session_id) {
            return None;
        }
        if let Some(cur) = &guard.active {
            if cur.session_id == session_id {
                return Some((cur.cancel.clone(), false));
            }
            let prev = guard.active.take().unwrap();
            prev.cancel.cancel();
            guard.mark_stopped(prev.session_id);
        }
        let cancel = CancellationToken::new();
        guard.active = Some(ActiveSession {
            session_id: session_id.to_string(),
            window: window.to_string(),
            cancel: cancel.clone(),
        });
        Some((cancel, true))
    }

    /// Cancel the active session if `session_id` matches (or unconditionally if `None`),
    /// and notify windows to halt playback.
    pub fn stop(&self, app: &AppHandle, session_id: Option<&str>) {
        let should_emit = {
            let mut guard = self.sessions.lock().unwrap();
            if let Some(sid) = session_id {
                guard.mark_stopped(sid.to_string());
                if guard.active.as_ref().is_some_and(|a| a.session_id == sid) {
                    if let Some(prev) = guard.active.take() {
                        prev.cancel.cancel();
                    }
                    true
                } else {
                    false
                }
            } else {
                if let Some(prev) = guard.active.take() {
                    prev.cancel.cancel();
                    guard.mark_stopped(prev.session_id);
                }
                true
            }
        };
        if should_emit {
            let _ = app.emit("tts-stop", TtsStopEvent { except_session_id: None });
        }
    }

    /// Stop playback only if `window` currently owns the active TTS session
    /// (called when a flyout/window hides so unrelated windows keep playing).
    pub fn stop_for(&self, app: &AppHandle, window: &str) {
        let stopped_id = {
            let mut guard = self.sessions.lock().unwrap();
            if guard.active.as_ref().is_some_and(|a| a.window == window) {
                let prev = guard.active.take().unwrap();
                prev.cancel.cancel();
                let id = prev.session_id.clone();
                guard.mark_stopped(prev.session_id);
                Some(id)
            } else {
                None
            }
        };
        if stopped_id.is_some() {
            let _ = app.emit("tts-stop", TtsStopEvent { except_session_id: None });
        }
    }
}

fn detect_lang(text: &str) -> TtsLang {
    match detect::detect(text) {
        Direction::Zh2En => TtsLang::Zh,
        Direction::En2Zh => TtsLang::En,
    }
}

#[tauri::command]
pub async fn tts_synthesize(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    req: TtsRequest,
) -> Result<Response, String> {
    let text = req.text.trim();
    if text.is_empty() {
        return Err("没有可朗读的内容".into());
    }

    let Some((cancel, is_new_session)) = state.tts.acquire(window.label(), &req.session_id) else {
        return Err(edge::EdgeError::Cancelled.to_string());
    };
    if is_new_session {
        let _ = app.emit(
            "tts-stop",
            TtsStopEvent { except_session_id: Some(req.session_id.clone()) },
        );
    }

    let tts_settings = state.settings.get().tts;
    let lang = req.lang.unwrap_or_else(|| detect_lang(text));
    let voice: String = match req.voice.as_deref() {
        Some(v) if is_any_valid_voice(v) => v.to_string(),
        _ => match lang {
            TtsLang::Zh => sanitize_voice(&tts_settings.zh_voice, TtsLang::Zh).to_string(),
            TtsLang::En => sanitize_voice(&tts_settings.en_voice, TtsLang::En).to_string(),
        },
    };
    let rate_pct = rate_to_percent(req.rate.unwrap_or(tts_settings.rate));
    let cache_key = (text.to_string(), voice.clone(), rate_pct);

    if let Some(hit) = state.tts.cache.lock().unwrap().get(&cache_key) {
        if cancel.is_cancelled() {
            return Err(edge::EdgeError::Cancelled.to_string());
        }
        return Ok(Response::new(hit));
    }

    tracing::debug!(
        "tts synthesize: window={} voice={} rate={}% chars={}",
        window.label(),
        voice,
        rate_pct,
        text.chars().count()
    );

    let audio = edge::synthesize(text, &voice, rate_pct, &cancel)
        .await
        .map_err(|e| {
            if !matches!(e, edge::EdgeError::Cancelled) {
                tracing::warn!("tts synthesis failed ({voice}): {e}");
            }
            e.to_string()
        })?;

    state.tts.cache.lock().unwrap().put(cache_key, audio.clone());
    if cancel.is_cancelled() {
        return Err(edge::EdgeError::Cancelled.to_string());
    }
    Ok(Response::new(audio))
}

#[tauri::command]
pub fn tts_stop(app: AppHandle, state: State<'_, AppState>, session_id: Option<String>) {
    state.tts.stop(&app, session_id.as_deref());
}

#[tauri::command]
pub fn tts_voices() -> VoiceCatalog {
    voices::catalog()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_cache_respects_byte_budget() {
        let mut c = AudioCache::new(10, 100);
        c.put(("a".into(), "v".into(), 0), vec![1; 40]);
        c.put(("b".into(), "v".into(), 0), vec![2; 40]);
        assert_eq!(c.total_bytes, 80);
        // Touch "a" so "b" becomes LRU.
        assert!(c.get(&("a".into(), "v".into(), 0)).is_some());
        c.put(("c".into(), "v".into(), 0), vec![3; 40]);
        assert_eq!(c.total_bytes, 80);
        assert!(c.get(&("b".into(), "v".into(), 0)).is_none());
        assert!(c.get(&("a".into(), "v".into(), 0)).is_some());
        assert!(c.get(&("c".into(), "v".into(), 0)).is_some());
    }

    #[test]
    fn stopped_session_cannot_reacquire() {
        let svc = TtsService::new();
        let (tok1, is_new) = svc.acquire("main", "s1").unwrap();
        assert!(is_new);
        let (_, is_new_again) = svc.acquire("main", "s1").unwrap();
        assert!(!is_new_again);
        let (_tok2, is_new_2) = svc.acquire("popup", "s2").unwrap();
        assert!(is_new_2);
        assert!(tok1.is_cancelled());
        // Late chunk for cancelled s1 must be rejected.
        assert!(svc.acquire("main", "s1").is_none());
    }
}
