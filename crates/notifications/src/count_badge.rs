use crate::style::{color, size, space, text};
use mecha_wayland::mechanix_widgets::prelude::TextContextExt;
use mecha_wayland::mechanix_widgets::{self as mw};
use mecha_wayland::prelude::*;

pub struct CountBadgeBuilder {
    pub count: usize,
}

pub struct CountBadge {
    label: Handle<mw::Text>,
}

pub struct SetCount(pub usize);
impl Event for SetCount {}

impl Build for CountBadgeBuilder {
    type Widget = CountBadge;
}

impl Widget for CountBadge {
    type Builder = CountBadgeBuilder;

    fn build(b: CountBadgeBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() =
            LayoutStyle::default().shrink(0.0).display(shown(b.count));

        let background = s.color(color::COUNT_BADGE);
        let pill = s.spawn(
            me,
            div()
                .background(background)
                .radius(size::COUNT_BADGE / 2.0)
                .style(
                    LayoutStyle::default()
                        .row()
                        .justify(Justify::Center)
                        .align_items(Align::Center)
                        .min_width(px(size::COUNT_BADGE))
                        .height(px(size::COUNT_BADGE))
                        .padding(Insets::new(
                            px(0.0),
                            px(space::MENU_ITEM_X.dp()),
                            px(0.0),
                            px(space::MENU_ITEM_X.dp()),
                        )),
                ),
        );
        let label = s.spawn(
            pill,
            mw::text(b.count.to_string())
                .variant(text::BADGE.variant)
                .color(text::BADGE.color)
                .max_lines(1),
        );

        s.on::<SetCount>(me, |ctx, e| {
            let label = ctx.me().label;
            if let Some(mut l) = ctx.at(label) {
                l.set_text(e.0.to_string());
            }
            ctx.set_display(shown(e.0));
        });

        CountBadge { label }
    }
}

fn shown(count: usize) -> Display {
    if count > 1 {
        Display::Flex
    } else {
        Display::Hidden
    }
}
