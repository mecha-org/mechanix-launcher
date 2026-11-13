use crate::card::{Card, SnoozeCard, UpdateCard};
use crate::card_list::{AddEntry, CardList, ClearGroup};
use crate::dbus::{DbusSignal, emit_signal};
use crate::dbus_spec::NOTIFIFICATION_DISMISSED;
use crate::group::RemoveCard;
use crate::millis::Millis;
use crate::notification_store::NotificationStore;
use crate::notifications::NotificationsCenter;
use crate::options_menu::ShowOptionsMenu;
use NotificationSignal::SnoozeNotifiation;
use mecha_wayland::prelude::*;

// this is app wide signal, not the the dbus signal
pub enum NotificationSignal {
    AddNotification(u32, String),
    RemoveNotification(u32, u32),
    UpdateNotification(u32),
    CloseAllNotifications,
    CloseGroupNotifications(String),
    SnoozeNotifiation(u32, u32),
    ShowOptions(u32, Rect),
}

pub fn notify_signal(app: &mut App, notify_signal: &NotificationSignal) {
    match notify_signal {
        NotificationSignal::AddNotification(id, _) => {
            let Some((list, _)) = app.widgets::<CardList>().next() else {
                return;
            };
            let Some(entry) = app.resource::<NotificationStore>().get(*id).cloned() else {
                return;
            };
            app.emit(AddEntry(entry), list);
        }
        NotificationSignal::UpdateNotification(id) => {
            let Some((card, _)) = app
                .widgets::<Card>()
                .find(|(_, card)| card.notification_id == *id)
            else {
                return;
            };
            let Some(entry) = app.resource::<NotificationStore>().get(*id).cloned() else {
                return;
            };
            app.emit(UpdateCard(entry), card);
        }
        NotificationSignal::RemoveNotification(id, reason) => {
            app.resource_mut::<NotificationStore>().remove(*id);
            let Some((card, _)) = app
                .widgets::<Card>()
                .find(|(_, card)| card.notification_id == *id)
            else {
                return;
            };
            emit_signal(app, &DbusSignal::NotificationClosed(*id, *reason));

            if let Some(group) = app.parent(card).and_then(|cards| app.parent(cards)) {
                app.emit(RemoveCard(card), group);
            } else {
                app.remove(card);
            }
        }
        NotificationSignal::CloseAllNotifications => {
            let removed_ids: Vec<u32> = app.resource_mut::<NotificationStore>().drain_ids();

            let Some((list, _)) = app.widgets::<CardList>().next() else {
                return;
            };
            let cards = app
                .children(list)
                .map(<[NodeId]>::to_vec)
                .unwrap_or_default();
            for card in cards {
                app.remove(card);
            }
            for id in removed_ids {
                emit_signal(
                    app,
                    &DbusSignal::NotificationClosed(id, NOTIFIFICATION_DISMISSED),
                );
            }
        }
        SnoozeNotifiation(id, expiry_timeout) => {
            let Some((card, _)) = app
                .widgets::<Card>()
                .find(|(_, card)| card.notification_id == *id)
            else {
                return;
            };
            app.emit(SnoozeCard(Millis(*expiry_timeout)), card);
        }
        NotificationSignal::ShowOptions(id, anchor) => {
            let Some(menu) = app
                .widgets::<NotificationsCenter>()
                .next()
                .map(|(_, center)| center.menu)
            else {
                return;
            };
            app.emit(
                ShowOptionsMenu {
                    notification_id: *id,
                    anchor: *anchor,
                },
                menu,
            );
        }
        NotificationSignal::CloseGroupNotifications(app_name) => {
            let removed_ids = app.resource_mut::<NotificationStore>().remove_app(app_name);

            let Some((list, _)) = app.widgets::<CardList>().next() else {
                return;
            };
            app.emit(ClearGroup(app_name.clone()), list);

            for id in removed_ids {
                emit_signal(
                    app,
                    &DbusSignal::NotificationClosed(id, NOTIFIFICATION_DISMISSED),
                );
            }
        }
    }
}

impl Signal for NotificationSignal {}
