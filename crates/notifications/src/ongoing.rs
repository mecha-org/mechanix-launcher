//! Ongoing state: what the shell is doing or holding right now, not
//! something that happened. Each is one compact row above the
//! notifications, with no time; a tap shows its detail lines. Rows are
//! the shell's own: the user can't dismiss or clear them, and nothing
//! reaches them over `org.freedesktop.Notifications`.
//!
//! [`set`] shows or replaces the row with its key, [`clear`] removes it.
//! The screen kept on by apps is one: one row however many apps keep it
//! on, one detail line each.

use std::collections::HashSet;

use mecha_wayland::mechanix_widgets::{self as mw, TextContextExt};
use mecha_wayland::prelude::*;
use services::KeepAwake;

use crate::style::{space, text};

/// One row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    key: String,
    /// The row, one line.
    label: String,
    /// Shown on a tap: (who, what), one line each.
    lines: Vec<(String, String)>,
}

/// The rows, in the order they first showed, and which are open.
#[derive(Default)]
struct Ongoing {
    rows: Vec<Row>,
    open: HashSet<String>,
}
impl Resource for Ongoing {}

const KEEP_AWAKE: &str = "keep-awake";
/// A row's height, closed. Figma min-h 48, less the card's padding.
const ROW_HEIGHT: f32 = 40.0;
const DOT: f32 = 8.0;

pub(crate) fn install(app: &mut App) {
    app.init_resource::<Ongoing>().system(on_keep_awake);
    rebuild(app);
}

/// Show `row`, or replace the row with its key.
fn set(app: &mut App, row: Row) {
    let mut ongoing = app.resource_mut::<Ongoing>();
    match ongoing.rows.iter_mut().find(|r| r.key == row.key) {
        Some(r) if *r == row => return,
        Some(r) => *r = row,
        None => ongoing.rows.push(row),
    }
    rebuild(app);
}

/// Remove the row `key`, if shown.
fn clear(app: &mut App, key: &str) {
    let mut ongoing = app.resource_mut::<Ongoing>();
    let before = ongoing.rows.len();
    ongoing.rows.retain(|r| r.key != key);
    ongoing.open.remove(key);
    if ongoing.rows.len() != before {
        rebuild(app);
    }
}

/// Each of `items` once, in the order they first come.
fn unique<'a>(items: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|i| seen.insert(*i)).collect()
}

/// The screen kept on by apps: who, and what for.
fn on_keep_awake(app: &mut App, _: &OnChanged<KeepAwake>) {
    let lines: Vec<(String, String)> = app
        .resource::<KeepAwake>()
        .apps
        .iter()
        .map(|hold| {
            let name = if hold.app.is_empty() {
                "An app".to_owned()
            } else {
                hold.app.clone()
            };
            (
                name,
                unique(hold.reasons.iter().map(String::as_str)).join(", "),
            )
        })
        .collect();
    if lines.is_empty() {
        clear(app, KEEP_AWAKE);
        return;
    }
    let names = unique(lines.iter().map(|(name, _)| name.as_str())).join(", ");
    set(
        app,
        Row {
            key: KEEP_AWAKE.into(),
            label: format!("Screen kept on · {names}"),
            lines,
        },
    );
}

/// Make the rows again from [`Ongoing`].
fn rebuild(app: &mut App) {
    let Some(list) = app.widgets::<OngoingList>().next().map(|(id, _)| id) else {
        return;
    };
    for row in app
        .children(list)
        .map(<[NodeId]>::to_vec)
        .unwrap_or_default()
    {
        app.remove(row);
    }
    let rows: Vec<(Row, bool)> = {
        let ongoing = app.resource::<Ongoing>();
        ongoing
            .rows
            .iter()
            .map(|r| (r.clone(), ongoing.open.contains(&r.key)))
            .collect()
    };
    if let Some(mut style) = app.component_mut::<LayoutStyle>(list) {
        style.display = display(!rows.is_empty());
    }
    for (row, open) in rows {
        app.spawn(list, OngoingRowBuilder { row, open });
    }
}

/// The rows, above the notifications. Hidden while there are none.
pub(crate) struct OngoingList;

pub(crate) struct OngoingListBuilder;

impl Build for OngoingListBuilder {
    type Widget = OngoingList;
}

impl Widget for OngoingList {
    type Builder = OngoingListBuilder;

    /// Starts hidden: the panel can be built before [`install`], which
    /// fills it.
    fn build(_: OngoingListBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .shrink(0.0)
            .padding(Insets::new(
                px(0.0),
                px(space::LIST_X.dp()),
                px(0.0),
                px(space::LIST_X.dp()),
            ))
            .gap(px(space::TITLE_TO_BODY.dp()))
            .hidden();
        OngoingList
    }
}

struct OngoingRow {
    key: String,
    details: Handle<Div>,
    toggle: Handle<mw::Text>,
}

struct OngoingRowBuilder {
    row: Row,
    /// Open as built: rebuilt rows stay as they were.
    open: bool,
}

impl Build for OngoingRowBuilder {
    type Widget = OngoingRow;
}

fn display(shown: bool) -> Display {
    if shown {
        Display::Flex
    } else {
        Display::Hidden
    }
}

