//! The dialog: a bottom sheet over the shell. Its header has Cancel (✕),
//! the title and Authenticate (✓); its body says as whom and what is
//! asked, and has the field for the answer with a line under it: how to
//! answer, or PAM's last word.
//!
//! - [`AuthRequested`] shows it for that request, the field empty and
//!   disabled until the helper asks something ([`AuthPrompt`]). The
//!   prompt is the field's placeholder; the field masks what is typed
//!   unless the prompt echoes, with an eye to show it.
//! - Return in the field, or ✓ once something is typed, sends the field's
//!   text as [`AuthAnswer`] and empties it. ✕, or dragging the sheet
//!   down, sends [`AuthCancel`].
//! - [`AuthMessage`] shows under the field; [`AuthRetry`] says to try
//!   again. Typing hides the line.
//! - [`AuthFinished`] or [`AuthCancelled`] for the request hides it.
//!
//! The window is a `prompt` (mx_shell_surface_v1): above every other
//! shell surface, except the on-screen keyboard while the prompt has the
//! keyboard focus. So its keyboard interactivity is on demand: a tap on
//! the field focuses the dialog and raises the keyboard over it. Without
//! that protocol (sway, say) only the layer orders it: it is on the top
//! layer, spawned before the keyboard's window, so the keys still show
//! over it. The window keeps out of the zones other surfaces reserve, so
//! it starts below the status bar. The sheet fills the window's height,
//! so its content stays at the top, clear of the keys.
//!
//! Not yet: closing for the lock screen, a picker when several users may
//! authenticate, the requesting program from the request's details.

use mecha_wayland::mechanix_widgets::prelude::{
    BottomSheet, BottomSheetContextExt, Button, ButtonBuilder, ButtonContextExt, ButtonSize,
    ButtonVariant, FontContextExt, FontWeight, SheetClosed, Text, TextContextExt, TextOverflow,
    TextVariant, bottom_sheet, button, text,
};
use mecha_wayland::prelude::*;

use crate::agent::{AuthCancelled, AuthFinished, AuthRequested};
use crate::helper::{AuthAnswer, AuthCancel, AuthMessage, AuthPrompt, AuthRetry, AuthUser};

const NAMESPACE: &str = "polkit-agent";

const ALL_SIDES: Anchor = Anchor::from_bits_truncate(
    Anchor::TOP.bits() | Anchor::BOTTOM.bits() | Anchor::LEFT.bits() | Anchor::RIGHT.bits(),
);

/// The icons' size in the atlas.
const ICON: u32 = 24;
/// The avatar's diameter.
const AVATAR: f32 = 56.0;
/// The header's buttons: square, as tall as a large button.
const HEADER_BUTTON: ButtonSize = ButtonSize::Large;

/// The placeholder before the helper asks anything.
const DEFAULT_PROMPT: &str = "Password";

/// The icons, loaded once. The eye's is what a press does.
#[derive(Clone, Copy)]
struct Icons {
    close: SpriteId,
    confirm: SpriteId,
    show: SpriteId,
    hide: SpriteId,
}
impl Resource for Icons {}

/// The question the helper asked, waiting for its answer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Prompt {
    text: String,
    echo: bool,
}

/// Said under the field.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    text: String,
    error: bool,
}

/// The request on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    cookie: String,
    message: String,
    user: Option<String>,
    /// Open: the field takes an answer.
    prompt: Option<Prompt>,
    /// The last prompt's text, kept as the placeholder between prompts.
    asked: Option<String>,
    note: Option<Note>,
}

struct Dialog {
    window: NodeId,
    view: Handle<PolkitDialog>,
    shown: Option<Shown>,
}
impl Resource for Dialog {}

impl Dialog {
    /// The shown request, if its cookie is `cookie`.
    fn shown(&mut self, cookie: &str) -> Option<&mut Shown> {
        self.shown.as_mut().filter(|s| s.cookie == cookie)
    }

    fn cookie(&self) -> Option<String> {
        self.shown.as_ref().map(|s| s.cookie.clone())
    }
}

