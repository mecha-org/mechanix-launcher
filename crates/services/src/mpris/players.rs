use mecha_wayland::prelude::*;

use super::interface::{METADATA, PLAYBACK_STATUS};
use super::types::{MediaPlayer, PlaybackPosition, PlaybackStatus, PlayerId, Track};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct MprisPlayers {
    pub(super) players: Vec<MediaPlayer>,
}
impl Resource for MprisPlayers {}

impl MprisPlayers {
    pub fn players(&self) -> &[MediaPlayer] {
        &self.players
    }

    pub fn get(&self, id: &PlayerId) -> Option<&MediaPlayer> {
        self.players.iter().find(|p| p.id == *id)
    }

    pub(super) fn add_player(&mut self, player: MediaPlayer) {
        self.remove_player(&player.id);
        self.players.push(player);
    }

    pub(super) fn remove_player(&mut self, id: &PlayerId) {
        self.players.retain(|p| p.id != *id);
    }

    pub(super) fn player_data_changed(&mut self, id: &PlayerId, changed: &PropMap) -> bool {
        let Some(player) = self.players.iter_mut().find(|p| p.id == *id) else {
            return false;
        };
        let mut updated = false;
        if changed.contains_key(PLAYBACK_STATUS) {
            player.position = PlaybackPosition {
                at: player.position_now(),
                since: Instant::now(),
            };
            player.status = PlaybackStatus::from(changed);
            updated = true;
        }
        if changed.contains_key(METADATA) {
            player.track = Track::from(changed);
            updated = true;
        }
        updated
    }

    pub(super) fn set_position(&mut self, id: &PlayerId, at: Duration) -> bool {
        let Some(player) = self.players.iter_mut().find(|p| p.id == *id) else {
            return false;
        };
        player.position = PlaybackPosition {
            at,
            since: Instant::now(),
        };
        true
    }
}

impl MediaPlayer {
    pub fn position_now(&self) -> Duration {
        let PlaybackPosition { at, since } = self.position;
        let pos = match self.status {
            PlaybackStatus::Playing => at.saturating_add(since.elapsed()),
            _ => at,
        };
        let Some(length) = self.track.track_length else {
            return pos;
        };
        pos.min(length)
    }

    pub fn time_to_next_second(&self) -> Option<Duration> {
        if self.status != PlaybackStatus::Playing {
            return None;
        }
        let pos = self.position_now();
        if self.track.track_length.is_some_and(|length| pos >= length) {
            return None;
        }
        let left = 1_000_000_000 - pos.subsec_nanos();
        Some(Duration::from_nanos(u64::from(left)))
    }
}
