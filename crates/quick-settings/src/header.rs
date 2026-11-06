use mecha_wayland::mechanix_widgets::prelude::{ButtonVariant, ClockInterval, button, clock};
use mecha_wayland::prelude::*;

use crate::power_overlay::{OpenPowerOverlay, POWER_ICON};
use crate::util::load_icon_sized;

/// Today's date with weekday, e.g. "Monday,2 Nov".
const DATE_FORMAT: &str = "%A,%-d %b";
const POWER_ICON_SIZE: u32 = 20;

pub struct Header;

pub struct HeaderBuilder;

impl Build for HeaderBuilder {
    type Widget = Header;
}

impl Widget for Header {
    type Builder = HeaderBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let side = px(Spacing::Space250.dp());
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .width(percent(100.0))
            .shrink(0.0)
            .row()
            .align_items(Align::Center)
            .justify(Justify::SpaceBetween)
            .padding(Insets::new(px(Spacing::Space300.dp()), side, px(0.0), side));

        // Ticks every second.
        s.spawn(
            me,
            clock(DATE_FORMAT)
                .interval(ClockInterval::Second)
                .variant(TextVariant::TitleSmall)
                .color(ColorRole::OnSurfaceVariant),
        );

        let sprite = load_icon_sized(&mut s.resource_mut::<Atlas>(), POWER_ICON, POWER_ICON_SIZE);

        let power = s.spawn(
            me,
            button()
                .variant(ButtonVariant::Text)
                .icon(sprite)
                .label("POWER")
                .icon_gap(px(10.0))
                .hover_background(Color::TRANSPARENT)
                .pressed_background(Color::TRANSPARENT)
                .height(px(32.))
                .text_color(ColorRole::OnSurface),
        );
        s.on::<Clicked>(power, |ctx, _| {
            ctx.emit(OpenPowerOverlay, ctx.tree().root());
        });

        Header
    }
}
