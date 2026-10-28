//! Feeding the engine: a channel's bytes, fetched or pushed, through the
//! engine's format readers into its decoder and store, and the pages that
//! arrived; the log lines and the link and cursor lines a page file
//! carries beyond its packets.

use super::{ChannelLog, Engine, LINE_PERIOD_US};

#[cfg(feature = "interaction")]
use crate::InputRecords;
#[cfg(feature = "links")]
use crate::links;
use crate::title::title;
use crate::voice_sink::Outbox;
use interface_core::channel::page_index;
use neoteletext_core::acq::StoredPage;
use neoteletext_core::consts::COLUMNS;
use neoteletext_core::consts::{MAGAZINES, PAGE_COMPLETION_TIMEOUT_US};
use neoteletext_core::fsm::PageState;
use neoteletext_core::packet::{Packet, PacketMeta};
use neoteletext_core::types::{Instant, PageAddress, SubCode};
use neoteletext_formats::Item;
use neoteletext_formats::item::Reader;
use std::collections::BTreeMap;

/// The most log lines held between two takes: a page carries at most the
/// engine's `MAX_LOG_LINES_PER_PAGE`, and a feed a few pages; past this a
/// channel that writes many is cut, not kept.
pub(super) const MAX_LOGS_HELD: usize = 64;

/// Keeps a log line read, within [`MAX_LOGS_HELD`].
fn keep_log(logs: &mut Vec<ChannelLog>, line: &neoteletext_formats::log::LogLine<'_>, page: Option<PageAddress>) {
    use neoteletext_formats::log::LogLevel;
    if logs.len() >= MAX_LOGS_HELD {
        return;
    }
    let level = match line.level {
        LogLevel::Error => interface_core::console::Level::Error,
        LogLevel::Warning => interface_core::console::Level::Warning,
        LogLevel::Information => interface_core::console::Level::Info,
    };
    logs.push(ChannelLog { level, page: page.map(PageAddress::mpp), text: line.text.to_owned() });
}

impl Engine {
    fn push_all(&mut self, packets: &[(Packet, PacketMeta)]) {
        for (packet, meta) in packets {
            // The channel's cursor and control records (INPUT.md section 7)
            // ride the audio profile's data channel; only one that reads
            // goes on, as its body.
            #[cfg(feature = "interaction")]
            if neoteletext_input::is_input_packet(packet) {
                self.input.take_packet(packet);
                continue;
            }
            // The audio profile's packets go to its receiver; the decoder
            // ignores packet 31 anyway (the engine's REQ-PKT-008).
            if self.audio.push_packet(packet, &mut Outbox::new(&mut self.sounds)) {
                continue;
            }
            self.clock = match meta.timestamp_us {
                Some(stamp) if stamp >= self.clock => stamp,
                _ => self.clock.saturating_add(LINE_PERIOD_US),
            };
            let changes = &mut self.changes;
            self.decoder.push(packet, meta, Instant(self.clock), &mut |event| changes.note(&event));
        }
    }

    /// Feeds more of the channel's bytes; returns the pages that arrived.
    /// Pages that changed are kept for [`Engine::take_updated`].
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<(u16, String)> {
        let mut packets = Vec::new();
        let mut pages = PageLines {
            #[cfg(feature = "links")]
            lines: &mut self.lines,
            #[cfg(feature = "interaction")]
            input: &mut self.input,
            receiver_rows: &mut self.receiver_rows,
            unused: core::marker::PhantomData,
        };
        let logs = &mut self.logs;
        self.reader.feed(bytes, &mut |item| match item {
            Item::Packet { packet, meta } => packets.push((packet, meta)),
            // A page file's link and cursor lines are kept beside its packets.
            Item::Page(page) => pages.read(page),
            Item::Log { line, page, .. } => keep_log(logs, &line, page),
            _ => {}
        });
        self.drop_pushed(&mut packets);
        self.push_all(&packets);
        self.arrivals()
    }

