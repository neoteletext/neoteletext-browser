//! The transport's tests: connections direct and through an HTTP or SOCKS
//! proxy, played by servers in this process, and the refusals.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::arithmetic_side_effects, reason = "a test")]

use super::*;
use neoteletext_web::port::{LinkState, Received, Security, Transport};
use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::TcpListener;
use std::time::Instant;

/// What the test's proxy does with a connection.
#[derive(Clone, Copy)]
enum Proxying {
    HttpAllows,
    /// Allows, and the host's first bytes come in the same write.
    HttpAllowsWithBytes,
    HttpRefuses,
    SocksAllows,
    SocksRefuses,
    Silent,
}

/// A proxy on this computer: it answers one connection as told, then
/// echoes what the tunnel carries.
fn proxy(proxying: Proxying) -> (Proxy, std::thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let kind = if matches!(proxying, Proxying::SocksAllows | Proxying::SocksRefuses) { "socks5" } else { "http" };
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut asked = Vec::new();
        let mut byte = [0_u8; 1];
        match proxying {
            Proxying::HttpAllows | Proxying::HttpAllowsWithBytes | Proxying::HttpRefuses => {
                while !asked.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap() == 1 {
                    asked.push(byte[0]);
                }
                let answer: &[u8] = match proxying {
                    Proxying::HttpAllows => b"HTTP/1.1 200 Connection established\r\n\r\n",
                    Proxying::HttpAllowsWithBytes => b"HTTP/1.1 200 Connection established\r\n\r\nfirst",
                    _ => b"HTTP/1.1 403 Forbidden\r\n\r\n",
                };
                socket.write_all(answer).unwrap();
            }
            Proxying::SocksAllows | Proxying::SocksRefuses => {
                let mut greeting = [0_u8; 3];
                socket.read_exact(&mut greeting).unwrap();
                asked.extend(greeting);
                socket.write_all(&[5, 0]).unwrap();
                let mut head = [0_u8; 5];
                socket.read_exact(&mut head).unwrap();
                let mut rest = vec![0_u8; usize::from(head[4]) + 2];
                socket.read_exact(&mut rest).unwrap();
                asked.extend(head);
                asked.extend(&rest);
                let reply = if matches!(proxying, Proxying::SocksAllows) { 0 } else { 5 };
                socket.write_all(&[5, reply, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
            }
            // Holds the connection and says nothing, for longer than the
            // connection may take.
            Proxying::Silent => std::thread::sleep(CONNECT_DEADLINE + Duration::from_secs(2)),
        }
        let mut echo = [0_u8; 64];
        if let Ok(count) = socket.read(&mut echo) {
            let _ = socket.write_all(&echo[..count]);
        }
        asked
    });
    (Proxy::parse(&format!("{kind}://127.0.0.1:{port}")).unwrap(), handle)
}

/// No local address may be reached directly.
const NO_LOCAL: AllowedLocal = [None; ALLOWED_LOCAL];

/// The test servers' address, allowed as a channel's arguments allow one.
fn this_computer() -> AllowedLocal {
    let mut allowed = NO_LOCAL;
    allowed[0] = Some(IpAddress::V4([127, 0, 0, 1]));
    allowed
}

fn tcp(proxy: Proxy) -> Tcp {
    Tcp::new(Arc::new(|| {}), Some(proxy), NO_LOCAL)
}

/// The connection's state once it is no longer opening, within the
/// connect limit and a little.
fn settled(tcp: &mut Tcp, connection: ConnectionId) -> LinkState {
    let started = Instant::now();
    loop {
        match tcp.state(connection) {
            LinkState::Connecting if started.elapsed() < CONNECT_DEADLINE + Duration::from_secs(1) => std::thread::sleep(Duration::from_millis(1)),
            state => return state,
        }
    }
}

/// Reads what comes back through the tunnel, within a second.
fn echoed(tcp: &mut Tcp) -> Vec<u8> {
    let started = Instant::now();
    let mut buffer = [0_u8; 64];
    while started.elapsed() < Duration::from_secs(1) {
        if let Ok(Received::Data(count)) = tcp.receive(ConnectionId::Fetch, &mut buffer) {
            return buffer[..count].to_vec();
        }
        std::thread::yield_now();
    }
    Vec::new()
}

