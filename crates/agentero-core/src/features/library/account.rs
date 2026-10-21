//! Signed-in account id cache.
//!
//! The id is the web user id from the keychain session. This file stores only
//! that id so catalog and mark writes can find it without a network call. It
//! is not a session token, and it is never written into a vault, `sync.json`,
//! or `libraries.json`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::paths::agentero_config_dir;

/// Path segment safe to use as `{userId}` or `{markId}` under a paper folder.
pub fn safe_member_id(raw: &str) -> Option<&str> {
    let id = raw.trim();
    if id.is_empty() || id.len() > 128 || id == "." || id == ".." {
        return None;
    }
    if id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        Some(id)
    } else {
        None
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct AccountIdFile {
    #[serde(rename = "userId")]
    user_id: String,
}

pub fn account_id_path() -> PathBuf {
    agentero_config_dir().join("account.json")
}

pub fn load_at(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let parsed: AccountIdFile = serde_json::from_str(&raw).ok()?;
    safe_member_id(&parsed.user_id).map(str::to_string)
}

pub fn store_at(path: &Path, user_id: &str) -> Result<(), AppError> {
    let user_id = safe_member_id(user_id)
        .ok_or_else(|| AppError::message("Signed-in account id is not a valid path"))?
        .to_string();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::fs::json_store(path, &AccountIdFile { user_id })
}

pub fn clear_at(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn load_account_id() -> Option<String> {
    load_at(&account_id_path())
}

pub fn store_account_id(user_id: &str) -> Result<(), AppError> {
    store_at(&account_id_path(), user_id)
}

pub fn clear_account_id() -> Result<(), AppError> {
    clear_at(&account_id_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_file_roundtrip_rejects_path_tokens() {
        let dir = std::env::temp_dir().join(format!("agentero-account-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("account.json");
        store_at(&path, "user_a-1").unwrap();
        assert_eq!(load_at(&path).as_deref(), Some("user_a-1"));
        assert!(store_at(&path, "../escape").is_err());
        assert_eq!(load_at(&path).as_deref(), Some("user_a-1"));
        clear_at(&path).unwrap();
        assert!(load_at(&path).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_member_id_rejects_separators() {
        assert_eq!(safe_member_id("abc-DEF_1"), Some("abc-DEF_1"));
        assert!(safe_member_id("a/b").is_none());
        assert!(safe_member_id("..").is_none());
        assert!(safe_member_id("user.json").is_none());
    }
}
