//! Model manifest `models.json` (DESIGN 3.2), shipped as a resource.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptProfile {
    IndexTranslate,
    HyMt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub sources: Vec<Source>,
    #[serde(rename = "minRamGB")]
    pub min_ram_gb: f64,
    #[serde(rename = "recommendVramGB")]
    pub recommend_vram_gb: f64,
    pub ctx: u32,
    pub prompt_profile: PromptProfile,
}

impl ModelEntry {
    pub fn path_in(&self, dir: &Path) -> PathBuf {
        dir.join(&self.file)
    }

    /// The model file exists with the expected size (full hash is checked at download time).
    pub fn is_installed(&self, dir: &Path) -> bool {
        std::fs::metadata(self.path_in(dir)).is_ok_and(|m| m.len() == self.size)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub models: Vec<ModelEntry>,
}

impl Manifest {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    pub fn get(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_manifest_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/models.json");
        let m = Manifest::load(&path).unwrap();
        assert_eq!(m.models.len(), 3);
        for e in &m.models {
            assert_eq!(e.sha256.len(), 64, "{}", e.id);
            assert!(e.sources.len() >= 2);
        }
        assert_eq!(m.get("hy-mt2-7b-q4").unwrap().prompt_profile, PromptProfile::HyMt);
    }
}
