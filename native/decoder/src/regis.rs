//! The sub-pages a channel draws in ReGIS (DEC-6335): the bytes its REGIS
//! records carry (the engine's REQ-6336) go to the ReGIS renderer, one
//! terminal for each sub-page, and what it draws is sent to the trusted
//! core as a picture with its pick record.
//!
//! The bytes are untrusted. The renderer bounds what they cost to read and
//! what one repaint costs; here the number of sub-pages kept is bounded,
//! and how much repainting each may cost in a second (REQ-6380): a
//! picture is repainted only while its repaint budget has something in
//! hand, and a repaint held back is made once, of the latest scene, when
//! the budget allows.

use std::collections::BTreeMap;

use neoteletext_web::records::regis::RegisRecord;
use protocol::regis::{RegisPick, RegisPicture, RegisViewer, pack};
use regis::{Event, Font, Picture, RepaintBudget, Screen, Terminal, ViewerState};

/// The most sub-pages whose ReGIS is kept. Each holds two pages of pixels,
/// a picture and its drawings: about a megabyte and a half before any
/// drawing is defined. One more arriving forgets the one fed longest ago.
pub const MAX_REGIS_PAGES: usize = 4;

/// Microseconds in a millisecond: the decoder's clock counts the first,
/// the repaint budget the second.
const MICROSECONDS_PER_MILLISECOND: u64 = 1000;

/// One sub-page's ReGIS.
struct Page {
    /// The stream being received.
    generation: u16,
    /// Where the next record of it must begin.
    next: u32,
    /// A record of it was lost: nothing more of it is drawn.
    lost: bool,
    terminal: Terminal,
    screen: Screen,
    picture: Picture,
    /// What it shows changed since its picture was last taken.
    changed: bool,
    /// When it was last fed, by the count of records read.
    fed: u64,
    /// What the viewer's pointer is doing to it, as the trusted core said.
    viewer: RegisViewer,
    /// The numbered drawing that is its cursor (the engine's REQ-6371), or
    /// 0 for none: blinked while keys go to the channel (REQ-6378).
    cursor: u16,
    /// When the cursor's blinking last began again, by the decoder's
    /// clock; `None` while it does not blink.
    blink_from: Option<u64>,
    /// The cursor is to be shown for a full half period from the next
    /// time the clock is read: it changed, or the picture did.
    blink_again: bool,
    /// What bounds its repainting (the renderer's REQ-6368).
    budget: RepaintBudget,
    /// The time the budget was last credited to, in milliseconds of the
    /// decoder's clock; `None` before its first repaint, when it is full.
    earned_ms: Option<u64>,
}

impl Page {
    /// A sub-page's ReGIS begun: `viewer` is what the pointer was doing to
    /// the stream before it, which a new stream does not change.
    fn new(generation: u16, viewer: RegisViewer) -> Self {
        let mut terminal = Terminal::new();
        // The font is this program's own data: if it did not parse, text
        // would be drawn as blocks, as the renderer draws a missing glyph.
        if let Ok(font) = Font::parse(regis_fonts::VT340) {
            terminal.set_font(font);
        }
        Self {
            generation,
            next: 0,
            lost: false,
            terminal,
            screen: Screen::new(),
            picture: Picture::new(),
            changed: true,
            fed: 0,
            viewer,
            cursor: 0,
            blink_from: None,
            blink_again: false,
            budget: RepaintBudget::new(),
            earned_ms: None,
        }
    }
}

impl Page {
    /// The half period of the cursor's blink on the decoder's clock.
    const BLINK_US: u64 = interface_core::regis::BLINK_MILLISECONDS * MICROSECONDS_PER_MILLISECOND;

    /// Whether the cursor blinks: keys go to the channel and the sub-page
    /// has a cursor drawing.
    const fn blinks(&self) -> bool {
        self.viewer.typing && self.cursor != 0
    }

