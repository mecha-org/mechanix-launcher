use mecha_wayland::prelude::*;

mod brightness;
mod volume;

use brightness::BrightnessSliderBuilder;
use volume::VolumeSliderBuilder;

const MIN_SLIDER_GAP: f32 = 15.0;
const MAX_SLIDER_GAP: f32 = 30.0;
const SLIDER_HEIGHT: f32 = 32.0;

/// single style, used for both volume and brightness sliders (and future
//sliders)
fn control_style() -> LayoutStyle {
    // Share the section equally, yielding room to the minimum slider gap.
    // The 24-unit icons set the lower bound for each control.
    LayoutStyle::default()
        .width(percent(100.0))
        .height(percent(50.0))
        .max_height(px(SLIDER_HEIGHT))
        .min_height(px(24.0))
        .shrink(1.0)
}

pub struct Sliders;

pub struct SlidersBuilder;

impl Build for SlidersBuilder {
    type Widget = Sliders;
}

impl Sliders {
    /// Portrait: take all the height left between the header and the
    /// bottom bar, and center the slider column vertically inside it.
    /// Landscape: take half the width beside the tiles, centered with them.
    pub fn style(landscape: bool) -> LayoutStyle {
        let style = LayoutStyle::default()
            .grow(1.0)
            .min_height(px(0.0))
            .column()
            .justify(Justify::Center);
        if landscape {
            style.flex_basis(px(0.0)).min_width(px(0.0))
        } else {
            style.width(percent(100.0))
        }
    }
}

impl Widget for Sliders {
    type Builder = SlidersBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = Sliders::style(false);

