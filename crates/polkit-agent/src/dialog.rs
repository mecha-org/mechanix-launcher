//! The polkit dialog: a bottom sheet asking for the password of the
//! request's user.
//!
//! Shown on [`AuthRequested`], hidden on [`AuthFinished`] or
//! [`AuthCancelled`]. Return or ✓ sends [`AuthAnswer`]; ✕ or dragging the
//! sheet down sends [`AuthCancel`].
//!
//! The window is a top-layer `prompt` (mx_shell_surface_v1), spawned
//! before the keyboard's, so the on-screen keyboard shows over it.
//!
//! Not yet: closing for the lock screen, a user picker, the requesting
//! program.

use mecha_wayland::mechanix_widgets::prelude::{
    BottomSheet, BottomSheetContextExt, FontContextExt, FontWeight, SheetClosed, Text,
    TextContextExt, TextOverflow, TextVariant, VerticalTrim, bottom_sheet, text,
};
use mecha_wayland::prelude::*;
use std::time::{Duration, Instant};

use crate::agent::{AuthCancelled, AuthFinished, AuthRequested};
use crate::helper::{
    AuthAnswer, AuthCancel, AuthLocked, AuthMessage, AuthPrompt, AuthRetry, AuthUser, UNSENDABLE,
    sendable,
};

const NAMESPACE: &str = "polkit-agent";

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

const ICON: u32 = 24;
/// The header's icons (✕, ✓), and their tap area.
const HEADER_ICON: f32 = 20.0;
const TAP: f32 = 48.0;
const FIELD_HEIGHT: f32 = 40.0;
const OK_CIRCLE: f32 = 32.0;
/// The header's side padding: ✕ (48), the title, ✓ (48), no gaps.
const HEADER_SIDE: f32 = 8.0;
const DISABLED_OPACITY: f32 = 0.10;
const AVATAR: f32 = 40.0;

const DEFAULT_PROMPT: &str = "Password";

const CHECKING: &str = "Authenticating…";

const WRONG: &str = "Incorrect password. Try again.";

const STROKE: f32 = 1.0;

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
    asked: Option<String>,
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
            asked: None,
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
            s.asked = Some(e.text.clone());
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

pub struct PolkitDialog {
    parts: Parts,
    empty: bool,
    revealed: bool,
    focused: bool,
    /// Something was typed for this request: ✓ shows from then on.
    started: bool,
    /// The field's length, to tell an edit from a focus change.
    len: usize,
    /// Refreshes the lockout's countdown every second.
    tick: TimerId,
}

#[derive(Clone, Copy)]
struct Parts {
    sheet: Handle<BottomSheet>,
    ok: Handle<Div>,
    ok_icon: Handle<Icon>,
    initial: Handle<Text>,
    user: Handle<Text>,
    message: Handle<Text>,
    input: Handle<Input>,
    placeholder: Handle<Text>,
    reveal: Handle<Div>,
    eye: Handle<Icon>,
    alert: Handle<Div>,
    line: Handle<Div>,
    note: Handle<Text>,
}

pub struct PolkitDialogBuilder;

impl Build for PolkitDialogBuilder {
    type Widget = PolkitDialog;
}

fn tap_area() -> LayoutStyle {
    LayoutStyle::default()
        .size(px(TAP), px(TAP))
        .shrink(0.0)
        .center()
}

impl Widget for PolkitDialog {
    type Builder = PolkitDialogBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .absolute()
            .inset(Insets::all(px(0.0)))
            .size(percent(100.0), percent(100.0));
        let icons = *s.resource::<Icons>();

        let sheet = s.spawn(me, bottom_sheet().height(percent(100.0)));
        let (header, body, title) = {
            let w = s.widget::<BottomSheet>(sheet).expect("the sheet is built");
            (w.header(), w.body(), w.title())
        };

