use std::collections::HashMap;

use mecha_wayland::prelude::*;

use super::interface::get_mpris_dbus_interface;

#[derive(Default)]
pub struct MprisBus {
    pub(super) names: Vec<BusName>,
}

impl Resource for MprisBus {}

pub enum PendingCall {
    ListNames,
    GetAll(String),
}

pub struct MprisDiscovery {
    pub(super) owner_match: MatchId,
    pub(super) pending: HashMap<CallId, PendingCall>,
}

impl Resource for MprisDiscovery {}

impl MprisDiscovery {
    pub fn new(bus: &mut Bus) -> Self {
        let owner_match = bus.add_match(get_mpris_dbus_interface());
        let list_call = bus.list_names();
        let mut pending = HashMap::new();
        pending.insert(list_call, PendingCall::ListNames);
        Self {
            owner_match,
            pending,
        }
    }
}
