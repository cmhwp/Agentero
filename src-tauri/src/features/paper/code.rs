//! Paper code-repository actions.
//!
//! A paper's `code_repo` metadata remains the source link.  Cloning is an
//! explicit user action and stores a shallow working copy below the paper
//! folder, so the Vault remains portable and the source PDF is untouched.

use crate::core::error::{map_err, ApiResult, AppError};
use crate::core::fs::{ensure_vault_dir, resolve_paper_dir, resolve_vault};
use crate::core::process::{hide_console_window_tokio, resolve_command, windows_shell_path};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const CLONE_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PaperCodeCloneArgs {
    pub vault_path: String,
    /// Vault-relative paper folder, e.g. `papers/1706.03762`.
    pub paper_path: String,
    pub repo_url: String,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PaperCodeCloneResult {
    /// Absolute path of the local repository checkout.
    pub code_path: String,
    /// A text entry suitable for opening in the in-app code viewer.
    pub open_path: Option<String>,
    /// True when the checkout already existed and was reused.
    pub already_present: bool,
}

/// Clone a paper's GitHub repository into `papers/<paper>/code/<repo>`.
/// Local Vaults only; platforms without `git` on PATH get `git_unavailable`.
/// It never overwrites an existing checkout and disables Git's interactive
/// credential prompts so a GUI invocation cannot hang.
#[tauri::command]
#[specta::specta]
pub async fn paper_code_clone(args: PaperCodeCloneArgs) -> ApiResult<PaperCodeCloneResult> {
    let vault = match resolve_vault(&args.vault_path) {
        Ok(vault) => vault,
        Err(error) => return map_err(error),
    };
    let (paper_dir, _) = match resolve_paper_dir(&vault, &args.paper_path) {
        Ok(value) => value,
        Err(error) => return map_err(error),
    };
    if let Err(error) = ensure_vault_dir(&paper_dir) {
        return map_err(error);
    }
    let repo_url = match normalize_repo_url(&args.repo_url) {
        Ok(url) => url,
        Err(error) => return map_err(error),
    };
    let repo_name = repository_name(&repo_url);
    let code_root = paper_dir.join("code");
    let target = code_root.join(&repo_name);
    if target.exists() {
        if !target.join(".git").is_dir() {
            return map_err(AppError::message(format!(
                "code destination already exists: {}",
                target.display()
            )));
        }
        return ApiResult::ok(PaperCodeCloneResult {
            open_path: find_open_path(&target).map(|path| path.to_string_lossy().into_owned()),
            code_path: target.to_string_lossy().into_owned(),
            already_present: true,
        });
    }

    if let Err(error) = fs::create_dir_all(&code_root) {
        return map_err(AppError::message(format!(
            "create code directory failed: {error}"
        )));
    }
    let Some(git) = resolve_command("git") else {
        return map_err(AppError::domain(
            "git_unavailable",
            "Git is not installed or not available on PATH",
        ));
    };

    let mut command = Command::new(git);
    command
        .current_dir(windows_shell_path(&code_root))
        .args(["clone", "--depth", "1", &repo_url, &repo_name])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    hide_console_window_tokio(&mut command);

    let output = match tokio::time::timeout(CLONE_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return map_err(AppError::domain(
                "git_clone_failed",
                format!("failed to run git: {error}"),
            ));
        }
        Err(_) => {
            return map_err(AppError::domain(
                "git_clone_timeout",
                "Git clone timed out; check the repository URL and network",
            ));
        }
    };
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if detail.is_empty() {
            "Git clone failed".to_string()
        } else {
            format!("Git clone failed: {detail}")
        };
        // The target did not exist before this operation, so a partial clone
        // is safe to remove and a retry can start cleanly.
        if target.exists() {
            let _ = fs::remove_dir_all(&target);
        }
        return map_err(AppError::domain("git_clone_failed", message));
    }

    ApiResult::ok(PaperCodeCloneResult {
        open_path: find_open_path(&target).map(|path| path.to_string_lossy().into_owned()),
        code_path: target.to_string_lossy().into_owned(),
        already_present: false,
    })
}

fn normalize_repo_url(raw: &str) -> Result<String, AppError> {
    let value = raw.trim().trim_end_matches('/');
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return Err(AppError::message("GitHub repository URL is invalid"));
    }
    if let Some(rest) = value.strip_prefix("git@github.com:") {
        if !rest.starts_with('/') {
            let segments: Vec<_> = rest.split('/').take(2).collect();
            if segments.len() == 2 && segments.iter().all(|segment| !segment.is_empty()) {
                return Ok(format!("git@github.com:{}/{}", segments[0], segments[1]));
            }
        }
        return Err(AppError::message("GitHub repository URL is invalid"));
    }
    let parsed = url::Url::parse(value)
        .map_err(|_| AppError::message("GitHub repository URL is invalid"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.username() != ""
        || parsed.password().is_some()
    {
        return Err(AppError::message(
            "Only public HTTPS or SSH GitHub repositories can be cloned",
        ));
    }
    let segments: Vec<String> = parsed
        .path_segments()
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .collect();
    if segments.len() < 2 || segments[0].is_empty() || segments[1].is_empty() {
        return Err(AppError::message("GitHub repository URL is invalid"));
    }
    let mut normalized = parsed;
    normalized.set_path(&format!("/{}/{}", segments[0], segments[1]));
    normalized.set_query(None);
    normalized.set_fragment(None);
    Ok(normalized.to_string())
}

fn repository_name(url: &str) -> String {
    let raw = url
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("repository")
        .trim_end_matches(".git");
    let mut name: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if name.is_empty() || name == "." || name == ".." {
        name = "repository".to_string();
    }
    name
}

fn find_open_path(repo: &Path) -> Option<PathBuf> {
    const PREFERRED: &[&str] = &[
        "README.md",
        "README.rst",
        "README",
        "pyproject.toml",
        "package.json",
        "Cargo.toml",
        "setup.py",
        "main.py",
    ];
    for name in PREFERRED {
        let candidate = repo.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let mut candidates: Vec<PathBuf> = fs::read_dir(repo)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.'))
        })
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "c" | "cc"
                            | "cpp"
                            | "go"
                            | "h"
                            | "hpp"
                            | "java"
                            | "js"
                            | "jsx"
                            | "json"
                            | "md"
                            | "py"
                            | "rb"
                            | "rs"
                            | "sh"
                            | "swift"
                            | "ts"
                            | "tsx"
                            | "toml"
                            | "txt"
                            | "yaml"
                            | "yml"
                    )
                })
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_github_https_and_ssh_urls() {
        assert!(normalize_repo_url("https://github.com/openai/example").is_ok());
        assert_eq!(
            normalize_repo_url("https://github.com/openai/example/tree/main").unwrap(),
            "https://github.com/openai/example"
        );
        assert_eq!(
            normalize_repo_url("https://github.com/openai/example?tab=readme").unwrap(),
            "https://github.com/openai/example"
        );
        assert!(normalize_repo_url("git@github.com:openai/example.git").is_ok());
        assert!(normalize_repo_url("https://gitlab.com/openai/example").is_err());
    }

    #[test]
    fn derives_safe_repository_directory_name() {
        assert_eq!(
            repository_name("https://github.com/a/my-repo.git"),
            "my-repo"
        );
        assert_eq!(repository_name("git@github.com:a/repo"), "repo");
    }
}
