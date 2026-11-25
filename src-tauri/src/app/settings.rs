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

/// Current settings format.
/// 2: the selection blacklist gained default Android emulators.
/// 3: UIA + cursor gating replaced the default emulator blacklist; added
///    blacklist/whitelist mode and removed double-click / Shift+click options.
const VERSION: u32 = 3;

/// Legacy defaults added by v2, stripped once on upgrade to v3 so existing
/// users get a clean list while keeping any entries they added manually.
/// Because v2 did not store provenance, exact-case matches of these 8 defaults
/// are treated as v2-injected, while differently-cased entries (e.g. a v1 user
/// who manually added `nox.exe`, which v2 kept verbatim) are preserved.
const LEGACY_DEFAULT_BLACKLIST: [&str; 8] = [
    "MuMuNxDevice.exe",
    "MuMuPlayer.exe",
    "NemuPlayer.exe",
    "dnplayer.exe",
    "HD-Player.exe",
    "Nox.exe",
    "MEmu.exe",
    "scrcpy.exe",
];

/// Canonical process-name key shared by filtering, adding, and removal
/// (trimmed Unicode lowercase, matching Windows case-insensitive file names).
pub fn normalize_exe(exe: &str) -> String {
    exe.trim().to_lowercase()
}

fn same_exe(a: &str, b: &str) -> bool {
    normalize_exe(a) == normalize_exe(b)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode {
    /// Active in every app except those in `blacklist`.
    #[default]
    Blacklist,
    /// Active only in apps listed in `whitelist`.
    Whitelist,
}

/// Selection translation (DESIGN 5.5): drag to select text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SelectionSettings {
    pub enabled: bool,
    pub mode: FilterMode,
    /// Process names (`xxx.exe`) excluded in `Blacklist` mode; compared
    /// case-insensitively.
    pub blacklist: Vec<String>,
    /// Process names (`xxx.exe`) allowed in `Whitelist` mode; compared
    /// case-insensitively.
    pub whitelist: Vec<String>,
}

impl SelectionSettings {
    fn list_mut(&mut self, mode: FilterMode) -> &mut Vec<String> {
        match mode {
            FilterMode::Blacklist => &mut self.blacklist,
            FilterMode::Whitelist => &mut self.whitelist,
        }
    }

    pub fn add_app(&mut self, mode: FilterMode, exe: &str) {
        let exe = exe.trim();
        if exe.is_empty() {
            return;
        }
        let list = self.list_mut(mode);
        if !list.iter().any(|n| same_exe(n, exe)) {
            list.push(exe.to_string());
        }
    }

    pub fn remove_app(&mut self, mode: FilterMode, exe: &str) {
        let exe = exe.trim();
        if exe.is_empty() {
            return;
        }
        self.list_mut(mode).retain(|n| !same_exe(n, exe));
    }

    /// Stop triggering in `exe`: adds it to `blacklist` in Blacklist mode, or
    /// removes it from `whitelist` in Whitelist mode.
    pub fn exclude_app(&mut self, exe: &str) {
        match self.mode {
            FilterMode::Blacklist => self.add_app(FilterMode::Blacklist, exe),
            FilterMode::Whitelist => self.remove_app(FilterMode::Whitelist, exe),
        }
    }
}

impl Default for SelectionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: FilterMode::Blacklist,
            blacklist: Vec::new(),
            whitelist: Vec::new(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: VERSION,
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
        let mut s: Self = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("settings.json is invalid ({e}); using defaults");
                Self::default()
            }),
            Err(_) => return Self::default(),
        };
        if s.migrate()
            && let Err(e) = s.save()
        {
            tracing::warn!("saving migrated settings failed: {e}");
        }
        s
    }

    /// Bring settings from an older version up to date, once. Returns
    /// whether anything changed.
    fn migrate(&mut self) -> bool {
        if self.version >= VERSION {
            return false;
        }
        if self.version == 2 {
            // Exact-case match removes v2-injected defaults while preserving
            // provably manual entries that kept user casing (e.g. `nox.exe`).
            self.selection.blacklist.retain(|n| !LEGACY_DEFAULT_BLACKLIST.contains(&n.as_str()));
        }
        tracing::info!("settings migrated from version {} to {VERSION}", self.version);
        self.version = VERSION;
        true
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

    /// `update` for commands: the error as UI text.
    pub fn save(&self, f: impl FnOnce(&mut Settings)) -> Result<(), String> {
        self.update(f).map(|_| ()).map_err(|e| format!("保存设置失败：{e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_cleans_legacy_defaults_and_keeps_custom() {
        let mut s: Settings = serde_json::from_str(
            r#"{"version":2,"selection":{"drag":true,"doubleClick":true,"shiftClick":false,"blacklist":["WINWORD.EXE","MuMuNxDevice.exe","nox.exe","Nox.exe","Code.exe","scrcpy.exe"]}}"#,
        )
        .unwrap();
        assert!(s.migrate());
        assert_eq!(s.version, VERSION);
        assert_eq!(s.selection.mode, FilterMode::Blacklist);
        // Exact-case v2 defaults are removed; differently-cased manual `nox.exe`
        // from v1 is preserved alongside other custom apps.
        assert_eq!(s.selection.blacklist, vec!["WINWORD.EXE", "nox.exe", "Code.exe"]);
        assert!(s.selection.whitelist.is_empty());

        // User re-adding a legacy app later keeps it.
        s.selection.blacklist.push("scrcpy.exe".into());
        assert!(!s.migrate());
        assert_eq!(s.selection.blacklist, vec!["WINWORD.EXE", "nox.exe", "Code.exe", "scrcpy.exe"]);
    }

    #[test]
    fn v1_keeps_existing_blacklist_unchanged() {
        let mut s: Settings =
            serde_json::from_str(r#"{"version":1,"selection":{"blacklist":["WINWORD.EXE","nox.exe"]}}"#).unwrap();
        assert!(s.migrate());
        assert_eq!(s.version, VERSION);
        assert_eq!(s.selection.blacklist, vec!["WINWORD.EXE", "nox.exe"]);
    }

    #[test]
    fn fresh_settings_start_clean() {
        let s = Settings::default();
        assert_eq!(s.version, VERSION);
        assert_eq!(s.selection.mode, FilterMode::Blacklist);
        assert!(s.selection.blacklist.is_empty());
        assert!(s.selection.whitelist.is_empty());
    }

    #[test]
    fn exclude_app_respects_filter_mode_and_unicode_case() {
        let mut sel = SelectionSettings::default();
        sel.exclude_app("Code.exe");
        sel.exclude_app("code.exe");
        sel.exclude_app("café.exe");
        sel.exclude_app("CAFÉ.EXE");
        assert_eq!(sel.blacklist, vec!["Code.exe", "café.exe"]);

        sel.mode = FilterMode::Whitelist;
        sel.whitelist = vec!["chrome.exe".into(), "Code.exe".into(), "café.exe".into()];
        sel.exclude_app("code.exe");
        sel.exclude_app("CAFÉ.exe");
        assert_eq!(sel.whitelist, vec!["chrome.exe"]);
    }
}
