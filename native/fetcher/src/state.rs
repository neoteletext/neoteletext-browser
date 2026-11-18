//! Viewer state in the fetcher (neoteletext-common's `docs/viewer-state.md`
//! VS-103, VS-119, VS-202 and VS-204, DEC-6471): it holds no key and
//! signs nothing. A `STATE_CHALLENGE` its channel's push connection carries
//! goes to the trusted core with only the challenge, whether to create, and
//! a request number; the core takes the channel from this fetcher's tab,
//! builds the signed bytes and signs, and this fetcher writes the answer it
//! is given on the push connection as the input protocol's event, at once,
//! before any input message the trusted core sent meanwhile, so that it
//! goes before SELECT (VS-202). Unprompted messages (`STATE_OFFER`, and
//! `STATE_NONE` `forgotten`) go at most once in each
//! [`STATE_OFFER_INTERVAL`], the latest held meanwhile (VS-204).

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use neoteletext_input::event::{Event, Item, write};
use neoteletext_input::state::{NoneReason, STATE_PROOF_BYTES, StateProof};
use neoteletext_web::records::state::StateChallenge;
use protocol::fetcher::{FromFetcher, StateReply, ToFetcher, write_to_core};

use crate::process::CorePipe;

/// How long the fetcher waits for the trusted core's answer. The core
/// answers from memory as soon as it reads the challenge; one that has not
/// answered in this long is ending, and the channel is told the viewer
/// keeps no key for it, so its server need not wait out its own limit.
const ANSWER_LIMIT: Duration = Duration::from_secs(2);
/// VS-204 and open question Q12: an unprompted `STATE_OFFER` or `STATE_NONE`
/// goes at most once in each 10,000 ms.
pub(crate) const STATE_OFFER_INTERVAL: Duration = Duration::from_millis(10_000);
/// The longest state event written: the event byte, its type and its
/// length, then `STATE_PROOF`'s payload, the longest (VS-204).
const EVENT_HEAD: usize = 3;
const MAX_STATE_EVENT: usize = EVENT_HEAD + STATE_PROOF_BYTES;

// The key identifier the pipe carries is the engine's.
const _: () = assert!(protocol::fetcher::KEY_ID_BYTES == neoteletext_web::records::state::KEY_ID_BYTES);
const _: () = assert!(protocol::fetcher::CHALLENGE_BYTES == neoteletext_web::records::state::CHALLENGE_BYTES);

/// The fetcher's part of viewer state.
pub(crate) struct State {
    /// The trusted core's answers, read by the thread that reads its
    /// messages.
    answers: Receiver<ToFetcher>,
    next: u32,
    /// When the last unprompted message went.
    last_unprompted: Option<Instant>,
    /// The latest unprompted message held until the interval allows it.
    held: Option<Vec<u8>>,
}

/// `event`, written as the input protocol writes it.
fn written(event: Event<'_>) -> Vec<u8> {
    let mut out = [0_u8; MAX_STATE_EVENT];
    let length = write(&Item::Event(event), &mut out).unwrap_or(0);
    out.get(..length).unwrap_or(&[]).to_vec()
}

/// The answer the trusted core gave, as the push connection carries it.
fn answer_bytes(reply: &StateReply) -> Vec<u8> {
    match reply {
        StateReply::Proof { algorithm, lifetime, public_key, signature } => {
            written(Event::StateProof(StateProof { algorithm: *algorithm, lifetime: *lifetime, public_key, signature }))
        }
        StateReply::None(reason) => written(Event::StateNone(*reason)),
    }
}

impl State {
    /// The fetcher's state, its answers read from `answers`.
    pub(crate) const fn new(answers: Receiver<ToFetcher>) -> Self {
        Self { answers, next: 0, last_unprompted: None, held: None }
    }

    /// A challenge the push connection carried: asked of the trusted core,
    /// whose answer this returns to be written on the push connection at
    /// once. Without an answer in time, `STATE_NONE` `no_key`: the channel
    /// works without state.
    pub(crate) fn challenged(&mut self, output: &CorePipe, challenge: StateChallenge) -> Vec<u8> {
        self.next = self.next.wrapping_add(1);
        let request = self.next;
        let asked = FromFetcher::StateChallenged { request, challenge: challenge.challenge, create: challenge.create };
        let told = {
            let mut output = output.borrow_mut();
            write_to_core(&mut *output, &asked).is_ok() && std::io::Write::flush(&mut *output).is_ok()
        };
        let reply = if told { self.wait(request) } else { None };
        answer_bytes(&reply.unwrap_or(StateReply::None(NoneReason::NoKey)))
    }

