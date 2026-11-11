//! System power-off, restart and suspend through systemd-logind on the
//! system bus.
use mecha_wayland::prelude::*;

pub(crate) const LOGIND: &str = "org.freedesktop.login1";
pub(crate) const MANAGER_PATH: &str = "/org/freedesktop/login1";
pub(crate) const MANAGER: &str = "org.freedesktop.login1.Manager";

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
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    #[test]
    fn power_off_is_sent_once_until_reply_or_disconnect() {
        let (client, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(b"OK 0123456789abcdef0123456789abcdef\r\nERROR no fd passing\r\n")
            .unwrap();
        let hello = Message::method_return(1)
            .with_sender("org.freedesktop.DBus")
            .with_destination(":1.1")
            .with_body(&(":1.1",));
        peer.write_all(&hello.encode(1).0).unwrap();

        let mut app = App::new();
        app.add_module(RingModule::default())
            .add_module(DBusModule::new().over(BusId::System, client));
        let mut auth = Vec::new();
        while !auth.ends_with(b"BEGIN\r\n") {
            let mut byte = [0];
            peer.read_exact(&mut byte).unwrap();
            auth.push(byte[0]);
        }
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
