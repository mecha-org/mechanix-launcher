use mecha_wayland::prelude::*;

use crate::bottom_bar::{BottomBar, BottomBarBuilder};
use crate::drag_handle::DragHandleBuilder;
use crate::header::HeaderBuilder;
use crate::sliders::{Sliders, SlidersBuilder};

const SECTION_GAP: f32 = 31.0;

/// The panel: header on top, drag handle at the bottom, and between them a
/// body holding the sliders and the tiles.
///
/// The body follows the panel's shape: in portrait the sliders sit above
/// the tiles; when the panel is wider than it is tall they sit side by
/// side, so the sections no longer have to share the short height.
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
        .padding(Insets::new(px(SECTION_GAP), px(0.0), px(0.0), px(0.0)))
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

        // Header and drag handle keep their fixed heights; the body takes
        // the rest. The drag handle sits flush under it, outside the gap.
        s.spawn(me, b.header);
        let body = s.spawn(me, div().style(body_style(false)));
        let sliders = s.spawn(body, b.sliders);
        let bottom_bar = s.spawn(body, b.bottom_bar);
        s.spawn(me, DragHandleBuilder);

        s.on::<OnChanged<ComputedLayout>>(me, |ctx, _| fit_orientation(ctx));

        QuickSettings {
            body,
            sliders,
            bottom_bar,
            landscape: false,
        }
    }
}

/// Switches the body between the portrait and landscape layouts when the
/// panel's shape changes.
fn fit_orientation(ctx: &mut Context<'_, QuickSettings>) {
    let Some(rect) = ctx.component::<ComputedLayout>().map(|l| l.rect) else {
        return;
    };
    let landscape = rect.width() > rect.height();
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
    fn sliders_sit_beside_the_tiles_only_while_the_panel_is_wider_than_tall() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .add_module(RingModule::default())
            .add_module(TimerModule);
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        let size = |w, h| LayoutStyle::default().size(px(w), px(h));
        let window = app.spawn_with(
            app.root(),
            div().style(size(800.0, 480.0)),
            (LayoutRoot(true),),
        );
        let panel = app.spawn(
            window,
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

        *app.component_mut::<LayoutStyle>(window).unwrap() = size(480.0, 800.0);
        settle(&mut app);
        let (sliders, tiles, landscape) = parts(&app);
        assert!(!landscape);
        assert!(
            sliders.origin.y + sliders.height() <= tiles.origin.y,
            "{sliders:?} {tiles:?}"
        );
        assert_eq!(tiles.width(), 480.0);
    }
}
