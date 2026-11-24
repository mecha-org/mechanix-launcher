use std::collections::HashMap;

use mecha_wayland::mechanix_widgets::NativeText;
use mecha_wayland::mechanix_widgets::prelude::FontBook;

use super::*;
use crate::agent::{Identity, Request};

const COOKIE: &str = "c1";

/// What the dialog sent: answers and cancels, by cookie.
#[derive(Default)]
struct Sent {
    answers: Vec<(String, String)>,
    cancels: Vec<String>,
}
impl Resource for Sent {}

fn app() -> App {
    let mut app = App::new();
    app.add_module(RingModule::default())
        .add_module(TimerModule)
        .add_module(LayoutModule)
        .add_module(WindowModule)
        .add_module(PaintModule)
        .add_module(InteractivityModule)
        .add_module(MechanixTheme::dark())
        // The window's layer role; presentation's, which needs Wayland.
        .register_component::<Role>();
    let mut atlas = Atlas::new();
    let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
    app.insert_resource(atlas);
    app.insert_resource(FontBook::new(font));
    app.init_resource::<Sent>();
    install(&mut app);
    app.system(|app: &mut App, e: &AuthAnswer| {
        let answer = String::from_utf8_lossy(e.answer.as_bytes()).into_owned();
        app.resource_mut::<Sent>()
            .answers
            .push((e.cookie.clone(), answer));
    })
    .system(|app: &mut App, e: &AuthCancel| {
        app.resource_mut::<Sent>().cancels.push(e.cookie.clone());
    });
    app.tick();
    app
}

fn signal<S: Signal>(app: &mut App, s: S) {
    app.signal(s);
    app.flush();
    app.tick();
}

fn request(app: &mut App, cookie: &str) {
    signal(
        app,
        AuthRequested(Request {
            action_id: "org.freedesktop.policykit.exec".into(),
            message: "Authentication is needed to run `/usr/bin/true`".into(),
            icon_name: String::new(),
            details: HashMap::new(),
            cookie: cookie.into(),
            identities: vec![Identity::User(1000)],
        }),
    );
}

fn prompt(app: &mut App, cookie: &str) {
    signal(
        app,
        AuthPrompt {
            cookie: cookie.into(),
            text: "Password: ".into(),
            echo: false,
        },
    );
}

/// A request for `dhanish`, its password asked for.
fn asking() -> App {
    let mut app = app();
    request(&mut app, COOKIE);
    signal(
        &mut app,
        AuthUser {
            cookie: COOKIE.into(),
            user: "dhanish".into(),
        },
    );
    prompt(&mut app, COOKIE);
    app
}

fn parts(app: &App) -> Parts {
    let view = app.resource::<Dialog>().view;
    app.widget::<PolkitDialog>(view).unwrap().parts
}

fn view(app: &App) -> &PolkitDialog {
    let view = app.resource::<Dialog>().view;
    app.widget::<PolkitDialog>(view).unwrap()
}

fn emit_to<E: Event>(app: &mut App, node: impl Into<NodeId>, e: E) {
    app.emit(e, node.into());
    app.flush();
    app.tick();
}

/// What an input method committing `text` does.
fn type_text(app: &mut App, text: &str) {
    let input = parts(app).input;
    emit_to(
        app,
        input,
        InputEdit {
            commit: Some(text.into()),
            ..Default::default()
        },
    );
}

fn backspace(app: &mut App, n: u32) {
    let input = parts(app).input;
    emit_to(
        app,
        input,
        InputEdit {
            delete_before: n,
            ..Default::default()
        },
    );
}

fn submit_field(app: &mut App) {
    let input = parts(app).input;
    emit_to(app, input, InputSubmit);
}

fn field(app: &App) -> &Input {
    app.widget::<Input>(parts(app).input.id()).unwrap()
}

fn text_of(app: &App, h: Handle<Text>) -> String {
    let native = app.widget::<Text>(h).unwrap().native();
    app.widget::<NativeText>(native)
        .unwrap()
        .props
        .string
        .clone()
}

fn shown(app: &App, node: impl Into<NodeId>) -> bool {
    app.component::<LayoutStyle>(node.into()).unwrap().display == Display::Flex
}

fn note(app: &App) -> Option<String> {
    let note = parts(app).note;
    shown(app, note).then(|| text_of(app, note))
}

fn ok_dimmed(app: &App) -> bool {
    let icon = parts(app).ok_icon;
    app.widget::<Icon>(icon).unwrap().color().a < 1.0
}

