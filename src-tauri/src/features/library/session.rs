//! Desktop login session.
//!
//! The web app signs in with better-auth (GitHub or Google only). After that,
//! `GET /api/auth/get-session` includes the existing `session.token`. The API
//! accepts that same token as `Authorization: Bearer` because the bearer
//! plugin runs inside `auth.api.getSession`, which `requireAuth` calls. This
//! is not a second credential.
//!
//! Sign-in opens the system browser at
//! `{AGENTERO_WEB_ORIGIN}/desktop/session?port=…&nonce=…` (default origin
//! `https://app.agentero.app`). The signed-in page POSTs the session token and
//! that nonce to `http://127.0.0.1:{port}/callback`. The token is stored in the
//! OS keychain (service `app.agentero.desktop`, account `session`) and is never
//! written into a vault, `sync.json`, or `libraries.json`.
//!
//! `library_session_set` stores a token pasted from that same get-session
//! response when the browser handoff cannot finish. API calls use
//! `{AGENTERO_API_ORIGIN}` (default `https://api.agentero.app`).

use crate::core::error::AppError;

// These types exist only when this target's real keychain store is enabled.
// keyring otherwise substitutes an in-memory mock that forgets the token.
#[cfg(target_os = "ios")]
#[allow(dead_code)]
fn _keychain_store(_: keyring::ios::IosCredential) {}
#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn _keychain_store(_: keyring::macos::MacCredential) {}
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
#[allow(dead_code)]
fn _keychain_store(_: keyring::secret_service::SsCredential) {}
#[cfg(target_os = "windows")]
#[allow(dead_code)]
fn _keychain_store(_: keyring::windows::WinCredential) {}

const SERVICE: &str = "app.agentero.desktop";
const ACCOUNT: &str = "session";
const USER_ACCOUNT: &str = "user";

pub fn web_origin() -> String {
    origin_from_env("AGENTERO_WEB_ORIGIN", "https://app.agentero.app")
}

pub fn api_origin() -> String {
    origin_from_env("AGENTERO_API_ORIGIN", "https://api.agentero.app")
}

fn origin_from_env(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub fn acceptable_token(token: &str) -> bool {
    let token = token.trim();
    (8..=1024).contains(&token.len())
        && token
            .chars()
            .all(|ch| ch.is_ascii_graphic() && ch != '"' && ch != '\\')
}

fn entry() -> Result<keyring::Entry, AppError> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|error| AppError::message(format!("keychain: {error}")))
}

pub fn store_token(token: &str) -> Result<(), AppError> {
    let token = token.trim();
    if !acceptable_token(token) {
        return Err(AppError::message("Session token was not recognized"));
    }
    entry()?
        .set_password(token)
        .map_err(|error| AppError::message(format!("keychain: {error}")))
}

pub fn load_token() -> Result<Option<String>, AppError> {
    match entry()?.get_password() {
        Ok(token) if acceptable_token(&token) => Ok(Some(token)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(AppError::message(format!("keychain: {error}"))),
    }
}

pub fn clear_token() -> Result<(), AppError> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(AppError::message(format!("keychain: {error}"))),
    }
}

fn user_entry() -> Result<keyring::Entry, AppError> {
    keyring::Entry::new(SERVICE, USER_ACCOUNT)
        .map_err(|error| AppError::message(format!("keychain: {error}")))
}

/// Remember the web user id beside the session token. This is not the token.
pub fn store_user_id(user_id: &str) -> Result<(), AppError> {
    let user_id = agentero_core::features::library::safe_member_id(user_id)
        .ok_or_else(|| AppError::message("Signed-in account id is not a valid path"))?;
    user_entry()?
        .set_password(user_id)
        .map_err(|error| AppError::message(format!("keychain: {error}")))?;
    agentero_core::features::library::store_account_id(user_id)
}

pub fn load_user_id() -> Result<Option<String>, AppError> {
    match user_entry()?.get_password() {
        Ok(user_id) => {
            Ok(agentero_core::features::library::safe_member_id(&user_id).map(str::to_string))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(AppError::message(format!("keychain: {error}"))),
    }
}

pub fn clear_user_id() -> Result<(), AppError> {
    match user_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(AppError::message(format!("keychain: {error}"))),
    }
    agentero_core::features::library::clear_account_id()
}

/// Copy the keychain user id into the account cache, or clear it when signed out.
pub fn hydrate_account_id() -> Result<(), AppError> {
    match load_token()? {
        None => clear_user_id(),
        Some(_) => {
            if agentero_core::features::library::load_account_id().is_none() {
                if let Some(user_id) = load_user_id()? {
                    agentero_core::features::library::store_account_id(&user_id)?;
                }
            }
            Ok(())
        }
    }
}

fn cached_user_id() -> Result<Option<String>, AppError> {
    if let Some(user_id) = load_user_id()? {
        if agentero_core::features::library::load_account_id().is_none() {
            agentero_core::features::library::store_account_id(&user_id)?;
        }
        return Ok(Some(user_id));
    }
    if let Some(user_id) = agentero_core::features::library::load_account_id() {
        store_user_id(&user_id)?;
        return Ok(Some(user_id));
    }
    Ok(None)
}

/// Keychain or account cache, otherwise the signed-in session.
/// A network failure leaves both stores empty and returns the error so callers
/// do not turn on the shared layout before an account id exists.
pub async fn ensure_account_id() -> Result<Option<String>, AppError> {
    let Some(token) = load_token()? else {
        clear_user_id()?;
        return Ok(None);
    };
    if let Some(user_id) = cached_user_id()? {
        return Ok(Some(user_id));
    }
    match agentero_core::features::library::fetch_session(&api_origin(), &token).await {
        Ok(Some(user)) => {
            store_user_id(&user.id)?;
            Ok(Some(user.id))
        }
        Ok(None) => Ok(None),
        Err(error) if error.code() == "unauthorized" => {
            clear_token()?;
            let _ = clear_user_id();
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
