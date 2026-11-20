//! Persistent settings in `%APPDATA%\LLMTranslate\settings.json`.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use super::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Vulkan,
    Cpu,
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
        Ok(())
    }
}

/// Shared settings handle; every mutation is written to disk.
#[derive(Clone)]
pub struct SettingsStore(Arc<RwLock<Settings>>);

impl SettingsStore {
    pub fn new(s: Settings) -> Self {
        Self(Arc::new(RwLock::new(s)))
    }

    pub fn get(&self) -> Settings {
        self.0.read().unwrap().clone()
    }

    pub fn update(&self, f: impl FnOnce(&mut Settings)) -> anyhow::Result<Settings> {
        let mut guard = self.0.write().unwrap();
        f(&mut guard);
        guard.save()?;
        Ok(guard.clone())
    }
}
