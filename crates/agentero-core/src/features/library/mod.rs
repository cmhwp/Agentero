//! Named libraries on this computer.
//!
//! Cloud sync stays off. This module only remembers which local folder belongs
//! to a vault id and talks to the account library list.

mod account;
mod api;
mod layout;
mod merge;
mod share;
mod store;

pub use account::{clear_account_id, load_account_id, safe_member_id, store_account_id};
pub use api::{
    fetch_libraries, fetch_session, register_library, set_library_billing, sign_out,
    RemoteOrgQuota, RemoteUser,
};
pub use layout::{layout_for, MemberLayout, ORG_SHARED};
pub use merge::{merge_libraries, LibraryRow, RecentFolder, ServerLibrary};
pub use share::{
    comment_on_share, create_share, download_share_files, import_granted_files, import_share_into,
    list_owned_shares, list_received_shares, open_share, revoke_share, ImportedPapers,
    ShareComment, ShareFileInfo, ShareOpened, ShareSummary,
};
pub use store::{load, normalize_library_path, remember_billing_sources, upsert};
