//! Whether a connected folder uses the shared-library on-disk layout.
//!
//! A folder is shared only when the roster's `billingSource` is `org_shared`
//! and that source has been remembered on the local library map. Personal and
//! `org_grant` libraries keep today's files.

use std::path::Path;

use super::account::{load_account_id, safe_member_id};
use super::store::load;
use crate::features::sync::local::read_vault_id;

pub const ORG_SHARED: &str = "org_shared";

/// Signed-in member context for one vault. `user_id` is the web account id,
/// not a device id. It is `None` when nobody is signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberLayout {
    pub shared: bool,
    pub user_id: Option<String>,
}

impl MemberLayout {
    pub fn personal() -> Self {
        Self {
            shared: false,
            user_id: None,
        }
    }
}

pub fn billing_source_for_vault(vault: &Path) -> Option<String> {
    let id = read_vault_id(vault)?;
    let map = load().ok()?;
    map.get(&id).and_then(|entry| entry.billing_source.clone())
}

pub fn layout_for(vault: &Path) -> MemberLayout {
    let shared = billing_source_for_vault(vault).as_deref() == Some(ORG_SHARED);
    let user_id = load_account_id().filter(|id| safe_member_id(id).is_some());
    MemberLayout { shared, user_id }
}
