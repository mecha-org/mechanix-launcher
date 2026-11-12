//! Routes touches and ticks to every container's triggers and motions, and
//! holds each container's open/closed state.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gestures::{AxisLock, VelocityTracker};
use screen::Screen;

use super::AreaId;
use super::config::NavConfig;
use super::container::{ContainerId, ContainerSpec, Entry};
use super::gesture::{Detect, Gesture, Phase};
use super::input::Input;
use super::motion::{Final, INSTANT, Motion, Visual};
use super::trigger::{ExternalEvent, Trigger};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ContainerState {
    /// How far open it is right now, 0 to 1.
    pub presence: f32,
    /// Where it is heading.
    pub target: Final,
}

impl ContainerState {
    pub fn visible(&self) -> bool {
        self.presence > 0.0 || self.target == Final::Open
    }
}

struct Node {
    id: ContainerId,
    parent: Option<usize>,
    children: Vec<usize>,
    entries: Vec<Entry>,
    state: ContainerState,
    /// The motion that last drove this container.
    driver: Rc<dyn Motion>,
    /// px/ms, screen-down positive: the velocity of the gesture that last
    /// drove this container as it let go, passed to every settle. Zero
    /// when something other than a gesture drove it.
    velocity: (f32, f32),
}

/// An entry of a node that the live gesture may still fire.
#[derive(Clone, Copy)]
struct Candidate {
    node: usize,
    entry: usize,
}

struct Live {
    contact: u32,
    start_t: Instant,
    gesture: Gesture,
    velocity: VelocityTracker,
    axis: AxisLock,
    candidates: Vec<Candidate>,
    /// Actions on the gesture's area that may still fire.
    actions: Vec<usize>,
    /// Nodes whose presence a `follow` has changed during this gesture.
    moved: Vec<usize>,
}

/// A container's `(id, parent, screen)`, as returned by [`Stack::new`].
pub type ScreenRow = (ContainerId, Option<ContainerId>, Box<dyn Screen>);

pub struct Stack {
    cfg: NavConfig,
    /// In z order: depth-first, declaration order, a parent before its children.
    nodes: Vec<Node>,
    /// Triggers that fire an action rather than move a container.
    actions: Vec<Box<dyn Trigger>>,
    /// Actions committed since the last [`Stack::take_fired`].
    fired: Vec<usize>,
    live: Option<Live>,
}

impl Stack {
    /// Returns the stack and, in z order, each container's `(id, parent, screen)`.
    pub fn new(cfg: NavConfig, roots: Vec<ContainerSpec>) -> (Stack, Vec<ScreenRow>) {
        let mut stack = Stack {
            cfg,
            nodes: Vec::new(),
            actions: Vec::new(),
            fired: Vec::new(),
            live: None,
        };
        let mut rows = Vec::new();
        for root in roots {
            stack.add(root, None, &mut rows);
        }
        (stack, rows)
    }

    /// Adds a trigger that moves no container: when a gesture commits it,
    /// its index is reported by [`Stack::take_fired`]. Returns that index.
    pub fn action(&mut self, trigger: Box<dyn Trigger>) -> usize {
        self.actions.push(trigger);
        self.actions.len() - 1
    }