    /// Gives the renderer the viewer state as it stands at `now_us`: the
    /// drawings hovered and pressed (REQ-6374), and the cursor drawing
    /// hidden through every second half period of its blink (REQ-6378).
    /// The blink begins again, with the cursor shown, when keys start
    /// going to the channel, when the cursor drawing changes and when a
    /// commit arrives, as a key typed moves the cursor by one. The picture
    /// is owed a repaint when the renderer says it may differ; a drawing
    /// with no look, and a cursor that does not exist, cost none.
    fn apply_viewer(&mut self, now_us: u64) {
        let mut hidden = 0;
        if self.blinks() {
            let from = match self.blink_from {
                Some(from) if !self.blink_again => from,
                _ => now_us,
            };
            self.blink_from = Some(from);
            let halves = now_us.saturating_sub(from).checked_div(Self::BLINK_US).unwrap_or(0);
            if !halves.is_multiple_of(2) {
                hidden = self.cursor;
            }
        } else {
            self.blink_from = None;
        }
        self.blink_again = false;
        let state = ViewerState { hovered: self.viewer.hovered, pressed: self.viewer.pressed, hidden };
        self.changed |= self.terminal.set_viewer_state(state);
    }

    /// When the cursor next shows or hides, if it blinks.
    fn next_blink_us(&self, now_us: u64) -> Option<u64> {
        let from = self.blink_from.filter(|_| self.blinks())?;
        let halves = now_us.saturating_sub(from).checked_div(Self::BLINK_US).unwrap_or(0);
        Some(from.saturating_add(halves.saturating_add(1).saturating_mul(Self::BLINK_US)))
    }

    /// Credits the repaint budget for the time since it was last credited.
    fn earn(&mut self, now_us: u64) {
        let milliseconds = now_us.checked_div(MICROSECONDS_PER_MILLISECOND).unwrap_or(0);
        if let Some(earned) = self.earned_ms {
            self.budget.earn(milliseconds.saturating_sub(earned));
        }
        self.earned_ms = Some(milliseconds);
    }
}

/// A sub-page's picture, ready to send.
pub struct Drawn {
    /// The page.
    pub page: u16,
    /// The sub-code.
    pub sub: u16,
    /// The picture.
    pub picture: RegisPicture,
    /// Its pick record.
    pub pick: RegisPick,
}

/// A sub-page's drawing links, whole: its page, its sub-code, and each
/// drawing with the target it leads to.
#[cfg(feature = "links")]
pub type DrawingLinks = (u16, u16, Vec<(u16, String)>);

/// A sub-page's drawing links being received: the parts of one sending.
#[cfg(feature = "links")]
struct LinkParts {
    key: (u16, u16),
    /// The part that must come next, and how many there are.
    next: u8,
    parts: u8,
    links: Vec<(u16, String)>,
}

// The trusted core is sent as many links as the engine's record carries.
#[cfg(feature = "links")]
const _: () = assert!(protocol::regis::MAX_REGIS_LINKS == neoteletext_web::records::drawing_links::MAX_DRAWING_LINKS);

/// Every sub-page's ReGIS a channel has sent.
#[derive(Default)]
pub struct RegisPages {
    pages: BTreeMap<(u16, u16), Page>,
    /// Records read.
    records: u64,
    /// The drawing links being received, of one sub-page at a time.
    #[cfg(feature = "links")]
    link_parts: Option<LinkParts>,
    /// Sub-pages' drawing links received whole, for the trusted core.
    #[cfg(feature = "links")]
    links: Vec<DrawingLinks>,
}

