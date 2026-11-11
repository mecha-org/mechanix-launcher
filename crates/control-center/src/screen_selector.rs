use mecha_wayland::mechanix_widgets::prelude::{Text, TextContextExt, text};
use mecha_wayland::prelude::*;

use crate::app_screens::AppScreen;

#[cfg(test)]
mod tests;

const TOP: f32 = 20.0;
const TAB_HEIGHT: f32 = 47.0;
const MIN_TAB_SPACING_X: f32 = 2.0;
const MAX_TAB_SPACING_X: f32 = 16.0;
const TAB_PADDING_Y: f32 = 14.0;
const INDICATOR: f32 = 2.0;
const DIVIDER: f32 = 1.0;

pub struct ScreenSelectorBuilder {
    pub control_center: NodeId,
    pub selected: AppScreen,
}

#[derive(Clone, Copy)]
struct Tab {
    screen: AppScreen,
    label: Handle<Text>,
    indicator: Handle<Div>,
}

pub struct ScreenSelector {}

impl Build for ScreenSelectorBuilder {
    type Widget = ScreenSelector;
}

fn label_color(selected: bool) -> ColorRole {
    if selected {
        ColorRole::OnSurface
    } else {
        ColorRole::OnSurfaceVariant
    }
}

fn shown(on: bool) -> Display {
    if on { Display::Flex } else { Display::Hidden }
}

impl Widget for ScreenSelector {
    type Builder = ScreenSelectorBuilder;
    fn build(builder: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .shrink(0.0)
            .padding(Insets::new(px(TOP), px(0.0), px(0.0), px(0.0)));

        let row = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .size(percent(100.0), px(TAB_HEIGHT)),
            ),
        );

        let app_screen_selected = builder.selected;
        let indicator_color = s.color(ColorRole::Primary);
        let side_spacing = LayoutStyle::default()
            .width(px(MAX_TAB_SPACING_X))
            .min_width(px(MIN_TAB_SPACING_X))
            .max_width(px(MAX_TAB_SPACING_X))
            .shrink(1.0);
        let tabs = AppScreen::ALL.map(|screen| {
            let on = screen == app_screen_selected;
            let cell = s.spawn(
                row,
                div().style(
                    LayoutStyle::default()
                        .relative()
                        .row()
                        // Auto basis keeps each label's natural width in the
                        // allocation; spare width is shared between the tabs.
                        .grow(1.0)
                        .min_width(px(0.0))
                        .height(percent(100.0))
                        .align_items(Align::Center)
                        .justify(Justify::Center)
                        .padding(Insets::new(
                            px(TAB_PADDING_Y),
                            px(0.0),
                            px(TAB_PADDING_Y),
                            px(0.0),
                        )),
                ),
            );
            s.spawn(cell, div().style(side_spacing.clone()));
            let label = s.spawn(
                cell,
                text(screen.label())
                    .variant(TextVariant::HeadlineSmall)
                    .emphasised()
                    .style(LayoutStyle::default().shrink(0.0))
                    .color(label_color(on)),
            );
            s.spawn(cell, div().style(side_spacing.clone()));
            let indicator = s.spawn(
                cell,
                div().background(indicator_color).style(
                    LayoutStyle::default()
                        .absolute()
                        .inset(Insets::new(Val::Auto, px(0.0), px(0.0), px(0.0)))
                        .height(px(INDICATOR))
                        .display(shown(on)),
                ),
            );
            (
                cell,
                Tab {
                    screen,
                    label,
                    indicator,
                },
            )
        });

        let divider_color = s.color(ColorRole::Outline);
        s.spawn(
            me,
            div()
                .background(divider_color)
                .style(LayoutStyle::default().size(percent(100.0), px(DIVIDER))),
        );

        let control_center = builder.control_center;
        let all = tabs.map(|(_, tab)| tab);
        for (cell, tab) in tabs {
            s.on::<Clicked>(cell, move |ctx, _| {
                ctx.emit(tab.screen, control_center);
            });
        }
        s.on::<AppScreen>(control_center, move |ctx, selected| {
            select(ctx, &all, *selected);
        });

        Self {}
    }
}

fn select<W: Widget>(ctx: &mut Context<'_, W>, tabs: &[Tab], selected: AppScreen) {
    for tab in tabs {
        let on = tab.screen == selected;
        if let Some(mut c) = ctx.at(tab.label) {
            c.set_color(label_color(on));
        }
        if let Some(mut c) = ctx.at(tab.indicator)
            && let Some(mut style) = c.component_mut::<LayoutStyle>()
        {
            style.display = shown(on);
        }
    }
}
