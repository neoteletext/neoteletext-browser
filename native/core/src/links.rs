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
use interface_core::channel::page_index;
use protocol::links::{
    Alias, Candidate, Found, LinkKind, LinkRun, MAX_DESTINATION, MAX_REMAPPED, MAX_RUNS, MAX_SERVICE_NAME, MAX_TARGET, NumberLink, PageLinks, Pointing, Prompt,
    PromptKind, Service, Spot,
};
use protocol::{MAX_ADDRESS, clean};

use crate::address::{self, Address};

/// A click follows a link only once its target has stayed the same for
/// half a second, the time browsers give a permission prompt before they
/// accept a click (`hyperlinks.md` section 10.2, requirement 17.7).
pub const DWELL_MILLISECONDS: u64 = 500;
/// The strip's questions accept no answer in their first half second
/// (`hyperlinks.md` section 12).
pub const PROMPT_DELAY_MILLISECONDS: u64 = 500;
/// At most one web page is handed to the system's web browser every two
/// seconds (requirement 17.9).
pub const HANDOFF_INTERVAL_MILLISECONDS: u64 = 2_000;
/// The images of each page whose links are kept: a click names the image it
/// was made on, and one only a little old still counts (`hyperlinks.md`
/// section 8.3).
pub const KEPT_IMAGES: usize = 4;
/// Text rows: row 0 is the header, row 24 the Fastext row, and neither
/// holds links (`hyperlinks.md` sections 6.1 and 8.2).
const FIRST_LINK_ROW: u8 = 1;
const LAST_LINK_ROW: u8 = 23;
/// A web address handed to the system's web browser: the address bar's
/// limit.
const MAX_WEB: usize = MAX_ADDRESS;
/// The page a channel opens at when a link names its folder and no page:
/// the teletext initial page (the engine's REQ-WEB-018 default).
const INITIAL_PAGE: u16 = 0x100;
/// The digits of a page number and of a sub-page number in a target
/// (TTI Link Extension section E.1).
const PAGE_DIGITS: usize = 3;
const SUBPAGE_DIGITS: usize = 4;
const HEXADECIMAL: u32 = 16;
/// The two schemes a link may name, and the mark of a web page.
const HTTPS: &str = "https";
const HTTP: &str = "http";
const SCHEME_SEPARATOR: &str = "://";
const WEB: &str = "web:";
/// The files of a channel that are pages (the engine's REQ-WEB-019).
const PAGE_FILES: [&str; 2] = [".tti", ".t42"];

/// A link target as a page or a descriptor writes it, parsed: the forms of
/// the TTI Link Extension's section E.1 and `web:` (the engine's decision
/// E.88).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A page of this channel.
    Page(u16),
    /// A page of a service the channel's descriptor names.
    Service {
        /// The service's short name.
        name: String,
        /// The page.
        page: u16,
    },
    /// A teletext channel or page file at an address.
    Channel(Address),
    /// A web page: its address, rebuilt from its parsed parts.
    Web(String),
}

/// Parses a link target. Everything that is not exactly one of the forms
/// is refused: a target is page data, the only page data the trusted core
/// reads (requirement 17.12).
#[must_use]
pub fn parse_target(text: &str) -> Option<Target> {
    // Printable ASCII with no spaces or commas, at most 256 bytes (TTI
    // Link Extension section E.1).
    if text.is_empty() || text.len() > MAX_TARGET || !text.bytes().all(|byte| byte.is_ascii_graphic() && byte != b',') {
        return None;
    }
    if let Some(web) = text.strip_prefix(WEB) {
        return web_address(web).map(Target::Web);
    }
    if text.contains(SCHEME_SEPARATOR) {
        return channel_address(text).map(Target::Channel);
    }
    let mut parts = text.split(':');
    let (first, second, third) = (parts.next()?, parts.next(), parts.next());
    if parts.next().is_some() {
        return None;
    }
    match (second, third) {
        (None, _) => page(first).map(Target::Page),
        // `mpp:ssss`, or `name:mpp`: a page number is three hexadecimal
        // digits and a name of three such letters is read as a page.
        (Some(second), None) => match (page(first), subpage(second)) {
            (Some(page), Some(())) => Some(Target::Page(page)),
            _ => Some(Target::Service { name: name(first)?, page: page(second)? }),
        },
        (Some(second), Some(third)) => {
            subpage(third)?;
            Some(Target::Service { name: name(first)?, page: page(second)? })
        }
    }
}

