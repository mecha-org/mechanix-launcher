use crate::card_list::CardListBuilder;
use crate::empty_notifications_screen::EmptyNotificationsScreenBuilder;
use crate::media::MediaListBuilder;
use crate::notification_store::NotificationStore;
use crate::notify_signal::NotificationSignal;
use crate::ongoing::OngoingListBuilder;
use crate::options_menu::{OptionsMenu, OptionsMenuBuilder};
use crate::style::{color, size, space, text};
use mecha_wayland::mechanix_widgets::{self as mw, button};
use mecha_wayland::prelude::*;

pub struct NotificationsCenter {
    pub menu: Handle<OptionsMenu>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum CenterView {
    Content,
    Empty,
}

impl From<&NotificationStore> for CenterView {
    fn from(store: &NotificationStore) -> Self {
        if store.entries.is_empty() {
            Self::Empty
        } else {
            Self::Content
        }
    }
}

pub struct ShowView(pub CenterView);
impl Event for ShowView {}

pub(crate) fn on_store_changed(app: &mut App, _: &OnChanged<NotificationStore>) {
    let view = CenterView::from(app.resource::<NotificationStore>());
    let centers: Vec<_> = app
        .widgets::<NotificationsCenter>()
        .map(|(id, _)| id)
        .collect();
    for center in centers {
        app.emit(ShowView(view), center);
    }
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

        s.spawn(me, MediaListBuilder);
        s.spawn(me, OngoingListBuilder);
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
        let content = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .column()
                    .width(percent(100.0))
                    .grow(1.0)
                    .min_height(px(0.0))
                    .hidden(),
            ),
        );
        let scroll = s.spawn(
            content,
            scrollable().style(
                LayoutStyle::default()
                    .block()
                    .width(percent(100.0))
                    .grow(1.0)
                    .min_height(px(0.0)),
            ),
        );
        s.spawn(scroll, CardListBuilder);
        let empty = s.spawn(me, EmptyNotificationsScreenBuilder);
        let menu = s.spawn(me, OptionsMenuBuilder);

        s.on::<ShowView>(me, move |ctx, e| match e.0 {
            CenterView::Content => {
                set_display(ctx, content, Display::Flex);
                set_display(ctx, empty, Display::Hidden);
            }
            CenterView::Empty => {
                set_display(ctx, content, Display::Hidden);
                set_display(ctx, empty, Display::Flex);
            }
        });

        NotificationsCenter { menu }
    }
}

fn set_display<W: Widget, T: Widget>(ctx: &mut Context<'_, W>, node: Handle<T>, display: Display) {
    let Some(mut n) = ctx.at(node) else {
        return;
    };
    if let Some(mut style) = n.component_mut::<LayoutStyle>()
        && style.display != display
    {
        style.display = display;
    }
}
