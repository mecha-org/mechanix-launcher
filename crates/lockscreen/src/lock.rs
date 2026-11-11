//! The lock's state: when to lock and unlock, the lock windows, and the
//! unlock conversation the view shows.

use ::status_bar::StatusBar;
use keyboard::KeyboardVisibilityChanged;
use mecha_wayland::prelude::*;
use services::{LockRequested, PrepareForSleep, SessionLockChanged, SleepReady, UnlockRequested};

use crate::LockSettings;
use crate::auth::{self, AuthEvent, FromWorker, Method, Outcome};
use crate::view::{self, Entry, ViewModel};

/// Lock the session now. Locked already does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockNow;
impl Signal for LockNow {}

/// The lock screen went up (`true`) or came down (`false`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockChanged(pub bool);
impl Signal for LockChanged {}

/// What the user did on the lock screen.
pub(crate) enum UserInput {
    Text(String),
    Backspace,
    Submit,
    /// Use this method instead.
    Switch(Method),
    /// A field's answer, sent as it stands.
    Answer(Secret),
    /// Show the input (from the glance screen).
    Expand,
    /// Back to the glance screen, dropping what was typed.
    Collapse,
}
impl Signal for UserInput {}

/// The last thing PAM said, shown under the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Note {
    pub text: String,
    pub error: bool,
}

/// A question PAM asked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Prompt {
    /// Hidden as typed.
    secret: bool,
    text: String,
}

/// One run of a method, from its start to its outcome. The typed one's
/// conversation is what the view shows.
#[derive(Default)]
struct Attempt {
    method: Option<Method>,
    /// The last question asked.
    prompt: Option<Prompt>,
    /// How many questions were asked.
    prompts: u32,
    /// An answer went out.
    answered: bool,
    /// An answer is out, or a failure delay runs: keys are ignored.
    busy: bool,
    /// Said during the attempt.
    note: Option<Note>,
    /// The PIN typed so far, ASCII digits; or a password answered before
    /// anything asked for it. (The view's field holds a password as it is
    /// typed.) Kept until a prompt takes it.
    typed: Secret,
}

impl Attempt {
    /// The method's own secret is asked for: its first question, a hidden
    /// one, or none yet. It stays so while its answer is checked.
    fn first_secret(&self) -> bool {
        self.prompts <= 1 && self.prompt.as_ref().is_none_or(|p| p.secret)
    }

    /// The PIN is typed on the numpad: the PIN method's first secret.
    /// Anything it asks after that goes in the field.
    fn on_numpad(&self) -> bool {
        self.method == Some(Method::Pin) && self.first_secret()
    }
}

#[derive(Default)]
pub(crate) struct Lockscreen {
    pub(crate) settings: LockSettings,
    /// The lock windows, one per output, while the lock is up. Every output
    /// must be covered; only the first shows the lock screen, the rest stay
    /// black.
    windows: Vec<(WlOutput, NodeId)>,
    /// The lock screen, in the first window: the one place to unlock from,
    /// with the shell's status bar and keys moved into it.
    view: Option<NodeId>,
    /// The shell's nodes moved into the view, each with the parent it goes
    /// back to.
    homes: Vec<(NodeId, NodeId)>,
    locked: bool,
    /// Asked to lock for sleep: say [`SleepReady`] once the compositor
    /// confirms.
    sleep_waiting: bool,
    focus_before: Option<NodeId>,
    /// Methods found unavailable since the lock went up.
    unavailable: Vec<Method>,
    /// Methods that have asked something since the lock went up. Only
    /// these show: a later attempt with one shows its input from the start,
    /// a first one only once it asks.
    asked: Vec<Method>,
    /// The typed method chosen with [`UserInput::Switch`].
    preferred: Option<Method>,
    /// The typed method's attempt: the one the input answers.
    attempt: Attempt,
    /// The methods that aren't typed, running alongside it from the lock
    /// on. Any success unlocks.
    others: Vec<Method>,
    /// The input shows; otherwise the glance screen: the time, and a tap to
    /// expand.
    expanded: bool,
}
impl Resource for Lockscreen {}

