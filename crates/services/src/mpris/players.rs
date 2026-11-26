use mecha_wayland::prelude::*;

use super::interface::{METADATA, PLAYBACK_STATUS};
use super::types::{MediaPlayer, PlaybackStatus, PlayerId, Track};

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
            player.status = PlaybackStatus::from(changed);
            updated = true;
        }
        if changed.contains_key(METADATA) {
            player.track = Track::from(changed);
            updated = true;
        }
        updated
    }
}