/// Spawn the dialog's window, hidden. Needs the window, presentation and
/// theme modules, the [`Atlas`] and the `FontBook`; install before the
/// keyboard so its keys show over the dialog.
pub(crate) fn install(app: &mut App) {
    if app.has_resource::<Dialog>() {
        return;
    }
    let icons = {
        let mut atlas = app.resource_mut::<Atlas>();
        let mut load = |svg, what| {
            atlas
                .insert_svg(svg, ICON)
                .unwrap_or_else(|e| panic!("the {what} icon loads: {e:?}"))
        };
        Icons {
            close: load(assets::icons::CLEAR, "x"),
            confirm: load(assets::icons::CHECK, "check"),
            show: load(assets::icons::EYE, "eye"),
            hide: load(assets::icons::EYE_SLASH, "eye-slash"),
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
        });
        set_mapped(app, true);
        refresh(app, true);
    })
    .system(|app, e: &AuthUser| {
        update(app, &e.cookie, |s| s.user = Some(e.user.clone()));
    })
    .system(|app, e: &AuthPrompt| {
        update(app, &e.cookie, |s| {
            s.asked = Some(e.text.clone());
            s.prompt = Some(Prompt {
                text: e.text.clone(),
                echo: e.echo,
            });
        });
    })
    .system(|app, e: &AuthMessage| {
        update(app, &e.cookie, |s| {
            s.note = Some(Note {
                text: e.text.clone(),
                error: e.error,
            });
        });
    })
    .system(|app, e: &AuthRetry| {
        update(app, &e.cookie, |s| {
            s.prompt = None;
            s.note = Some(Note {
                text: "Sorry, that didn't work. Please try again.".into(),
                error: true,
            });
        });
    })
    // Sent: the field waits for the next question.
    .system(|app, e: &AuthAnswer| update(app, &e.cookie, |s| s.prompt = None))
    .system(|app, e: &AuthFinished| close(app, &e.cookie))
    .system(|app, e: &AuthCancelled| close(app, &e.cookie));
}

/// Change the shown request, if it is `cookie`'s, and show the change.
fn update(app: &mut App, cookie: &str, change: impl FnOnce(&mut Shown)) {
    {
        let mut dialog = app.resource_mut::<Dialog>();
        let Some(shown) = dialog.shown(cookie) else {
            return;
        };
        change(shown);
    }
    refresh(app, false);
}

/// Hide the dialog, if it shows `cookie`'s request.
fn close(app: &mut App, cookie: &str) {
    {
        let mut dialog = app.resource_mut::<Dialog>();
        if dialog.shown(cookie).is_none() {
            return;
        }
        dialog.shown = None;
    }
    refresh(app, true);
    set_mapped(app, false);
}

/// Show the state; `clear` empties the field too.
fn refresh(app: &mut App, clear: bool) {
    let view = app.resource::<Dialog>().view;
    app.emit(Refresh { clear }, view);
}

/// Map or unmap the dialog's window.
fn set_mapped(app: &mut App, mapped: bool) {
    if !app.has_resource::<Surfaces>() || !app.has_resource::<Wayland>() {
        return;
    }
    let window = app.resource::<Dialog>().window;
    let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    surfaces.set_mapped(window, mapped, &mut wl);
}

/// Show [`Dialog::shown`].
struct Refresh {
    /// Empty the field.
    clear: bool,
}
impl Event for Refresh {}

/// The dialog's view: the sheet and what is in it.
pub struct PolkitDialog {
    parts: Parts,
    /// The field holds nothing.
    empty: bool,
    /// A secret answer shows as typed.
    revealed: bool,
}

#[derive(Clone, Copy)]
struct Parts {
    sheet: Handle<BottomSheet>,
    ok: Handle<Button>,
    initial: Handle<Text>,
    user: Handle<Text>,
    message: Handle<Text>,
    input: Handle<Input>,
    placeholder: Handle<Text>,
    /// Shows or hides the text: there while the answer is secret and
    /// something is typed.
    reveal: Handle<Div>,
    eye: Handle<Icon>,
    note: Handle<Text>,
}

