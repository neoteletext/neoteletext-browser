//! The developer console's entries (`settings-home-console-proxy.md`
//! section 3, requirements 19.8 and 19.9): the trusted core keeps each
//! tab's channel's newest entries of each console tab, and while the
//! console is open sends the active channel's as cleaned text, at most
//! [`MAX_ENTRIES`] a message. Everything here is the channel's own data,
//! shown as text and never interpreted.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;

use neoteletext_core::consts::{DATA_BYTES, PACKET_BYTES};
use neoteletext_core::packet::{Decoded, Packet, decode, decode_address};
use protocol::clean;
use protocol::console::{ConsoleEntry, ConsoleTab, ConsoleUpdate, KEPT, Level, MAX_ENTRIES, MAX_ENTRY, Stat};

/// Microseconds in a millisecond, for an exchange's time.
const MICROSECONDS_PER_MILLISECOND: u64 = 1000;
/// Where a packet's data bytes start: after its two address bytes.
const DATA_START: usize = PACKET_BYTES - DATA_BYTES;
/// Bytes outside printable ASCII show as this in a packet's text.
const UNPRINTABLE: char = '.';
/// A packet's bytes are seven bits and a parity bit (EN 300 706 section 8.2).
const SEVEN_BITS: u8 = 0x7F;

/// One channel's entries: each console tab's newest, with the number of
/// each, in the order they came; and what the Stats tab counts.
#[derive(Default)]
struct Log {
    tabs: [VecDeque<(u64, ConsoleEntry)>; ConsoleTab::ALL.len()],
    counts: Counts,
}

/// What the Stats tab counts of a channel since it opened (the owner, 27
/// October 2026): what came, by push and by fetch, its packets and their
/// sound, what was sent back, and the pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    connections: u64,
    pushes: u64,
    pushed_bytes: u64,
    fetches: u64,
    fetched_bytes: u64,
    packets: u64,
    /// Packets of the audio profile's data channel that carry sound.
    audio_packets: u64,
    /// Packets of that channel that carry the channel's cursor and control records.
    input_packets: u64,
    events_sent: u64,
    bytes_sent: u64,
    pages_arrived: u64,
    pages_changed: u64,
}

/// Digits between the commas of a large number.
const GROUP: usize = 3;

/// `value` with its thousands apart, as "1,234,567".
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && digits.len().saturating_sub(index).is_multiple_of(GROUP) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A count of bytes, as the Stats tab writes it.
fn bytes_text(bytes: u64) -> String {
    format!("{} bytes", grouped(bytes))
}

impl Counts {
    /// The Stats tab's figures.
    #[must_use]
    pub fn stats(&self) -> Vec<Stat> {
        let packet_bytes = |packets: u64| packets.saturating_mul(PACKET_BYTES as u64);
        let stat = |label: &str, value: String| Stat { label: label.into(), value };
        vec![
            stat("Bytes received", bytes_text(self.pushed_bytes.saturating_add(self.fetched_bytes))),
            stat("  by push (WebSocket)", format!("{} in {} messages", bytes_text(self.pushed_bytes), grouped(self.pushes))),
            stat("  by fetch (HTTP)", format!("{} in {} requests", bytes_text(self.fetched_bytes), grouped(self.fetches))),
            stat("Packets received", grouped(self.packets)),
            stat("Audio received", format!("{} in {} packets", bytes_text(packet_bytes(self.audio_packets)), grouped(self.audio_packets))),
            stat("Cursor records received", format!("{} in {} packets", bytes_text(packet_bytes(self.input_packets)), grouped(self.input_packets))),
            stat("Input events sent", grouped(self.events_sent)),
            stat("Bytes sent (input)", bytes_text(self.bytes_sent)),
            stat("Pages arrived", grouped(self.pages_arrived)),
            stat("Page changes", grouped(self.pages_changed)),
            stat("Push connections", grouped(self.connections)),
        ]
    }
}

/// The console's entries for every open channel.
#[derive(Default)]
pub struct Console {
    channels: BTreeMap<u32, Log>,
    /// The browser's own log lines, about no one channel: how each process
    /// was confined, the settings, the cookie file's key. Every channel's
    /// Log tab shows them among its own (decision 70).
    browser: VecDeque<(u64, ConsoleEntry)>,
    /// The number the next entry takes.
    next: u64,
    open: bool,
    /// The channel last sent and the number of its newest entry sent.
    sent: Option<(u32, u64)>,
    /// Whether anything came or went since the last send: without it, a
    /// send has nothing to look for.
    changed: bool,
    /// Whether a channel's counts changed since the last send.
    counted: bool,
}

