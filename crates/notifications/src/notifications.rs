use crate::card_list::CardListBuilder;
use crate::notify_signal::NotificationSignal;
use crate::options_menu::{OptionsMenu, OptionsMenuBuilder};
use crate::style::{color, size, space, text};
use mecha_wayland::mechanix_widgets::{self as mw, button};
use mecha_wayland::prelude::*;

pub struct NotificationsCenter {
    pub menu: Handle<OptionsMenu>,
}

pub struct NotificationsCenterBuilder;

impl Build for NotificationsCenterBuilder {
    type Widget = NotificationsCenter;
}

impl Widget for NotificationsCenter {
    type Builder = NotificationsCenterBuilder;
    fn build(_: NotificationsCenterBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .relative()
            .column()
            .width(percent(100.0))
            .height(percent(100.0))
            .padding(Insets::new(
                px(space::HEADER_TOP.dp()),
                px(0.0),
                px(0.0),
                px(0.0),
            ))
            .gap(px(space::HEADER_TO_LIST.dp()));

        let header = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .justify(Justify::SpaceBetween)
                    .align_items(Align::Center)
                    .width(percent(100.0))
                    .padding(Insets::new(
                        px(0.0),
                        px(space::HEADER_X.dp()),
                        px(0.0),
                        px(space::HEADER_X.dp()),
                    )),
            ),
        );

        s.spawn(
            header,
            mw::text("Today")
                .variant(text::DATE_HEADER.variant)
                .color(text::DATE_HEADER.color),
        );
        let close_all_notifications = s.spawn(
            header,
            button()
                .variant(mw::ButtonVariant::Text)
                .size(mw::ButtonSize::Medium)
                .text_color(color::CLEAR_ALL)
                .height(px(size::CLEAR_ALL_HEIGHT))
                .padding(Insets::new(
                    px(space::CLEAR_ALL_Y.dp()),
                    px(space::CLEAR_ALL_X.dp()),
                    px(space::CLEAR_ALL_Y.dp()),
                    px(space::CLEAR_ALL_X.dp()),
                ))
                .label("CLEAR ALL"),
        );
        s.on::<Clicked>(close_all_notifications, move |ctx, _| {
            ctx.signal(NotificationSignal::CloseAllNotifications)
        });
        s.spawn(me, CardListBuilder);
        let menu = s.spawn(me, OptionsMenuBuilder);

        NotificationsCenter { menu }
    }
}
