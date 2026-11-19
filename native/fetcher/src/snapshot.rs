//! The fetcher's snapshot mode (neoteletext-common's `docs/tab-thumbnails.md`
//! section 9, REQ-6884): one fresh process fetches one snapshot as a
//! stranger would. It is given an address, a page, a sub-code and the
//! transport's rules, and nothing of the viewer; it runs the engine's agent
//! in its snapshot mode (the engine's REQ-6880), which sends no viewer's
//! input, answers no viewer-state challenge and sends no cookie, referrer,
//! authorization or conditional header; and it speaks only
//! `protocol::snapshot`, whose messages have no field for anything of the
//! viewer. Nothing here uses the viewer-state code of `state.rs`.

use std::cell::RefCell;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant as Clock};

use neoteletext_core::types::{Instant, PageAddress, SubCode};
use neoteletext_web::agent::{Event, Failure, Input, Output, SnapshotEnd as AgentEnd, SnapshotTarget};
use neoteletext_web::http::HttpError;
use neoteletext_web::port::{ConnectionId, NetOutput, Pump, Security};
use neoteletext_web::proxy::Proxy;
use protocol::snapshot::{FromSnapshot, MAX_SNAPSHOT_BYTES, SnapshotEnd, SnapshotFailure, read_to_fetcher, write_to_core};

use crate::agent::Random;
use crate::transport::Tcp;

// The store keeps what the fetcher keeps: the largest sub-page file with a
// picture (docs/tab-thumbnails.md section 11).
const _: () = assert!(MAX_SNAPSHOT_BYTES == crate::pictured::MAX_PICTURED_SUBPAGE_FILE, "a snapshot is at most a pictured sub-page file");
/// Bytes passed to the agent at a time.
const READ_CHUNK: usize = 16 * 1024;
/// HTTP's status codes (RFC 9110 section 15): a DENIED record's status
/// outside them is no refusal the trusted core can name.
const STATUS_CODES: core::ops::RangeInclusive<u16> = 100..=599;

/// What the loop is woken by.
enum Wake {
    /// A socket has bytes, or has ended.
    Network,
    /// The trusted core said Close, closed the pipe, or broke it.
    Ended,
}

/// What the agent asked for, kept until the loop does it.
enum Action {
    Connect { connection: ConnectionId, host: String, port: u16, security: Security },
    Send(ConnectionId, Vec<u8>),
    Close(ConnectionId),
    Timer(Instant),
    Bytes(Vec<u8>),
    Ended(SnapshotEnd),
}

/// Fetches the snapshot of `page`'s `sub` (3F7F for the page as a whole)
/// of the channel at `channel`, under the address rules of `allowed` and
/// `proxy`, and tells the trusted core the answer's bytes and how it
/// ended, on the process's pipes.
///
/// # Errors
/// When writing to the trusted core fails, or the place is no display page
/// and sub-code.
pub fn run(channel: &str, page: u16, sub: u16, allowed: &[IpAddr], proxy: Option<Proxy>) -> Result<(), String> {
    let target = SnapshotTarget { page: PageAddress::from_mpp(page).ok_or("snapshot page")?, sub: SubCode::new(sub) };
    let config = snapshot_config(allowed, proxy.is_some(), target);
    let allowed_local = config.allowed_local;
    run_with(channel, config, |wake| Tcp::new(wake, proxy, allowed_local), Box::new(io::stdin()), Box::new(io::stdout()))
}

/// The agent's settings for one snapshot: a tab's transport rules, the
/// agent's snapshot mode, and a file's body bounded at
/// [`MAX_SNAPSHOT_BYTES`], so that a page file past the bound is refused
/// by the agent as it arrives and told as `TooLarge` (REQ-6884 item 4),
/// rather than taken whole up to a page file's larger bound.
pub(crate) fn snapshot_config(allowed: &[IpAddr], proxied: bool, target: SnapshotTarget) -> neoteletext_web::agent::Config {
    let mut config = crate::process::agent_config(allowed, proxied);
    config.snapshot = Some(target);
    config.page_limit = MAX_SNAPSHOT_BYTES;
    config
}

