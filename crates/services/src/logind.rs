//! systemd-logind's names on the system bus.

pub(crate) const LOGIND: &str = "org.freedesktop.login1";
pub(crate) const MANAGER_PATH: &str = "/org/freedesktop/login1";
pub(crate) const MANAGER: &str = "org.freedesktop.login1.Manager";
pub(crate) const SESSION: &str = "org.freedesktop.login1.Session";
pub(crate) const NO_SUCH_SESSION: &str = "org.freedesktop.login1.NoSuchSession";
