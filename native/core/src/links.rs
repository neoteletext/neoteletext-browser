//! The trusted core's hyperlinks (`hyperlinks.md` sections 7, 8, 10 to 12;
//! `hyperlinks-security.md`). Here are the parser of link targets, which
//! is the only page data the trusted core reads; the classification of
//! every target before anything is fetched; the links of each image on
//! screen, checked; and the rules that decide whether a click, a typed
//! number or a confirmation in the trusted strip follows a link.
//!
//! Nothing here acts: it says what to show and what to follow, and the
//! trusted core's main loop does it. Times are milliseconds on any clock
//! that only moves forward.

use std::collections::{BTreeMap, VecDeque};

use interface_core::canvas::{COLUMNS, PageKey};
use interface_core::controls::FastextKey;
use protocol::links::{Alias, Candidate, Fastext, Found, LinkRun, MAX_REMAPPED, MAX_RUNS, NumberLink, PageLinks, Pointing, Prompt, Service, Spot};

use crate::address::Address;

mod destination;
mod prompt;
mod target;

use destination::Descriptor;
pub use destination::{Channel, Destination};
use prompt::Asking;
pub use prompt::{HANDOFF_INTERVAL_MILLISECONDS, PROMPT_DELAY_MILLISECONDS};
pub use target::{Target, parse_target, web_address};
use target::{channel_address, name};

/// A click follows a link only once its target has stayed the same for
/// half a second, the time browsers give a permission prompt before they
/// accept a click (`hyperlinks.md` section 10.2, requirement 17.7).
pub const DWELL_MILLISECONDS: u64 = 500;
/// The images of each page whose links are kept: a click names the image it
/// was made on, and one only a little old still counts (`hyperlinks.md`
/// section 8.3).
pub const KEPT_IMAGES: usize = 4;
/// Text rows: row 0 is the header, row 24 the Fastext row, and neither
/// holds links (`hyperlinks.md` sections 6.1 and 8.2).
const FIRST_LINK_ROW: u8 = 1;
const LAST_LINK_ROW: u8 = 23;
/// One link of an image, decided.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Link {
    run: LinkRun,
    destination: Destination,
    /// When it first led where it leads.
    since: u64,
}

/// The links of one image of a page.
#[derive(Clone, Debug)]
struct Image {
    version: u32,
    links: Vec<Link>,
    /// Page numbers the page links elsewhere (`LK` lines), decided.
    numbers: Vec<(u16, Destination)>,
    /// Fastext's keys, decided, each with the sub-page it names; none when
    /// row 24 was not shown with this image (the engine's REQ-SEC-024).
    fastext: [Option<(Destination, Option<u16>)>; 5],
}

#[derive(Default)]
struct PageState {
    /// What the decoder last found in the page.
    found: (Vec<Candidate>, Vec<NumberLink>),
    /// The page's Fastext as the decoder last found it.
    fastext: Fastext,
    /// The latest images first.
    images: VecDeque<Image>,
}

/// A page's links and its `LK` lines, decided.
type Decided = (Vec<(LinkRun, Destination)>, Vec<(u16, Destination)>);

/// What a click, a typed number or a confirmation leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Follow {
    /// Open this page of the active channel.
    Page(u16),
    /// Ask in the trusted strip first.
    Ask(Prompt),
    /// The user confirmed leaving for this channel.
    Leave(Address),
    /// The user confirmed opening this web page in the system's web browser.
    HandOff(String),
}

/// Why nothing was followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The click was not on the open page of the active tab.
    NotOpen,
    /// The image the click names is not one the trusted core sent lately.
    Stale,
    /// No link is there.
    NotALink,
    /// The strip was not showing that link's destination.
    Unshown,
    /// The link changed less than half a second ago.
    Changed,
    /// The question the answer names is not the one asked.
    NoPrompt,
    /// The question was answered within half a second of being asked.
    TooSoon,
    /// The window does not have the keyboard focus.
    Unfocused,
    /// A web page was handed over less than two seconds ago.
    TooOften,
}