/// A display page: three hexadecimal digits naming a page of 100 to 899
/// with no digit above 9.
fn page(text: &str) -> Option<u16> {
    if text.len() != PAGE_DIGITS || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let page = u16::from_str_radix(text, HEXADECIMAL).ok()?;
    page_index(page).map(|_| page)
}

/// A sub-page: four hexadecimal digits. Links go to a page; its sub-page
/// is shown as the page shows it.
fn subpage(text: &str) -> Option<()> {
    (text.len() == SUBPAGE_DIGITS && text.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(())
}

/// A service's short name: letters, digits and hyphens (TTI Link Extension
/// section E.1).
fn name(text: &str) -> Option<String> {
    let valid = !text.is_empty() && text.len() <= MAX_SERVICE_NAME && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    valid.then(|| text.to_owned())
}

/// The scheme of `text`, when it is `https` or `http` written in any case.
fn web_scheme(text: &str) -> Option<(&'static str, &str)> {
    let (scheme, rest) = text.split_once(SCHEME_SEPARATOR)?;
    [HTTPS, HTTP].into_iter().find(|known| scheme.eq_ignore_ascii_case(known)).map(|known| (known, rest))
}

/// A teletext channel's address: `https` or `http` only, by the address
/// rules the address field uses. A link never leads to a file.
fn channel_address(text: &str) -> Option<Address> {
    web_scheme(text)?;
    let address = address::parse(text).ok()?;
    address.file.is_none().then_some(address)
}

/// Characters a web address may hold after its host, as RFC 3986 section 2
/// allows them: unreserved, reserved and the percent sign of an escape.
fn uri_character(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&byte)
}

/// A web address, rebuilt from its parsed parts so that what is handed on
/// is exactly what was checked (`hyperlinks.md` section 12): `https` or
/// `http`, a host of letters, digits, hyphens and dots, a numeric port,
/// then only the characters RFC 3986 allows, every `%` starting an escape.
/// Anything else is refused, never repaired.
#[must_use]
pub fn web_address(text: &str) -> Option<String> {
    if text.len() > MAX_WEB {
        return None;
    }
    let (scheme, rest) = web_scheme(text)?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, Some(port.parse::<u16>().ok()?)),
        None => (authority, None),
    };
    let label = |label: &str| !label.is_empty() && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if host.is_empty() || !host.split('.').all(label) {
        return None;
    }
    let bytes = tail.as_bytes();
    let escapes_whole = bytes
        .iter()
        .enumerate()
        .filter(|&(_, &byte)| byte == b'%')
        .all(|(at, _)| bytes.get(at.saturating_add(1)..at.saturating_add(3)).is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit)));
    if !bytes.iter().copied().all(uri_character) || !escapes_whole {
        return None;
    }
    let host = host.to_ascii_lowercase();
    let port = port.map(|port| format!(":{port}")).unwrap_or_default();
    let tail = if tail.is_empty() || tail.starts_with(['?', '#']) { format!("/{tail}") } else { tail.to_owned() };
    let rebuilt = format!("{scheme}{SCHEME_SEPARATOR}{host}{port}{tail}");
    (rebuilt.len() <= MAX_WEB).then_some(rebuilt)
}

/// Where a link leads, decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// A page of this channel: followed at once.
    Page(u16),
    /// Another channel, at the page its address names, if any: after the
    /// site-change screen.
    Channel(Address),
    /// A web page: after the web-browser prompt, in the system's web browser.
    Web(String),
}

impl Destination {
    /// The kind the interface draws.
    #[must_use]
    pub fn kind(&self) -> LinkKind {
        match self {
            Self::Page(_) => LinkKind::Page,
            Self::Channel(_) => LinkKind::Channel,
            Self::Web(_) => LinkKind::Web,
        }
    }

