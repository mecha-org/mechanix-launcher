use crate::card::restyle;
use crate::card_drag::open_offset;
use crate::style::{color, space, text};
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

    fn style(self, width: f32) -> LayoutStyle {
        let fixed = LayoutStyle::default().width(px(width)).shrink(0.0).center();
        match self {
            Self::Options => fixed,
            Self::Dismiss => fixed.grow(1.0),
        }
    }
}

pub struct CardOptionsBuilder {
    pub card: NodeId,
}

pub struct CardOptions {
    buttons: [Handle<Div>; 2],
}

impl Build for CardOptionsBuilder {
    type Widget = CardOptions;
}

impl Widget for CardOptions {
    type Builder = CardOptionsBuilder;

    fn build(builder: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = style(0.0);
        s.component_mut::<Viewport>(me).unwrap().clip = true;

        let card = builder.card;
        let buttons = CardOption::ALL.map(|option| {
            let option_button = s.spawn(
                me,
                div()
                    .background(s.color(option.color()))
                    .style(option.style(0.0)),
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
            option_button
        });

        CardOptions { buttons }
    }
}

pub fn reveal(n: &mut Context<'_, CardOptions>, offset: f32, card_width: f32) {
    restyle(n, style(offset));
    let width = button_width(card_width);
    let buttons = n.me().buttons;
    for (option, button) in CardOption::ALL.into_iter().zip(buttons) {
        if let Some(mut b) = n.at(button) {
            restyle(&mut b, option.style(width));
        }
    }
}

fn button_width(card_width: f32) -> f32 {
    (open_offset(card_width) / 2.0 - space::SWIPE_GAP.dp()).max(0.0)
}

fn style(offset: f32) -> LayoutStyle {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::size;

    #[test]
    fn buttons_follow_the_card_and_stop_at_figma() {
        assert_eq!(button_width(524.0), size::OPTIONS_WIDTH, "Figma card");
        assert_eq!(button_width(1200.0), size::OPTIONS_WIDTH);
        let narrow = button_width(300.0);
        assert!(narrow > 0.0 && narrow < size::OPTIONS_WIDTH);
        // Options, Clear and their gaps fill exactly where the card rests.
        let gaps = 2.0 * space::SWIPE_GAP.dp();
        assert!((2.0 * narrow + gaps - open_offset(300.0)).abs() < 0.01);
    }
}
