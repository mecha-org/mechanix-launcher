//! The agent on the bus: exports `org.freedesktop.PolicyKit1.AuthenticationAgent`.
//!
//! `BeginAuthentication` calls are kept unanswered until finished, and
//! handled one at a time in arrival order. Only the registered polkitd
//! may call; others get `AccessDenied`. The cookie is never logged.

use std::collections::{HashMap, VecDeque};

use mecha_wayland::prelude::*;

/// Where the agent is exported.
pub const AGENT_PATH: &str = "/org/mechanix/PolicyKit1/AuthenticationAgent";
const AGENT_IFACE: &str = "org.freedesktop.PolicyKit1.AuthenticationAgent";

const FAILED: &str = "org.freedesktop.PolicyKit1.Error.Failed";
const CANCELLED: &str = "org.freedesktop.PolicyKit1.Error.Cancelled";
const ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identity {
    User(u32),
    Group(u32),
    /// Unknown kind, or missing its id.
    Other(String),
}

/// One `BeginAuthentication`.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub action_id: String,
    pub message: String,
    pub icon_name: String,
    pub details: HashMap<String, String>,
    pub cookie: String,
    pub identities: Vec<Identity>,
}

impl std::fmt::Debug for Request {
    /// Leaves out the cookie.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("action_id", &self.action_id)
            .field("message", &self.message)
            .field("icon_name", &self.icon_name)
            .field("details", &self.details)
            .field("identities", &self.identities)
            .finish_non_exhaustive()
    }
}

type BeginArgs = (
    String,
    String,
    String,
    HashMap<String, String>,
    String,
    Vec<(String, HashMap<String, Value>)>,
);

impl Request {
    fn from_args(
        (action_id, message, icon_name, details, cookie, identities): BeginArgs,
    ) -> Request {
        let identities = identities
            .into_iter()
            .map(|(kind, details)| {
                let id = |key: &str| details.get(key).and_then(|v| v.get::<u32>().ok());
                match (kind.as_str(), id("uid"), id("gid")) {
                    ("unix-user", Some(uid), _) => Identity::User(uid),
                    ("unix-group", _, Some(gid)) => Identity::Group(gid),
                    _ => Identity::Other(kind),
                }
            })
            .collect();
        Request {
            action_id,
            message,
            icon_name,
            details,
            cookie,
            identities,
        }
    }
}

/// The active request: answer it with [`AuthFinished`].
#[derive(Debug, Clone)]
pub struct AuthRequested(pub Request);
impl Signal for AuthRequested {}

/// polkitd cancelled the active request.
#[derive(Debug, Clone)]
pub struct AuthCancelled {
    pub cookie: String,
}
impl Signal for AuthCancelled {}

/// Answer the active request with this cookie.
#[derive(Debug, Clone)]
pub struct AuthFinished {
    pub cookie: String,
    pub result: Result<(), String>,
}
impl Signal for AuthFinished {}

struct Kept {
    request: Request,
    call: IncomingCall,
}

pub(crate) struct Agent {
    bus: BusId,
    export: Option<ExportId>,
    /// The only caller taken.
    polkitd: Option<BusName>,
    active: Option<Kept>,
    queued: VecDeque<Kept>,
}
impl Resource for Agent {}

/// Export the agent on `bus`; false if that failed.
pub(crate) fn install(app: &mut App, bus: BusId) -> bool {
    if app.has_resource::<Agent>() {
        return app.resource::<Agent>().export.is_some();
    }
    let export = {
        let mut dbus = app.resource_mut::<DBus>();
        match dbus.bus(bus) {
            Some(mut b) => {
                let spec = InterfaceSpec::new(AGENT_IFACE)
                    .method("BeginAuthentication", "sssa{ss}sa(sa{sv})", "")
                    .method("CancelAuthentication", "s", "");
                b.export(AGENT_PATH, spec)
                    .inspect_err(|e| tracing::error!("polkit: exporting the agent: {e:?}"))
                    .ok()
            }
            None => {
                tracing::error!("polkit: the bus is not connected, no agent for the session");
                None
            }
        }
    };
    let exported = export.is_some();
    app.insert_resource(Agent {
        bus,
        export,
        polkitd: None,
        active: None,
        queued: VecDeque::new(),
    });
    app.system(on_call)
        .system(on_finished)
        .system(on_disconnect);
    exported
}