/// [`run`] with the agent's settings, the transport and the pipes, which a
/// test replaces.
pub(crate) fn run_with(
    channel: &str,
    config: neoteletext_web::agent::Config,
    transport: impl FnOnce(crate::transport::Wake) -> Tcp,
    input: Box<dyn Read + Send>,
    output: Box<dyn Write>,
) -> Result<(), String> {
    let output = RefCell::new(BufWriter::new(output));
    let send = |message: &FromSnapshot| write_to_core(&mut *output.borrow_mut(), message).map_err(|error| error.to_string());
    send(&FromSnapshot::Confined)?;
    output.borrow_mut().flush().map_err(|error| error.to_string())?;
    let mut storage = crate::pictured::Storage::new();
    let mut agent = Box::new(neoteletext_web::agent::UserAgent::new(config, storage.buffers(), Random::new()));
    let (wakes, woken) = mpsc::channel::<Wake>();
    let network = wakes.clone();
    let mut pump = Pump::new(transport(Arc::new(move || {
        let _ = network.send(Wake::Network);
    })));
    // The trusted core's one message is Close; anything else, or the pipe
    // ending, ends the fetch the same way.
    std::thread::spawn(move || {
        let _ = read_to_fetcher(&mut BufReader::new(input));
        let _ = wakes.send(Wake::Ended);
    });
    let start = Clock::now();
    let clock = || Instant(u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX));
    let mut buffer = vec![0_u8; READ_CHUNK];
    let mut next_tick: Option<Instant> = None;
    let mut kept = 0_usize;
    let mut actions = Vec::new();
    agent.handle(clock(), Input::OpenChannel(channel), &mut |out| collect(out, &mut actions));
    loop {
        let mut busy = !actions.is_empty();
        for action in core::mem::take(&mut actions) {
            match action {
                Action::Connect { connection, host, port, security } => pump.execute(NetOutput::Connect { connection, host: &host, port, security }),
                Action::Send(connection, bytes) => pump.execute(NetOutput::Send(connection, &bytes)),
                Action::Close(connection) => pump.execute(NetOutput::Close(connection)),
                Action::Timer(at) => next_tick = Some(next_tick.map_or(at, |current| current.min(at))),
                Action::Bytes(bytes) => {
                    kept = kept.saturating_add(bytes.len());
                    if kept > MAX_SNAPSHOT_BYTES {
                        return end(&send, &output, SnapshotEnd::TooLarge);
                    }
                    for piece in bytes.chunks(protocol::MAX_PAYLOAD) {
                        send(&FromSnapshot::Piece(piece.to_vec()))?;
                    }
                }
                Action::Ended(how) => return end(&send, &output, how),
            }
        }
        for connection in [ConnectionId::Fetch, ConnectionId::Push] {
            if let Some(input) = pump.poll(connection, &mut buffer) {
                busy = true;
                agent.handle(clock(), Input::Net(input), &mut |out| collect(out, &mut actions));
            }
        }
        if next_tick.is_some_and(|at| at <= clock()) {
            next_tick = None;
            busy = true;
            agent.handle(clock(), Input::Tick, &mut |out| collect(out, &mut actions));
        }
        output.borrow_mut().flush().map_err(|error| error.to_string())?;
        let wake = if busy {
            woken.try_recv().ok()
        } else if let Some(at) = next_tick {
            match woken.recv_timeout(Duration::from_micros(at.0.saturating_sub(clock().0))) {
                Ok(wake) => Some(wake),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => Some(Wake::Ended),
            }
        } else {
            Some(woken.recv().unwrap_or(Wake::Ended))
        };
        if matches!(wake, Some(Wake::Ended)) {
            return Ok(());
        }
    }
}

/// Tells the trusted core how the fetch ended, and stops.
fn end(send: &dyn Fn(&FromSnapshot) -> Result<(), String>, output: &RefCell<BufWriter<Box<dyn Write>>>, how: SnapshotEnd) -> Result<(), String> {
    send(&FromSnapshot::Ended(how))?;
    output.borrow_mut().flush().map_err(|error| error.to_string())
}

/// What the agent's output asks of the loop: the network, its timer, the
/// answer's bytes, and its end. Every other event of the agent is dropped:
/// a snapshot has nothing else to tell.
fn collect(output: Output<'_>, actions: &mut Vec<Action>) {
    if let Some(net) = output.net() {
        actions.push(match net {
            NetOutput::Connect { connection, host, port, security } => Action::Connect { connection, host: host.to_owned(), port, security },
            NetOutput::Send(connection, bytes) => Action::Send(connection, bytes.to_vec()),
            NetOutput::Close(connection) => Action::Close(connection),
        });
        return;
    }
    match output {
        Output::PageBytes(page) => actions.push(Action::Bytes(page.bytes.to_vec())),
        Output::Timer { at } => actions.push(Action::Timer(at)),
        Output::Event(Event::SnapshotEnded(how)) => actions.push(Action::Ended(ended(how))),
        _ => {}
    }
}

/// The agent's end as the trusted core is told it, by kind alone.
fn ended(how: AgentEnd) -> SnapshotEnd {
    match how {
        AgentEnd::Answered => SnapshotEnd::Answered,
        AgentEnd::Denied(status) if STATUS_CODES.contains(&status) => SnapshotEnd::Denied(status),
        AgentEnd::Denied(_) => SnapshotEnd::Failed(SnapshotFailure::Protocol),
        AgentEnd::NotOffered => SnapshotEnd::NotOffered,
        // The body passed the bound snapshot_config gave the agent.
        AgentEnd::Failed(Failure::Http(HttpError::BodyTooLarge)) => SnapshotEnd::TooLarge,
        AgentEnd::Failed(failure) => SnapshotEnd::Failed(failure_kind(failure)),
    }
}

const fn failure_kind(failure: Failure) -> SnapshotFailure {
    match failure {
        Failure::Network => SnapshotFailure::Network,
        Failure::Timeout => SnapshotFailure::Timeout,
        Failure::Address(_) | Failure::ChannelAddress(_) | Failure::Blocked | Failure::Downgrade | Failure::TlsUnavailable => SnapshotFailure::Address,
        Failure::Status(_) => SnapshotFailure::Status,
        Failure::Http(_) | Failure::Content | Failure::MediaType(_) | Failure::BadRedirect => SnapshotFailure::Protocol,
        _ => SnapshotFailure::Other,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects,
    reason = "a test's servers and frames, short and well formed"
)]
mod tests;
