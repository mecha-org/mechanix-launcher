use mecha_wayland::prelude::*;

use crate::notifications_control::NotificationsControlBuilder;
use crate::toggle::{Toggle, toggle_display};
use crate::{control::Control, style::space};

pub struct NotificationsControlListBuilder {
    pub notification_id: u32,
}

pub struct NotificationsControlList;

impl Build for NotificationsControlListBuilder {
    type Widget = NotificationsControlList;
}

impl Widget for NotificationsControlList {
    type Builder = NotificationsControlListBuilder;

    fn build(
        b: NotificationsControlListBuilder,
        me: Handle<Self>,
        s: &mut Spawner<'_, Self>,
    ) -> Self {
        // TODO: Match the figma styles
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .row()
            .align_items(Align::Center)
            .width(percent(1.0))
            .padding(Insets::new(
                px(space::CONTENT_TO_ACTIONS.dp()),
                px(0.0),
                px(0.0),
                px(0.0),
            ))
            .display(Display::Hidden)
            .gap(px(space::TITLE_TO_BODY.dp()));
        for control in Control::ALL {
            s.spawn(
                me,
                NotificationsControlBuilder {
                    control,
                    notification_id: b.notification_id,
                },
            );
        }

        s.on::<Toggle>(me, toggle_display);
        NotificationsControlList
    }
}
