//! Library account commands. A library with its own S3, WebDAV, or Git entry
//! keeps that entry. Otherwise the welcome list records managed-sync quota
//! and the sync scheduler uses the Worker library token.

use std::sync::atomic::{AtomicBool, Ordering};

use std::collections::BTreeMap;
use std::path::Path;

use agentero_core::features::library::{
    self, merge_libraries, LibraryRow, RecentFolder, RemoteUser,
};
use agentero_core::features::sync::config as sync_config;
use agentero_core::features::sync::local::{self as vault_identity, read_vault_id};
use agentero_core::features::sync::managed::{self, LibraryQuota};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::integration::sync::SyncService;

use super::loopback::PendingSignIn;
use super::session;
use crate::core::error::{map_err, ApiResult, AppError};
use crate::core::fs::resolve_vault;

static SIGNING_IN: AtomicBool = AtomicBool::new(false);

struct SignInGuard;

impl SignInGuard {
    fn try_begin() -> Result<Self, AppError> {
        if SIGNING_IN.swap(true, Ordering::AcqRel) {
            return Err(AppError::message("Sign-in is already in progress"));
        }
        Ok(Self)
    }
}

impl Drop for SignInGuard {
    fn drop(&mut self) {
        SIGNING_IN.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryUser {
    pub id: String,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryListItem {
    pub id: Option<String>,
    pub name: String,
    pub path: Option<String>,
    pub connected: bool,
    pub on_server: bool,
    pub used_bytes: Option<u64>,
    pub limit_bytes: Option<u64>,
    pub cloud_mode: Option<String>,
    pub sync_error: Option<String>,
    pub billing_source: Option<String>,
    pub billing_org_id: Option<String>,
    pub owned: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryOrgQuota {
    pub id: String,
    pub name: String,
    pub role: String,
    pub shared_pool_used_bytes: u64,
    pub shared_pool_limit_bytes: u64,
    pub seat_used_bytes: u64,
    pub seat_allocated_bytes: u64,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAccountSnapshot {
    pub signed_in: bool,
    pub user: Option<LibraryUser>,
    pub personal_cloud_bytes: u64,
    pub plan_key: Option<String>,
    pub plan_version: Option<i32>,
    pub remote_error: Option<String>,
    pub libraries: Vec<LibraryListItem>,
    pub organizations: Vec<LibraryOrgQuota>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRecentArgs {
    pub recent_paths: Vec<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySessionSetArgs {
    pub token: String,
    pub recent_paths: Vec<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRegisterArgs {
    pub path: String,
    pub display_name: String,
    pub recent_paths: Vec<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryConnectArgs {
    pub id: String,
    pub path: String,
    pub name: String,
    pub recent_paths: Vec<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryBillingArgs {
    pub id: String,
    pub billing_source: String,
    pub billing_org_id: Option<String>,
    pub recent_paths: Vec<String>,
}

fn validate_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    let chars = trimmed.chars().count();
    if !(1..=80).contains(&chars) {
        return Err(AppError::message("Display name must be 1–80 characters"));
    }
    Ok(trimmed.to_string())
}

fn validate_id(id: &str) -> Result<String, AppError> {
    let trimmed = id.trim();
    let chars = trimmed.chars().count();
    if !(1..=128).contains(&chars) {
        return Err(AppError::message("Library id must be 1–128 characters"));
    }
    Ok(trimmed.to_string())
}

fn org_from(org: library::RemoteOrgQuota) -> LibraryOrgQuota {
    LibraryOrgQuota {
        id: org.id,
        name: org.name,
        role: org.role,
        shared_pool_used_bytes: org.shared_pool_used_bytes,
        shared_pool_limit_bytes: org.shared_pool_limit_bytes,
        seat_used_bytes: org.seat_used_bytes,
        seat_allocated_bytes: org.seat_allocated_bytes,
    }
}

fn user_from(user: RemoteUser) -> LibraryUser {
    LibraryUser {
        id: user.id,
        name: user.name,
        email: user.email,
    }
}

fn quota_for<'a>(
    row: &LibraryRow,
    quota: &'a BTreeMap<String, LibraryQuota>,
) -> Option<&'a LibraryQuota> {
    if let Some(id) = row.id.as_ref() {
        if let Some(recorded) = quota.get(id) {
            return Some(recorded);
        }
    }
    let path = row.path.as_deref()?;
    let id = read_vault_id(Path::new(path))?;
    quota.get(&id)
}

fn item_from(row: LibraryRow, quota: &BTreeMap<String, LibraryQuota>) -> LibraryListItem {
    let recorded = quota_for(&row, quota);
    LibraryListItem {
        id: row.id,
        name: row.name,
        path: row.path,
        connected: row.connected,
        on_server: row.on_server,
        used_bytes: recorded.map(|item| item.used_bytes),
        limit_bytes: recorded.map(|item| item.limit_bytes),
        cloud_mode: recorded.map(|item| item.mode.clone()),
        sync_error: recorded.and_then(|item| item.message.clone()),
        billing_source: row.billing_source,
        billing_org_id: row.billing_org_id,
        owned: row.owned,
    }
}

fn recent_folders(paths: &[String]) -> Vec<RecentFolder> {
    paths
        .iter()
        .filter_map(|path| {
            let path = path.trim();
            if path.is_empty() || path.starts_with("remote:") {
                return None;
            }
            let vault_id = std::path::Path::new(path)
                .is_dir()
                .then(|| read_vault_id(std::path::Path::new(path)))
                .flatten();
            Some(RecentFolder {
                path: path.to_string(),
                vault_id,
            })
        })
        .collect()
}

fn finish(
    signed_in: bool,
    user: Option<LibraryUser>,
    personal_cloud_bytes: u64,
    plan_key: Option<String>,
    plan_version: Option<i32>,
    remote_error: Option<String>,
    rows: Vec<LibraryRow>,
    organizations: Vec<LibraryOrgQuota>,
    show_quota: bool,
) -> LibraryAccountSnapshot {
    let quota = if show_quota {
        managed::load_library_quota(&managed::library_quota_path())
    } else {
        BTreeMap::new()
    };
    LibraryAccountSnapshot {
        signed_in,
        user,
        personal_cloud_bytes,
        plan_key,
        plan_version,
        remote_error,
        libraries: rows.into_iter().map(|row| item_from(row, &quota)).collect(),
        organizations,
    }
}

fn remember_roster(libraries: &[agentero_core::features::library::ServerLibrary]) {
    let updates: Vec<(String, Option<String>)> = libraries
        .iter()
        .map(|library| (library.id.clone(), library.billing_source.clone()))
        .collect();
    if let Err(error) = library::remember_billing_sources(&updates) {
        log::warn!(target: "agentero::library", "could not remember billing source: {error}");
    }
}

async fn migrate_shared_libraries() {
    match tauri::async_runtime::spawn_blocking(|| {
        agentero_core::features::catalog::readers::migrate_known_shared_libraries()
    })
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            log::warn!(target: "agentero::library", "shared library migration: {error}");
        }
        Err(error) => {
            log::warn!(target: "agentero::library", "shared library migration: {error}");
        }
    }
}

async fn snapshot(recent_paths: &[String]) -> Result<LibraryAccountSnapshot, AppError> {
    let _ = session::hydrate_account_id();
    let recent = recent_folders(recent_paths);
    let map = library::load()?;
    let Some(token) = session::load_token()? else {
        let rows = merge_libraries(&map, &recent, None);
        return Ok(finish(
            false,
            None,
            0,
            None,
            None,
            None,
            rows,
            Vec::new(),
            false,
        ));
    };
    let origin = session::api_origin();
    let user = match library::fetch_session(&origin, &token).await {
        Ok(Some(user)) => {
            if let Err(error) = session::store_user_id(&user.id) {
                log::warn!(target: "agentero::library", "could not store account id: {error}");
            }
            Some(user_from(user))
        }
        Ok(None) => {
            session::clear_token()?;
            let _ = session::clear_user_id();
            let rows = merge_libraries(&map, &recent, None);
            return Ok(finish(
                false,
                None,
                0,
                None,
                None,
                None,
                rows,
                Vec::new(),
                false,
            ));
        }
        Err(error) if error.code() == "unauthorized" => {
            session::clear_token()?;
            let _ = session::clear_user_id();
            let rows = merge_libraries(&map, &recent, None);
            return Ok(finish(
                false,
                None,
                0,
                None,
                None,
                None,
                rows,
                Vec::new(),
                false,
            ));
        }
        Err(error) => {
            let rows = merge_libraries(&map, &recent, None);
            return Ok(finish(
                true,
                None,
                0,
                None,
                None,
                Some(error.to_string()),
                rows,
                Vec::new(),
                true,
            ));
        }
    };
    match library::fetch_libraries(&origin, &token).await {
        Ok(remote) => {
            remember_roster(&remote.libraries);
            migrate_shared_libraries().await;
            let organizations = remote.organizations.into_iter().map(org_from).collect();
            let rows = merge_libraries(&map, &recent, Some(&remote.libraries));
            refresh_managed_quota(&rows, &origin, &token).await;
            Ok(finish(
                true,
                user,
                remote.personal.personal_cloud_bytes,
                remote.personal.plan_key,
                remote.personal.plan_version,
                None,
                rows,
                organizations,
                true,
            ))
        }
        Err(error) if error.code() == "unauthorized" => {
            session::clear_token()?;
            let _ = session::clear_user_id();
            let rows = merge_libraries(&map, &recent, None);
            Ok(finish(
                false,
                None,
                0,
                None,
                None,
                None,
                rows,
                Vec::new(),
                false,
            ))
        }
        Err(error) => {
            let rows = merge_libraries(&map, &recent, None);
            Ok(finish(
                true,
                user,
                0,
                None,
                None,
                Some(error.to_string()),
                rows,
                Vec::new(),
                true,
            ))
        }
    }
}

async fn refresh_managed_quota(rows: &[LibraryRow], origin: &str, token: &str) {
    let quota_path = managed::library_quota_path();
    for row in rows {
        if !row.on_server {
            continue;
        }
        let Some(path) = row.path.as_deref() else {
            continue;
        };
        if sync_config::get(path).is_some() {
            continue;
        }
        let dir = Path::new(path);
        if !dir.is_dir() {
            continue;
        }
        if read_vault_id(dir).is_none() {
            if let Some(id) = &row.id {
                let _ = managed::record_library_quota(
                    &quota_path,
                    id,
                    &LibraryQuota {
                        used_bytes: 0,
                        limit_bytes: 0,
                        mode: "local".into(),
                        message: Some(
                            "This folder has no library id in .agentero/vault.json".into(),
                        ),
                    },
                );
            }
            continue;
        }
        let _ = managed::refresh_library_quota(dir, origin, token, &quota_path).await;
    }
}

fn command_result<T: serde::Serialize>(
    result: Result<T, AppError>,
) -> Result<ApiResult<T>, String> {
    Ok(match result {
        Ok(value) => ApiResult::ok(value),
        Err(error) => map_err(error),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn library_list(
    args: LibraryRecentArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    command_result(snapshot(&args.recent_paths).await)
}

#[tauri::command]
#[specta::specta]
pub async fn library_sign_in(
    app: AppHandle,
    service: State<'_, SyncService>,
    args: LibraryRecentArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    let result = sign_in(app.clone(), &args.recent_paths).await;
    if result.is_ok() {
        service.start_managed_libraries(&app);
    }
    command_result(result)
}

async fn sign_in(
    app: AppHandle,
    recent_paths: &[String],
) -> Result<LibraryAccountSnapshot, AppError> {
    let _guard = SignInGuard::try_begin()?;
    let pending = PendingSignIn::bind().await?;
    let url = format!(
        "{}/desktop/session?port={}&nonce={}",
        session::web_origin(),
        pending.port,
        pending.nonce
    );
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| AppError::message(format!("Could not open the browser: {error}")))?;
    let token = pending.wait().await?;
    session::store_token(&token)?;
    match library::fetch_session(&session::api_origin(), &token).await {
        Ok(Some(_)) => snapshot(recent_paths).await,
        Ok(None) => {
            session::clear_token()?;
            Err(AppError::message("The session token was rejected"))
        }
        Err(error) if error.code() == "unauthorized" => {
            session::clear_token()?;
            Err(AppError::message("The session token was rejected"))
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn library_session_set(
    app: AppHandle,
    service: State<'_, SyncService>,
    args: LibrarySessionSetArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    let result = set_session(args).await;
    if result.is_ok() {
        service.start_managed_libraries(&app);
    }
    command_result(result)
}

async fn set_session(args: LibrarySessionSetArgs) -> Result<LibraryAccountSnapshot, AppError> {
    session::store_token(&args.token)?;
    match library::fetch_session(&session::api_origin(), args.token.trim()).await {
        Ok(Some(_)) => snapshot(&args.recent_paths).await,
        Ok(None) => {
            session::clear_token()?;
            Err(AppError::message("The session token was rejected"))
        }
        Err(error) if error.code() == "unauthorized" => {
            session::clear_token()?;
            Err(AppError::message("The session token was rejected"))
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn library_sign_out(
    service: State<'_, SyncService>,
    args: LibraryRecentArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    command_result(sign_out(&service, &args.recent_paths).await)
}

async fn sign_out(
    service: &SyncService,
    recent_paths: &[String],
) -> Result<LibraryAccountSnapshot, AppError> {
    service.stop_managed_libraries();
    if let Some(token) = session::load_token()? {
        let _ = library::sign_out(&session::api_origin(), &token).await;
    }
    session::clear_token()?;
    let _ = managed::clear_library_quota(&managed::library_quota_path());
    snapshot(recent_paths).await
}

#[tauri::command]
#[specta::specta]
pub async fn library_register(
    app: AppHandle,
    service: State<'_, SyncService>,
    args: LibraryRegisterArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    let result = register(args).await;
    if result.is_ok() {
        service.start_managed_libraries(&app);
    }
    command_result(result)
}

async fn register(args: LibraryRegisterArgs) -> Result<LibraryAccountSnapshot, AppError> {
    let name = validate_name(&args.display_name)?;
    let dir = resolve_vault(&args.path)?;
    let token = session::load_token()?
        .ok_or_else(|| AppError::message("Sign in before registering a library"))?;
    let id = vault_identity::ensure_vault_id(&dir)?;
    library::register_library(&session::api_origin(), &token, &id, &name).await?;
    library::upsert(&id, &args.path, &name)?;
    snapshot(&args.recent_paths).await
}

#[tauri::command]
#[specta::specta]
pub async fn library_connect(
    app: AppHandle,
    service: State<'_, SyncService>,
    args: LibraryConnectArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    let result = connect(args).await;
    if result.is_ok() {
        service.start_managed_libraries(&app);
    }
    command_result(result)
}

#[tauri::command]
#[specta::specta]
pub async fn library_set_billing(
    app: AppHandle,
    service: State<'_, SyncService>,
    args: LibraryBillingArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    let result = set_billing(args).await;
    if result.is_ok() {
        service.start_managed_libraries(&app);
    }
    command_result(result)
}

async fn set_billing(args: LibraryBillingArgs) -> Result<LibraryAccountSnapshot, AppError> {
    let id = validate_id(&args.id)?;
    let source = args.billing_source.trim();
    if source != "personal" && source != "org_grant" {
        return Err(AppError::message(
            "Choose a personal library or an organization seat",
        ));
    }
    if source == "personal" && args.billing_org_id.is_some() {
        return Err(AppError::message("A personal library has no organization"));
    }
    if source == "org_grant"
        && args
            .billing_org_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err(AppError::message("Choose an organization"));
    }
    let token = session::load_token()?
        .ok_or_else(|| AppError::message("Sign in before changing library billing"))?;
    library::set_library_billing(
        &session::api_origin(),
        &token,
        &id,
        source,
        args.billing_org_id.as_deref(),
    )
    .await?;
    snapshot(&args.recent_paths).await
}

async fn connect(args: LibraryConnectArgs) -> Result<LibraryAccountSnapshot, AppError> {
    let id = validate_id(&args.id)?;
    let name = validate_name(&args.name)?;
    let dir = resolve_vault(&args.path)?;
    let token = session::load_token()?
        .ok_or_else(|| AppError::message("Sign in before connecting a library"))?;
    let remote = library::fetch_libraries(&session::api_origin(), &token).await?;
    if !remote.libraries.iter().any(|library| library.id == id) {
        return Err(AppError::message(
            "This account does not include that library",
        ));
    }
    match read_vault_id(&dir) {
        Some(existing) if existing == id => {}
        Some(_) => {
            return Err(AppError::message(
                "This folder already belongs to a different library",
            ));
        }
        None => vault_identity::set_vault_id(&dir, &id)?,
    }
    library::upsert(&id, &args.path, &name)?;
    snapshot(&args.recent_paths).await
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPathArgs {
    /// Vault folder, or any file or folder inside it.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMemberLayout {
    pub shared: bool,
    pub user_id: Option<String>,
}

fn vault_root_for(path: &Path) -> std::path::PathBuf {
    let mut current = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    loop {
        if current.join(".agentero").join("vault.json").is_file() {
            return current.to_path_buf();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return path.to_path_buf(),
        }
    }
}

/// Whether `path` belongs to an `org_shared` library, and the signed-in account id.
#[tauri::command]
#[specta::specta]
pub async fn library_member_layout(
    args: LibraryPathArgs,
) -> Result<ApiResult<LibraryMemberLayout>, String> {
    command_result(
        (|| async {
            if let Err(error) = session::ensure_account_id().await {
                log::warn!(target: "agentero::library", "account id: {error}");
            }
            let vault = vault_root_for(Path::new(args.path.trim()));
            let layout = library::layout_for(&vault);
            Ok(LibraryMemberLayout {
                shared: layout.shared,
                user_id: layout.user_id,
            })
        })()
        .await,
    )
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareSummary {
    pub id: String,
    pub library_id: String,
    pub name: String,
    pub permission: String,
    pub paper_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareFileInfo {
    pub key: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareOpenResult {
    pub share: LibraryShareSummary,
    pub files: Vec<LibraryShareFileInfo>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareImportResult {
    pub paper_paths: Vec<String>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareCommentResult {
    pub id: String,
    pub share_id: String,
    pub author_id: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareCreateArgs {
    pub library_id: String,
    pub name: String,
    pub permission: String,
    pub paper_paths: Vec<String>,
    pub emails: Vec<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareIdArgs {
    pub library_id: String,
    pub share_id: String,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareLibraryArgs {
    pub library_id: String,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareOpenArgs {
    pub share_id: String,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareImportArgs {
    pub share_id: String,
    pub path: String,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LibraryShareCommentArgs {
    pub share_id: String,
    pub body: String,
}

fn share_summary(share: library::ShareSummary) -> LibraryShareSummary {
    LibraryShareSummary {
        id: share.id,
        library_id: share.library_id,
        name: share.name,
        permission: share.permission,
        paper_paths: share.paper_paths,
    }
}

async fn session_token() -> Result<String, AppError> {
    session::load_token()?.ok_or_else(|| AppError::message("Sign in before sharing a library"))
}

/// Share selected paper folders from a managed library. The session token
/// stays in the keychain; the request does not store a library token.
#[tauri::command]
#[specta::specta]
pub async fn library_share_create(
    args: LibraryShareCreateArgs,
) -> Result<ApiResult<LibraryShareSummary>, String> {
    command_result(
        async {
            let id = validate_id(&args.library_id)?;
            let token = session_token().await?;
            let created = library::create_share(
                &session::api_origin(),
                &token,
                &id,
                &args.name,
                &args.permission,
                &args.paper_paths,
                &args.emails,
            )
            .await?;
            Ok(share_summary(created))
        }
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn library_share_revoke(args: LibraryShareIdArgs) -> Result<ApiResult<()>, String> {
    command_result(
        async {
            let id = validate_id(&args.library_id)?;
            let token = session_token().await?;
            library::revoke_share(&session::api_origin(), &token, &id, &args.share_id).await
        }
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn library_shares_received() -> Result<ApiResult<Vec<LibraryShareSummary>>, String> {
    command_result(
        async {
            let token = session_token().await?;
            let shares = library::list_received_shares(&session::api_origin(), &token).await?;
            Ok(shares.into_iter().map(share_summary).collect())
        }
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn library_shares_owned(
    args: LibraryShareLibraryArgs,
) -> Result<ApiResult<Vec<LibraryShareSummary>>, String> {
    command_result(
        async {
            let id = validate_id(&args.library_id)?;
            let token = session_token().await?;
            let shares = library::list_owned_shares(&session::api_origin(), &token, &id).await?;
            Ok(shares.into_iter().map(share_summary).collect())
        }
        .await,
    )
}

/// List the granted files from the source. A later call sees a newer source PDF.
#[tauri::command]
#[specta::specta]
pub async fn library_share_open(
    args: LibraryShareOpenArgs,
) -> Result<ApiResult<LibraryShareOpenResult>, String> {
    command_result(
        async {
            let token = session_token().await?;
            let opened =
                library::open_share(&session::api_origin(), &token, &args.share_id).await?;
            Ok(LibraryShareOpenResult {
                share: share_summary(opened.share),
                files: opened
                    .files
                    .into_iter()
                    .map(|file| LibraryShareFileInfo {
                        key: file.key,
                        size: file.size,
                    })
                    .collect(),
            })
        }
        .await,
    )
}

/// Copy the granted papers into a local library. The copy does not follow the source.
#[tauri::command]
#[specta::specta]
pub async fn library_share_import(
    args: LibraryShareImportArgs,
) -> Result<ApiResult<LibraryShareImportResult>, String> {
    command_result(
        async {
            let dest = resolve_vault(&args.path)?;
            let token = session_token().await?;
            let imported =
                library::import_share_into(&session::api_origin(), &token, &args.share_id, &dest)
                    .await?;
            Ok(LibraryShareImportResult {
                paper_paths: imported.paper_paths,
                files: imported.files,
            })
        }
        .await,
    )
}

#[tauri::command]
#[specta::specta]
pub async fn library_share_comment(
    args: LibraryShareCommentArgs,
) -> Result<ApiResult<LibraryShareCommentResult>, String> {
    command_result(
        async {
            let token = session_token().await?;
            let comment = library::comment_on_share(
                &session::api_origin(),
                &token,
                &args.share_id,
                &args.body,
            )
            .await?;
            Ok(LibraryShareCommentResult {
                id: comment.id,
                share_id: comment.share_id,
                author_id: comment.author_id,
                body: comment.body,
                created_at: comment.created_at,
            })
        }
        .await,
    )
}
