use crate::style::{color, space, text};
use mecha_wayland::mechanix_widgets as mw;
use mecha_wayland::prelude::*;

pub struct EmptyNotificationsScreenBuilder;
pub struct EmptyNotificationsScreen;

impl Build for EmptyNotificationsScreenBuilder {
    type Widget = EmptyNotificationsScreen;
}

impl Widget for EmptyNotificationsScreen {
    type Builder = EmptyNotificationsScreenBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .center()
            .width(percent(100.0))
            .grow(1.0)
            .gap(px(space::EMPTY_ICON_TO_TEXT.dp()));
        // As tall as the caption's line, so it follows the theme's type scale.
        let side = s.typography(text::EMPTY.variant).line_height.round() as u32;
        let bell = s
            .resource_mut::<Atlas>()
            .insert_svg(assets::icons::BELL, side)
            .expect("embedded bell is a valid SVG");
        s.spawn(
            me,
            icon(bell)
                .color(s.color(color::EMPTY_ICON))
                .style(LayoutStyle::default().shrink(0.0)),
        );
        s.spawn(
            me,
            mw::text("No new notifications.")
                .variant(text::EMPTY.variant)
                .color(text::EMPTY.color),
        );
        EmptyNotificationsScreen
    }
}