/// Requirement 19.10: an HTTP proxy is asked by CONNECT for the host
/// and port, and the connection is its tunnel.
#[test]
fn a_connection_goes_through_an_http_proxy() {
    let (proxy, server) = proxy(Proxying::HttpAllows);
    let mut tcp = tcp(proxy);
    tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Open(_)));
    assert!(!tcp.take_proxy_failure());
    tcp.send(ConnectionId::Fetch, b"through").unwrap();
    assert_eq!(echoed(&mut tcp), b"through");
    let asked = String::from_utf8(server.join().unwrap()).unwrap();
    assert!(asked.starts_with("CONNECT news.example:80 HTTP/1.1\r\n"), "{asked}");
    assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Open(IpAddress::V4([127, 0, 0, 1]))), "the peer is the proxy");
}

/// `certificate-errors.md` section 3: each of the TLS library's
/// certificate errors is told apart, and any other handshake failure is
/// one kind.
#[test]
fn certificate_failures_are_told_apart() {
    use rustls::CertificateError as Certificate;
    let of = |error: rustls::Error| certificate_problem(&std::io::Error::new(std::io::ErrorKind::InvalidData, error));
    assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::Expired)), CertificateProblem::Expired);
    assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::NotValidYet)), CertificateProblem::NotYetValid);
    assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::UnknownIssuer)), CertificateProblem::UnknownIssuer);
    assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::NotValidForName)), CertificateProblem::WrongHost);
    assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::Revoked)), CertificateProblem::Revoked);
    assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::BadSignature)), CertificateProblem::Invalid);
    assert_eq!(of(rustls::Error::DecryptError), CertificateProblem::Handshake);
    assert_eq!(certificate_problem(&std::io::Error::from(std::io::ErrorKind::ConnectionReset)), CertificateProblem::Handshake);
}

/// A server that does not speak TLS fails the handshake: the connection
/// fails and the refusal is reported once, as a failed handshake.
#[test]
fn a_failed_handshake_is_reported_once() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\nnot a TLS server");
    });
    let mut tcp = Tcp::new(Arc::new(|| {}), None, this_computer());
    tcp.connect(ConnectionId::Fetch, "127.0.0.1", port, Security::Tls).unwrap();
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed));
    assert_eq!(tcp.take_certificate_refusal(), Some(CertificateProblem::Handshake));
    assert_eq!(tcp.take_certificate_refusal(), None, "once");
    assert!(!tcp.take_proxy_failure());
    server.join().unwrap();
}

/// The engine's E.102: the host's bytes that arrive with the proxy's
/// answer are the connection's first, not lost.
#[test]
fn the_hosts_first_bytes_after_the_proxys_answer_are_kept() {
    let (proxy, server) = proxy(Proxying::HttpAllowsWithBytes);
    let mut tcp = tcp(proxy);
    tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Open(_)));
    assert_eq!(echoed(&mut tcp), b"first");
    tcp.send(ConnectionId::Fetch, b"then").unwrap();
    assert_eq!(echoed(&mut tcp), b"then");
    server.join().unwrap();
}

/// Requirement 19.10: a SOCKS5 proxy, no authentication, given the
/// host's name to resolve.
#[test]
fn a_connection_goes_through_a_socks5_proxy() {
    let (proxy, server) = proxy(Proxying::SocksAllows);
    let mut tcp = tcp(proxy);
    tcp.connect(ConnectionId::Fetch, "news.example", 443, Security::Plain).unwrap();
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Open(_)));
    tcp.send(ConnectionId::Fetch, b"socks").unwrap();
    assert_eq!(echoed(&mut tcp), b"socks");
    let asked = server.join().unwrap();
    let mut expected = vec![5, 1, 0, 5, 1, 0, 3, 12];
    expected.extend(b"news.example");
    expected.extend(443_u16.to_be_bytes());
    assert_eq!(asked, expected);
}

