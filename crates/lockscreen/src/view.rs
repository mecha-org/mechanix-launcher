//! The lock screen, in one lock window: the time and date, the prompt and
//! what has been typed, what PAM last said, and the input for the method in
//! use.
//!
//! It shows a [`ViewModel`]; the state lives in
//! [`Lockscreen`](crate::lock::Lockscreen), and input goes back to it as
//! [`UserInput`] signals, from the swipe up, the numpad, the bar and a
//! hardware keyboard alike. The field types as any app's does, the
//! on-screen keyboard's included.
//!
//! Standard widgets throughout: `button()`s for the numpad and the bar,
//! and the `Input` field for a password, whose text, caret and editing are
//! its own. The status bar and the keyboard's keys are the shell's own,
//! moved in while locked (see [`slots`]).

mod field;
mod layout;
mod numpad;
mod swipe;

use mecha_wayland::mechanix_widgets::prelude::{
    Button, ButtonBuilder, ButtonContextExt, ButtonSize, ButtonVariant, Clock, ClockInterval, Text,
    TextContextExt, button, clock, text,
};
use mecha_wayland::prelude::*;
use power_menu::{OpenPowerMenu, PowerMenu, PowerMenuBuilder};

use self::field::Field;
use self::layout::layout;
use self::numpad::NumpadCell;
use self::swipe::{SwipeUp, Swiped};
use crate::auth::Method;
use crate::lock::{Note, UserInput};

/// Which input the method in use takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Pin,
    Password,
    /// No methods configured: the swipe up unlocks.
    Unlock,
    /// Nothing to answer: starting, or no method left.
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ViewModel {
    /// The input shows; otherwise the glance screen.
    pub expanded: bool,
    pub entry_kind: Entry,
    /// What is asked, if anything.
    pub prompt: Option<String>,
    /// The answer is secret: dots for a PIN, a masked field otherwise.
    pub hidden: bool,
    pub typed: usize,
    /// How many dots a PIN shows.
    pub pin_length: usize,
    pub note: Option<Note>,
    /// An answer is being checked, or a failure delay runs: keys are
    /// ignored.
    pub busy: bool,
    /// The keyboard's keys show: its field raised them, and they weren't
    /// hidden since.
    pub keyboard: bool,
    /// The method to switch to, if there is another.
    pub switch: Option<Method>,
}

/// Show `model` in `view`.
pub(crate) fn show(app: &mut App, view: NodeId, model: ViewModel) {
    app.emit(Show(model), view);
}

#[derive(Clone)]
struct Show(ViewModel);
impl Event for Show {}

/// Open the lock screen's power menu.
pub(crate) fn open_power_menu(app: &mut App) {
    let menus: Vec<NodeId> = app
        .widgets::<LockView>()
        .map(|(_, view)| view.power_menu.id())
        .collect();
    app.emit(OpenPowerMenu, menus);
}

/// A PIN dot, and as many as the longest PIN has.
const DOT: f32 = 14.0;
const MAX_DOTS: usize = 6;
const SMALL: f32 = 12.0;

pub(crate) fn install(app: &mut App) {
    numpad::install(app);
    field::install(app);
}

/// Spawn the lock screen in `window`, as its one child.
pub(crate) fn spawn(app: &mut App, window: NodeId) -> NodeId {
    app.spawn(window, LockViewBuilder { window }).id()
}

/// Where `view` takes the status bar and the keyboard's keys.
pub(crate) fn slots(app: &App, view: NodeId) -> (NodeId, NodeId) {
    let parts = app.widget::<LockView>(view).expect("a lock view").parts;
    (parts.status.id(), parts.keyboard.id())
}

struct LockViewBuilder {
    window: NodeId,
}
impl Build for LockViewBuilder {
    type Widget = LockView;
}

