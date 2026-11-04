use mecha_wayland::prelude::*;

use crate::bottom_bar::BottomBarBuilder;
use crate::drag_handle::DragHandleBuilder;
use crate::header::HeaderBuilder;
use crate::sliders::SlidersBuilder;

const SECTION_GAP: f32 = 31.0;

pub struct QuickSettings;

pub struct QuickSettingsBuilder {
    pub header: HeaderBuilder,
    pub bottom_bar: BottomBarBuilder,
    pub sliders: SlidersBuilder,
}

impl Build for QuickSettingsBuilder {
    type Widget = QuickSettings;
}

impl Widget for QuickSettings {
    type Builder = QuickSettingsBuilder;

    fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .fill()
            .width(percent(100.0))
            .height(percent(100.0))
            .column()
            .justify(Justify::SpaceBetween)
            .gap(px(SECTION_GAP));

        // Header and footer keep their fixed heights; the sliders grow
        // into the space between them and center themselves there.
        s.spawn(me, b.header);
        s.spawn(me, b.sliders);

        // The drag handle sits flush under the tiles, outside the section gap.
        let footer = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .width(percent(100.0))
                    .column()
                    .shrink(0.0),
            ),
        );
        s.spawn(footer, b.bottom_bar);
        s.spawn(footer, DragHandleBuilder);

        QuickSettings
    }
}
