//! What the trusted core decides of a script host's use of the network
//! (the script API's specification, `neoteletext-common/docs/script-api.md`,
//! sections 2, 11.1 and 11.3; DEC-7268; TODO-7410, TODO-7420): whether a
//! request a script host wrote is relayed, and which tab of its
//! partition's web client it is relayed for. The decisions are here, in
//! the library, with no process and no pipe, so that a sequence of
//! reports and requests can be given to them by a test or a fuzz target
//! (rdm TODO-7439); the trusted core's program keeps the processes and
//! writes what these decide.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use protocol::fetcher::PROCESS_TAB;
use protocol::script::relay::{Relayed, STEP_DEADLINE};
use protocol::web::{PartitionToken, relay_to_client};

/// How long a script host whose contexts no longer have the network may
/// still write to its web client (TODO-7420): the time its closing
/// context has to finish, which is a step's. A page context that had the
/// network ends its requests, its event streams and its sockets as it
/// closes, and may start a `fetch` with `keepalive` in its one handling
/// of `close` (section 4.2); its decoder reports the next page's claims,
/// with no network, at the same moment, and the two arrive in no fixed
/// order. After this time a request that starts something is the breach
/// it always was, and one that only ends something is dropped, since
/// the web client has by then been told to end everything of the tab.
pub const NETWORK_DRAIN: Duration = STEP_DEADLINE;

/// What becomes of a request a script host wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// It is relayed to the web client.
    Relay,
    /// It is dropped: the end of something whose claim has gone, which
    /// the web client was already told to end.
    Drop,
    /// The script host asked for the network its scripts do not have: it
    /// is stopped.
    Stop,
}

/// Whether a script host's scripts have the network, as its decoder
/// reported and the trusted core holds it (section 11.1: the network
/// kinds are relayed only for a context the decoder reported with the
/// network).
#[derive(Clone, Copy, Debug, Default)]
pub struct NetworkClaim {
    /// Whether the scripts have the network now.
    has: bool,
    /// When they last ceased to have it.
    left: Option<Instant>,
    /// Whether the web client was told the tab closed since then.
    drained: bool,
}

impl NetworkClaim {
    /// The decoder reported whether the scripts that run have the
    /// network, at `now`.
    pub const fn reported(&mut self, has: bool, now: Instant) {
        let had = core::mem::replace(&mut self.has, has);
        if self.has {
            self.left = None;
        } else if had {
            self.left = Some(now);
            self.drained = false;
        }
    }

    /// What becomes of a request the script host wrote at `now`;
    /// `starts` is whether it begins something
    /// (`protocol::web::starts_request`) and does not end or feed it.
    #[must_use]
    pub fn asked(&self, starts: bool, now: Instant) -> Asked {
        if self.has {
            return Asked::Relay;
        }
        let draining = self.left.is_some_and(|left| now.saturating_duration_since(left) <= NETWORK_DRAIN);
        if draining {
            Asked::Relay
        } else if self.left.is_some() && !starts {
            Asked::Drop
        } else {
            Asked::Stop
        }
    }

    /// When the drain ends, while one is under way.
    #[must_use]
    pub fn drain_due(&self) -> Option<Instant> {
        self.left.filter(|_| !self.has && !self.drained).and_then(|left| left.checked_add(NETWORK_DRAIN))
    }

    /// Whether the drain has just ended at `now`: true once, when the
    /// web client is to be told the tab closed.
    pub fn drain_ended(&mut self, now: Instant) -> bool {
        let ended = !self.has && !self.drained && self.left.is_some_and(|left| now.saturating_duration_since(left) > NETWORK_DRAIN);
        if ended {
            self.drained = true;
        }
        ended
    }
}

/// The tabs of one web client: each script host it serves is a tab of
/// its own, by a number that is never the process's own
/// ([`PROCESS_TAB`]).
#[derive(Clone, Debug)]
pub struct Tabs {
    /// The channel each tab serves, by the tab's number.
    tabs: BTreeMap<u32, u32>,
    /// The last number given.
    last: u32,
}

impl Default for Tabs {
    fn default() -> Self {
        Self { tabs: BTreeMap::new(), last: PROCESS_TAB }
    }
}

impl Tabs {
    /// The tab that serves `channel`, when one does.
    #[must_use]
    pub fn of(&self, channel: u32) -> Option<u32> {
        self.tabs.iter().find(|(_, served)| **served == channel).map(|(&tab, _)| tab)
    }

    /// The channel `tab` serves, when it is open.
    #[must_use]
    pub fn channel(&self, tab: u32) -> Option<u32> {
        self.tabs.get(&tab).copied()
    }

