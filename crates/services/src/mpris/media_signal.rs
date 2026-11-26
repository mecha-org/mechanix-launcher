use super::PlayerId;
use mecha_wayland::prelude::*;

pub enum MediaSignals {
    NotifyPlayerArrival(PlayerId),
    PlayerChanged(PlayerId),
    PlayerRemoved(PlayerId),
}

impl Signal for MediaSignals {}