impl RegisPages {
    /// Reads one REGIS record: its bytes go to its sub-page's terminal.
    pub fn read(&mut self, record: &RegisRecord<'_>) {
        let key = (record.page.mpp(), record.subcode.value());
        self.records = self.records.saturating_add(1);
        let fresh = self.pages.get(&key).is_none_or(|page| page.generation != record.generation);
        if fresh {
            // A stream is followed from its first byte: one joined part
            // way through waits for the next generation.
            if record.offset != 0 {
                if let Some(page) = self.pages.get_mut(&key) {
                    page.lost = true;
                }
                return;
            }
            if !self.pages.contains_key(&key) && self.pages.len() >= MAX_REGIS_PAGES {
                let oldest = self.pages.iter().min_by_key(|(_, page)| page.fed).map(|(key, _)| *key);
                if let Some(oldest) = oldest {
                    self.pages.remove(&oldest);
                }
            }
            // What the pointer was doing, and which drawing is the cursor,
            // are not the stream's to change.
            let (viewer, cursor) = self.pages.get(&key).map(|page| (page.viewer, page.cursor)).unwrap_or_default();
            let mut page = Page::new(record.generation, viewer);
            page.cursor = cursor;
            self.pages.insert(key, page);
        }
        let Some(page) = self.pages.get_mut(&key) else { return };
        page.fed = self.records;
        if page.lost || record.offset != page.next {
            page.lost = true;
            return;
        }
        page.next = page.next.saturating_add(u32::try_from(record.data.len()).unwrap_or(u32::MAX));
        let events = page.terminal.feed_regis(&mut page.screen, record.data);
        // A commit, or drawing outside numbered drawings, changes what the
        // sub-page shows. The other events ask for things a page does not
        // do: a pause (no local animation) or a reply to the host.
        let committed = events.iter().any(|event| matches!(event, Event::Committed(_)));
        page.changed |= committed | page.screen.take_changed();
        // The channel moves the cursor by a commit after each key: it is
        // shown again at once, never hidden where the viewer looks for it.
        page.blink_again |= committed;
    }

    /// The trusted core said what the viewer's pointer is doing to a
    /// sub-page's picture (REQ-6374). A sub-page with no ReGIS has no
    /// picture for it to change.
    pub fn set_viewer(&mut self, page: u16, sub: u16, viewer: RegisViewer) {
        if let Some(held) = self.pages.get_mut(&(page, sub)) {
            held.viewer = viewer;
        }
    }

    /// The channel named the drawing that is a sub-page's cursor, or 0
    /// for none (the engine's REQ-6371). A sub-page with no ReGIS yet has
    /// nothing to blink; its channel names the cursor again with its ReGIS.
    pub fn set_cursor(&mut self, page: u16, sub: u16, drawing: u16) {
        if let Some(held) = self.pages.get_mut(&(page, sub)).filter(|held| held.cursor != drawing) {
            held.cursor = drawing;
            held.blink_again = true;
        }
    }

    /// Reads one `DRAWING_LINKS` record (the engine's REQ-6370). Part 0
    /// begins a sub-page's links again and later parts add to them, in
    /// order; when the last has come the links are kept for the trusted
    /// core. The engine's reader has checked each record; across the parts
    /// a drawing named twice, or a link past the most a sub-page has,
    /// drops the whole sending, as does a part out of its place.
    #[cfg(feature = "links")]
    pub fn read_links(&mut self, record: &neoteletext_web::records::drawing_links::DrawingLinksRecord<'_>) {
        let key = (record.page.mpp(), record.subcode.value());
        if record.part == 0 {
            self.link_parts = Some(LinkParts { key, next: 0, parts: record.parts, links: Vec::new() });
        }
        let Some(mut parts) = self.link_parts.take().filter(|parts| parts.key == key && parts.next == record.part && parts.parts == record.parts) else {
            return;
        };
        for link in record.links() {
            let named_twice = parts.links.iter().any(|(drawing, _)| *drawing == link.drawing);
            // A target is printable ASCII, as the reader checked.
            let Ok(target) = core::str::from_utf8(link.target) else { return };
            if named_twice || parts.links.len() >= protocol::regis::MAX_REGIS_LINKS {
                return;
            }
            parts.links.push((link.drawing, target.to_owned()));
        }
        parts.next = parts.next.saturating_add(1);
        if parts.next == parts.parts {
            // Only the latest of a sub-page's links is waiting to be sent.
            self.links.retain(|(page, sub, _)| (*page, *sub) != key);
            self.links.push((key.0, key.1, parts.links));
        } else {
            self.link_parts = Some(parts);
        }
    }

