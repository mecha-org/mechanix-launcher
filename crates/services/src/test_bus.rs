//! A system bus for tests: the far end of a socket pair that has only done
//! the D-Bus handshake. Tests raise replies and signals on the app itself,
//! and may read what the app sent from the far end.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use mecha_wayland::prelude::*;

/// An app with `RingModule` and a system bus, and the bus's far end, past
/// the app's handshake (its `Hello` is the next thing to read there).
pub(crate) fn system_bus() -> (App, UnixStream) {
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
    (app, peer)
}