/// The view's nodes. Handles are copies, so a handler takes them all at
/// once with `ctx.me().parts`.
#[derive(Clone, Copy)]
struct Parts {
    body: Handle<Div>,
    time: Handle<Div>,
    clock: Handle<Clock>,
    date: Handle<Clock>,
    input: Handle<Div>,
    title: Handle<Text>,
    dots: Handle<Div>,
    dot: [Handle<Div>; MAX_DOTS],
    /// The field for an answer that isn't a PIN: masked for a secret.
    field: Field,
    numpad: Handle<Div>,
    /// Where the status bar and the keyboard's keys go, in the window that
    /// has them (see [`slots`]).
    status: Handle<Div>,
    keyboard: Handle<Div>,
    bar: Handle<Div>,
    hint: Handle<Text>,
    cancel: Handle<Button>,
    switch: Handle<Button>,
}

/// The status bar across the top, and under it the layout (see
/// [`layout`]), from four parts: the time, the input (what is
/// asked, the dots or the field, the numpad; only once expanded), the bar,
/// and the keyboard.
struct LockView {
    window: NodeId,
    parts: Parts,
    /// Over everything else in the view: nothing but the lock windows
    /// shows while locked.
    power_menu: Handle<PowerMenu>,
    cells: Vec<NumpadCell>,
    shown: Option<ViewModel>,
    /// The field shows a secret as typed, until it next hides.
    revealed: bool,
    /// The glance screen's swipe up.
    swipe: SwipeUp,
    /// How far the hint may follow it, from the swipe's start.
    swipe_room: f32,
    /// The window's size, from its last `Resized`.
    size: Size,
}

impl ViewModel {
    /// A PIN is typed as dots.
    fn dots(&self) -> bool {
        self.entry_kind == Entry::Pin && self.hidden
    }
}

impl LockView {
    /// Whether the field takes the answer now.
    fn field_shown(&self) -> bool {
        self.shown.as_ref().is_some_and(|m| {
            m.expanded && matches!(m.entry_kind, Entry::Pin | Entry::Password) && !m.dots()
        })
    }

    /// The field takes typing: shown, and no answer being checked. The
    /// focus, the keys and the field's being enabled all follow this,
    /// changing in the same frame.
    fn typing(&self) -> bool {
        self.field_shown() && !self.shown.as_ref().is_some_and(|m| m.busy)
    }

    /// The keyboard's keys show: the field takes typing, and the keyboard
    /// is up for it. Not the keyboard's word alone: that comes back from
    /// the compositor only after the field has gone.
    fn keys_shown(&self) -> bool {
        self.typing() && self.shown.as_ref().is_some_and(|m| m.keyboard)
    }

    /// The glance screen shows: the swipe up leaves it. With no method
    /// there is nothing else to show.
    fn glancing(&self) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|m| !m.expanded || m.entry_kind == Entry::Unlock)
    }
}

/// A bar action: a standard text button.
fn bar_button(label: &str) -> ButtonBuilder {
    button()
        .variant(ButtonVariant::Text)
        .size(ButtonSize::Small)
        .text_color(ColorRole::OnSurfaceVariant)
        .label(label)
}

/// The time and the date.
fn spawn_time(
    s: &mut Spawner<'_, LockView>,
    body: Handle<Div>,
) -> (Handle<Div>, Handle<Clock>, Handle<Clock>) {
    let time = s.spawn(body, div());
    let clock_ = s.spawn(
        time,
        clock("%H:%M")
            .interval(ClockInterval::Minute)
            .size(TIME_SIZES[0])
            .color(ColorRole::OnSurface),
    );
    let date = s.spawn(
        time,
        clock(DATE)
            .interval(ClockInterval::Day)
            .size(SMALL)
            .color(ColorRole::Primary),
    );
    (time, clock_, date)
}

/// The time's size, from the glance screen's to the expanded view's. A
/// swipe steps through them rather than following the finger exactly:
/// every size is a set of glyphs the atlas keeps for good.
const TIME_SIZES: [f32; 4] = [46.0, 42.0, 37.0, 32.0];
/// The date's, expanded.
const DATE_EXPANDED: f32 = 10.0;
/// How far above its glance-screen place the time sits, expanded.
const TIME_RISE: f32 = 12.0;

