use mecha_wayland::prelude::*;

use super::interface::get_mpris_dbus_interface;

#[derive(Default)]
pub struct MprisBus {
    pub(super) names: Vec<String>,
}

impl Resource for MprisBus {}

pub struct MprisDiscovery {
    pub(super) list_call: CallId,
    pub(super) owner_match: MatchId,
}

impl Resource for MprisDiscovery {}

impl MprisDiscovery {
    pub fn new(bus: &mut Bus) -> Self {
        let owner_match = bus.add_match(get_mpris_dbus_interface());
        let list_call = bus.list_names();
        Self {
            list_call,
            owner_match,
        }
    }
}
