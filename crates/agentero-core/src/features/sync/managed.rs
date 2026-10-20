//! Managed library sync.
//!
//! The Worker signs each object operation. This client keeps only the
//! short-lived library token from `POST /api/libraries/:id/sync-credentials`
//! and never sees the bucket key. A vault that already has an S3, WebDAV, or
//! Git entry in `sync.json` keeps that entry; [`prefers_user_sync_config`]
//! is how the host decides.
//!
//! Read-only mode downloads remote files the local vault does not own and
//! does not upload or delete. A new local file stays on this computer.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::io::Read;

use crate::error::AppError;
use crate::features::sync::config::{SyncBackendConfig, SyncBackendKind};
use crate::features::sync::engine::{self, SyncOutcome};
use crate::features::sync::local;
use crate::features::sync::snapshot::{self, Manifest};
use crate::features::sync::store::{
    check, etag_of, send_with_retries, PutCondition, PutOutcome, RemoteStore, SyncStore,
};
use crate::http;
use crate::paths::agentero_config_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncAccessMode {
    Read,
    #[serde(rename = "readwrite")]
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCredentials {
    pub token: String,
    pub expires_at: String,
    pub prefix: String,
    pub mode: SyncAccessMode,
    pub used_bytes: u64,
    pub limit_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryQuota {
    pub used_bytes: u64,
    pub limit_bytes: u64,
    pub mode: String,
    #[serde(default)]
    pub message: Option<String>,
}

pub struct ManagedClient {
    http: reqwest::Client,
    api_origin: String,
    library_id: String,
    token: String,
    quota_path: PathBuf,
}

impl ManagedClient {
    pub fn new(
        api_origin: impl Into<String>,
        library_id: impl Into<String>,
        token: impl Into<String>,
        quota_path: PathBuf,
    ) -> Result<Self, AppError> {
        let http = http::client_builder()
            .timeout(Duration::from_secs(20))
            .user_agent(http::USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| AppError::message(format!("http client: {error}")))?;
        Ok(Self {
            http,
            api_origin: api_origin.into().trim_end_matches('/').to_string(),
            library_id: library_id.into(),
            token: token.into(),
            quota_path,
        })
    }

    fn object_url(&self, key: &str) -> String {
        let encoded = key
            .split('/')
            .map(|segment| urlencoding::encode(segment).into_owned())
            .collect::<Vec<_>>()
            .join("/");
        format!(
            "{}/api/libraries/{}/objects/{encoded}",
            self.api_origin,
            urlencoding::encode(&self.library_id)
        )
    }

    async fn send(
        &self,
        method: reqwest::Method,
        key: &str,
        extra: Option<(&str, String)>,
        body: Vec<u8>,
    ) -> Result<reqwest::Response, AppError> {
        let url = self.object_url(key);
        let token = self.token.clone();
        let extra = extra.map(|(name, value)| (name.to_string(), value));
        send_with_retries(&method, &url, || {
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .header("authorization", format!("Bearer {token}"))
                .header("accept", "*/*");
            if let Some((name, value)) = &extra {
                request = request.header(name, value);
            }
            if method == reqwest::Method::PUT {
                request = request.body(body.clone());
            }
            request
        })
        .await
    }

    fn note_denied(&self, body: &str) {
        let Some(error) = parse_error(body) else {
            return;
        };
        let Some(used) = error.used_bytes else {
            return;
        };
        let Some(limit) = error.limit_bytes else {
            return;
        };
        let _ = record_library_quota(
            &self.quota_path,
            &self.library_id,
            &LibraryQuota {
                used_bytes: used,
                limit_bytes: limit,
                mode: "read".into(),
                message: Some(error.message),
            },
        );
    }
}

impl RemoteStore for ManagedClient {
    async fn ensure_root(&self) -> Result<(), AppError> {
        let response = self
            .send(reqwest::Method::HEAD, "HEAD", None, Vec::new())
            .await?;
        if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        check(response, "HEAD", "HEAD").await.map(|_| ())
    }

    async fn get(&self, key: &str) -> Result<Option<(Vec<u8>, String)>, AppError> {
        let response = self
            .send(reqwest::Method::GET, key, None, Vec::new())
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = check(response, "GET", key).await?;
        let etag = etag_of(&response);
        let body = response
            .bytes()
            .await
            .map_err(|error| AppError::message(format!("GET {key}: {error}")))?;
        Ok(Some((body.to_vec(), etag)))
    }

    async fn put(
        &self,
        key: &str,
        body: Vec<u8>,
        condition: PutCondition,
    ) -> Result<PutOutcome, AppError> {
        let header = match &condition {
            PutCondition::IfNoneMatch => ("If-None-Match", "*".to_string()),
            PutCondition::IfMatch(etag) => ("If-Match", etag.clone()),
        };
        let response = self
            .send(reqwest::Method::PUT, key, Some(header), body)
            .await?;
        let status = response.status();
        if status == reqwest::StatusCode::PRECONDITION_FAILED
            || status == reqwest::StatusCode::CONFLICT
        {
            return Ok(PutOutcome::PreconditionFailed);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            let text = response.text().await.unwrap_or_default();
            self.note_denied(&text);
            let message = parse_error(&text)
                .map(|error| error.message)
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| "Cloud library is at its quota".to_string());
            return Err(AppError::domain("library_quota_exceeded", message));
        }
        check(response, "PUT", key).await?;
        Ok(PutOutcome::Ok)
    }

    /// The Worker always enforces conditional writes. Probing with a real PUT
    /// would leave an object in the library, so this reports support directly.
    async fn probe_conditional_writes(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// `Some` means the vault already has a user-supplied S3, WebDAV, or Git
/// binding. Managed sync must not call `sync-credentials` in that case.
pub fn prefers_user_sync_config(existing: Option<&SyncBackendConfig>) -> bool {
    existing.is_some()
}

pub fn library_quota_path() -> PathBuf {
    agentero_config_dir().join("library-quota.json")
}

pub fn clear_library_quota(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn managed_backend_config(library_id: &str) -> SyncBackendConfig {
    let mut config = SyncBackendConfig::default();
    config.backend = SyncBackendKind::S3;
    config.endpoint = "https://library-sync.invalid".into();
    config.region = "auto".into();
    config.bucket = "managed".into();
    config.prefix = format!("lib/{library_id}");
    config.force_path_style = true;
    config.conditional_writes = true;
    config.auto_sync = true;
    config.interval_minutes = 30;
    config
}

fn absent<T>() -> Option<T> {
    None
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    ok: bool,
    #[serde(default = "absent")]
    data: Option<T>,
    #[serde(default)]
    error: Option<ErrorBody>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    code: String,
    message: String,
    #[serde(default)]
    used_bytes: Option<u64>,
    #[serde(default)]
    limit_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialsBody {
    token: String,
    expires_at: String,
    prefix: String,
    mode: SyncAccessMode,
    used_bytes: u64,
    limit_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeadPointer {
    manifest_key: String,
}

fn parse_error(body: &str) -> Option<ErrorBody> {
    serde_json::from_str::<Envelope<serde_json::Value>>(body)
        .ok()
        .and_then(|envelope| envelope.error)
}

pub fn load_library_quota(path: &Path) -> BTreeMap<String, LibraryQuota> {
    let Ok(raw) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    if raw.trim().is_empty() {
        return BTreeMap::new();
    }
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        libraries: BTreeMap<String, LibraryQuota>,
    }
    serde_json::from_str::<File>(&raw)
        .map(|file| file.libraries)
        .unwrap_or_default()
}

pub fn record_library_quota(
    path: &Path,
    library_id: &str,
    quota: &LibraryQuota,
) -> Result<(), AppError> {
    let mut libraries = load_library_quota(path);
    libraries.insert(library_id.to_string(), quota.clone());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    #[derive(Serialize)]
    struct File<'a> {
        libraries: &'a BTreeMap<String, LibraryQuota>,
    }
    crate::fs::json_store(
        path,
        &File {
            libraries: &libraries,
        },
    )?;
    Ok(())
}

fn quota_from_credentials(credentials: &SyncCredentials, message: Option<String>) -> LibraryQuota {
    LibraryQuota {
        used_bytes: credentials.used_bytes,
        limit_bytes: credentials.limit_bytes,
        mode: match credentials.mode {
            SyncAccessMode::Read => "read".into(),
            SyncAccessMode::ReadWrite => "readwrite".into(),
        },
        message,
    }
}

async fn request_credentials(
    api_origin: &str,
    session_token: &str,
    library_id: &str,
) -> Result<SyncCredentials, CredentialDenied> {
    let http = http::client(Duration::from_secs(20))
        .map_err(|error| CredentialDenied::plain(error.to_string()))?;
    let url = format!(
        "{}/api/libraries/{}/sync-credentials",
        api_origin.trim_end_matches('/'),
        urlencoding::encode(library_id)
    );
    let response = http
        .post(url)
        .header("authorization", format!("Bearer {session_token}"))
        .header("accept", "application/json")
        .json(&serde_json::json!({ "mode": "write" }))
        .send()
        .await
        .map_err(|error| {
            CredentialDenied::plain(format!("library sync request failed: {error}"))
        })?;
    let status = response.status();
    let text = response.text().await.map_err(|error| {
        CredentialDenied::plain(format!("library sync response failed: {error}"))
    })?;
    let envelope: Envelope<CredentialsBody> = serde_json::from_str(&text)
        .map_err(|_| CredentialDenied::plain(format!("library sync request failed ({status})")))?;
    if envelope.ok {
        let body = envelope
            .data
            .ok_or_else(|| CredentialDenied::plain("library sync response was empty"))?;
        return Ok(SyncCredentials {
            token: body.token,
            expires_at: body.expires_at,
            prefix: body.prefix,
            mode: body.mode,
            used_bytes: body.used_bytes,
            limit_bytes: body.limit_bytes,
        });
    }
    let error = envelope.error.unwrap_or(ErrorBody {
        code: "message".into(),
        message: format!("library sync request failed ({status})"),
        used_bytes: None,
        limit_bytes: None,
    });
    Err(CredentialDenied::from_body(status, error))
}

struct CredentialDenied {
    error: AppError,
    used_bytes: u64,
    limit_bytes: u64,
    mode: &'static str,
}

impl CredentialDenied {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            error: AppError::message(message),
            used_bytes: 0,
            limit_bytes: 0,
            mode: "local",
        }
    }

    fn from_body(status: reqwest::StatusCode, error: ErrorBody) -> Self {
        let message = if error.message.is_empty() {
            format!("library sync request failed ({status})")
        } else {
            error.message
        };
        let (failure, mode) = if error.code == "LIBRARY_SYNC_LOCAL_ONLY" {
            (
                AppError::domain("library_sync_local_only", message),
                "local",
            )
        } else if error.code == "LIBRARY_QUOTA_EXCEEDED" {
            (AppError::domain("library_quota_exceeded", message), "read")
        } else {
            (AppError::message(message), "local")
        };
        Self {
            error: failure,
            used_bytes: error.used_bytes.unwrap_or(0),
            limit_bytes: error.limit_bytes.unwrap_or(0),
            mode,
        }
    }
}

/// Ask the Worker which mode this library may use. Records usage on the quota
/// file the welcome panel reads. Does not upload.
pub async fn refresh_library_quota(
    vault: &Path,
    api_origin: &str,
    session_token: &str,
    quota_path: &Path,
) -> Result<SyncCredentials, AppError> {
    let library_id = local::read_vault_id(vault).ok_or_else(|| {
        AppError::message("This folder has no library id in .agentero/vault.json")
    })?;
    match request_credentials(api_origin, session_token, &library_id).await {
        Ok(credentials) => {
            record_library_quota(
                quota_path,
                &library_id,
                &quota_from_credentials(&credentials, None),
            )?;
            Ok(credentials)
        }
        Err(denied) => {
            let _ = record_library_quota(
                quota_path,
                &library_id,
                &LibraryQuota {
                    used_bytes: denied.used_bytes,
                    limit_bytes: denied.limit_bytes,
                    mode: denied.mode.into(),
                    message: Some(denied.error.to_string()),
                },
            );
            Err(denied.error)
        }
    }
}

/// One managed pass. The library id is the one in `.agentero/vault.json`.
/// A rejected id is surfaced and nothing is uploaded.
pub async fn sync_managed_library(
    vault: &Path,
    api_origin: &str,
    session_token: &str,
    quota_path: &Path,
    progress: engine::Progress<'_>,
) -> Result<SyncOutcome, AppError> {
    let library_id = local::read_vault_id(vault).ok_or_else(|| {
        AppError::message("This folder has no library id in .agentero/vault.json")
    })?;
    let credentials = refresh_library_quota(vault, api_origin, session_token, quota_path).await?;
    let client = ManagedClient::new(
        api_origin,
        library_id.clone(),
        credentials.token.clone(),
        quota_path.to_path_buf(),
    )?;
    if credentials.mode == SyncAccessMode::ReadWrite {
        if let Some((bytes, _)) = client.get("vault.json").await? {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct RemoteIdentity {
                vault_id: String,
            }
            let remote: RemoteIdentity = serde_json::from_slice(&bytes)
                .map_err(|_| AppError::message("remote vault.json is not a sync store"))?;
            if remote.vault_id != library_id {
                return Err(AppError::message(
                    "remote sync store belongs to a different vault",
                ));
            }
        }
    }
    match credentials.mode {
        SyncAccessMode::Read => pull_without_upload(vault, &client).await,
        SyncAccessMode::ReadWrite => {
            let store = SyncStore::managed(client);
            engine::sync_vault_with_store_locked(
                vault,
                &managed_backend_config(&library_id),
                &store,
                progress,
            )
            .await
        }
    }
}

fn decode_payload(bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut out = Vec::new();
        GzDecoder::new(bytes).read_to_end(&mut out)?;
        return Ok(out);
    }
    Ok(bytes.to_vec())
}

fn is_safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.contains('\\')
        && !rel
            .split('/')
            .any(|part| part.is_empty() || part == ".." || part == ".")
}

