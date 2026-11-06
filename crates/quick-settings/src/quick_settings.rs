use mecha_wayland::prelude::*;
use screen::{Orientation, OrientationChanged};

use crate::bottom_bar::{BottomBar, BottomBarBuilder};
use crate::header::HeaderBuilder;
use crate::sliders::{Sliders, SlidersBuilder};

const SECTION_GAP: f32 = 31.0;

/// The panel: header on top and, under it, a body holding the sliders and
/// the tiles. The drag handle under it all is the control center's.
///
/// The body follows the launcher's [`Orientation`]: in portrait the sliders
/// sit above the tiles; in landscape they sit side by side, so the
/// sections no longer have to share the short height.
pub struct QuickSettings {
    body: Handle<Div>,
    sliders: Handle<Sliders>,
    bottom_bar: Handle<BottomBar>,
    /// The layout currently applied.
    landscape: bool,
}

pub struct QuickSettingsBuilder {
    pub header: HeaderBuilder,
    pub bottom_bar: BottomBarBuilder,
    pub sliders: SlidersBuilder,
}

impl Build for QuickSettingsBuilder {
    type Widget = QuickSettings;
}

/// Portrait: a column whose sliders grow into the space above the tiles.
/// Landscape: a row of sliders and tiles, centered on each other.
fn body_style(landscape: bool) -> LayoutStyle {
    let style = LayoutStyle::default()
        .width(percent(100.0))
        .grow(1.0)
        .min_height(px(0.0))
        .gap(px(SECTION_GAP));
    if landscape {
        style.row().align_items(Align::Center)
    } else {
        style.column()
    }
}

impl Widget for QuickSettings {
    type Builder = QuickSettingsBuilder;

    fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .fill()
            .width(percent(100.0))
            .height(percent(100.0))
            .column();

        // The header keeps its fixed height; the body takes the rest. It
        // builds portrait; `OrientationChanged` says when it is not.
        s.spawn(me, b.header);
        let body = s.spawn(me, div().style(body_style(false)));
        let sliders = s.spawn(body, b.sliders);
        let bottom_bar = s.spawn(body, b.bottom_bar);

        s.on::<OrientationChanged>(me, |ctx, e| fit_orientation(ctx, e.0));

        QuickSettings {
            body,
            sliders,
            bottom_bar,
            landscape: false,
        }
    }
}

/// Switches the body between the portrait and landscape layouts when the
/// launcher's orientation flips.
fn fit_orientation(ctx: &mut Context<'_, QuickSettings>, orientation: Orientation) {
    let landscape = orientation.is_landscape();
    if ctx.me().landscape == landscape {
        return;
    }
    let (body, sliders, bottom_bar) = {
        let me = ctx.me();
        me.landscape = landscape;
        (me.body, me.sliders, me.bottom_bar)
    };
    if let Some(mut c) = ctx.at(body) {
        c.set_style(body_style(landscape));
    }
    if let Some(mut c) = ctx.at(sliders) {
        c.set_style(Sliders::style(landscape));
    }
    if let Some(mut c) = ctx.at(bottom_bar) {
        c.set_style(BottomBar::style(landscape));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_wayland::mechanix_widgets::prelude::FontBook;

    fn rect(app: &App, id: impl Into<NodeId>) -> Rect {
        app.component::<ComputedLayout>(id).unwrap().rect
    }

    #[test]
    fn sliders_sit_beside_the_tiles_only_while_the_window_is_wider_than_tall() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(screen::OrientationModule)
            .add_module(MechanixTheme::dark())
            .add_module(RingModule::default())
            .add_module(TimerModule);
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        let size = |w, h| LayoutStyle::default().column().size(px(w), px(h));
        let root = app.root();
        let window = app.spawn(root, window().layout(size(800.0, 480.0)));
        // What shares the window with the panel in the control center (the
        // screen selector, the drag handle); empty until the portrait case.
        let above = app.spawn(window, div().style(LayoutStyle::default().shrink(0.0)));
        let slot = app.spawn(
            window,
            div().style(
                LayoutStyle::default()
                    .width(percent(100.0))
                    .grow(1.0)
                    .min_height(px(0.0)),
            ),
        );
        let panel = app.spawn(
            slot,
            QuickSettingsBuilder {
                header: HeaderBuilder,
                bottom_bar: BottomBarBuilder,
                sliders: SlidersBuilder,
            },
        );
        let settle = |app: &mut App| {
            for _ in 0..3 {
                app.tick();
            }
        };
        settle(&mut app);

        let parts = |app: &App| {
            let me = app.widget::<QuickSettings>(panel).unwrap();
            (
                rect(app, me.sliders),
                rect(app, me.bottom_bar),
                me.landscape,
            )
        };
        let (sliders, tiles, landscape) = parts(&app);
        assert!(landscape);
        assert!(
            sliders.origin.x + sliders.width() <= tiles.origin.x,
            "{sliders:?} {tiles:?}"
        );
        assert!(tiles.origin.y + tiles.height() <= 480.0, "{tiles:?}");

        // A portrait window whose panel, under 400 px of other content,
        // is itself wider than tall: the window's shape wins.
        *app.component_mut::<LayoutStyle>(window).unwrap() = size(480.0, 800.0);
        *app.component_mut::<LayoutStyle>(above).unwrap() =
            LayoutStyle::default().height(px(400.0)).shrink(0.0);
        settle(&mut app);
        let panel_rect = rect(&app, panel);
        assert!(panel_rect.width() > panel_rect.height(), "{panel_rect:?}");
        let (sliders, tiles, landscape) = parts(&app);
        assert!(!landscape);
        assert!(
            sliders.origin.y + sliders.height() <= tiles.origin.y,
            "{sliders:?} {tiles:?}"
        );
        assert_eq!(tiles.width(), 480.0);
    }
}
