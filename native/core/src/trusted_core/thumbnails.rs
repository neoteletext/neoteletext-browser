//! The filmstrip's thumbnails (REQ-6883; neoteletext-common's
//! `docs/tab-thumbnails.md` section 8): the interface names the tabs in
//! view that have no still, and the trusted core alone decides what is
//! read from the store, fetched and drawn for them, a few at a time, and
//! sends each drawn picture back. Private tabs are never fetched, read
//! from the store or written to it.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use neoteletext::snapshots::{Misses, Place, SNAPSHOT_DRAWS_AT_ONCE, SNAPSHOT_FETCHES_AT_ONCE, SNAPSHOT_FETCHES_PER_HOST, SNAPSHOT_REFRESH, Snapshot, Store};
use protocol::console::Level;
use protocol::{ThumbnailImage, ToInterface};

use super::Core;
use super::processes::{Reach, SnapshotDrawn, start_snapshot_draw, start_snapshot_fetch};

/// A tab the interface asked a picture for, as the trusted core found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Wanted {
    channel: u32,
    place: Place,
    host: String,
}

/// A drawing waiting for a decoder: its place, its bytes, and whether they
/// were fetched now (and are stored once drawn) or read from the store.
struct ToDraw {
    place: Place,
    bytes: Arc<Vec<u8>>,
    fresh: bool,
}

/// The thumbnails' state: the store, the misses, what is wanted, and what
/// is being fetched and drawn.
#[derive(Default)]
pub(super) struct Thumbnails {
    pub(super) store: Store,
    misses: Misses,
    /// The latest request, resolved to places: only the latest counts.
    wanted: Vec<Wanted>,
    /// Fetches running: their job, place and host.
    fetching: Vec<(u64, Place, String)>,
    /// Drawings running: their job, place and whether fetched now.
    drawing: Vec<(u64, Place, bool)>,
    /// Drawings waiting for a decoder.
    to_draw: VecDeque<ToDraw>,
    /// The tabs sent a picture of a place this run: not drawn again from
    /// the store for them.
    sent: BTreeSet<(u32, Place)>,
    next_job: u64,
}

impl Thumbnails {
    /// The store in the settings folder, if one.
    pub(super) fn new(settings_folder: Option<&std::path::Path>) -> Self {
        Self { store: Store::new(settings_folder), ..Self::default() }
    }

    /// Whether `place` is being fetched, drawn or waits to be drawn.
    fn busy(&self, place: &Place) -> bool {
        self.fetching.iter().any(|(_, busy, _)| busy == place)
            || self.drawing.iter().any(|(_, busy, _)| busy == place)
            || self.to_draw.iter().any(|waiting| &waiting.place == place)
    }

    /// Whether another fetch may start to `host` now.
    fn may_fetch(&self, host: &str) -> bool {
        self.fetching.len() < SNAPSHOT_FETCHES_AT_ONCE && self.fetching.iter().filter(|(_, _, busy)| busy == host).count() < SNAPSHOT_FETCHES_PER_HOST
    }

    fn job(&mut self) -> u64 {
        self.next_job = self.next_job.wrapping_add(1);
        self.next_job
    }

    /// Forgets every miss (history cleared).
    pub(super) fn forget_misses(&mut self) {
        self.misses.clear();
    }
}

/// What the scheduler decided to start for one pass over the wanted tabs.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Plan {
    /// Fetches to start: the place and its host.
    pub(super) fetch: Vec<(Place, String)>,
    /// Stored snapshots to draw.
    pub(super) draw_stored: Vec<Place>,
}

/// One pass over `wanted`, left to right (REQ-6883's order of sources):
/// a stored snapshot not yet sent is drawn; a place with none, or one
/// older than `SNAPSHOT_REFRESH`, is fetched unless it missed recently,
/// within the fetches' bounds; a place already in hand is left.
fn plan(thumbnails: &Thumbnails, wanted: &[Wanted], stored: impl Fn(&Place) -> Option<i64>, now_utc: i64, now: Instant) -> Plan {
    let mut plan = Plan::default();
    let mut fetching: Vec<String> = thumbnails.fetching.iter().map(|(_, _, host)| host.clone()).collect();
    let refresh = i64::try_from(SNAPSHOT_REFRESH.as_secs()).unwrap_or(i64::MAX);
    for want in wanted {
        if thumbnails.busy(&want.place) || plan.draw_stored.contains(&want.place) || plan.fetch.iter().any(|(place, _)| place == &want.place) {
            continue;
        }
        let fetched = stored(&want.place);
        if fetched.is_some() && !thumbnails.sent.contains(&(want.channel, want.place.clone())) {
            plan.draw_stored.push(want.place.clone());
        }
        let stale = fetched.is_none_or(|fetched| now_utc.saturating_sub(fetched) >= refresh);
        let host_free = fetching.iter().filter(|host| **host == want.host).count() < SNAPSHOT_FETCHES_PER_HOST;
        if stale && !thumbnails.misses.recent(&want.place, now) && fetching.len() < SNAPSHOT_FETCHES_AT_ONCE && host_free {
            fetching.push(want.host.clone());
            plan.fetch.push((want.place.clone(), want.host.clone()));
        }
    }
    plan
}

