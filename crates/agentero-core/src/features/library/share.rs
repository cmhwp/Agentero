//! Share specific papers and copy them into a local library.
//!
//! The session token and the short-lived share token stay in memory. This
//! module does not write them into a vault, `sync.json`, or `libraries.json`,
//! and an imported paper does not keep a pointer back to the source share.
//! Uploading the copy, if it happens at all, is the destination library's own
//! sync. With no quota the files stay on disk.

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::error::AppError;
use crate::http;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareSummary {
    pub id: String,
    pub library_id: String,
    pub name: String,
    pub permission: String,
    pub paper_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareFileInfo {
    pub key: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareOpened {
    pub share: ShareSummary,
    pub files: Vec<ShareFileInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareComment {
    pub id: String,
    pub share_id: String,
    pub author_id: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedPapers {
    pub paper_paths: Vec<String>,
    pub files: Vec<String>,
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
struct OpenBody {
    id: String,
    library_id: String,
    name: String,
    permission: String,
    paper_paths: Vec<String>,
    files: Vec<ShareFileInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareListBody {
    shares: Vec<ShareSummary>,
}

#[derive(Debug, Deserialize)]
struct CredentialsBody {
    token: String,
    mode: String,
}

/// `papers/<id>` and nested folders under it. The library root is not a paper.
pub fn normalize_paper_folder(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 512 {
        return None;
    }
    if trimmed.contains('\\')
        || trimmed.contains('\0')
        || trimmed.contains("..")
        || trimmed.starts_with('/')
    {
        return None;
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() < 2 || parts[0] != "papers" {
        return None;
    }
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return None;
    }
    Some(parts.join("/"))
}

fn key_in_folders(folders: &[String], key: &str) -> bool {
    folders
        .iter()
        .any(|folder| key == folder || key.starts_with(&format!("{folder}/")))
}

fn normalize_object_key(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim().trim_start_matches('/');
    if trimmed.is_empty()
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || trimmed.contains("..")
    {
        return Err(AppError::message("This file is not part of the share"));
    }
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(AppError::message("This file is not part of the share"));
    }
    Ok(parts.join("/"))
}

/// Copy granted paper files into `dest`. Nothing else is written, and the
/// copy does not record the share id or a token.
pub fn import_granted_files(
    dest: &Path,
    paper_paths: &[String],
    files: &[(String, Vec<u8>)],
) -> Result<Vec<String>, AppError> {
    let folders: Vec<String> = paper_paths
        .iter()
        .filter_map(|path| normalize_paper_folder(path))
        .collect();
    if folders.is_empty() {
        return Err(AppError::message("Choose at least one paper"));
    }
    let mut planned = Vec::new();
    for (key, bytes) in files {
        let relative = normalize_object_key(key)?;
        if !key_in_folders(&folders, &relative) {
            return Err(AppError::message("This file is not part of the share"));
        }
        let target = dest.join(Path::new(&relative));
        planned.push((target, relative, bytes.as_slice()));
    }
    let mut written = Vec::new();
    for (target, relative, bytes) in planned {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, bytes)?;
        written.push(relative);
    }
    Ok(written)
}

fn endpoint(origin: &str, path: &str) -> String {
    format!("{}{path}", origin.trim_end_matches('/'))
}

fn encode_key(key: &str) -> String {
    key.split('/')
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/")
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
        .map_err(|error| AppError::message(format!("share request failed: {error}")))
}

async fn read_envelope<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, AppError> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| AppError::message(format!("share response failed: {error}")))?;
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::domain("unauthorized", "Sign in again"));
    }
    let envelope: Envelope<T> = serde_json::from_str(&text)
        .map_err(|_| AppError::message(format!("share request failed ({status})")))?;
    if envelope.ok {
        return envelope
            .data
            .ok_or_else(|| AppError::message("share response was empty"));
    }
    let message = envelope
        .error
        .map(|error| error.message)
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| format!("share request failed ({status})"));
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err(AppError::domain("share_unavailable", message));
    }
    Err(AppError::message(message))
}

