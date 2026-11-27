use super::PlayerId;
use mecha_wayland::prelude::*;

pub enum MediaSignals {
    NotifyPlayerArrival(PlayerId),
    PlayerChanged(PlayerId),
    PlayerRemoved(PlayerId),
    PositionChanged(PlayerId),
}

impl Signal for MediaSignals {}
