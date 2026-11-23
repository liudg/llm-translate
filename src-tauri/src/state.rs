//! Shared application state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::app::paths;
use crate::app::settings::{Backend, Settings, SettingsStore};
use crate::deploy::downloader::Control;
use crate::deploy::manifest::{Manifest, ModelEntry};
use crate::inference::supervisor::{LaunchSpec, Supervisor};
use crate::translate::scheduler::Translator;

/// A running model download and which window started it.
pub struct ActiveDownload {
    pub ctl: Control,
    /// Started by the deployment wizard (paused when the wizard is closed).
    pub from_wizard: bool,
}

pub struct AppState {
    pub settings: SettingsStore,
    pub manifest: Arc<Manifest>,
    pub llama_exe: PathBuf,
    pub sup: Arc<Supervisor>,
    pub translator: Arc<Translator>,
    pub downloads: Mutex<HashMap<String, ActiveDownload>>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn spec_for(
        &self,
        model: &ModelEntry,
        dir: &Path,
        backend: Backend,
        device: Option<String>,
        threads: u32,
    ) -> LaunchSpec {
        LaunchSpec {
            exe: self.llama_exe.clone(),
            model_path: model.path_in(dir),
            model_id: model.id.clone(),
            backend,
            device,
            threads: threads.max(1),
            ctx: model.ctx,
            log_path: paths::logs_dir().join("llama-server.log"),
        }
    }

    /// Launch spec from saved settings, if a deployed model is present.
    pub fn spec_from_settings(&self, s: &Settings) -> Option<LaunchSpec> {
        if !s.deployed {
            return None;
        }
        let model = self.manifest.get(s.model_id.as_deref()?)?;
        if !model.is_installed(&s.models_dir) {
            return None;
        }
        Some(self.spec_for(model, &s.models_dir, s.backend, s.device.clone(), s.threads))
    }
}
