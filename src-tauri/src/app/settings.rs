//! Persistent settings in `%APPDATA%\LLMTranslate\settings.json`.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Vulkan,
    Cpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub version: u32,
    /// Deployment wizard finished successfully at least once.
    pub deployed: bool,
    pub model_id: Option<String>,
    pub models_dir: PathBuf,
    pub backend: Backend,
    /// llama.cpp device name such as `Vulkan0`; `None` lets llama.cpp choose.
    pub device: Option<String>,
    pub threads: u32,
    /// Release the model after this many idle minutes; 0 keeps it loaded.
    pub idle_release_minutes: u32,
    pub theme: Theme,
    pub selection: SelectionSettings,
    pub hotkeys: HotkeySettings,
    /// The "still running in the tray" notification was shown once.
    pub tray_hint_shown: bool,
    /// Version tag (e.g. `0.2.0`) the user chose to ignore in the update banner.
    pub ignored_update_version: Option<String>,
}

/// Global shortcuts as display strings such as `Ctrl+Alt+Q` (DESIGN 5.7);
/// `None` means unbound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HotkeySettings {
    pub mini: Option<String>,
    pub shot: Option<String>,
    pub selection: Option<String>,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self { mini: Some("Alt+Q".into()), shot: Some("Alt+W".into()), selection: None }
    }
}

/// Selection translation (DESIGN 5.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SelectionSettings {
    pub enabled: bool,
    /// Trigger gestures.
    pub drag: bool,
    pub double_click: bool,
    pub shift_click: bool,
    /// Process names (`xxx.exe`) where the icon never shows; compared
    /// case-insensitively.
    pub blacklist: Vec<String>,
}

impl Default for SelectionSettings {
    fn default() -> Self {
        Self { enabled: true, drag: true, double_click: true, shift_click: true, blacklist: Vec::new() }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            deployed: false,
            model_id: None,
            models_dir: paths::default_models_dir(),
            backend: Backend::Cpu,
            device: None,
            threads: 4,
            idle_release_minutes: 0,
            theme: Theme::System,
            selection: SelectionSettings::default(),
            hotkeys: HotkeySettings::default(),
            tray_hint_shown: false,
            ignored_update_version: None,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let path = paths::settings_file();
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("settings.json is invalid ({e}); using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = paths::settings_file();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let data = serde_json::to_vec_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &data)?;
        if let Err(e) = std::fs::rename(&tmp, &path) {
            // Rename can fail in redirected/virtualized folders; write in place.
            tracing::warn!("settings rename failed ({e}); writing directly");
            let _ = std::fs::remove_file(&tmp);
            std::fs::write(&path, &data)?;
        }
        sync_models_dir_registry(&self.models_dir);
        Ok(())
    }
}

/// Mirror `models_dir` to `HKCU\Software\LLMTranslate\ModelsDir` (native UTF-16)
/// so the NSIS uninstaller can locate downloaded models reliably even when the
/// user chose a custom or non-ASCII directory.
pub fn sync_models_dir_registry(dir: &std::path::Path) {
    let r = windows_registry::CURRENT_USER
        .create(r"Software\LLMTranslate")
        .and_then(|k| k.set_string("ModelsDir", &dir.to_string_lossy()));
    if let Err(e) = r {
        tracing::warn!("mirroring modelsDir to registry failed: {e}");
    }
}

/// Shared settings handle; every mutation is written to disk and published to
/// subscribers ([`SettingsStore::subscribe`]).
#[derive(Clone)]
pub struct SettingsStore {
    inner: Arc<RwLock<Settings>>,
    changed: Arc<watch::Sender<Settings>>,
}

impl SettingsStore {
    pub fn new(s: Settings) -> Self {
        let changed = Arc::new(watch::channel(s.clone()).0);
        Self { inner: Arc::new(RwLock::new(s)), changed }
    }

    pub fn get(&self) -> Settings {
        self.inner.read().unwrap().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Settings> {
        self.changed.subscribe()
    }

    pub fn update(&self, f: impl FnOnce(&mut Settings)) -> anyhow::Result<Settings> {
        let mut guard = self.inner.write().unwrap();
        f(&mut guard);
        let saved = guard.save();
        let snapshot = guard.clone();
        drop(guard);
        // Publish even when saving failed: the in-memory value did change.
        self.changed.send_replace(snapshot.clone());
        saved?;
        Ok(snapshot)
    }
}