pub struct PolkitDialogBuilder;

impl Build for PolkitDialogBuilder {
    type Widget = PolkitDialog;
}

/// A square, round, icon-only button for the header.
fn header_button(variant: ButtonVariant, sprite: SpriteId) -> ButtonBuilder {
    let side = px(HEADER_BUTTON.height());
    button()
        .variant(variant)
        .size(HEADER_BUTTON)
        .width(side)
        .height(side)
        .padding(Insets::all(px(0.0)))
        .shape(Shape::Full)
        .icon(sprite)
}

impl Widget for PolkitDialog {
    type Builder = PolkitDialogBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .absolute()
            .inset(Insets::all(px(0.0)))
            .size(percent(100.0), percent(100.0));
        let icons = *s.resource::<Icons>();

        // As tall as the window: the content stays at the top, clear of
        // the on-screen keyboard.
        let sheet = s.spawn(me, bottom_sheet().height(percent(100.0)));
        let (header, body, title) = {
            let w = s.widget::<BottomSheet>(sheet).expect("the sheet is built");
            (w.header(), w.body(), w.title())
        };

        // The header: ✕, the title, ✓. The sheet's own title would sit
        // before the ✕, so ours replaces it.
        {
            let mut style = s.component_mut::<LayoutStyle>(title).unwrap();
            let hidden = style.clone().hidden();
            *style = hidden;
        }
        let cancel = s.spawn(
            header,
            header_button(ButtonVariant::Text, icons.close).text_color(ColorRole::OnSurface),
        );
        s.spawn(
            header,
            text("Authentication required")
                .variant(TextVariant::HeadlineSmall)
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
        let ok = s.spawn(
            header,
            header_button(ButtonVariant::Tonal, icons.confirm).disabled(true),
        );

        // As whom, and what for.
        let identity = s.spawn(
            body,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
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
                .background(s.color(ColorRole::SurfaceBright))
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

        // The field: the text over a line, the eye over its right end.
        let answer = s.spawn(
            body,
            div().style(
                LayoutStyle::default()
                    .column()
                    .width(percent(100.0))
                    .gap(px(Spacing::Space100.dp())),
            ),
        );
        let field = s.spawn(
            answer,
            div().style(LayoutStyle::default().relative().width(percent(100.0))),
        );
        let inset = px(Spacing::Space300.dp());
        let eye_room = px(HEADER_BUTTON.height());
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
                .content_hint(ContentHint::HIDDENTEXT | ContentHint::SENSITIVEDATA)
                .content_purpose(ContentPurpose::Password)
                .style(
                    LayoutStyle::default()
                        .row()
                        .align_items(Align::Center)
                        .width(percent(100.0))
                        .min_height(eye_room)
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
        // Over the field's right end, beside the input: a press on it keeps
        // the field's focus.
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
        s.spawn(
            answer,
            div()
                .style(LayoutStyle::default().size(percent(100.0), px(1.0)))
                .background(s.color(ColorRole::Primary)),
        );
        let note = s.spawn(
            answer,
            text("")
                .variant(TextVariant::BodySmall)
                .color(ColorRole::OnSurfaceVariant)
                .style(LayoutStyle::default().padding(Insets::new(px(0.0), inset, px(0.0), inset))),
        );

        s.on::<InputSubmit>(input, |ctx, _| submit(ctx));
        // After the input's own handlers: what it holds now.
        s.on::<InputFocus>(input, |ctx, _| typed(ctx));
        s.on::<InputEdit>(input, |ctx, _| typed(ctx));
        s.on::<Clicked>(ok, |ctx, _| submit(ctx));
        s.on::<Clicked>(reveal, |ctx, _| {
            ctx.me().revealed ^= true;
            sync_field(ctx);
        });
        s.on::<Clicked>(cancel, |ctx, _| cancel_shown(ctx));
        s.on::<SheetClosed>(sheet, |ctx, _| cancel_shown(ctx));
        s.on::<Refresh>(me, show);

        PolkitDialog {
            parts: Parts {
                sheet,
                ok,
                initial,
                user,
                message,
                input,
                placeholder,
                reveal,
                eye,
                note,
            },
            empty: true,
            revealed: false,
        }
    }
}

/// The user gave up on the shown request.
fn cancel_shown(ctx: &mut Context<'_, PolkitDialog>) {
    if let Some(cookie) = ctx.resource::<Dialog>().cookie() {
        ctx.signal(AuthCancel { cookie });
    }
}

/// Send what the field holds, if a prompt is open, and empty it.
fn submit(ctx: &mut Context<'_, PolkitDialog>) {
    let cookie = ctx
        .resource::<Dialog>()
        .shown
        .as_ref()
        .filter(|s| s.prompt.is_some())
        .map(|s| s.cookie.clone());
    let Some(cookie) = cookie else {
        return;
    };
    let input = ctx.me().parts.input;
    let Some(answer) = ctx.at(input).map(|mut i| i.take_text()) else {
        return;
    };
    ctx.me().empty = true;
    ctx.me().revealed = false;
    ctx.signal(AuthAnswer { cookie, answer });
}

/// The field's text changed, or it was focused: follow whether it is
/// empty.
fn typed(ctx: &mut Context<'_, PolkitDialog>) {
    let input = ctx.me().parts.input;
    let empty = ctx
        .widget::<Input>(input.id())
        .is_none_or(|i| i.text().is_empty());
    if ctx.me().empty == empty {
        return;
    }
    ctx.me().empty = empty;
    sync_field(ctx);
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

/// Show the field, its placeholder, eye, the line under it and ✓ from the
/// shown request and what is typed.
fn sync_field(ctx: &mut Context<'_, PolkitDialog>) {
    let shown = ctx.resource::<Dialog>().shown.clone();
    let (p, empty, revealed) = {
        let me = ctx.me();
        (me.parts, me.empty, me.revealed)
    };
    let icons = *ctx.resource::<Icons>();
    let prompt = shown.as_ref().and_then(|s| s.prompt.clone());
    let open = prompt.is_some();
    let secret = !prompt.as_ref().is_some_and(|q| q.echo);

    if let Some(mut i) = ctx.at(p.input) {
        i.set_hidden(secret && !revealed);
        i.set_disabled(!open);
    }
    let asked = shown.as_ref().and_then(|s| s.asked.as_deref());
    if let Some(mut t) = ctx.at(p.placeholder) {
        t.set_text(placeholder(asked.unwrap_or(DEFAULT_PROMPT)));
        t.set_display(display(empty));
    }
    if let Some(mut r) = ctx.at(p.reveal) {
        r.set_display(display(secret && !empty));
    }
    if let Some(mut e) = ctx.at(p.eye) {
        e.set_sprite(if revealed { icons.hide } else { icons.show });
    }
    if let Some(mut b) = ctx.at(p.ok) {
        b.set_disabled(!open || empty);
    }

    // Typing hides the line; empty, it says what happened or what to do.
    let note = shown.as_ref().and_then(|s| s.note.clone());
    let (line, error) = match note {
        _ if !empty || shown.is_none() => (String::new(), false),
        Some(n) => (n.text, n.error),
        None if secret => ("Type your password, then press Enter".into(), false),
        None => ("Type your answer, then press Enter".into(), false),
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
    let shown = ctx.resource::<Dialog>().shown.clone();
    let p = ctx.me().parts;
    if e.clear {
        if let Some(mut i) = ctx.at(p.input) {
            // Dropped: the text is wiped.
            drop(i.take_text());
        }
        ctx.me().empty = true;
        ctx.me().revealed = false;
    }
    // Back up, if the user dragged it away last time.
    if shown.is_some()
        && let Some(mut sheet) = ctx.at(p.sheet)
    {
        sheet.open();
    }
    let user = shown
        .as_ref()
        .and_then(|s| s.user.clone())
        .unwrap_or_default();
    let initial = user
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    let message = shown
        .as_ref()
        .map(|s| s.message.clone())
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
