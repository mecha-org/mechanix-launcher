use std::collections::HashMap;

use mecha_wayland::prelude::*;

use super::interface::{MPRIS_PATH, PLAYER_INTERFACE, get_mpris_dbus_interface};
use super::types::PlayerId;

#[derive(Debug)]
pub enum PendingCall {
    ListNames,
    GetAll(PlayerId),
}

pub struct MprisDiscovery {
    pub(super) pending_replies: HashMap<CallId, PendingCall>,
    pub(super) player_subscriptions: HashMap<MatchId, PlayerId>,
}

impl Resource for MprisDiscovery {}

impl MprisDiscovery {
    pub fn new(bus: &mut Bus) -> Self {
        bus.add_match(get_mpris_dbus_interface());
        let list_call = bus.list_names();
        let mut pending_replies = HashMap::new();

        pending_replies.insert(list_call, PendingCall::ListNames);
        Self {
            pending_replies,
            player_subscriptions: HashMap::new(),
        }
    }

    pub(super) fn subscribe_player(app: &mut App, name: BusName) {
        if app.resource::<MprisDiscovery>().is_subscribed(&name) {
            return;
        }
        let (subscription, call) = subscribe_on_bus(app.resource_mut::<DBus>().session(), &name);
        app.resource_mut::<MprisDiscovery>()
            .add_subscription(&name, subscription, call);
    }

    pub(super) fn unsubscribe_player(app: &mut App, name: &BusName) {
        let subscription = app
            .resource_mut::<MprisDiscovery>()
            .remove_subscription(name);
        let Some(subscription) = subscription else {
            return;
        };
        app.resource_mut::<DBus>()
            .session()
            .remove_match(subscription);
    }

    pub(super) fn is_subscribed(&self, name: &BusName) -> bool {
        self.player_subscriptions.values().any(|id| id.0 == *name)
    }

    fn add_subscription(&mut self, name: &BusName, subscription: MatchId, call: CallId) {
        let id = PlayerId::from(name.clone());
        self.player_subscriptions.insert(subscription, id.clone());
        self.pending_replies.insert(call, PendingCall::GetAll(id));
    }

    fn remove_subscription(&mut self, name: &BusName) -> Option<MatchId> {
        let subscription = self
            .player_subscriptions
            .iter()
            .find(|(_, id)| id.0 == *name)
            .map(|(subscription, _)| *subscription)?;
        self.player_subscriptions.remove(&subscription);
        Some(subscription)
    }
}

fn subscribe_on_bus(mut bus: Bus<'_>, name: &BusName) -> (MatchId, CallId) {
    let subscription = bus.watch_properties(name.clone(), MPRIS_PATH, PLAYER_INTERFACE);
    let call = bus
        .object(name.clone(), MPRIS_PATH)
        .get_all(PLAYER_INTERFACE);
    (subscription, call)
}