fn slot(tab: ConsoleTab) -> usize {
    ConsoleTab::ALL.iter().position(|&each| each == tab).unwrap_or(0)
}

impl Console {
    /// Keeps an entry for `channel`'s `tab`, cleaned and cut to an entry.
    pub fn add(&mut self, channel: u32, tab: ConsoleTab, at_ms: u64, text: &str) {
        self.keep(Some(channel), tab, Level::Info, at_ms, text);
    }

    /// Keeps a log line of `channel`, or of the browser when none, at
    /// `level` (decision 70).
    pub fn log(&mut self, channel: Option<u32>, level: Level, at_ms: u64, text: &str) {
        self.keep(channel, ConsoleTab::Log, level, at_ms, text);
    }

    fn keep(&mut self, channel: Option<u32>, tab: ConsoleTab, level: Level, at_ms: u64, text: &str) {
        let entry = ConsoleEntry { tab, level, at_ms, text: clean(text, MAX_ENTRY) };
        let number = self.next;
        self.next = self.next.saturating_add(1);
        self.changed = true;
        let entries = match channel {
            Some(channel) => {
                let Some(entries) = self.channels.entry(channel).or_default().tabs.get_mut(slot(tab)) else { return };
                entries
            }
            None => &mut self.browser,
        };
        if entries.len() >= KEPT {
            entries.pop_front();
        }
        entries.push_back((number, entry));
    }

    /// Keeps a packet entry for each whole packet of `bytes`, the newest
    /// [`KEPT`]: where it came from, its address, a header's page number,
    /// and its data bytes as text and as hexadecimal.
    pub fn packets(&mut self, channel: u32, at_ms: u64, pushed: bool, bytes: &[u8]) {
        let (packets, _) = bytes.as_chunks::<PACKET_BYTES>();
        let skip = packets.len().saturating_sub(KEPT);
        for packet in packets.iter().skip(skip) {
            let text = packet_text(pushed, *packet);
            self.add(channel, ConsoleTab::Packets, at_ms, &text);
        }
    }

    /// `channel`'s counts, to change.
    fn counts(&mut self, channel: u32) -> &mut Counts {
        self.counted = true;
        &mut self.channels.entry(channel).or_default().counts
    }

    /// `channel`'s counts as they are.
    #[must_use]
    pub fn counted(&self, channel: u32) -> Counts {
        self.channels.get(&channel).map(|log| log.counts).unwrap_or_default()
    }

    /// Counts what a push brought: its bytes, and when they are packets,
    /// each packet, and those of the audio profile's data channel by what
    /// they carry, as the engine tells them (`idl::is_audio`, INPUT.md
    /// section 7).
    pub fn received(&mut self, channel: u32, pushed: bool, packets: bool, bytes: &[u8]) {
        let counts = self.counts(channel);
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if pushed {
            counts.pushes = counts.pushes.saturating_add(1);
            counts.pushed_bytes = counts.pushed_bytes.saturating_add(length);
        }
        if !packets {
            return;
        }
        let (whole, _) = bytes.as_chunks::<PACKET_BYTES>();
        for bytes in whole {
            counts.packets = counts.packets.saturating_add(1);
            let packet = Packet { bytes: *bytes };
            if !neoteletext_audio::idl::is_audio(&packet) {
                continue;
            }
            #[cfg(feature = "interaction")]
            if neoteletext_input::is_input_packet(&packet) {
                counts.input_packets = counts.input_packets.saturating_add(1);
                continue;
            }
            counts.audio_packets = counts.audio_packets.saturating_add(1);
        }
    }

    /// Counts an exchange the fetcher made and the bytes it brought.
    pub fn fetched(&mut self, channel: u32, bytes: u64) {
        let counts = self.counts(channel);
        counts.fetches = counts.fetches.saturating_add(1);
        counts.fetched_bytes = counts.fetched_bytes.saturating_add(bytes);
    }

    /// Counts the push connection opening.
    pub fn connected(&mut self, channel: u32) {
        let counts = self.counts(channel);
        counts.connections = counts.connections.saturating_add(1);
    }

