//! systemd unit names: the unit a process runs in, the app a unit is for,
//! and the escaping both ways.

use std::fmt::Write;

/// The app id in the name of the systemd unit `pid` runs in, when that is
/// an app's unit; see [`unit_app_id`].
pub(crate) fn scope_app_id(pid: u32) -> Option<String> {
    unit_app_id(&unit_of(pid)?)
}

/// The systemd unit `pid` runs in, when that is an app's own unit (see
/// [`unit_app_id`]): ending that unit ends the app, and nothing else.
pub fn app_unit(pid: u32) -> Option<String> {
    unit_of(pid).filter(|unit| unit_app_id(unit).is_some())
}

/// The last part of `pid`'s cgroup path: its unit, if systemd made it.
pub fn unit_of(pid: u32) -> Option<String> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    Some(path.rsplit('/').next()?.to_owned())
}

/// The app id in an app's unit name, as freedesktop names them and the
/// portals read them: `app[-<launcher>]-<id>-<random>.scope`, or
/// `app[-<launcher>]-<id>[@<random>|-autostart].service`. The id's own
/// dashes are escaped (`\x2d`), so a dash ends it.
///
/// Also the units dbus-broker starts an activated app in,
/// `dbus-<its own connection>-<bus name>@<n>.service`: an app that takes
/// D-Bus activation has its desktop id as its bus name.
pub fn unit_app_id(unit: &str) -> Option<String> {
    let id = if let Some(rest) = unit.strip_prefix("dbus-:") {
        let (_connection, named) = rest.strip_suffix(".service")?.split_once('-')?;
        named.split_once('@')?.0
    } else {
        let rest = unit.strip_prefix("app-")?;
        let named = if let Some(scope) = rest.strip_suffix(".scope") {
            scope.rsplit_once('-')?.0
        } else {
            let service = rest.strip_suffix(".service")?;
            let service = service.split_once('@').map_or(service, |(id, _)| id);
            service.strip_suffix("-autostart").unwrap_or(service)
        };
        // The launcher, when there is one, is a plain word.
        match named.split_once('-') {
            Some((launcher, id)) if launcher.chars().all(|c| c.is_ascii_alphanumeric()) => id,
            _ => named,
        }
    };
    unescape_unit(id).filter(|id| !id.is_empty())
}

/// `s` escaped as systemd escapes a unit name's parts: anything but ASCII
/// letters, digits, `_`, `.` and `:` as `\xNN` (so `-` is `\x2d`).
pub fn escape_unit(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"_.:".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "\\x{b:02x}");
        }
    }
    out
}

/// Undo systemd's `\xNN` escaping of a unit name.
fn unescape_unit(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find("\\x") {
        out.extend_from_slice(&rest.as_bytes()[..at]);
        let hex = rest.get(at + 2..at + 4)?;
        out.push(u8::from_str_radix(hex, 16).ok()?);
        rest = &rest[at + 4..];
    }
    out.extend_from_slice(rest.as_bytes());
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names_give_the_app_id() {
        let cases = [
            (
                "app-mechanix-org.gnome.Nautilus-42.scope",
                Some("org.gnome.Nautilus"),
            ),
            (
                "app-gnome-org.gnome.Evolution\\x2dalarm\\x2dnotify-2437.scope",
                Some("org.gnome.Evolution-alarm-notify"),
            ),
            ("app-firefox-1234.scope", Some("firefox")),
            ("app-org.kde.dolphin@a1b2.service", Some("org.kde.dolphin")),
            ("app-gnome-foo-autostart.service", Some("foo")),
            (
                "dbus-:1.3-org.gnome.Loupe@0.service",
                Some("org.gnome.Loupe"),
            ),
            (
                "dbus-:1.3-org.example.foo\\x2dbar@12.service",
                Some("org.example.foo-bar"),
            ),
            ("dbus-broker.service", None),
            ("mechanix-shell.service", None),
            ("session-2.scope", None),
        ];
        for (unit, want) in cases {
            assert_eq!(unit_app_id(unit).as_deref(), want, "{unit}");
        }
    }

    #[test]
    fn escaping_comes_back_undone() {
        for s in ["org.gnome.Nautilus", "foot-server", "a b/c", "Ärger"] {
            assert_eq!(unescape_unit(&escape_unit(s)).as_deref(), Some(s), "{s}");
        }
        assert_eq!(escape_unit("foot-server"), "foot\\x2dserver");
    }
}