/// The time and date `p` of the way (0 to 1) from the glance screen to the
/// expanded view: smaller, and a little higher.
fn shrink_time(ctx: &mut Context<'_, LockView>, p: f32) {
    let p = p.clamp(0.0, 1.0);
    let parts = ctx.me().parts;
    let step = (p * (TIME_SIZES.len() - 1) as f32).round() as usize;
    let date = if p >= 1.0 { DATE_EXPANDED } else { SMALL };
    for (clock, size) in [(parts.clock, TIME_SIZES[step]), (parts.date, date)] {
        if let Some(text) = ctx.widget::<Clock>(clock.id()).map(Clock::text) {
            ctx.at(text).unwrap().set_size(size);
        }
    }
    ctx.at(parts.time).unwrap().set_margin(Insets::new(
        px(-TIME_RISE * p),
        px(0.0),
        px(0.0),
        px(0.0),
    ));
}

/// A row of dots, one per PIN digit, all hidden until a PIN is asked for.
fn spawn_dots(
    s: &mut Spawner<'_, LockView>,
    input: Handle<Div>,
) -> (Handle<Div>, [Handle<Div>; MAX_DOTS]) {
    let dots = s.spawn(
        input,
        div().style(
            LayoutStyle::default()
                .row()
                .gap(px(16.0))
                .height(px(DOT))
                .shrink(0.0),
        ),
    );
    let dot = std::array::from_fn(|_| {
        s.spawn(
            dots,
            div().style(
                LayoutStyle::default()
                    .size(px(DOT), px(DOT))
                    .shrink(0.0)
                    .hidden(),
            ),
        )
    });
    (dots, dot)
}

/// The bar: the swipe hint on the glance screen, CANCEL and the method
/// switch once expanded.
fn spawn_bar(
    s: &mut Spawner<'_, LockView>,
    body: Handle<Div>,
) -> (Handle<Div>, Handle<Text>, Handle<Button>, Handle<Button>) {
    let bar = s.spawn(body, div());
    let hint = s.spawn(
        bar,
        text("")
            .size(SMALL)
            .color(ColorRole::OnSurfaceVariant)
            .wrap(TextWrap::NoWrap),
    );
    let cancel = s.spawn(bar, bar_button("CANCEL"));
    let switch = s.spawn(bar, bar_button(""));
    s.on::<Clicked>(cancel, |ctx, _| ctx.signal(UserInput::Collapse));
    s.on::<Clicked>(switch, |ctx, _| {
        if let Some(m) = ctx.me().shown.as_ref().and_then(|m| m.switch) {
            ctx.signal(UserInput::Switch(m));
        }
    });
    (bar, hint, cancel, switch)
}

impl Widget for LockView {
    type Builder = LockViewBuilder;

