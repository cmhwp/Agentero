//! Local id → folder map for named libraries.
//!
//! `libraries.json` lives in the Agentero config directory, next to but
//! separate from `sync.json`. It stores only a vault id, a display name, and
//! a folder path on this computer. Session tokens never go here, and nothing
//! in this file is written into a vault.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::paths::agentero_config_dir;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalLibraryEntry {
    pub path: String,
    pub name: String,
}

pub fn libraries_path() -> PathBuf {
    agentero_config_dir().join("libraries.json")
}

pub fn normalize_library_path(path: &str) -> String {
    let mut trimmed = path.trim().to_string();
    while trimmed.len() > 1 && (trimmed.ends_with('/') || trimmed.ends_with('\\')) {
        trimmed.pop();
    }
    trimmed
}

pub fn load_at(path: &Path) -> Result<BTreeMap<String, LocalLibraryEntry>, AppError> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    match serde_json::from_str(&raw) {
        Ok(map) => Ok(map),
        Err(error) => {
            log::warn!(target: "agentero::library", "invalid libraries.json: {error}");
            Ok(BTreeMap::new())
        }
    }
}

pub fn save_at(path: &Path, map: &BTreeMap<String, LocalLibraryEntry>) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::fs::json_store(path, map)
}

pub fn upsert_at(path: &Path, id: &str, folder: &str, name: &str) -> Result<(), AppError> {
    let mut map = load_at(path)?;
    map.insert(
        id.to_string(),
        LocalLibraryEntry {
            path: normalize_library_path(folder),
            name: name.trim().to_string(),
        },
    );
    save_at(path, &map)
}

pub fn load() -> Result<BTreeMap<String, LocalLibraryEntry>, AppError> {
    load_at(&libraries_path())
}

pub fn upsert(id: &str, folder: &str, name: &str) -> Result<(), AppError> {
    upsert_at(&libraries_path(), id, folder, name)
}