    /// Feeds one whole page file, bundle or pushed message, and completes
    /// the pages in it at once: a page shows as it arrives, not when the
    /// next page of its magazine begins (`interaction.md` section 10). A
    /// fetched copy of a page the channel pushed is older, and dropped.
    pub fn feed_whole(&mut self, packets: bool, pushed: bool, bytes: &[u8]) -> Vec<(u16, String)> {
        let mut items = Vec::new();
        let mut pages = PageLines {
            #[cfg(feature = "links")]
            lines: &mut self.lines,
            #[cfg(feature = "interaction")]
            input: &mut self.input,
            receiver_rows: &mut self.receiver_rows,
            unused: core::marker::PhantomData,
        };
        let reader = if packets { &mut self.pushed_packets } else { &mut self.pushed_pages };
        let logs = &mut self.logs;
        let mut take = |item: Item<'_>| match item {
            Item::Packet { packet, meta } => items.push((packet, meta)),
            Item::Page(page) => pages.read(page),
            Item::Log { line, page, .. } => keep_log(logs, &line, page),
            _ => {}
        };
        reader.feed(bytes, &mut take);
        reader.finish(&mut take);
        if pushed {
            for (packet, _) in &items {
                if let Some(page) = header_page(packet) {
                    self.pushed.insert(page);
                }
            }
        } else {
            self.drop_pushed(&mut items);
        }
        self.push_all(&items);
        self.complete();
        self.arrivals()
    }

    /// Drops the fetched packets of pages the channel has pushed: a page
    /// file fetched in the contents' order may arrive after the channel
    /// pushed a newer version, and must not replace it.
    fn drop_pushed(&mut self, packets: &mut Vec<(Packet, PacketMeta)>) {
        if self.pushed.is_empty() {
            return;
        }
        let (pushed, skipping) = (&self.pushed, &mut self.skipping);
        packets.retain(|(packet, _)| {
            let Ok(address) = neoteletext_core::packet::decode_address(packet) else { return true };
            let magazine = usize::from(address.magazine.number()) % MAGAZINES;
            let Some(skip) = skipping.get_mut(magazine) else { return true };
            if address.number == 0 {
                *skip = header_page(packet).is_some_and(|page| pushed.contains(&page));
            }
            !*skip
        });
    }

    /// Completes every page still being received, as the end of a stream
    /// or a timeout does (EN 300 706 clause 7.2: a page ends with the next
    /// header of its magazine, or when the receiver gives up waiting).
    fn complete(&mut self) {
        self.clock = self.clock.saturating_add(PAGE_COMPLETION_TIMEOUT_US.saturating_mul(2));
        let changes = &mut self.changes;
        self.decoder.tick(Instant(self.clock), &mut |event| changes.note(&event));
    }

    /// Ends the channel's data: completes every page still being received.
    pub fn finish(&mut self) -> Vec<(u16, String)> {
        let mut packets = Vec::new();
        let mut pages = PageLines {
            #[cfg(feature = "links")]
            lines: &mut self.lines,
            #[cfg(feature = "interaction")]
            input: &mut self.input,
            receiver_rows: &mut self.receiver_rows,
            unused: core::marker::PhantomData,
        };
        let logs = &mut self.logs;
        self.reader.finish(&mut |item| match item {
            Item::Packet { packet, meta } => packets.push((packet, meta)),
            Item::Page(page) => pages.read(page),
            Item::Log { line, page, .. } => keep_log(logs, &line, page),
            _ => {}
        });
        self.drop_pushed(&mut packets);
        self.push_all(&packets);
        self.complete();
        self.arrivals()
    }

    /// Display pages received completely and not yet reported, each with
    /// a title from its text.
    fn arrivals(&mut self) -> Vec<(u16, String)> {
        let mut fresh = Vec::new();
        // Only pages whose reception completed can have arrived or changed:
        // the engine's events say which, so the store is not searched.
        for number in self.changes.take_completed() {
            let Some(index) = page_index(number) else { continue };
            let Some(address) = PageAddress::from_mpp(number) else { continue };
            let Some(page) = self.decoder.store().get(address, SubCode::ANY) else { continue };
            if page.state != PageState::Complete {
                continue;
            }
            #[cfg(test)]
            {
                self.digests = self.digests.saturating_add(1);
            }
            let digest = digest(page);
            let Some(reported) = self.reported.get_mut(index) else { continue };
            match *reported {
                None => fresh.push(number),
                Some(before) if before != digest => self.updated.push(number),
                Some(_) => {}
            }
            *reported = Some(digest);
        }
        self.updated.sort_unstable();
        self.updated.dedup();
        fresh.sort_unstable();
        fresh.into_iter().map(|page| (page, self.grid(page).map(|(grid, _)| title(&grid)).unwrap_or_default())).collect()
    }
}

/// The page number a header packet starts, as teletext writes it.
fn header_page(packet: &Packet) -> Option<u16> {
    match neoteletext_core::packet::decode(packet) {
        Ok(neoteletext_core::packet::Decoded::Header(header)) => Some(PageAddress { magazine: header.magazine, page: header.page? }.mpp()),
        _ => None,
    }
}

/// What a page file says beyond its packets: its link lines and its
/// cursor line, kept as each page is read.
/// A row 24 drawn by the receiver from a page's `LN` lines: its codes, and
/// the columns where "»" marks a link that leaves the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReceiverRow {
    codes: [u8; COLUMNS],
    marks: Vec<usize>,
}

impl ReceiverRow {
    /// The row's codes.
    pub(super) fn codes(&self) -> [u8; COLUMNS] {
        self.codes
    }

    /// The columns of its "»" marks.
    pub(super) fn marks(&self) -> &[usize] {
        &self.marks
    }
}

struct PageLines<'a> {
    #[cfg(feature = "links")]
    lines: &'a mut links::Lines,
    receiver_rows: &'a mut BTreeMap<(u16, u16), ReceiverRow>,
    #[cfg(feature = "interaction")]
    input: &'a mut InputRecords,
    unused: core::marker::PhantomData<&'a ()>,
}

impl PageLines<'_> {
    #[cfg_attr(not(any(feature = "links", feature = "interaction")), allow(unused_variables, clippy::unused_self, reason = "the lines are compiled out"))]
    fn read(&mut self, page: &neoteletext_formats::page::PageFile) {
        #[cfg(feature = "links")]
        self.lines.read(page);
        // The engine's receiver row: the browser follows no linked service
        // (decision E.88), so a link to another service is null.
        let key = (page.address.mpp(), page.subcode.value());
        match neoteletext_formats::links::receiver_links(page, &[] as &[&str], false) {
            Some(receiver) => {
                let marks = receiver.row.marks.as_slice().iter().map(|&column| usize::from(column)).collect();
                self.receiver_rows.insert(key, ReceiverRow { codes: receiver.row.codes, marks });
            }
            None => {
                self.receiver_rows.remove(&key);
            }
        }
        // REQ-INT-006: a `CU` line is the page's cursor record.
        #[cfg(feature = "interaction")]
        if let Some(line) = page.cursor {
            let [p0, p1] = page.address.mpp().to_be_bytes();
            let [a0, a1] = line.acked.to_be_bytes();
            // Byte 6 is reserved, written 0 (the engine's decision E.134).
            self.input.cursors.push([p0, p1, a0, a1, line.row, line.column, 0, line.flags]);
        }
    }
}

/// A digest of a page's rows: two versions with the same digest show the
/// same text and mosaics.
fn digest(page: &StoredPage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    page.cells.hash(&mut hasher);
    hasher.finish()
}
