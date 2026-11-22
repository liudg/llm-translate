//! In-app updater powered by `tauri-plugin-updater` (GitHub Releases).
//!
//! Automatic checks run silently when the main window is opened, throttled by a
//! cooldown so repeated tray clicks do not hammer GitHub. Manual checks from
//! Settings bypass the cooldown and report "up to date" or errors directly.
//! Before launching the NSIS installer, the running `llama-server` child is
//! stopped so no files under `llama/` remain locked during upgrade.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Error as UpdaterError, Update, Updater, UpdaterExt};

use super::windows;
use crate::state::AppState;

/// Minimum interval between automatic checks after a successful check.
const CHECK_COOLDOWN: Duration = Duration::from_secs(30 * 60);
/// Shorter cooldown after a failed automatic check (e.g. offline).
const FAIL_COOLDOWN: Duration = Duration::from_secs(5 * 60);
/// Timeout for fetching `latest.json`.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// Per-chunk read timeout when downloading the installer (~57 MB).
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(60);
/// Progress event throttle interval.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

pub const EVENT_NAME: &str = "update-state";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePhase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installing,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    pub phase: UpdatePhase,
    /// True when the current or most recent check was triggered manually.
    pub manual: bool,
    pub available: Option<AvailableUpdate>,
    pub progress: Option<DownloadProgress>,
    pub error: Option<String>,
}

impl Default for UpdateSnapshot {
    fn default() -> Self {
        Self {
            phase: UpdatePhase::Idle,
            manual: false,
            available: None,
            progress: None,
            error: None,
        }
    }
}

struct Inner {
    snapshot: UpdateSnapshot,
    pending: Option<Update>,
    last_checked: Option<Instant>,
    last_failed: Option<Instant>,
    busy: bool,
}

#[derive(Clone)]
pub struct UpdaterState {
    inner: Arc<Mutex<Inner>>,
}

impl Default for UpdaterState {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                snapshot: UpdateSnapshot::default(),
                pending: None,
                last_checked: None,
                last_failed: None,
                busy: false,
            })),
        }
    }
}

impl UpdaterState {
    pub fn snapshot(&self) -> UpdateSnapshot {
        self.inner.lock().unwrap().snapshot.clone()
    }

    fn publish(&self, app: &AppHandle, snapshot: UpdateSnapshot) {
        self.inner.lock().unwrap().snapshot = snapshot.clone();
        let _ = app.emit(EVENT_NAME, snapshot);
    }
}

fn auto_check_enabled() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("LLMT_AUTO_UPDATE").is_some()
}

fn should_auto_check(
    busy: bool,
    has_pending: bool,
    last_checked: Option<Instant>,
    last_failed: Option<Instant>,
    now: Instant,
) -> bool {
    if busy || has_pending {
        return false;
    }
    if last_checked.is_some_and(|t| now.saturating_duration_since(t) < CHECK_COOLDOWN) {
        return false;
    }
    if last_failed.is_some_and(|t| now.saturating_duration_since(t) < FAIL_COOLDOWN) {
        return false;
    }
    true
}

fn format_pub_date(raw: Option<String>) -> Option<String> {
    let s = raw?;
    // `OffsetDateTime::to_string()` starts with `YYYY-MM-DD`.
    let date = s.get(..10)?;
    if date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-' {
        Some(date.to_string())
    } else {
        None
    }
}

fn clean_notes(notes: Option<String>) -> Option<String> {
    notes.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    })
}

fn to_available(u: &Update) -> AvailableUpdate {
    AvailableUpdate {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: clean_notes(u.body.clone()),
        pub_date: format_pub_date(u.date.map(|d| d.to_string())),
    }
}

fn friendly_error(err: &UpdaterError) -> String {
    match err {
        UpdaterError::ReleaseNotFound => "暂未找到可用发布版本".into(),
        UpdaterError::Reqwest(e) => {
            if e.is_timeout() {
                "连接更新服务器超时，请稍后重试".into()
            } else if e.is_connect() {
                "无法连接更新服务器，请检查网络连接".into()
            } else {
                format!("网络请求失败：{e}")
            }
        }
        UpdaterError::Minisign(_)
        | UpdaterError::Base64(_)
        | UpdaterError::SignatureUtf8(_)
        | UpdaterError::SignedVersionMismatch { .. }
        | UpdaterError::MissingSignedVersion => "更新包签名校验失败，已中止安装".into(),
        UpdaterError::TargetNotFound(_) | UpdaterError::TargetsNotFound(_) => {
            "当前发布版本不包含 Windows x64 安装包".into()
        }
        UpdaterError::Network(msg) => format!("下载更新失败：{msg}"),
        UpdaterError::InvalidUpdaterFormat => "安装包格式异常，已中止安装".into(),
        other => format!("更新失败：{other}"),
    }
}

