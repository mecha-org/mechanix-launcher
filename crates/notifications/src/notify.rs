use crate::markup::Markup;
use crate::millis::Millis;
use crate::{
    notification_store::{Entry, NotificationStore},
    notify_signal::NotificationSignal::{AddNotification, UpdateNotification},
};
use mecha_wayland::prelude::*;
use std::time::SystemTime;

#[derive(Default, Debug, Clone)]
pub struct Notification {
    pub desktop_entry: String,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: Markup,
    pub actions: Vec<(String, String)>,
    pub expire_timeout: Option<Millis>,
    pub hints: Categories,
}

#[derive(Default, Debug, Clone)]
pub enum Categories {
    #[default]
    LOW,
    NORMAL,
    CRITICAL,
}

#[derive(Debug)]
pub struct Notify {
    pub replaces_id: Option<u32>,
    pub notification: Notification,
}

impl Signal for Notify {}

pub type NotifyArgs = (
    String,
    u32,
    String,
    String,
    String,
    Vec<String>,
    PropMap,
    i32,
);

impl From<NotifyArgs> for Notify {
    fn from(args: NotifyArgs) -> Self {
        let (app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout) = args;

        let replaces_id = (replaces_id != 0).then_some(replaces_id);

        let actions = actions
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect();

        let urgency = match hints.get_as::<u8>("urgency") {
            Some(Ok(0)) => Categories::LOW,
            Some(Ok(2)) => Categories::CRITICAL,
            _ => Categories::NORMAL,
        };
        let desktop_entry = hints
            .get_as::<String>("desktop-entry")
            .and_then(Result::ok)
            .unwrap_or_default();

        let body = Markup::parse(&body);

        Notify {
            replaces_id,
            notification: Notification {
                desktop_entry,
                app_name,
                app_icon,
                summary,
                body,
                actions,
                expire_timeout: Millis::from_expire_timeout(expire_timeout),
                hints: urgency,
            },
        }
    }
}

pub fn append_notifications(app: &mut App, notify: &Notify) -> u32 {
    let replaced_notification = replace_notification(app, notify);
    if let Some(id) = replaced_notification {
        app.signal(UpdateNotification(id));
        return id;
    }
    let id = add_notification(app, notify);
    app.signal(AddNotification(id));
    id
}

fn replace_notification(app: &mut App, notify: &Notify) -> Option<u32> {
    let mut notifications_store = app.resource_mut::<NotificationStore>();
    notify.replaces_id.and_then(|replaces_id| {
        let entry = notifications_store
            .entries
            .iter_mut()
            .find(|entry| entry.id == replaces_id)?;
        entry.notification = notify.notification.clone();
        entry.received_time = SystemTime::now();
        Some(replaces_id)
    })
}

fn add_notification(app: &mut App, notify: &Notify) -> u32 {
    let mut notifications_store = app.resource_mut::<NotificationStore>();
    notifications_store.next_id += 1;
    let id = notifications_store.next_id;
    notifications_store.entries.push(Entry {
        id,
        received_time: SystemTime::now(),
        notification: notify.notification.clone(),
        // no controls applied yet
        control: None,
    });
    id
}
