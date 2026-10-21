//! On-disk mark files.
//!
//! Personal and `org_grant` libraries keep `marks/<markId>.json` plus the
//! `marks/annotations.json` aggregate. An `org_shared` library stores each
//! mark at `marks/{userId}/{markId}.json`. The viewer builds its annotation
//! list from those files. Two members never write the same path.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::AppError;
use crate::features::library::safe_member_id;

pub const ANNOTATIONS_JSON: &str = "annotations.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkFile {
    pub path: PathBuf,
    pub id: String,
    /// `None` for a legacy flat `marks/<id>.json`.
    pub owner: Option<String>,
}

fn json_mark_id(name: &str) -> Option<String> {
    if name == ANNOTATIONS_JSON || !name.ends_with(".json") {
        return None;
    }
    let id = name.trim_end_matches(".json");
    safe_member_id(id).map(str::to_string)
}

/// Flat mark files plus one level of `marks/{userId}/`. Skips `annotations.json`.
pub fn list_mark_files(paper_dir: &Path) -> Vec<MarkFile> {
    let marks = paper_dir.join("marks");
    let Ok(entries) = fs::read_dir(&marks) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            let Some(owner) = safe_member_id(&name).map(str::to_string) else {
                continue;
            };
            let Ok(children) = fs::read_dir(&path) else {
                continue;
            };
            for child in children.flatten() {
                let child_name = child.file_name().to_string_lossy().to_string();
                if let Some(id) = json_mark_id(&child_name) {
                    out.push(MarkFile {
                        path: child.path(),
                        id,
                        owner: Some(owner.clone()),
                    });
                }
            }
            continue;
        }
        if let Some(id) = json_mark_id(&name) {
            out.push(MarkFile {
                path,
                id,
                owner: None,
            });
        }
    }
    out
}

pub fn member_mark_path(
    paper_dir: &Path,
    user_id: &str,
    mark_id: &str,
) -> Result<PathBuf, AppError> {
    let user_id = safe_member_id(user_id)
        .ok_or_else(|| AppError::message("Signed-in account id is not a valid path"))?;
    let mark_id =
        safe_member_id(mark_id).ok_or_else(|| AppError::message("mark id is not a valid path"))?;
    Ok(paper_dir
        .join("marks")
        .join(user_id)
        .join(format!("{mark_id}.json")))
}

fn write_json(path: &Path, value: &Value) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut body = serde_json::to_vec_pretty(value)?;
    body.push(b'\n');
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        AppError::message(format!("replace mark file: {error}"))
    })?;
    Ok(())
}

/// Write one mark owned by `user_id`. Does not touch any other member's file.
pub fn write_owned_mark(
    paper_dir: &Path,
    user_id: &str,
    mark_id: &str,
    value: &Value,
) -> Result<PathBuf, AppError> {
    let path = member_mark_path(paper_dir, user_id, mark_id)?;
    write_json(&path, value)?;
    Ok(path)
}

fn annotation_id(item: &Value) -> Option<String> {
    item.get("annotation")?
        .get("id")?
        .as_str()
        .and_then(safe_member_id)
        .map(str::to_string)
}

fn load_annotation_array(path: &Path) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Array(items)) => items,
        _ => Vec::new(),
    }
}

/// Append highlight transfer items under `marks/{userId}/`. An id that already
/// exists for any member is left untouched, so two writers cannot overwrite
/// one file. Does not write `annotations.json`.
pub fn append_owned_highlights(
    paper_dir: &Path,
    user_id: &str,
    items: &[Value],
) -> Result<usize, AppError> {
    let mut existing: HashSet<String> = list_mark_files(paper_dir)
        .into_iter()
        .map(|file| file.id)
        .collect();
    let mut added = 0;
    for item in items {
        let Some(id) = annotation_id(item) else {
            continue;
        };
        if existing.contains(&id) {
            continue;
        }
        write_owned_mark(paper_dir, user_id, &id, item)?;
        existing.insert(id);
        added += 1;
    }
    Ok(added)
}

/// Copy `marks/annotations.json` into the signed-in member's mark files, then
/// remove the aggregate so it is no longer the shared source of truth.
/// Items that already have a per-member file are not copied again.
pub fn migrate_annotation_aggregate(paper_dir: &Path, user_id: &str) -> Result<(), AppError> {
    let aggregate = paper_dir.join("marks").join(ANNOTATIONS_JSON);
    if !aggregate.is_file() {
        return Ok(());
    }
    let existing: HashSet<String> = list_mark_files(paper_dir)
        .into_iter()
        .map(|file| file.id)
        .collect();
    for item in load_annotation_array(&aggregate) {
        let Some(id) = annotation_id(&item) else {
            continue;
        };
        if existing.contains(&id) {
            continue;
        }
        write_owned_mark(paper_dir, user_id, &id, &item)?;
    }
    fs::remove_file(&aggregate)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: &str) -> Value {
        json!({ "annotation": { "id": id, "type": 9, "pageIndex": 1 } })
    }

    #[test]
    fn two_members_keep_both_highlight_files() {
        let paper = std::env::temp_dir().join(format!("agentero-marks-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(paper.join("marks")).unwrap();
        append_owned_highlights(&paper, "user-a", &[item("hl-a")]).unwrap();
        append_owned_highlights(&paper, "user-b", &[item("hl-b")]).unwrap();
        assert_eq!(
            append_owned_highlights(&paper, "user-b", &[item("hl-a")]).unwrap(),
            0,
            "an existing id is not overwritten"
        );
        assert!(paper.join("marks/user-a/hl-a.json").is_file());
        assert!(paper.join("marks/user-b/hl-b.json").is_file());
        assert!(!paper.join("marks/annotations.json").exists());
        let files = list_mark_files(&paper);
        assert_eq!(files.len(), 2);
        let _ = fs::remove_dir_all(&paper);
    }

    #[test]
    fn aggregate_moves_into_the_signed_in_members_files() {
        let paper = std::env::temp_dir().join(format!("agentero-agg-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(paper.join("marks")).unwrap();
        fs::write(
            paper.join("marks/annotations.json"),
            serde_json::to_string_pretty(&json!([item("hl-old")])).unwrap(),
        )
        .unwrap();
        fs::write(paper.join("marks/ask-1.json"), b"{\"kind\":\"ask\"}\n").unwrap();
        migrate_annotation_aggregate(&paper, "user-a").unwrap();
        assert!(!paper.join("marks/annotations.json").exists());
        assert!(paper.join("marks/user-a/hl-old.json").is_file());
        assert!(paper.join("marks/ask-1.json").is_file());
        let _ = fs::remove_dir_all(&paper);
    }
}