fn build_updater(app: &AppHandle) -> Result<Updater, UpdaterError> {
    let builder = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .configure_client(|c| {
            c.connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
                .read_timeout(DOWNLOAD_READ_TIMEOUT)
        });

    #[cfg(debug_assertions)]
    let builder = if let Ok(url) = std::env::var("LLMT_UPDATE_ENDPOINT") {
        let parsed = url.parse().map_err(UpdaterError::UrlParse)?;
        builder.endpoints(vec![parsed])?
    } else {
        builder
    };

    builder.build()
}

/// Trigger a silent background check when the main window is opened.
pub fn on_main_shown(app: &AppHandle) {
    if !auto_check_enabled() {
        return;
    }
    let Some(state) = app.try_state::<UpdaterState>() else {
        return;
    };
    let should_run = {
        let mut g = state.inner.lock().unwrap();
        let now = Instant::now();
        if !should_auto_check(g.busy, g.pending.is_some(), g.last_checked, g.last_failed, now) {
            false
        } else {
            g.busy = true;
            g.snapshot = UpdateSnapshot {
                phase: UpdatePhase::Checking,
                manual: false,
                available: None,
                progress: None,
                error: None,
            };
            true
        }
    };
    if !should_run {
        return;
    }

    let snap = state.snapshot();
    let _ = app.emit(EVENT_NAME, snap);

    let app = app.clone();
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        run_check(app, state, false).await;
    });
}

async fn run_check(app: AppHandle, state: UpdaterState, initial_manual: bool) -> UpdateSnapshot {
    let result = match build_updater(&app) {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };

    let now = Instant::now();
    let snapshot = {
        let mut g = state.inner.lock().unwrap();
        g.busy = false;
        // A manual check may have upgraded an in-flight automatic check.
        let manual = initial_manual || g.snapshot.manual;
        match result {
            Ok(Some(update)) => {
                tracing::info!("update available: {} -> {}", update.current_version, update.version);
                let avail = to_available(&update);
                g.pending = Some(update);
                g.last_checked = Some(now);
                g.last_failed = None;
                UpdateSnapshot {
                    phase: UpdatePhase::Available,
                    manual,
                    available: Some(avail),
                    progress: None,
                    error: None,
                }
            }
            Ok(None) => {
                tracing::info!("application is up to date ({})", env!("CARGO_PKG_VERSION"));
                g.pending = None;
                g.last_checked = Some(now);
                g.last_failed = None;
                UpdateSnapshot {
                    phase: UpdatePhase::UpToDate,
                    manual,
                    available: None,
                    progress: None,
                    error: None,
                }
            }
            Err(e) => {
                if matches!(e, UpdaterError::ReleaseNotFound) {
                    tracing::info!("update check: no published release found yet");
                } else {
                    tracing::warn!("update check failed: {e}");
                }
                g.last_failed = Some(now);
                if manual {
                    g.pending = None;
                    UpdateSnapshot {
                        phase: UpdatePhase::Error,
                        manual: true,
                        available: None,
                        progress: None,
                        error: Some(friendly_error(&e)),
                    }
                } else {
                    UpdateSnapshot::default()
                }
            }
        }
    };

    state.publish(&app, snapshot.clone());
    snapshot
}

#[tauri::command]
pub fn update_status(state: State<'_, UpdaterState>) -> UpdateSnapshot {
    state.snapshot()
}

#[tauri::command]
pub async fn update_check(app: AppHandle, state: State<'_, UpdaterState>) -> Result<UpdateSnapshot, String> {
    let start_new = {
        let mut g = state.inner.lock().unwrap();
        match g.snapshot.phase {
            UpdatePhase::Downloading | UpdatePhase::Installing => {
                return Ok(g.snapshot.clone());
            }
            UpdatePhase::Checking if g.busy => {
                // Upgrade an in-flight automatic check to manual so its result is shown.
                g.snapshot.manual = true;
                false
            }
            _ => {
                g.busy = true;
                g.snapshot = UpdateSnapshot {
                    phase: UpdatePhase::Checking,
                    manual: true,
                    available: None,
                    progress: None,
                    error: None,
                };
                true
            }
        }
    };

    let snap = state.snapshot();
    let _ = app.emit(EVENT_NAME, snap.clone());

    if !start_new {
        // Wait briefly for the in-flight check to settle isn't needed; UI reacts to `update-state`.
        return Ok(snap);
    }

    Ok(run_check(app, state.inner().clone(), true).await)
}

