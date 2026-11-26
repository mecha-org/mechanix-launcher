//! Apps starting, and their placeholder: from the tap that starts an app
//! until its window is up, a full-screen view of the app's icon and name
//! over the theme's surface, the [`LaunchPlaceholder`] screen. The host
//! puts it in a container of its own, which opens on [`LaunchPlaceholderUp`]
//! and closes on [`LaunchPlaceholderDown`], or as the user leaves it like
//! any screen.
//!
//! Each start is listed in [`Launches`], where the running apps show it
//! too. A launch ends, by [`end`] alone, when:
//! - the window the app opened with our activation token is listed; else
//!   a window of a process in the app's own unit, whatever its app id says;
//!   else the registry first lists the app running, for an app that ignores
//!   the token and hands the start to a process elsewhere;
//! - the app failed: its start failed, or it ended before a window came
//!   ([`failed`], from the start's own bookkeeping);
//! - [`TIMEOUT`] passed, for an app that does neither;
//! - it is stopped ([`StopLaunch`]), which calls the app's start off.
//!
//! Its placeholder goes with it, and comes back by [`ShowLaunch`]. It also
//! goes once any window is activated under it, launch ended or not: it
//! never covers an app.

use std::collections::HashMap;
use std::time::Duration;

use mecha_wayland::mechanix_widgets::{self as mw, TextWrap};
use mecha_wayland::prelude::*;
use screen::{Hidden, LaunchPlaceholderDown, LaunchPlaceholderUp, Screen};

use apps::{
    DesktopApp, LaunchId, Launches, LaunchesChanged, Registry, RegistryChanged, RowIcon,
    ShowLaunch, StopLaunch,
};

/// How long it waits for the window of an app that neither shows one nor
/// fails. Long enough for a slow cold start: the placeholder doesn't keep
/// the user from going elsewhere meanwhile.
const TIMEOUT: Duration = Duration::from_secs(15);
const ICON: f32 = 96.0;
/// Between the icon and the name.
const GAP: f32 = 24.0;
const NAME_PX: f32 = 32.0;

#[derive(Default)]
struct Placeholder {
    /// Our activation token each launch's app was started with, once it has
    /// one.
    tokens: HashMap<LaunchId, String>,
    /// The [`LaunchPlaceholder`] screen's root, once built, and the view in
    /// it with the launch it shows.
    screen: Option<NodeId>,
    view: Option<(LaunchId, NodeId)>,
    /// The launch whose placeholder is up.
    shown: Option<LaunchId>,
    /// The latest window activation when the placeholder went up: a later
    /// one is of a window it would cover.
    shown_after: u64,
    /// Every ring timeout not finished yet and its launch, including those
    /// of launches already done.
    timeouts: Vec<(Token, LaunchId)>,
}
impl Resource for Placeholder {}

/// Registers the systems for apps starting. Safe to call more than once.
pub(crate) fn install(app: &mut App) {
    if app.has_resource::<Placeholder>() {
        return;
    }
    if !app.has_resource::<Launches>() {
        app.init_resource::<Launches>();
    }
    app.init_resource::<Placeholder>()
        .system(|app: &mut App, _: &ToplevelsChanged| {
            uncover(app);
            end_arrived(app);
        })
        .system(|app: &mut App, _: &RegistryChanged| end_arrived(app))
        .system(|app: &mut App, e: &ShowLaunch| show(app, e.0))
        .system(|app: &mut App, e: &StopLaunch| end(app, e.0, Ending::CalledOff))
        .system(on_hidden)
        .system(on_timeout);
}

/// The screen of the launch placeholder: the starting app's icon and name.
/// Its container opens on [`LaunchPlaceholderUp`] and closes on
/// [`LaunchPlaceholderDown`]; it takes the keyboard focus while shown, so an app
/// under it doesn't get it back, and with it the keyboard, on the way.
pub struct LaunchPlaceholder;