/// The image a drawn snapshot is sent as, checked and encoded as a page's
/// frame is; none when it is not a page's image.
fn thumbnail_image(image: protocol::decoder::SnapshotImage) -> Option<ThumbnailImage> {
    match image {
        protocol::decoder::SnapshotImage::Frame { palette, screen, indices } => {
            protocol::frame::encode(&palette, screen, &indices, None).map(ThumbnailImage::Frame)
        }
        protocol::decoder::SnapshotImage::Regis(picture) => picture.whole().then_some(ThumbnailImage::Regis(picture)),
    }
}

impl Core {
    /// The places the tabs of every window that is not private show:
    /// what the store keeps.
    fn shown_places(&self) -> Vec<Place> {
        self.session.all_tabs().filter(|tab| !tab.private).filter_map(|tab| tab.address().and_then(Place::of)).collect()
    }

    /// The store keeps only what the tabs show (REQ-6886 item 7), and,
    /// at start, nothing else at all (item 5).
    pub(super) fn keep_shown_snapshots(&self) {
        let places = self.shown_places();
        self.thumbnails.store.keep_only(&places);
    }

    /// The filmstrip's tabs in view without stills (REQ-6883): numbers not
    /// of the window's tabs, and private tabs, are dropped; with the
    /// setting off, or in a private window, nothing is wanted.
    pub(super) fn thumbnails_wanted(&mut self, channels: &[u32]) {
        self.thumbnails.wanted.clear();
        if !self.settings.tab_pictures || self.session.private_window {
            return;
        }
        for &channel in channels {
            let Some(tab) = self.session.tabs.iter().find(|tab| tab.channel.id == channel && !tab.private) else { continue };
            let Some(address) = tab.address() else { continue };
            let Some(place) = Place::of(address) else { continue };
            if !self.thumbnails.wanted.iter().any(|want| want.channel == channel) {
                self.thumbnails.wanted.push(Wanted { channel, place, host: address.host.clone() });
            }
        }
        self.pump_thumbnails();
    }

    /// Starts what the wanted tabs need, within the bounds.
    fn pump_thumbnails(&mut self) {
        let wanted = self.thumbnails.wanted.clone();
        let store = &self.thumbnails.store;
        let plan =
            plan(&self.thumbnails, &wanted, |place| store.read(place).map(|snapshot| snapshot.fetched), neoteletext::calendar::utc_now(), Instant::now());
        for place in plan.draw_stored {
            if let Some(snapshot) = self.thumbnails.store.read(&place) {
                self.thumbnails.to_draw.push_back(ToDraw { place, bytes: Arc::new(snapshot.body), fresh: false });
            }
        }
        for (place, host) in plan.fetch {
            if !self.thumbnails.may_fetch(&host) {
                continue;
            }
            let job = self.thumbnails.job();
            let reach = Reach::of(&self.allow_local, &self.settings);
            let Some(broker) = self.broker.clone() else { continue };
            match start_snapshot_fetch(&broker, &place.channel, place.page_asked(), place.sub, &reach, job, self.events.clone()) {
                Ok(()) => self.thumbnails.fetching.push((job, place, host)),
                Err(error) => {
                    self.log(Level::Warning, None, &format!("a snapshot fetcher could not start: {error}"));
                    self.thumbnails.misses.missed(&place, Instant::now());
                }
            }
        }
        self.start_draws();
    }

    /// Starts waiting drawings, at most `SNAPSHOT_DRAWS_AT_ONCE`.
    fn start_draws(&mut self) {
        while self.thumbnails.drawing.len() < SNAPSHOT_DRAWS_AT_ONCE {
            let Some(waiting) = self.thumbnails.to_draw.pop_front() else { return };
            let job = self.thumbnails.job();
            let Some(broker) = self.broker.clone() else { return };
            match start_snapshot_draw(&broker, waiting.bytes, waiting.place.page_asked(), waiting.place.sub, job, self.events.clone()) {
                Ok(()) => self.thumbnails.drawing.push((job, waiting.place, waiting.fresh)),
                Err(error) => {
                    self.log(Level::Warning, None, &format!("a snapshot decoder could not start: {error}"));
                    self.thumbnails.misses.missed(&waiting.place, Instant::now());
                }
            }
        }
    }

