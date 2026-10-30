use mecha_wayland::mechanix_widgets::button;
use mecha_wayland::prelude::*;

use crate::notify_signal::NotificationSignal::SnoozeNotifiation;
use crate::snooze_controls::SnoozeControls;
use crate::style::space;

pub struct SnoozeBuilder {
    pub notification_id: u32,
    pub snooze_control: SnoozeControls,
}

pub struct Snooze;

impl Build for SnoozeBuilder {
    type Widget = Snooze;
}

impl Widget for Snooze {
    type Builder = SnoozeBuilder;

    fn build(b: SnoozeBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .gap(px(space::CARD_GAP.dp()));
        let btn = s.spawn(me, button().label(b.snooze_control.label()));

        s.on::<Clicked>(btn, move |ctx, _| {
            apply_snooze_controls(ctx, b.notification_id, b.snooze_control.millis().0);
        });
        Snooze {}
    }
}

fn apply_snooze_controls(ctx: &mut Context<'_, Snooze>, id: u32, expiry_timeout: u32) {
    ctx.signal(SnoozeNotifiation(id, expiry_timeout));
}