        // Our title replaces the sheet's, which would sit before the ✕.
        {
            let mut style = s.component_mut::<LayoutStyle>(title).unwrap();
            let hidden = style.clone().hidden();
            *style = hidden;
        }
        {
            let mut style = s.component_mut::<LayoutStyle>(header).unwrap();
            let edged = style
                .clone()
                .justify(Justify::Start)
                .column_gap(px(0.0))
                .padding(Insets::new(
                    px(Spacing::Space100.dp()),
                    px(HEADER_SIDE),
                    px(Spacing::Space100.dp()),
                    px(HEADER_SIDE),
                ));
            *style = edged;
        }
        // Tighter above and below the user, so the field clears the keys.
        {
            let mut style = s.component_mut::<LayoutStyle>(body).unwrap();
            let raised = style
                .clone()
                .row_gap(px(Spacing::Space100.dp()))
                .padding(Insets::new(
                    px(0.0),
                    px(Spacing::Space300.dp()),
                    px(Spacing::Space300.dp()),
                    px(Spacing::Space300.dp()),
                ));
            *style = raised;
        }
        let header_icon = Size::new(HEADER_ICON, HEADER_ICON);
        let cancel = s.spawn(header, div().style(tap_area()));
        s.spawn(
            cancel,
            icon(icons.close)
                .size(header_icon)
                .color(s.color(ColorRole::OnSurface)),
        );
        let title_large = s.typography(TextVariant::TitleLarge);
        s.spawn(
            header,
            text("Authentication required")
                .variant(TextVariant::TitleLarge)
                .weight(FontWeight::Regular)
                .line_height(title_large.line_height)
                .letter_spacing(title_large.letter_spacing)
                .vertical_trim(VerticalTrim::CapHeight)
                .color(ColorRole::OnSurface)
                .max_lines(1)
                .overflow(TextOverflow::Ellipsis)
                // At the start, after ✕: the auto margin pushes ✓ to the end.
                .style(LayoutStyle::default().margin(Insets::new(
                    px(0.0),
                    auto(),
                    px(0.0),
                    px(0.0),
                ))),
        );
        let ok = s.spawn(header, div().style(tap_area().hidden()));
        let ok_circle = s.spawn(
            ok,
            div()
                .style(
                    LayoutStyle::default()
                        .size(px(OK_CIRCLE), px(OK_CIRCLE))
                        .center(),
                )
                .background(s.color(ColorRole::SurfaceContainerHighest))
                .radius(OK_CIRCLE / 2.0),
        );
        let ok_icon = s.spawn(
            ok_circle,
            icon(icons.confirm)
                .size(header_icon)
                .color(s.color(ColorRole::OnSurface).with_alpha(DISABLED_OPACITY)),
        );

