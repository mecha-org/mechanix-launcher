use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use super::protocol::*;
use super::*;
use crate::signals::{Identity, Request};

const WAIT: Duration = Duration::from_secs(5);

/// What the helper client said, in order, each as `<cookie> <what>`.
#[derive(Default)]
struct Seen(Vec<String>);
impl Resource for Seen {}

fn note(app: &mut App, line: String) {
    app.resource_mut::<Seen>().0.push(line);
}

/// The client, with a fake helper socket it connects to.
struct Fake {
    app: App,
    listener: UnixListener,
    dir: PathBuf,
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fake() -> Fake {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "polkit-helper-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("agent-helper.socket");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();

    let mut app = App::new();
    app.add_module(RingModule::default())
        .init_resource::<Seen>();
    install(&mut app, BusId::System, path);
    app.system(|app, e: &AuthUser| note(app, format!("{} user {}", e.cookie, e.user)))
        .system(|app, e: &AuthPrompt| {
            note(
                app,
                format!("{} prompt {:?} echo={}", e.cookie, e.text, e.echo),
            );
        })
        .system(|app, e: &AuthMessage| {
            let kind = if e.error { "error" } else { "info" };
            note(app, format!("{} {kind} {:?}", e.cookie, e.text));
        })
        .system(|app, e: &AuthRetry| note(app, format!("{} retry", e.cookie)))
        .system(|app, e: &AuthLocked| note(app, format!("{} locked", e.cookie)))
        .system(|app, e: &AuthFinished| {
            let result = match &e.result {
                Ok(()) => "ok".to_string(),
                Err(why) => format!("err {why:?}"),
            };
            note(app, format!("{} finished {result}", e.cookie));
        });
    Fake { app, listener, dir }
}

fn request(cookie: &str, identities: Vec<Identity>) -> Request {
    Request {
        action_id: "org.freedesktop.policykit.exec".into(),
        message: "Authentication is needed".into(),
        icon_name: String::new(),
        details: HashMap::new(),
        cookie: cookie.into(),
        identities,
    }
}

fn own_name() -> String {
    user_name(own_uid()).expect("the test's user has a name")
}

fn begin(f: &mut Fake, cookie: &str) {
    let me = Identity::User(own_uid());
    f.app.signal(AuthRequested(request(cookie, vec![me])));
    f.app.flush();
}

fn answer(f: &mut Fake, cookie: &str, text: &str) {
    f.app.signal(AuthAnswer {
        cookie: cookie.into(),
        answer: Secret::from(text.to_string()),
    });
    f.app.flush();
}

/// Turn the ring until `n` things were said; everything said.
fn turn_until(f: &mut Fake, n: usize) -> Vec<String> {
    let deadline = Instant::now() + WAIT;
    // Wakes a turn that would otherwise wait on nothing.
    f.app.resource_mut::<Ring>().timeout(WAIT);
    while f.app.resource::<Seen>().0.len() < n {
        assert!(
            Instant::now() < deadline,
            "timed out; said: {:?}",
            f.app.resource::<Seen>().0
        );
        Ring::turn(&mut f.app);
    }
    f.app.resource::<Seen>().0.clone()
}

fn said(f: &Fake) -> Vec<String> {
    f.app.resource::<Seen>().0.clone()
}

/// The helper's end of one connection.
struct Peer {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Peer {
    /// The connection the client made. It connects before returning, so
    /// one is waiting.
    fn accept(f: &Fake) -> Peer {
        let (stream, _) = f.listener.accept().expect("the client connected");
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(WAIT)).unwrap();
        Peer {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
        }
    }

    /// The next line, without its line break; `None` at the end.
    fn line(&mut self) -> Option<String> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).expect("the client wrote");
        (n > 0).then(|| line.trim_end_matches('\n').to_string())
    }

    /// The user and cookie the client opens with.
    fn hello(&mut self) -> (String, String) {
        (self.line().unwrap(), self.line().unwrap())
    }

    fn say(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
    }
}

fn no_connection(f: &Fake) -> bool {
    matches!(f.listener.accept(), Err(e) if e.kind() == ErrorKind::WouldBlock)
}

#[test]
fn authenticates_through_the_helper() {
    let mut f = fake();
    begin(&mut f, "c1");
    let mut peer = Peer::accept(&f);
    assert_eq!(peer.hello(), (own_name(), "c1".to_string()));

    peer.say(b"PAM_TEXT_INFO Caf\\303\\251 is open\n");
    peer.say(b"PAM_PROMPT_ECHO_OFF Password: \n");
    turn_until(&mut f, 3);
    answer(&mut f, "c1", "hunter2");
    assert_eq!(peer.line().as_deref(), Some("hunter2"));

    peer.say(b"SUCCESS\n");
    let said = turn_until(&mut f, 4);
    let user = own_name();
    assert_eq!(
        said,
        [
            format!("c1 user {user}"),
            "c1 info \"Café is open\"".into(),
            "c1 prompt \"Password: \" echo=false".into(),
            "c1 finished ok".into(),
        ]
    );
    // The attempt is over: the client hung up.
    assert_eq!(peer.line(), None);
}