impl Lockscreen {
    /// The methods still worth trying, typed or not.
    fn available(&self, typed: bool) -> impl Iterator<Item = Method> + '_ {
        self.settings
            .methods
            .iter()
            .copied()
            .filter(move |m| m.is_typed() == typed && !self.unavailable.contains(m))
    }

    /// The typed methods still worth trying, preferred first.
    fn candidates(&self) -> Vec<Method> {
        let mut methods: Vec<Method> = self.available(true).collect();
        if let Some(i) = methods.iter().position(|m| Some(*m) == self.preferred) {
            methods[..=i].rotate_right(1);
        }
        methods
    }

    /// An empty PIN, with room for all its digits.
    fn empty_pin(&self) -> Secret {
        Secret::with_capacity(self.settings.pin_length.digits())
    }

    /// What was typed is a whole answer: every digit of the PIN, or a
    /// password.
    fn typed_whole(&self) -> bool {
        let typed = self.attempt.typed.len();
        if self.attempt.on_numpad() {
            typed == self.settings.pin_length.digits()
        } else {
            typed > 0
        }
    }
}

pub(crate) fn install(app: &mut App, settings: LockSettings) {
    let at_start = settings.lock_at_start;
    app.insert_resource(Lockscreen {
        settings,
        ..Lockscreen::default()
    });
    app.system(|app, _: &LockNow| lock(app))
        .system(|app, _: &LockRequested| lock(app))
        .system(|app, _: &UnlockRequested| unlock(app))
        .system(on_prepare_for_sleep)
        .system(|app, _: &SessionLocked| {
            app.signal(SessionLockChanged(true));
            release_sleep(app);
        })
        .system(|app, _: &SessionLockFinished| {
            tracing::warn!("the compositor ended the lock");
            auth::cancel_all(app);
            tear_down(app);
        })
        .system(on_output_added)
        .system(on_output_removed)
        .system(on_auth)
        .system(on_input)
        // The view lays out around the keyboard's keys while they show.
        .system(|app, _: &KeyboardVisibilityChanged| refresh(app));
    if at_start {
        lock(app);
    }
}

fn spawn_window(app: &mut App, output: WlOutput) -> NodeId {
    let root = app.root();
    app.spawn_with(
        root,
        window()
            .title("lock")
            .layout(LayoutStyle::default().column())
            .clear(Color::BLACK),
        (Role::Lock(output),),
    )
    .id()
}

/// Show the lock screen in the first window, if it isn't up yet: the view,
/// the shell's status bar and keys moved into it, and the keyboard focus.
fn show_view(app: &mut App) {
    let first = {
        let ls = app.resource::<Lockscreen>();
        if !ls.locked || ls.view.is_some() {
            return;
        }
        let Some(&(_, first)) = ls.windows.first() else {
            return;
        };
        first
    };
    // The lock starts with the keyboard down: its field raises it.
    keyboard::hide_keyboard(app);
    let view = view::spawn(app, first);
    let (status, keys) = view::slots(app, view);
    let bar = app.widgets::<StatusBar>().next().map(|(id, _)| id);
    for (node, slot) in [(bar, status), (keyboard::keys(app), keys)] {
        let Some(node) = node else { continue };
        let home = app.parent(node).expect("a shared node has a parent");
        app.resource_mut::<Lockscreen>().homes.push((node, home));
        app.move_to(node, slot);
    }
    app.resource_mut::<Lockscreen>().view = Some(view);
    app.resource_mut::<KeyboardFocus>().focus(first);
}