impl Screen for LaunchPlaceholder {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        install(app);
        let root = app
            .spawn(parent, div().style(LayoutStyle::default().fill()))
            .id();
        app.resource_mut::<Placeholder>().screen = Some(root);
        root
    }
}

/// Its container closed: the user went elsewhere, or it was taken down.
fn on_hidden(app: &mut App, e: &Emitted<Hidden>) {
    let mut placeholder = app.resource_mut::<Placeholder>();
    if placeholder.screen.is_some_and(|s| e.targets.contains(&s)) {
        placeholder.shown = None;
    }
}

/// How a launch ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// Its window came, the app failed, or it was given up on: the app's
    /// start is only forgotten.
    Over,
    /// Called off: the app's start is stopped.
    CalledOff,
}

/// `entry` is starting: its launch is listed and its placeholder shown, in
/// place of any other.
pub(crate) fn start(app: &mut App, entry: &DesktopApp, icon: RowIcon) -> LaunchId {
    let id = app.resource_mut::<Launches>().add(entry.clone(), icon);
    if app.has_resource::<Ring>() {
        let timeout = app.resource_mut::<Ring>().timeout(TIMEOUT);
        app.resource_mut::<Placeholder>()
            .timeouts
            .push((timeout, id));
    }
    app.signal(LaunchesChanged);
    show(app, id);
    id
}

/// `launch`'s app is starting with our activation `token`: its window is
/// the one that comes with it.
pub(crate) fn started_with(app: &mut App, launch: LaunchId, token: &str) {
    if !app.has_resource::<Placeholder>() {
        return;
    }
    app.resource_mut::<Placeholder>()
        .tokens
        .insert(launch, token.to_owned());
    end_arrived(app);
}

/// Show launch `id`'s placeholder, in place of any other's. Its view is
/// built once per launch: shown again, there is nothing to redraw.
fn show(app: &mut App, id: LaunchId) {
    let Some((name, icon)) = app
        .resource::<Launches>()
        .get(id)
        .map(|l| (l.app.name.clone(), l.icon))
    else {
        return;
    };
    let (screen, view) = {
        let placeholder = app.resource::<Placeholder>();
        (placeholder.screen, placeholder.view)
    };
    if view.map(|(of, _)| of) != Some(id) {
        if let Some((_, old)) = view {
            app.remove(old);
        }
        let view = screen.map(|s| (id, app.spawn(s, PlaceholderViewBuilder { name, icon }).id()));
        app.resource_mut::<Placeholder>().view = view;
    }
    let after = latest_activation(app);
    let mut placeholder = app.resource_mut::<Placeholder>();
    placeholder.shown = Some(id);
    placeholder.shown_after = after;
    app.signal(LaunchPlaceholderUp);
}

/// Take the placeholder down, if up. Its launch goes on.
pub(crate) fn take_down(app: &mut App) {
    if app.resource_mut::<Placeholder>().shown.take().is_some() {
        app.signal(LaunchPlaceholderDown);
    }
}

/// The launch whose placeholder is up.
#[cfg(test)]
pub(crate) fn shown(app: &App) -> Option<LaunchId> {
    app.resource::<Placeholder>().shown
}

/// A window activated since the placeholder went up takes it down, whatever
/// launch it ends, if any: the placeholder never covers an app.
fn uncover(app: &mut App) {
    let shown_after = app.resource::<Placeholder>().shown_after;
    if latest_activation(app) > shown_after {
        take_down(app);
    }
}

/// The latest window activation; 0 for none.
fn latest_activation(app: &App) -> u64 {
    windows(app).map(|w| w.last_activated).max().unwrap_or(0)
}

/// Launch `id`'s app failed: its start failed, or it ended before a window
/// came. Its launch is over. A no-op for a launch already over.
pub(crate) fn failed(app: &mut App, id: LaunchId) {
    if app.has_resource::<Placeholder>() {
        end(app, id, Ending::Over);
    }
}