/// The links of every page on screen, and what the trusted strip is
/// showing and asking about them.
#[derive(Default)]
pub struct Links {
    pages: BTreeMap<PageKey, PageState>,
    descriptors: BTreeMap<u32, Descriptor>,
    /// The cell the strip is showing a destination for, and that destination.
    pointed: Option<(Spot, Destination)>,
    asking: Option<Asking>,
    next_prompt: u32,
    last_handoff: Option<u64>,
    /// The window has the keyboard focus; it starts with it.
    unfocused: bool,
    /// The page open in the active tab.
    opened: Option<PageKey>,
}

impl Links {
    /// Forgets every page's links, as when the interface reports what is on
    /// screen afresh.
    pub fn clear_pages(&mut self) {
        self.pages.clear();
        self.pointed = None;
    }

    /// Keeps only the links of the pages on screen.
    pub fn retain(&mut self, visible: &[PageKey]) {
        self.pages.retain(|key, _| visible.contains(key));
    }

    /// Forgets a channel that has closed.
    pub fn channel_gone(&mut self, channel: u32) {
        self.pages.retain(|key, _| key.channel != channel);
        self.descriptors.remove(&channel);
    }

    /// The page open in the active tab, or none.
    pub fn opened(&mut self, key: Option<PageKey>) {
        self.opened = key;
    }

    /// The page open in the active tab.
    #[must_use]
    pub fn opened_page(&self) -> Option<PageKey> {
        self.opened
    }

    /// Whether the window has the keyboard focus.
    pub fn focused(&mut self, focused: bool) {
        self.unfocused = !focused;
    }

    /// The links the decoder found in a page, for its next image.
    pub fn found(&mut self, key: PageKey, candidates: Vec<Candidate>, numbers: Vec<NumberLink>, fastext: Fastext) {
        let state = self.pages.entry(key).or_default();
        state.found = (candidates, numbers);
        state.fastext = fastext;
    }

    /// Decides a page's Fastext (`page-controls.md` section 2): a page of
    /// this channel, or a page of a service the descriptor lists, by its
    /// place there; nothing while row 24 is not shown.
    fn decide_fastext(&self, key: PageKey) -> [Option<(Destination, Option<u16>)>; 5] {
        let Some(state) = self.pages.get(&key) else { return Default::default() };
        if !state.fastext.row24 {
            return Default::default();
        }
        let descriptor = self.descriptors.get(&key.channel);
        state.fastext.keys.map(|target| {
            let target = target?;
            let destination = match target.service {
                None => Destination::Page(target.page),
                Some(place) => {
                    let address = descriptor?.by_place.get(usize::from(place))?.as_ref()?;
                    Destination::Channel(address.with_page(target.page))
                }
            };
            Some((destination, target.subcode))
        })
    }

    /// Which of Fastext's keys lead somewhere on the latest image of the
    /// open page, for the controls the interface draws.
    #[must_use]
    pub fn fastext_keys(&self, key: PageKey) -> [bool; 5] {
        let image = self.pages.get(&key).and_then(|state| state.images.front());
        core::array::from_fn(|index| image.is_some_and(|image| image.fastext.get(index).is_some_and(Option::is_some)))
    }

    /// One of Fastext's keys on the open page, as it was last drawn: a page
    /// of this channel opens at once, with the sub-page the link names;
    /// another channel is asked about in the strip first (requirement 8.4).
    ///
    /// # Errors
    /// Why nothing is followed: no page open, or the key leads nowhere.
    pub fn fastext(&mut self, key: FastextKey, active: Option<u32>, now: u64) -> Result<(Follow, Option<u16>), Refusal> {
        let opened = self.opened.filter(|opened| Some(opened.channel) == active).ok_or(Refusal::NotOpen)?;
        let image = self.pages.get(&opened).and_then(|state| state.images.front()).ok_or(Refusal::NotALink)?;
        let (destination, subcode) = image.fastext.get(key.index()).cloned().flatten().ok_or(Refusal::NotALink)?;
        let subcode = matches!(destination, Destination::Page(_)).then_some(subcode).flatten();
        Ok((self.follow(destination, now)?, subcode))
    }