    /// A snapshot fetcher answered: its bytes wait for a decoder, and a
    /// miss is remembered (REQ-6883). A fetch started finishes and is
    /// drawn and stored even if its tab left the view.
    pub(super) fn snapshot_fetched(&mut self, job: u64, fetched: Result<Vec<u8>, String>) {
        let Some(at) = self.thumbnails.fetching.iter().position(|(running, _, _)| *running == job) else { return };
        let (_, place, _) = self.thumbnails.fetching.remove(at);
        match fetched {
            Ok(bytes) => self.thumbnails.to_draw.push_back(ToDraw { place, bytes: Arc::new(bytes), fresh: true }),
            Err(reason) => {
                self.log(Level::Info, None, &format!("no snapshot of page {:03X}: {reason}", place.page_asked()));
                self.thumbnails.misses.missed(&place, Instant::now());
            }
        }
        self.pump_thumbnails();
    }

    /// A snapshot decoder answered: a real page drawn is stored when it
    /// was fetched now, with its blanked source (REQ-6886 item 6), and sent
    /// to every tab of a window that is not private showing its place;
    /// anything else is a miss.
    pub(super) fn snapshot_drawn(&mut self, job: u64, drawn: Result<SnapshotDrawn, String>) -> Result<(), String> {
        let Some(at) = self.thumbnails.drawing.iter().position(|(running, _, _)| *running == job) else { return Ok(()) };
        let (_, place, fresh) = self.thumbnails.drawing.remove(at);
        let shown = self.shown_places().contains(&place);
        match drawn.map(|drawn| (drawn.page, drawn.sub, drawn.source, thumbnail_image(drawn.image))) {
            Ok((page, sub, source, Some(image))) if self.settings.tab_pictures && shown => {
                if fresh {
                    let snapshot = Snapshot { fetched: neoteletext::calendar::utc_now(), drawn: sub, body: source };
                    if let Err(error) = self.thumbnails.store.write(&place, &snapshot) {
                        self.log(Level::Warning, None, &format!("a snapshot could not be kept: {error}"));
                    }
                }
                self.send_thumbnail(&place, page, sub, &image)?;
            }
            Ok(_) => {}
            Err(reason) => {
                self.log(Level::Info, None, &format!("no snapshot of page {:03X} drawn: {reason}", place.page_asked()));
                self.thumbnails.misses.missed(&place, Instant::now());
            }
        }
        self.pump_thumbnails();
        Ok(())
    }

    /// Sends the picture of `place` to each window's tabs that show it.
    fn send_thumbnail(&mut self, place: &Place, page: u16, sub: u16, image: &ThumbnailImage) -> Result<(), String> {
        let channels: Vec<u32> =
            self.session.all_tabs().filter(|tab| !tab.private && tab.address().and_then(Place::of).as_ref() == Some(place)).map(|tab| tab.channel.id).collect();
        for channel in channels {
            self.thumbnails.sent.insert((channel, place.clone()));
            let message = ToInterface::Thumbnail { channel, page, sub, image: image.clone() };
            self.in_window_of(channel, |core| core.send(&message))?;
        }
        Ok(())
    }

