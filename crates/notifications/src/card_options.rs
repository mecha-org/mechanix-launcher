use crate::style::{color, size, space, text};
use mecha_wayland::mechanix_widgets as mw;
use mecha_wayland::prelude::*;

#[derive(Clone, Copy)]
pub enum CardOption {
    Options,
    Dismiss,
}

pub struct CardOptionsPressed {
    pub option: CardOption,
    pub anchor: Option<Rect>,
}
impl Event for CardOptionsPressed {}

impl CardOption {
    pub const ALL: [Self; 2] = [Self::Options, Self::Dismiss];

    fn label(self) -> &'static str {
        match self {
            Self::Options => "Options",
            Self::Dismiss => "Clear",
        }
    }

    fn color(self) -> ColorRole {
        match self {
            Self::Options => color::OPTIONS,
            Self::Dismiss => color::CLEAR,
        }
    }

    fn style(self) -> LayoutStyle {
        let fixed = LayoutStyle::default()
            .width(px(size::OPTIONS_WIDTH))
            .shrink(0.0);
        match self {
            Self::Options => fixed,
            Self::Dismiss => fixed.grow(1.0),
        }
    }
}

pub struct CardOptionsBuilder {
    pub card: NodeId,
}

pub struct CardOptions {}

impl Build for CardOptionsBuilder {
    type Widget = CardOptions;
}

impl Widget for CardOptions {
    type Builder = CardOptionsBuilder;

    fn build(builder: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = style(0.0);
        s.component_mut::<Viewport>(me).unwrap().clip = true;

        let card = builder.card;
        for option in CardOption::ALL {
            let option_button = s.spawn(
                me,
                div()
                    .background(s.color(option.color()))
                    .style(option.style().center()),
            );
            s.spawn(
                option_button,
                mw::text(option.label())
                    .variant(text::SWIPE_BUTTON.variant)
                    .color(text::SWIPE_BUTTON.color)
                    .max_lines(1),
            );
            s.on::<Clicked>(option_button, move |ctx, _| {
                let anchor = match option {
                    CardOption::Options => ctx
                        .at(option_button)
                        .and_then(|b| b.component::<ComputedLayout>().map(|l| l.rect)),
                    CardOption::Dismiss => None,
                };
                ctx.emit(CardOptionsPressed { option, anchor }, card);
            });
        }

        CardOptions {}
    }
}

pub fn style(offset: f32) -> LayoutStyle {
    let gap = space::SWIPE_GAP.dp();
    let width = (-offset - gap).max(0.0);
    LayoutStyle::default()
        .row()
        .absolute()
        .inset(Insets::new(px(0.0), px(0.0), px(0.0), auto()))
        .width(px(width))
        .gap(px(gap))
        .display(if width > 0.0 {
            Display::Flex
        } else {
            Display::Hidden
        })
}
