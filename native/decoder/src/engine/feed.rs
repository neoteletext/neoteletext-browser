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
use neoteletext_formats::links::{Destination, KEYS_WITH_INDEX};
use std::collections::BTreeMap;
#[cfg(feature = "links")]
use std::collections::BTreeSet;

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
    pub(super) fn push_all(&mut self, packets: &[(Packet, PacketMeta)]) {
        for (packet, meta) in packets {
            // The channel's cursor and control records (DOC-1951)
            // ride the audio profile's data channel; only one that reads
            // goes on, as its body.
            #[cfg(feature = "interaction")]
            if neoteletext_input::is_input_packet(packet) {
                self.input.take_packet(packet);
                continue;
            }
            // The audio profile's packets go to its receiver; the decoder
            // ignores packet 31 anyway (the engine's REQ-PKT-008).
            if self.audio.push_packet(packet, &mut Outbox::new(&mut self.sounds, &mut self.heard)) {
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
            // The sub-pages of this message and the links each held.
            #[cfg(feature = "links")]
            let mut frames: BTreeMap<(u16, u16), Option<links::LinkLines>> = BTreeMap::new();
            #[cfg(feature = "links")]
            let mut linked = BTreeSet::new();
            let now = self.now_us();
            // The packets of a page whose atomic frame is open are held
            // for its end (animation.md ANIM-2), and not given to the
            // engine's decoder with the rest below.
            let mut kept = Vec::with_capacity(items.len());
            for (packet, meta) in &items {
                let held = self.hold_packet(packet, meta, header_key(packet));
                kept.push(!held);
                if let Some(key) = header_key(packet) {
                    self.pushed.insert(key.0);
                    self.cache.get_mut().store(key.0, key.1, now);
                    #[cfg(feature = "links")]
                    frames.entry(key).or_insert_with(|| self.lines.get(key.0, key.1).cloned());
                }
                // A pushed sub-page's Link Extension links come as a LINKS
                // record after it (REQ-1494), into the store a page file's
                // lines fill; the engine's agent counted any it refused.
                let read = self.records.push(packet);
                // A sub-page's ReGIS (DEC-6335), NAPLPS (DEC-6407) and
                // ReGIS cursor (the engine's REQ-6371), and the records of
                // animation.md (ANIMATE, SPRITES, NAPLPS_OBJECTS, SHEET
                // and FRAME): held for the page's open frame, or applied
                // now (ANIM-2).
                if let Some(taken) = crate::animation::frames::Taken::of(&read) {
                    self.take_animation(taken);
                    continue;
                }
                match read {
                    #[cfg(feature = "links")]
                    neoteletext_web::records::Read::Record(neoteletext_web::records::ChannelRecord::Links(record)) => {
                        let key = (record.page.mpp(), record.subcode.value());
                        frames.entry(key).or_insert_with(|| self.lines.get(key.0, key.1).cloned());
                        linked.insert(key);
                        self.lines.read_record(&record);
                    }
                    // A page's rule table (page-rules.md RULE-41), which
                    // only this process decodes (RULE-58).
                    neoteletext_web::records::Read::Record(neoteletext_web::records::ChannelRecord::Rules(record)) => {
                        // Copied out of the reader, which the host's call
                        // may not borrow.
                        let data = record.fragment.map(|fragment| (fragment.total, fragment.offset, fragment.data.to_vec()));
                        let (page, subcode, generation) = (record.page, record.subcode, record.generation);
                        let fragment =
                            data.as_ref().map(|(total, offset, data)| neoteletext_web::records::rules::Fragment { total: *total, offset: *offset, data });
                        self.rules_record(&neoteletext_web::records::rules::RulesRecord { page, subcode, generation, fragment });
                    }
                    // A command the server sent a page directly
                    // (RULE-114), for its evaluator or its local sound.
                    neoteletext_web::records::Read::Record(
                        neoteletext_web::records::ChannelRecord::LocalMidi(record)
                        | neoteletext_web::records::ChannelRecord::Variable(record)
                        | neoteletext_web::records::ChannelRecord::Timer(record)
                        | neoteletext_web::records::ChannelRecord::Report(record)
                        | neoteletext_web::records::ChannelRecord::Run(record)
                        | neoteletext_web::records::ChannelRecord::Sequence(record),
                    ) => {
                        let page = protocol::rules::PageSub::new(record.page.mpp(), record.subcode.value());
                        let (kind, body) = (record.kind, record.body.to_vec());
                        self.rule_command(kind, page, &body);
                    }
                    // The drawings of a ReGIS sub-page that are links (the
                    // engine's REQ-6370), for the trusted core.
                    #[cfg(feature = "links")]
                    neoteletext_web::records::Read::Record(neoteletext_web::records::ChannelRecord::DrawingLinks(record)) => {
                        self.regis.read_links(&record);
                    }
                    _ => {}
                }
            }
            #[cfg(feature = "links")]
            if packets {
                self.settle_links(frames, &linked);
            }
            // What the frames that ended in this message held goes first,
            // then the message's own packets that no frame holds.
            let mut kept = kept.into_iter();
            items.retain(|_| kept.next().unwrap_or(true));
            let mut released = self.take_released();
            released.append(&mut items);
            items = released;
        } else {
            self.drop_pushed(&mut items);
        }
        self.push_all(&items);
        self.complete();
        self.arrivals()
    }

    /// After a pushed message, each sub-page's links are what the message
    /// said: a frame with no LINKS record means no links, whole or changed
    /// rows (neoteletext-common's DEC-1517). A TTI page pushed whole is not
    /// such a frame: its own lines were read with it. Links that changed
    /// redraw the page even when no row did, as a frame of the header alone
    /// says (REQ-1494).
    #[cfg(feature = "links")]
    fn settle_links(&mut self, frames: BTreeMap<(u16, u16), Option<links::LinkLines>>, linked: &BTreeSet<(u16, u16)>) {
        for (key, before) in frames {
            let (page, subcode) = key;
            if !linked.contains(&key) {
                self.lines.clear(page, subcode);
            }
            let now = self.lines.get(page, subcode);
            if now == before.as_ref() {
                continue;
            }
            match now.and_then(links::LinkLines::key_file).as_ref().and_then(receiver_row) {
                Some(row) => self.receiver_rows.insert(key, row),
                None => self.receiver_rows.remove(&key),
            };
            self.updated.push(page);
        }
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
    pub(super) fn complete(&mut self) {
        self.clock = self.clock.saturating_add(PAGE_COMPLETION_TIMEOUT_US.saturating_mul(2));
        let changes = &mut self.changes;
        self.decoder.tick(Instant(self.clock), &mut |event| changes.note(&event));
    }

    /// Ends the channel's data: completes every page still being received.
    pub fn finish(&mut self) -> Vec<(u16, String)> {
        // ANIM-2: the connection's end ends every frame, and what each
        // held is applied.
        self.end_frames();
        let released = self.take_released();
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
        self.push_all(&released);
        self.push_all(&packets);
        self.complete();
        self.arrivals()
    }

    /// Display pages received completely and not yet reported, each with
    /// a title from its text.
    pub(super) fn arrivals(&mut self) -> Vec<(u16, String)> {
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
        // A NAPLPS picture's title names its page when it has one (DEC-6638
        // item 3); the program's index, when it names the page, still wins
        // in the trusted core.
        fresh
            .into_iter()
            .map(|page| {
                let named = self.naplps.title(page).map(String::from);
                (page, named.unwrap_or_else(|| self.grid(page).map(|(grid, _)| title(&grid)).unwrap_or_default()))
            })
            .collect()
    }
}

/// The page number a header packet starts, as teletext writes it.
fn header_page(packet: &Packet) -> Option<u16> {
    header_key(packet).map(|(page, _)| page)
}

/// The page number and sub-code a header packet starts.
fn header_key(packet: &Packet) -> Option<(u16, u16)> {
    match neoteletext_core::packet::decode(packet) {
        Ok(neoteletext_core::packet::Decoded::Header(header)) => {
            Some((PageAddress { magazine: header.magazine, page: header.page? }.mpp(), header.subcode.map_or(0, SubCode::value)))
        }
        _ => None,
    }
}

/// The receiver's row 24 of a page file's `LN` lines, the one way a page
/// file and a LINKS record get theirs. The browser follows no linked
/// service (decision E.88), so a link to another service is null.
fn receiver_row(page: &neoteletext_formats::page::PageFile) -> Option<ReceiverRow> {
    let receiver = neoteletext_formats::links::receiver_links(page, &[] as &[&str], false)?;
    let marks = receiver.row.marks.as_slice().iter().map(|&column| usize::from(column)).collect();
    let keys = receiver.keys.map(|key| match key {
        Some(Destination::Page { address, subcode, service: None }) => Some((address.mpp(), subcode.filter(|subcode| !subcode.is_any()).map(SubCode::value))),
        _ => None,
    });
    Some(ReceiverRow { codes: receiver.row.codes, marks, keys })
}

/// What a page file says beyond its packets: its link lines and its
/// cursor line, kept as each page is read.
/// A row 24 drawn by the receiver from a page's `LN` lines: its codes, and
/// the columns where "»" marks a link that leaves the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReceiverRow {
    codes: [u8; COLUMNS],
    marks: Vec<usize>,
    /// Where each key leads on this channel: red, green, yellow, cyan and
    /// Index, a page and its sub-code when it names one.
    keys: [Option<(u16, Option<u16>)>; KEYS_WITH_INDEX],
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

    /// Where its keys lead.
    #[cfg(feature = "links")]
    pub(super) fn keys(&self) -> [Option<(u16, Option<u16>)>; KEYS_WITH_INDEX] {
        self.keys
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
        let key = (page.address.mpp(), page.subcode.value());
        match receiver_row(page) {
            Some(row) => self.receiver_rows.insert(key, row),
            None => self.receiver_rows.remove(&key),
        };
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
