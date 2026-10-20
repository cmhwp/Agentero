//! Named libraries on this computer.
//!
//! Cloud sync stays off. This module only remembers which local folder belongs
//! to a vault id and talks to the account library list.

mod api;
mod merge;
mod store;

pub use api::{fetch_libraries, fetch_session, register_library, sign_out, RemoteUser};
pub use merge::{merge_libraries, LibraryRow, RecentFolder, ServerLibrary};
pub use store::{load, normalize_library_path, upsert};