    /// Where it is, as the strip's questions name it, cleaned and cut to
    /// the strip's limit (requirement 11.5).
    #[must_use]
    pub fn name(&self) -> String {
        let text = match self {
            Self::Page(page) => format!("Page {page:03X}"),
            Self::Channel(address) => match address.page {
                Some(page) => format!("Page {page:03X} of {}", address.host),
                None => address.channel_text().to_owned(),
            },
            Self::Web(address) => address.clone(),
        };
        clean(&text, MAX_DESTINATION)
    }

    /// What the trusted strip says of it while the pointer or the focus is
    /// on it: where it is, and whether it leaves.
    #[must_use]
    pub fn describe(&self) -> String {
        let text = match self {
            Self::Page(_) => self.name(),
            Self::Channel(_) => format!("{} (leaves this channel)", self.name()),
            Self::Web(_) => format!("{} (opens in your web browser)", self.name()),
        };
        clean(&text, MAX_DESTINATION)
    }
}

/// The channel a link is on, as the trusted core knows it.
pub struct Channel<'a> {
    /// The channel's number.
    pub id: u32,
    /// Its address: a web channel's folder, or a file.
    pub address: &'a Address,
    /// Whether it has a page: listed in its contents, or arrived.
    pub exists: &'a dyn Fn(u16) -> bool,
}

impl Channel<'_> {
    /// The page of this channel an address names: its own folder with a
    /// page, or one of its page files (`hyperlinks.md` section 7.1).
    fn own_page(&self, address: &Address) -> Option<u16> {
        if self.address.file.is_some() {
            return None;
        }
        let folder = self.address.channel_text();
        let other = address.channel_text();
        if other == folder {
            return Some(address.page.unwrap_or(INITIAL_PAGE));
        }
        let file = other.strip_prefix(folder)?;
        let number = PAGE_FILES.iter().find_map(|extension| file.strip_suffix(extension))?;
        // Page files are named in decimal digits: `302.tti` is page 302.
        page(number).filter(|_| number.bytes().all(|byte| byte.is_ascii_digit()))
    }
}

/// The aliases and named services of a channel's descriptor, parsed.
#[derive(Clone, Debug, Default)]
struct Descriptor {
    aliases: Vec<(u16, Target)>,
    services: Vec<(String, Address)>,
}

impl Descriptor {
    /// Where `target` leads from `channel`. A target that names an unknown
    /// service leads nowhere (TTI Link Extension rule E.2.2); a named
    /// service is another channel, at its own address
    /// (`hyperlinks-security.md` finding 1).
    fn resolve(&self, target: &Target, channel: &Channel<'_>) -> Option<Destination> {
        match target {
            Target::Page(page) => Some(Destination::Page(*page)),
            Target::Service { name, page } => {
                let (_, address) = self.services.iter().find(|(known, _)| known == name)?;
                Some(Destination::Channel(address.with_page(*page)))
            }
            Target::Channel(address) => Some(channel.own_page(address).map_or_else(|| Destination::Channel(address.clone()), Destination::Page)),
            Target::Web(address) => Some(Destination::Web(address.clone())),
        }
    }
}

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
}

#[derive(Default)]
struct PageState {
    /// What the decoder last found in the page.
    found: (Vec<Candidate>, Vec<NumberLink>),
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

/// A question the trusted strip is asking.
struct Asking {
    prompt: Prompt,
    asked: u64,
    destination: Destination,
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
    pub fn found(&mut self, key: PageKey, candidates: Vec<Candidate>, numbers: Vec<NumberLink>) {
        self.pages.entry(key).or_default().found = (candidates, numbers);
    }