fn on_call(app: &mut App, e: &MethodCallReceived) {
    let polkitd = {
        let agent = app.resource::<Agent>();
        if agent.export != Some(e.export) {
            return;
        }
        agent.polkitd.clone()
    };
    if polkitd.is_none() || e.call.sender() != polkitd.as_ref() {
        tracing::warn!(
            "polkit: refused {} from {:?}: not polkitd",
            e.call.member(),
            e.call.sender()
        );
        app.resource_mut::<DBus>().reply_error(
            &e.call,
            ACCESS_DENIED,
            "Only polkitd may call the agent",
        );
        return;
    }
    match e.call.member().as_str() {
        "BeginAuthentication" => begin(app, &e.call),
        "CancelAuthentication" => cancel(app, &e.call),
        _ => {}
    }
}

fn begin(app: &mut App, call: &IncomingCall) {
    let request = match call.args::<BeginArgs>() {
        Ok(args) => Request::from_args(args),
        Err(error) => {
            tracing::error!("polkit: BeginAuthentication: bad arguments: {error:?}");
            app.resource_mut::<DBus>()
                .reply_error(call, FAILED, "Bad arguments");
            return;
        }
    };
    tracing::info!("polkit: authentication requested: {request:?}");
    app.resource_mut::<Agent>().queued.push_back(Kept {
        request,
        call: call.clone(),
    });
    next(app);
}

fn next(app: &mut App) {
    let request = {
        let mut agent = app.resource_mut::<Agent>();
        if agent.active.is_some() {
            if !agent.queued.is_empty() {
                tracing::info!("polkit: {} request(s) waiting", agent.queued.len());
            }
            return;
        }
        let Some(kept) = agent.queued.pop_front() else {
            return;
        };
        let request = kept.request.clone();
        agent.active = Some(kept);
        request
    };
    app.signal(AuthRequested(request));
}

fn cancel(app: &mut App, call: &IncomingCall) {
    let Ok((cookie,)) = call.args::<(String,)>() else {
        app.resource_mut::<DBus>()
            .reply_error(call, FAILED, "Bad arguments");
        return;
    };
    let found = match take_active(app, &cookie) {
        Some(kept) => Some((kept, true)),
        None => take_queued(app, &cookie).map(|kept| (kept, false)),
    };
    let Some((kept, was_active)) = found else {
        app.resource_mut::<DBus>()
            .reply_error(call, FAILED, "No such authentication");
        return;
    };
    tracing::info!("polkit: {} cancelled", kept.request.action_id);
    {
        let mut dbus = app.resource_mut::<DBus>();
        dbus.reply_error(&kept.call, CANCELLED, "Authentication was cancelled");
        dbus.reply(call, &());
    }
    if was_active {
        app.signal(AuthCancelled { cookie });
        next(app);
    }
}

fn on_finished(app: &mut App, e: &AuthFinished) {
    let Some(kept) = take_active(app, &e.cookie) else {
        return;
    };
    {
        let mut dbus = app.resource_mut::<DBus>();
        match &e.result {
            Ok(()) => {
                tracing::info!("polkit: {} authenticated", kept.request.action_id);
                dbus.reply(&kept.call, &());
            }
            Err(why) => {
                tracing::info!(
                    "polkit: {} not authenticated: {why}",
                    kept.request.action_id
                );
                dbus.reply_error(&kept.call, FAILED, why);
            }
        }
    }
    next(app);
}

fn take_active(app: &mut App, cookie: &str) -> Option<Kept> {
    let mut agent = app.resource_mut::<Agent>();
    if agent.active.as_ref()?.request.cookie != cookie {
        return None;
    }
    agent.active.take()
}

