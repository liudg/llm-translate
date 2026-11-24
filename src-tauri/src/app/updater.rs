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
/// Shorter cooldown after a failed check (e.g. offline).
const FAIL_COOLDOWN: Duration = Duration::from_secs(5 * 60);
/// Timeout for fetching `latest.json`.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// Per-chunk read timeout when downloading the installer (~57 MB).
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(60);
/// Progress event throttle interval.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

const EVENT_NAME: &str = "update-state";

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installing,
    Error,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    pub phase: UpdatePhase,
    /// True when the current or most recent check was triggered manually.
    pub manual: bool,
    pub available: Option<AvailableUpdate>,
    pub progress: Option<DownloadProgress>,
    pub error: Option<String>,
}

impl UpdateSnapshot {
    fn new(phase: UpdatePhase, manual: bool, available: Option<AvailableUpdate>) -> Self {
        Self { phase, manual, available, progress: None, error: None }
    }

    fn failed(manual: bool, available: Option<AvailableUpdate>, error: String) -> Self {
        Self { error: Some(error), ..Self::new(UpdatePhase::Error, manual, available) }
    }

    fn downloading(available: AvailableUpdate, progress: DownloadProgress) -> Self {
        Self { progress: Some(progress), ..Self::new(UpdatePhase::Downloading, true, Some(available)) }
    }

    /// A check, download or install is running.
    fn busy(&self) -> bool {
        matches!(self.phase, UpdatePhase::Checking | UpdatePhase::Downloading | UpdatePhase::Installing)
    }
}

#[derive(Default)]
struct Inner {
    snapshot: UpdateSnapshot,
    pending: Option<Update>,
    /// No automatic check before this.
    next_auto_check: Option<Instant>,
}

#[derive(Clone, Default)]
pub struct UpdaterState {
    inner: Arc<Mutex<Inner>>,
}

impl UpdaterState {
    pub fn snapshot(&self) -> UpdateSnapshot {
        self.inner.lock().unwrap().snapshot.clone()
    }

    fn set(&self, app: &AppHandle, snapshot: UpdateSnapshot) {
        self.inner.lock().unwrap().snapshot = snapshot.clone();
        let _ = app.emit(EVENT_NAME, snapshot);
    }
}

fn auto_check_enabled() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("LLMT_AUTO_UPDATE").is_some()
}