fn blob_key(hash: &str) -> String {
    format!("blobs/{}/{}", &hash[..2], hash)
}

/// Download remote files this computer does not already have a local edit for.
/// Local-only files are left in place. Nothing is uploaded.
async fn pull_without_upload(
    vault: &Path,
    client: &ManagedClient,
) -> Result<SyncOutcome, AppError> {
    let mut outcome = SyncOutcome::default();
    let Some((head_bytes, _)) = client.get("HEAD").await? else {
        return Ok(outcome);
    };
    let pointer: HeadPointer = serde_json::from_slice(&head_bytes)
        .map_err(|_| AppError::message("remote HEAD is not a sync store"))?;
    if !is_safe_rel(&pointer.manifest_key) {
        return Err(AppError::message("remote HEAD points outside the library"));
    }
    let (manifest_bytes, _) = client
        .get(&pointer.manifest_key)
        .await?
        .ok_or_else(|| AppError::message(format!("manifest {} missing", pointer.manifest_key)))?;
    let manifest: Manifest = serde_json::from_slice(&decode_payload(&manifest_bytes)?)?;
    let base = local::read_base(vault);
    for (rel, entry) in &manifest.files {
        if !is_safe_rel(rel) || entry.hash.len() < 2 {
            return Err(AppError::message(format!(
                "remote manifest has an unsafe path: {rel}"
            )));
        }
        let local_hash = fs::read(vault.join(rel))
            .ok()
            .map(|bytes| snapshot::hash_bytes(&bytes));
        let base_hash = base.files.get(rel).map(|file| file.hash.clone());
        let download = match &local_hash {
            None => true,
            Some(hash) if hash == &entry.hash => false,
            Some(hash) if base_hash.as_deref() == Some(hash.as_str()) => true,
            Some(_) => false,
        };
        if !download {
            continue;
        }
        let (blob, _) = client
            .get(&blob_key(&entry.hash))
            .await?
            .ok_or_else(|| AppError::message(format!("blob missing for {rel}")))?;
        let raw = decode_payload(&blob)?;
        if snapshot::hash_bytes(&raw) != entry.hash {
            return Err(AppError::message(format!("blob corrupt for {rel}")));
        }
        let target = vault.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::fs::atomic_write(&target, &raw)?;
        outcome.downloaded += 1;
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    struct Fake {
        objects: HashMap<String, (Vec<u8>, String)>,
        puts: usize,
        last_put_condition: Option<String>,
        credentials: String,
    }

    struct Server {
        origin: String,
        stop: Arc<AtomicBool>,
        state: Arc<Mutex<Fake>>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn spawn(fake: Fake) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(fake));
        let stop_flag = Arc::clone(&stop);
        let shared = Arc::clone(&state);
        let handle = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        if let Ok((method, path, headers, body)) = read_request(&mut stream) {
                            let response = handle_request(&shared, &method, &path, &headers, &body);
                            let _ = write_response(&mut stream, response);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Server {
            origin,
            stop,
            state,
            handle: Some(handle),
        }
    }

    fn handle_request(
        state: &Mutex<Fake>,
        method: &str,
        path: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let path = path.split('?').next().unwrap_or(path);
        if path.contains("/sync-credentials") {
            let credentials = state.lock().unwrap().credentials.clone();
            return (
                if credentials.contains("\"ok\":false") {
                    403
                } else {
                    200
                },
                vec![("content-type".into(), "application/json".into())],
                credentials.into_bytes(),
            );
        }
        let Some(key) = path.split("/objects/").nth(1) else {
            return (404, vec![], Vec::new());
        };
        let key = decode_key(key);
        if key.split('/').any(|part| part == "..") {
            return (
                400,
                vec![("content-type".into(), "application/json".into())],
                br#"{"ok":false,"error":{"code":"INVALID_PARAM","message":"Object key is not allowed"}}"#.to_vec(),
            );
        }
        let token = headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.trim_start_matches("Bearer ").to_string())
            .unwrap_or_default();
        let mut fake = state.lock().unwrap();
        match method {
            "GET" | "HEAD" => match fake.objects.get(&key) {
                Some((bytes, etag)) => {
                    let payload = if method == "HEAD" {
                        Vec::new()
                    } else {
                        bytes.clone()
                    };
                    (
                        200,
                        vec![
                            ("etag".into(), etag.clone()),
                            ("content-length".into(), bytes.len().to_string()),
                        ],
                        payload,
                    )
                }
                None => (404, vec![], Vec::new()),
            },
            "PUT" => {
                fake.puts += 1;
                let condition = headers.iter().find_map(|(name, value)| {
                    if name == "if-none-match" || name == "if-match" {
                        Some(format!("{name}: {value}"))
                    } else {
                        None
                    }
                });
                fake.last_put_condition = condition;
                if token == "read-token" {
                    return (
                        403,
                        vec![("content-type".into(), "application/json".into())],
                        br#"{"ok":false,"error":{"code":"LIBRARY_QUOTA_EXCEEDED","message":"Cloud library is at its quota","usedBytes":4,"limitBytes":4}}"#.to_vec(),
                    );
                }
                let existing = fake.objects.get(&key).cloned();
                let if_none = header(headers, "if-none-match");
                let if_match = header(headers, "if-match");
                if if_none.as_deref() == Some("*") {
                    if existing.is_some() {
                        return (412, vec![], Vec::new());
                    }
                } else if let Some(expected) = if_match {
                    match &existing {
                        Some((_, etag)) if etag_bare(etag) == etag_bare(&expected) => {}
                        _ => return (412, vec![], Vec::new()),
                    }
                } else {
                    return (400, vec![], Vec::new());
                }
                let etag = format!("\"{}\"", fake.objects.len() + 1);
                fake.objects.insert(key, (body.to_vec(), etag.clone()));
                (200, vec![("etag".into(), etag)], Vec::new())
            }
            _ => (405, vec![], Vec::new()),
        }
    }

    fn header(headers: &[(String, String)], name: &str) -> Option<String> {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }

    fn etag_bare(etag: &str) -> String {
        etag.trim().trim_matches('"').to_string()
    }

    fn decode_key(key: &str) -> String {
        key.split('/')
            .map(|segment| {
                urlencoding::decode(segment)
                    .unwrap_or(std::borrow::Cow::Borrowed(segment))
                    .into_owned()
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    fn read_request(
        stream: &mut std::net::TcpStream,
    ) -> std::io::Result<(String, String, Vec<(String, String)>, Vec<u8>)> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 2048];
        let header_end = loop {
            let read = stream.read(&mut tmp)?;
            if read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "closed",
                ));
            }
            buf.extend_from_slice(&tmp[..read]);
            if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let header_text = String::from_utf8_lossy(&buf[..header_end - 4]).to_string();
        let mut lines = header_text.split("\r\n");
        let request = lines.next().unwrap_or("");
        let mut parts = request.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let mut headers = Vec::new();
        let mut content_length = 0usize;
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
        let mut body = buf[header_end..].to_vec();
        while body.len() < content_length {
            let read = stream.read(&mut tmp)?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..read]);
        }
        body.truncate(content_length);
        Ok((method, path, headers, body))
    }

    fn write_response(
        stream: &mut std::net::TcpStream,
        response: (u16, Vec<(String, String)>, Vec<u8>),
    ) -> std::io::Result<()> {
        let (status, headers, body) = response;
        let mut head = format!(
            "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            head.push_str(&name);
            head.push_str(": ");
            head.push_str(&value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes())?;
        stream.write_all(&body)?;
        Ok(())
    }

    fn write_identity(vault: &Path, id: &str) {
        fs::create_dir_all(vault.join(".agentero")).unwrap();
        fs::write(
            vault.join(".agentero/vault.json"),
            format!(r#"{{"id":"{id}","createdAt":"2026-01-01T00:00:00.000Z"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn user_sync_config_keeps_s3_webdav_and_git() {
        let mut config = SyncBackendConfig::default();
        assert!(prefers_user_sync_config(Some(&config)));
        config.backend = SyncBackendKind::Webdav;
        assert!(prefers_user_sync_config(Some(&config)));
        config.backend = SyncBackendKind::Git;
        assert!(prefers_user_sync_config(Some(&config)));
        assert!(!prefers_user_sync_config(None));
    }

    #[tokio::test]
    async fn conditional_headers_and_read_only_put_keep_the_previous_body() {
        let mut objects = HashMap::new();
        objects.insert("note.bin".into(), (b"kept".to_vec(), "\"1\"".into()));
        let server = spawn(Fake {
            objects,
            puts: 0,
            last_put_condition: None,
            credentials: "{}".into(),
        });
        let quota = tempfile::tempdir().unwrap();
        let quota_path = quota.path().join("quota.json");
        let writer =
            ManagedClient::new(&server.origin, "lib-a", "write-token", quota_path.clone()).unwrap();
        let created = writer
            .put("blobs/aa/hash", b"one".to_vec(), PutCondition::IfNoneMatch)
            .await
            .unwrap();
        assert_eq!(created, PutOutcome::Ok);
        assert_eq!(
            server.state.lock().unwrap().last_put_condition.as_deref(),
            Some("if-none-match: *")
        );
        let raced = writer
            .put("blobs/aa/hash", b"two".to_vec(), PutCondition::IfNoneMatch)
            .await
            .unwrap();
        assert_eq!(raced, PutOutcome::PreconditionFailed);
        let (body, etag) = writer.get("blobs/aa/hash").await.unwrap().unwrap();
        assert_eq!(body, b"one");
        let wrong = writer
            .put(
                "blobs/aa/hash",
                b"three".to_vec(),
                PutCondition::IfMatch("\"no\"".into()),
            )
            .await
            .unwrap();
        assert_eq!(wrong, PutOutcome::PreconditionFailed);
        let replaced = writer
            .put(
                "blobs/aa/hash",
                b"four".to_vec(),
                PutCondition::IfMatch(etag),
            )
            .await
            .unwrap();
        assert_eq!(replaced, PutOutcome::Ok);
        assert_eq!(
            writer.get("blobs/aa/hash").await.unwrap().unwrap().0,
            b"four"
        );

        let reader =
            ManagedClient::new(&server.origin, "lib-a", "read-token", quota_path.clone()).unwrap();
        let denied = reader
            .put(
                "note.bin",
                b"changed".to_vec(),
                PutCondition::IfMatch("\"1\"".into()),
            )
            .await;
        assert_eq!(denied.unwrap_err().code(), "library_quota_exceeded");
        assert_eq!(reader.get("note.bin").await.unwrap().unwrap().0, b"kept");
        let stored = load_library_quota(&quota_path);
        assert_eq!(stored["lib-a"].used_bytes, 4);
        assert_eq!(stored["lib-a"].limit_bytes, 4);
        // Prefix checks live on the Worker. The client still sends the key.
        let _ = writer.get("foo/../secret").await;
    }

    #[tokio::test]
    async fn rejected_library_id_does_not_upload() {
        let server = spawn(Fake {
            objects: HashMap::new(),
            puts: 0,
            last_put_condition: None,
            credentials: r#"{"ok":false,"error":{"code":"FORBIDDEN","message":"This library belongs to another account"}}"#.into(),
        });
        let vault = tempfile::tempdir().unwrap();
        write_identity(vault.path(), "vault-a");
        fs::write(vault.path().join("local-only.md"), b"stay").unwrap();
        let quota = tempfile::tempdir().unwrap();
        let error = sync_managed_library(
            vault.path(),
            &server.origin,
            "session",
            &quota.path().join("quota.json"),
            &|_, _, _| {},
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("another account"));
        assert_eq!(server.state.lock().unwrap().puts, 0);
        assert_eq!(
            fs::read(vault.path().join("local-only.md")).unwrap(),
            b"stay"
        );
    }

    #[tokio::test]
    async fn read_mode_downloads_missing_files_and_keeps_local_ones() {
        let raw = b"hello";
        let hash = snapshot::hash_bytes(raw);
        let blob = format!("blobs/{}/{}", &hash[..2], hash);
        let manifest = serde_json::json!({
            "version": 1,
            "files": {
                "note.md": { "hash": hash, "size": raw.len(), "mtimeMs": 1 },
                "edited.md": { "hash": hash, "size": raw.len(), "mtimeMs": 1 }
            }
        });
        let head = serde_json::json!({
            "version": 1,
            "manifestKey": "manifests/0000000001-test.json",
            "updatedAt": "2026-01-01T00:00:00.000Z"
        });
        let mut objects = HashMap::new();
        objects.insert(
            "HEAD".into(),
            (serde_json::to_vec(&head).unwrap(), "\"h\"".into()),
        );
        objects.insert(
            "manifests/0000000001-test.json".into(),
            (serde_json::to_vec(&manifest).unwrap(), "\"m\"".into()),
        );
        objects.insert(blob, (raw.to_vec(), "\"b\"".into()));
        let server = spawn(Fake {
            objects,
            puts: 0,
            last_put_condition: None,
            credentials: r#"{"ok":true,"data":{"token":"read-token","expiresAt":"2099-01-01T00:00:00.000Z","prefix":"lib/vault-a/","mode":"read","usedBytes":5,"limitBytes":5}}"#.into(),
        });
        let vault = tempfile::tempdir().unwrap();
        write_identity(vault.path(), "vault-a");
        fs::write(vault.path().join("local-only.md"), b"stay").unwrap();
        fs::write(vault.path().join("edited.md"), b"mine").unwrap();
        let quota = tempfile::tempdir().unwrap();
        let outcome = sync_managed_library(
            vault.path(),
            &server.origin,
            "session",
            &quota.path().join("quota.json"),
            &|_, _, _| {},
        )
        .await
        .unwrap();
        assert_eq!(outcome.downloaded, 1);
        assert_eq!(fs::read(vault.path().join("note.md")).unwrap(), b"hello");
        assert_eq!(
            fs::read(vault.path().join("local-only.md")).unwrap(),
            b"stay"
        );
        assert_eq!(fs::read(vault.path().join("edited.md")).unwrap(), b"mine");
        assert_eq!(server.state.lock().unwrap().puts, 0);
        let stored = load_library_quota(&quota.path().join("quota.json"));
        assert_eq!(stored["vault-a"].mode, "read");
        assert_eq!(stored["vault-a"].used_bytes, 5);
        assert_eq!(stored["vault-a"].limit_bytes, 5);
    }
}
