use mecha_wayland::mechanix_widgets::prelude::{Slider, SliderSize, Text, slider, text};
use mecha_wayland::prelude::*;

use crate::PowerMenu;

pub(crate) const SIDE_PADDING: f32 = 6.0;
pub(crate) const ICON_GAP: f32 = 4.0;
const SLIDER: SliderSize = SliderSize::XSmall;
const MIN_SLIDE: f32 = 0.5;

pub(crate) const ICON_SIZE: u32 = 24;
const HERO_ICON_SIZE: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    PowerOff,
    Restart,
    LogOut,
}

impl Action {
    pub(crate) const ALL: [Action; 3] = [Action::PowerOff, Action::Restart, Action::LogOut];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Action::PowerOff => "POWER OFF",
            Action::Restart => "RESTART",
            Action::LogOut => "LOG OUT",
        }
    }

    pub(crate) fn busy_label(self) -> &'static str {
        match self {
            Action::PowerOff => "POWERING OFF",
            Action::Restart => "RESTARTING",
            Action::LogOut => "LOGGING OUT",
        }
    }

    pub(crate) fn icon(self) -> &'static [u8] {
        match self {
            Action::PowerOff => assets::icons::POWER,
            Action::Restart => assets::icons::REBOOT,
            Action::LogOut => assets::icons::SIGN_OUT,
        }
    }

    pub(crate) fn is_hero(self) -> bool {
        self == Action::PowerOff
    }

    pub(crate) fn row_height(self) -> f32 {
        if self.is_hero() { 36.0 } else { 20.0 }
    }

    pub(crate) fn text_variant(self) -> TextVariant {
        if self.is_hero() {
            TextVariant::HeadlineLarge
        } else {
            TextVariant::HeadlineSmall
        }
    }

    pub(crate) fn slider_scale(self) -> f32 {
        if self.is_hero() { 1.0 } else { 0.75 }
    }

    pub(crate) fn thumb_height(self) -> f32 {
        SLIDER.thumb_height() * self.slider_scale()
    }

    pub(crate) fn tick_height(self) -> f32 {
        SLIDER.tick_height() * self.slider_scale()
    }

    pub(crate) fn icon_size(self) -> u32 {
        if self.is_hero() {
            HERO_ICON_SIZE
        } else {
            ICON_SIZE
        }
    }
}

pub(crate) struct Row {
    pub(crate) action: Action,
    pub(crate) node: Handle<Div>,
    pub(crate) fill: Handle<Div>,
    pub(crate) icon: Handle<Icon>,
    pub(crate) label: Handle<Text>,
    pub(crate) arrow: Handle<Icon>,
    pub(crate) slider: Handle<Slider>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Slide {
    pub(crate) row: usize,
    pub(crate) contact: ContactId,
    pub(crate) start_x: f32,
    pub(crate) progress: f32,
}
pub(crate) fn slide_progress(start_x: f32, x: f32, row_left: f32, row_width: f32) -> f32 {
    let span = (row_left + row_width - start_x).max(row_width * MIN_SLIDE);
    if !(span.is_finite() && span > 0.0) {
        return 0.0;
    }
    ((x - start_x) / span).clamp(0.0, 1.0)
}

pub(crate) fn padded_row(height: f32) -> LayoutStyle {
    LayoutStyle::default()
        .row()
        .align_items(Align::Center)
        .width(percent(100.0))
        .height(percent(height))
        .padding(Insets::new(
            px(0.0),
            percent(SIDE_PADDING),
            px(0.0),
            percent(SIDE_PADDING),
        ))
}

pub(crate) fn divider() -> DivBuilder {
    div().style(
        LayoutStyle::default()
            .width(percent(100.0))
            .height(px(1.0))
            .shrink(0.0),
    )
}

pub(crate) fn spawn_row(
    s: &mut Spawner<'_, PowerMenu>,
    parent: Handle<PowerMenu>,
    action: Action,
    arrow_sprite: SpriteId,
) -> Row {
    let node = s.spawn(
        parent,
        div().style(
            LayoutStyle::default()
                .relative()
                .width(percent(100.0))
                .height(percent(action.row_height())),
        ),
    );

    let fill = s.spawn(
        node,
        div()
            .style(
                LayoutStyle::default()
                    .hidden()
                    .absolute()
                    // Down to the slider.
                    .inset(Insets::new(
                        px(0.0),
                        auto(),
                        px(action.thumb_height()),
                        px(0.0),
                    ))
                    .width(percent(0.0)),
            )
            .background(s.color(ColorRole::SurfaceContainerLow)),
    );

    let content = s.spawn(node, div().style(padded_row(100.0).gap(percent(ICON_GAP))));
    let size = action.icon_size();
    let sprite = s
        .resource_mut::<Atlas>()
        .insert_svg(action.icon(), size)
        .expect("embedded icon SVG should render and fit in the atlas");
    let icon_node = s.spawn(
        content,
        icon(sprite)
            .size(Size::new(size as f32, size as f32))
            .color(s.color(ColorRole::OnSurface))
            .opaque(false)
            .style(LayoutStyle::default().shrink(0.0)),
    );
    let label = s.spawn(
        content,
        text(action.label())
            .variant(action.text_variant())
            .color(ColorRole::OnSurface)
            .opaque(false)
            .style(LayoutStyle::default().grow(1.0)),
    );
    let arrow = s.spawn(
        content,
        icon(arrow_sprite)
            .size(Size::new(ICON_SIZE as f32, ICON_SIZE as f32))
            .color(s.color(ColorRole::OnSecondaryFixed))
            .style(LayoutStyle::default().shrink(0.0)),
    );

    let slider = s.spawn(
        node,
        slider()
            .size(SLIDER)
            .tick_height(px(action.tick_height()))
            .thumb_height(action.thumb_height())
            .active_color(ColorRole::Primary)
            .thumb_color(ColorRole::OnSurface)
            .range(0.0, 1.0)
            .value(0.0)
            .length(percent(100.0))
            .show_value(false)
            .interactive(false)
            .style(
                LayoutStyle::default()
                    .hidden()
                    .absolute()
                    .inset(Insets::new(auto(), px(0.0), px(0.0), px(0.0))),
            ),
    );

    Row {
        action,
        node,
        fill,
        icon: icon_node,
        label,
        arrow,
        slider,
    }
}
