//! System power-off, restart and suspend through systemd-logind on the
//! system bus.
use mecha_wayland::prelude::*;

use crate::logind::{LOGIND, MANAGER, MANAGER_PATH};

/// Request an interactive power-off (logind may prompt for authorization).
pub struct PowerOff;
impl Signal for PowerOff {}

/// Request an interactive reboot (logind may prompt for authorization).
pub struct Restart;
impl Signal for Restart {}

/// Suspend now. logind still waits for its delay inhibitors, such as the
/// session's lock before sleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Suspend;
impl Signal for Suspend {}

struct PowerStatus {
    pending: Option<(CallId, &'static str)>,
}
impl Resource for PowerStatus {}

/// Install after `RingModule` and `DBusModule::new().system()`.
pub struct PowerModule;
impl Module for PowerModule {
    fn install(self, app: &mut App) {
        if app.has_resource::<PowerStatus>() {
            return;
        }
        app.insert_resource(PowerStatus { pending: None });
        app.system(on_power_off)
            .system(on_restart)
            .system(|app, _: &Suspend| {
                let call =
                    MethodCall::new(LOGIND, MANAGER_PATH, MANAGER, "Suspend").body(&(false,));
                app.resource_mut::<DBus>().system().send_no_reply(call);
            })
            .system(on_reply)
            .system(on_disconnect);
    }
}

fn on_power_off(app: &mut App, _: &PowerOff) {
    request(app, "PowerOff");
}

fn on_restart(app: &mut App, _: &Restart) {
    request(app, "Reboot");
}

// FIXME: this ends the session at once: apps are killed with no chance to
// save. Power-off, reboot and the logout still to come should share one
// session ending flow, offered over D-Bus so a terminal can ask for it too:
// ask the apps first (the portal's Inhibit monitor; whoever holds a logout
// inhibitor goes in a prompt to go ahead or cancel), ask each window to
// close and wait a little, and only then ask logind.
fn request(app: &mut App, method: &'static str) {
    if app.resource::<PowerStatus>().pending.is_some() {
        return;
    }
    // The `interactive` argument alone is not enough: logind only asks polkit
    // to prompt when the message header allows interactive authorization.
    let call = app.resource_mut::<DBus>().system().send(
        MethodCall::new(LOGIND, MANAGER_PATH, MANAGER, method)
            .body(&(true,))
            .allow_interactive_auth(),
    );
    app.resource_mut::<PowerStatus>().pending = Some((call, method));
}

fn on_reply(app: &mut App, event: &MethodReply) {
    let Some((call, method)) = app.resource::<PowerStatus>().pending else {
        return;
    };
    if call != event.call {
        return;
    }
    app.resource_mut::<PowerStatus>().pending = None;
    if let Err(error) = &event.result {
        eprintln!("[power] {method} failed: {error:?}");
    }
}

fn on_disconnect(app: &mut App, event: &BusDisconnected) {
    if event.bus == BusId::System {
        app.resource_mut::<PowerStatus>().pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_off_is_sent_once_until_reply_or_disconnect() {
        let (mut app, _peer) = crate::test_bus::system_bus();
        app.add_module(PowerModule);

        app.signal(PowerOff);
        app.flush();
        let (first, method) = app.resource::<PowerStatus>().pending.unwrap();
        assert_eq!(method, "PowerOff");
        app.signal(PowerOff);
        app.flush();
        assert_eq!(app.resource::<PowerStatus>().pending, Some((first, method)));

        app.signal(Restart);
        app.flush();
        assert_eq!(app.resource::<PowerStatus>().pending, Some((first, method)));

        app.signal(MethodReply {
            call: first,
            result: Ok(Message::method_return(first.serial)),
        });
        app.flush();
        assert_eq!(app.resource::<PowerStatus>().pending, None);

        app.signal(Restart);
        app.flush();
        let (second, method) = app.resource::<PowerStatus>().pending.unwrap();
        assert_ne!(second, first);
        assert_eq!(method, "Reboot");
        app.signal(BusDisconnected { bus: BusId::System });
        app.flush();
        assert_eq!(app.resource::<PowerStatus>().pending, None);
    }
}