/// The ✕: the first header item holding an icon, before the ✓.
fn close_icon(app: &App) -> NodeId {
    let header = app
        .widget::<BottomSheet>(parts(app).sheet)
        .unwrap()
        .header();
    let holds_icon = |n: &NodeId| {
        app.children(*n)
            .is_some_and(|c| c.iter().any(|i| app.widget::<Icon>(*i).is_some()))
    };
    *app.children(header)
        .unwrap()
        .iter()
        .find(|n| holds_icon(n))
        .expect("the header has the ✕")
}

fn sent(app: &App) -> &Sent {
    app.resource::<Sent>()
}

#[test]
fn a_request_shows_who_and_what() {
    let app = asking();
    let p = parts(&app);
    assert_eq!(text_of(&app, p.user), "dhanish");
    assert_eq!(text_of(&app, p.initial), "D");
    assert_eq!(
        text_of(&app, p.message),
        "Authentication is needed to run `/usr/bin/true`"
    );
    assert_eq!(text_of(&app, p.placeholder), "Password");
    assert!(!field(&app).is_disabled());
    assert_eq!(note(&app), None);
}

#[test]
fn the_field_waits_for_the_first_prompt() {
    let mut app = app();
    request(&mut app, COOKIE);
    assert!(field(&app).is_disabled());
    prompt(&mut app, COOKIE);
    assert!(!field(&app).is_disabled());
}

#[test]
fn the_check_shows_once_typing_starts_and_stays() {
    let mut app = asking();
    assert!(!shown(&app, parts(&app).ok));

    type_text(&mut app, "a");
    assert!(shown(&app, parts(&app).ok));
    assert!(!ok_dimmed(&app));

    // Emptied again: still there, dimmed.
    backspace(&mut app, 1);
    assert!(shown(&app, parts(&app).ok));
    assert!(ok_dimmed(&app));
}

#[test]
fn submitting_sends_the_answer_and_shows_authenticating() {
    let mut app = asking();
    type_text(&mut app, "hunter2");
    submit_field(&mut app);

    assert_eq!(sent(&app).answers, [(COOKIE.into(), "hunter2".into())]);
    // Kept, masked and disabled, while it is checked.
    assert_eq!(field(&app).text(), "hunter2");
    assert!(field(&app).is_disabled());
    assert_eq!(note(&app).as_deref(), Some(CHECKING));
}

#[test]
fn an_empty_field_sends_nothing() {
    let mut app = asking();
    submit_field(&mut app);
    assert!(sent(&app).answers.is_empty());
    assert_eq!(note(&app), None);
}

#[test]
fn an_answer_too_long_is_not_sent_and_stays_to_edit() {
    let mut app = asking();
    let long = "x".repeat(600);
    type_text(&mut app, &long);
    submit_field(&mut app);

    assert!(sent(&app).answers.is_empty());
    assert!(!field(&app).is_disabled());
    assert_eq!(field(&app).text(), long);
    assert_eq!(note(&app).as_deref(), Some(UNSENDABLE));
    assert!(shown(&app, parts(&app).alert));

    // Editing clears the error; an answer that fits goes.
    backspace(&mut app, 595);
    assert_eq!(note(&app), None);
    submit_field(&mut app);
    assert_eq!(sent(&app).answers, [(COOKIE.into(), "xxxxx".into())]);
}

#[test]
fn an_answer_with_a_line_break_is_not_sent() {
    let mut app = asking();
    type_text(&mut app, "two\nlines");
    submit_field(&mut app);
    assert!(sent(&app).answers.is_empty());
    assert!(!field(&app).is_disabled());
    assert_eq!(note(&app).as_deref(), Some(UNSENDABLE));
}

#[test]
fn a_wrong_password_empties_the_field_and_shows_the_error() {
    let mut app = asking();
    type_text(&mut app, "wrong");
    submit_field(&mut app);
    signal(
        &mut app,
        AuthRetry {
            cookie: COOKIE.into(),
        },
    );

    assert_eq!(field(&app).text(), "");
    assert_eq!(note(&app).as_deref(), Some(WRONG));
    assert!(shown(&app, parts(&app).alert));
    // Still shown after the first attempt.
    assert!(shown(&app, parts(&app).ok));
}

#[test]
fn typing_again_clears_the_error() {
    let mut app = asking();
    type_text(&mut app, "wrong");
    submit_field(&mut app);
    signal(
        &mut app,
        AuthRetry {
            cookie: COOKIE.into(),
        },
    );
    prompt(&mut app, COOKIE);

    type_text(&mut app, "r");
    assert_eq!(note(&app), None);
    assert!(!shown(&app, parts(&app).alert));
}

