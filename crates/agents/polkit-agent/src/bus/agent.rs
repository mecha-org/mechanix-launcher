//! The agent on the bus: exports `org.freedesktop.PolicyKit1.AuthenticationAgent`.
//!
//! `BeginAuthentication` calls are kept unanswered until finished, and
//! handled one at a time in arrival order. Only the registered polkitd
//! may call; others get `AccessDenied`. The cookie is never logged.

use std::collections::{HashMap, VecDeque};

use mecha_wayland::prelude::*;

use crate::signals::{AuthCancelled, AuthFinished, AuthRequested, Identity, Request};

/// Where the agent is exported.
pub const AGENT_PATH: &str = "/org/mechanix/PolicyKit1/AuthenticationAgent";
const AGENT_IFACE: &str = "org.freedesktop.PolicyKit1.AuthenticationAgent";

const FAILED: &str = "org.freedesktop.PolicyKit1.Error.Failed";
const CANCELLED: &str = "org.freedesktop.PolicyKit1.Error.Cancelled";
const ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";

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

/// A request, and its `BeginAuthentication` call, kept unanswered.
struct PendingCall {
    request: Request,
    call: IncomingCall,
}

pub(crate) struct Agent {
    bus: BusId,
    export: Option<ExportId>,
    /// The only caller taken.
    polkitd: Option<BusName>,
    active: Option<PendingCall>,
    queued: VecDeque<PendingCall>,
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
    app.resource_mut::<Agent>().queued.push_back(PendingCall {
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

fn take_active(app: &mut App, cookie: &str) -> Option<PendingCall> {
    let mut agent = app.resource_mut::<Agent>();
    if agent.active.as_ref()?.request.cookie != cookie {
        return None;
    }
    agent.active.take()
}

fn take_queued(app: &mut App, cookie: &str) -> Option<PendingCall> {
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
mod tests;
