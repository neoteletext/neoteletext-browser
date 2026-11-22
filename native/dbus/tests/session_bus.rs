//! The client against a real bus: a private `dbus-daemon` of the session
//! kind, started for the test in its own folder, so that no desktop's bus
//! is touched.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "a test")]

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};

use neoteletext_dbus::{Connection, Error, Kind, Message, Value, parse_addresses};

/// The longest path a local socket may be bound at on Linux and macOS:
/// `sun_path` of `sockaddr_un` is 108 bytes on Linux and 104 on macOS, with
/// its terminating zero (`unix(7)`), so 103 holds on both.
const MOST_SOCKET_PATH: usize = 103;
/// How many names are tried for a bus's folder before the test gives up.
const FOLDER_NAMES_TRIED: u32 = 1_000;

/// Where a test's bus has its folder: the user's own runtime folder when
/// the session names one that can be written, and otherwise the temporary
/// folder the environment names (`TMPDIR`). Never the build folder, whose
/// path is as long as wherever the workspace was checked out, where a
/// socket's may be at most [`MOST_SOCKET_PATH`] bytes (TODO-7446).
fn short_base() -> std::path::PathBuf {
    let usable = |folder: &std::path::PathBuf| {
        let trial = folder.join(format!("nt-dbus-{}-trial", std::process::id()));
        let made = std::fs::create_dir(&trial).is_ok();
        let _ = std::fs::remove_dir(&trial);
        made
    };
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|folder| folder.is_absolute() && folder.is_dir() && usable(folder))
        .unwrap_or_else(std::env::temp_dir)
}

/// A new folder under [`short_base`], for its owner alone, removed when
/// dropped.
struct Folder(std::path::PathBuf);

impl Folder {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt as _;
        static MADE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let base = short_base();
        (0..FOLDER_NAMES_TRIED)
            .find_map(|_| {
                let count = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let folder = base.join(format!("nt-dbus-{}-{count}", std::process::id()));
                match std::fs::DirBuilder::new().mode(0o700).create(&folder) {
                    Ok(()) => Some(Self(folder)),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => panic!("cannot make a bus's folder {}: {error}", folder.display()),
                }
            })
            .unwrap_or_else(|| panic!("every name tried for a bus's folder in {} is taken", base.display()))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A private session bus, ended when dropped, and its folder removed
/// after it.
struct Bus {
    daemon: Child,
    address: String,
    _folder: Folder,
}

impl Bus {
    fn start() -> Self {
        let folder = Folder::new();
        let socket = folder.0.join("bus");
        let length = socket.as_os_str().len();
        assert!(
            length <= MOST_SOCKET_PATH,
            "the bus's socket {} would have a path of {length} bytes, and a local socket's may be at most {MOST_SOCKET_PATH}: \
             run the test with XDG_RUNTIME_DIR or TMPDIR naming a folder with a shorter path",
            socket.display()
        );
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .arg(format!("--address=unix:path={}", socket.display()))
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon, which this test needs");
        // The daemon prints its address once it listens.
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap()).read_line(&mut address).unwrap();
        Self { daemon, address: address.trim().to_owned(), _folder: folder }
    }

