//! Data directories (DESIGN 4.2).

use std::path::PathBuf;

const APP_DIR: &str = "LLMTranslate";

/// `%APPDATA%\LLMTranslate`
pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR)
}

/// `%LOCALAPPDATA%\LLMTranslate`
pub fn local_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR)
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn logs_dir() -> PathBuf {
    local_dir().join("logs")
}

pub fn default_models_dir() -> PathBuf {
    local_dir().join("models")
}