fn paper_folders(paths: &[String]) -> Result<Vec<String>, AppError> {
    if paths.is_empty() {
        return Err(AppError::message("Choose at least one paper"));
    }
    let mut folders = Vec::new();
    for raw in paths {
        let Some(folder) = normalize_paper_folder(raw) else {
            return Err(AppError::message(
                "Paper path must be a folder under papers/",
            ));
        };
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    Ok(folders)
}

pub async fn create_share(
    api_origin: &str,
    session_token: &str,
    library_id: &str,
    name: &str,
    permission: &str,
    paper_paths: &[String],
    emails: &[String],
) -> Result<ShareSummary, AppError> {
    let folders = paper_folders(paper_paths)?;
    if permission != "view" && permission != "comment" {
        return Err(AppError::message(
            "Share permission must be view or comment",
        ));
    }
    let grants: Vec<serde_json::Value> = emails
        .iter()
        .map(|email| email.trim().to_lowercase())
        .filter(|email| !email.is_empty())
        .map(|email| serde_json::json!({ "email": email }))
        .collect();
    if grants.is_empty() {
        return Err(AppError::message("Grant needs an email or a user"));
    }
    let response = request(
        reqwest::Method::POST,
        &endpoint(api_origin, &format!("/api/libraries/{library_id}/shares")),
        session_token,
        Some(serde_json::json!({
            "name": name.trim(),
            "permission": permission,
            "paperPaths": folders,
            "grants": grants,
        })),
    )
    .await?;
    read_envelope(response).await
}

pub async fn revoke_share(
    api_origin: &str,
    session_token: &str,
    library_id: &str,
    share_id: &str,
) -> Result<(), AppError> {
    let response = request(
        reqwest::Method::POST,
        &endpoint(
            api_origin,
            &format!("/api/libraries/{library_id}/shares/{share_id}/revoke"),
        ),
        session_token,
        None,
    )
    .await?;
    let _: serde_json::Value = read_envelope(response).await?;
    Ok(())
}

pub async fn list_owned_shares(
    api_origin: &str,
    session_token: &str,
    library_id: &str,
) -> Result<Vec<ShareSummary>, AppError> {
    let response = request(
        reqwest::Method::GET,
        &endpoint(api_origin, &format!("/api/libraries/{library_id}/shares")),
        session_token,
        None,
    )
    .await?;
    let body: ShareListBody = read_envelope(response).await?;
    Ok(body.shares)
}

pub async fn list_received_shares(
    api_origin: &str,
    session_token: &str,
) -> Result<Vec<ShareSummary>, AppError> {
    let response = request(
        reqwest::Method::GET,
        &endpoint(api_origin, "/api/library-shares"),
        session_token,
        None,
    )
    .await?;
    let body: ShareListBody = read_envelope(response).await?;
    Ok(body.shares)
}

pub async fn open_share(
    api_origin: &str,
    session_token: &str,
    share_id: &str,
) -> Result<ShareOpened, AppError> {
    let response = request(
        reqwest::Method::GET,
        &endpoint(api_origin, &format!("/api/library-shares/{share_id}")),
        session_token,
        None,
    )
    .await?;
    let body: OpenBody = read_envelope(response).await?;
    Ok(ShareOpened {
        share: ShareSummary {
            id: body.id,
            library_id: body.library_id,
            name: body.name,
            permission: body.permission,
            paper_paths: body.paper_paths,
        },
        files: body.files,
    })
}

pub async fn comment_on_share(
    api_origin: &str,
    session_token: &str,
    share_id: &str,
    body: &str,
) -> Result<ShareComment, AppError> {
    let response = request(
        reqwest::Method::POST,
        &endpoint(
            api_origin,
            &format!("/api/library-shares/{share_id}/comments"),
        ),
        session_token,
        Some(serde_json::json!({ "body": body.trim() })),
    )
    .await?;
    read_envelope(response).await
}

async fn share_read_token(
    api_origin: &str,
    session_token: &str,
    share_id: &str,
) -> Result<String, AppError> {
    let response = request(
        reqwest::Method::POST,
        &endpoint(
            api_origin,
            &format!("/api/library-shares/{share_id}/credentials"),
        ),
        session_token,
        None,
    )
    .await?;
    let credentials: CredentialsBody = read_envelope(response).await?;
    if credentials.mode != "read" || credentials.token.is_empty() {
        return Err(AppError::message("Share token was not read only"));
    }
    Ok(credentials.token)
}

/// Download the granted files. The share token is dropped before returning.
pub async fn download_share_files(
    api_origin: &str,
    session_token: &str,
    share_id: &str,
) -> Result<(ShareOpened, Vec<(String, Vec<u8>)>), AppError> {
    let opened = open_share(api_origin, session_token, share_id).await?;
    let share_token = share_read_token(api_origin, session_token, share_id).await?;
    let client = http::client(Duration::from_secs(60))?;
    let mut files = Vec::new();
    for info in &opened.files {
        let url = endpoint(
            api_origin,
            &format!(
                "/api/library-shares/{share_id}/objects/{}",
                encode_key(&info.key)
            ),
        );
        let response = client
            .get(url)
            .header("authorization", format!("Bearer {share_token}"))
            .send()
            .await
            .map_err(|error| AppError::message(format!("share download failed: {error}")))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<Envelope<serde_json::Value>>(&text)
                .ok()
                .and_then(|envelope| envelope.error)
                .map(|error| error.message)
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| format!("share download failed ({status})"));
            if status == reqwest::StatusCode::FORBIDDEN {
                return Err(AppError::domain("share_unavailable", message));
            }
            return Err(AppError::message(message));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| AppError::message(format!("share download failed: {error}")))?;
        files.push((info.key.clone(), bytes.to_vec()));
    }
    drop(share_token);
    Ok((opened, files))
}

