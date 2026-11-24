use mecha_wayland::prelude::*;

const MPRIS_DBUS_INTERFACE: &str = "org.mpris.MediaPlayer2";

pub fn get_mpris_dbus_interface() -> MatchRule {
    MatchRule::signal()
        .sender("org.freedesktop.DBus")
        .interface("org.freedesktop.DBus")
        .member("NameOwnerChanged")
        .arg0_namespace(MPRIS_DBUS_INTERFACE)
}