        // The avatar stays beside the name however long the message is.
        let identity = s.spawn(
            body,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Start)
                    .gap(px(Spacing::Space200.dp())),
            ),
        );
        let avatar = s.spawn(
            identity,
            div()
                .style(
                    LayoutStyle::default()
                        .size(px(AVATAR), px(AVATAR))
                        .shrink(0.0)
                        .center(),
                )
                .background(s.color(ColorRole::SecondaryFixedDim))
                .radius(AVATAR / 2.0),
        );
        let initial = s.spawn(
            avatar,
            text("")
                .variant(TextVariant::TitleLarge)
                .color(ColorRole::Primary),
        );
        let who = s.spawn(
            identity,
            div().style(
                LayoutStyle::default()
                    .column()
                    .grow(1.0)
                    .shrink(1.0)
                    .min_width(px(0.0)),
            ),
        );
        let user = s.spawn(
            who,
            text("")
                .variant(TextVariant::TitleMedium)
                .color(ColorRole::OnSurface)
                .max_lines(1)
                .overflow(TextOverflow::Ellipsis),
        );
        let message = s.spawn(
            who,
            text("")
                .wrap(TextWrap::Word)
                .variant(TextVariant::BodyMedium)
                .color(ColorRole::OnSurfaceVariant),
        );

        let answer = s.spawn(
            body,
            div().style(LayoutStyle::default().column().width(percent(100.0))),
        );
        let field = s.spawn(
            answer,
            div().style(LayoutStyle::default().relative().width(percent(100.0))),
        );
        let inset = px(Spacing::Space200.dp());
        let eye_room = px(TAP);
        let font = s.font(FontWeight::Regular);
        // As the placeholder over it.
        let text_size = s.typography(TextVariant::BodyLarge).font_size.round() as u16;
        let input = s.spawn(
            field,
            input(font)
                .size(text_size)
                .color(s.color(ColorRole::OnSurface))
                .caret(s.color(ColorRole::Primary))
                .background(Color::TRANSPARENT)
                .border(0.0, Color::TRANSPARENT)
                .content_hint(ContentHint::HIDDENTEXT | ContentHint::SENSITIVEDATA)
                .content_purpose(ContentPurpose::Password)
                .style(
                    LayoutStyle::default()
                        .row()
                        .align_items(Align::Center)
                        .width(percent(100.0))
                        .min_height(px(FIELD_HEIGHT))
                        .padding(Insets::new(px(0.0), eye_room, px(0.0), inset)),
                ),
        );
        *s.component_mut::<LayoutStyle>(input).unwrap() =
            LayoutStyle::default().relative().width(percent(100.0));
        // Inside the input, so a press on it focuses the field.
        let hint_box = s.spawn(
            input,
            div().style(
                LayoutStyle::default()
                    .absolute()
                    .inset(Insets::new(px(0.0), eye_room, px(0.0), inset))
                    .row()
                    .align_items(Align::Center),
            ),
        );
        let placeholder = s.spawn(
            hint_box,
            text(DEFAULT_PROMPT)
                .variant(TextVariant::BodyLarge)
                .color(ColorRole::OnSurfaceVariant)
                .max_lines(1)
                .overflow(TextOverflow::Ellipsis),
        );
        // Beside the input, so a press on it keeps the field's focus.
        let reveal = s.spawn(
            field,
            div().style(
                LayoutStyle::default()
                    .absolute()
                    .inset(Insets::new(px(0.0), px(0.0), px(0.0), auto()))
                    .width(eye_room)
                    .center()
                    .hidden(),
            ),
        );
        let eye = s.spawn(
            reveal,
            icon(icons.show).color(s.color(ColorRole::OnSurface)),
        );
        let alert = s.spawn(
            field,
            div().style(
                LayoutStyle::default()
                    .absolute()
                    .inset(Insets::new(px(0.0), px(0.0), px(0.0), auto()))
                    .width(eye_room)
                    .center()
                    .hidden(),
            ),
        );
        s.spawn(alert, icon(icons.warning).color(s.color(ColorRole::Error)));
        let line = s.spawn(
            answer,
            div()
                .style(LayoutStyle::default().size(percent(100.0), px(STROKE)))
                .background(s.color(ColorRole::Primary)),
        );
        let note = s.spawn(
            answer,
            text("")
                .variant(TextVariant::BodySmall)
                .color(ColorRole::OnSurfaceVariant)
                .style(LayoutStyle::default().padding(Insets::new(
                    px(Spacing::Space100.dp()),
                    inset,
                    px(0.0),
                    inset,
                ))),
        );

        s.on::<InputSubmit>(input, |ctx, _| submit(ctx));
        // After the input's own handlers: what it holds now.
        s.on::<InputFocus>(input, |ctx, e| {
            let changed = ctx.me().focused != e.focused;
            ctx.me().focused = e.focused;
            if !typed(ctx) && changed {
                sync_field(ctx);
            }
        });
        s.on::<InputEdit>(input, |ctx, _| {
            typed(ctx);
        });
        s.on::<Clicked>(ok, |ctx, _| submit(ctx));
        s.on::<Clicked>(reveal, |ctx, _| {
            ctx.me().revealed ^= true;
            sync_field(ctx);
        });
        s.on::<Clicked>(cancel, |ctx, _| cancel_shown(ctx));
        s.on::<SheetClosed>(sheet, |ctx, _| cancel_shown(ctx));
        s.on::<Refresh>(me, show);
        let tick = s.add_interval(Duration::from_secs(1), |ctx, e| {
            let locked = ctx
                .resource::<Dialog>()
                .shown
                .as_ref()
                .and_then(|s| s.locked)
                .is_some_and(|until| until > Instant::now());
            if !locked {
                ctx.cancel_timer(e.id);
            }
            sync_field(ctx);
        });
        s.on::<Countdown>(me, |ctx, _| {
            let tick = ctx.me().tick;
            ctx.start_timer(tick);
        });

        PolkitDialog {
            parts: Parts {
                sheet,
                ok,
                ok_icon,
                initial,
                user,
                message,
                input,
                placeholder,
                reveal,
                eye,
                alert,
                line,
                note,
            },
            empty: true,
            revealed: false,
            focused: false,
            started: false,
            len: 0,
            tick,
        }
    }
}