    fn connect(&self) -> Connection {
        Connection::connect(parse_addresses(&self.address).unwrap().first().unwrap()).unwrap()
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

#[test]
#[ignore = "needs dbus-daemon (Debian and Ubuntu's dbus-daemon package), so it runs by hand: cargo test -p neoteletext-dbus -- --ignored"]
fn the_client_authenticates_calls_and_hears_a_signal() {
    let bus = Bus::start();
    let mut first = bus.connect();
    let mut second = bus.connect();
    assert!(first.unique_name().starts_with(':') && first.unique_name() != second.unique_name());
    assert_eq!(first.name_owner("org.freedesktop.DBus").unwrap(), "org.freedesktop.DBus");
    // An error reply comes back as the bus's error.
    let missing = first.name_owner("org.example.Nobody");
    assert!(matches!(&missing, Err(Error::Remote { name, .. }) if name == "org.freedesktop.DBus.Error.NameHasNoOwner"), "{missing:?}");
    // A signal the first asked for, sent by the second, with a secret's
    // shape of body.
    first.add_match("type='signal',interface='org.example.Test',member='Ping'").unwrap();
    let body = vec![Value::Struct(vec![
        Value::ObjectPath("/a/b".into()),
        Value::Bytes(Vec::new()),
        Value::Bytes(vec![7; 32]),
        Value::String("application/octet-stream".into()),
    ])];
    let mut signal = Message::method_call("org.freedesktop.DBus", "/org/example", "org.example.Test", "Ping", body.clone());
    signal.kind = Kind::Signal;
    signal.destination = None;
    second.call_without_reply(signal).unwrap();
    let sender = second.unique_name().to_owned();
    assert_eq!(first.wait_for_signal(&sender, "/org/example", "org.example.Test", "Ping").unwrap(), body);
}

#[test]
#[ignore = "needs dbus-daemon (Debian and Ubuntu's dbus-daemon package), so it runs by hand: cargo test -p neoteletext-dbus -- --ignored"]
fn a_signal_from_another_sender_is_not_taken() {
    let bus = Bus::start();
    let mut listener = bus.connect();
    let mut impostor = bus.connect();
    let mut genuine = bus.connect();
    listener.add_match("type='signal',interface='org.example.Test'").unwrap();
    let signal = |text: &str| {
        let mut signal = Message::method_call("org.freedesktop.DBus", "/org/example", "org.example.Test", "Response", vec![Value::String(text.into())]);
        signal.kind = Kind::Signal;
        signal.destination = None;
        signal
    };
    impostor.call_without_reply(signal("forged")).unwrap();
    genuine.call_without_reply(signal("genuine")).unwrap();
    let sender = genuine.unique_name().to_owned();
    assert_eq!(listener.wait_for_signal(&sender, "/org/example", "org.example.Test", "Response").unwrap(), vec![Value::String("genuine".into())]);
}

/// A signal of `member` on `/org/example`, to `destination` if one is given.
fn signal_to(destination: Option<&str>, member: &str, text: &str) -> Message {
    let mut signal = Message::method_call("org.freedesktop.DBus", "/org/example", "org.example.Test", member, vec![Value::String(text.into())]);
    signal.kind = Kind::Signal;
    signal.destination = destination.map(str::to_owned);
    signal
}

/// The signals a connection sent reach their destination before the reply
/// to its next call reaches it: the bus handles one connection's messages
/// in order, so once that reply is in, the signals are in the destination's
/// socket, ahead of any reply the destination is sent later.
fn flush(connection: &mut Connection) {
    connection.name_owner("org.freedesktop.DBus").unwrap();
}

/// TODO-7173 finding A2: 33 unasked-for signals, sent straight to another
/// connection's unique name (which the session bus allows any connection),
/// failed that connection's next call with `TooManySignals`; and finding
/// A1: such signals were kept, up to 32 of them, whatever their size.
#[test]
#[ignore = "needs dbus-daemon (Debian and Ubuntu's dbus-daemon package), so it runs by hand: cargo test -p neoteletext-dbus -- --ignored"]
fn unasked_for_signals_neither_fail_a_call_nor_are_kept() {
    let bus = Bus::start();
    let mut victim = bus.connect();
    let mut sender = bus.connect();
    let flood = neoteletext_dbus::MAX_WAITING_SIGNALS + 8;
    for number in 0..flood {
        sender.call_without_reply(signal_to(Some(victim.unique_name()), "Flood", &number.to_string())).unwrap();
    }
    flush(&mut sender);
    assert_eq!(victim.name_owner("org.freedesktop.DBus").unwrap(), "org.freedesktop.DBus");
    assert_eq!(victim.waiting_signals(), 0);
}

/// The signal a wait expects is kept when it comes during a call, and the
/// flood that comes with it is not.
#[test]
#[ignore = "needs dbus-daemon (Debian and Ubuntu's dbus-daemon package), so it runs by hand: cargo test -p neoteletext-dbus -- --ignored"]
fn an_expected_signal_during_a_call_is_kept_for_its_wait() {
    let bus = Bus::start();
    let mut listener = bus.connect();
    let mut service = bus.connect();
    let mut impostor = bus.connect();
    let owner = service.unique_name().to_owned();
    listener.expect_signal(&owner, "/org/example", "org.example.Test", "Response").unwrap();
    for _ in 0..neoteletext_dbus::MAX_WAITING_SIGNALS + 8 {
        impostor.call_without_reply(signal_to(Some(listener.unique_name()), "Response", "forged")).unwrap();
    }
    flush(&mut impostor);
    service.call_without_reply(signal_to(None, "Response", "genuine")).unwrap();
    flush(&mut service);
    listener.name_owner("org.freedesktop.DBus").unwrap();
    assert_eq!(listener.waiting_signals(), 1);
    assert_eq!(listener.wait_for_signal(&owner, "/org/example", "org.example.Test", "Response").unwrap(), vec![Value::String("genuine".into())]);
    assert_eq!(listener.waiting_signals(), 0);
}
