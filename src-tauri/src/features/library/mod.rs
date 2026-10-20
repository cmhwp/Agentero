//! Named libraries: keychain session plus the local id→path map.
//! A library with no S3, WebDAV, or Git entry in `sync.json` syncs through
//! the managed Worker store.

#[cfg(feature = "desktop")]
pub mod commands;
#[cfg(feature = "desktop")]
mod loopback;
#[cfg(feature = "desktop")]
pub(crate) mod session;
