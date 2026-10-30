use crate::control::Control;
use crate::dbus_spec::NOTIFIFICATION_DISMISSED;
use crate::notify_signal::NotificationSignal::RemoveNotification;
use crate::snooze_center::SnoozeListBuilder;
use crate::toggle::Toggle;
use mecha_wayland::mechanix_widgets::button;
use mecha_wayland::prelude::*;

pub struct NotificationsControlBuilder {
    pub notification_id: u32,
    pub control: Control,
}

pub struct NotificationsControl {}

impl Build for NotificationsControlBuilder {
    type Widget = NotificationsControl;
}

impl Widget for NotificationsControl {
    type Builder = NotificationsControlBuilder;

    fn build(b: NotificationsControlBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let btn = s.spawn(me, button().label(b.control.label()));
        match b.control {
            Control::Snooze => {
                let snooze_list = s.spawn(
                    me,
                    SnoozeListBuilder {
                        notification_id: b.notification_id,
                    },
                );
                s.on::<Clicked>(btn, move |ctx, _| {
                    ctx.emit(Toggle, snooze_list);
                });
            }
            _ => {
                s.on::<Clicked>(btn, move |ctx, _| {
                    apply_controls(ctx, b.notification_id, &b.control);
                });
            }
        }

        Self {}
    }
}

fn apply_controls(ctx: &mut Context<'_, NotificationsControl>, id: u32, control: &Control) {
    match control {
        Control::Dismiss => ctx.signal(RemoveNotification(id, NOTIFIFICATION_DISMISSED)),
        _ => {}
    }
}