    /// The drawing links received whole since they were last taken.
    #[cfg(feature = "links")]
    pub fn take_links(&mut self) -> Vec<DrawingLinks> {
        core::mem::take(&mut self.links)
    }

    /// Whether the channel has sent ReGIS for a page.
    #[must_use]
    pub fn has(&self, page: u16) -> bool {
        self.pages.keys().any(|(held, _)| *held == page)
    }

    /// Marks a page's pictures as not yet sent: the page came on screen.
    pub fn show_again(&mut self, page: u16) {
        for (_, held) in self.pages.iter_mut().filter(|((held, _), _)| *held == page) {
            held.changed = true;
        }
    }

    /// The sub-page of `page` on screen: `sub`, or with none, or the
    /// sub-code that names none, the page's first with any ReGIS.
    fn shown(&self, page: u16, sub: Option<u16>) -> Option<(u16, u16)> {
        match sub {
            // The sub-page on screen, and no other: a viewer stepping
            // through a page's sub-pages must never be shown the picture
            // of one that is not on screen while this one's is on its way.
            Some(sub) if sub != neoteletext_core::limits::SUBCODE_ANY => Some((page, sub)),
            _ => self.pages.keys().find(|(held, _)| *held == page).copied(),
        }
    }

    /// The picture of a page's sub-page on screen (see [`Self::shown`]),
    /// if what it shows changed since it was last taken and its repaint
    /// budget allows a repaint at `now_us` of the decoder's clock. A
    /// repaint the budget holds back stays owed: [`Self::owed_us`] says
    /// when to ask again, and the picture then made is of the latest
    /// scene, however much changed meanwhile (REQ-6380).
    pub fn take(&mut self, page: u16, sub: Option<u16>, now_us: u64) -> Option<Drawn> {
        let key = self.shown(page, sub)?;
        let held = self.pages.get_mut(&key)?;
        held.apply_viewer(now_us);
        if !held.changed {
            return None;
        }
        held.earn(now_us);
        if !held.budget.may_repaint() {
            return None;
        }
        held.changed = false;
        held.terminal.compose(&held.screen, &mut held.picture);
        held.budget.spend(held.picture.work());
        let packed = pack(held.picture.canvas().pixels())?;
        // What the sub-page's drawing has cost: on the base since its
        // stream began, and to paint its numbered drawings this time.
        let work = held.terminal.base_work().saturating_add(held.picture.work());
        let picture = RegisPicture { commits: held.picture.commits(), work, palette: held.terminal.palette().colours(), packed };
        let pick = RegisPick::of(held.picture.pick_record(), held.picture.origins());
        Some(Drawn { page: key.0, sub: key.1, picture, pick })
    }

    /// When a page's sub-page on screen next needs the decoder's loop, in
    /// microseconds of its clock: when the repaint it is owed may be made,
    /// or when its cursor next shows or hides; `None` when neither. The
    /// loop waits on the earliest of these.
    pub fn owed_us(&mut self, page: u16, sub: Option<u16>, now_us: u64) -> Option<u64> {
        let key = self.shown(page, sub)?;
        let held = self.pages.get_mut(&key)?;
        held.apply_viewer(now_us);
        let blink = held.next_blink_us(now_us);
        if !held.changed {
            return blink;
        }
        held.earn(now_us);
        let wait = held.budget.milliseconds_until_repaint().saturating_mul(MICROSECONDS_PER_MILLISECOND);
        Some(blink.map_or(now_us.saturating_add(wait), |blink| blink.min(now_us.saturating_add(wait))))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test reports a failure by panicking, and indexes data it built to a known size")]

    use neoteletext_core::types::{PageAddress, SubCode};
    use protocol::regis::{REGIS_WIDTH, unpack};

    use super::*;

    fn record(page: u16, generation: u16, offset: u32, data: &[u8]) -> RegisRecord<'_> {
        RegisRecord { page: PageAddress::from_mpp(page).unwrap(), subcode: SubCode::new(0), generation, offset, data }
    }

