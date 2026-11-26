use mecha_wayland::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerId(pub(super) BusName);

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
    pub art_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlayer {
    pub id: PlayerId,
    pub status: PlaybackStatus,
    pub track: Track,
}