    /// The aliases and named services of a channel's descriptor. Those that
    /// are not targets are dropped.
    pub fn descriptor(&mut self, channel: u32, aliases: &[Alias], services: &[Service]) {
        let aliases = aliases.iter().filter_map(|alias| Some((alias.page, parse_target(&alias.target).filter(|target| !matches!(target, Target::Web(_)))?)));
        let by_place: Vec<Option<Address>> = services.iter().map(|service| channel_address(&service.address)).collect();
        let services = services.iter().filter_map(|service| Some((name(&service.name)?, channel_address(&service.address)?)));
        self.descriptors.insert(channel, Descriptor { aliases: aliases.take(protocol::links::MAX_ALIASES).collect(), services: services.collect(), by_place });
    }

    /// The page numbers a channel's descriptor links elsewhere.
    #[must_use]
    pub fn aliases(&self, channel: u32) -> Vec<u16> {
        self.descriptors.get(&channel).map(|descriptor| descriptor.aliases.iter().map(|(page, _)| *page).collect()).unwrap_or_default()
    }

    /// Decides the links of a page from what the decoder found, the page's
    /// `LK` lines and the channel's descriptor (`hyperlinks.md` sections 6
    /// and 8.2). Runs outside rows 1 to 23 are dropped; a run that overlaps
    /// an earlier one is dropped, a declared link winning over a number found
    /// in the text; targets that do not parse, or lead nowhere, are dropped.
    fn decide(&self, key: PageKey, channel: &Channel<'_>) -> Decided {
        let empty = Descriptor::default();
        let descriptor = self.descriptors.get(&key.channel).unwrap_or(&empty);
        let Some(state) = self.pages.get(&key) else { return (Vec::new(), Vec::new()) };
        let (candidates, numbers) = &state.found;
        // A page's `LK` lines; a web page is never the target of a typed
        // number (the engine's decision E.88).
        let numbers: Vec<(u16, Destination)> = numbers
            .iter()
            .filter_map(|link| {
                let target = parse_target(&link.target).filter(|target| !matches!(target, Target::Web(_)))?;
                Some((link.number, descriptor.resolve(&target, channel)?))
            })
            .collect();
        let numbered = |number: u16| {
            numbers
                .iter()
                .find(|(linked, _)| *linked == number)
                .map(|(_, destination)| destination.clone())
                .or_else(|| descriptor.aliases.iter().find(|(page, _)| *page == number).and_then(|(_, target)| descriptor.resolve(target, channel)))
        };
        let mut runs: Vec<(LinkRun, Destination)> = Vec::new();
        // Declared links first, so that they win where they overlap a number.
        let declared = candidates.iter().filter(|candidate| matches!(candidate.target, Found::Declared(_)));
        let found = candidates.iter().filter(|candidate| matches!(candidate.target, Found::Number(_)));
        for candidate in declared.chain(found) {
            let ends = u16::from(candidate.column).saturating_add(u16::from(candidate.length)) <= COLUMNS;
            if !(FIRST_LINK_ROW..=LAST_LINK_ROW).contains(&candidate.row) || candidate.length == 0 || !ends {
                continue;
            }
            let destination = match &candidate.target {
                Found::Number(number) => numbered(*number).or_else(|| (channel.exists)(*number).then_some(Destination::Page(*number))),
                Found::Declared(text) => parse_target(text).and_then(|target| descriptor.resolve(&target, channel)),
            };
            let Some(destination) = destination else { continue };
            let run = LinkRun {
                row: candidate.row,
                column: candidate.column,
                length: candidate.length,
                double: candidate.double,
                light: candidate.light,
                kind: destination.kind(),
            };
            if runs.iter().any(|(other, _)| overlap(*other, run)) {
                continue;
            }
            runs.push((run, destination));
        }
        runs.truncate(MAX_RUNS);
        runs.sort_by_key(|(run, _)| (run.row, run.column));
        (runs, numbers)
    }

