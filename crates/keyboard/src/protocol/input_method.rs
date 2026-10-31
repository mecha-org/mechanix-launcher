//! `zwp_input_method_v2` — the input-method-v2 protocol client.
//!
//! [`InputMethodState`] is a `Resource`; the module binds
//! `ZwpInputMethodManagerV2` conditionally from `Globals`, creates the
//! `ZwpInputMethodV2` object, and handles `ZwpInputMethodV2Event`.
//!
//! Inbound state is double-buffered: `activate`/`deactivate`/`content_type`
//! events fill a pending context applied on `done`. Surrounding text and change
//! cause are not tracked yet. Outbound text commits go through [`commit_text`].

use mecha_wayland::prelude::*;

use crate::visibility::KeyboardVisibilityExt;
use crate::zone::ExclusiveZone;

/// The text-input context the compositor reports for the focused field.
///
/// Inbound state is double-buffered: events fill `pending`, `Done` copies it
/// into `current`. Fields mirror the protocol's "initial values".
#[derive(Debug, Clone)]
pub struct InputMethodContext {
    /// `true` once `activate` has applied (on a `done`); `false` after
    /// `deactivate` applies. Drives whether taps route through IM2.
    pub active: bool,
    /// The focused field's content purpose; picks the view shown on activate.
    pub purpose: ZwpTextInputV3ContentPurpose,
}

impl Default for InputMethodContext {
    fn default() -> Self {
        Self {
            active: false,
            purpose: ZwpTextInputV3ContentPurpose::Normal,
        }
    }
}

/// All state the input-method-v2 client owns. Lives on the app as a `Resource`.
#[derive(Default)]
pub struct InputMethodState {
    /// The per-seat `zwp_input_method_v2` object; `None` until both the manager
    /// and a seat are available.
    pub input_method: Option<ZwpInputMethodV2>,

    /// The applied (post-`done`) inbound context — what the keyboard reads.
    pub current: InputMethodContext,
    /// The pending inbound context — what events are filling this batch.
    pending: InputMethodContext,
    /// Whether this batch carries an `activate`. A text input re-enabled
    /// while already active (a field tapped again) sends one, and that should
    /// bring back a keyboard the user hid.
    pending_activate: bool,

    /// Number of `done` events received. This is the serial echoed back in
    /// `commit(serial)`.
    pub serial: u32,

    /// `true` after `unavailable` — the object is inert.
    pub inert: bool,
}

impl Resource for InputMethodState {}

impl InputMethodState {
    /// Whether text taps should commit through IM2 rather than the
    /// virtual-keyboard-v1 transport. True only when active and not inert.
    pub fn should_commit(&self) -> bool {
        !self.inert && self.input_method.is_some() && self.current.active
    }
}

/// Commit `text` through `zwp_input_method_v2`: a `commit_string` followed by
/// `commit(serial)`. Returns `true` if sent (IM bound and active), `false` if
/// the caller should fall back to the virtual-keyboard transport.
pub(crate) fn commit_text(state: &InputMethodState, wl: &mut Wayland, text: &str) -> bool {
    if !state.should_commit() {
        return false;
    }
    let Some(im) = state.input_method else {
        return false;
    };
    let serial = state.serial;
    im.commit_string(&mut *wl, text);
    im.commit(&mut *wl, serial);
    true
}

/// `Signal`: a text field gained focus, or changed purpose while focused.
/// The keyboard picks the view to show from the purpose.
#[derive(Debug, Clone, Copy)]
pub struct TextInputFocused {
    pub purpose: ZwpTextInputV3ContentPurpose,
}
impl Signal for TextInputFocused {}

/// Module that binds the input-method manager global, creates the IM object,
/// and handles `ZwpInputMethodV2Event`. Install after `WaylandModule`.
pub struct InputMethodModule;

impl Module for InputMethodModule {
    fn install(self, app: &mut App) {
        app.insert_resource(InputMethodState::default());
        init(app);
        app.system(on_input_method_event);
    }
}

/// Try to bind the input-method manager and create the IM object.
fn init(app: &mut App) {
    if app.resource::<InputMethodState>().input_method.is_some()
        || app.resource::<InputMethodState>().inert
    {
        return;
    }

    let seat = *app.resource::<WlSeat>();

    // The manager is an optional global — not every compositor supports
    // input-method-v2. Bind it from `Globals` if advertised.
    let manager = app
        .resource::<Globals>()
        .find(ZwpInputMethodManagerV2::NAME)
        .cloned()
        .map(|g| {
            let (globals, mut wl) = app.query::<(Res<Globals>, ResMut<Wayland>)>();
            globals.bind::<ZwpInputMethodManagerV2>(&g, &mut wl)
        });

    let Some(manager) = manager else {
        tracing::warn!("compositor does not advertise zwp_input_method_manager_v2");
        return;
    };

    let im = {
        let mut wl = app.resource_mut::<Wayland>();
        manager.get_input_method(&mut wl, seat)
    };

    tracing::info!("input-method-v2 bound");
    app.resource_mut::<InputMethodState>().input_method = Some(im);
}

