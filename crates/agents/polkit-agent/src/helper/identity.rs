//! Who a request is authenticated as.

use nix::unistd::{Uid, User};

use crate::signals::Identity;

/// Who to authenticate as: `own`, else root, else the first listed.
pub(super) fn choose_uid(identities: &[Identity], own: u32) -> Option<u32> {
    let users: Vec<u32> = identities
        .iter()
        .filter_map(|i| match i {
            Identity::User(uid) => Some(*uid),
            _ => None,
        })
        .collect();
    [own, 0]
        .into_iter()
        .find(|u| users.contains(u))
        .or_else(|| users.first().copied())
}

pub(super) fn own_uid() -> u32 {
    Uid::current().as_raw()
}

pub(super) fn user_name(uid: u32) -> Option<String> {
    User::from_uid(Uid::from_raw(uid))
        .ok()
        .flatten()
        .map(|u| u.name)
        .filter(|n| !n.is_empty())
}
