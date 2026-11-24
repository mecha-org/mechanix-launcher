use super::PlayerId;
use mecha_wayland::prelude::*;

pub enum MediaSignals {
    NotifyPlayerArrival(PlayerId),
}

impl Signal for MediaSignals {}
