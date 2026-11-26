use mecha_wayland::prelude::*;

use super::bus::{MprisDiscovery, PendingCall};
use super::interface::{MPRIS_PREFIX, NAME_OWNER_CHANGED};
use super::types::PlayerId;

pub(super) enum MprisEvent {
    PlayerNames(Vec<String>),
    PlayerData(PlayerId, PropMap),
    PlayerAppeared(BusName),
    PlayerLeft(BusName),
    PlayerUpdated(PlayerId, PropMap),
}

impl MprisEvent {
    pub(super) fn from_reply(app: &mut App, e: &MethodReply) -> Option<Self> {
        let call = app
            .resource_mut::<MprisDiscovery>()
            .pending_replies
            .remove(&e.call)?;
        let msg = match &e.result {
            Ok(msg) => msg,
            Err(error) => {
                eprintln!("[mpris] {call:?} failed: {error:?}");
                return None;
            }
        };

        match call {
            PendingCall::ListNames => {
                let Ok((names,)) = msg.body::<(Vec<String>,)>() else {
                    eprintln!("[mpris] ListNames: unexpected reply");
                    return None;
                };
                Some(Self::PlayerNames(names))
            }
            PendingCall::GetAll(id) => {
                let Ok((props,)) = msg.body::<(PropMap,)>() else {
                    eprintln!("[mpris] GetAll {}: unexpected reply", id.0.as_str());
                    return None;
                };
                Some(Self::PlayerData(id, props))
            }
        }
    }

    pub(super) fn from_signal(app: &App, e: &SignalReceived) -> Option<Self> {
        let member = e.msg.member().map(|m| m.as_str());
        if member == Some(NAME_OWNER_CHANGED) {
            return Self::from_owner_changed(e);
        }

        let discovery = app.resource::<MprisDiscovery>();
        let id = e
            .matches
            .iter()
            .find_map(|m| discovery.player_subscriptions.get(m))?
            .clone();
        let Ok((_, changed, _)) = PropertiesChanged::parse(&e.msg) else {
            eprintln!(
                "[mpris] PropertiesChanged {}: unexpected body",
                id.0.as_str()
            );
            return None;
        };
        Some(Self::PlayerUpdated(id, changed))
    }

    fn from_owner_changed(e: &SignalReceived) -> Option<Self> {
        let Ok((name, _old, new)) = e.msg.body::<(String, String, String)>() else {
            return None;
        };
        if !name.starts_with(MPRIS_PREFIX) {
            return None;
        }
        let name = BusName::try_from(name).ok()?;
        if new.is_empty() {
            return Some(Self::PlayerLeft(name));
        }
        Some(Self::PlayerAppeared(name))
    }
}
