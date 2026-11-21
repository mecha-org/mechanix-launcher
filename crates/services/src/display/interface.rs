//! mxbrightnessd D-Bus names and match rules.
use mecha_wayland::prelude::*;

pub(super) const BRIGHTNESS: &str = "org.mechanix.Brightness";
pub(super) const PATH: &str = "/org/mechanix/Brightness";
pub(super) const INTERFACE: &str = "org.mechanix.Brightness";

pub(super) fn owner_changed() -> MatchRule {
    MatchRule::signal()
        .sender("org.freedesktop.DBus")
        .interface("org.freedesktop.DBus")
        .member("NameOwnerChanged")
        .arg(0, BRIGHTNESS)
}
