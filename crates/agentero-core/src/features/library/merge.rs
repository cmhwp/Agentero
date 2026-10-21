//! Merge the local id→path map and recent folders with the account's libraries.
//!
//! A server library has a name and no folder until this computer connects one.
//! The path is never invented from the account.

use std::collections::BTreeMap;

use super::store::{normalize_library_path, LocalLibraryEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentFolder {
    pub path: String,
    pub vault_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLibrary {
    pub id: String,
    pub display_name: String,
    pub cloud_bytes: u64,
    pub billing_source: Option<String>,
    pub billing_org_id: Option<String>,
    pub owned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRow {
    pub id: Option<String>,
    pub name: String,
    pub path: Option<String>,
    pub connected: bool,
    pub on_server: bool,
    pub cloud_bytes: u64,
    pub billing_source: Option<String>,
    pub billing_org_id: Option<String>,
    pub owned: bool,
}

struct Draft {
    id: Option<String>,
    name: String,
    path: Option<String>,
    on_server: bool,
    cloud_bytes: u64,
    billing_source: Option<String>,
    billing_org_id: Option<String>,
    owned: bool,
    from_saved_map: bool,
}

fn folder_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn same_path(left: &str, right: &str) -> bool {
    normalize_library_path(left) == normalize_library_path(right)
}

pub fn merge_libraries(
    map: &BTreeMap<String, LocalLibraryEntry>,
    recent: &[RecentFolder],
    server: Option<&[ServerLibrary]>,
) -> Vec<LibraryRow> {
    let mut by_id: BTreeMap<String, Draft> = BTreeMap::new();
    let mut unbound: Vec<Draft> = Vec::new();

    for (id, entry) in map {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        by_id.insert(
            id.to_string(),
            Draft {
                id: Some(id.to_string()),
                name: if entry.name.trim().is_empty() {
                    folder_name(&entry.path)
                } else {
                    entry.name.trim().to_string()
                },
                path: Some(normalize_library_path(&entry.path)),
                on_server: false,
                cloud_bytes: 0,
                billing_source: None,
                billing_org_id: None,
                owned: false,
                from_saved_map: true,
            },
        );
    }

    for folder in recent {
        let path = normalize_library_path(&folder.path);
        if path.is_empty() {
            continue;
        }
        if let Some(id) = folder
            .vault_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            if let Some(row) = by_id.get_mut(id) {
                if row.path.is_none() {
                    row.path = Some(path);
                }
                continue;
            }
            by_id.insert(
                id.to_string(),
                Draft {
                    id: Some(id.to_string()),
                    name: folder_name(&path),
                    path: Some(path),
                    on_server: false,
                    cloud_bytes: 0,
                    billing_source: None,
                    billing_org_id: None,
                    owned: false,
                    from_saved_map: false,
                },
            );
            continue;
        }
        let already = unbound.iter().any(|row| {
            row.path
                .as_deref()
                .is_some_and(|existing| same_path(existing, &path))
        }) || by_id.values().any(|row| {
            row.path
                .as_deref()
                .is_some_and(|existing| same_path(existing, &path))
        });
        if already {
            continue;
        }
        unbound.push(Draft {
            id: None,
            name: folder_name(&path),
            path: Some(path),
            on_server: false,
            cloud_bytes: 0,
            billing_source: None,
            billing_org_id: None,
            owned: false,
            from_saved_map: false,
        });
    }

    if let Some(server) = server {
        for library in server {
            let id = library.id.trim();
            if id.is_empty() {
                continue;
            }
            let name = if library.display_name.trim().is_empty() {
                id.to_string()
            } else {
                library.display_name.trim().to_string()
            };
            if let Some(row) = by_id.get_mut(id) {
                row.name = name;
                row.on_server = true;
                row.cloud_bytes = library.cloud_bytes;
                row.billing_source = library.billing_source.clone();
                row.billing_org_id = library.billing_org_id.clone();
                row.owned = library.owned;
            } else {
                by_id.insert(
                    id.to_string(),
                    Draft {
                        id: Some(id.to_string()),
                        name,
                        path: None,
                        on_server: true,
                        cloud_bytes: library.cloud_bytes,
                        billing_source: library.billing_source.clone(),
                        billing_org_id: library.billing_org_id.clone(),
                        owned: library.owned,
                        from_saved_map: false,
                    },
                );
            }
        }
        by_id.retain(|_, row| row.on_server || !row.from_saved_map);
    }

    let mut rows: Vec<LibraryRow> = by_id
        .into_values()
        .chain(unbound)
        .map(|row| LibraryRow {
            connected: row.path.is_some(),
            id: row.id,
            name: row.name,
            path: row.path,
            on_server: row.on_server,
            cloud_bytes: row.cloud_bytes,
            billing_source: row.billing_source,
            billing_org_id: row.billing_org_id,
            owned: row.owned,
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .on_server
            .cmp(&left.on_server)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, name: &str) -> LocalLibraryEntry {
        LocalLibraryEntry {
            path: path.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn server_library_without_a_folder_stays_unconnected() {
        let server = vec![ServerLibrary {
            id: "vault-a".into(),
            display_name: "Research".into(),
            cloud_bytes: 0,
            billing_source: None,
            billing_org_id: None,
            owned: true,
        }];
        let rows = merge_libraries(&BTreeMap::new(), &[], Some(&server));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Research");
        assert_eq!(rows[0].path, None);
        assert!(!rows[0].connected);
        assert!(rows[0].on_server);
        assert_eq!(rows[0].cloud_bytes, 0);
    }

    #[test]
    fn local_map_connects_a_server_library_and_keeps_its_name() {
        let mut map = BTreeMap::new();
        map.insert("vault-a".into(), entry("/Users/me/research/", "old"));
        let server = vec![ServerLibrary {
            id: "vault-a".into(),
            display_name: "Research".into(),
            cloud_bytes: 0,
            billing_source: Some("org_grant".into()),
            billing_org_id: Some("org-a".into()),
            owned: true,
        }];
        let rows = merge_libraries(&map, &[], Some(&server));
        assert_eq!(rows[0].billing_source.as_deref(), Some("org_grant"));
        assert_eq!(rows[0].billing_org_id.as_deref(), Some("org-a"));
        assert!(rows[0].owned);
        assert_eq!(rows[0].path.as_deref(), Some("/Users/me/research"));
        assert!(rows[0].connected);
        assert_eq!(rows[0].name, "Research");
    }

    #[test]
    fn another_accounts_saved_library_is_hidden_once_this_account_loads() {
        let mut map = BTreeMap::new();
        map.insert("vault-a".into(), entry("/Users/me/research", "Research"));
        map.insert("vault-b".into(), entry("/Users/me/thesis", "Thesis"));
        let server = vec![ServerLibrary {
            id: "vault-b".into(),
            display_name: "Thesis".into(),
            cloud_bytes: 0,
            billing_source: Some("org_shared".into()),
            billing_org_id: Some("org-a".into()),
            owned: false,
        }];
        let rows = merge_libraries(&map, &[], Some(&server));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id.as_deref(), Some("vault-b"));
        assert_eq!(rows[0].path.as_deref(), Some("/Users/me/thesis"));
    }

    #[test]
    fn recent_folder_is_local_when_signed_out() {
        let recent = vec![RecentFolder {
            path: "/Users/me/notes".into(),
            vault_id: Some("vault-b".into()),
        }];
        let rows = merge_libraries(&BTreeMap::new(), &recent, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id.as_deref(), Some("vault-b"));
        assert!(rows[0].connected);
        assert!(!rows[0].on_server);
        assert_eq!(rows[0].name, "notes");
    }
}
