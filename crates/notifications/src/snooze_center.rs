use mecha_wayland::prelude::*;

use crate::snooze::SnoozeBuilder;
use crate::snooze_controls::SNOOZE_OPTIONS;
use crate::style::space;
use crate::toggle::{Toggle, toggle_display};

pub struct SnoozeListBuilder {
    pub notification_id: u32,
}

pub struct SnoozeList;

impl Build for SnoozeListBuilder {
    type Widget = SnoozeList;
}

impl Widget for SnoozeList {
    type Builder = SnoozeListBuilder;

    fn build(b: SnoozeListBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .display(Display::Hidden)
            .gap(px(space::CARD_GAP.dp()));
        for snooze_control in SNOOZE_OPTIONS {
            s.spawn(
                me,
                SnoozeBuilder {
                    notification_id: b.notification_id,
                    snooze_control,
                },
            );
        }
        s.on::<Toggle>(me, toggle_display);

        SnoozeList
    }
}