fn cancel_shown(ctx: &mut Context<'_, PolkitDialog>) {
    if let Some(cookie) = ctx.resource::<Dialog>().cookie() {
        ctx.signal(AuthCancel { cookie });
    }
}

/// Send the typed answer; it stays in the field, masked, while checked.
fn submit(ctx: &mut Context<'_, PolkitDialog>) {
    let cookie = ctx
        .resource::<Dialog>()
        .shown
        .as_ref()
        .filter(|s| s.prompt.is_some())
        .map(|s| s.cookie.clone());
    let Some(cookie) = cookie.filter(|_| !ctx.me().empty) else {
        return;
    };
    let input = ctx.me().parts.input;
    let Some(answer) = ctx.widget::<Input>(input.id()).map(|i| i.secret()) else {
        return;
    };
    // The helper would refuse it: say why, and leave it to edit.
    if !sendable(answer.as_bytes()) {
        if let Some(s) = ctx.resource_mut::<Dialog>().shown.as_mut() {
            s.note = Some(Note {
                text: UNSENDABLE.into(),
                error: true,
            });
        }
        sync_field(ctx);
        return;
    }
    ctx.me().revealed = false;
    ctx.signal(AuthAnswer { cookie, answer });
}

/// Follow whether the field is empty; typing after an error clears it.
fn typed(ctx: &mut Context<'_, PolkitDialog>) -> bool {
    let input = ctx.me().parts.input;
    let len = ctx
        .widget::<Input>(input.id())
        .map_or(0, |i| i.text().len());
    let empty = len == 0;
    let edited = len != std::mem::replace(&mut ctx.me().len, len);
    let cleared = edited
        && !empty
        && ctx
            .resource_mut::<Dialog>()
            .shown
            .as_mut()
            .is_some_and(|s| s.note.take_if(|n| n.error).is_some());
    if ctx.me().empty == empty && !cleared {
        return false;
    }
    ctx.me().empty = empty;
    ctx.me().started |= !empty;
    sync_field(ctx);
    true
}

/// The lockout's message, `left` to go, in whole seconds rounded up.
fn locked_text(left: Duration) -> String {
    let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    let unit = if secs == 1 { "second" } else { "seconds" };
    format!("Too many attempts. Try again in {secs} {unit}")
}

/// The prompt as a placeholder: `Password: ` reads `Password`.
fn placeholder(prompt: &str) -> &str {
    let p = prompt.trim().trim_end_matches(':').trim_end();
    if p.is_empty() { DEFAULT_PROMPT } else { p }
}

fn display(shown: bool) -> Display {
    if shown {
        Display::Flex
    } else {
        Display::Hidden
    }
}

