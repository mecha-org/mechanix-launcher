use crate::card::{Card, CardBuilder, SnoozeCard, UpdateCard};
use crate::card_list::CardList;
use crate::dbus::{DbusSignal, emit_signal};
use crate::dbus_spec::NOTIFIFICATION_DISMISSED;
use crate::millis::Millis;
use crate::notification_store::NotificationStore;
use NotificationSignal::SnoozeNotifiation;
use mecha_wayland::prelude::*;

// this is app wide signal, not the the dbus signal
pub enum NotificationSignal {
    AddNotification(u32),
    RemoveNotification(u32, u32),
    UpdateNotification(u32),
    CloseAllNotifications,
    SnoozeNotifiation(u32, u32),
}

pub fn notify_signal(app: &mut App, notify_signal: &NotificationSignal) {
    match notify_signal {
        NotificationSignal::AddNotification(id) => {
            let Some((list, _)) = app.widgets::<CardList>().next() else {
                return;
            };
            let Some(entry) = app
                .resource::<NotificationStore>()
                .entries
                .iter()
                .find(|e| e.id == *id)
                .cloned()
            else {
                return;
            };
            app.spawn(list, CardBuilder { entry });
        }
        NotificationSignal::UpdateNotification(id) => {
            let Some((card, _)) = app
                .widgets::<Card>()
                .find(|(_, card)| card.notification_id == *id)
            else {
                return;
            };
            let Some(entry) = app
                .resource::<NotificationStore>()
                .entries
                .iter()
                .find(|e| e.id == *id)
                .cloned()
            else {
                return;
            };
            app.emit(UpdateCard(entry), card);
        }
        NotificationSignal::RemoveNotification(id, reason) => {
            app.resource_mut::<NotificationStore>()
                .entries
                .retain(|e| e.id != *id);
            let Some((card, _)) = app
                .widgets::<Card>()
                .find(|(_, card)| card.notification_id == *id)
            else {
                return;
            };
            emit_signal(app, &DbusSignal::NotificationClosed(*id, *reason));
            app.remove(card);
        }
        NotificationSignal::CloseAllNotifications => {
            let removed_ids: Vec<u32> = app
                .resource_mut::<NotificationStore>()
                .entries
                .drain(..)
                .map(|x| x.id)
                .collect();

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
    }
}

impl Signal for NotificationSignal {}
