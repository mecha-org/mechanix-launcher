use mecha_wayland::prelude::*;
use screen::Screen;

pub mod card;
pub mod card_list;
pub mod control;
pub mod dbus;
pub mod dbus_spec;
pub mod markup;
pub mod millis;
pub mod module;
pub mod notification_store;
pub mod notifications;
pub mod notifications_control;
pub mod notifications_control_center;
pub mod notify;
pub mod notify_signal;
pub mod rich_text;
pub mod snooze;
pub mod snooze_center;
pub mod snooze_controls;
pub mod style;
pub mod toggle;

use crate::notifications::NotificationsCenterBuilder;
pub use module::NotificationsModule;

pub struct Notifications;

impl Screen for Notifications {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        app.spawn(parent, NotificationsCenterBuilder).id()
    }
}