#[test]
fn a_wrong_answer_is_tried_again() {
    let mut f = fake();
    begin(&mut f, "c1");
    let mut first = Peer::accept(&f);
    first.hello();
    first.say(b"PAM_PROMPT_ECHO_OFF Password: \n");
    turn_until(&mut f, 2);
    answer(&mut f, "c1", "wrong");
    assert_eq!(first.line().as_deref(), Some("wrong"));
    first.say(b"PAM_ERROR_MSG Authentication failure\nFAILURE\n");
    turn_until(&mut f, 4);

    // A new attempt, on a new connection, for the same request.
    let mut second = Peer::accept(&f);
    assert_eq!(second.hello(), (own_name(), "c1".to_string()));
    assert_eq!(first.line(), None);
    second.say(b"PAM_PROMPT_ECHO_OFF Password: \n");
    turn_until(&mut f, 5);
    answer(&mut f, "c1", "right");
    assert_eq!(second.line().as_deref(), Some("right"));
    second.say(b"SUCCESS\n");
    let said = turn_until(&mut f, 6);
    assert_eq!(
        said[2..],
        [
            "c1 error \"Authentication failure\"",
            "c1 retry",
            "c1 prompt \"Password: \" echo=false",
            "c1 finished ok",
        ]
    );
}

#[test]
fn locks_out_after_too_many_failures_then_tries_again() {
    let mut f = fake();
    f.app.resource_mut::<Helper>().lockout = Duration::from_millis(50);
    begin(&mut f, "c1");
    for _ in 0..MAX_FAILURES {
        let mut peer = Peer::accept(&f);
        peer.hello();
        peer.say(b"FAILURE\n");
        let n = said(&f).len() + 1;
        turn_until(&mut f, n);
    }
    let seen = said(&f);
    assert_eq!(seen.last().unwrap(), "c1 locked");
    let retries = seen.iter().filter(|s| s.ends_with("retry")).count();
    assert_eq!(retries, MAX_FAILURES as usize - 1);
    // Not failed: the request waits out the lockout.
    assert!(!seen.iter().any(|s| s.contains("finished")));
    assert!(no_connection(&f));

    // Then a new attempt, with the count reset.
    let deadline = Instant::now() + WAIT;
    while no_connection(&f) {
        assert!(Instant::now() < deadline, "no attempt after the lockout");
        f.app
            .resource_mut::<Ring>()
            .timeout(Duration::from_millis(10));
        Ring::turn(&mut f.app);
    }
    assert_eq!(
        f.app
            .resource::<Helper>()
            .session
            .as_ref()
            .unwrap()
            .failures,
        0
    );
}

#[test]
fn cancelling_during_a_lockout_fails_the_request() {
    let mut f = fake();
    f.app.resource_mut::<Helper>().lockout = Duration::from_millis(50);
    begin(&mut f, "c1");
    for _ in 0..MAX_FAILURES {
        let mut peer = Peer::accept(&f);
        peer.hello();
        peer.say(b"FAILURE\n");
        let n = said(&f).len() + 1;
        turn_until(&mut f, n);
    }
    f.app.signal(AuthCancel {
        cookie: "c1".into(),
    });
    f.app.flush();
    assert_eq!(
        said(&f).last().unwrap(),
        "c1 finished err \"Dismissed by the user\""
    );
    // The lockout's end starts nothing for a request that is gone.
    let deadline = Instant::now() + WAIT;
    while !f.app.resource::<Helper>().locks.is_empty() {
        assert!(Instant::now() < deadline, "the lockout never ended");
        f.app
            .resource_mut::<Ring>()
            .timeout(Duration::from_millis(10));
        Ring::turn(&mut f.app);
    }
    assert!(no_connection(&f));
}

#[test]
fn polkitd_cancelling_ends_the_attempt_quietly() {
    let mut f = fake();
    begin(&mut f, "c1");
    let mut peer = Peer::accept(&f);
    peer.hello();
    peer.say(b"PAM_PROMPT_ECHO_OFF Password: \n");
    turn_until(&mut f, 2);

    f.app.signal(AuthCancelled {
        cookie: "c1".into(),
    });
    f.app.flush();
    assert_eq!(peer.line(), None);

    // Nothing more about it: no answer goes, no verdict is given.
    answer(&mut f, "c1", "late");
    assert_eq!(said(&f).len(), 2);
    assert!(no_connection(&f));
}

