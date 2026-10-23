//! The keyboard process ends when the trusted core closes its input, even
//! while it is still connecting to the relay (finding F-26): the core then
//! never waits on a helper held by a relay that does not answer.

#![cfg(feature = "phone-keyboard")]
#![allow(clippy::unwrap_used, reason = "a test")]

use std::io::Read;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How soon the process must end once its input closes (the fix plan's
/// F-26 test).
const END_WITHIN: Duration = Duration::from_secs(1);
/// The longest the process may take to start and reach the relay: a
/// ceiling, not a wait. On macOS the first start of a newly built program
/// is held while the system scans it, measured at 19.6 s on 24 October
/// 2026, against 2 ms after that.
const CONNECT_WITHIN: Duration = Duration::from_secs(60);

#[test]
fn closing_its_input_ends_it_while_the_relay_never_answers() {
    // A relay that takes the connection and never answers it: it only
    // reports when the keyboard process connects, and when the connection
    // closes, which it does when the process ends.
    let relay = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = relay.local_addr().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_neoteletext-keyboard"))
        .arg(format!("http://{address}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (said, heard) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut connection, _)) = relay.accept() else { return };
        let _ = said.send("connected");
        let mut request = [0_u8; 4096];
        while connection.read(&mut request).is_ok_and(|read| read > 0) {}
        let _ = said.send("closed");
    });
    let reached = heard.recv_timeout(CONNECT_WITHIN);
    if reached != Ok("connected") {
        let _ = child.kill();
        panic!("the keyboard process never reached the relay: {reached:?}");
    }
    let closed = Instant::now();
    drop(child.stdin.take());
    let ended = heard.recv_timeout(END_WITHIN);
    if ended != Ok("closed") {
        let _ = child.kill();
        let _ = child.wait();
        panic!("still running {:?} after its input closed", closed.elapsed());
    }
    assert!(child.wait().unwrap().success(), "it ends cleanly");
}
