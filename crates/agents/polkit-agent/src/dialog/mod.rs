//! The polkit dialog: a bottom sheet asking for the password of the
//! request's user.
//!
//! Shown on [`AuthRequested`], hidden on [`AuthFinished`] or
//! [`AuthCancelled`]. Return or ✓ sends [`AuthAnswer`]; ✕ or dragging the
//! sheet down sends [`AuthCancel`](crate::signals::AuthCancel).
//!
//! The window is a top-layer `prompt` (mx_shell_surface_v1), spawned
//! before the keyboard's, so the on-screen keyboard shows over it.
//!
//! Not yet: closing for the lock screen, a user picker, the requesting
//! program.

use mecha_wayland::prelude::*;
use std::time::Instant;

use crate::signals::{
    AuthAnswer, AuthCancelled, AuthFinished, AuthLocked, AuthMessage, AuthPrompt, AuthRequested,
    AuthRetry, AuthUser,
};

mod view;

use view::{PolkitDialog, PolkitDialogBuilder};

const NAMESPACE: &str = "polkit-agent";

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

const ICON: u32 = 24;
/// The header's icons (✕, ✓), and their tap area.
const HEADER_ICON: f32 = 20.0;

const WRONG: &str = "Incorrect password. Try again.";
#[derive(Clone, Copy)]
struct Icons {
    close: SpriteId,
    confirm: SpriteId,
    show: SpriteId,
    hide: SpriteId,
    warning: SpriteId,
}
impl Resource for Icons {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Prompt {
    text: String,
    echo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    text: String,
    error: bool,
}

/// Holds the cookie: no `Debug`, so it can't be logged.
struct Shown {
    cookie: String,
    message: String,
    user: Option<String>,
    prompt: Option<Prompt>,
    /// The last prompt's text, kept as the placeholder between prompts.
    last_prompt: Option<String>,
    note: Option<Note>,
    /// An answer was sent: it stays in the field, disabled, until the
    /// helper says what it made of it.
    checking: bool,
    /// Locked out until then, after too many wrong answers.
    locked: Option<Instant>,
}

struct Dialog {
    window: NodeId,
    view: Handle<PolkitDialog>,
    shown: Option<Shown>,
}
impl Resource for Dialog {}

impl Dialog {
    fn shown(&mut self, cookie: &str) -> Option<&mut Shown> {
        self.shown.as_mut().filter(|s| s.cookie == cookie)
    }

    fn cookie(&self) -> Option<String> {
        self.shown.as_ref().map(|s| s.cookie.clone())
    }
}

/// Spawn the dialog's window, hidden. Install before the keyboard.
pub(crate) fn install(app: &mut App) {
    if app.has_resource::<Dialog>() {
        return;
    }
    let icons = {
        let mut atlas = app.resource_mut::<Atlas>();
        let mut load = |svg, size, what| {
            atlas
                .insert_svg(svg, size)
                .unwrap_or_else(|e| panic!("the {what} icon loads: {e:?}"))
        };
        Icons {
            close: load(assets::icons::CLEAR, HEADER_ICON as u32, "x"),
            confirm: load(assets::icons::CHECK, HEADER_ICON as u32, "check"),
            show: load(assets::icons::EYE, ICON, "eye"),
            hide: load(assets::icons::EYE_SLASH, ICON, "eye-slash"),
            warning: load(assets::icons::WARNING, ICON, "warning"),
        }
    };
    app.insert_resource(icons);
    let root = app.root();
    let role = Role::Layer(LayerRole {
        layer: Layer::Top,
        anchor: ALL_SIDES,
        // Clear of the zones others reserve: below the status bar.
        exclusive_zone: 0,
        namespace: NAMESPACE.into(),
        // A hardware keyboard types into it once it is tapped.
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        shell_role: Some(ShellRole::Prompt),
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
    let view = app.spawn(window, PolkitDialogBuilder);
    app.insert_resource(Dialog {
        window,
        view,
        shown: None,
    });
    set_mapped(app, false);
    app.system(|app, e: &AuthRequested| {
        app.resource_mut::<Dialog>().shown = Some(Shown {
            cookie: e.0.cookie.clone(),
            message: e.0.message.clone(),
            user: None,
            prompt: None,
            last_prompt: None,
            note: None,
            checking: false,
            locked: None,
        });
        set_mapped(app, true);
        refresh(app, Clear::All);
    })
    .system(|app, e: &AuthUser| {
        update(app, &e.cookie, |s| {
            s.user = Some(e.user.clone());
            false
        });
    })
    .system(|app, e: &AuthPrompt| {
        update(app, &e.cookie, |s| {
            s.last_prompt = Some(e.text.clone());
            s.prompt = Some(Prompt {
                text: e.text.clone(),
                echo: e.echo,
            });
            s.locked = None;
            std::mem::take(&mut s.checking)
        });
    })
    // Too many wrong answers: count down to the next attempt.
    .system(|app, e: &AuthLocked| {
        update(app, &e.cookie, |s| {
            s.prompt = None;
            s.checking = false;
            s.note = None;
            s.locked = Some(Instant::now() + e.wait);
            true
        });
        let view = app.resource::<Dialog>().view;
        app.emit(Countdown, view);
    })
    .system(|app, e: &AuthMessage| {
        update(app, &e.cookie, |s| {
            s.note = Some(Note {
                text: e.text.clone(),
                error: e.error,
            });
            false
        });
    })
    .system(|app, e: &AuthRetry| {
        update(app, &e.cookie, |s| {
            s.prompt = None;
            s.checking = false;
            s.note = Some(Note {
                text: WRONG.into(),
                error: true,
            });
            true
        });
    })
    .system(|app, e: &AuthAnswer| {
        update(app, &e.cookie, |s| {
            s.prompt = None;
            s.checking = true;
            s.note = None;
            false
        });
    })
    .system(|app, e: &AuthFinished| close(app, &e.cookie))
    .system(|app, e: &AuthCancelled| close(app, &e.cookie));
}

/// `change` says whether to empty the field.
fn update(app: &mut App, cookie: &str, change: impl FnOnce(&mut Shown) -> bool) {
    let empty_field = {
        let mut dialog = app.resource_mut::<Dialog>();
        let Some(shown) = dialog.shown(cookie) else {
            return;
        };
        change(shown)
    };
    refresh(
        app,
        if empty_field {
            Clear::Field
        } else {
            Clear::Nothing
        },
    );
}

fn close(app: &mut App, cookie: &str) {
    {
        let mut dialog = app.resource_mut::<Dialog>();
        if dialog.shown(cookie).is_none() {
            return;
        }
        dialog.shown = None;
    }
    refresh(app, Clear::All);
    set_mapped(app, false);
}

fn refresh(app: &mut App, clear: Clear) {
    let view = app.resource::<Dialog>().view;
    app.emit(Refresh { clear }, view);
}

fn set_mapped(app: &mut App, mapped: bool) {
    if !app.has_resource::<Surfaces>() || !app.has_resource::<Wayland>() {
        return;
    }
    let window = app.resource::<Dialog>().window;
    let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    surfaces.set_mapped(window, mapped, &mut wl);
}

/// Start the lockout's countdown.
struct Countdown;
impl Event for Countdown {}

struct Refresh {
    clear: Clear,
}
impl Event for Refresh {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Clear {
    Nothing,
    Field,
    /// Empty the field and hide ✓: a new request, or none.
    All,
}

#[cfg(test)]
mod tests;