/// Download the granted papers and copy them into `dest`. The copy is a normal
/// local paper: it does not follow the source, and this function does not upload.
pub async fn import_share_into(
    api_origin: &str,
    session_token: &str,
    share_id: &str,
    dest: &Path,
) -> Result<ImportedPapers, AppError> {
    let (opened, files) = download_share_files(api_origin, session_token, share_id).await?;
    let written = import_granted_files(dest, &opened.share.paper_paths, &files)?;
    Ok(ImportedPapers {
        paper_paths: opened.share.paper_paths,
        files: written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    struct Fake {
        pdf: Vec<u8>,
        revoked: bool,
        puts: usize,
        session: String,
        share_token: String,
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
    ) -> (u16, Vec<u8>) {
        let path = path.split('?').next().unwrap_or(path);
        let auth = headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.trim_start_matches("Bearer ").to_string())
            .unwrap_or_default();
        let mut fake = state.lock().unwrap();
        if path.contains("/objects/") {
            if method == "PUT" {
                fake.puts += 1;
                return json(
                    403,
                    r#"{"ok":false,"error":{"message":"A share is read only"}}"#,
                );
            }
            if auth != fake.share_token || fake.revoked {
                return json(
                    403,
                    r#"{"ok":false,"error":{"message":"This share is no longer available"}}"#,
                );
            }
            let key = path.split("/objects/").nth(1).unwrap_or("");
            if key.contains("secret") {
                return json(
                    403,
                    r#"{"ok":false,"error":{"message":"This file is not part of the share"}}"#,
                );
            }
            if method == "GET" && key.ends_with("paper.pdf") {
                return (200, fake.pdf.clone());
            }
            return (404, Vec::new());
        }
        if auth != fake.session {
            return json(401, r#"{"ok":false,"error":{"message":"Sign in again"}}"#);
        }
        if path.ends_with("/shares") && method == "POST" {
            let posted = String::from_utf8_lossy(body);
            if !posted.contains("papers/attention") || !posted.contains("student@school.edu") {
                return json(400, r#"{"ok":false,"error":{"message":"bad share"}}"#);
            }
            if posted.contains(&fake.session) {
                return json(400, r#"{"ok":false,"error":{"message":"token in body"}}"#);
            }
            return json(
                200,
                r#"{"ok":true,"data":{"id":"share-1","libraryId":"lib-1","name":"Chapter","permission":"view","paperPaths":["papers/attention"]}}"#,
            );
        }
        if path.ends_with("/shares") && method == "GET" {
            return json(
                200,
                r#"{"ok":true,"data":{"shares":[{"id":"share-1","libraryId":"lib-1","name":"Chapter","permission":"view","paperPaths":["papers/attention"]}]}}"#,
            );
        }
        if path.contains("/revoke") {
            fake.revoked = true;
            return json(200, r#"{"ok":true,"data":{"id":"share-1"}}"#);
        }
        if path.ends_with("/library-shares/share-1") && method == "GET" {
            return json(
                200,
                r#"{"ok":true,"data":{"id":"share-1","libraryId":"lib-1","name":"Chapter","permission":"view","paperPaths":["papers/attention"],"files":[{"key":"papers/attention/paper.pdf","size":1}]}}"#,
            );
        }
        if path.ends_with("/credentials") {
            if fake.revoked {
                return json(
                    403,
                    r#"{"ok":false,"error":{"message":"This share is no longer available"}}"#,
                );
            }
            let token = fake.share_token.clone();
            return json(
                200,
                &format!(
                    r#"{{"ok":true,"data":{{"token":"{token}","mode":"read","libraryId":"lib-1","paperPaths":["papers/attention"]}}}}"#
                ),
            );
        }
        if path.ends_with("/library-shares") && method == "GET" {
            return json(
                200,
                r#"{"ok":true,"data":{"shares":[{"id":"share-1","libraryId":"lib-1","name":"Chapter","permission":"view","paperPaths":["papers/attention"]}]}}"#,
            );
        }
        json(404, r#"{"ok":false,"error":{"message":"missing"}}"#)
    }

    fn json(status: u16, body: &str) -> (u16, Vec<u8>) {
        (status, body.as_bytes().to_vec())
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
        response: (u16, Vec<u8>),
    ) -> std::io::Result<()> {
        let (status, body) = response;
        let head = format!(
            "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(&body)?;
        Ok(())
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentero-share-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn contains_token(dir: &Path, token: &str) -> bool {
        let mut pending = vec![dir.to_path_buf()];
        while let Some(path) = pending.pop() {
            let entries = fs::read_dir(&path).unwrap();
            for entry in entries.flatten() {
                let child = entry.path();
                if child.is_dir() {
                    pending.push(child);
                    continue;
                }
                if fs::read(&child)
                    .unwrap()
                    .windows(token.len())
                    .any(|window| window == token.as_bytes())
                {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn import_copies_papers_without_following_or_storing_tokens() {
        let dest = temp_dir("import");
        let vault_json = dest.join(".agentero/vault.json");
        fs::create_dir_all(vault_json.parent().unwrap()).unwrap();
        fs::write(&vault_json, r#"{"id":"local-library"}"#).unwrap();
        let note = dest.join("papers/attention/NOTES.md");
        fs::create_dir_all(note.parent().unwrap()).unwrap();
        fs::write(&note, "local note").unwrap();
        let config = temp_dir("config");
        let libraries = config.join("libraries.json");
        let sync = config.join("sync.json");
        fs::write(&libraries, r#"{"lib":{"path":"/vault","name":"Mine"}}"#).unwrap();
        fs::write(&sync, r#"{"/vault":{"backend":"s3"}}"#).unwrap();
        let libraries_before = fs::read(&libraries).unwrap();
        let sync_before = fs::read(&sync).unwrap();
        let token = "session-token-should-not-land";

        let written = import_granted_files(
            &dest,
            &["papers/attention".into()],
            &[
                ("papers/attention/paper.pdf".into(), b"pdf-v1".to_vec()),
                (
                    "papers/attention/metadata.json".into(),
                    br#"{"title":"Attention"}"#.to_vec(),
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            written,
            vec![
                "papers/attention/paper.pdf".to_string(),
                "papers/attention/metadata.json".to_string(),
            ]
        );
        assert_eq!(
            fs::read(dest.join("papers/attention/paper.pdf")).unwrap(),
            b"pdf-v1"
        );
        assert_eq!(fs::read_to_string(&note).unwrap(), "local note");
        assert_eq!(
            fs::read_to_string(&vault_json).unwrap(),
            r#"{"id":"local-library"}"#
        );
        assert!(!dest.join("papers/attention/.share.json").exists());
        assert!(!contains_token(&dest, token));
        assert_eq!(fs::read(&libraries).unwrap(), libraries_before);
        assert_eq!(fs::read(&sync).unwrap(), sync_before);
        let outside = import_granted_files(
            &dest,
            &["papers/attention".into()],
            &[("papers/other/secret.pdf".into(), b"secret".to_vec())],
        );
        assert!(outside.is_err());
        assert!(!dest.join("papers/other/secret.pdf").exists());
        assert!(normalize_paper_folder("papers").is_none());
        assert!(normalize_paper_folder("").is_none());
    }

    #[tokio::test]
    async fn later_open_sees_a_newer_pdf_and_revoke_keeps_the_import() {
        let session = "session-token-value";
        let share_token = "share-token-secret-value";
        let server = spawn(Fake {
            pdf: b"pdf-v1".to_vec(),
            revoked: false,
            puts: 0,
            session: session.into(),
            share_token: share_token.into(),
        });
        let created = create_share(
            &server.origin,
            session,
            "lib-1",
            "Chapter",
            "view",
            &["papers/attention".into()],
            &["Student@School.edu".into()],
        )
        .await
        .unwrap();
        assert_eq!(created.paper_paths, vec!["papers/attention".to_string()]);
        let owned = list_owned_shares(&server.origin, session, "lib-1")
            .await
            .unwrap();
        assert_eq!(owned[0].id, "share-1");
        assert!(create_share(
            &server.origin,
            session,
            "lib-1",
            "Empty",
            "view",
            &[],
            &["student@school.edu".into()],
        )
        .await
        .is_err());

        let dest = temp_dir("open");
        let config = temp_dir("open-config");
        let libraries = config.join("libraries.json");
        let sync = config.join("sync.json");
        fs::write(&libraries, "{\"keep\":true}").unwrap();
        fs::write(&sync, "{\"keep\":true}").unwrap();
        fs::create_dir_all(dest.join(".agentero")).unwrap();
        fs::write(dest.join(".agentero/vault.json"), "{\"id\":\"dest\"}").unwrap();

        let imported = import_share_into(&server.origin, session, "share-1", &dest)
            .await
            .unwrap();
        assert_eq!(
            imported.files,
            vec!["papers/attention/paper.pdf".to_string()]
        );
        assert_eq!(
            fs::read(dest.join("papers/attention/paper.pdf")).unwrap(),
            b"pdf-v1"
        );

        server.state.lock().unwrap().pdf = b"pdf-v2-newer".to_vec();
        let (_opened, again) = download_share_files(&server.origin, session, "share-1")
            .await
            .unwrap();
        assert_eq!(again[0].1, b"pdf-v2-newer");
        assert_eq!(
            fs::read(dest.join("papers/attention/paper.pdf")).unwrap(),
            b"pdf-v1"
        );

        revoke_share(&server.origin, session, "lib-1", "share-1")
            .await
            .unwrap();
        let failed = download_share_files(&server.origin, session, "share-1").await;
        assert!(failed.is_err());
        assert_eq!(
            fs::read(dest.join("papers/attention/paper.pdf")).unwrap(),
            b"pdf-v1"
        );
        assert_eq!(server.state.lock().unwrap().puts, 0);
        assert!(!contains_token(&dest, session));
        assert!(!contains_token(&dest, share_token));
        assert_eq!(fs::read_to_string(&libraries).unwrap(), "{\"keep\":true}");
        assert_eq!(fs::read_to_string(&sync).unwrap(), "{\"keep\":true}");
        assert_eq!(
            fs::read_to_string(dest.join(".agentero/vault.json")).unwrap(),
            "{\"id\":\"dest\"}"
        );
    }
}
