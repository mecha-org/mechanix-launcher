use mecha_wayland::prelude::*;

use super::interface::{ARTIST, METADATA, PAUSED, PLAYBACK_STATUS, PLAYING, TITLE};
use super::types::{PlaybackStatus, Track};

impl From<&PropMap> for PlaybackStatus {
    fn from(props: &PropMap) -> Self {
        match text(props, PLAYBACK_STATUS).as_deref() {
            Some(PLAYING) => PlaybackStatus::Playing,
            Some(PAUSED) => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        }
    }
}

impl From<&PropMap> for Track {
    fn from(props: &PropMap) -> Self {
        let Some(Ok(metadata)) = props.get_as::<PropMap>(METADATA) else {
            return Track::default();
        };
        Track {
            title: text(&metadata, TITLE),
            artists: list(&metadata, ARTIST),
        }
    }
}

fn text(map: &PropMap, key: &str) -> Option<String> {
    let Some(Ok(text)) = map.get_as::<String>(key) else {
        return None;
    };
    Some(text)
}

fn list(map: &PropMap, key: &str) -> Vec<String> {
    let Some(Ok(list)) = map.get_as::<Vec<String>>(key) else {
        return Vec::new();
    };
    list
}
