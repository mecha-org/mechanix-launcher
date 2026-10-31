use mecha_wayland::prelude::*;

use crate::bottom_bar::BottomBarBuilder;
use crate::header::{HEADER_HEIGHT, HeaderBuilder};
use crate::sliders::{SLIDERS_HEIGHT, SlidersBuilder};
use crate::util::full_width;

const TOP_HEIGHT: f32 = HEADER_HEIGHT + SLIDERS_HEIGHT;
const SECTION_GAP: f32 = 31.0;
const MAX_HEIGHT: f32 = 600.0;

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
            .max_height(px(MAX_HEIGHT))
            .column()
            .gap(px(SECTION_GAP))
            .justify(Justify::SpaceBetween);

        let top = s.spawn(me, div().style(full_width(TOP_HEIGHT).column()));
        s.spawn(top, b.header);
        s.spawn(top, b.sliders);
        s.spawn(me, b.bottom_bar);

        QuickSettings
    }
}
