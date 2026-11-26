use super::PlayerId;
use mecha_wayland::prelude::*;

pub enum MediaCommand {
    Pause(PlayerId),
    PlayPause(PlayerId),
}

impl Signal for MediaCommand {}
