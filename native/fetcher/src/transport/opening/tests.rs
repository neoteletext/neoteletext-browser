//! A connection's opening against its deadline: a handshake that never
//! ends, and resolutions that never return.
#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects, reason = "a test")]

use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;

use super::super::cancel::Cancel;
use super::*;

/// The deadline these tests give an opening: short, so a test waits it
/// out quickly.
const SHORT: Duration = Duration::from_millis(300);
/// Long enough, against [`SHORT`], to tell an opening that waited for its
/// deadline from one that did not wait at all.
const AT_ONCE: Duration = Duration::from_millis(150);

/// An opener of the test's own, its deadline `deadline` from now, which may
/// reach this computer.
fn opener(deadline: Duration, resolving: &Arc<AtomicUsize>) -> Opener {
    let mut allowed = [None; neoteletext_web::consts::ALLOWED_LOCAL];
    allowed[0] = Some(neoteletext_web::url::IpAddress::V4([127, 0, 0, 1]));
    Opener {
        tls: Arc::new(neoteletext_transport::client_config(neoteletext_transport::mozilla_roots())),
        proxy: None,
        allowed,
        wake: Arc::new(|| {}),
        cancel: Cancel::default(),
        until: Instant::now() + deadline,
        resolving: Arc::clone(resolving),
    }
}

/// TODO-1277, finding F-63: a server that takes the connection and never
/// answers the handshake runs the opening out of its deadline, which is
/// the channel not answering in time, not a certificate refused. On the
/// code before, it was reported as a failed handshake, which the
/// certificate-error screen showed.
#[test]
fn a_handshake_that_runs_out_of_the_deadline_is_a_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (done, finished) = mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (_socket, _) = listener.accept().unwrap();
        // Says nothing, and holds the connection, until the test is done.
        let _ = finished.recv();
    });
    let Err(failure) = opener(SHORT, &Arc::new(AtomicUsize::new(0))).open("127.0.0.1", port, Security::Tls) else { panic!("it opened") };
    assert!(failure.timed_out, "the deadline ended it");
    assert_eq!(failure.certificate, None, "no certificate was refused");
    drop(done);
    server.join().unwrap();
}

/// TODO-1277, finding F-62: resolutions that never return hold at most
/// [`MAX_RESOLVING`] threads; one more fails at once as out of time, and
/// never reaches the resolver. Once those end, a name resolves again. On
/// the code before, every resolution started a thread of its own and
/// waited out its whole deadline.
#[test]
fn resolver_threads_that_never_return_are_bounded() {
    let resolving = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(AtomicUsize::new(0));
    let mut releases = Vec::new();
    let stuck = |release: mpsc::Receiver<()>, asked: &Arc<AtomicUsize>| {
        let asked = Arc::clone(asked);
        move |_: &str, _: u16| {
            asked.fetch_add(1, Ordering::SeqCst);
            let _ = release.recv();
            Ok(Vec::new())
        }
    };
    for _ in 0..MAX_RESOLVING {
        let (release, held) = mpsc::channel::<()>();
        releases.push(release);
        let until = Instant::now() + SHORT;
        assert_eq!(resolve_with("never.example", 80, until, &resolving, stuck(held, &asked)), Resolution::TimedOut);
    }
    assert_eq!(resolving.load(Ordering::SeqCst), MAX_RESOLVING, "each stuck resolution holds its thread");
    let (_release, held) = mpsc::channel::<()>();
    let started = Instant::now();
    let long = started + Duration::from_secs(60);
    assert_eq!(resolve_with("never.example", 80, long, &resolving, stuck(held, &asked)), Resolution::TimedOut);
    assert!(started.elapsed() < AT_ONCE, "refused at once, not after {:?}", started.elapsed());
    assert_eq!(asked.load(Ordering::SeqCst), MAX_RESOLVING, "the resolver was not asked again");
    drop(releases);
    let freed = Instant::now();
    while resolving.load(Ordering::SeqCst) > 0 {
        assert!(freed.elapsed() < Duration::from_secs(5), "the threads gave their places back");
        std::thread::sleep(Duration::from_millis(10));
    }
    let address: SocketAddr = "127.0.0.1:80".parse().unwrap();
    let answered = resolve_with("now.example", 80, Instant::now() + SHORT, &resolving, move |_, _| Ok(vec![address]));
    assert_eq!(answered, Resolution::Addresses(vec![address]));
}
