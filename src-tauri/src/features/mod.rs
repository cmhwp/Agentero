//! Domain features (feature-first layout, aligned with frontend `src/lib`).
//!
//! Tauri-free service bodies live in `agentero_core::features::*` and are
//! bridged here; callers use the semantic `crate::features::<domain>::<module>`
//! paths. This crate keeps the desktop shells: `#[tauri::command]` surfaces,
//! JobCenter runners, watcher/agent/integration wiring. The headless CLI
//! consumes `agentero_core` directly; BYOA (`agent`) is desktop-only.

#[cfg(feature = "desktop")]
pub mod agent;
#[cfg(feature = "desktop")]
#[path = "agent/install/mod.rs"]
pub mod cli_install;
#[cfg(feature = "desktop")]
pub mod compile;
#[cfg(feature = "desktop")]
pub mod decision;
#[cfg(feature = "desktop")]
pub mod host_hooks;
#[cfg(feature = "desktop")]
pub mod jobs;

#[cfg(feature = "desktop")]
pub mod library;

pub mod lifecycle;

#[path = "../app/open_request/mod.rs"]
pub mod open_request;

// jEV smart highlights extract text through the PDFium-backed `pdf::locate`,
// which is itself target-gated out on mobile; keep the whole feature off there.
#[cfg(all(
    feature = "desktop",
    not(any(target_os = "ios", target_os = "android"))
))]
pub mod jev;
pub mod markdown;
pub mod paper;
pub mod pdf;
#[cfg(all(
    feature = "desktop",
    not(any(target_os = "ios", target_os = "android"))
))]
pub mod rtex;
pub mod system;
pub mod translate;
pub mod vault;
#[cfg(feature = "desktop")]
pub mod web;
