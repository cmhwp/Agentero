//! Per-member read flag and reading position for `org_shared` libraries.
//!
//! `papers/<id>/.readers/{userId}.json` belongs to one signed-in account.
//! `userId` is the web user id, stable across that account's machines.
//! Personal and `org_grant` libraries do not use this directory.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;
use crate::features::library::{load, safe_member_id, MemberLayout, ORG_SHARED};
use crate::fs::sanitize_vault_rel;

pub const READERS_DIR: &str = ".readers";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ReaderFile {
    #[serde(default)]
    is_read: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    page: Option<u32>,
}

fn paper_dir(vault: &Path, rel: &str) -> Result<PathBuf, AppError> {
    let rel = sanitize_vault_rel(rel).map_err(AppError::message)?;
    Ok(vault.join(rel))
}

pub fn reader_path(vault: &Path, rel: &str, user_id: &str) -> Result<PathBuf, AppError> {
    let user_id = safe_member_id(user_id)
        .ok_or_else(|| AppError::message("Signed-in account id is not a valid path"))?;
    Ok(paper_dir(vault, rel)?
        .join(READERS_DIR)
        .join(format!("{user_id}.json")))
}

fn read_file(path: &Path) -> Option<ReaderFile> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_file(path: &Path, file: &ReaderFile) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut body = serde_json::to_vec_pretty(file)?;
    body.push(b'\n');
    crate::fs::atomic_write(path, &body)?;
    Ok(())
}

pub fn read_is_read(vault: &Path, rel: &str, user_id: &str) -> bool {
    reader_path(vault, rel, user_id)
        .ok()
        .and_then(|path| read_file(&path))
        .is_some_and(|file| file.is_read)
}

pub fn reading_page(vault: &Path, rel: &str, user_id: &str) -> u32 {
    reader_path(vault, rel, user_id)
        .ok()
        .and_then(|path| read_file(&path))
        .and_then(|file| file.page)
        .filter(|page| *page > 1)
        .unwrap_or(0)
}

pub fn set_read_flag(
    vault: &Path,
    rel: &str,
    user_id: &str,
    is_read: bool,
) -> Result<(), AppError> {
    let path = reader_path(vault, rel, user_id)?;
    let mut file = read_file(&path).unwrap_or(ReaderFile {
        is_read: false,
        page: None,
    });
    file.is_read = is_read;
    write_file(&path, &file)
}

pub fn set_reading_page(vault: &Path, rel: &str, user_id: &str, page: u32) -> Result<(), AppError> {
    let path = reader_path(vault, rel, user_id)?;
    let mut file = read_file(&path).unwrap_or(ReaderFile {
        is_read: false,
        page: None,
    });
    file.page = (page > 1).then_some(page);
    write_file(&path, &file)
}

/// Move a legacy group `is_read` out of `metadata.json` into this member's
/// reader file, once. An existing reader file keeps its own flag. Bibliographic
/// fields stay. No-op when the key is already gone.
pub fn migrate_paper(vault: &Path, rel: &str, user_id: &str) -> Result<bool, AppError> {
    let meta_path = paper_dir(vault, rel)?.join(super::sidecar::SIDECAR_FILE);
    let Ok(text) = fs::read_to_string(&meta_path) else {
        return Ok(false);
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        return Ok(false);
    };
    let Some(obj) = value.as_object_mut() else {
        return Ok(false);
    };
    let Some(flag) = obj.remove("is_read") else {
        return Ok(false);
    };
    let reader = reader_path(vault, rel, user_id)?;
    if read_file(&reader).is_none() {
        write_file(
            &reader,
            &ReaderFile {
                is_read: flag.as_bool().unwrap_or(false),
                page: None,
            },
        )?;
    }
    let body = serde_json::to_vec_pretty(&value)?;
    crate::fs::atomic_write(&meta_path, &body)?;
    Ok(true)
}

fn is_paper_dir(dir: &Path) -> bool {
    dir.join(super::sidecar::SIDECAR_FILE).is_file() || dir.join("NOTES.md").is_file()
}

/// Strip group read flags and the shared annotation aggregate for one vault.
/// Requires a signed-in member. Does nothing for any other billing source.
pub fn migrate_library(vault: &Path, layout: &MemberLayout) -> Result<(), AppError> {
    if !layout.shared {
        return Ok(());
    }
    let Some(user_id) = layout.user_id.as_deref() else {
        return Ok(());
    };
    let papers = vault.join("papers");
    if !papers.is_dir() {
        return Ok(());
    }
    let mut stack = vec![papers];
    while let Some(dir) = stack.pop() {
        if is_paper_dir(&dir) {
            if let Some(rel) = dir
                .strip_prefix(vault)
                .ok()
                .and_then(|path| path.to_str())
                .map(crate::fs::normalize_rel_separators)
                .filter(|rel| !rel.is_empty())
            {
                migrate_paper(vault, &rel, user_id)?;
                crate::features::pdf::marks::files::migrate_annotation_aggregate(&dir, user_id)?;
            }
            continue;
        }
        if let Ok(read) = fs::read_dir(&dir) {
            for entry in read.flatten() {
                if entry.path().is_dir() {
                    stack.push(entry.path());
                }
            }
        }
    }
    Ok(())
}

