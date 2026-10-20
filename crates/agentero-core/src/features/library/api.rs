//! Account library API. The caller supplies the session token; this module
//! only attaches it as `Authorization: Bearer` and never writes it to disk.

use std::time::Duration;

use serde::Deserialize;

use crate::error::AppError;
use crate::http;

use super::merge::ServerLibrary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePersonal {
    pub personal_cloud_bytes: u64,
    pub plan_key: Option<String>,
    pub plan_version: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteLibraryList {
    pub personal: RemotePersonal,
    pub libraries: Vec<ServerLibrary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemoteUser {
    pub id: String,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<ErrorBody>,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryListBody {
    personal: PersonalBody,
    libraries: Vec<LibraryBody>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersonalBody {
    personal_cloud_bytes: u64,
    plan_key: Option<String>,
    version: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryBody {
    id: String,
    display_name: String,
    cloud_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct SessionBody {
    user: RemoteUser,
}

fn endpoint(origin: &str, path: &str) -> String {
    format!("{}{path}", origin.trim_end_matches('/'))
}

fn unauthorized() -> AppError {
    AppError::domain("unauthorized", "Sign in again")
}

async fn request(
    method: reqwest::Method,
    url: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> Result<reqwest::Response, AppError> {
    let client = http::client(Duration::from_secs(20))?;
    let mut builder = client
        .request(method, url)
        .header("authorization", format!("Bearer {token}"))
        .header("accept", "application/json");
    if let Some(body) = body {
        builder = builder.json(&body);
    }
    builder
        .send()
        .await
        .map_err(|error| AppError::message(format!("library request failed: {error}")))
}

fn api_error(status: reqwest::StatusCode, message: String) -> AppError {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return unauthorized();
    }
    AppError::message(message)
}

async fn read_envelope<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, AppError> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| AppError::message(format!("library response failed: {error}")))?;
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(unauthorized());
    }
    let envelope: Envelope<T> = serde_json::from_str(&text)
        .map_err(|_| AppError::message(format!("library request failed ({status})")))?;
    if envelope.ok {
        return envelope
            .data
            .ok_or_else(|| AppError::message("library response was empty"));
    }
    let message = envelope
        .error
        .map(|error| error.message)
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| format!("library request failed ({status})"));
    Err(api_error(status, message))
}

pub async fn fetch_libraries(api_origin: &str, token: &str) -> Result<RemoteLibraryList, AppError> {
    let response = request(
        reqwest::Method::GET,
        &endpoint(api_origin, "/api/libraries"),
        token,
        None,
    )
    .await?;
    let body: LibraryListBody = read_envelope(response).await?;
    Ok(RemoteLibraryList {
        personal: RemotePersonal {
            personal_cloud_bytes: body.personal.personal_cloud_bytes,
            plan_key: body.personal.plan_key,
            plan_version: body.personal.version,
        },
        libraries: body
            .libraries
            .into_iter()
            .map(|library| ServerLibrary {
                id: library.id,
                display_name: library.display_name,
                cloud_bytes: library.cloud_bytes,
            })
            .collect(),
    })
}

pub async fn register_library(
    api_origin: &str,
    token: &str,
    id: &str,
    display_name: &str,
) -> Result<(), AppError> {
    let response = request(
        reqwest::Method::POST,
        &endpoint(api_origin, "/api/libraries"),
        token,
        Some(serde_json::json!({
            "id": id,
            "displayName": display_name,
        })),
    )
    .await?;
    let _: serde_json::Value = read_envelope(response).await?;
    Ok(())
}

pub async fn fetch_session(api_origin: &str, token: &str) -> Result<Option<RemoteUser>, AppError> {
    let response = request(
        reqwest::Method::GET,
        &endpoint(api_origin, "/api/auth/get-session"),
        token,
        None,
    )
    .await?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(unauthorized());
    }
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| AppError::message(format!("session request failed: {error}")))?;
    if !status.is_success() {
        return Err(AppError::message(format!(
            "session request failed ({status})"
        )));
    }
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(None);
    }
    let body: SessionBody = serde_json::from_str(trimmed)
        .map_err(|_| AppError::message("session response was not recognized"))?;
    Ok(Some(body.user))
}

pub async fn sign_out(api_origin: &str, token: &str) -> Result<(), AppError> {
    let response = request(
        reqwest::Method::POST,
        &endpoint(api_origin, "/api/auth/sign-out"),
        token,
        Some(serde_json::json!({})),
    )
    .await?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Ok(());
    }
    Err(AppError::message(format!(
        "sign-out failed ({})",
        response.status()
    )))
}
