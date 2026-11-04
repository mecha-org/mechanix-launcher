//! Haptic (and sound/LED, per the user's theme) feedback for key presses
//! through feedbackd on the session bus.

use mecha_wayland::prelude::*;

const FEEDBACKD: &str = "org.sigxcpu.Feedback";
const FEEDBACKD_PATH: &str = "/org/sigxcpu/Feedback";
const FEEDBACKD_INTERFACE: &str = "org.sigxcpu.Feedback";

/// The id feedbackd picks per-app profiles by.
const APP_ID: &str = "mechanix.keyboard";
const BUTTON_PRESSED: &str = "button-pressed";
/// Play the event's feedback once.
const ONCE: i32 = -1;

/// The `TriggerFeedback` call, built once and resent on every press; `None`
/// when the host has no session bus (such as a headless test).
pub(crate) struct Feedbackd {
    button_pressed: Option<MethodCall>,
}
impl Resource for Feedbackd {}

pub(crate) fn install(app: &mut App) {
    let has_session =
        app.has_resource::<DBus>() && app.resource_mut::<DBus>().bus(BusId::Session).is_some();
    let button_pressed = has_session.then(|| {
        MethodCall::new(
            FEEDBACKD,
            FEEDBACKD_PATH,
            FEEDBACKD_INTERFACE,
            "TriggerFeedback",
        )
        .body(&button_pressed())
    });
    app.insert_resource(Feedbackd { button_pressed });
}

/// `TriggerFeedback`'s arguments: app id, event, hints, timeout.
fn button_pressed() -> (&'static str, &'static str, PropMap, i32) {
    (APP_ID, BUTTON_PRESSED, PropMap::new(), ONCE)
}

/// Ask feedbackd for the key-press feedback. Fire and forget: the call goes
/// out with `NO_REPLY_EXPECTED`, so a missing daemon costs nothing.
pub(crate) fn key_pressed<W: Widget>(ctx: &mut Context<'_, W>) {
    let Some(call) = ctx.resource::<Feedbackd>().button_pressed.clone() else {
        return;
    };
    if let Some(mut bus) = ctx.resource_mut::<DBus>().bus(BusId::Session) {
        bus.send_no_reply(call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_feedback_signature() {
        let msg =
            Message::method_call(FEEDBACKD_PATH, "TriggerFeedback").with_body(&button_pressed());
        assert_eq!(msg.signature().as_str(), "ssa{sv}i");
    }
}
