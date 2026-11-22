//! The trusted core's hold on a script host's use of the network
//! (script-api.md sections 2, 11.1 and 11.3; DEC-7268; TODO-7410,
//! TODO-7420; `neoteletext::script_network`, opened by TODO-7439): any
//! sequence of a decoder's reports of the network claim, the clock's
//! steps and a script host's output, which is untrusted. The input's
//! tail is the script host's bytes, read as the trusted core reads them
//! (`read_host_output`), and each request among them is judged against
//! the claim and relayed for a tab of the partition. Whatever the
//! sequence:
//! - a script host whose scripts never had the network is stopped at
//!   any request;
//! - while the scripts have it, every request is relayed;
//! - after the drain, a request that starts something stops the host,
//!   and only one that ends or feeds something is dropped;
//! - a request relayed that the web client reads is for its tab (the
//!   payload is the web client's to judge: the core reads none).
#![no_main]

use std::time::{Duration, Instant};

use libfuzzer_sys::fuzz_target;
use neoteletext::script_network::{Asked, NETWORK_DRAIN, NetworkClaim, Tabs, request_for_client};
use protocol::script::relay::{HostOutput, read_host_output};
use protocol::web::{PARTITION_TOKEN_BYTES, PartitionToken, read_to_client, starts_request};

/// The most steps of the claim one input gives.
const MAX_STEPS: usize = 64;
/// The most requests read of the script host's bytes.
const MAX_REQUESTS: usize = 64;

/// The claim by the rule's own words: whether the scripts have the
/// network, whether they ever had it, and when they last ceased to.
#[derive(Default)]
struct Model {
    has: bool,
    left: Option<Instant>,
}

impl Model {
    fn expected(&self, starts: bool, now: Instant) -> Asked {
        match self.left {
            _ if self.has => Asked::Relay,
            None => Asked::Stop,
            Some(left) if now.saturating_duration_since(left) <= NETWORK_DRAIN => Asked::Relay,
            Some(_) if starts => Asked::Stop,
            Some(_) => Asked::Drop,
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&steps, rest)) = data.split_first() else { return };
    let steps = usize::from(steps) % MAX_STEPS;
    let (schedule, host) = rest.split_at((steps * 2).min(rest.len()));
    let token = PartitionToken::new([0x5A; PARTITION_TOKEN_BYTES]);
    let mut tabs = Tabs::default();
    let tab = tabs.open(7);
    assert_eq!(tabs.channel(tab), Some(7));
    let began = Instant::now();
    let mut now = began;
    let mut claim = NetworkClaim::default();
    let mut model = Model::default();
    let mut schedule = schedule.as_chunks::<2>().0.iter();
    let mut host = host;
    for _ in 0..MAX_REQUESTS {
        // A report of the claim, and the clock's step, between requests.
        if let Some(&[report, wait]) = schedule.next() {
            now += Duration::from_millis(u64::from(wait) * 4);
            if report & 1 != 0 {
                let has = report & 2 != 0;
                claim.reported(has, now);
                if has {
                    model.left = None;
                } else if model.has {
                    model.left = Some(now);
                }
                model.has = has;
            }
            let _ = claim.drain_ended(now);
        }
        let Ok(Some(output)) = read_host_output(&mut host) else { break };
        let HostOutput::Request(request) = output else { continue };
        let starts = starts_request(request.kind());
        let asked = claim.asked(starts, now);
        assert_eq!(asked, model.expected(starts, now), "a request of kind {} at {:?}", request.kind(), now - began);
        if asked == Asked::Relay
            && let Ok(written) = request_for_client(tab, &token, request)
        {
            // The trusted core reads no payload: the web client is the
            // judge of a start's fields, and may refuse them. What the
            // relay promises is the frame: for this tab, with this
            // partition's token.
            if let Ok(Some((to, _))) = read_to_client(&mut written.as_slice(), &token) {
                assert_eq!(to, tab, "a request relayed for another tab");
            }
        }
    }
});
