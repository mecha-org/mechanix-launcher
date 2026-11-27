use std::time::{Duration, Instant};

use mecha_wayland::prelude::*;

use super::interface::{
    ART_URL, ARTIST, LENGTH, METADATA, PAUSED, PLAYBACK_STATUS, PLAYING, POSITION, TITLE,
};
use super::types::{PlaybackPosition, PlaybackStatus, PlayerId, Track};

impl From<BusName> for PlayerId {
    fn from(name: BusName) -> Self {
        Self(name)
    }
}

impl From<&PropMap> for PlaybackStatus {
    fn from(props: &PropMap) -> Self {
        let status: Option<String> = prop(props, PLAYBACK_STATUS);
        match status.as_deref() {
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
        let track = Track {
            title: prop(&metadata, TITLE),
            artists: prop(&metadata, ARTIST),
            art_url: prop(&metadata, ART_URL),
            track_length: prop(&metadata, LENGTH),
        };
        return track;
    }
}

impl From<&PropMap> for PlaybackPosition {
    fn from(props: &PropMap) -> Self {
        let at: Option<Duration> = prop(props, POSITION);
        PlaybackPosition {
            at: at.unwrap_or_default(),
            since: Instant::now(),
        }
    }
}

trait FromPropMap {
    fn from_map(map: &PropMap, key: &str) -> Self
    where
        Self: Sized;
}

impl FromPropMap for Option<String> {
    fn from_map(map: &PropMap, key: &str) -> Self {
        let Some(Ok(text)) = map.get_as::<String>(key) else {
            return None;
        };
        Some(text)
    }
}

impl FromPropMap for Vec<String> {
    fn from_map(map: &PropMap, key: &str) -> Self {
        let Some(Ok(list)) = map.get_as::<Vec<String>>(key) else {
            return Vec::new();
        };
        list
    }
}

impl FromPropMap for Option<Duration> {
    fn from_map(map: &PropMap, key: &str) -> Self {
        let micros = match map.get(key)? {
            Value::I64(v) => u64::try_from(*v).ok()?,
            Value::U64(v) => *v,
            _ => return None,
        };
        Some(Duration::from_micros(micros))
    }
}

fn prop<T: FromPropMap>(map: &PropMap, key: &str) -> T {
    T::from_map(map, key)
}