    /// Opens a tab for `channel` and gives its number.
    pub fn open(&mut self, channel: u32) -> u32 {
        self.last = self.last.wrapping_add(1).max(PROCESS_TAB.saturating_add(1));
        self.tabs.insert(self.last, channel);
        self.last
    }

    /// Closes `tab`.
    pub fn close(&mut self, tab: u32) {
        self.tabs.remove(&tab);
    }

    /// Whether no tab is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// The channels served, as the tabs are numbered.
    pub fn channels(&self) -> impl Iterator<Item = u32> + '_ {
        self.tabs.values().copied()
    }
}

/// A request a script host wrote, as the web client is written it: for
/// `tab`, and with the partition's `token` when it starts something. No
/// payload is read (`protocol::web::relay_to_client`).
///
/// # Errors
/// Why the request is refused, in words for the developer's log.
pub fn request_for_client(tab: u32, token: &PartitionToken, request: Relayed) -> Result<Vec<u8>, String> {
    let mut written = request.into_frame();
    let mut out = Vec::new();
    relay_to_client(tab, token, &mut written, &mut out).map_err(|error| format!("the script host's request was refused: {error}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::panic, reason = "a test")]
    use super::*;

    /// Section 11.1: a script host whose scripts never had the network
    /// and asks for it is stopped; one whose claim has just gone is
    /// relayed for the time of a step; after that what ends a request is
    /// dropped and what starts one stops it; and the drain's end is told
    /// once.
    #[test]
    fn a_host_without_the_network_that_asks_for_it_is_stopped_after_the_drain() {
        let began = Instant::now();
        let mut claim = NetworkClaim::default();
        assert_eq!((claim.asked(true, began), claim.asked(false, began)), (Asked::Stop, Asked::Stop));
        claim.reported(true, began);
        assert_eq!((claim.asked(true, began), claim.drain_due()), (Asked::Relay, None));
        claim.reported(false, began);
        assert_eq!(claim.drain_due(), Some(began + NETWORK_DRAIN));
        assert_eq!(claim.asked(true, began + NETWORK_DRAIN), Asked::Relay);
        assert!(!claim.drain_ended(began + NETWORK_DRAIN));
        let after = began + NETWORK_DRAIN + Duration::from_millis(1);
        assert_eq!((claim.asked(false, after), claim.asked(true, after)), (Asked::Drop, Asked::Stop));
        assert!(claim.drain_ended(after) && !claim.drain_ended(after));
        assert_eq!(claim.drain_due(), None);
    }

    /// Section 11.3 (DEC-7268): a request a script host wrote reaches its
    /// web client for the host's own tab, as the web client's reader takes
    /// it with the partition's token.
    #[test]
    fn a_script_hosts_request_is_written_for_its_own_tab_with_the_partitions_token() {
        use protocol::script::relay::{HostOutput, read_host_output};
        use protocol::web::{PARTITION_TOKEN_BYTES, ToWebClient, read_to_client, write_request};
        let token = PartitionToken::new([7; PARTITION_TOKEN_BYTES]);
        let mut tabs = Tabs::default();
        let tab = tabs.open(3);
        let mut written = Vec::new();
        write_request(&mut written, &ToWebClient::Abort(9)).unwrap();
        let Some(HostOutput::Request(request)) = read_host_output(&mut written.as_slice()).unwrap() else { panic!("not a request") };
        assert!(!protocol::web::starts_request(request.kind()));
        let relayed = request_for_client(tab, &token, request).unwrap();
        assert_eq!(read_to_client(&mut relayed.as_slice(), &token).unwrap(), Some((tab, ToWebClient::Abort(9))));
    }

    /// A web client's tabs: each channel its own, none the process's own
    /// number, and a closed tab serves nothing.
    #[test]
    fn each_script_host_is_a_tab_of_its_own_and_a_closed_tab_serves_nothing() {
        let mut tabs = Tabs::default();
        let (first, second) = (tabs.open(7), tabs.open(9));
        assert!(first != second && first != PROCESS_TAB && second != PROCESS_TAB);
        assert_eq!((tabs.of(7), tabs.of(9), tabs.channel(first), tabs.channel(second)), (Some(first), Some(second), Some(7), Some(9)));
        tabs.close(first);
        assert_eq!((tabs.of(7), tabs.channel(first), tabs.is_empty()), (None, None, false));
        assert_eq!(tabs.channels().collect::<Vec<_>>(), vec![9]);
    }
}