    /// The setting turned off: nothing is wanted and every snapshot goes
    /// (REQ-6886 item 7).
    pub(super) fn tab_pictures_off(&mut self) {
        self.thumbnails.wanted.clear();
        self.thumbnails.to_draw.clear();
        self.thumbnails.store.clear(None);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;
    use neoteletext::snapshots::{INITIAL_PAGE_ASKED, WHOLE_PAGE};

    fn want(channel: u32, host: &str, page: u16) -> Wanted {
        Wanted { channel, place: Place { channel: format!("https://{host}/tt/"), page, sub: WHOLE_PAGE }, host: host.to_owned() }
    }

    /// REQ-6883: the tabs in view are served left to right, at most two
    /// fetches at once and one to any one host.
    #[test]
    fn only_tabs_in_view_are_fetched_two_at_a_time_and_one_per_host() {
        let thumbnails = Thumbnails::default();
        let wanted = [want(1, "a.example", 0x100), want(2, "a.example", 0x101), want(3, "b.example", 0x100), want(4, "c.example", 0x100)];
        let plan = plan(&thumbnails, &wanted, |_| None, 0, Instant::now());
        let hosts: Vec<&str> = plan.fetch.iter().map(|(_, host)| host.as_str()).collect();
        assert_eq!(hosts, ["a.example", "b.example"]);
        assert_eq!(plan.fetch[0].0.page, 0x100, "left to right");
        assert_eq!(plan.draw_stored, Vec::<Place>::new());
        assert_eq!(super::plan(&thumbnails, &[], |_| None, 0, Instant::now()), Plan::default(), "nothing for no tab in view");
    }

    /// REQ-6883: a miss is not asked again within its interval.
    #[test]
    fn a_miss_is_not_asked_again_within_its_interval() {
        let mut thumbnails = Thumbnails::default();
        let now = Instant::now();
        let wanted = [want(1, "a.example", 0x100)];
        thumbnails.misses.missed(&wanted[0].place, now);
        assert_eq!(plan(&thumbnails, &wanted, |_| None, 0, now), Plan::default());
        let later = now + neoteletext::snapshots::SNAPSHOT_MISS_RETRY;
        assert_eq!(plan(&thumbnails, &wanted, |_| None, 0, later).fetch.len(), 1);
    }

    /// REQ-6883 items 2 and 3: a stored snapshot is drawn; a fresh one is
    /// not fetched again; a stale one is drawn and fetched again, so the
    /// old shows meanwhile; once sent, it is not drawn again.
    #[test]
    fn a_stale_snapshot_is_shown_while_it_is_fetched_again() {
        let mut thumbnails = Thumbnails::default();
        let wanted = [want(1, "a.example", 0x100)];
        let refresh = i64::try_from(SNAPSHOT_REFRESH.as_secs()).unwrap();
        let fresh = plan(&thumbnails, &wanted, |_| Some(1_000), 1_000 + refresh - 1, Instant::now());
        assert_eq!((fresh.draw_stored.len(), fresh.fetch.len()), (1, 0));
        let stale = plan(&thumbnails, &wanted, |_| Some(1_000), 1_000 + refresh, Instant::now());
        assert_eq!((stale.draw_stored.len(), stale.fetch.len()), (1, 1));
        thumbnails.sent.insert((1, wanted[0].place.clone()));
        let sent = plan(&thumbnails, &wanted, |_| Some(1_000), 1_000, Instant::now());
        assert_eq!(sent, Plan::default());
    }

    /// The channels a test core's wanted list names.
    #[cfg(unix)]
    fn wanted_channels(core: &Core) -> Vec<u32> {
        core.thumbnails.wanted.iter().map(|want| want.channel).collect()
    }

    /// REQ-6883 and REQ-687's fifth exception: a request naming a tab of
    /// another window, a private tab or no tab at all is ignored for that
    /// number; one from a private window, or with tab pictures off, wants
    /// nothing.
    #[cfg(unix)]
    #[test]
    fn a_thumbnails_request_naming_another_windows_tab_is_ignored() {
        let mut core = super::super::test_support::test_core();
        let first = core.session.open("https://a.example/tt/").unwrap().id;
        let file = core.session.open("https://b.example/tt/302").unwrap().id;
        let here = core.session.window;
        let other = core.session.new_window(false).unwrap();
        assert!(core.session.switch_to(other));
        let elsewhere = core.session.open("https://c.example/tt/").unwrap().id;
        assert!(core.session.switch_to(here));
        core.thumbnails_wanted(&[elsewhere, file, 999, first, first]);
        assert_eq!(wanted_channels(&core), [file, first], "the window's own tabs, once each, in the order asked");
        assert_eq!(core.thumbnails.wanted[0].place, Place { channel: String::from("https://b.example/tt/"), page: 0x302, sub: WHOLE_PAGE });
        core.settings.tab_pictures = false;
        core.thumbnails_wanted(&[first]);
        assert_eq!(wanted_channels(&core), [], "tab pictures off");
    }

    /// REQ-6883, REQ-6886 item 8: a private tab is never wanted, so never
    /// fetched or read from the store; a private window wants nothing.
    #[cfg(unix)]
    #[test]
    fn a_private_tab_is_never_fetched_or_read_from_the_store() {
        let mut core = super::super::test_support::test_core();
        let public = core.session.open("https://a.example/tt/").unwrap().id;
        let private = core.session.open_private("https://b.example/tt/").unwrap().id;
        core.thumbnails_wanted(&[private, public]);
        assert_eq!(wanted_channels(&core), [public]);
        let window = core.session.new_window(true).unwrap();
        assert!(core.session.switch_to(window));
        let inside = core.session.open("https://c.example/tt/").unwrap().id;
        core.thumbnails_wanted(&[inside]);
        assert_eq!(wanted_channels(&core), [], "a private window");
    }

    /// The bytes of `bytes` with each one's parity bit cleared (EN 300 706
    /// clause 8.2: data bytes are seven bits and odd parity), so that text
    /// in packets can be looked for.
    fn without_parity(bytes: &[u8]) -> Vec<u8> {
        bytes.iter().map(|byte| byte & 0x7F).collect()
    }

    /// REQ-6886 item 6, end to end: a page with concealed text, drawn by
    /// the snapshot decoder's own answer and kept by the trusted core as a
    /// fetched snapshot is, leaves a file in the store that holds none of
    /// the concealed characters, while the text shown is kept.
    #[cfg(unix)]
    #[test]
    fn a_stored_snapshot_of_a_page_with_concealed_text_holds_none_of_it() {
        use neoteletext_core::consts::COLUMNS;
        use neoteletext_core::packet::Packet;
        use neoteletext_core::types::{PageAddress, SubCode};
        use neoteletext_formats::page::PageFile;
        use protocol::decoder::{FromDecoder, read_from_decoder};
        /// Spacing attributes (EN 300 706 clause 12.2, Table 26).
        const ALPHA_WHITE: u8 = 0x07;
        const CONCEAL: u8 = 0x18;
        const HIDDEN: &[u8] = b"XYZZY";

        let mut file = PageFile { address: PageAddress::from_mpp(INITIAL_PAGE_ASKED).unwrap(), subcode: SubCode::new(0), ..PageFile::default() };
        let mut codes = [b' '; COLUMNS];
        let text = [&[ALPHA_WHITE][..], b"OPEN ", &[CONCEAL], HIDDEN].concat();
        codes[..text.len()].copy_from_slice(&text);
        file.set_row(3, &codes);
        file.rows_present |= 1 << 3;
        let mut page = Vec::new();
        neoteletext_formats::encode::encode_page(&file, &mut |packet: Packet| page.extend_from_slice(&packet.bytes));
        assert!(without_parity(&page).windows(HIDDEN.len()).any(|window| window == HIDDEN), "the page as sent holds the concealed text");

        let folder = std::env::temp_dir().join(format!("neoteletext-thumbnails-concealed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let mut core = super::super::test_support::test_core();
        core.thumbnails = Thumbnails::new(Some(&folder));
        core.session.open("https://a.example/tt/").unwrap();
        let place = Place { channel: String::from("https://a.example/tt/"), page: neoteletext::snapshots::INITIAL_PAGE, sub: WHOLE_PAGE };

        let mut answer = Vec::new();
        decoder::snapshot::answer(&mut answer, place.page_asked(), place.sub, &page).unwrap();
        let mut answer = answer.as_slice();
        let mut source = Vec::new();
        let drawn = loop {
            match read_from_decoder(&mut answer).unwrap() {
                Some(FromDecoder::SnapshotSource(piece)) => source.extend_from_slice(&piece),
                Some(FromDecoder::SnapshotDrawn { page, sub, image }) => break SnapshotDrawn { page, sub, image, source },
                other => panic!("the snapshot decoder answered {other:?}"),
            }
        };
        let job = core.thumbnails.job();
        core.thumbnails.drawing.push((job, place.clone(), true));
        core.snapshot_drawn(job, Ok(drawn)).unwrap();

        let kept = std::fs::read(folder.join(neoteletext::snapshots::FOLDER).join(place.file_name())).expect("the snapshot was kept");
        let kept = without_parity(&kept);
        assert!(!kept.windows(HIDDEN.len()).any(|window| window == HIDDEN), "the kept file holds the concealed text");
        assert!(kept.windows(4).any(|window| window == b"OPEN"), "the text shown is kept");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// REQ-6883: a place being fetched or drawn is not started again.
    #[test]
    fn a_place_in_hand_is_not_started_again() {
        let mut thumbnails = Thumbnails::default();
        let wanted = [want(1, "a.example", 0x100), want(2, "a.example", 0x100)];
        thumbnails.fetching.push((1, wanted[0].place.clone(), String::from("a.example")));
        assert_eq!(plan(&thumbnails, &wanted, |_| None, 0, Instant::now()), Plan::default());
    }
}