#[test]
fn a_next_prompt_clears_the_sent_answer() {
    let mut app = asking();
    type_text(&mut app, "first");
    submit_field(&mut app);
    prompt(&mut app, COOKIE);

    assert_eq!(field(&app).text(), "");
    assert!(!field(&app).is_disabled());
    assert_eq!(note(&app), None);
}

#[test]
fn a_message_shows_under_the_field() {
    let mut app = asking();
    signal(
        &mut app,
        AuthMessage {
            cookie: COOKIE.into(),
            text: "Account expires soon".into(),
            error: false,
        },
    );
    assert_eq!(note(&app).as_deref(), Some("Account expires soon"));
    assert!(!shown(&app, parts(&app).alert));
}

#[test]
fn the_close_icon_cancels_the_request() {
    let mut app = asking();
    let cancel = close_icon(&app);
    emit_to(
        &mut app,
        cancel,
        Clicked {
            contact: ContactId::Mouse,
            position: Point::default(),
        },
    );
    assert_eq!(sent(&app).cancels, [COOKIE]);
}

#[test]
fn finishing_hides_and_wipes_and_the_next_request_starts_fresh() {
    let mut app = asking();
    type_text(&mut app, "hunter2");
    signal(
        &mut app,
        AuthFinished {
            cookie: COOKIE.into(),
            result: Ok(()),
        },
    );
    assert!(app.resource::<Dialog>().shown.is_none());
    assert_eq!(field(&app).text(), "");

    request(&mut app, "c2");
    assert!(!view(&app).started);
    assert!(!shown(&app, parts(&app).ok));
}

fn lock(app: &mut App, wait: Duration) {
    signal(
        app,
        AuthLocked {
            cookie: COOKIE.into(),
            wait,
        },
    );
}

#[test]
fn a_lockout_counts_down_then_the_next_prompt_reopens_the_field() {
    let mut app = asking();
    type_text(&mut app, "wrong");
    submit_field(&mut app);
    lock(&mut app, Duration::from_secs(60));

    assert_eq!(field(&app).text(), "");
    assert!(field(&app).is_disabled());
    assert_eq!(
        note(&app).as_deref(),
        Some("Too many attempts. Try again in 60 seconds")
    );
    assert!(shown(&app, parts(&app).alert));

    // A tick keeps counting while locked.
    let id = view(&app).tick;
    let node = app.resource::<Dialog>().view;
    emit_to(&mut app, node, Timeout { id });
    assert!(note(&app).unwrap().starts_with("Too many attempts"));

    prompt(&mut app, COOKIE);
    assert!(!field(&app).is_disabled());
    assert_eq!(note(&app), None);
    assert!(!shown(&app, parts(&app).alert));
}

#[test]
fn a_lockout_can_be_cancelled() {
    let mut app = asking();
    lock(&mut app, Duration::from_secs(60));
    let close = close_icon(&app);
    emit_to(
        &mut app,
        close,
        Clicked {
            contact: ContactId::Mouse,
            position: Point::default(),
        },
    );
    assert_eq!(sent(&app).cancels, [COOKIE]);
}

#[test]
fn the_lockout_rounds_up_to_whole_seconds() {
    assert_eq!(
        locked_text(Duration::from_secs(60)),
        "Too many attempts. Try again in 60 seconds"
    );
    assert_eq!(
        locked_text(Duration::from_millis(59_200)),
        "Too many attempts. Try again in 60 seconds"
    );
    assert_eq!(
        locked_text(Duration::from_millis(400)),
        "Too many attempts. Try again in 1 second"
    );
}

#[test]
fn another_requests_signals_are_ignored() {
    let mut app = asking();
    signal(
        &mut app,
        AuthRetry {
            cookie: "other".into(),
        },
    );
    signal(
        &mut app,
        AuthFinished {
            cookie: "other".into(),
            result: Ok(()),
        },
    );
    assert_eq!(note(&app), None);
    assert!(app.resource::<Dialog>().shown.is_some());
}

#[test]
fn a_prompt_reads_as_a_placeholder() {
    assert_eq!(placeholder("Password: "), "Password");
    assert_eq!(placeholder("  Verification code:"), "Verification code");
    assert_eq!(placeholder(":"), DEFAULT_PROMPT);
    assert_eq!(placeholder(""), DEFAULT_PROMPT);
}
