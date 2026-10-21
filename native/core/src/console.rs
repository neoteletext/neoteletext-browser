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
use protocol::console::{ConsoleEntry, ConsoleTab, ConsoleUpdate, KEPT, MAX_ENTRIES, MAX_ENTRY};

/// Microseconds in a millisecond, for an exchange's time.
const MICROSECONDS_PER_MILLISECOND: u64 = 1000;
/// Where a packet's data bytes start: after its two address bytes.
const DATA_START: usize = PACKET_BYTES - DATA_BYTES;
/// Bytes outside printable ASCII show as this in a packet's text.
const UNPRINTABLE: char = '.';
/// A packet's bytes are seven bits and a parity bit (EN 300 706 section 8.2).
const SEVEN_BITS: u8 = 0x7F;

/// One channel's entries: each console tab's newest, with the number of
/// each, in the order they came.
#[derive(Default)]
struct Log {
    tabs: [VecDeque<(u64, ConsoleEntry)>; ConsoleTab::ALL.len()],
}

/// The console's entries for every open channel.
#[derive(Default)]
pub struct Console {
    channels: BTreeMap<u32, Log>,
    /// The number the next entry takes.
    next: u64,
    open: bool,
    /// The channel last sent and the number of its newest entry sent.
    sent: Option<(u32, u64)>,
    /// Whether anything came or went since the last send: without it, a
    /// send has nothing to look for.
    changed: bool,
}

fn slot(tab: ConsoleTab) -> usize {
    ConsoleTab::ALL.iter().position(|&each| each == tab).unwrap_or(0)
}

