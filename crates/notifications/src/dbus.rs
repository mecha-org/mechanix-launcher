use mecha_wayland::{
    dbus::{ExportId, MethodCallReceived},
    prelude::*,
};
use std::env;

use crate::{
    dbus_spec::{self, NOTIFICATIONS_CLOSED},
    notify::{Notify, NotifyArgs, append_notifications},
    notify_signal::NotificationSignal::RemoveNotification,
};

pub struct NotificationServer {
    pub export_id: ExportId,
}

pub enum DbusSignal {
    NotificationClosed(u32, u32),
}

impl Resource for NotificationServer {}

pub fn on_call(app: &mut App, e: &MethodCallReceived) {
    if e.export != app.resource::<NotificationServer>().export_id {
        return;
    }

    match e.call.member().as_str() {
        "GetCapabilities" => {
            app.resource_mut::<DBus>()
                .reply(&e.call, &(CAPABILITIES.to_vec(),));
        }
        "GetServerInformation" => {
            app.resource_mut::<DBus>()
                .reply(&e.call, &server_information());
            return;
        }
        "Notify" => {
            let notify_args: Notify = e.call.args::<NotifyArgs>().unwrap().into();
            let id = append_notifications(app, &notify_args);
            app.resource_mut::<DBus>().reply(&e.call, &(id,));
        }
        "CloseNotification" => {
            let (id,) = e.call.args::<(u32,)>().unwrap();
            app.signal(RemoveNotification(id, NOTIFICATIONS_CLOSED));
            app.resource_mut::<DBus>().reply(&e.call, &());
        }
        _ => return,
    };
}

/// Markup is parsed; cards stay until dismissed or expired. No actions yet.
const CAPABILITIES: &[&str] = &["body", "body-markup", "persistence"];

/// Name, vendor, version and spec version.
fn server_information() -> (&'static str, &'static str, &'static str, &'static str) {
    (
        dbus_spec::SERVICE_NAME,
        dbus_spec::VENDOR,
        env!("CARGO_PKG_VERSION"),
        dbus_spec::SPEC_VERSION,
    )
}

pub fn emit_signal(app: &mut App, signal: &DbusSignal) {
    match signal {
        DbusSignal::NotificationClosed(id, reason) => {
            app.resource_mut::<DBus>().session().emit_signal(
                dbus_spec::OBJECT,
                dbus_spec::INTERFACE,
                "NotificationClosed",
                &(id, reason),
            );
        }
    }
}
