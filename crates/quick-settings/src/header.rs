use mecha_wayland::mechanix_widgets::prelude::{ButtonVariant, ClockInterval, button, clock};
use mecha_wayland::prelude::*;

use crate::power_overlay::{OpenPowerOverlay, POWER_ICON};
use crate::util::{full_width, load_icon_sized};

pub const HEADER_HEIGHT: f32 = 80.0;
const PORTRAIT_HEADER_PERCENT: f32 = 8.5;
// The power button's computed height is 40, despite its requested 32-unit height.
const MIN_HEADER_HEIGHT: f32 = 40.0;
/// Today's date with weekday, e.g. "Monday,2 Nov".
const DATE_FORMAT: &str = "%A,%-d %b";
const POWER_ICON_SIZE: u32 = 20;

pub struct Header;

pub struct HeaderBuilder;

impl Build for HeaderBuilder {
    type Widget = Header;
}

impl Header {
    pub fn style(landscape: bool) -> LayoutStyle {
        let height = if landscape {
            full_width(HEADER_HEIGHT)
        } else {
            LayoutStyle::default()
                .width(percent(100.0))
                .height(percent(PORTRAIT_HEADER_PERCENT))
                .min_height(px(MIN_HEADER_HEIGHT))
                .max_height(px(HEADER_HEIGHT))
                .shrink(0.0)
        };
        height
            .row()
            .align_items(Align::Center)
            .justify(Justify::SpaceBetween)
            .padding(Insets::symmetric(px(20.0), px(0.0)))
    }
}

impl Widget for Header {
    type Builder = HeaderBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = Header::style(false);

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

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_wayland::mechanix_widgets::prelude::FontBook;

    #[test]
    fn portrait_header_uses_a_bounded_percentage_and_landscape_stays_fixed() {
        let portrait = Header::style(false);
        assert_eq!(portrait.height, percent(PORTRAIT_HEADER_PERCENT));
        assert_eq!(portrait.min_height, px(MIN_HEADER_HEIGHT));
        assert_eq!(portrait.max_height, px(HEADER_HEIGHT));
        assert_eq!(portrait.flex_shrink, 0.0);

        let landscape = Header::style(true);
        assert_eq!(landscape.height, px(HEADER_HEIGHT));
        assert_eq!(landscape.flex_shrink, 0.0);
    }

    #[test]
    fn portrait_header_resizes_without_clipping_its_contents() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(MechanixTheme::dark())
            .add_module(RingModule::default())
            .add_module(TimerModule);
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));

        let size = |height| LayoutStyle::default().size(px(308.0), px(height));
        let window = app.spawn(app.root(), window().layout(size(958.0)));
        let header = app.spawn(window, HeaderBuilder);

        for (panel_height, expected_height) in [(958.0, 80.0), (600.0, 51.0), (230.0, 40.0)] {
            *app.component_mut::<LayoutStyle>(window).unwrap() = size(panel_height);
            app.tick();

            let rect = |id| app.component::<ComputedLayout>(id).unwrap().rect;
            let header_rect = rect(header.id());
            assert_eq!(header_rect.height(), expected_height);
            for &child in app.children(header.id()).unwrap() {
                let content = rect(child);
                assert!(
                    content.origin.y >= header_rect.origin.y,
                    "{header_rect:?} {content:?}"
                );
                assert!(
                    content.origin.y + content.height()
                        <= header_rect.origin.y + header_rect.height(),
                    "{header_rect:?} {content:?}"
                );
            }
        }
    }
}