    /// A new image of a page was sent: its links are decided afresh, for the
    /// interface to draw over that image.
    pub fn image(&mut self, key: PageKey, version: u32, channel: &Channel<'_>, now: u64) -> PageLinks {
        let (runs, numbers) = self.decide(key, channel);
        let fastext = self.decide_fastext(key);
        let state = self.pages.entry(key).or_default();
        let links = carry_since(state.images.front(), runs, now);
        state.images.push_front(Image { version, links, numbers, fastext });
        state.images.truncate(KEPT_IMAGES);
        self.page_links(key, key.channel)
    }

    /// The decoder's links changed while the page's image did not, or the
    /// channel's pages or descriptor changed: the latest image's links are
    /// decided again. Returns them when they changed.
    pub fn redecide(&mut self, key: PageKey, channel: &Channel<'_>, now: u64) -> Option<PageLinks> {
        let (runs, numbers) = self.decide(key, channel);
        let fastext = self.decide_fastext(key);
        let image = self.pages.get_mut(&key)?.images.front_mut()?;
        let links = carry_since(Some(image), runs, now);
        if links == image.links && numbers == image.numbers && fastext == image.fastext {
            return None;
        }
        image.links = links;
        image.numbers = numbers;
        image.fastext = fastext;
        Some(self.page_links(key, key.channel))
    }

    /// The pages on screen of `channel` whose links are kept.
    #[must_use]
    pub fn pages_of(&self, channel: u32) -> Vec<PageKey> {
        self.pages.keys().copied().filter(|key| key.channel == channel).collect()
    }

    /// What the interface is told of the latest image's links.
    fn page_links(&self, key: PageKey, channel: u32) -> PageLinks {
        let Some(image) = self.pages.get(&key).and_then(|state| state.images.front()) else { return PageLinks::default() };
        let mut remapped: Vec<u16> = image.numbers.iter().map(|(number, _)| *number).chain(self.aliases(channel)).collect();
        remapped.sort_unstable();
        remapped.dedup();
        remapped.truncate(MAX_REMAPPED);
        PageLinks { version: image.version, runs: image.links.iter().map(|link| link.run).collect(), remapped }
    }

    /// The link at a cell of an image the trusted core sent.
    fn link_at(&self, spot: &Spot) -> Result<&Link, Refusal> {
        let image = self.pages.get(&spot.key).and_then(|state| state.images.iter().find(|image| image.version == spot.version)).ok_or(Refusal::Stale)?;
        image.links.iter().find(|link| link.run.contains(spot.column, spot.row)).ok_or(Refusal::NotALink)
    }

    /// The pointer or the keyboard's focus is on a cell, or on none: the
    /// strip shows where the link there leads (requirement 17.6). Returns
    /// what the strip shows.
    pub fn point(&mut self, spot: Option<Spot>) -> Option<Pointing> {
        self.pointed = spot.and_then(|spot| Some((spot, self.link_at(&spot).ok()?.destination.clone())));
        self.pointing()
    }

    /// The links changed: the strip's destination follows them. Returns
    /// what the strip shows when that changed.
    pub fn repoint(&mut self) -> Option<Option<Pointing>> {
        let before = self.pointed.as_ref().map(|(_, destination)| destination.clone());
        let spot = self.pointed.as_ref().map(|(spot, _)| *spot);
        self.point(spot);
        let after = self.pointed.as_ref().map(|(_, destination)| destination.clone());
        (before != after).then(|| self.pointing())
    }

    fn pointing(&self) -> Option<Pointing> {
        self.pointed.as_ref().map(|(_, destination)| Pointing { kind: destination.kind(), text: destination.describe() })
    }

    /// Asks the strip's question for `destination`, as a link to it would:
    /// what a start-page bookmark opened in the page area leads to
    /// (requirement 4.5, finding F-67).
    ///
    /// # Errors
    /// Why it is not asked.
    pub fn ask(&mut self, destination: Destination, now: u64) -> Result<Follow, Refusal> {
        self.follow(destination, now)
    }