/// Launch `id` ends: unlisted, its placeholder down, and the app's own
/// start forgotten or called off. The one way a launch ends; a no-op for
/// one already over.
fn end(app: &mut App, id: LaunchId, ending: Ending) {
    if !app.resource_mut::<Launches>().remove(id) {
        return;
    }
    let shown = {
        let mut placeholder = app.resource_mut::<Placeholder>();
        placeholder.tokens.remove(&id);
        placeholder.shown == Some(id)
    };
    match ending {
        Ending::CalledOff => crate::launch::stop(app, id),
        Ending::Over => crate::launch::forget(app, id),
    }
    if shown {
        take_down(app);
    }
    app.signal(LaunchesChanged);
}

/// Every launch whose app's window is up is over.
pub(crate) fn end_arrived(app: &mut App) {
    if !app.has_resource::<Placeholder>() {
        return;
    }
    let arrived: Vec<LaunchId> = app
        .resource::<Launches>()
        .iter()
        .filter(|l| opened_with(app, l.id) || opened_in(app, l.id) || running(app, &l.app.id))
        .map(|l| l.id)
        .collect();
    for id in arrived {
        end(app, id, Ending::Over);
    }
}

/// The windows listed; none without a compositor.
fn windows(app: &App) -> impl Iterator<Item = &ToplevelInfo> {
    app.has_resource::<Toplevels>()
        .then(|| app.resource::<Toplevels>().iter())
        .into_iter()
        .flatten()
}

/// Whether a window that came with `launch`'s activation token is listed.
fn opened_with(app: &App, launch: LaunchId) -> bool {
    let Some(token) = app.resource::<Placeholder>().tokens.get(&launch) else {
        return false;
    };
    windows(app).any(|w| w.activation_token.as_ref() == Some(token))
}

/// Whether a window of a process in `launch`'s own unit is listed: the
/// app's, or one it started, as everything it starts stays in its unit.
fn opened_in(app: &App, launch: LaunchId) -> bool {
    let Some(unit) = crate::launch::unit(app, launch) else {
        return false;
    };
    windows(app).any(|w| w.pid.and_then(apps::unit_of).as_deref() == Some(unit))
}

/// Whether the registry lists the app of desktop id `id` running.
fn running(app: &App, id: &str) -> bool {
    app.has_resource::<Registry>() && app.resource::<Registry>().running_id(id).is_some()
}

/// A launch whose app never opened a window is done.
fn on_timeout(app: &mut App, e: &IoEvent) {
    let launch = {
        let (mut placeholder, mut ring) = app.query::<(ResMut<Placeholder>, ResMut<Ring>)>();
        let Some(at) = placeholder.timeouts.iter().position(|(t, _)| *t == e.token) else {
            return;
        };
        let _ = ring.finish(e.token);
        placeholder.timeouts.swap_remove(at).1
    };
    let Some(entry) = app
        .resource::<Launches>()
        .get(launch)
        .map(|l| l.app.id.clone())
    else {
        return;
    };
    tracing::warn!(%entry, "no window came for the app, dropping its launch");
    end(app, launch, Ending::Over);
}

struct PlaceholderViewBuilder {
    name: String,
    icon: RowIcon,
}

impl Build for PlaceholderViewBuilder {
    type Widget = PlaceholderView;
}

/// The app's icon over its name, centred on the theme's surface.
struct PlaceholderView;

impl Widget for PlaceholderView {
    type Builder = PlaceholderViewBuilder;

    fn build(b: PlaceholderViewBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .fill()
            .align_items(Align::Center)
            .justify(Justify::Center)
            .gap(px(GAP));
        let surface = s.color(ColorRole::Surface);
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(surface));
        b.icon.spawn(s, me, ICON);
        s.spawn(
            me,
            mw::text(b.name)
                .size(NAME_PX)
                .weight(FontWeight::Light)
                .color(ColorRole::OnSurface)
                .wrap(TextWrap::NoWrap),
        );
        PlaceholderView
    }
}