    /// The answer to `request`; an answer to an earlier one that came late
    /// is dropped.
    fn wait(&self, request: u32) -> Option<StateReply> {
        let deadline = Instant::now().checked_add(ANSWER_LIMIT)?;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            match self.answers.recv_timeout(left).ok()? {
                ToFetcher::StateAnswer { request: answered, reply } if answered == request => return Some(reply),
                _ => {}
            }
        }
    }

    /// An unprompted message the trusted core asked for (VS-111, VS-113),
    /// held until [`State::due`] gives it: the latest replaces one held.
    pub(crate) fn hold(&mut self, message: &ToFetcher) {
        self.held = match message {
            ToFetcher::StateSendOffer => Some(written(Event::StateOffer)),
            ToFetcher::StateSendNone(reason) => Some(written(Event::StateNone(*reason))),
            _ => return,
        };
    }

    /// The held message, once the interval allows it.
    pub(crate) fn due(&mut self, now: Instant) -> Option<Vec<u8>> {
        let allowed = self.last_unprompted.is_none_or(|last| now.saturating_duration_since(last) >= STATE_OFFER_INTERVAL);
        if !allowed || self.held.is_none() {
            return None;
        }
        self.last_unprompted = Some(now);
        self.held.take()
    }

    /// When the held message may go, if one is held.
    pub(crate) fn next_due(&self) -> Option<Instant> {
        self.held.as_ref()?;
        self.last_unprompted.and_then(|last| last.checked_add(STATE_OFFER_INTERVAL)).or_else(|| Some(Instant::now()))
    }

    /// The push connection was lost: what was held was for it, and a new
    /// connection is challenged afresh (VS-207).
    pub(crate) fn lost(&mut self) {
        self.held = None;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::BufWriter;
    use std::rc::Rc;
    use std::sync::mpsc;

    /// A pipe to the trusted core whose bytes the test reads back.
    #[derive(Clone, Default)]
    struct Shared(Rc<RefCell<Vec<u8>>>);

    impl std::io::Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn output() -> (CorePipe, Shared) {
        let shared = Shared::default();
        (Rc::new(RefCell::new(BufWriter::new(Box::new(shared.clone())))), shared)
    }

    /// VS-103 and VS-119: the trusted core is told the challenge, whether to
    /// create and a request number, and nothing of any channel; its proof
    /// goes on the push connection as the input protocol's `STATE_PROOF`
    /// event, 0xFF 0x48 and 98 bytes.
    #[test]
    fn a_challenge_is_asked_of_the_core_and_its_proof_written() {
        let (answers, answered) = mpsc::channel();
        let (output, shared) = output();
        let mut state = State::new(answered);
        let reply = StateReply::Proof {
            algorithm: neoteletext_input::state::Algorithm::Ed25519,
            lifetime: neoteletext_input::state::Lifetime::Visit,
            public_key: [7; 32],
            signature: [9; 64],
        };
        // A late answer to another request first, which is dropped.
        answers.send(ToFetcher::StateAnswer { request: 99, reply: StateReply::None(NoneReason::Declined) }).unwrap();
        answers.send(ToFetcher::StateAnswer { request: 1, reply }).unwrap();
        let bytes = state.challenged(&output, StateChallenge { challenge: [5; 32], create: true });
        assert_eq!(&bytes[..3], [0xFF, 0x48, 98]);
        assert_eq!(bytes.len(), 3 + 98);
        assert_eq!(&bytes[3..5], [1, 0], "Ed25519, a visit key");
        let told = protocol::fetcher::read_from_fetcher(&mut shared.0.borrow().as_slice()).unwrap().unwrap();
        assert_eq!(told, FromFetcher::StateChallenged { request: 1, challenge: [5; 32], create: true });
    }

    /// Without the core's answer in time, the channel is told the viewer
    /// keeps no key for it: `STATE_NONE` `no_key`, 0xFF 0x49 1 0.
    #[test]
    fn no_answer_is_no_key() {
        let (answers, answered) = mpsc::channel::<ToFetcher>();
        drop(answers);
        let (output, _) = output();
        let mut state = State::new(answered);
        assert_eq!(state.challenged(&output, StateChallenge { challenge: [0; 32], create: false }), [0xFF, 0x49, 1, 0]);
    }

    /// VS-204: unprompted messages go at most once in each interval, the
    /// latest held meanwhile, and nothing held outlives the connection.
    #[test]
    fn unprompted_messages_keep_their_interval() {
        let (_answers, answered) = mpsc::channel::<ToFetcher>();
        let mut state = State::new(answered);
        let start = Instant::now();
        state.hold(&ToFetcher::StateSendOffer);
        assert_eq!(state.due(start), Some(alloc_vec(&[0xFF, 0x4A, 0])));
        state.hold(&ToFetcher::StateSendNone(NoneReason::Forgotten));
        assert_eq!(state.due(start), None, "held");
        assert!(state.next_due().is_some());
        let later = start.checked_add(STATE_OFFER_INTERVAL).unwrap();
        assert_eq!(state.due(later), Some(alloc_vec(&[0xFF, 0x49, 1, 3])), "the latest, once the interval allows");
        assert_eq!(state.due(later), None);
        state.hold(&ToFetcher::StateSendOffer);
        state.lost();
        assert_eq!(state.next_due(), None);
    }

    fn alloc_vec(bytes: &[u8]) -> Vec<u8> {
        bytes.to_vec()
    }
}
