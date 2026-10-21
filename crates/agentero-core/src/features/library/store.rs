//! Local id → folder map for named libraries.
//!
//! `libraries.json` lives in the Agentero config directory, next to but
//! separate from `sync.json`. It stores a vault id, a display name, a folder
//! path on this computer, and the billing source last returned by the roster.
//! Session tokens never go here, and nothing in this file is written into a vault.

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
    /// `personal`, `org_grant`, or `org_shared` from `GET /api/libraries`.
    /// Remembered so a connected folder keeps the shared layout while offline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_source: Option<String>,
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
    let billing_source = map.get(id).and_then(|entry| entry.billing_source.clone());
    map.insert(
        id.to_string(),
        LocalLibraryEntry {
            path: normalize_library_path(folder),
            name: name.trim().to_string(),
            billing_source,
        },
    );
    save_at(path, &map)
}

/// Remember roster billing sources for libraries this computer has connected.
/// Entries the roster does not mention are left alone. Tokens are not stored.
pub fn remember_billing_sources_at(
    path: &Path,
    updates: &[(String, Option<String>)],
) -> Result<(), AppError> {
    let mut map = load_at(path)?;
    let mut changed = false;
    for (id, source) in updates {
        if let Some(entry) = map.get_mut(id) {
            if entry.billing_source != *source {
                entry.billing_source = source.clone();
                changed = true;
            }
        }
    }
    if changed {
        save_at(path, &map)?;
    }
    Ok(())
}

pub fn remember_billing_sources(updates: &[(String, Option<String>)]) -> Result<(), AppError> {
    remember_billing_sources_at(&libraries_path(), updates)
}

pub fn load() -> Result<BTreeMap<String, LocalLibraryEntry>, AppError> {
    load_at(&libraries_path())
}

pub fn upsert(id: &str, folder: &str, name: &str) -> Result<(), AppError> {
    upsert_at(&libraries_path(), id, folder, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_billing_source_and_upsert_keeps_it() {
        let dir = std::env::temp_dir().join(format!("agentero-libraries-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("libraries.json");
        fs::write(&path, r#"{"vault-a":{"path":"/old","name":"Old"}}"#).unwrap();
        let loaded = load_at(&path).unwrap();
        assert_eq!(loaded["vault-a"].billing_source, None);

        remember_billing_sources_at(&path, &[("vault-a".into(), Some("org_shared".into()))])
            .unwrap();
        upsert_at(&path, "vault-a", "/Users/me/shared", "Shared").unwrap();
        let loaded = load_at(&path).unwrap();
        assert_eq!(loaded["vault-a"].path, "/Users/me/shared");
        assert_eq!(
            loaded["vault-a"].billing_source.as_deref(),
            Some("org_shared")
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