#[tauri::command]
pub async fn update_install(
    app: AppHandle,
    updater_state: State<'_, UpdaterState>,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    let (update, avail) = {
        let mut g = updater_state.inner.lock().unwrap();
        if g.busy {
            return Err("已有更新任务正在进行".into());
        }
        let Some(update) = g.pending.clone() else {
            return Err("当前没有可安装的更新".into());
        };
        let avail = to_available(&update);
        g.busy = true;
        g.snapshot = UpdateSnapshot {
            phase: UpdatePhase::Downloading,
            manual: true,
            available: Some(avail.clone()),
            progress: Some(DownloadProgress { downloaded: 0, total: None }),
            error: None,
        };
        (update, avail)
    };

    let _ = app.emit(EVENT_NAME, updater_state.snapshot());
    tracing::info!("downloading update {} from {}", avail.version, update.download_url);

    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now() - PROGRESS_INTERVAL;
    let progress_app = app.clone();
    let progress_state = updater_state.inner().clone();
    let progress_avail = avail.clone();

    let finish_app = app.clone();
    let finish_state = updater_state.inner().clone();
    let finish_avail = avail.clone();

    let bytes = match update
        .download(
            move |chunk_len, total| {
                downloaded += chunk_len as u64;
                if last_emit.elapsed() >= PROGRESS_INTERVAL {
                    last_emit = Instant::now();
                    progress_state.publish(
                        &progress_app,
                        UpdateSnapshot {
                            phase: UpdatePhase::Downloading,
                            manual: true,
                            available: Some(progress_avail.clone()),
                            progress: Some(DownloadProgress { downloaded, total }),
                            error: None,
                        },
                    );
                }
            },
            move || {
                finish_state.publish(
                    &finish_app,
                    UpdateSnapshot {
                        phase: UpdatePhase::Installing,
                        manual: true,
                        available: Some(finish_avail),
                        progress: None,
                        error: None,
                    },
                );
            },
        )
        .await
    {
        Ok(b) => b,
        Err(e) => {
            let msg = friendly_error(&e);
            tracing::error!("update download/verify failed: {e}");
            updater_state.inner.lock().unwrap().busy = false;
            updater_state.publish(
                &app,
                UpdateSnapshot {
                    phase: UpdatePhase::Error,
                    manual: true,
                    available: Some(avail),
                    progress: None,
                    error: Some(msg.clone()),
                },
            );
            return Err(msg);
        }
    };

    // Stop active translations and llama-server so NSIS can overwrite `llama/`
    // cleanly without file-in-use conflicts, and persist main window placement.
    tracing::info!("update {} verified ({} bytes); stopping inference before install", avail.version, bytes.len());
    app_state.translator.cancel();
    app_state.sup.stop().await;
    windows::save_main_placement(&app);

    // On Windows, `Update::install` launches the NSIS installer with `/UPDATE`
    // and exits the current process (`std::process::exit(0)`).
    let install_res = tokio::task::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(|e| format!("启动安装程序失败：{e}"))
        .and_then(|r| r.map_err(|e| friendly_error(&e)));

    if let Err(msg) = install_res {
        tracing::error!("update install failed: {msg}");
        updater_state.inner.lock().unwrap().busy = false;
        updater_state.publish(
            &app,
            UpdateSnapshot {
                phase: UpdatePhase::Error,
                manual: true,
                available: Some(avail),
                progress: None,
                error: Some(msg.clone()),
            },
        );
        return Err(msg);
    }

    Ok(())
}

#[tauri::command]
pub fn update_ignore(state: State<'_, AppState>, version: String) -> Result<(), String> {
    state
        .settings
        .update(|s| s.ignored_update_version = Some(version))
        .map(|_| ())
        .map_err(|e| format!("保存设置失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_gates_repeated_auto_checks() {
        let now = Instant::now();
        assert!(should_auto_check(false, false, None, None, now));
        assert!(!should_auto_check(true, false, None, None, now));
        assert!(!should_auto_check(false, true, None, None, now));
        assert!(!should_auto_check(false, false, Some(now), None, now + Duration::from_secs(60)));
        assert!(should_auto_check(false, false, Some(now), None, now + CHECK_COOLDOWN));
        assert!(!should_auto_check(false, false, None, Some(now), now + Duration::from_secs(60)));
        assert!(should_auto_check(false, false, None, Some(now), now + FAIL_COOLDOWN));
    }

    #[test]
    fn parses_pub_date_prefix() {
        assert_eq!(
            format_pub_date(Some("2026-11-22 13:20:00.0 +00:00:00".into())),
            Some("2026-11-22".into())
        );
        assert_eq!(format_pub_date(Some("bad".into())), None);
        assert_eq!(format_pub_date(None), None);
    }

    #[test]
    fn cleans_empty_notes() {
        assert_eq!(clean_notes(Some("  \n\t ".into())), None);
        assert_eq!(clean_notes(Some("  - fix bug\n".into())), Some("- fix bug".into()));
    }
}
