//! Sleeping tabs (decision 47), as other browsers put background tabs to
//! sleep: a channel's decoder, and its fetcher unless the fetcher holds a
//! push connection, run only while its tab is awake. A tab is awake while
//! it is the active one, while its sound plays, and for [`SLEEP_AFTER`]
//! after it was last shown; a tab restored with a session and never shown
//! sleeps from the start. What the trusted core gave a channel's decoder
//! is kept, and given to a new decoder when the tab wakes, so its pages are
//! there at once. A decoder at rest takes 16 MB (measured 21 October
//! 2026); the bytes kept of a channel are a few hundred kilobytes.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use protocol::decoder::ToDecoder;

/// How long a tab stays awake after it was last shown.
pub const SLEEP_AFTER: Duration = Duration::from_secs(15 * 60);
/// The most kept of one channel's data for its next decoder. A channel whose
/// data grows beyond it stays awake, rather than lose what it showed.
pub const MAX_KEPT_BYTES: usize = 8 * 1024 * 1024;

/// What one channel's decoder was given.
#[derive(Default)]
struct Kept {
    messages: Vec<ToDecoder>,
    bytes: usize,
    overflowed: bool,
}

/// Which tabs may sleep, and what to give their decoders when they wake.
#[derive(Default)]
pub struct Sleep {
    kept: BTreeMap<u32, Kept>,
    seen: BTreeMap<u32, Instant>,
    sounding: BTreeSet<u32>,
    pushing: BTreeSet<u32>,
}

impl Sleep {
    /// Keeps `message` for the channel's next decoder: its data, not its
    /// requests.
    pub fn keep(&mut self, channel: u32, message: &ToDecoder) {
        let bytes = match message {
            ToDecoder::Data(bytes) | ToDecoder::Whole { bytes, .. } => bytes.len(),
            ToDecoder::End => 0,
            _ => return,
        };
        let kept = self.kept.entry(channel).or_default();
        if kept.overflowed {
            return;
        }
        kept.bytes = kept.bytes.saturating_add(bytes);
        if kept.bytes > MAX_KEPT_BYTES {
            kept.overflowed = true;
            kept.messages = Vec::new();
            return;
        }
        kept.messages.push(message.clone());
    }

    /// What the channel's decoder was given, to give a new one.
    #[must_use]
    pub fn kept(&self, channel: u32) -> &[ToDecoder] {
        self.kept.get(&channel).map_or(&[], |kept| &kept.messages)
    }

    /// The channel's tab is shown at `now`.
    pub fn seen(&mut self, channel: u32, now: Instant) {
        self.seen.insert(channel, now);
    }

    /// The channels whose sound plays, as the interface reports them.
    pub fn set_sounding(&mut self, channels: &[u32]) {
        self.sounding = channels.iter().copied().collect();
    }

    /// Whether the channel's fetcher holds a push connection.
    pub fn set_pushing(&mut self, channel: u32, open: bool) {
        if open {
            self.pushing.insert(channel);
        } else {
            self.pushing.remove(&channel);
        }
    }

    /// Whether the channel's fetcher must keep running while its tab
    /// sleeps: it holds a push connection, whose pages are kept.
    #[must_use]
    pub fn keeps_fetcher(&self, channel: u32) -> bool {
        self.pushing.contains(&channel)
    }

    /// Whether the channel's tab is awake at `now`, `active` being the
    /// active tab's channel.
    #[must_use]
    pub fn awake(&self, channel: u32, active: Option<u32>, now: Instant) -> bool {
        active == Some(channel)
            || self.sounding.contains(&channel)
            || self.kept.get(&channel).is_some_and(|kept| kept.overflowed)
            || self.seen.get(&channel).is_some_and(|&seen| now.saturating_duration_since(seen) < SLEEP_AFTER)
    }

    /// The channel is closed: nothing of it is kept.
    pub fn forget(&mut self, channel: u32) {
        self.kept.remove(&channel);
        self.seen.remove(&channel);
        self.sounding.remove(&channel);
        self.pushing.remove(&channel);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, reason = "a test")]
    use super::*;

    #[test]
    fn a_tab_sleeps_once_it_has_not_been_shown_for_a_while_unless_it_sounds() {
        let start = Instant::now();
        let mut sleep = Sleep::default();
        assert!(!sleep.awake(2, Some(1), start), "a restored tab never shown sleeps");
        assert!(sleep.awake(1, Some(1), start), "the active tab is awake");
        sleep.seen(2, start);
        assert!(sleep.awake(2, Some(1), start + SLEEP_AFTER / 2));
        assert!(!sleep.awake(2, Some(1), start + SLEEP_AFTER));
        sleep.set_sounding(&[2]);
        assert!(sleep.awake(2, Some(1), start + SLEEP_AFTER), "its sound plays");
        sleep.set_sounding(&[]);
        assert!(!sleep.awake(2, Some(1), start + SLEEP_AFTER));
    }

    #[test]
    fn a_channels_data_is_kept_for_its_next_decoder_and_too_much_keeps_it_awake() {
        let start = Instant::now();
        let mut sleep = Sleep::default();
        let data = ToDecoder::Data(vec![1; 10]);
        sleep.keep(3, &data);
        sleep.keep(3, &ToDecoder::Showing(vec![0x100]));
        sleep.keep(3, &ToDecoder::End);
        assert_eq!(sleep.kept(3), [data, ToDecoder::End], "data, not requests");
        sleep.keep(3, &ToDecoder::Whole { packets: false, pushed: true, bytes: vec![0; MAX_KEPT_BYTES] });
        assert_eq!(sleep.kept(3), [] as [ToDecoder; 0]);
        assert!(sleep.awake(3, None, start), "what it showed would be lost");
        sleep.forget(3);
        assert!(!sleep.awake(3, None, start));
    }

    #[test]
    fn a_fetcher_holding_a_push_connection_is_kept() {
        let mut sleep = Sleep::default();
        assert!(!sleep.keeps_fetcher(4));
        sleep.set_pushing(4, true);
        assert!(sleep.keeps_fetcher(4));
        sleep.set_pushing(4, false);
        assert!(!sleep.keeps_fetcher(4));
    }
}
