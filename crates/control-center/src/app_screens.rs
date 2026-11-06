use mecha_wayland::prelude::{App, Event, NodeId};
use notifications::Notifications;
use quick_settings::QuickSettingsScreen;
use screen::Screen;

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppScreen {
    #[default]
    NotificationsScreen,
    QuickSettingsScreen,
}

impl AppScreen {
    pub const ALL: [AppScreen; 2] = [
        AppScreen::NotificationsScreen,
        AppScreen::QuickSettingsScreen,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AppScreen::NotificationsScreen => "NOTIFICATIONS",
            AppScreen::QuickSettingsScreen => "SETTINGS",
        }
    }

    pub fn build(self, app: &mut App, parent: NodeId) -> NodeId {
        match self {
            AppScreen::NotificationsScreen => Notifications.build(app, parent),
            AppScreen::QuickSettingsScreen => QuickSettingsScreen.build(app, parent),
        }
    }
}

impl Event for AppScreen {}