/// Requirement 19.10: a proxy that refuses fails the connection, and
/// the fetcher hears it was the proxy; nothing goes direct.
#[test]
fn a_refusal_fails_the_connection() {
    for proxying in [Proxying::HttpRefuses, Proxying::SocksRefuses] {
        let (proxy, server) = proxy(proxying);
        let mut tcp = tcp(proxy);
        tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
        assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed));
        assert!(tcp.take_proxy_failure());
        assert!(!tcp.take_proxy_failure(), "said once");
        drop(server.join());
    }
    // No proxy listening at all is the proxy's failure too.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut tcp = tcp(Proxy::parse(&format!("http://127.0.0.1:{port}")).unwrap());
    tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed));
    assert!(tcp.take_proxy_failure());
}

/// Requirement 19.10: a proxy that does not answer fails the connection
/// within the connect limit; and the loop is never held while it is
/// waited on (REQ-INT-135): connecting returns at once, and the other
/// connections go on. Before this, a silent proxy held the fetcher's
/// one loop for the whole ten seconds.
#[test]
fn a_silent_proxy_fails_in_time_and_holds_nothing_up() {
    let (proxy, server) = proxy(Proxying::Silent);
    let mut tcp = tcp(proxy);
    let started = Instant::now();
    tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
    assert!(started.elapsed() < Duration::from_millis(100), "connecting returned at once: {:?}", started.elapsed());
    assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Connecting));
    assert_eq!(tcp.receive(ConnectionId::Fetch, &mut [0; 8]), Ok(Received::Nothing), "nothing yet, and no waiting");
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed));
    assert!(started.elapsed() < CONNECT_DEADLINE + Duration::from_secs(1), "{:?}", started.elapsed());
    assert!(tcp.take_proxy_failure());
    drop(server.join());
}

/// Requirement 5.8: the TLS handshake, now on the connection's own
/// thread, completes against a real host, and the connection carries
/// a request and its answer. It needs the internet, so it runs only
/// when asked: `cargo test -- --ignored`.
#[test]
#[ignore = "needs the internet"]
fn a_tls_connection_opens_on_its_thread_and_answers() {
    let mut tcp = Tcp::new(Arc::new(|| {}), None, NO_LOCAL);
    tcp.connect(ConnectionId::Fetch, "example.com", 443, Security::Tls).unwrap();
    assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Connecting | LinkState::Open(_)));
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Open(_)));
    tcp.send(ConnectionId::Fetch, b"HEAD / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n").unwrap();
    let started = Instant::now();
    let mut answer = Vec::new();
    let mut buffer = [0_u8; 1024];
    while started.elapsed() < Duration::from_secs(10) && !answer.starts_with(b"HTTP/1.1 ") {
        match tcp.receive(ConnectionId::Fetch, &mut buffer) {
            Ok(Received::Data(count)) => answer.extend_from_slice(&buffer[..count]),
            Ok(Received::Closed) => break,
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    assert!(answer.starts_with(b"HTTP/1.1 "), "{}", String::from_utf8_lossy(&answer));
}

/// A connection closed while it opens is dropped by its thread, and
/// a later connection on the same slot is its own.
#[test]
fn a_connection_closed_while_opening_is_let_go() {
    let (proxy, _server) = proxy(Proxying::Silent);
    let mut tcp = tcp(proxy);
    tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
    tcp.close(ConnectionId::Fetch);
    assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Closed));
    assert!(!tcp.take_proxy_failure(), "a closed connection's failure is nobody's");
}

