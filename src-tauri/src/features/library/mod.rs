//! Named libraries: keychain session plus the local id→path map.
//! The sync engine is not involved.

#[cfg(feature = "desktop")]
pub mod commands;
#[cfg(feature = "desktop")]
mod loopback;
#[cfg(feature = "desktop")]
mod session;
