use mecha_wayland::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerId(pub(super) String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Track {
    pub title: Option<String>,
    pub artists: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlayer {
    pub id: PlayerId,
    pub status: PlaybackStatus,
    pub track: Track,
}

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
}