    /// The actions committed since the last call, in commit order.
    pub fn take_fired(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.fired)
    }

    fn add(&mut self, spec: ContainerSpec, parent: Option<usize>, rows: &mut Vec<ScreenRow>) {
        assert!(
            self.nodes.iter().all(|n| n.id != spec.id),
            "duplicate container id {:?}",
            spec.id
        );
        let index = self.nodes.len();
        let target = if spec.initially_open {
            Final::Open
        } else {
            Final::Closed
        };
        self.nodes.push(Node {
            id: spec.id,
            parent,
            children: Vec::new(),
            entries: spec.entries,
            state: ContainerState {
                presence: target.value(),
                target,
            },
            driver: Rc::new(INSTANT),
            velocity: (0.0, 0.0),
        });
        if let Some(p) = parent {
            self.nodes[p].children.push(index);
        }
        rows.push((spec.id, parent.map(|p| self.nodes[p].id), spec.screen));
        for child in spec.children {
            self.add(child, Some(index), rows);
        }
    }

    pub fn handle(&mut self, input: Input) {
        match input {
            Input::Down {
                contact,
                area,
                area_size,
                pos,
                t,
            } => {
                if self.live.is_some() {
                    return;
                }
                let candidates = self.candidates(area);
                let actions = (0..self.actions.len())
                    .filter(|&a| self.actions[a].area() == area)
                    .collect();
                let mut velocity = VelocityTracker::new(
                    self.cfg.velocity_window,
                    self.cfg.velocity_stale,
                    self.cfg.velocity_min_span,
                );
                velocity.push(t, pos.x, pos.y);
                self.live = Some(Live {
                    contact,
                    start_t: t,
                    gesture: Gesture {
                        area,
                        area_size,
                        start: pos,
                        pos,
                        elapsed: Duration::ZERO,
                        velocity: (0.0, 0.0),
                        axis: None,
                    },
                    velocity,
                    axis: AxisLock::new(self.cfg.axis_lock_px),
                    candidates,
                    actions,
                    moved: Vec::new(),
                });
            }
            Input::Move { contact, pos, t } => {
                let Some(live) = self.live.as_mut().filter(|l| l.contact == contact) else {
                    return;
                };
                live.velocity.push(t, pos.x, pos.y);
                let g = &mut live.gesture;
                g.pos = pos;
                g.elapsed = t.saturating_duration_since(live.start_t);
                g.velocity = live.velocity.velocity(t);
                let (dx, dy) = g.delta();
                g.axis = live.axis.update(dx, dy);
                let nodes = &mut self.nodes;
                let cfg = &self.cfg;
                let g = &live.gesture;
                let triggers = &self.actions;
                live.actions
                    .retain(|&a| triggers[a].detect(g, Phase::Moving, cfg) != Detect::Ignore);
                let moved = &mut live.moved;
                live.candidates.retain(|c| {
                    let node = &mut nodes[c.node];
                    let entry = &node.entries[c.entry];
                    match entry.trigger.detect(g, Phase::Moving, cfg) {
                        Detect::Progress(p) => {
                            let presence = entry.motion.follow(
                                node.state.presence,
                                entry.toward,
                                p,
                                g.velocity,
                            );
                            // The motion that moves a container drives it
                            // from here on, so it draws it and settles it.
                            if presence != node.state.presence {
                                node.state.presence = presence;
                                node.driver = entry.motion.clone();
                                if !moved.contains(&c.node) {
                                    moved.push(c.node);
                                }
                            }
                            true
                        }
                        Detect::Ignore => {
                            if moved.contains(&c.node) {
                                node.velocity = g.velocity;
                            }
                            false
                        }
                        Detect::Commit | Detect::Cancel => true,
                    }
                });
            }
            Input::Up { contact, t } => {
                if self.live.as_ref().is_none_or(|l| l.contact != contact) {
                    return;
                }
                let mut live = self.live.take().unwrap();
                let g = &mut live.gesture;
                g.elapsed = t.saturating_duration_since(live.start_t);
                g.velocity = live.velocity.velocity(t);
                let velocity = g.velocity;
                let mut driven = Vec::new();
                for c in live.candidates {
                    let entry = &self.nodes[c.node].entries[c.entry];
                    match entry
                        .trigger
                        .detect(&live.gesture, Phase::Released, &self.cfg)
                    {
                        // An earlier commit of this gesture may have closed
                        // the parent: then the child stays closed.
                        Detect::Commit if self.parent_allows(c) => {
                            self.commit(c, velocity, &mut driven)
                        }
                        Detect::Commit | Detect::Cancel => {
                            self.drive(c, velocity);
                            driven.push(c.node);
                        }
                        Detect::Progress(_) | Detect::Ignore => {}
                    }
                }
                self.settle(&driven, Duration::ZERO);
                for a in live.actions {
                    if self.actions[a].detect(&live.gesture, Phase::Released, &self.cfg)
                        == Detect::Commit
                    {
                        self.fired.push(a);
                    }
                }
            }
            Input::Cancel => {
                let Some(live) = self.live.take() else { return };
                let mut driven = Vec::new();
                for c in live.candidates {
                    let state = self.nodes[c.node].state;
                    if state.presence != state.target.value() {
                        self.drive(c, live.gesture.velocity);
                        driven.push(c.node);
                    }
                }
                self.settle(&driven, Duration::ZERO);
            }
            Input::Tick { dt } => {
                // A container the finger has moved, and may still move,
                // stays where the finger put it.
                let held: Vec<usize> = self
                    .live
                    .iter()
                    .flat_map(|l| {
                        l.candidates
                            .iter()
                            .map(|c| c.node)
                            .filter(|n| l.moved.contains(n))
                    })
                    .collect();
                let moving: Vec<usize> = (0..self.nodes.len())
                    .filter(|i| self.is_animating(*i) && !held.contains(i))
                    .collect();
                self.settle(&moving, dt);
            }
            Input::External(e) => self.external(&e),
        }
    }

    fn external(&mut self, e: &ExternalEvent) {
        let mut driven = Vec::new();
        for node in 0..self.nodes.len() {
            for entry in 0..self.nodes[node].entries.len() {
                if self.nodes[node].entries[entry].trigger.on_external(e) {
                    self.commit(Candidate { node, entry }, (0.0, 0.0), &mut driven);
                }
            }
        }
        self.settle(&driven, Duration::ZERO);
    }

    /// The entries on `area` that may fire from the current state.
    fn candidates(&self, area: AreaId) -> Vec<Candidate> {
        let mut out = Vec::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let target = node.state.target;
            let can_open = target == Final::Closed
                && node.parent.is_none_or(|p| {
                    let parent = &self.nodes[p];
                    parent.state.target == Final::Open
                        && parent
                            .children
                            .iter()
                            .all(|s| *s == i || self.nodes[*s].state.target != Final::Open)
                });
            for (e, entry) in node.entries.iter().enumerate() {
                if entry.trigger.area() != area {
                    continue;
                }
                let eligible = match entry.toward {
                    Final::Open => can_open,
                    Final::Closed => target == Final::Open,
                };
                if eligible {
                    out.push(Candidate { node: i, entry: e });
                }
            }
        }
        out
    }

    /// Whether `c`'s parent, as it is now, lets it commit: an open needs
    /// the parent heading open.
    fn parent_allows(&self, c: Candidate) -> bool {
        let node = &self.nodes[c.node];
        node.entries[c.entry].toward == Final::Closed
            || node
                .parent
                .is_none_or(|p| self.nodes[p].state.target == Final::Open)
    }

    /// Hands the container to the entry's motion, with the velocity it
    /// settles from.
    fn drive(&mut self, c: Candidate, velocity: (f32, f32)) {
        let node = &mut self.nodes[c.node];
        node.driver = node.entries[c.entry].motion.clone();
        node.velocity = velocity;
    }

    /// Sets the target from the entry, records its motion and, on close,
    /// closes every descendant at once.
    fn commit(&mut self, c: Candidate, velocity: (f32, f32), driven: &mut Vec<usize>) {
        self.drive(c, velocity);
        driven.push(c.node);
        let toward = self.nodes[c.node].entries[c.entry].toward;
        self.nodes[c.node].state.target = toward;
        if toward == Final::Closed {
            let mut stack = self.nodes[c.node].children.clone();
            while let Some(d) = stack.pop() {
                let node = &mut self.nodes[d];
                node.state = ContainerState {
                    presence: 0.0,
                    target: Final::Closed,
                };
                node.velocity = (0.0, 0.0);
                stack.extend(node.children.iter().copied());
            }
        }
    }

    fn settle(&mut self, nodes: &[usize], dt: Duration) {
        for &i in nodes {
            let node = &mut self.nodes[i];
            node.state.presence = node.driver.settle(
                node.state.presence,
                node.state.target,
                dt,
                node.velocity,
                &self.cfg,
            );
        }
    }

    fn index(&self, id: ContainerId) -> usize {
        self.nodes
            .iter()
            .position(|n| n.id == id)
            .unwrap_or_else(|| panic!("unknown container {id:?}"))
    }

    fn is_animating(&self, i: usize) -> bool {
        let s = self.nodes[i].state;
        s.presence != s.target.value()
    }

    pub fn state(&self, id: ContainerId) -> ContainerState {
        self.nodes[self.index(id)].state
    }

    /// How the container is drawn, from the motion that last drove it.
    pub fn visual(&self, id: ContainerId) -> Visual {
        let node = &self.nodes[self.index(id)];
        node.driver.visual(node.state.presence)
    }

    /// The containers whose presence has not reached their target.
    pub fn animating(&self) -> Vec<ContainerId> {
        (0..self.nodes.len())
            .filter(|i| self.is_animating(*i))
            .map(|i| self.nodes[i].id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use mecha_wayland::prelude::{App, NodeId, Point, Size};
    use screen::Screen;

    use super::*;
    use crate::core::{
        AreaId, BOTTOM_EDGE, Direction, FADE, Final, INSTANT, Motion, NavConfig, OnExternal, Span,
        TOP_EDGE, Timing, container, gated, swipe,
    };

    const HOME: AreaId = AreaId("home");
    const PANEL: AreaId = AreaId("panel");
    const CLOSER: AreaId = AreaId("closer");

    const HOME_ID: ContainerId = ContainerId("home");
    const DRAWER: ContainerId = ContainerId("drawer");
    const SEARCH: ContainerId = ContainerId("search");
    const RUNNING: ContainerId = ContainerId("running");
    const NOTIFICATIONS: ContainerId = ContainerId("notifications");
    const QUICK_SETTINGS: ContainerId = ContainerId("quick_settings");

    const ALL: [ContainerId; 6] = [
        HOME_ID,
        DRAWER,
        SEARCH,
        RUNNING,
        NOTIFICATIONS,
        QUICK_SETTINGS,
    ];

    struct Dummy;
    impl Screen for Dummy {
        fn build(&self, _app: &mut App, parent: NodeId) -> NodeId {
            parent
        }
    }

    fn bottom_quick() -> crate::core::Swipe {
        swipe(BOTTOM_EDGE, Direction::Up, Timing::Quick)
    }

    fn drawer(m: impl Motion + Clone) -> ContainerSpec {
        container(DRAWER, Dummy)
            .open_close(swipe(HOME, Direction::Up, Timing::Any), m)
            .close(bottom_quick(), INSTANT)
    }

    fn home(initially_open: bool, drawer: ContainerSpec) -> ContainerSpec {
        let home = container(HOME_ID, Dummy)
            .open(bottom_quick(), FADE)
            .child(drawer)
            .child(
                container(SEARCH, Dummy)
                    .open_close(swipe(HOME, Direction::Right, Timing::Any), INSTANT)
                    .close(bottom_quick(), INSTANT),
            );
        if initially_open {
            home.initially_open()
        } else {
            home
        }
    }

    fn rest(home: ContainerSpec) -> Vec<ContainerSpec> {
        let panel = |id, span| {
            container(id, Dummy)
                .open(
                    swipe(TOP_EDGE, Direction::Down, Timing::Any).span(span),
                    INSTANT,
                )
                .close(swipe(PANEL, Direction::Up, Timing::Any), INSTANT)
        };
        vec![
            home,
            container(RUNNING, Dummy)
                .open(swipe(BOTTOM_EDGE, Direction::Up, Timing::Held), INSTANT)
                .close(bottom_quick(), INSTANT),
            panel(NOTIFICATIONS, Span::LeftHalf),
            panel(QUICK_SETTINGS, Span::RightHalf),
        ]
    }

    fn build(roots: Vec<ContainerSpec>) -> Stack {
        Stack::new(NavConfig::default(), roots).0
    }

    fn tree() -> Stack {
        build(rest(home(true, drawer(INSTANT))))
    }

    fn tree_home_closed() -> Stack {
        build(rest(home(false, drawer(INSTANT))))
    }

    fn size_of(area: AreaId) -> Size {
        if area == BOTTOM_EDGE || area == TOP_EDGE {
            Size {
                width: 600.0,
                height: 20.0,
            }
        } else {
            Size {
                width: 600.0,
                height: 1000.0,
            }
        }
    }

    fn pt(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    const STEP: Duration = Duration::from_millis(8);
    /// Long enough to fill the velocity window, so the release velocity is exactly the tail's.
    const TAIL: Duration = Duration::from_millis(80);

    /// Moves along a line from `from`, then ends with `TAIL` of motion at
    /// `end_velocity` (px/ms) that lands on `to`, at `duration` (> `TAIL`).
    fn moves(
        from: Point,
        to: Point,
        duration: Duration,
        end_velocity: (f32, f32),
    ) -> Vec<(Duration, Point)> {
        let tail_ms = TAIL.as_secs_f32() * 1000.0;
        let tail_start = pt(
            to.x - end_velocity.0 * tail_ms,
            to.y - end_velocity.1 * tail_ms,
        );
        let body = duration - TAIL;
        let mut out = Vec::new();
        let mut t = STEP;
        while t <= duration {
            let p = if t <= body {
                let f = t.as_secs_f32() / body.as_secs_f32();
                pt(
                    from.x + (tail_start.x - from.x) * f,
                    from.y + (tail_start.y - from.y) * f,
                )
            } else {
                let f = (t - body).as_secs_f32() / TAIL.as_secs_f32();
                pt(
                    tail_start.x + (to.x - tail_start.x) * f,
                    tail_start.y + (to.y - tail_start.y) * f,
                )
            };
            out.push((t, p));
            t += STEP;
        }
        out
    }

    fn down(stack: &mut Stack, contact: u32, area: AreaId, pos: Point, t: Instant) {
        stack.handle(Input::Down {
            contact,
            area,
            area_size: size_of(area),
            pos,
            t,
        });
    }

    fn swipe_on(
        stack: &mut Stack,
        area: AreaId,
        from: Point,
        to: Point,
        duration: Duration,
        end_velocity: (f32, f32),
    ) {
        let t0 = Instant::now();
        down(stack, 1, area, from, t0);
        for (dt, pos) in moves(from, to, duration, end_velocity) {
            stack.handle(Input::Move {
                contact: 1,
                pos,
                t: t0 + dt,
            });
        }
        stack.handle(Input::Up {
            contact: 1,
            t: t0 + duration,
        });
    }

    fn quick_bottom(stack: &mut Stack) {
        swipe_on(
            stack,
            BOTTOM_EDGE,
            pt(300.0, 10.0),
            pt(300.0, -110.0),
            Duration::from_millis(150),
            (0.0, -1.5),
        );
    }

    fn held_bottom(stack: &mut Stack) {
        swipe_on(
            stack,
            BOTTOM_EDGE,
            pt(300.0, 10.0),
            pt(300.0, -110.0),
            Duration::from_millis(500),
            (0.0, 0.0),
        );
    }

    fn home_up(stack: &mut Stack) {
        swipe_on(
            stack,
            HOME,
            pt(300.0, 800.0),
            pt(300.0, 600.0),
            Duration::from_millis(200),
            (0.0, -1.0),
        );
    }

    fn home_down(stack: &mut Stack) {
        swipe_on(
            stack,
            HOME,
            pt(300.0, 600.0),
            pt(300.0, 800.0),
            Duration::from_millis(200),
            (0.0, 1.0),
        );
    }

    fn home_right(stack: &mut Stack) {
        swipe_on(
            stack,
            HOME,
            pt(100.0, 500.0),
            pt(300.0, 500.0),
            Duration::from_millis(200),
            (1.0, 0.0),
        );
    }

    fn top_down(stack: &mut Stack, x: f32) {
        swipe_on(
            stack,
            TOP_EDGE,
            pt(x, 0.0),
            pt(x, 200.0),
            Duration::from_millis(200),
            (0.0, 1.0),
        );
    }

    fn panel_up(stack: &mut Stack) {
        swipe_on(
            stack,
            PANEL,
            pt(300.0, 600.0),
            pt(300.0, 400.0),
            Duration::from_millis(200),
            (0.0, -1.0),
        );
    }

    fn target(stack: &Stack, id: ContainerId) -> Final {
        stack.state(id).target
    }

    fn open_ones(stack: &Stack) -> Vec<ContainerId> {
        ALL.into_iter()
            .filter(|id| target(stack, *id) == Final::Open)
            .collect()
    }

    fn settled(stack: &Stack) -> bool {
        ALL.iter().all(|id| {
            let s = stack.state(*id);
            s.presence == s.target.value()
        })
    }

    #[test]
    fn home_starts_open_others_closed() {
        let (stack, rows) = Stack::new(NavConfig::default(), rest(home(true, drawer(INSTANT))));
        assert_eq!(open_ones(&stack), [HOME_ID]);
        assert_eq!(stack.state(HOME_ID).presence, 1.0);
        assert!(stack.state(HOME_ID).visible());
        assert!(!stack.state(DRAWER).visible());
        assert!(stack.animating().is_empty());
        let order: Vec<_> = rows.iter().map(|(id, parent, _)| (*id, *parent)).collect();
        assert_eq!(
            order,
            [
                (HOME_ID, None),
                (DRAWER, Some(HOME_ID)),
                (SEARCH, Some(HOME_ID)),
                (RUNNING, None),
                (NOTIFICATIONS, None),
                (QUICK_SETTINGS, None),
            ]
        );
    }

    #[test]
    fn quick_bottom_from_app_opens_home_with_fade() {
        let mut stack = tree_home_closed();
        assert!(open_ones(&stack).is_empty());
        quick_bottom(&mut stack);
        assert_eq!(target(&stack, HOME_ID), Final::Open);
        assert_eq!(stack.state(HOME_ID).presence, 0.0);
        assert_eq!(stack.visual(HOME_ID).cover, 1.0);
        assert_eq!(stack.animating(), [HOME_ID]);
        assert_eq!(open_ones(&stack), [HOME_ID]);

        stack.handle(Input::Tick {
            dt: Duration::from_millis(80),
        });
        assert_eq!(stack.visual(HOME_ID).cover, 0.5);
        stack.handle(Input::Tick {
            dt: Duration::from_millis(80),
        });
        assert_eq!(stack.state(HOME_ID).presence, 1.0);
        assert!(stack.animating().is_empty());
    }

    #[test]
    fn held_bottom_opens_running_apps() {
        let mut stack = tree();
        held_bottom(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID, RUNNING]);
        assert!(settled(&stack));

        let mut stack = tree_home_closed();
        held_bottom(&mut stack);
        assert_eq!(open_ones(&stack), [RUNNING]);
        assert!(settled(&stack));
    }

    #[test]
    fn quick_bottom_from_running_closes_running_and_keeps_home() {
        let mut stack = tree();
        held_bottom(&mut stack);
        quick_bottom(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID]);
        assert!(!stack.animating().contains(&HOME_ID));
        assert!(settled(&stack));
    }

    #[test]
    fn quick_bottom_from_drawer_returns_to_plain_home() {
        let mut stack = tree();
        home_up(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID, DRAWER]);
        quick_bottom(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID]);
        assert!(settled(&stack));
    }

    #[test]
    fn home_swipes_open_children_only_while_home_open() {
        let mut stack = tree_home_closed();
        home_up(&mut stack);
        home_right(&mut stack);
        assert!(open_ones(&stack).is_empty());

        let mut stack = tree();
        home_right(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID, SEARCH]);
    }

    #[test]
    fn sibling_rule() {
        let mut stack = tree();
        home_up(&mut stack);
        home_right(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID, DRAWER]);
    }

    #[test]
    fn top_left_opens_notifications_top_right_opens_quick_settings() {
        let mut stack = tree();
        top_down(&mut stack, 100.0);
        assert_eq!(open_ones(&stack), [HOME_ID, NOTIFICATIONS]);

        let mut stack = tree();
        top_down(&mut stack, 500.0);
        assert_eq!(open_ones(&stack), [HOME_ID, QUICK_SETTINGS]);
    }

    #[test]
    fn panel_swipe_up_closes_both() {
        let mut stack = tree();
        top_down(&mut stack, 100.0);
        top_down(&mut stack, 500.0);
        assert_eq!(open_ones(&stack), [HOME_ID, NOTIFICATIONS, QUICK_SETTINGS]);
        panel_up(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID]);
        assert!(settled(&stack));
    }

    #[test]
    fn closing_home_closes_children() {
        let home =
            home(true, drawer(INSTANT)).close(swipe(CLOSER, Direction::Down, Timing::Any), INSTANT);
        let mut stack = build(rest(home));
        home_up(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID, DRAWER]);
        swipe_on(
            &mut stack,
            CLOSER,
            pt(300.0, 100.0),
            pt(300.0, 300.0),
            Duration::from_millis(200),
            (0.0, 1.0),
        );
        assert!(open_ones(&stack).is_empty());
        assert_eq!(stack.state(DRAWER).presence, 0.0);
        assert!(settled(&stack));
    }

    #[test]
    fn a_gated_close_keeps_the_drawer_open() {
        let gate = Rc::new(Cell::new(false));
        let drawer = container(DRAWER, Dummy)
            .open(swipe(HOME, Direction::Up, Timing::Any), INSTANT)
            .close(
                gated(swipe(HOME, Direction::Down, Timing::Any), gate.clone()),
                INSTANT,
            );
        let mut stack = build(rest(home(true, drawer)));
        home_up(&mut stack);
        assert_eq!(target(&stack, DRAWER), Final::Open);
        home_down(&mut stack);
        assert_eq!(target(&stack, DRAWER), Final::Open);
        gate.set(true);
        home_down(&mut stack);
        assert_eq!(target(&stack, DRAWER), Final::Closed);
        assert!(settled(&stack));
    }

    #[test]
    fn app_launched_closes_home_and_children() {
        let home =
            home(true, drawer(INSTANT)).close(OnExternal(ExternalEvent::AppLaunched), INSTANT);
        let mut stack = build(rest(home));
        home_up(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID, DRAWER]);
        stack.handle(Input::External(ExternalEvent::AppLaunched));
        assert_eq!(target(&stack, HOME_ID), Final::Closed);
        assert_eq!(target(&stack, DRAWER), Final::Closed);
        assert!(open_ones(&stack).is_empty());
        assert!(settled(&stack));
    }

    #[test]
    fn last_window_closed_opens_home() {
        let home = home(false, drawer(INSTANT))
            .close(OnExternal(ExternalEvent::AppLaunched), INSTANT)
            .open(OnExternal(ExternalEvent::LastWindowClosed), INSTANT);
        let mut stack = build(rest(home));
        assert!(open_ones(&stack).is_empty());
        stack.handle(Input::External(ExternalEvent::LastWindowClosed));
        assert_eq!(open_ones(&stack), [HOME_ID]);
        assert!(settled(&stack));
        // Already open: it stays so.
        stack.handle(Input::External(ExternalEvent::LastWindowClosed));
        assert_eq!(open_ones(&stack), [HOME_ID]);
    }

    #[test]
    fn last_window_closed_from_recents_lands_on_home() {
        let home =
            home(false, drawer(INSTANT)).open(OnExternal(ExternalEvent::LastWindowClosed), INSTANT);
        let mut roots = rest(home);
        roots[1] = container(RUNNING, Dummy)
            .initially_open()
            .close(OnExternal(ExternalEvent::LastWindowClosed), INSTANT);
        let mut stack = build(roots);
        assert_eq!(open_ones(&stack), [RUNNING]);
        stack.handle(Input::External(ExternalEvent::LastWindowClosed));
        assert_eq!(open_ones(&stack), [HOME_ID]);
        assert!(settled(&stack));
    }

    #[test]
    fn a_bar_makes_way_for_fullscreen_and_comes_back() {
        const BAR: ContainerId = ContainerId("bar");
        let mut roots = rest(home(true, drawer(INSTANT)));
        roots.push(
            container(BAR, Dummy)
                .initially_open()
                .close(OnExternal(ExternalEvent::FullscreenEntered), INSTANT)
                .open(OnExternal(ExternalEvent::FullscreenLeft), INSTANT),
        );
        let mut stack = build(roots);
        assert!(stack.state(BAR).visible());
        stack.handle(Input::External(ExternalEvent::FullscreenEntered));
        assert!(!stack.state(BAR).visible());
        assert!(stack.state(HOME_ID).visible(), "nothing else moves");
        stack.handle(Input::External(ExternalEvent::FullscreenLeft));
        assert!(stack.state(BAR).visible());
        assert!(settled(&stack));
    }

    /// Records every `follow` progress; otherwise behaves like `INSTANT`.
    #[derive(Clone)]
    struct Recorder(Rc<RefCell<Vec<f32>>>);

    impl Motion for Recorder {
        fn follow(&self, current: f32, _toward: Final, progress: f32, _v: (f32, f32)) -> f32 {
            self.0.borrow_mut().push(progress);
            current
        }
        fn settle(
            &self,
            _current: f32,
            target: Final,
            _dt: Duration,
            _velocity: (f32, f32),
            _cfg: &NavConfig,
        ) -> f32 {
            target.value()
        }
    }

    #[test]
    fn progress_reaches_follow_every_move() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut stack = build(rest(home(true, drawer(Recorder(log.clone())))));
        let t0 = Instant::now();
        down(&mut stack, 1, HOME, pt(300.0, 800.0), t0);
        for (i, y) in [770.0, 740.0, 710.0].into_iter().enumerate() {
            stack.handle(Input::Move {
                contact: 1,
                pos: pt(300.0, y),
                t: t0 + STEP * (i as u32 + 1),
            });
        }
        let got = log.borrow().clone();
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(got.windows(2).all(|w| w[0] < w[1]), "{got:?}");
        assert!(got.iter().all(|p| (0.0..=1.0).contains(p)), "{got:?}");
        assert_eq!(got[2], 1.0);
    }

    /// Follows the finger (`presence = progress`), settles like `FADE`, and
    /// draws a cover no other motion draws, so its use is visible.
    #[derive(Clone)]
    struct Track;
    const TRACK_COVER: f32 = 0.25;

    impl Motion for Track {
        fn follow(&self, _current: f32, _toward: Final, progress: f32, _v: (f32, f32)) -> f32 {
            progress
        }
        fn settle(
            &self,
            current: f32,
            target: Final,
            dt: Duration,
            velocity: (f32, f32),
            cfg: &NavConfig,
        ) -> f32 {
            FADE.settle(current, target, dt, velocity, cfg)
        }
        fn visual(&self, _presence: f32) -> Visual {
            Visual { cover: TRACK_COVER }
        }
    }

    fn mv(stack: &mut Stack, t0: Instant, n: u32, x: f32, y: f32) {
        stack.handle(Input::Move {
            contact: 1,
            pos: pt(x, y),
            t: t0 + STEP * n,
        });
    }

    #[test]
    fn mid_drag_visual_comes_from_the_following_motion() {
        let mut stack = build(rest(home(true, drawer(Track))));
        let t0 = Instant::now();
        down(&mut stack, 1, HOME, pt(300.0, 800.0), t0);
        mv(&mut stack, t0, 1, 300.0, 755.0);
        assert_eq!(stack.state(DRAWER).presence, 0.5);
        assert_eq!(stack.visual(DRAWER).cover, TRACK_COVER);
    }

    #[test]
    fn a_container_left_by_the_axis_lock_settles_back_with_its_motion() {
        let mut stack = build(rest(home(true, drawer(Track))));
        let t0 = Instant::now();
        down(&mut stack, 1, HOME, pt(300.0, 800.0), t0);
        // Under the axis lock: the drawer follows.
        mv(&mut stack, t0, 1, 300.0, 791.0);
        assert_eq!(stack.state(DRAWER).presence, 0.1);
        // Locks horizontal: the drawer's swipe is ignored from here on.
        mv(&mut stack, t0, 2, 330.0, 791.0);
        assert_eq!(stack.state(DRAWER).presence, 0.1);
        stack.handle(Input::Tick { dt: STEP });
        let p = stack.state(DRAWER).presence;
        assert!((p - 0.05).abs() < 1e-6, "{p}");
        stack.handle(Input::Tick { dt: STEP });
        assert_eq!(stack.state(DRAWER).presence, 0.0);
        assert_eq!(target(&stack, DRAWER), Final::Closed);
    }

    #[test]
    fn cancel_after_movement_settles_back_to_the_target() {
        let mut stack = build(rest(home(true, drawer(Track))));
        let t0 = Instant::now();
        down(&mut stack, 1, HOME, pt(300.0, 800.0), t0);
        mv(&mut stack, t0, 1, 300.0, 755.0);
        stack.handle(Input::Cancel);
        assert_eq!(stack.state(DRAWER).presence, 0.5);
        assert_eq!(target(&stack, DRAWER), Final::Closed);
        assert_eq!(stack.animating(), [DRAWER]);
        stack.handle(Input::Tick {
            dt: Duration::from_millis(80),
        });
        assert_eq!(stack.state(DRAWER).presence, 0.0);
        assert!(stack.animating().is_empty());
    }

    #[test]
    fn a_bare_tap_does_not_freeze_a_running_fade() {
        let home = home(false, drawer(INSTANT))
            .close(swipe(BOTTOM_EDGE, Direction::Down, Timing::Any), FADE);
        let mut stack = build(rest(home));
        quick_bottom(&mut stack);
        assert_eq!(stack.animating(), [HOME_ID]);
        down(&mut stack, 1, BOTTOM_EDGE, pt(300.0, 10.0), Instant::now());
        stack.handle(Input::Tick {
            dt: Duration::from_millis(80),
        });
        assert_eq!(stack.state(HOME_ID).presence, 0.5);
    }

    #[test]
    fn an_open_under_a_parent_closed_by_the_same_gesture_is_skipped() {
        let home =
            home(true, drawer(INSTANT)).close(swipe(HOME, Direction::Up, Timing::Any), INSTANT);
        let mut stack = build(rest(home));
        home_up(&mut stack);
        assert!(open_ones(&stack).is_empty());
        assert_eq!(stack.state(DRAWER).presence, 0.0);
        assert!(settled(&stack));
    }

    /// Logs the velocity every `follow` and `settle` receives; settles like `FADE`.
    #[derive(Clone, Default)]
    struct Velocities {
        follow: Rc<RefCell<Vec<(f32, f32)>>>,
        settle: Rc<RefCell<Vec<(f32, f32)>>>,
    }

    impl Motion for Velocities {
        fn follow(&self, current: f32, _toward: Final, _progress: f32, v: (f32, f32)) -> f32 {
            self.follow.borrow_mut().push(v);
            current
        }
        fn settle(
            &self,
            current: f32,
            target: Final,
            dt: Duration,
            velocity: (f32, f32),
            cfg: &NavConfig,
        ) -> f32 {
            self.settle.borrow_mut().push(velocity);
            FADE.settle(current, target, dt, velocity, cfg)
        }
    }

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn follow_gets_the_gesture_velocity_and_settle_the_release_velocity() {
        let m = Velocities::default();
        let mut stack = build(rest(home(true, drawer(m.clone()))));
        home_up(&mut stack);
        assert_eq!(target(&stack, DRAWER), Final::Open);
        let follow = m.follow.borrow().clone();
        assert!(!follow.is_empty());
        assert!(follow.iter().all(|v| v.1 < 0.0), "upward: {follow:?}");
        assert!(near(*follow.last().unwrap(), (0.0, -1.0)), "{follow:?}");
        stack.handle(Input::Tick { dt: STEP });
        stack.handle(Input::Tick { dt: STEP });
        // The release settle and every tick after it get the release velocity.
        let settle = m.settle.borrow().clone();
        assert_eq!(settle.len(), 3, "{settle:?}");
        assert!(settle.iter().all(|v| near(*v, (0.0, -1.0))), "{settle:?}");
    }

    #[test]
    fn second_contact_is_ignored() {
        let mut stack = tree();
        let t0 = Instant::now();
        let from = pt(300.0, 800.0);
        let to = pt(300.0, 600.0);
        let duration = Duration::from_millis(200);
        down(&mut stack, 1, HOME, from, t0);
        for (n, (dt, pos)) in moves(from, to, duration, (0.0, -1.0))
            .into_iter()
            .enumerate()
        {
            stack.handle(Input::Move {
                contact: 1,
                pos,
                t: t0 + dt,
            });
            if n == 3 {
                down(&mut stack, 2, HOME, pt(100.0, 500.0), t0 + dt);
                stack.handle(Input::Move {
                    contact: 2,
                    pos: pt(400.0, 500.0),
                    t: t0 + dt,
                });
                stack.handle(Input::Up {
                    contact: 2,
                    t: t0 + dt,
                });
                stack.handle(Input::Up {
                    contact: 7,
                    t: t0 + dt,
                });
                assert_eq!(open_ones(&stack), [HOME_ID]);
            }
        }
        stack.handle(Input::Up {
            contact: 1,
            t: t0 + duration,
        });
        assert_eq!(open_ones(&stack), [HOME_ID, DRAWER]);
    }

    fn corner_action(stack: &mut Stack, span: Span) -> usize {
        stack.action(Box::new(
            swipe(BOTTOM_EDGE, Direction::Up, Timing::Any).span(span),
        ))
    }

    fn bottom_from(stack: &mut Stack, x: f32, duration_ms: u64, end_velocity: (f32, f32)) {
        swipe_on(
            stack,
            BOTTOM_EDGE,
            pt(x, 10.0),
            pt(x, -110.0),
            Duration::from_millis(duration_ms),
            end_velocity,
        );
    }

    #[test]
    fn an_action_fires_on_commit_and_moves_no_container() {
        let mut stack = tree_home_closed();
        let left = corner_action(&mut stack, Span::LeftCorner);
        let right = corner_action(&mut stack, Span::RightCorner);
        bottom_from(&mut stack, 20.0, 150, (0.0, -1.5));
        assert_eq!(stack.take_fired(), [left]);
        assert!(stack.take_fired().is_empty(), "taking clears");
        bottom_from(&mut stack, 580.0, 500, (0.0, 0.0));
        assert_eq!(stack.take_fired(), [right]);
        // The whole-edge home and running triggers still fire too; only
        // spans keep them apart.
        assert!(open_ones(&stack).contains(&RUNNING));
    }

    #[test]
    fn an_action_does_not_fire_outside_its_span_or_on_a_short_swipe() {
        let mut stack = tree();
        corner_action(&mut stack, Span::LeftCorner);
        quick_bottom(&mut stack);
        assert!(stack.take_fired().is_empty());
        swipe_on(
            &mut stack,
            BOTTOM_EDGE,
            pt(20.0, 10.0),
            pt(20.0, -30.0),
            Duration::from_millis(150),
            (0.0, -1.0),
        );
        assert!(stack.take_fired().is_empty());
    }

    #[test]
    fn an_action_on_another_area_does_not_fire() {
        let mut stack = tree();
        stack.action(Box::new(swipe(TOP_EDGE, Direction::Up, Timing::Any)));
        bottom_from(&mut stack, 300.0, 150, (0.0, -1.5));
        assert!(stack.take_fired().is_empty());
    }

    #[test]
    fn cancel_changes_nothing() {
        let mut stack = tree();
        let t0 = Instant::now();
        let from = pt(300.0, 800.0);
        down(&mut stack, 1, HOME, from, t0);
        for (dt, pos) in moves(
            from,
            pt(300.0, 600.0),
            Duration::from_millis(200),
            (0.0, -1.0),
        ) {
            stack.handle(Input::Move {
                contact: 1,
                pos,
                t: t0 + dt,
            });
        }
        stack.handle(Input::Cancel);
        stack.handle(Input::Up {
            contact: 1,
            t: t0 + Duration::from_millis(200),
        });
        assert_eq!(open_ones(&stack), [HOME_ID]);
        assert!(settled(&stack));
        // The stack takes a fresh gesture afterwards.
        home_up(&mut stack);
        assert_eq!(open_ones(&stack), [HOME_ID, DRAWER]);
    }

    #[test]
    fn diagonal_swipe_opens_one_child() {
        let mut stack = tree();
        swipe_on(
            &mut stack,
            HOME,
            pt(100.0, 800.0),
            pt(220.0, 700.0),
            Duration::from_millis(200),
            (0.6, -0.5),
        );
        assert_eq!(open_ones(&stack), [HOME_ID, SEARCH]);
    }
}