        let col = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .column()
                    .width(percent(100.0))
                    .height(percent(100.0))
                    .min_height(px(0.0))
                    .center(),
            ),
        );
        s.spawn(col, VolumeSliderBuilder);
        // This consumes spare height up to 40 units; beyond that the column
        // centers the controls, and at its minimum they still have 10 between.
        s.spawn(
            col,
            div().style(
                LayoutStyle::default()
                    .height(px(MIN_SLIDER_GAP))
                    .max_height(px(MAX_SLIDER_GAP))
                    .grow(1.0)
                    .shrink(0.0),
            ),
        );
        s.spawn(col, BrightnessSliderBuilder);

        Sliders
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT_SLIDER_HEIGHT: f32 = 28.0;
    const SHORT_SECTION_HEIGHT: f32 = 2.0 * SHORT_SLIDER_HEIGHT + MIN_SLIDER_GAP;

    fn slider_layout(height: f32) -> (Rect, Rect, Rect) {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(MechanixTheme::dark());
        app.insert_resource(Atlas::new());

        let window = app.spawn(
            app.root(),
            window().layout(LayoutStyle::default().column().size(px(308.0), px(height))),
        );
        let sliders = app.spawn(window, SlidersBuilder);
        for _ in 0..3 {
            app.tick();
        }

        let rect = |id| app.component::<ComputedLayout>(id).unwrap().rect;
        let col = app.children(sliders.id()).unwrap()[0];
        let controls = app.children(col).unwrap();
        assert_eq!(controls.len(), 3);
        for control in [controls[0], controls[2]] {
            assert_ticks_fill_control(&app, control);
        }
        (rect(sliders.id()), rect(controls[0]), rect(controls[2]))
    }

    fn assert_ticks_fill_control(app: &App, control: NodeId) {
        let rect = |id| app.component::<ComputedLayout>(id).unwrap().rect;
        let control_rect = rect(control);
        let row = app.children(control).unwrap()[0];
        let track = app.children(row).unwrap()[1];
        let ticks = app.children(track).unwrap();
        assert!(!ticks.is_empty());
        for &tick in ticks {
            let tick = rect(tick);
            assert_eq!(tick.origin.y, control_rect.origin.y);
            assert_eq!(tick.height(), control_rect.height());
        }
    }

    #[test]
    fn short_portrait_section_fits_rows_and_ticks_with_minimum_gap() {
        let (section, volume, brightness) = slider_layout(SHORT_SECTION_HEIGHT);
        assert!((volume.height() - SHORT_SLIDER_HEIGHT).abs() <= 1.0);
        assert!((brightness.height() - SHORT_SLIDER_HEIGHT).abs() <= 1.0);
        assert_eq!(
            brightness.origin.y - volume.origin.y - volume.height(),
            MIN_SLIDER_GAP
        );
        assert!(volume.origin.y >= section.origin.y);
        assert!(brightness.origin.y + brightness.height() <= section.origin.y + section.height());
    }

    #[test]
    fn ticks_follow_resizing_and_orientation_without_rebuilding_controls() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(MechanixTheme::dark());
        app.insert_resource(Atlas::new());
        let size = |h| LayoutStyle::default().column().size(px(308.0), px(h));
        let window = app.spawn(app.root(), window().layout(size(635.0)));
        let sliders = app.spawn(window, SlidersBuilder);
        let column = app.children(sliders).unwrap()[0];
        let children = app.children(column).unwrap();
        let controls = [children[0], children[2]];
        for (height, landscape, expected) in [
            (635.0, false, SLIDER_HEIGHT),
            (SHORT_SECTION_HEIGHT, false, SHORT_SLIDER_HEIGHT),
            (SHORT_SECTION_HEIGHT, true, SHORT_SLIDER_HEIGHT),
            (SHORT_SECTION_HEIGHT, false, SHORT_SLIDER_HEIGHT),
            (635.0, false, SLIDER_HEIGHT),
        ] {
            *app.component_mut::<LayoutStyle>(window).unwrap() = size(height);
            app.emit(
                screen::OrientationChanged(if landscape {
                    screen::Orientation::Landscape
                } else {
                    screen::Orientation::Portrait
                }),
                sliders,
            );
            for _ in 0..3 {
                app.tick();
            }
            for control in controls {
                let rect = app.component::<ComputedLayout>(control).unwrap().rect;
                assert!((rect.height() - expected).abs() <= 1.0, "{rect:?}");
                assert_ticks_fill_control(&app, control);
            }
        }
    }

    #[test]
    fn compact_sliders_keep_the_minimum_gap_between_controls() {
        let height = 2.0 * SLIDER_HEIGHT + MIN_SLIDER_GAP;
        let (section, volume, brightness) = slider_layout(height);
        assert_eq!(section.height(), height);
        assert_eq!(volume.height(), SLIDER_HEIGHT);
        assert_eq!(brightness.height(), SLIDER_HEIGHT);
        assert_eq!(
            brightness.origin.y - (volume.origin.y + volume.height()),
            MIN_SLIDER_GAP
        );
        assert!(volume.origin.y >= section.origin.y);
        assert!(brightness.origin.y + brightness.height() <= section.origin.y + section.height());
    }

    #[test]
    fn slider_gap_uses_the_height_between_compact_and_spacious() {
        let gap = ((MIN_SLIDER_GAP + MAX_SLIDER_GAP) / 2.0).round();
        let (_, volume, brightness) = slider_layout(2.0 * SLIDER_HEIGHT + gap);
        assert_eq!(
            brightness.origin.y - (volume.origin.y + volume.height()),
            gap
        );
    }

    #[test]
    fn spacious_sliders_keep_the_maximum_gap_between_controls_and_center_them() {
        let (section, volume, brightness) = slider_layout(635.0);
        assert_eq!(
            brightness.origin.y - (volume.origin.y + volume.height()),
            MAX_SLIDER_GAP
        );
        let above = volume.origin.y - section.origin.y;
        let below = section.origin.y + section.height() - brightness.origin.y - brightness.height();
        assert!(
            (above - below).abs() <= 1.0,
            "{section:?} {volume:?} {brightness:?}"
        );
    }
}
