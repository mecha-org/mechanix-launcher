//! Apps starting: tapped, their window not up yet.
//!
//! The drawer starts apps and keeps [`Launches`]; other screens show them,
//! and ask for one's placeholder ([`ShowLaunch`]) or for it to stop
//! ([`StopLaunch`]).

use mecha_wayland::prelude::*;

use crate::{DesktopApp, RowIcon};

/// Identifies one launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LaunchId(u64);

/// One app starting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub id: LaunchId,
    pub app: DesktopApp,
    /// Its icon, already loaded.
    pub icon: RowIcon,
}

/// The apps starting now, the newest first. [`LaunchesChanged`] is
/// signalled whenever this changes.
#[derive(Debug, Default)]
pub struct Launches {
    launches: Vec<Launch>,
    next: u64,
}
impl Resource for Launches {}

impl Launches {
    pub fn iter(&self) -> impl Iterator<Item = &Launch> {
        self.launches.iter()
    }

    pub fn get(&self, id: LaunchId) -> Option<&Launch> {
        self.launches.iter().find(|l| l.id == id)
    }

    /// `app` starting: its launch, listed first.
    pub fn add(&mut self, app: DesktopApp, icon: RowIcon) -> LaunchId {
        let id = LaunchId(self.next);
        self.next += 1;
        self.launches.insert(0, Launch { id, app, icon });
        id
    }

    /// The launch done with: its window came, it stopped, or it gave up.
    /// Whether it was listed.
    pub fn remove(&mut self, id: LaunchId) -> bool {
        let before = self.launches.len();
        self.launches.retain(|l| l.id != id);
        self.launches.len() != before
    }
}

/// [`Launches`] changed.
pub struct LaunchesChanged;
impl Signal for LaunchesChanged {}

/// Asks for the launch's placeholder to be shown.
pub struct ShowLaunch(pub LaunchId);
impl Signal for ShowLaunch {}

/// Asks for the launch to be stopped: the app's start called off.
pub struct StopLaunch(pub LaunchId);
impl Signal for StopLaunch {}