/// Migrate every connected folder whose remembered billing source is `org_shared`.
pub fn migrate_known_shared_libraries() -> Result<(), AppError> {
    let map = load()?;
    for entry in map.values() {
        if entry.billing_source.as_deref() != Some(ORG_SHARED) {
            continue;
        }
        let vault = Path::new(&entry.path);
        if !vault.is_dir() {
            continue;
        }
        let layout = crate::features::library::layout_for(vault);
        migrate_library(vault, &layout)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::catalog::papers;
    use crate::features::library::MemberLayout;

    fn layout(user: &str) -> MemberLayout {
        MemberLayout {
            shared: true,
            user_id: Some(user.to_string()),
        }
    }

    #[test]
    fn one_member_read_does_not_mark_the_other_read() {
        let vault = std::env::temp_dir().join(format!("agentero-readers-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(vault.join("papers/x")).unwrap();
        let conn = crate::features::catalog::ensure_catalog(&vault).unwrap();
        conn.execute(
            "INSERT INTO papers (path, id, type, title, added_at, updated_at) \
             VALUES ('papers/x', 'x', 'article', 'Attention', 't', 't')",
            [],
        )
        .unwrap();

        papers::set_is_read_with(&vault, "papers/x", true, &layout("user-a")).unwrap();
        papers::set_is_read_with(&vault, "papers/x", false, &layout("user-b")).unwrap();

        assert!(read_is_read(&vault, "papers/x", "user-a"));
        assert!(!read_is_read(&vault, "papers/x", "user-b"));
        let raw = fs::read_to_string(vault.join("papers/x/metadata.json")).unwrap();
        let meta: Value = serde_json::from_str(&raw).unwrap();
        assert!(meta.get("is_read").is_none());
        assert_eq!(meta["title"], "Attention");
        crate::features::catalog::evict_catalog_conn(&vault);
        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn legacy_is_read_moves_once_and_keeps_bibliography() {
        let vault = std::env::temp_dir().join(format!("agentero-migrate-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(vault.join("papers/x")).unwrap();
        fs::write(
            vault.join("papers/x/metadata.json"),
            r#"{"id":"x","title":"Attention","authors":["Vaswani"],"tags":["nlp"],"is_read":true,"status":"completed"}"#,
        )
        .unwrap();

        assert!(migrate_paper(&vault, "papers/x", "user-a").unwrap());
        assert!(read_is_read(&vault, "papers/x", "user-a"));
        assert!(!read_is_read(&vault, "papers/x", "user-b"));
        let meta: Value = serde_json::from_str(
            &fs::read_to_string(vault.join("papers/x/metadata.json")).unwrap(),
        )
        .unwrap();
        assert!(meta.get("is_read").is_none());
        assert_eq!(meta["title"], "Attention");
        assert_eq!(meta["authors"][0], "Vaswani");
        assert_eq!(meta["tags"][0], "nlp");

        fs::write(
            vault.join("papers/x/metadata.json"),
            r#"{"id":"x","title":"Attention","is_read":false}"#,
        )
        .unwrap();
        assert!(migrate_paper(&vault, "papers/x", "user-a").unwrap());
        assert!(
            read_is_read(&vault, "papers/x", "user-a"),
            "a later metadata flag must not overwrite the member file"
        );
        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn reading_page_stays_with_the_member_who_wrote_it() {
        let vault = std::env::temp_dir().join(format!("agentero-page-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(vault.join("papers/x")).unwrap();
        set_read_flag(&vault, "papers/x", "user-a", true).unwrap();
        set_reading_page(&vault, "papers/x", "user-a", 4).unwrap();
        set_reading_page(&vault, "papers/x", "user-b", 9).unwrap();
        assert_eq!(reading_page(&vault, "papers/x", "user-a"), 4);
        assert_eq!(reading_page(&vault, "papers/x", "user-b"), 9);
        assert!(read_is_read(&vault, "papers/x", "user-a"));
        assert!(!read_is_read(&vault, "papers/x", "user-b"));
        let _ = fs::remove_dir_all(&vault);
    }

    #[test]
    fn unsigned_layout_does_not_write_a_reader_or_adopt_metadata() {
        let vault =
            std::env::temp_dir().join(format!("agentero-noreader-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(vault.join("papers/x")).unwrap();
        fs::write(
            vault.join("papers/x/metadata.json"),
            r#"{"id":"x","title":"Attention","is_read":true}"#,
        )
        .unwrap();
        let unsigned = MemberLayout {
            shared: true,
            user_id: None,
        };
        assert!(migrate_library(&vault, &unsigned).is_ok());
        assert!(
            vault.join("papers/x/.readers").read_dir().is_err() || {
                vault
                    .join("papers/x/.readers")
                    .read_dir()
                    .unwrap()
                    .next()
                    .is_none()
            }
        );
        let meta: Value = serde_json::from_str(
            &fs::read_to_string(vault.join("papers/x/metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta["is_read"], true);
        assert!(!papers::member_is_read(&vault, "papers/x", &unsigned));
        let _ = fs::remove_dir_all(&vault);
    }
}