/// Take the lock screen down: the shell's nodes go back where they were,
/// then the view goes. Its window stays.
fn drop_view(app: &mut App) {
    let (view, homes) = {
        let mut ls = app.resource_mut::<Lockscreen>();
        (ls.view.take(), std::mem::take(&mut ls.homes))
    };
    for (node, home) in homes {
        if app.is_live(node) && app.is_live(home) {
            app.move_to(node, home);
        }
    }
    if let Some(view) = view {
        app.remove(view);
    }
}

/// Put the lock up: ask the compositor, cover every output, start the
/// first typed method and every other. Nothing when locking is off: a
/// sleep waiting on the lock goes on.
fn lock(app: &mut App) {
    let (locked, enabled) = {
        let ls = app.resource::<Lockscreen>();
        (ls.locked, ls.settings.enabled)
    };
    if !enabled {
        release_sleep(app);
    }
    if locked || !enabled {
        return;
    }
    let asked = {
        let (mut session, mut wl) = app.query::<(ResMut<SessionLock>, ResMut<Wayland>)>();
        session.lock(&mut wl)
    };
    if !asked {
        tracing::warn!("the compositor can't lock the session now");
        release_sleep(app);
        return;
    }
    let outputs: Vec<WlOutput> = app.resource::<Outputs>().iter().collect();
    let windows = outputs
        .into_iter()
        .map(|o| (o, spawn_window(app, o)))
        .collect();
    let focus_before = app.resource::<KeyboardFocus>().focused();
    {
        let mut ls = app.resource_mut::<Lockscreen>();
        ls.windows = windows;
        ls.locked = true;
        ls.focus_before = focus_before;
        ls.unavailable.clear();
        ls.asked.clear();
        ls.preferred = None;
        ls.expanded = false;
    }
    show_view(app);
    app.signal(LockChanged(true));
    start_others(app);
    next_attempt(app);
}

/// Start every method that isn't typed: each runs until the lock goes, or
/// until it fails.
fn start_others(app: &mut App) {
    let methods: Vec<Method> = app.resource::<Lockscreen>().available(false).collect();
    for m in methods {
        let started = auth::start(app, m);
        let mut ls = app.resource_mut::<Lockscreen>();
        if started {
            ls.others.push(m);
        } else {
            ls.unavailable.push(m);
        }
    }
}

/// Take the lock down.
fn unlock(app: &mut App) {
    if !app.resource::<Lockscreen>().locked {
        return;
    }
    auth::cancel_all(app);
    {
        let (mut session, mut wl) = app.query::<(ResMut<SessionLock>, ResMut<Wayland>)>();
        session.unlock(&mut wl);
    }
    tear_down(app);
}

/// Forget the lock: the windows, the conversation, the focus.
fn tear_down(app: &mut App) {
    {
        let mut ls = app.resource_mut::<Lockscreen>();
        ls.locked = false;
        ls.attempt = Attempt::default();
        ls.others.clear();
    }
    drop_view(app);
    let (windows, focus) = {
        let mut ls = app.resource_mut::<Lockscreen>();
        (std::mem::take(&mut ls.windows), ls.focus_before.take())
    };
    for (_, w) in windows {
        app.remove(w);
    }
    {
        let mut kf = app.resource_mut::<KeyboardFocus>();
        match focus {
            Some(n) => kf.focus(n),
            None => kf.blur(),
        }
    }
    app.signal(SessionLockChanged(false));
    app.signal(LockChanged(false));
    release_sleep(app);
}

fn release_sleep(app: &mut App) {
    if std::mem::take(&mut app.resource_mut::<Lockscreen>().sleep_waiting) {
        app.signal(SleepReady);
    }
}