fn sync_field(ctx: &mut Context<'_, PolkitDialog>) {
    let (open, secret, note, hint, checking) = {
        let shown = ctx.resource::<Dialog>().shown.as_ref();
        let prompt = shown.and_then(|s| s.prompt.as_ref());
        let locked = shown
            .and_then(|s| s.locked)
            .map(|until| until.saturating_duration_since(Instant::now()))
            .filter(|left| !left.is_zero());
        let note = match locked {
            Some(left) => Some(Note {
                text: locked_text(left),
                error: true,
            }),
            None => shown.and_then(|s| s.note.clone()),
        };
        let asked = shown.and_then(|s| s.asked.as_deref());
        (
            prompt.is_some(),
            !prompt.is_some_and(|q| q.echo),
            note,
            placeholder(asked.unwrap_or(DEFAULT_PROMPT)).to_owned(),
            shown.is_some_and(|s| s.checking),
        )
    };
    let (p, empty, revealed, focused, started) = {
        let me = ctx.me();
        (me.parts, me.empty, me.revealed, me.focused, me.started)
    };
    let icons = *ctx.resource::<Icons>();
    let error = note.as_ref().is_some_and(|n| n.error);
    let accent = ctx.color(if error {
        ColorRole::Error
    } else {
        ColorRole::Primary
    });

    if let Some(mut i) = ctx.at(p.input) {
        i.set_hidden(secret && !revealed);
        i.set_disabled(!open);
        i.set_caret_color(accent);
    }
    if let Some(mut l) = ctx.at(p.line) {
        l.set_background(accent);
    }
    if let Some(mut t) = ctx.at(p.placeholder) {
        t.set_text(hint);
        t.set_display(display(empty));
    }
    if let Some(mut r) = ctx.at(p.reveal) {
        r.set_display(display(secret && !empty && focused && !error));
    }
    if let Some(mut e) = ctx.at(p.eye) {
        e.set_sprite(if revealed { icons.hide } else { icons.show });
    }
    if let Some(mut a) = ctx.at(p.alert) {
        a.set_display(display(error));
    }
    if let Some(mut b) = ctx.at(p.ok) {
        b.set_display(display(started));
    }
    let on_surface = ctx.color(ColorRole::OnSurface);
    if let Some(mut i) = ctx.at(p.ok_icon) {
        i.set_color(if open && !empty {
            on_surface
        } else {
            on_surface.with_alpha(DISABLED_OPACITY)
        });
    }

    let (line, error) = match note {
        Some(n) => (n.text, n.error),
        None if checking => (CHECKING.into(), false),
        None => Default::default(),
    };
    if let Some(mut t) = ctx.at(p.note) {
        t.set_display(display(!line.is_empty()));
        t.set_text(line);
        t.set_color(if error {
            ColorRole::Error
        } else {
            ColorRole::OnSurfaceVariant
        });
    }
}

fn show(ctx: &mut Context<'_, PolkitDialog>, e: &Refresh) {
    let (shown, user, message) = match &ctx.resource::<Dialog>().shown {
        Some(s) => (true, s.user.clone().unwrap_or_default(), s.message.clone()),
        None => (false, String::new(), String::new()),
    };
    let p = ctx.me().parts;
    if e.clear != Clear::Nothing {
        if let Some(mut i) = ctx.at(p.input) {
            // Dropped: the text is wiped.
            drop(i.take_text());
        }
        ctx.me().empty = true;
        ctx.me().len = 0;
        ctx.me().revealed = false;
    }
    if e.clear == Clear::All {
        ctx.me().started = false;
    }
    // Back up, if the user dragged it away last time.
    if shown && let Some(mut sheet) = ctx.at(p.sheet) {
        sheet.open();
    }
    let initial = user
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    if let Some(mut t) = ctx.at(p.initial) {
        t.set_text(initial);
    }
    if let Some(mut t) = ctx.at(p.user) {
        t.set_text(user);
    }
    if let Some(mut t) = ctx.at(p.message) {
        t.set_text(message);
    }
    sync_field(ctx);
}

#[cfg(test)]
mod tests;