fn take_queued(app: &mut App, cookie: &str) -> Option<Kept> {
    let mut agent = app.resource_mut::<Agent>();
    let i = agent
        .queued
        .iter()
        .position(|k| k.request.cookie == cookie)?;
    agent.queued.remove(i)
}

/// Take calls only from `polkitd`. A new polkitd drops every request.
pub(crate) fn set_polkitd(app: &mut App, polkitd: Option<BusName>) {
    let cancelled = {
        let mut agent = app.resource_mut::<Agent>();
        if agent.polkitd == polkitd {
            return;
        }
        agent.polkitd = polkitd;
        agent.queued.clear();
        agent.active.take().map(|kept| kept.request.cookie)
    };
    if let Some(cookie) = cancelled {
        app.signal(AuthCancelled { cookie });
    }
}

#[cfg(test)]
pub(crate) fn polkitd(app: &App) -> Option<BusName> {
    app.resource::<Agent>().polkitd.clone()
}

/// The bus is gone, and polkitd's calls with it: drop them, and close
/// the active one's dialog.
fn on_disconnect(app: &mut App, e: &BusDisconnected) {
    let cancelled = {
        let mut agent = app.resource_mut::<Agent>();
        if e.bus != agent.bus {
            return;
        }
        agent.polkitd = None;
        agent.queued.clear();
        agent.active.take().map(|kept| kept.request.cookie)
    };
    if let Some(cookie) = cancelled {
        app.signal(AuthCancelled { cookie });
    }
}

#[cfg(test)]
mod tests {
    use dbus::fake::{FakeBus, UNIQUE};

    use super::*;

    const POLKITD: &str = ":1.5";

    /// The cookies [`AuthCancelled`] named.
    struct Cancelled(Vec<String>);
    impl Resource for Cancelled {}

    fn record_cancelled(app: &mut App, e: &AuthCancelled) {
        app.resource_mut::<Cancelled>().0.push(e.cookie.clone());
    }

    fn agent() -> FakeBus {
        let mut bus = FakeBus::new();
        assert!(install(&mut bus.app, BusId::Session));
        set_polkitd(&mut bus.app, Some(BusName::new(POLKITD).unwrap()));
        bus.turn();
        bus.received();
        bus
    }

    fn call(member: &'static str) -> Message {
        call_from(POLKITD, member)
    }

    fn call_from(sender: &'static str, member: &'static str) -> Message {
        Message::method_call(AGENT_PATH, member)
            .with_interface(AGENT_IFACE)
            .with_destination(UNIQUE)
            .with_sender(sender)
    }

    fn begin(bus: &mut FakeBus, cookie: &str) -> u32 {
        let details = HashMap::from([("program".to_string(), "/usr/bin/true".to_string())]);
        let identities = vec![
            (
                "unix-user".to_string(),
                HashMap::from([("uid".to_string(), Value::U32(1000))]),
            ),
            (
                "unix-group".to_string(),
                HashMap::from([("gid".to_string(), Value::U32(27))]),
            ),
            ("unix-netgroup".to_string(), HashMap::new()),
        ];
        let serial = bus.send(call("BeginAuthentication").with_body(&(
            "org.freedesktop.policykit.exec",
            "Authentication is needed",
            "",
            details,
            cookie,
            identities,
        )));
        bus.turn();
        serial
    }

    fn cancel(bus: &mut FakeBus, cookie: &str) -> u32 {
        let serial = bus.send(call("CancelAuthentication").with_body(&(cookie,)));
        bus.turn();
        serial
    }

    /// The answer to the call with `serial`, if any.
    fn answer(received: &[Message], serial: u32) -> Option<&Message> {
        received.iter().find(|m| m.reply_serial() == Some(serial))
    }

    fn error_name(m: &Message) -> &str {
        m.error_name().map_or("", |n| n.as_str())
    }