    /// DEC-6335: a sub-page's records, in order, draw its picture; a
    /// numbered drawing is in the picture and its pick record.
    #[test]
    fn records_in_order_draw_a_picture_with_its_pick_record() {
        let mut pages = RegisPages::default();
        let first = b"P[10,10]V[30,10]D(B1)W(I3)V[0,0][19,0]D(E)";
        pages.read(&record(0x100, 1, 0, first));
        pages.read(&record(0x100, 1, u32::try_from(first.len()).unwrap(), b"D(S1(X100,Y50,I1),C)"));
        let drawn = pages.take(0x100, Some(0), 0).unwrap();
        let pixels = unpack(&drawn.picture.packed);
        assert_eq!(pixels[10 * REGIS_WIDTH + 20], 7, "the base");
        assert!(drawn.picture.work >= 21 + 20, "the base's line and the drawing's are both counted");
        assert_eq!(pixels[50 * REGIS_WIDTH + 110], 3, "the numbered drawing");
        assert_eq!(drawn.picture.commits, 1);
        assert!(drawn.picture.work > 0);
        let picked = drawn.pick.at(110, 50);
        assert_eq!((picked.drawing, picked.x, picked.y), (1, 10, 0));
        assert!(pages.take(0x100, Some(0), 0).is_none(), "nothing changed since");
        pages.show_again(0x100);
        assert!(pages.take(0x100, None, 0).is_some(), "sent again when the page comes on screen");
    }

    /// A record lost shows nothing more of its generation; the next
    /// generation starts the sub-page again.
    #[test]
    fn a_lost_record_stops_a_generation_and_the_next_starts_again() {
        let mut pages = RegisPages::default();
        pages.read(&record(0x100, 1, 0, b"P[10,10]"));
        let _ = pages.take(0x100, None, 0);
        pages.read(&record(0x100, 1, 99, b"V[30,10]"));
        assert!(pages.take(0x100, None, 0).is_none(), "the record after a gap is not drawn");
        pages.read(&record(0x100, 2, 5, b"V[30,10]"));
        assert!(pages.take(0x100, None, 0).is_none(), "a generation joined part way through is not drawn");
        pages.read(&record(0x100, 3, 0, b"P[10,10]V[30,10]"));
        let drawn = pages.take(0x100, None, 0).unwrap();
        assert_eq!(unpack(&drawn.picture.packed)[10 * REGIS_WIDTH + 20], 7);
    }