    /// A click on a cell. It follows a link only when the click is on the
    /// open page of the active channel, in an image the trusted core sent
    /// lately, on a link whose destination the strip is showing, and which
    /// has led there for at least half a second (requirement 17.7).
    ///
    /// # Errors
    /// Why nothing is followed.
    pub fn click(&mut self, spot: Spot, active: Option<u32>, now: u64) -> Result<Follow, Refusal> {
        if self.opened != Some(spot.key) || active != Some(spot.key.channel) {
            return Err(Refusal::NotOpen);
        }
        let link = self.link_at(&spot)?;
        if self.pointed.as_ref().is_none_or(|(_, shown)| *shown != link.destination) {
            return Err(Refusal::Unshown);
        }
        if now.saturating_sub(link.since) < DWELL_MILLISECONDS {
            return Err(Refusal::Changed);
        }
        let destination = link.destination.clone();
        self.follow(destination, now)
    }

    /// A page number typed while `opened` was open, which the page or the
    /// channel links elsewhere. A typed number never leads to a web page.
    ///
    /// # Errors
    /// Why nothing is followed.
    pub fn typed(&mut self, number: u16, opened: Option<(PageKey, u32)>, active: Option<u32>, now: u64) -> Result<Follow, Refusal> {
        let channel = active.ok_or(Refusal::NotOpen)?;
        let page_link = match opened {
            Some((key, version)) => {
                if self.opened != Some(key) || key.channel != channel {
                    return Err(Refusal::NotOpen);
                }
                let image = self.pages.get(&key).and_then(|state| state.images.iter().find(|image| image.version == version)).ok_or(Refusal::Stale)?;
                image.numbers.iter().find(|(linked, _)| *linked == number).map(|(_, destination)| destination.clone())
            }
            None => None,
        };
        let destination = if let Some(destination) = page_link {
            destination
        } else {
            {
                let descriptor = self.descriptors.get(&channel).ok_or(Refusal::NotALink)?;
                let (_, target) = descriptor.aliases.iter().find(|(page, _)| *page == number).ok_or(Refusal::NotALink)?;
                // An alias's own channel is checked when its links are
                // decided; here only another channel or this one's page.
                match target {
                    Target::Page(page) => Destination::Page(*page),
                    Target::Service { name, page } => {
                        let (_, address) = descriptor.services.iter().find(|(known, _)| known == name).ok_or(Refusal::NotALink)?;
                        Destination::Channel(address.with_page(*page))
                    }
                    Target::Channel(address) => Destination::Channel(address.clone()),
                    Target::Web(_) => return Err(Refusal::NotALink),
                }
            }
        };
        if matches!(destination, Destination::Web(_)) {
            return Err(Refusal::NotALink);
        }
        self.follow(destination, now)
    }
}

/// Whether two runs share a cell.
fn overlap(one: LinkRun, other: LinkRun) -> bool {
    let rows = |run: LinkRun| (run.row, if run.double { run.row.saturating_add(1) } else { run.row });
    let ((top, bottom), (other_top, other_bottom)) = (rows(one), rows(other));
    let columns = |run: LinkRun| (run.column, run.column.saturating_add(run.length));
    let ((left, right), (other_left, other_right)) = (columns(one), columns(other));
    top <= other_bottom && other_top <= bottom && left < other_right && other_left < right
}

/// The links of a new decision, each keeping when it first led where it
/// leads if the image before had the same link.
fn carry_since(before: Option<&Image>, runs: Vec<(LinkRun, Destination)>, now: u64) -> Vec<Link> {
    runs.into_iter()
        .map(|(run, destination)| {
            let since =
                before.and_then(|image| image.links.iter().find(|link| link.run == run && link.destination == destination)).map_or(now, |link| link.since);
            Link { run, destination, since }
        })
        .collect()
}

#[cfg(test)]
mod tests;
