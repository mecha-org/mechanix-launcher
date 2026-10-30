//! One app in the drawer's list: its name on the left, its icon on the right.

use mecha_wayland::mechanix_widgets::{self as mw, TextWrap};
use mecha_wayland::prelude::*;

use apps::RowIcon;

const HEIGHT: f32 = 56.0;
const ICON: f32 = 48.0;
pub(crate) const SIDE: f32 = 24.0;
/// Between the name and the icon.
pub(crate) const GAP: f32 = 16.0;
pub(crate) const NAME_PX: f32 = 32.0;

/// Emitted at the drawer when a row is tapped: the app's index in its list.
pub(crate) struct Tapped(pub usize);
impl Event for Tapped {}

pub(crate) struct RowBuilder {
    /// Already fitted to the row's width.
    pub name: String,
    /// Char indices into `name` that matched the query.
    pub positions: Vec<u32>,
    pub icon: RowIcon,
    pub index: usize,
    pub drawer: NodeId,
}

impl Build for RowBuilder {
    type Widget = Row;
}

pub(crate) struct Row {
    #[cfg_attr(not(test), allow(dead_code))]
    name: String,
    runs: Vec<(String, bool)>,
}

impl Row {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The name split into runs, each flagged whether it matched.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn runs(&self) -> &[(String, bool)] {
        &self.runs
    }
}

impl Widget for Row {
    type Builder = RowBuilder;

    fn build(b: RowBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .row()
            .align_items(Align::Center)
            .justify(Justify::SpaceBetween)
            .gap(px(GAP))
            .size(percent(100.0), px(HEIGHT))
            .padding(Insets::new(px(0.0), px(SIDE), px(0.0), px(SIDE)));

        let label = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .min_width(px(0.0)),
            ),
        );
        let runs = runs(&b.name, &b.positions);
        for (words, matched) in &runs {
            let (weight, role) = if *matched {
                (FontWeight::Regular, ColorRole::Primary)
            } else {
                (FontWeight::Light, ColorRole::OnSurface)
            };
            s.spawn(
                label,
                mw::text(words.clone())
                    .size(NAME_PX)
                    .weight(weight)
                    .color(role)
                    .wrap(TextWrap::NoWrap),
            );
        }

        let size = LayoutStyle::default().size(px(ICON), px(ICON)).shrink(0.0);
        match b.icon {
            RowIcon::Image(sprite) => {
                s.spawn(me, image(sprite).style(size));
            }
            RowIcon::Glyph(sprite) => {
                let color = s.color(ColorRole::OnSurfaceVariant);
                s.spawn(
                    me,
                    icon(sprite)
                        .size(Size::new(ICON, ICON))
                        .color(color)
                        .style(size),
                );
            }
        }

        let (drawer, index) = (b.drawer, b.index);
        s.on::<Clicked>(me, move |ctx, _| ctx.emit(Tapped(index), drawer));
        Row { name: b.name, runs }
    }
}

/// `name` cut into maximal runs of matched and unmatched chars.
fn runs(name: &str, positions: &[u32]) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for (i, c) in name.chars().enumerate() {
        let matched = positions.contains(&(i as u32));
        match out.last_mut() {
            Some((run, m)) if *m == matched => run.push(c),
            _ => out.push((c.to_string(), matched)),
        }
    }
    out
}