    fn active_cookie(bus: &FakeBus) -> Option<String> {
        let agent = bus.app.resource::<Agent>();
        agent.active.as_ref().map(|k| k.request.cookie.clone())
    }

    fn queued_cookies(bus: &FakeBus) -> Vec<String> {
        let agent = bus.app.resource::<Agent>();
        agent
            .queued
            .iter()
            .map(|k| k.request.cookie.clone())
            .collect()
    }

    #[test]
    fn a_request_is_decoded_and_kept_unanswered() {
        let mut bus = agent();
        let serial = begin(&mut bus, "c1");
        assert!(answer(&bus.received(), serial).is_none());

        let agent = bus.app.resource::<Agent>();
        let request = &agent.active.as_ref().unwrap().request;
        assert_eq!(request.action_id, "org.freedesktop.policykit.exec");
        assert_eq!(request.message, "Authentication is needed");
        assert_eq!(request.details["program"], "/usr/bin/true");
        assert_eq!(request.cookie, "c1");
        assert_eq!(
            request.identities,
            [
                Identity::User(1000),
                Identity::Group(27),
                Identity::Other("unix-netgroup".into()),
            ]
        );
    }

    #[test]
    fn finishing_answers_the_request() {
        let mut bus = agent();
        let serial = begin(&mut bus, "c1");
        bus.received();

        // Another cookie changes nothing.
        bus.app.signal(AuthFinished {
            cookie: "other".into(),
            result: Ok(()),
        });
        bus.turn();
        assert!(answer(&bus.received(), serial).is_none());

        bus.app.signal(AuthFinished {
            cookie: "c1".into(),
            result: Ok(()),
        });
        bus.turn();
        let received = bus.received();
        let reply = answer(&received, serial).unwrap();
        assert_eq!(reply.kind(), MessageKind::MethodReturn);
        assert_eq!(active_cookie(&bus), None);
    }

    #[test]
    fn a_failure_answers_failed() {
        let mut bus = agent();
        let serial = begin(&mut bus, "c1");
        bus.received();
        bus.app.signal(AuthFinished {
            cookie: "c1".into(),
            result: Err("Wrong password".into()),
        });
        bus.turn();
        let received = bus.received();
        assert_eq!(error_name(answer(&received, serial).unwrap()), FAILED);
    }

    #[test]
    fn cancelling_answers_cancelled() {
        let mut bus = agent();
        let begun = begin(&mut bus, "c1");
        bus.received();

        // An unknown cookie is refused and the request stays.
        let wrong = cancel(&mut bus, "other");
        let received = bus.received();
        assert_eq!(error_name(answer(&received, wrong).unwrap()), FAILED);
        assert!(answer(&received, begun).is_none());

        let cancelled = cancel(&mut bus, "c1");
        let received = bus.received();
        assert_eq!(error_name(answer(&received, begun).unwrap()), CANCELLED);
        assert_eq!(
            answer(&received, cancelled).unwrap().kind(),
            MessageKind::MethodReturn
        );
        assert_eq!(active_cookie(&bus), None);
    }

    #[test]
    fn requests_wait_their_turn() {
        let mut bus = agent();
        let first = begin(&mut bus, "c1");
        let second = begin(&mut bus, "c2");
        let third = begin(&mut bus, "c3");
        let received = bus.received();
        for serial in [first, second, third] {
            assert!(answer(&received, serial).is_none());
        }
        assert_eq!(active_cookie(&bus).as_deref(), Some("c1"));
        assert_eq!(queued_cookies(&bus), ["c2", "c3"]);

        // Finishing a queued one does nothing: only the active is answered.
        bus.app.signal(AuthFinished {
            cookie: "c2".into(),
            result: Ok(()),
        });
        bus.turn();
        assert!(answer(&bus.received(), second).is_none());

        // The first done, the next in line becomes active.
        bus.app.signal(AuthFinished {
            cookie: "c1".into(),
            result: Ok(()),
        });
        bus.turn();
        assert!(answer(&bus.received(), first).is_some());
        assert_eq!(active_cookie(&bus).as_deref(), Some("c2"));
        assert_eq!(queued_cookies(&bus), ["c3"]);
    }