    fn build(b: LockViewBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .size(percent(100.0), percent(100.0));
        *s.component_mut::<Paint>(me).unwrap() =
            Paint::Quad(Quad::new(s.color(ColorRole::SurfaceDim)));
        s.on_theme(me, |ctx| {
            let bg = ctx.color(ColorRole::SurfaceDim);
            ctx.set_paint(Paint::Quad(Quad::new(bg)));
        });

        // The status bar, as on the desktop: the same one, moved in here.
        let status = s.spawn(
            me,
            div().style(LayoutStyle::default().width(percent(100.0)).shrink(0.0)),
        );
        let body = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .width(percent(100.0))
                    .grow(1.0)
                    .min_height(px(0.0)),
            ),
        );
        let (time, clock, date) = spawn_time(s, body);
        let input = s.spawn(body, div());
        // Wraps: faillock's lockout line is long.
        let title = s.spawn(
            input,
            text("")
                .size(SMALL)
                .color(ColorRole::OnSurfaceVariant)
                .wrap(TextWrap::Word)
                .align(TextAlign::Center)
                .style(LayoutStyle::default().max_width(percent(100.0))),
        );
        let (dots, dot) = spawn_dots(s, input);
        let field = field::spawn(s, input);
        let (numpad, cells) = numpad::spawn(s, input);
        let (bar, hint, cancel, switch) = spawn_bar(s, body);
        // The keyboard's keys, moved in here too: as tall as they are, and
        // nothing while the keyboard hides them.
        let keyboard = s.spawn(
            body,
            div().style(LayoutStyle::default().width(percent(100.0)).shrink(0.0)),
        );
        let power_menu = s.spawn(me, PowerMenuBuilder);

        // A swipe up anywhere on the glance screen: the hint follows the
        // finger, then it expands, or unlocks when no method is set; or the
        // hint snaps back.
        s.on::<Press>(body, |ctx, e| {
            if ctx.me().glancing() {
                ctx.me().swipe_room = swipe_room(ctx);
                ctx.me().swipe.press(e.contact, e.position);
            }
        })
        .on::<PressMoved>(body, |ctx, e| {
            let size = ctx.me().size;
            if let Swiped::Follow(up) = ctx.me().swipe.moved(e.contact, e.position, size) {
                ctx.claim_contact(e.contact);
                follow(ctx, up);
                let p = ctx.me().swipe.progress(up);
                shrink_time(ctx, p);
            }
        })
        .on::<Release>(body, |ctx, e| {
            let size = ctx.me().size;
            match ctx.me().swipe.release(e.contact, e.position, size) {
                Swiped::Commit => {
                    follow(ctx, 0.0);
                    shrink_time(ctx, 1.0);
                    let unlock = ctx
                        .me()
                        .shown
                        .as_ref()
                        .is_some_and(|m| m.entry_kind == Entry::Unlock);
                    ctx.signal(if unlock {
                        UserInput::Submit
                    } else {
                        UserInput::Expand
                    });
                }
                Swiped::Rest => {
                    follow(ctx, 0.0);
                    shrink_time(ctx, 0.0);
                }
                Swiped::Follow(_) | Swiped::Ignore => {}
            }
        })
        .on::<PressCancelled>(body, |ctx, _| {
            ctx.me().swipe.cancel();
            follow(ctx, 0.0);
            shrink_time(ctx, 0.0);
        });

        // A hardware keyboard: the field has the keyboard focus while it
        // shows; otherwise the window has it, and keys are PIN input.
        s.on::<KeyPress>(b.window, |ctx, e| key(ctx, &e.meaning));
        s.on::<KeyRepeat>(b.window, |ctx, e| key(ctx, &e.meaning));
        s.on::<Resized>(b.window, |ctx, e| {
            ctx.me().size = e.size;
            layout(ctx);
        });
        s.on::<Show>(me, |ctx, e| apply(ctx, &e.0));

        LockView {
            window: b.window,
            parts: Parts {
                body,
                time,
                clock,
                date,
                input,
                title,
                dots,
                dot,
                field,
                numpad,
                status,
                keyboard,
                bar,
                hint,
                cancel,
                switch,
            },
            power_menu,
            cells,
            shown: None,
            revealed: false,
            swipe: SwipeUp::default(),
            swipe_room: 0.0,
            size: Size::ZERO,
        }
    }
}

/// The swipe hint, `up` px above where it rests, but never into the time.
/// The time stays: it shows expanded too.
fn follow(ctx: &mut Context<'_, LockView>, up: f32) {
    let (bar, room) = (ctx.me().parts.bar, ctx.me().swipe_room);
    ctx.at(bar)
        .unwrap()
        .set_margin(Insets::new(px(0.0), px(0.0), px(up.min(room)), px(0.0)));
}

/// How far the hint has to rise, from where it rests, before it meets the
/// time: both share a parent, so their rects compare.
fn swipe_room(ctx: &mut Context<'_, LockView>) -> f32 {
    let p = ctx.me().parts;
    let mut rect = |node: Handle<Div>| {
        ctx.at(node)
            .and_then(|c| c.component::<ComputedLayout>().map(|l| l.rect))
    };
    match (rect(p.time), rect(p.bar)) {
        (Some(time), Some(bar)) => (bar.y() - time.bottom() - HINT_GAP).max(0.0),
        _ => 0.0,
    }
}

/// The least the hint keeps from the time.
const HINT_GAP: f32 = 16.0;

/// The date line.
const DATE: &str = "%a %d";

