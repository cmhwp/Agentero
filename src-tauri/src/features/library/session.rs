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