    /// The aliases and named services of a channel's descriptor. Those that
    /// are not targets are dropped.
    pub fn descriptor(&mut self, channel: u32, aliases: &[Alias], services: &[Service]) {
        let aliases = aliases.iter().filter_map(|alias| Some((alias.page, parse_target(&alias.target).filter(|target| !matches!(target, Target::Web(_)))?)));
        let services = services.iter().filter_map(|service| Some((name(&service.name)?, channel_address(&service.address)?)));
        self.descriptors.insert(channel, Descriptor { aliases: aliases.take(protocol::links::MAX_ALIASES).collect(), services: services.collect() });
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
        let state = self.pages.entry(key).or_default();
        let links = carry_since(state.images.front(), runs, now);
        state.images.push_front(Image { version, links, numbers });
        state.images.truncate(KEPT_IMAGES);
        self.page_links(key, key.channel)
    }

    /// The decoder's links changed while the page's image did not, or the
    /// channel's pages or descriptor changed: the latest image's links are
    /// decided again. Returns them when they changed.
    pub fn redecide(&mut self, key: PageKey, channel: &Channel<'_>, now: u64) -> Option<PageLinks> {
        let (runs, numbers) = self.decide(key, channel);
        let image = self.pages.get_mut(&key)?.images.front_mut()?;
        let links = carry_since(Some(image), runs, now);
        if links == image.links && numbers == image.numbers {
            return None;
        }
        image.links = links;
        image.numbers = numbers;
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

    /// A page of this channel opens at once; anything else is asked about
    /// in the trusted strip first, and a web page only while the window has
    /// the keyboard focus.
    fn follow(&mut self, destination: Destination, now: u64) -> Result<Follow, Refusal> {
        let (kind, secure) = match &destination {
            Destination::Page(page) => return Ok(Follow::Page(*page)),
            Destination::Channel(address) => (PromptKind::Leave, address.encrypted),
            Destination::Web(address) => {
                if self.unfocused {
                    return Err(Refusal::Unfocused);
                }
                (PromptKind::Web, address.starts_with(HTTPS))
            }
        };
        self.next_prompt = self.next_prompt.wrapping_add(1);
        let prompt = Prompt { id: self.next_prompt, kind, destination: destination.name(), secure };
        // A new question replaces the one before; questions never stack.
        self.asking = Some(Asking { prompt: prompt.clone(), asked: now, destination });
        Ok(Follow::Ask(prompt))
    }

    /// The strip's question now, if any.
    #[must_use]
    pub fn prompt(&self) -> Option<Prompt> {
        self.asking.as_ref().map(|asking| asking.prompt.clone())
    }

    /// Yes to the strip's question `id`. Refused within half a second of the
    /// question, for another question, and, for a web page, without the
    /// keyboard focus or within two seconds of the last web page handed over.
    ///
    /// # Errors
    /// Why nothing is followed.
    pub fn confirm(&mut self, id: u32, now: u64) -> Result<Follow, Refusal> {
        let asking = self.asking.as_ref().filter(|asking| asking.prompt.id == id).ok_or(Refusal::NoPrompt)?;
        if now.saturating_sub(asking.asked) < PROMPT_DELAY_MILLISECONDS {
            return Err(Refusal::TooSoon);
        }
        if let Destination::Web(_) = asking.destination {
            if self.unfocused {
                return Err(Refusal::Unfocused);
            }
            if self.last_handoff.is_some_and(|last| now.saturating_sub(last) < HANDOFF_INTERVAL_MILLISECONDS) {
                return Err(Refusal::TooOften);
            }
        }
        let asking = self.asking.take().ok_or(Refusal::NoPrompt)?;
        Ok(match asking.destination {
            Destination::Page(page) => Follow::Page(page),
            Destination::Channel(address) => Follow::Leave(address),
            Destination::Web(address) => {
                self.last_handoff = Some(now);
                Follow::HandOff(address)
            }
        })
    }

    /// No to the strip's question `id`; returns whether it was the question
    /// asked. Declining has no other effect (requirement 8.6).
    pub fn cancel(&mut self, id: u32) -> bool {
        let asked = self.asking.as_ref().is_some_and(|asking| asking.prompt.id == id);
        if asked {
            self.asking = None;
        }
        asked
    }

    /// Forgets the strip's question, as when the tab or its channel changes.
    pub fn forget_prompt(&mut self) -> bool {
        self.asking.take().is_some()
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
