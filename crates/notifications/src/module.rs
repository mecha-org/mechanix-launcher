use mecha_wayland::prelude::*;

pub struct NotificationsModule;
use crate::{
    dbus::{NotificationServer, on_call},
    dbus_spec::register_notifications_interface,
    media::on_media_signal,
    notification_store::NotificationStore,
    notifications::on_store_changed,
    notify_signal::notify_signal,
};

impl Module for NotificationsModule {
    fn install(self, app: &mut App) {
        assert!(
            app.resource_mut::<DBus>().bus(BusId::Session).is_some(),
            "Notifications install after dbus session"
        );

        let export_id = register_notifications_interface(app);
        app.insert_resource(NotificationServer { export_id });

        app.system(notify_signal);
        app.system(on_call);
        app.system(on_store_changed);
        app.system(on_media_signal);

        app.init_resource::<NotificationStore>();
        crate::ongoing::install(app);
    }
}