    /// Counts an input event sent to the channel, and its bytes.
    pub fn sent(&mut self, channel: u32, bytes: usize) {
        let counts = self.counts(channel);
        counts.events_sent = counts.events_sent.saturating_add(1);
        counts.bytes_sent = counts.bytes_sent.saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    /// Counts a page that arrived, or one that changed.
    pub fn page(&mut self, channel: u32, arrived: bool) {
        let counts = self.counts(channel);
        if arrived {
            counts.pages_arrived = counts.pages_arrived.saturating_add(1);
        } else {
            counts.pages_changed = counts.pages_changed.saturating_add(1);
        }
    }

    /// The channel is gone: its entries go.
    pub fn forget(&mut self, channel: u32) {
        self.channels.remove(&channel);
        self.changed = true;
    }

    /// The console opened or closed. Opened, it is sent the active
    /// channel's entries afresh.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        self.sent = None;
        self.changed = true;
    }

    /// Whether the console is open, so it is to be sent what comes.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// What to send the interface for the `active` channel: nothing while
    /// the console is closed; all its entries when it is newly shown,
    /// replacing what was shown; otherwise its entries since the last.
    pub fn updates(&mut self, active: Option<u32>) -> Vec<ConsoleUpdate> {
        let Some(channel) = active.filter(|_| self.open) else { return Vec::new() };
        let (replace, after) = match self.sent {
            Some((sent, last)) if sent == channel => (false, Some(last)),
            _ => (true, None),
        };
        if !replace && !self.changed && !self.counted {
            return Vec::new();
        }
        let stats = (replace || self.counted).then(|| self.counted(channel).stats());
        self.changed = false;
        self.counted = false;
        let new = |(number, _): &&(u64, ConsoleEntry)| after.is_none_or(|last| *number > last);
        let mut entries: Vec<&(u64, ConsoleEntry)> = self.channels.get(&channel).map(|log| log.tabs.iter().flatten().filter(new).collect()).unwrap_or_default();
        entries.extend(self.browser.iter().filter(new));
        entries.sort_by_key(|(number, _)| *number);
        let last = entries.last().map(|(number, _)| *number).or(after).unwrap_or(0);
        self.sent = Some((channel, last));
        let mut updates: Vec<ConsoleUpdate> = entries
            .chunks(MAX_ENTRIES)
            .enumerate()
            .map(|(index, chunk)| ConsoleUpdate {
                channel,
                replace: replace && index == 0,
                entries: chunk.iter().map(|(_, entry)| entry.clone()).collect(),
                stats: None,
            })
            .collect();
        // A channel newly shown with nothing yet still clears what was
        // shown; figures that changed go even when no entry came.
        if updates.is_empty() && (replace || stats.is_some()) {
            updates.push(ConsoleUpdate { channel, replace, entries: Vec::new(), stats: None });
        }
        if let Some(last) = updates.last_mut() {
            last.stats = stats;
        }
        updates
    }
}

/// An exchange as the Network tab says it.
#[must_use]
pub fn exchange_text(method: &str, url: &str, status: u16, bytes: u64, elapsed_us: u64) -> String {
    format!("{method} {url} {status} {bytes} bytes {} ms", elapsed_us / MICROSECONDS_PER_MILLISECOND)
}

/// A packet as the Packets tab says it, read with the engine's decoder.
fn packet_text(pushed: bool, bytes: [u8; PACKET_BYTES]) -> String {
    let mut text = String::from(if pushed { "pushed" } else { "fetched" });
    let packet = Packet { bytes };
    match decode_address(&packet) {
        Ok(address) => {
            let _ = write!(text, " M{} R{:02}", address.magazine.number(), address.number);
            if let Ok(Decoded::Header(header)) = decode(&packet) {
                match header.page {
                    Some(page) => {
                        let _ = write!(text, " page {}{:X}{:X}", header.magazine.number(), page.tens(), page.units());
                    }
                    None => text.push_str(" page ?"),
                }
            }
        }
        Err(_) => text.push_str(" bad address"),
    }
    let data = bytes.get(DATA_START..).unwrap_or_default();
    text.push_str("  ");
    text.extend(data.iter().map(|&byte| {
        let character = char::from(byte & SEVEN_BITS);
        if character.is_ascii_graphic() || character == ' ' { character } else { UNPRINTABLE }
    }));
    text.push(' ');
    for byte in data {
        let _ = write!(text, " {byte:02X}");
    }
    text
}

#[cfg(test)]
mod tests;
