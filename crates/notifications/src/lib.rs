use mecha_wayland::prelude::*;
use screen::Screen;

pub mod card;
pub mod card_drag;
pub mod card_icon;
pub mod card_list;
pub mod card_options;
pub mod count_badge;
pub mod dbus;
pub mod dbus_spec;
pub mod empty_notifications_screen;
pub mod group;
pub mod hints;
pub mod image;
pub mod markup;
mod media;
pub mod millis;
pub mod module;
pub mod notification_store;
pub mod notifications;
pub mod notify;
pub mod notify_signal;
mod ongoing;
pub mod options_menu;
pub mod popup;
pub mod rich_text;
pub mod snooze_controls;
pub mod style;

#[cfg(test)]
mod tests;

use crate::notifications::NotificationsCenterBuilder;
pub use module::NotificationsModule;

pub struct Notifications;

impl Screen for Notifications {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        app.spawn(parent, NotificationsCenterBuilder).id()
    }
}