#[test]
fn the_user_cancelling_fails_the_request() {
    let mut f = fake();
    begin(&mut f, "c1");
    let mut peer = Peer::accept(&f);
    peer.hello();

    // Another request's cancel changes nothing.
    f.app.signal(AuthCancel {
        cookie: "other".into(),
    });
    f.app.signal(AuthCancel {
        cookie: "c1".into(),
    });
    f.app.flush();
    assert_eq!(
        said(&f).last().unwrap(),
        "c1 finished err \"Dismissed by the user\""
    );
    assert_eq!(said(&f).len(), 2);
    assert_eq!(peer.line(), None);
}

#[test]
fn a_helper_that_hangs_up_fails_the_request() {
    let mut f = fake();
    begin(&mut f, "c1");
    let mut peer = Peer::accept(&f);
    peer.hello();
    peer.say(b"PAM_PROMPT_ECHO_OFF Password: \n");
    drop(peer);
    let said = turn_until(&mut f, 3);
    assert_eq!(
        said[2],
        "c1 finished err \"The authentication helper failed\""
    );
}

#[test]
fn no_helper_socket_fails_at_once() {
    let mut f = fake();
    std::fs::remove_file(f.dir.join("agent-helper.socket")).unwrap();
    begin(&mut f, "c1");
    assert_eq!(
        said(&f)[1],
        "c1 finished err \"The authentication helper is not available\""
    );
}

#[test]
fn no_user_to_authenticate_as_fails_at_once() {
    let mut f = fake();
    let netgroup = Identity::Other("unix-netgroup".into());
    f.app.signal(AuthRequested(request("c1", vec![netgroup])));
    f.app.flush();
    assert_eq!(said(&f), ["c1 finished err \"No user to authenticate as\""]);
    assert!(no_connection(&f));
}

#[test]
fn answers_only_an_open_prompt_and_refuses_bad_ones() {
    let mut f = fake();
    begin(&mut f, "c1");
    let mut peer = Peer::accept(&f);
    peer.hello();

    // Nothing asked yet: dropped.
    answer(&mut f, "c1", "early");
    // A line split over two writes, and two lines in one.
    peer.say(b"PAM_TEXT_INFO a");
    peer.say(b"b\nPAM_PROMPT_ECHO_ON Login: \n");
    turn_until(&mut f, 3);

    answer(&mut f, "other", "not this request");
    answer(&mut f, "c1", "two\nlines");
    answer(&mut f, "c1", &"x".repeat(MAX_ANSWER + 1));
    let said = said(&f);
    assert_eq!(said[1], "c1 info \"ab\"");
    assert_eq!(said[2], "c1 prompt \"Login: \" echo=true");
    assert!(said[3].starts_with("c1 error"), "{said:?}");
    assert!(said[4].starts_with("c1 error"), "{said:?}");

    // Still open: the first answer that fits is the one sent.
    answer(&mut f, "c1", &"x".repeat(MAX_ANSWER));
    assert_eq!(peer.line(), Some("x".repeat(MAX_ANSWER)));
}

#[test]
fn lines_are_parsed_and_unescaped() {
    assert_eq!(
        parse_line(b"PAM_PROMPT_ECHO_OFF Password: "),
        Some(Line::Prompt {
            text: "Password: ".into(),
            echo: false
        })
    );
    assert_eq!(
        parse_line(b"PAM_PROMPT_ECHO_ON Login: "),
        Some(Line::Prompt {
            text: "Login: ".into(),
            echo: true
        })
    );
    assert_eq!(
        parse_line(b"PAM_ERROR_MSG Locked for \\\"10\\\" minutes\\n"),
        Some(Line::Error("Locked for \"10\" minutes\n".into()))
    );
    assert_eq!(
        parse_line(b"PAM_TEXT_INFO Mot de passe \\303\\251chou\\303\\251"),
        Some(Line::Info("Mot de passe échoué".into()))
    );
    assert_eq!(parse_line(b"SUCCESS"), Some(Line::Success));
    assert_eq!(parse_line(b"FAILURE"), Some(Line::Failure));
    assert_eq!(parse_line(b"SUCCESS "), None);
    assert_eq!(parse_line(b"PAM_SOMETHING_NEW x"), None);

    assert_eq!(unescape(b"a\\\\b\\tc\\1d\\"), b"a\\b\tc\x01d");
    assert_eq!(unescape(b"\\777\\q"), b"\xffq");
}

#[test]
fn chooses_the_own_user_then_root_then_the_first() {
    use Identity::{Group, Other, User};
    assert_eq!(choose_uid(&[User(0), User(1000)], 1000), Some(1000));
    assert_eq!(choose_uid(&[User(1001), User(1000)], 1000), Some(1000));
    // Root, wherever it is listed.
    assert_eq!(choose_uid(&[User(1001), User(0)], 1000), Some(0));
    assert_eq!(choose_uid(&[Group(10), User(1001)], 1000), Some(1001));
    assert_eq!(choose_uid(&[Group(10), Other("x".into())], 1000), None);
    assert_eq!(choose_uid(&[], 1000), None);
}