fn should_auto_check(busy: bool, has_pending: bool, next: Option<Instant>, now: Instant) -> bool {
    !busy && !has_pending && next.is_none_or(|t| now >= t)
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
    notes.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
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
        .configure_client(|c| c.connect_timeout(DOWNLOAD_CONNECT_TIMEOUT).read_timeout(DOWNLOAD_READ_TIMEOUT));

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
    {
        let mut g = state.inner.lock().unwrap();
        if !should_auto_check(g.snapshot.busy(), g.pending.is_some(), g.next_auto_check, Instant::now()) {
            return;
        }
        g.snapshot = UpdateSnapshot::new(UpdatePhase::Checking, false, None);
    }
    let _ = app.emit(EVENT_NAME, state.snapshot());
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
        // A manual check may have upgraded an in-flight automatic check.
        let manual = initial_manual || g.snapshot.manual;
        let snapshot = match result {
            Ok(update) => {
                g.next_auto_check = Some(now + CHECK_COOLDOWN);
                let available = update.as_ref().map(to_available);
                match &update {
                    Some(u) => tracing::info!("update available: {} -> {}", u.current_version, u.version),
                    None => tracing::info!("application is up to date ({})", env!("CARGO_PKG_VERSION")),
                }
                let phase = if update.is_some() { UpdatePhase::Available } else { UpdatePhase::UpToDate };
                g.pending = update;
                UpdateSnapshot::new(phase, manual, available)
            }
            Err(e) => {
                if matches!(e, UpdaterError::ReleaseNotFound) {
                    tracing::info!("update check: no published release found yet");
                } else {
                    tracing::warn!("update check failed: {e}");
                }
                let retry = now + FAIL_COOLDOWN;
                g.next_auto_check = Some(g.next_auto_check.map_or(retry, |t| t.max(retry)));
                if manual {
                    g.pending = None;
                    UpdateSnapshot::failed(true, None, friendly_error(&e))
                } else {
                    UpdateSnapshot::default()
                }
            }
        };
        g.snapshot = snapshot.clone();
        snapshot
    };
    let _ = app.emit(EVENT_NAME, snapshot.clone());
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
            UpdatePhase::Downloading | UpdatePhase::Installing => return Ok(g.snapshot.clone()),
            // Upgrade an in-flight automatic check to manual so its result is shown.
            UpdatePhase::Checking => {
                g.snapshot.manual = true;
                false
            }
            _ => {
                g.snapshot = UpdateSnapshot::new(UpdatePhase::Checking, true, None);
                true
            }
        }
    };
    let snap = state.snapshot();
    let _ = app.emit(EVENT_NAME, snap.clone());
    if !start_new {
        return Ok(snap); // the running check publishes its result
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
        if g.snapshot.busy() {
            return Err("已有更新任务正在进行".into());
        }
        let Some(update) = g.pending.clone() else {
            return Err("当前没有可安装的更新".into());
        };
        let avail = to_available(&update);
        g.snapshot = UpdateSnapshot::downloading(avail.clone(), DownloadProgress { downloaded: 0, total: None });
        (update, avail)
    };

    let _ = app.emit(EVENT_NAME, updater_state.snapshot());
    tracing::info!("downloading update {} from {}", avail.version, update.download_url);

    let state = updater_state.inner().clone();
    let fail = |msg: String| {
        state.set(&app, UpdateSnapshot::failed(true, Some(avail.clone()), msg.clone()));
        Err(msg)
    };

    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now() - PROGRESS_INTERVAL;
    let (progress_app, progress_state, progress_avail) = (app.clone(), state.clone(), avail.clone());
    let (finish_app, finish_state, finish_avail) = (app.clone(), state.clone(), avail.clone());
    let bytes = match update
        .download(
            move |chunk_len, total| {
                downloaded += chunk_len as u64;
                if last_emit.elapsed() >= PROGRESS_INTERVAL {
                    last_emit = Instant::now();
                    let progress = DownloadProgress { downloaded, total };
                    progress_state.set(&progress_app, UpdateSnapshot::downloading(progress_avail.clone(), progress));
                }
            },
            move || {
                finish_state
                    .set(&finish_app, UpdateSnapshot::new(UpdatePhase::Installing, true, Some(finish_avail)));
            },
        )
        .await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("update download/verify failed: {e}");
            return fail(friendly_error(&e));
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
        return fail(msg);
    }
    Ok(())
}

#[tauri::command]
pub fn update_ignore(state: State<'_, AppState>, version: String) -> Result<(), String> {
    state.settings.save(|s| s.ignored_update_version = Some(version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_gates_repeated_auto_checks() {
        let now = Instant::now();
        assert!(should_auto_check(false, false, None, now));
        assert!(!should_auto_check(true, false, None, now));
        assert!(!should_auto_check(false, true, None, now));
        assert!(!should_auto_check(false, false, Some(now + CHECK_COOLDOWN), now + Duration::from_secs(60)));
        assert!(should_auto_check(false, false, Some(now + CHECK_COOLDOWN), now + CHECK_COOLDOWN));
    }

    #[test]
    fn parses_pub_date_prefix() {
        assert_eq!(format_pub_date(Some("2026-11-22 13:20:00.0 +00:00:00".into())), Some("2026-11-22".into()));
        assert_eq!(format_pub_date(Some("bad".into())), None);
        assert_eq!(format_pub_date(None), None);
    }

    #[test]
    fn cleans_empty_notes() {
        assert_eq!(clean_notes(Some("  \n\t ".into())), None);
        assert_eq!(clean_notes(Some("  - fix bug\n".into())), Some("- fix bug".into()));
    }
}