/// Finding F-62 (REQ-SEC-006): a name that resolves to this computer
/// opens no connection at all, so a server reached by such a name never
/// sees the handshake or a TLS hello naming whatever host the channel
/// chose; the engine's own check of the peer came only after both. A
/// local address named in the arguments is reached as before.
#[test]
fn a_name_resolving_to_this_computer_opens_no_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut tcp = Tcp::new(Arc::new(|| {}), None, NO_LOCAL);
    tcp.connect(ConnectionId::Fetch, "localhost", port, Security::Tls).unwrap();
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed));
    assert!(matches!(listener.accept(), Err(error) if error.kind() == IoErrorKind::WouldBlock), "no connection reached the server");
    assert!(tcp.take_local_refusal(), "the refusal is told");
    assert!(!tcp.take_local_refusal(), "once");
    assert!(!tcp.take_proxy_failure());
    assert_eq!(tcp.take_certificate_refusal(), None, "no handshake was tried");

    let mut tcp = Tcp::new(Arc::new(|| {}), None, this_computer());
    tcp.connect(ConnectionId::Fetch, "localhost", port, Security::Plain).unwrap();
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Open(IpAddress::V4([127, 0, 0, 1]))));
    assert!(!tcp.take_local_refusal());
}

/// How often the dripping proxy sends its next byte: well inside the
/// connect limit, so a timeout per read never fires.
const DRIP: Duration = Duration::from_millis(500);

/// A proxy on this computer that takes the request for a tunnel, then
/// answers one byte at a time, never finishing, until the connect limit
/// and more have passed or the browser's end of the connection closes;
/// it returns how long after accepting it saw that end, if it did.
fn dripping_proxy() -> (Proxy, std::thread::JoinHandle<Option<Duration>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let accepted = Instant::now();
        socket.set_read_timeout(Some(DRIP)).unwrap();
        let mut byte = [0_u8; 1];
        let mut asked = Vec::new();
        while !asked.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap() == 1 {
            asked.push(byte[0]);
        }
        let answer = b"HTTP/1.1 200 Connection established, slowly".iter().cycle();
        for &next in answer {
            if accepted.elapsed() > CONNECT_DEADLINE + Duration::from_secs(3) || socket.write_all(&[next]).is_err() {
                return None;
            }
            match socket.read(&mut byte) {
                Ok(0) => return Some(accepted.elapsed()),
                Ok(_) => {}
                Err(error) if matches!(error.kind(), IoErrorKind::WouldBlock | IoErrorKind::TimedOut) => {}
                Err(_) => return Some(accepted.elapsed()),
            }
        }
        None
    });
    (Proxy::parse(&format!("http://127.0.0.1:{port}")).unwrap(), handle)
}

/// Finding F-63: the whole opening of a connection is held to one
/// deadline, the engine's connect limit, however its server or proxy
/// spaces its bytes. Before this, every read had the limit afresh, so a
/// proxy answering a byte at a time held the connection's thread, its
/// socket and its buffer for as long as it went on.
#[test]
fn a_proxy_answering_a_byte_at_a_time_fails_within_the_deadline() {
    let (proxy, server) = dripping_proxy();
    let mut tcp = tcp(proxy);
    let started = Instant::now();
    tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
    assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed), "still opening after {:?}", started.elapsed());
    assert!(started.elapsed() < CONNECT_DEADLINE + Duration::from_secs(1), "{:?}", started.elapsed());
    assert!(tcp.take_proxy_failure());
    drop(server.join());
}

/// Finding F-63: closing a connection that is still opening shuts its
/// socket, so its thread ends at once rather than when the opening
/// would have ended. Before this, `close` reached only an open
/// connection, and the thread went on with the server or proxy.
#[test]
fn closing_a_connection_while_it_opens_ends_its_thread() {
    let (proxy, server) = dripping_proxy();
    let wake: Wake = Arc::new(|| {});
    let mut tcp = Tcp::new(Arc::clone(&wake), Some(proxy), NO_LOCAL);
    tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
    // Long enough for the request for a tunnel to be sent and answered in part.
    std::thread::sleep(DRIP * 2);
    assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Connecting));
    let closed = Instant::now();
    tcp.close(ConnectionId::Fetch);
    // The thread holds its own handle to the wake until it ends.
    while Arc::strong_count(&wake) > 2 && closed.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(Arc::strong_count(&wake), 2, "the connection's thread ended: {:?} after the close", closed.elapsed());
    assert!(closed.elapsed() < Duration::from_secs(1), "{:?}", closed.elapsed());
    assert!(server.join().unwrap().is_some(), "the proxy saw the connection end");
}