/// Handle the inbound double-buffered state machine: events fill `pending`,
/// `done` applies it to `current` and bumps the serial, `unavailable` marks the
/// object inert. An activate/deactivate also flips the keyboard surface
/// visibility through `KeyboardVisibilityExt`.
fn on_input_method_event(app: &mut App, event: &ZwpInputMethodV2Event) {
    // Both are computed inside the `InputMethodState` borrow and acted on
    // after the guard drops, since signalling needs `&mut App`.
    let mut focused: Option<TextInputFocused> = None;
    let apply_visible: Option<bool> = {
        let st = &mut app.resource_mut::<InputMethodState>();
        if st.inert {
            return;
        }
        match event {
            ZwpInputMethodV2Event::Activate { .. } => {
                // Activate resets all prior inbound state then arms active.
                st.pending = InputMethodContext {
                    active: true,
                    ..Default::default()
                };
                st.pending_activate = true;
                tracing::info!("input-method: activate (pending)");
                None
            }
            ZwpInputMethodV2Event::Deactivate { .. } => {
                st.pending.active = false;
                tracing::info!("input-method: deactivate (pending)");
                None
            }
            ZwpInputMethodV2Event::SurroundingText { .. }
            | ZwpInputMethodV2Event::TextChangeCause { .. } => None,
            ZwpInputMethodV2Event::ContentType { purpose, .. } => {
                st.pending.purpose = *purpose;
                None
            }
            ZwpInputMethodV2Event::Done { .. } => {
                let next = st.pending.clone();
                let activated = std::mem::take(&mut st.pending_activate);
                let prev = std::mem::replace(&mut st.current, next);
                st.serial = st.serial.wrapping_add(1);
                let now = &st.current;
                tracing::info!(
                    active = now.active,
                    serial = st.serial,
                    "input-method: state applied"
                );

                if now.active && (activated || !prev.active || prev.purpose != now.purpose) {
                    focused = Some(TextInputFocused {
                        purpose: now.purpose,
                    });
                }
                // Only an activate/deactivate moves the surface, so other
                // `done`s (e.g. surrounding-text updates) don't undo a manual
                // show or hide. An activate shows it even when already
                // active: the field was tapped again.
                if now.active && activated {
                    Some(true)
                } else {
                    (prev.active != now.active).then_some(now.active)
                }
            }
            ZwpInputMethodV2Event::Unavailable { .. } => {
                st.inert = true;
                tracing::warn!("input-method: unavailable; object now inert");
                None
            }
        }
    };
    // Under the host's own screens, an activation shows the keyboard only
    // for the host's field: an app beneath that gets keyboard focus back
    // re-enables its text input too, and must not bring the keyboard up.
    if apply_visible == Some(true) && !may_show(app) {
        return;
    }
    if let Some(focused) = focused {
        app.signal(focused);
    }
    if let Some(visible) = apply_visible {
        app.set_keyboard_visibility(visible);
    }
}

/// Whether an activation may show the keyboard: always over apps; while
/// [`suspend_keyboard_zone`](crate::suspend_keyboard_zone) holds (the host's
/// screens cover the apps), only when the host's own text field has focus.
fn may_show(app: &App) -> bool {
    let covered = app.has_resource::<ExclusiveZone>() && app.resource::<ExclusiveZone>().suspended;
    !covered || (app.has_resource::<TextInput>() && app.resource::<TextInput>().focused().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visibility::KeyboardVisibility;

    fn app() -> App {
        let mut app = App::new();
        app.insert_resource(KeyboardVisibility::default());
        app.insert_resource(InputMethodState::default());
        app
    }

    fn send(app: &mut App, events: &[fn(ZwpInputMethodV2) -> ZwpInputMethodV2Event]) {
        let im = ZwpInputMethodV2(ObjectId(1));
        for e in events {
            on_input_method_event(app, &e(im));
        }
    }

    fn activate(input_method: ZwpInputMethodV2) -> ZwpInputMethodV2Event {
        ZwpInputMethodV2Event::Activate { input_method }
    }
    fn deactivate(input_method: ZwpInputMethodV2) -> ZwpInputMethodV2Event {
        ZwpInputMethodV2Event::Deactivate { input_method }
    }
    fn done(input_method: ZwpInputMethodV2) -> ZwpInputMethodV2Event {
        ZwpInputMethodV2Event::Done { input_method }
    }

    #[test]
    fn activate_shows_and_deactivate_hides() {
        let mut app = app();
        send(&mut app, &[activate, done]);
        assert!(app.keyboard_visible());
        send(&mut app, &[deactivate, done]);
        assert!(!app.keyboard_visible());
    }

    #[test]
    fn tapping_the_field_again_brings_back_a_hidden_keyboard() {
        let mut app = app();
        send(&mut app, &[activate, done]);
        app.set_keyboard_visibility(false);
        send(&mut app, &[activate, done]);
        assert!(app.keyboard_visible());
    }

    #[test]
    fn an_app_under_the_hosts_screens_cannot_bring_it_up() {
        let mut app = app();
        app.insert_resource(ExclusiveZone {
            suspended: true,
            ..Default::default()
        });
        send(&mut app, &[activate, done]);
        assert!(!app.keyboard_visible());

        app.resource_mut::<ExclusiveZone>().suspended = false;
        send(&mut app, &[activate, done]);
        assert!(app.keyboard_visible());
    }

    #[test]
    fn a_done_without_activate_keeps_a_manual_hide() {
        let mut app = app();
        send(&mut app, &[activate, done]);
        app.set_keyboard_visibility(false);
        send(&mut app, &[done]);
        assert!(!app.keyboard_visible());
    }
}
