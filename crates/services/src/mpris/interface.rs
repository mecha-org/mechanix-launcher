use mecha_wayland::prelude::*;

const DBUS_DAEMON: &str = "org.freedesktop.DBus";
pub(super) const NAME_OWNER_CHANGED: &str = "NameOwnerChanged";

const MPRIS_DBUS_INTERFACE: &str = "org.mpris.MediaPlayer2";
pub(super) const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
pub(super) const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
pub(super) const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

pub(super) const PAUSE: &str = "Pause";
pub(super) const PLAY_PAUSE: &str = "PlayPause";

pub(super) const PLAYBACK_STATUS: &str = "PlaybackStatus";
pub(super) const METADATA: &str = "Metadata";

pub(super) const PLAYING: &str = "Playing";
pub(super) const PAUSED: &str = "Paused";

pub(super) const TITLE: &str = "xesam:title";
pub(super) const ARTIST: &str = "xesam:artist";
pub(super) const ART_URL: &str = "mpris:artUrl";

pub fn get_mpris_dbus_interface() -> MatchRule {
    MatchRule::signal()
        .sender(DBUS_DAEMON)
        .interface(DBUS_DAEMON)
        .member(NAME_OWNER_CHANGED)
        .arg0_namespace(MPRIS_DBUS_INTERFACE)
}