/// A key the field doesn't take: PIN input, or the first of a password,
/// typed on the glance screen, which goes into the field as it opens.
fn key(ctx: &mut Context<'_, LockView>, meaning: &KeyMeaning) {
    let Some((kind, busy)) = ctx.me().shown.as_ref().map(|m| (m.entry_kind, m.busy)) else {
        return;
    };
    let input = match meaning {
        KeyMeaning::Enter => UserInput::Submit,
        KeyMeaning::Text(t) if t.chars().all(|c| !c.is_control()) => {
            if kind == Entry::Password && !busy {
                let field = ctx.me().parts.field.input;
                let meaning = meaning.clone();
                let modifiers = Modifiers::default();
                ctx.emit(KeyPress { meaning, modifiers }, field.id());
            }
            UserInput::Text(t.clone())
        }
        KeyMeaning::Backspace => UserInput::Backspace,
        _ => return,
    };
    ctx.signal(input);
}

fn display(shown: bool) -> Display {
    if shown {
        Display::Flex
    } else {
        Display::Hidden
    }
}

/// Keyboard focus: the field while it takes typing, else the window, whose
/// keys are PIN input or open the field.
fn focus(ctx: &mut Context<'_, LockView>) {
    let (window, field, typing) = {
        let v = ctx.me();
        (v.window, v.parts.field.input, v.typing())
    };
    ctx.emit(InputFocus { focused: typing }, field.id());
    let target = if typing { field.id() } else { window };
    ctx.resource_mut::<KeyboardFocus>().focus(target);
}

fn apply(ctx: &mut Context<'_, LockView>, m: &ViewModel) {
    let was_shown = ctx.me().field_shown();
    let old = ctx.me().shown.replace(m.clone());
    let p = ctx.me().parts;

    // The title is what PAM last said, or else what is asked.
    let (line, color) = match &m.note {
        Some(n) if !n.text.is_empty() => (
            n.text.to_uppercase(),
            if n.error {
                ColorRole::Error
            } else {
                ColorRole::Primary
            },
        ),
        _ => (
            m.prompt.as_deref().unwrap_or_default().to_uppercase(),
            ColorRole::OnSurfaceVariant,
        ),
    };
    {
        let mut t = ctx.at(p.title).unwrap();
        t.set_text(line);
        t.set_color(color);
    }

    // A dot per digit the PIN has, filled as typed.
    let shown = m.pin_length.min(MAX_DOTS);
    let filled = ctx.color(ColorRole::OnSurface);
    let ring = ctx.color(ColorRole::OnSurfaceVariant);
    for (i, d) in p.dot.into_iter().enumerate() {
        let paint = if i < m.typed {
            Paint::Quad(Quad::new(filled).radius(DOT / 2.0))
        } else {
            Paint::Quad(
                Quad::new(Color::TRANSPARENT)
                    .radius(DOT / 2.0)
                    .border(1.5, ring),
            )
        };
        let mut c = ctx.at(d).unwrap();
        c.set_paint(paint);
        c.set_display(display(i < shown));
    }
    ctx.at(p.dots).unwrap().set_display(display(m.dots()));

    // The field: emptied and masked again as it goes; emptied too once
    // its answer is checked, having shown it, disabled, meanwhile.
    let gone = was_shown && !ctx.me().field_shown();
    let checked = old.as_ref().is_some_and(|o| o.busy) && !m.busy;
    if gone {
        ctx.me().revealed = false;
    }
    if gone || checked {
        InputContext::set_text(&mut ctx.at(p.field.input).unwrap(), "");
    }
    field::sync(ctx);

    {
        let unlock = m.entry_kind == Entry::Unlock;
        let glancing = ctx.me().glancing();
        let mut h = ctx.at(p.hint).unwrap();
        h.set_display(display(glancing));
        h.set_text(if unlock {
            "SWIPE UP TO UNLOCK"
        } else {
            "SWIPE UP TO EXPAND"
        });
    }
    ctx.at(p.cancel)
        .unwrap()
        .set_display(display(m.expanded && m.entry_kind != Entry::Unlock));
    {
        let mut sw = ctx.at(p.switch).unwrap();
        sw.set_display(display(m.expanded && m.switch.is_some()));
        match m.switch {
            Some(Method::Pin) => sw.set_label("USE PIN"),
            Some(Method::Password) => sw.set_label("USE PASSWORD"),
            None => {}
        }
    }
    let shape = |m: &ViewModel| (m.expanded, m.entry_kind, m.hidden, m.busy, m.keyboard);
    if old.as_ref().map(shape) != Some(shape(m)) {
        layout(ctx);
        focus(ctx);
    }
}
