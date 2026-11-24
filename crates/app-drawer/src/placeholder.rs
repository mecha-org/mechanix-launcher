//! Apps starting, and their placeholder: from the tap that starts an app
//! until its window is up, a full-screen view of the app's icon and name
//! over the theme's surface. It holds the keyboard focus meanwhile, so the
//! app under it doesn't get the focus back, and with it the keyboard, on
//! the way.
//!
//! Each start is listed in [`Launches`], where the running apps show it
//! too. A launch ends, by [`end`] alone, when:
//! - the window the app opened with our activation token is listed; else
//!   the registry first lists the app running, for an app that ignores the
//!   token;
//! - the app failed: its start failed, or it ended before a window came
//!   ([`failed`], from the start's own bookkeeping);
//! - [`TIMEOUT`] passed, for an app that does neither;
//! - it is stopped ([`StopLaunch`]), which calls the app's start off.
//!
//! Its placeholder goes with it, or at once by [`hide_launch_placeholder`],
//! and comes back by [`ShowLaunch`].

use std::collections::HashMap;
use std::time::Duration;

use mecha_wayland::mechanix_widgets::{self as mw, TextWrap};
use mecha_wayland::prelude::*;

use apps::{
    DesktopApp, LaunchId, Launches, LaunchesChanged, Registry, RegistryChanged, RowIcon,
    ShowLaunch, StopLaunch,
};

/// How long it waits for the window of an app that neither shows one nor
/// fails. Long enough for a slow cold start: the placeholder doesn't keep
/// the user from going elsewhere meanwhile.
const TIMEOUT: Duration = Duration::from_secs(30);
const ICON: f32 = 96.0;
/// Between the icon and the name.
const GAP: f32 = 24.0;
const NAME_PX: f32 = 32.0;
const NAMESPACE: &str = "launch-placeholder";

#[derive(Default)]
struct Placeholder {
    /// Our activation token each launch's app was started with, once it has
    /// one.
    tokens: HashMap<LaunchId, String>,
    /// The launch whose placeholder is up, and its window.
    shown: Option<(LaunchId, NodeId)>,
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
        .system(|app: &mut App, _: &ToplevelsChanged| end_arrived(app))
        .system(|app: &mut App, _: &RegistryChanged| end_arrived(app))
        .system(|app: &mut App, e: &ShowLaunch| show(app, e.0))
        .system(|app: &mut App, e: &StopLaunch| end(app, e.0, Ending::CalledOff))
        .system(on_timeout);
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

/// Show launch `id`'s placeholder, in place of any other.
fn show(app: &mut App, id: LaunchId) {
    hide_launch_placeholder(app);
    let Some((name, icon)) = app
        .resource::<Launches>()
        .get(id)
        .map(|l| (l.app.name.clone(), l.icon))
    else {
        return;
    };
    let root = app.root();
    let role = Role::Layer(LayerRole {
        layer: Layer::Overlay,
        anchor: Anchor::all(),
        exclusive_zone: 0,
        namespace: NAMESPACE.into(),
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        shell_role: Some(ShellRole::View),
    });
    let window = app
        .spawn_with(
            root,
            window()
                .title(NAMESPACE)
                .layout(LayoutStyle::default())
                .clear(Color::TRANSPARENT),
            (role,),
        )
        .id();
    app.spawn(window, PlaceholderViewBuilder { name, icon });
    app.resource_mut::<Placeholder>().shown = Some((id, window));
}

/// Take the launch placeholder down now, if one is up: e.g. when the user
/// goes elsewhere before the app's window comes. Its launch goes on.
pub fn hide_launch_placeholder(app: &mut App) {
    if !app.has_resource::<Placeholder>() {
        return;
    }
    let shown = app.resource_mut::<Placeholder>().shown.take();
    if let Some((_, window)) = shown {
        app.remove(window);
    }
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
        placeholder.shown.is_some_and(|(s, _)| s == id)
    };
    match ending {
        Ending::CalledOff => crate::launch::stop(app, id),
        Ending::Over => crate::launch::forget(app, id),
    }
    if shown {
        hide_launch_placeholder(app);
    }
    app.signal(LaunchesChanged);
}

/// Every launch whose app's window is up is over.
fn end_arrived(app: &mut App) {
    let arrived: Vec<LaunchId> = app
        .resource::<Launches>()
        .iter()
        .filter(|l| opened_with(app, l.id) || running(app, &l.app.id))
        .map(|l| l.id)
        .collect();
    for id in arrived {
        end(app, id, Ending::Over);
    }
}

/// Whether a window that came with `launch`'s activation token is listed.
fn opened_with(app: &App, launch: LaunchId) -> bool {
    let placeholder = app.resource::<Placeholder>();
    let Some(token) = placeholder.tokens.get(&launch) else {
        return false;
    };
    app.has_resource::<Toplevels>()
        && app
            .resource::<Toplevels>()
            .iter()
            .any(|w| w.activation_token.as_ref() == Some(token))
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

pub(crate) struct PlaceholderViewBuilder {
    name: String,
    icon: RowIcon,
}

impl Build for PlaceholderViewBuilder {
    type Widget = PlaceholderView;
}

/// The app's icon over its name, centred on the theme's surface.
pub(crate) struct PlaceholderView;

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
