use mecha_wayland::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerId(pub(super) String);

#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlayer {
    pub id: PlayerId,
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
