//! Library account commands. Sync itself is unchanged: these commands only
//! store the local id→path map and register a display name with the account.

use std::sync::atomic::{AtomicBool, Ordering};

use agentero_core::features::library::{
    self, merge_libraries, LibraryRow, RecentFolder, RemoteUser,
};
use agentero_core::features::sync::local::{self as vault_identity, read_vault_id};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

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

fn user_from(user: RemoteUser) -> LibraryUser {
    LibraryUser {
        id: user.id,
        name: user.name,
        email: user.email,
    }
}

fn item_from(row: LibraryRow) -> LibraryListItem {
    LibraryListItem {
        id: row.id,
        name: row.name,
        path: row.path,
        connected: row.connected,
        on_server: row.on_server,
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
) -> LibraryAccountSnapshot {
    LibraryAccountSnapshot {
        signed_in,
        user,
        personal_cloud_bytes,
        plan_key,
        plan_version,
        remote_error,
        libraries: rows.into_iter().map(item_from).collect(),
    }
}

async fn snapshot(recent_paths: &[String]) -> Result<LibraryAccountSnapshot, AppError> {
    let recent = recent_folders(recent_paths);
    let map = library::load()?;
    let Some(token) = session::load_token()? else {
        let rows = merge_libraries(&map, &recent, None);
        return Ok(finish(false, None, 0, None, None, None, rows));
    };
    let origin = session::api_origin();
    let user = match library::fetch_session(&origin, &token).await {
        Ok(Some(user)) => Some(user_from(user)),
        Ok(None) => {
            session::clear_token()?;
            let rows = merge_libraries(&map, &recent, None);
            return Ok(finish(false, None, 0, None, None, None, rows));
        }
        Err(error) if error.code() == "unauthorized" => {
            session::clear_token()?;
            let rows = merge_libraries(&map, &recent, None);
            return Ok(finish(false, None, 0, None, None, None, rows));
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
            ));
        }
    };
    match library::fetch_libraries(&origin, &token).await {
        Ok(remote) => {
            let rows = merge_libraries(&map, &recent, Some(&remote.libraries));
            Ok(finish(
                true,
                user,
                remote.personal.personal_cloud_bytes,
                remote.personal.plan_key,
                remote.personal.plan_version,
                None,
                rows,
            ))
        }
        Err(error) if error.code() == "unauthorized" => {
            session::clear_token()?;
            let rows = merge_libraries(&map, &recent, None);
            Ok(finish(false, None, 0, None, None, None, rows))
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
            ))
        }
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
    args: LibraryRecentArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    command_result(sign_in(app, &args.recent_paths).await)
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
    args: LibrarySessionSetArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    command_result(set_session(args).await)
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
    args: LibraryRecentArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    command_result(sign_out(&args.recent_paths).await)
}

async fn sign_out(recent_paths: &[String]) -> Result<LibraryAccountSnapshot, AppError> {
    if let Some(token) = session::load_token()? {
        let _ = library::sign_out(&session::api_origin(), &token).await;
    }
    session::clear_token()?;
    snapshot(recent_paths).await
}

#[tauri::command]
#[specta::specta]
pub async fn library_register(
    args: LibraryRegisterArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    command_result(register(args).await)
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
    args: LibraryConnectArgs,
) -> Result<ApiResult<LibraryAccountSnapshot>, String> {
    command_result(connect(args).await)
}

async fn connect(args: LibraryConnectArgs) -> Result<LibraryAccountSnapshot, AppError> {
    let id = validate_id(&args.id)?;
    let name = validate_name(&args.name)?;
    let dir = resolve_vault(&args.path)?;
    let token = session::load_token()?
        .ok_or_else(|| AppError::message("Sign in before connecting a library"))?;
    let remote = library::fetch_libraries(&session::api_origin(), &token).await?;
    if !remote.libraries.iter().any(|library| library.id == id) {
        return Err(AppError::message("This account does not own that library"));
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