impl Console {
    /// Keeps an entry for `channel`'s `tab`, cleaned and cut to an entry.
    pub fn add(&mut self, channel: u32, tab: ConsoleTab, at_ms: u64, text: &str) {
        let entry = ConsoleEntry { tab, at_ms, text: clean(text, MAX_ENTRY) };
        let number = self.next;
        self.next = self.next.saturating_add(1);
        self.changed = true;
        let log = self.channels.entry(channel).or_default();
        let Some(entries) = log.tabs.get_mut(slot(tab)) else { return };
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
        if !replace && !self.changed {
            return Vec::new();
        }
        self.changed = false;
        let mut entries: Vec<&(u64, ConsoleEntry)> = self
            .channels
            .get(&channel)
            .map(|log| log.tabs.iter().flatten().filter(|(number, _)| after.is_none_or(|last| *number > last)).collect())
            .unwrap_or_default();
        entries.sort_by_key(|(number, _)| *number);
        let last = entries.last().map(|(number, _)| *number).or(after).unwrap_or(0);
        self.sent = Some((channel, last));
        let mut updates: Vec<ConsoleUpdate> = entries
            .chunks(MAX_ENTRIES)
            .enumerate()
            .map(|(index, chunk)| ConsoleUpdate { channel, replace: replace && index == 0, entries: chunk.iter().map(|(_, entry)| entry.clone()).collect() })
            .collect();
        // A channel newly shown with nothing yet still clears what was shown.
        if replace && updates.is_empty() {
            updates.push(ConsoleUpdate { channel, replace: true, entries: Vec::new() });
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
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;
    use neoteletext_core::ecc::hamming84_encode;

    /// A packet of magazine 1, row `row`, its data `data`, as the engine
    /// writes one.
    fn packet(row: u8, data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![hamming84_encode(1 | ((row & 1) << 3)), hamming84_encode(row >> 1)];
        bytes.extend(data.iter().copied().chain(std::iter::repeat(b' ')).take(DATA_BYTES));
        bytes
    }

    /// Requirement 19.9: each packet's address read by the engine, its
    /// bytes shown as text and hexadecimal, never anything else.
    #[test]
    fn packets_are_read_with_the_engine_and_shown_as_text() {
        let mut console = Console::default();
        let mut header = vec![hamming84_encode(0), hamming84_encode(1)];
        header.extend(std::iter::repeat_n(hamming84_encode(0), 6));
        header.extend(b"HEADER".iter().copied().chain(std::iter::repeat(b' ')).take(DATA_BYTES - 8));
        let mut bytes = packet(0, &[]);
        bytes.splice(DATA_START.., header);
        bytes.extend(packet(5, b"\x1b\x07Hello\r\n<script>"));
        // A byte with two bits wrong cannot be corrected.
        let broken = (0..=u8::MAX).find(|&byte| neoteletext_core::ecc::hamming84(byte).is_none()).unwrap();
        bytes.extend([broken; PACKET_BYTES]);
        console.packets(1, 7, true, &bytes);
        console.set_open(true);
        let updates = console.updates(Some(1));
        let texts: Vec<&str> = updates[0].entries.iter().map(|entry| entry.text.as_str()).collect();
        assert!(texts[0].starts_with("pushed M1 R00 page 110"), "{texts:?}");
        assert!(texts[1].starts_with("pushed M1 R05  ..Hello..<script>"), "{texts:?}");
        assert!(texts[1].contains(" 1B 07 48"), "{texts:?}");
        assert!(texts[2].starts_with("pushed bad address"), "{texts:?}");
        assert!(texts.iter().all(|text| text.len() <= MAX_ENTRY && text.chars().all(|character| (' '..='~').contains(&character))));
    }

    #[test]
    fn each_tab_keeps_its_newest_1000() {
        let mut console = Console::default();
        for n in 0..1005 {
            console.add(1, ConsoleTab::Notices, n, &format!("notice {n}"));
        }
        console.add(1, ConsoleTab::Network, 9, "GET");
        console.set_open(true);
        let updates = console.updates(Some(1));
        let entries: Vec<&ConsoleEntry> = updates.iter().flat_map(|update| &update.entries).collect();
        assert_eq!(entries.len(), KEPT + 1);
        assert_eq!(entries[0].text, "notice 5", "the oldest went");
        assert!(updates.iter().all(|update| update.entries.len() <= MAX_ENTRIES));
        assert_eq!(updates.iter().filter(|update| update.replace).count(), 1, "the first replaces, the rest follow");
        assert!(updates[0].replace);
    }

    /// Requirement 19.9: sent only while open, the new ones only, and
    /// afresh when another channel is shown.
    #[test]
    fn only_new_entries_go_and_only_while_open() {
        let mut console = Console::default();
        console.add(1, ConsoleTab::Pages, 0, "page 100 arrived");
        assert_eq!(console.updates(Some(1)), [], "closed");
        console.set_open(true);
        assert_eq!(console.updates(Some(1))[0].entries.len(), 1);
        assert!(console.updates(Some(1)).iter().all(|update| update.entries.is_empty()));
        console.add(1, ConsoleTab::Pages, 1, "page 101 arrived");
        console.add(2, ConsoleTab::Pages, 1, "other channel");
        let updates = console.updates(Some(1));
        assert_eq!((updates[0].replace, updates[0].entries[0].text.as_str()), (false, "page 101 arrived"));
        let updates = console.updates(Some(2));
        assert_eq!((updates[0].replace, updates[0].entries.len()), (true, 1), "another channel replaces");
        let updates = console.updates(Some(3));
        assert_eq!((updates[0].replace, updates[0].entries.len()), (true, 0), "an empty channel clears what was shown");
        console.forget(2);
        assert_eq!(console.updates(Some(2))[0].entries, []);
        console.set_open(false);
        assert_eq!(console.updates(Some(1)), []);
    }

    #[test]
    fn hidden_characters_are_cleaned() {
        let mut console = Console::default();
        console.add(1, ConsoleTab::Notices, 0, &format!("a\u{202E}b{}", "x".repeat(400)));
        console.set_open(true);
        let text = &console.updates(Some(1))[0].entries[0].text;
        assert!(text.starts_with("ab") && text.len() <= MAX_ENTRY);
        assert_eq!(exchange_text("GET", "https://a.example/100.tti", 200, 512, 12_345), "GET https://a.example/100.tti 200 512 bytes 12 ms");
    }
}