    /// A page with several sub-pages drawn in ReGIS shows the one on
    /// screen and no other, even while that one's ReGIS has not come.
    #[test]
    fn only_the_sub_page_on_screen_is_taken() {
        let sub = |code: u16, data: &'static [u8]| RegisRecord {
            page: PageAddress::from_mpp(0x101).unwrap(),
            subcode: SubCode::new(code),
            generation: 1,
            offset: 0,
            data,
        };
        let mut pages = RegisPages::default();
        pages.read(&sub(1, b"P[10,10]V[30,10]"));
        assert!(pages.take(0x101, Some(2), 0).is_none(), "sub-page 2 is on screen and has no ReGIS yet");
        pages.read(&sub(2, b"P[10,20]V[30,20]"));
        let drawn = pages.take(0x101, Some(2), 0).unwrap();
        assert_eq!(drawn.sub, 2);
        assert_eq!(unpack(&drawn.picture.packed)[20 * REGIS_WIDTH + 20], 7);
        assert_eq!(pages.take(0x101, Some(1), 0).unwrap().sub, 1);
    }

    /// The sub-pages kept are bounded: one more forgets the one fed longest ago.
    #[test]
    fn the_sub_pages_kept_are_bounded() {
        let mut pages = RegisPages::default();
        for page in 0..=u16::try_from(MAX_REGIS_PAGES).unwrap() {
            pages.read(&record(0x100 + page, 1, 0, b"V[5,5]"));
        }
        assert!(!pages.has(0x100));
        assert!(pages.has(0x101) && pages.has(0x100 + u16::try_from(MAX_REGIS_PAGES).unwrap()));
    }

    /// REQ-6380: a stream of minimal commits to a picture that is dear to
    /// repaint, each followed by a wish to draw it, buys no more repaint
    /// work than the budget allows for the time passed; the repaint held
    /// back is made when the budget allows, and shows the last scene.
    #[test]
    fn repaints_are_held_to_the_budget_and_the_last_scene_is_shown() {
        use regis::limits::{MAX_REBUILD_WORK, REPAINT_WORK_HELD, REPAINT_WORK_PER_SECOND};

        /// The commits sent, a millisecond of the decoder's clock apart.
        const COMMITS: u32 = 200;
        // A filled screen used 100 times: about 77 million work units a repaint.
        let uses = (2..102).map(|use_of| format!("D(S{use_of}(R1))")).collect::<Vec<_>>().concat();
        let scene = format!("D(B1)F(P[0,0]V[799,0][799,479][0,479])D(E)D(S1(V0)){uses}D(C)");
        let mut pages = RegisPages::default();
        pages.read(&record(0x100, 1, 0, scene.as_bytes()));
        let mut offset = u32::try_from(scene.len()).unwrap();
        let (mut work, mut frames, mut last) = (0_u64, 0_u32, None);
        for commit in 0..COMMITS {
            let moved = format!("D(S2(X{commit}),C)");
            pages.read(&record(0x100, 1, offset, moved.as_bytes()));
            offset += u32::try_from(moved.len()).unwrap();
            let now_us = u64::from(commit) * MICROSECONDS_PER_MILLISECOND;
            if let Some(drawn) = pages.take(0x100, Some(0), now_us) {
                // The work reported includes the base's, which is none here.
                work += drawn.picture.work;
                frames += 1;
                last = Some(drawn.picture.commits);
            } else {
                let owed = pages.owed_us(0x100, Some(0), now_us).unwrap();
                assert!(owed > now_us, "a repaint held back is owed at a later time");
            }
        }
        let seconds_thousandths = u64::from(COMMITS);
        let bound = REPAINT_WORK_HELD + REPAINT_WORK_PER_SECOND * seconds_thousandths / 1000 + MAX_REBUILD_WORK;
        assert!(work <= bound, "{frames} repaints cost {work}, over the bound of {bound}");
        assert!((2..COMMITS / 10).contains(&frames), "some repaints are made and most are held back: {frames}");
        assert_ne!(last, Some(COMMITS + 1), "the last commit's repaint was held back");
        // The repaint owed is made at the time given, and is of the last scene.
        let now_us = u64::from(COMMITS) * MICROSECONDS_PER_MILLISECOND;
        let owed = pages.owed_us(0x100, Some(0), now_us).unwrap();
        assert!(pages.take(0x100, Some(0), owed - MICROSECONDS_PER_MILLISECOND).is_none(), "not a millisecond sooner");
        let drawn = pages.take(0x100, Some(0), owed).unwrap();
        assert_eq!(drawn.picture.commits, COMMITS + 1);
        assert_eq!(pages.owed_us(0x100, Some(0), owed), None, "nothing is owed once it is made");
    }

    /// With the budget in hand, each commit's picture is made at once.
    #[test]
    fn a_cheap_picture_is_repainted_at_every_commit() {
        let mut pages = RegisPages::default();
        let scene = "D(B1)W(I3)V[0,0][19,0]D(E)D(S1(X100,Y50),C)";
        pages.read(&record(0x100, 1, 0, scene.as_bytes()));
        let mut offset = u32::try_from(scene.len()).unwrap();
        for commit in 0..50_u32 {
            let drawn = pages.take(0x100, Some(0), 0).unwrap();
            assert_eq!(drawn.picture.commits, commit + 1);
            let moved = format!("D(S1(X{commit}),C)");
            pages.read(&record(0x100, 1, offset, moved.as_bytes()));
            offset += u32::try_from(moved.len()).unwrap();
        }
    }

    /// REQ-6374: the hovered drawing is painted with its hover look and the
    /// pressed one with its pressed look, with no bytes from the channel;
    /// a drawing with no look, and a viewer state that changes nothing,
    /// cost no repaint.
    #[test]
    fn the_viewer_state_paints_looks_and_costs_nothing_where_there_are_none() {
        let square = |colour: u8| format!("W(I{colour})V[0,0][19,0][19,19][0,19][0,0]");
        let scene = format!(
            "D(B1){}D(E)D(B2){}D(E)D(B3){}D(E)D(B4){}D(E)D(S1(X100,Y50,I1,H2,A3),S2(V0),S3(V0),S4(X300,Y50,I1),C)",
            square(3),
            square(5),
            square(6),
            square(7)
        );
        let mut pages = RegisPages::default();
        pages.read(&record(0x100, 1, 0, scene.as_bytes()));
        let at = |pages: &mut RegisPages| pages.take(0x100, Some(0), 0).map(|drawn| unpack(&drawn.picture.packed)[50 * REGIS_WIDTH + 100]);
        assert_eq!(at(&mut pages), Some(3));
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 1, pressed: 0, typing: true });
        assert_eq!(at(&mut pages), Some(5), "the hover look");
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 1, pressed: 1, typing: true });
        assert_eq!(at(&mut pages), Some(6), "the pressed look wins");
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 1, pressed: 1, typing: true });
        assert_eq!(at(&mut pages), None, "the same state again repaints nothing");
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 4, pressed: 0, typing: true });
        assert_eq!(at(&mut pages), Some(3), "drawing 1 is itself again");
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 0, pressed: 4, typing: true });
        assert_eq!(at(&mut pages), None, "drawing 4 has no looks: nothing to repaint");
        // Another sub-page's viewer state is not this one's.
        pages.set_viewer(0x100, 9, RegisViewer { hovered: 1, pressed: 0, typing: true });
        assert_eq!(at(&mut pages), None);
    }

    /// REQ-6378: while keys go to the channel the cursor drawing is shown
    /// and hidden in turn, each for the half period of the typing cursor's
    /// blink; a commit, as a key typed sends, shows it again for a full
    /// half period; when keys stop it is left as the channel drew it.
    #[test]
    fn the_cursor_drawing_blinks_while_keys_go_to_the_channel() {
        /// The half period, on the decoder's clock.
        const HALF: u64 = interface_core::regis::BLINK_MILLISECONDS * MICROSECONDS_PER_MILLISECOND;
        let scene = "D(B1)W(I7)T'Hi'D(E)D(B2)W(I3)V[0,0][8,0]D(E)D(S1(X60,Y200),S2(X78,Y200),C)";
        let mut pages = RegisPages::default();
        pages.read(&record(0x100, 1, 0, scene.as_bytes()));
        // The cursor's pixel in the picture taken at a time, if one was made.
        let cursor_at = |pages: &mut RegisPages, now: u64| pages.take(0x100, Some(0), now).map(|drawn| unpack(&drawn.picture.packed)[200 * REGIS_WIDTH + 80]);
        assert_eq!(cursor_at(&mut pages, 0), Some(3));
        pages.set_cursor(0x100, 0, 2);
        assert_eq!(cursor_at(&mut pages, 0), None, "keys do not go to the channel: it does not blink");
        assert_eq!(pages.owed_us(0x100, Some(0), 0), None);
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 0, pressed: 0, typing: true });
        assert_eq!(cursor_at(&mut pages, 1000), None, "shown for the first half period, as it already is");
        assert_eq!(pages.owed_us(0x100, Some(0), 2000), Some(1000 + HALF), "the loop wakes when it is to hide");
        assert_eq!(cursor_at(&mut pages, 1000 + HALF), Some(0), "hidden for the second");
        assert_eq!(cursor_at(&mut pages, 1000 + 2 * HALF), Some(3), "and shown again");
        assert_eq!(cursor_at(&mut pages, 1000 + 3 * HALF), Some(0));
        // A key typed: the channel moves the cursor by a commit, and it is
        // shown at once at its new place, for a full half period.
        let moved = "D(S2(X87),C)";
        pages.read(&record(0x100, 1, u32::try_from(scene.len()).unwrap(), moved.as_bytes()));
        let typed_at = 1000 + 3 * HALF + 5000;
        let drawn = pages.take(0x100, Some(0), typed_at).unwrap();
        assert_eq!(unpack(&drawn.picture.packed)[200 * REGIS_WIDTH + 89], 3);
        assert_eq!(pages.owed_us(0x100, Some(0), typed_at), Some(typed_at + HALF));
        assert_eq!(cursor_at(&mut pages, typed_at + HALF - 1), None, "still shown just before the half period ends");
        // Keys stop going to the channel while it is hidden: it comes back and stays.
        assert!(cursor_at(&mut pages, typed_at + HALF).is_some());
        pages.set_viewer(0x100, 0, RegisViewer::default());
        let drawn = pages.take(0x100, Some(0), typed_at + HALF + 1).unwrap();
        assert_eq!(unpack(&drawn.picture.packed)[200 * REGIS_WIDTH + 89], 3);
        assert_eq!(pages.owed_us(0x100, Some(0), typed_at + 5 * HALF), None);
        // A cursor drawing that does not exist blinks nothing and costs no repaint.
        pages.set_cursor(0x100, 0, 99);
        pages.set_viewer(0x100, 0, RegisViewer { hovered: 0, pressed: 0, typing: true });
        assert_eq!(cursor_at(&mut pages, typed_at + 9 * HALF), None);
        assert_eq!(cursor_at(&mut pages, typed_at + 10 * HALF), None);
    }

    /// REQ-6376: a sub-page's drawing links, in one record or several in
    /// order, are kept whole for the trusted core; a part out of its
    /// place, or a drawing named in two parts, drops the sending.
    #[cfg(feature = "links")]
    #[test]
    fn drawing_links_are_gathered_from_their_parts() {
        use neoteletext_web::records::drawing_links::{DrawingLink, encode};
        use neoteletext_web::records::{ChannelRecord, Read, RecordReader};

        // Sends `links` as records and reads each into `pages`, leaving
        // out the parts `skip` names.
        let send = |pages: &mut RegisPages, links: &[DrawingLink<'_>], skip: Option<u8>| {
            let mut reader = RecordReader::new();
            encode(PageAddress::from_mpp(0x105).unwrap(), SubCode::new(0), links, 3, 0, &mut |packet| {
                if let Read::Record(ChannelRecord::DrawingLinks(record)) = reader.push(&packet)
                    && skip != Some(record.part)
                {
                    pages.read_links(&record);
                }
            })
            .unwrap();
        };
        let mut pages = RegisPages::default();
        send(&mut pages, &[DrawingLink { drawing: 11, target: b"100" }, DrawingLink { drawing: 13, target: b"web:https://example.invalid/" }], None);
        assert_eq!(pages.take_links(), [(0x105, 0, vec![(11, "100".to_owned()), (13, "web:https://example.invalid/".to_owned())])]);
        assert_eq!(pages.take_links(), []);
        // Three long links take two records: both parts, then the first lost.
        let long = format!("web:https://example.invalid/{}", "a".repeat(3000));
        let three: Vec<DrawingLink<'_>> = (1..=3).map(|drawing| DrawingLink { drawing, target: long.as_bytes() }).collect();
        send(&mut pages, &three, None);
        assert_eq!(pages.take_links().first().map(|(_, _, links)| links.len()), Some(3));
        send(&mut pages, &three, Some(0));
        assert_eq!(pages.take_links(), [], "the first part was lost: the rest is not a sub-page's links");
        // No links at all are a sub-page's links: none.
        send(&mut pages, &[], None);
        assert_eq!(pages.take_links(), [(0x105, 0, Vec::new())]);
    }
}