impl Widget for OngoingRow {
    type Builder = OngoingRowBuilder;

    fn build(b: OngoingRowBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .padding(Insets::new(
                px(0.0),
                px(space::CARD_PAD.dp()),
                px(0.0),
                px(space::CARD_PAD.dp()),
            ));
        *s.component_mut::<Paint>(me).unwrap() =
            Paint::Quad(Quad::new(s.color(ColorRole::SurfaceContainerLow)));

        let head = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .width(percent(100.0))
                    .height(px(ROW_HEIGHT))
                    .gap(px(space::ICON_TO_TEXT.dp())),
            ),
        );
        // Lit: something is going on.
        let dot = s.spawn(
            head,
            div().style(LayoutStyle::default().size(px(DOT), px(DOT)).shrink(0.0)),
        );
        let lit = s.color(ColorRole::Primary);
        *s.component_mut::<Paint>(dot).unwrap() = Paint::Quad(Quad::new(lit).radius(DOT / 2.0));
        s.spawn(
            head,
            mw::text(b.row.label)
                .variant(text::CARD_BODY.variant)
                .color(ColorRole::OnSurface)
                .max_lines(1)
                .overflow(TextOverflow::Ellipsis)
                .style(
                    LayoutStyle::default()
                        .grow(1.0)
                        .shrink(1.0)
                        .min_width(px(0.0)),
                ),
        );
        let has_lines = !b.row.lines.is_empty();
        let toggle = s.spawn(
            head,
            mw::text(if b.open { "HIDE" } else { "SHOW" })
                .variant(text::SWIPE_BUTTON.variant)
                .color(text::SWIPE_BUTTON.color)
                .style(
                    LayoutStyle::default()
                        .shrink(0.0)
                        .display(display(has_lines)),
                ),
        );

        let details = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .column()
                    .width(percent(100.0))
                    .gap(px(space::MENU_Y.dp()))
                    .padding(Insets::new(
                        px(0.0),
                        px(0.0),
                        px(space::CARD_PAD.dp()),
                        px(DOT + space::ICON_TO_TEXT.dp()),
                    ))
                    .display(display(b.open && has_lines)),
            ),
        );
        for (who, what) in b.row.lines {
            let line = s.spawn(
                details,
                div().style(
                    LayoutStyle::default()
                        .row()
                        .width(percent(100.0))
                        .gap(px(space::ICON_TO_TEXT.dp())),
                ),
            );
            s.spawn(
                line,
                mw::text(who)
                    .variant(text::CARD_TIME.variant)
                    .color(ColorRole::OnSurface)
                    .max_lines(1)
                    .style(LayoutStyle::default().shrink(0.0)),
            );
            s.spawn(
                line,
                mw::text(what)
                    .variant(text::CARD_TIME.variant)
                    .color(text::CARD_TIME.color)
                    .max_lines(1)
                    .overflow(TextOverflow::Ellipsis)
                    .style(LayoutStyle::default().shrink(1.0).min_width(px(0.0))),
            );
        }

        s.on::<Clicked>(head, |ctx, _| {
            let (key, details, toggle) = (ctx.me().key.clone(), ctx.me().details, ctx.me().toggle);
            let open = {
                let mut ongoing = ctx.resource_mut::<Ongoing>();
                !ongoing.open.remove(&key) && ongoing.open.insert(key)
            };
            if let Some(mut n) = ctx.at(details) {
                n.set_display(display(open));
            }
            if let Some(mut n) = ctx.at(toggle) {
                n.set_text(if open { "HIDE" } else { "SHOW" });
            }
        });

        OngoingRow {
            key: b.row.key,
            details,
            toggle,
        }
    }
}

#[cfg(test)]
mod tests {
    use services::AppHold;

    use super::*;

    fn hold(owner: &str, app: &str, reasons: &[&str]) -> AppHold {
        AppHold {
            owner: owner.into(),
            app: app.into(),
            reasons: reasons.iter().map(|r| (*r).into()).collect(),
        }
    }

    fn rows(app: &App) -> Vec<Row> {
        app.resource::<Ongoing>().rows.clone()
    }

    #[test]
    fn the_screen_kept_on_is_one_row_however_many_apps_hold_it() {
        let mut app = App::new();
        app.init_resource::<KeepAwake>();
        install(&mut app);

        app.resource_mut::<KeepAwake>().apps = vec![
            hold(":1.7", "ES-DE", &["Playing a game"]),
            hold(":1.8", "PCSX2", &["VM running", "Paused", "VM running"]),
            hold(":1.9", "", &["Video"]),
            hold(":1.10", "ES-DE", &["Menu"]),
        ];
        app.tick();
        assert_eq!(
            rows(&app),
            [Row {
                key: KEEP_AWAKE.into(),
                label: "Screen kept on · ES-DE, PCSX2, An app".into(),
                lines: vec![
                    ("ES-DE".into(), "Playing a game".into()),
                    ("PCSX2".into(), "VM running, Paused".into()),
                    ("An app".into(), "Video".into()),
                    ("ES-DE".into(), "Menu".into()),
                ],
            }]
        );

        app.resource_mut::<KeepAwake>().apps.clear();
        app.tick();
        assert!(rows(&app).is_empty());
    }
}