/// Start the best typed method left, ending the one before, or settle on
/// tap to unlock or on nothing to type.
fn next_attempt(app: &mut App) {
    let (candidates, none_configured, before) = {
        let ls = app.resource::<Lockscreen>();
        let none = ls.settings.methods.is_empty();
        (ls.candidates(), none, ls.attempt.method)
    };
    if let Some(m) = before {
        auth::cancel(app, m);
    }
    let mut started = None;
    for m in candidates {
        if auth::start(app, m) {
            started = Some(m);
            break;
        }
        app.resource_mut::<Lockscreen>().unavailable.push(m);
    }
    {
        let mut ls = app.resource_mut::<Lockscreen>();
        // What was said about the last attempt stays until this one says
        // something; with nothing left to start, say so.
        let note = if started.is_none() && !none_configured && ls.others.is_empty() {
            Some(Note {
                text: "No way to unlock is set up".into(),
                error: true,
            })
        } else {
            ls.attempt.note.take()
        };
        ls.attempt = Attempt {
            method: started,
            note,
            typed: ls.empty_pin(),
            ..Attempt::default()
        };
    }
    refresh(app);
}

/// Whether the compositor has the lock up.
pub(crate) fn is_locked(app: &App) -> bool {
    app.resource::<Lockscreen>().locked
}

/// The lock screen's settings.
pub(crate) fn settings(app: &App) -> &LockSettings {
    &app.resource::<Lockscreen>().settings
}

/// The view of the state, for the lock screen.
pub(crate) fn refresh(app: &mut App) {
    let model = {
        let ls = app.resource::<Lockscreen>();
        let a = &ls.attempt;
        // A method shows once it has asked something: one that fails
        // without asking (no PIN set) never shows at all.
        let shown = a.method.filter(|m| ls.asked.contains(m));
        let entry_kind = match (shown, ls.settings.methods.is_empty()) {
            (_, true) => Entry::Unlock,
            (Some(_), _) if a.on_numpad() => Entry::Pin,
            (Some(_), _) => Entry::Password,
            (None, false) => Entry::None,
        };
        // The method's own secret is asked for by name, whatever PAM calls
        // it: a PIN module asks with the generic "Password". Anything after
        // it, PAM's words.
        let hidden = a.prompt.as_ref().is_none_or(|p| p.secret);
        let prompt = match (shown, &a.prompt) {
            (None, _) => None,
            (Some(Method::Pin), _) if a.first_secret() => Some("Enter PIN to unlock".into()),
            (Some(Method::Password), _) if a.first_secret() => {
                Some("Enter password to unlock".into())
            }
            (_, p) => p.as_ref().map(|p| p.text.clone()),
        };
        ViewModel {
            expanded: ls.expanded,
            entry_kind,
            prompt,
            hidden,
            typed: a.typed.len(),
            pin_length: ls.settings.pin_length.digits(),
            note: a.note.clone(),
            busy: a.busy,
            keyboard: keyboard::is_keyboard_visible(app),
            switch: a
                .method
                .and_then(|current| ls.candidates().into_iter().find(|m| *m != current)),
        }
    };
    if let Some(view) = app.resource::<Lockscreen>().view {
        view::show(app, view, model);
    }
}

/// The typed method, if a prompt of its waits for an answer; if so, the
/// answer is out from now on.
fn take_turn(app: &mut App) -> Option<Method> {
    let a = &mut app.resource_mut::<Lockscreen>().attempt;
    let open = !a.busy && a.prompt.is_some();
    if open {
        a.busy = true;
        a.answered = true;
        a.note = None;
    }
    a.method.filter(|_| open)
}

/// Answer the prompt with `answer`. Nothing while busy or with nothing
/// asked.
fn send(app: &mut App, answer: &Secret) {
    if let Some(m) = take_turn(app) {
        auth::answer(app, m, answer.as_bytes());
    }
    refresh(app);
}

/// Send what was typed. Kept while nothing asks for it.
fn send_typed(app: &mut App) {
    if let Some(m) = take_turn(app) {
        let typed = {
            let mut ls = app.resource_mut::<Lockscreen>();
            let empty = ls.empty_pin();
            std::mem::replace(&mut ls.attempt.typed, empty)
        };
        auth::answer(app, m, typed.as_bytes());
    }
    refresh(app);
}