    #[test]
    fn a_queued_request_can_be_cancelled() {
        let mut bus = agent();
        begin(&mut bus, "c1");
        let second = begin(&mut bus, "c2");
        begin(&mut bus, "c3");
        bus.received();

        cancel(&mut bus, "c2");
        let received = bus.received();
        assert_eq!(error_name(answer(&received, second).unwrap()), CANCELLED);
        assert_eq!(active_cookie(&bus).as_deref(), Some("c1"));
        assert_eq!(queued_cookies(&bus), ["c3"]);

        // Cancelling the active one moves the queue on.
        cancel(&mut bus, "c1");
        assert_eq!(active_cookie(&bus).as_deref(), Some("c3"));
        assert!(queued_cookies(&bus).is_empty());
    }

    #[test]
    fn only_polkitd_may_call() {
        let mut bus = agent();
        let begun = begin(&mut bus, "c1");

        let stranger = bus.send(call_from(":1.66", "CancelAuthentication").with_body(&("c1",)));
        bus.turn();
        let received = bus.received();
        assert_eq!(
            error_name(answer(&received, stranger).unwrap()),
            ACCESS_DENIED
        );
        // The request is untouched.
        assert!(answer(&received, begun).is_none());
        assert_eq!(active_cookie(&bus).as_deref(), Some("c1"));

        let details: HashMap<String, String> = HashMap::new();
        let identities: Vec<(String, HashMap<String, Value>)> = Vec::new();
        let fake = bus.send(call_from(":1.66", "BeginAuthentication").with_body(&(
            "org.freedesktop.policykit.exec",
            "Give me your password",
            "",
            details,
            "c2",
            identities,
        )));
        bus.turn();
        let received = bus.received();
        assert_eq!(error_name(answer(&received, fake).unwrap()), ACCESS_DENIED);
        assert!(queued_cookies(&bus).is_empty());
    }

    #[test]
    fn unregistered_takes_no_calls() {
        let mut bus = agent();
        set_polkitd(&mut bus.app, None);
        let serial = begin(&mut bus, "c1");
        let received = bus.received();
        assert_eq!(
            error_name(answer(&received, serial).unwrap()),
            ACCESS_DENIED
        );
        assert_eq!(active_cookie(&bus), None);
    }

    #[test]
    fn polkitd_going_away_drops_its_requests() {
        let mut bus = agent();
        begin(&mut bus, "c1");
        begin(&mut bus, "c2");
        bus.received();

        bus.app.insert_resource(Cancelled(Vec::new()));
        bus.app.system(record_cancelled);

        set_polkitd(&mut bus.app, None);
        bus.turn();
        assert_eq!(active_cookie(&bus), None);
        assert!(queued_cookies(&bus).is_empty());
        assert_eq!(bus.app.resource::<Cancelled>().0, ["c1"]);
    }

    #[test]
    fn without_the_bus_there_is_no_agent() {
        // The fake connects only the session bus.
        let mut bus = FakeBus::new();
        assert!(!install(&mut bus.app, BusId::System));
        // Installing again says the same.
        assert!(!install(&mut bus.app, BusId::System));
    }

    #[test]
    fn the_bus_going_away_cancels_the_active_request() {
        let mut bus = agent();
        begin(&mut bus, "c1");
        begin(&mut bus, "c2");
        bus.received();

        bus.app.insert_resource(Cancelled(Vec::new()));
        bus.app.system(record_cancelled);

        bus.app.signal(BusDisconnected {
            bus: BusId::Session,
        });
        bus.turn();
        assert_eq!(active_cookie(&bus), None);
        assert!(queued_cookies(&bus).is_empty());
        assert_eq!(bus.app.resource::<Cancelled>().0, ["c1"]);
        assert_eq!(polkitd(&bus.app), None);
    }
}