/// Back to the glance screen: what was typed goes.
pub(crate) fn collapse(app: &mut App) {
    {
        let mut ls = app.resource_mut::<Lockscreen>();
        if !ls.locked || !ls.expanded {
            return;
        }
        ls.expanded = false;
        ls.attempt.typed = ls.empty_pin();
    }
    refresh(app);
}

fn on_prepare_for_sleep(app: &mut App, e: &PrepareForSleep) {
    if !e.0 {
        return;
    }
    app.resource_mut::<Lockscreen>().sleep_waiting = true;
    if !app.resource::<Lockscreen>().locked {
        lock(app);
    } else if app.resource::<SessionLock>().state() == LockState::Locked {
        release_sleep(app);
    }
}

fn on_output_added(app: &mut App, e: &OutputAdded) {
    if !app.resource::<Lockscreen>().locked {
        return;
    }
    let w = spawn_window(app, e.0);
    app.resource_mut::<Lockscreen>().windows.push((e.0, w));
    // The first output back after none: the lock screen with it.
    show_view(app);
    refresh(app);
}

/// An output went: its window goes. The lock screen's own output going
/// takes it to the next first window.
fn on_output_removed(app: &mut App, e: &OutputRemoved) {
    let (gone, had_view) = {
        let mut ls = app.resource_mut::<Lockscreen>();
        let Some(i) = ls.windows.iter().position(|w| w.0 == e.0) else {
            return;
        };
        (ls.windows.remove(i).1, i == 0)
    };
    if had_view {
        // Before the window goes: removing it would free the shell's nodes.
        drop_view(app);
    }
    app.remove(gone);
    if had_view {
        show_view(app);
        refresh(app);
    }
}

/// Add what PAM said to `note`. faillock says two things in a row
/// (locked, and for how long): keep both.
fn add_note(note: &mut Option<Note>, text: &str, error: bool) {
    let text = text.trim();
    *note = Some(match note.take() {
        Some(n) if !n.text.is_empty() => Note {
            text: format!("{} {text}", n.text),
            error: n.error || error,
        },
        _ => Note {
            text: text.to_string(),
            error,
        },
    });
}

fn on_auth(app: &mut App, e: &AuthEvent) {
    let (typed, other) = {
        let ls = app.resource::<Lockscreen>();
        (
            ls.attempt.method == Some(e.method),
            ls.others.contains(&e.method),
        )
    };
    if other {
        on_other(app, e);
        return;
    }
    if !typed {
        return;
    }
    if let FromWorker::Done(outcome) = e.message {
        finished(app, e.method, outcome);
        return;
    }
    if matches!(e.message, FromWorker::Prompt { .. }) {
        let mut ls = app.resource_mut::<Lockscreen>();
        if !ls.asked.contains(&e.method) {
            ls.asked.push(e.method);
        }
    }
    {
        let a = &mut app.resource_mut::<Lockscreen>().attempt;
        match &e.message {
            FromWorker::Prompt { secret, text } => {
                a.prompt = Some(Prompt {
                    secret: *secret,
                    text: text.trim_end_matches([':', ' ']).to_string(),
                });
                a.prompts += 1;
                a.busy = false;
            }
            FromWorker::Info(text) => add_note(&mut a.note, text, false),
            FromWorker::Error(text) => add_note(&mut a.note, text, true),
            FromWorker::Delay(_) => a.busy = true,
            FromWorker::Done(_) => unreachable!("handled above"),
        }
    }
    // An answer typed before it was asked for goes now.
    if matches!(e.message, FromWorker::Prompt { .. }) && app.resource::<Lockscreen>().typed_whole()
    {
        send_typed(app);
    } else {
        refresh(app);
    }
}

/// A method that isn't typed: what it says shows with the typed one's
/// conversation, its success unlocks, and anything else sets it aside
/// until the next lock. It has no input to ask with: a question from it
/// goes unanswered.
fn on_other(app: &mut App, e: &AuthEvent) {
    match &e.message {
        FromWorker::Done(Outcome::Success) => unlock(app),
        FromWorker::Done(_) => {
            {
                let mut ls = app.resource_mut::<Lockscreen>();
                ls.others.retain(|m| *m != e.method);
                ls.unavailable.push(e.method);
            }
            refresh(app);
        }
        FromWorker::Info(text) | FromWorker::Error(text) => {
            let error = matches!(e.message, FromWorker::Error(_));
            add_note(
                &mut app.resource_mut::<Lockscreen>().attempt.note,
                text,
                error,
            );
            refresh(app);
        }
        FromWorker::Prompt { .. } => {
            tracing::warn!("{:?} asked a question: it has no input", e.method);
        }
        FromWorker::Delay(_) => {}
    }
}

fn finished(app: &mut App, method: Method, outcome: Outcome) {
    if outcome == Outcome::Success {
        unlock(app);
        return;
    }
    {
        let mut guard = app.resource_mut::<Lockscreen>();
        let ls = &mut *guard;
        let a = &mut ls.attempt;
        if outcome == Outcome::Failed && a.answered {
            // Wrong: try again, keeping what PAM said, if anything.
            a.note.get_or_insert_with(|| Note {
                text: match method {
                    Method::Pin => "Wrong PIN".into(),
                    Method::Password => "Wrong password".into(),
                },
                error: true,
            });
        } else {
            // Failed without asking anything (a stack that only denies),
            // unavailable, or broken: skip it until the next lock.
            if outcome != Outcome::Error {
                a.note = None;
            }
            ls.unavailable.push(method);
        }
    }
    next_attempt(app);
}

fn on_input(app: &mut App, e: &UserInput) {
    if !app.resource::<Lockscreen>().locked {
        return;
    }
    match e {
        UserInput::Switch(m) => {
            {
                let mut ls = app.resource_mut::<Lockscreen>();
                ls.preferred = Some(*m);
                ls.attempt.note = None;
            }
            next_attempt(app);
        }
        UserInput::Expand => {
            app.resource_mut::<Lockscreen>().expanded = true;
            refresh(app);
        }
        UserInput::Collapse => collapse(app),
        UserInput::Answer(answer) => {
            let early = {
                let a = &mut app.resource_mut::<Lockscreen>().attempt;
                let early = !a.busy && a.prompt.is_none() && a.method.is_some();
                if early {
                    a.typed = Secret::from(answer.as_bytes().to_vec());
                }
                early
            };
            if early {
                refresh(app);
            } else {
                send(app, answer);
            }
        }
        UserInput::Submit if app.resource::<Lockscreen>().settings.methods.is_empty() => {
            unlock(app);
        }
        UserInput::Submit => {
            let numpad = {
                let mut ls = app.resource_mut::<Lockscreen>();
                ls.expanded = true;
                ls.attempt.on_numpad()
            };
            if numpad {
                send_typed(app);
            } else {
                refresh(app);
            }
        }
        // PIN digits: as many as the PIN has, sent as soon as it has them
        // all, or kept until it is asked for. Typing on the glance screen
        // opens the input, and counts.
        UserInput::Text(t) => {
            let full = {
                let mut guard = app.resource_mut::<Lockscreen>();
                let ls = &mut *guard;
                ls.expanded = true;
                let a = &mut ls.attempt;
                let numpad = !a.busy && a.on_numpad();
                if numpad {
                    for digit in t.bytes().filter(u8::is_ascii_digit) {
                        a.typed.push(digit);
                    }
                }
                numpad && ls.typed_whole()
            };
            if full {
                send_typed(app);
            } else {
                refresh(app);
            }
        }
        UserInput::Backspace => {
            {
                let mut ls = app.resource_mut::<Lockscreen>();
                ls.expanded = true;
                if !ls.attempt.busy {
                    ls.attempt.typed.pop();
                }
            }
            refresh(app);
        }
    }
}
