//! The link messages (`hyperlinks.md` section 8.1). A decoder sends the
//! links it finds in each page it renders, and a fetcher the aliases and
//! named services of its channel's descriptor; both are untrusted, so their
//! targets arrive as raw text for the trusted core's own parser. The
//! trusted core sends the interface only where each link of a page lies and
//! what kind of place it leads to, never the target itself.

use interface_core::canvas::{COLUMNS, PageKey, ROWS};
use interface_core::channel::page_index;
pub use interface_core::links::{LinkKind, LinkRun, PageLinks, Pointing, Prompt, PromptKind};

use super::{Fields, MAX_ADDRESS, ProtocolError, push_key, push_text};

/// The most links a page has: beyond this the trusted core gives it none
/// (`hyperlinks.md` section 8.2).
pub const MAX_RUNS: usize = 32;
/// The most page numbers found in one page's text (`hyperlinks.md`
/// section 6.1).
pub const MAX_FOUND: usize = 24;
/// The most `LT` and `LK` lines a page keeps (the engine's `TEXT_LINKS`
/// and `NUMBER_LINKS`, TTI Link Extension section E.1).
pub const MAX_TEXT_LINKS: usize = 8;
/// The most `LK` lines a page keeps.
pub const MAX_NUMBER_LINKS: usize = 8;
/// A target as a page file writes it: at most 256 bytes (the engine's
/// `LINK_ADDRESS_BYTES`, TTI Link Extension section E.1), a `web:` target
/// included.
pub const MAX_TARGET: usize = 256;
/// The most aliases a descriptor has (the engine's REQ-WEB-002).
pub const MAX_ALIASES: usize = 16;
/// The most named services a descriptor lists (the engine's
/// `MAX_LINKED_SERVICES`).
pub const MAX_SERVICES: usize = 16;
/// A named service's short name (the engine's `MAX_SHORT_NAME`).
pub const MAX_SERVICE_NAME: usize = 32;
/// Page numbers a page remaps: its `LK` lines and the channel's aliases.
pub const MAX_REMAPPED: usize = MAX_NUMBER_LINKS + MAX_ALIASES;
/// A destination as the trusted strip shows it: an address and a few
/// words about it.
pub const MAX_DESTINATION: usize = MAX_ADDRESS + 64;

// Trusted core to interface.
const KIND_PAGE_LINKS: u8 = 11;
const KIND_POINTING: u8 = 12;
const KIND_PROMPT: u8 = 13;
const KIND_SHOW: u8 = 14;
const KIND_ALIASES: u8 = 15;
// A run's flags on the wire.
const DOUBLE: u8 = 1;
const LIGHT: u8 = 2;
const RUN_FLAGS: u8 = DOUBLE | LIGHT;
// A found link's target on the wire.
const FOUND_NUMBER: u8 = 1;
const FOUND_DECLARED: u8 = 2;
// A prompt's flags.
const SECURE: u8 = 1;

/// What the trusted core tells the interface about hyperlinks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Where the links of one image of a page lie: no target, only places
    /// and kinds.
    Page {
        /// Which page.
        key: PageKey,
        /// Its links, for the image of their version.
        links: PageLinks,
    },
    /// What the trusted strip shows of the link under the pointer or the
    /// keyboard's focus, or nothing (requirement 17.6).
    Pointing(Option<Pointing>),
    /// The question the trusted strip asks before a link leaves, or none
    /// (requirements 8.4 and 17.8).
    Prompt(Option<Prompt>),
    /// Open this page of the active channel: a link was followed.
    Show(PageKey),
    /// The page numbers the channel's descriptor links elsewhere: typed,
    /// they go to the trusted core.
    Aliases {
        /// The channel.
        channel: u32,
        /// The page numbers.
        numbers: Vec<u16>,
    },
}

/// Writes a message to the interface's payload; returns its kind.
pub(crate) fn push_message(payload: &mut Vec<u8>, message: &Message) -> Result<u8, ProtocolError> {
    Ok(match message {
        Message::Page { key, links } => {
            push_page_links(payload, *key, links)?;
            KIND_PAGE_LINKS
        }
        Message::Pointing(pointing) => {
            push_pointing(payload, pointing.as_ref())?;
            KIND_POINTING
        }
        Message::Prompt(prompt) => {
            push_prompt(payload, prompt.as_ref())?;
            KIND_PROMPT
        }
        Message::Show(key) => {
            push_key(payload, *key);
            KIND_SHOW
        }
        Message::Aliases { channel, numbers } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            push_numbers(payload, numbers)?;
            KIND_ALIASES
        }
    })
}

/// Whether `kind` is a link message from the trusted core.
pub(crate) fn is_message(kind: u8) -> bool {
    (KIND_PAGE_LINKS..=KIND_ALIASES).contains(&kind)
}

/// Reads a link message from the trusted core of `kind`.
pub(crate) fn read_message(kind: u8, fields: &mut Fields<'_>) -> Result<Message, ProtocolError> {
    let empty = fields.bytes.is_empty();
    Ok(match kind {
        KIND_PAGE_LINKS => {
            let (key, links) = read_page_links(fields)?;
            Message::Page { key, links }
        }
        KIND_POINTING => Message::Pointing(read_pointing(fields, empty)?),
        KIND_PROMPT => Message::Prompt(read_prompt(fields, empty)?),
        KIND_SHOW => Message::Show(fields.key()?),
        KIND_ALIASES => Message::Aliases { channel: fields.u32()?, numbers: read_numbers(fields)? },
        other => return Err(ProtocolError::UnknownKind(other)),
    })
}

/// Where a link the decoder found leads, before the trusted core decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// A page number printed in the page's text (`hyperlinks.md` section 6.1).
    Number(u16),
    /// A target the page declares (`LT`), as written.
    Declared(String),
}

/// A link the decoder found in a page, untrusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The row.
    pub row: u8,
    /// The first column.
    pub column: u8,
    /// How many cells.
    pub length: u8,
    /// Drawn in double height.
    pub double: bool,
    /// The cells' background is light.
    pub light: bool,
    /// Where it leads.
    pub target: Found,
}

/// An `LK` line: a page number the page links elsewhere, as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberLink {
    /// The page number.
    pub number: u16,
    /// The target, as written.
    pub target: String,
}

/// A page number the channel's descriptor links elsewhere (the engine's
/// REQ-WEB-002 `aliases`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    /// The page number.
    pub page: u16,
    /// The target, `name:mpp` or an address, as written.
    pub target: String,
}

/// A service the channel's descriptor names, for `name:mpp` targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    /// Its short name.
    pub name: String,
    /// The address of its descriptor or channel folder.
    pub address: String,
}

/// A cell of a page image: where the pointer or the keyboard is, or a
/// click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    /// The page.
    pub key: PageKey,
    /// The image of the page that was on screen.
    pub version: u32,
    /// The column, 0 to 39.
    pub column: u8,
    /// The row, 0 to 24.
    pub row: u8,
}

/// Untrusted text whole, with a two-byte length before it. A target is
/// never cut: cut short, an address can name another place.
fn push_whole(payload: &mut Vec<u8>, text: &str, limit: usize) -> Result<(), ProtocolError> {
    if text.len() > limit {
        return Err(ProtocolError::BadValue("link target too long"));
    }
    payload.extend_from_slice(&u16::try_from(text.len()).map_err(|_| ProtocolError::BadValue("link target too long"))?.to_be_bytes());
    payload.extend_from_slice(text.as_bytes());
    Ok(())
}

fn raw_text(fields: &mut Fields<'_>, limit: usize) -> Result<String, ProtocolError> {
    let length = usize::from(fields.u16()?);
    if length > limit {
        return Err(ProtocolError::BadValue("text too long"));
    }
    // Invalid UTF-8 is replaced, never trusted; the trusted core's parser
    // refuses what remains if it is not a target.
    Ok(String::from_utf8_lossy(fields.take(length)?).into_owned())
}

fn count(value: usize, limit: usize, what: &'static str) -> Result<u8, ProtocolError> {
    if value > limit {
        return Err(ProtocolError::BadValue(what));
    }
    u8::try_from(value).map_err(|_| ProtocolError::BadValue(what))
}

fn read_count(fields: &mut Fields<'_>, limit: usize, what: &'static str) -> Result<usize, ProtocolError> {
    let value = usize::from(fields.u8()?);
    if value > limit {
        return Err(ProtocolError::BadValue(what));
    }
    Ok(value)
}

/// A run of cells inside the page: on one of its 25 rows, within its 40
/// columns, at least one cell long.
fn place(fields: &mut Fields<'_>) -> Result<(u8, u8, u8), ProtocolError> {
    let (row, column, length) = (fields.u8()?, fields.u8()?, fields.u8()?);
    let end = u16::from(column).saturating_add(u16::from(length));
    if u16::from(row) >= ROWS || length == 0 || end > COLUMNS {
        return Err(ProtocolError::BadValue("link outside the page"));
    }
    Ok((row, column, length))
}

fn flags(double: bool, light: bool) -> u8 {
    (if double { DOUBLE } else { 0 }) | (if light { LIGHT } else { 0 })
}

fn read_flags(fields: &mut Fields<'_>) -> Result<(bool, bool), ProtocolError> {
    let flags = fields.u8()?;
    if flags & !RUN_FLAGS != 0 {
        return Err(ProtocolError::BadValue("link flags"));
    }
    Ok((flags & DOUBLE != 0, flags & LIGHT != 0))
}

fn display_page(fields: &mut Fields<'_>) -> Result<u16, ProtocolError> {
    let page = fields.u16()?;
    page_index(page).ok_or(ProtocolError::BadValue("page number"))?;
    Ok(page)
}

pub use interface_core::controls::ANY_SUBCODE;

/// Where one of Fastext's keys leads, as the decoder reads it from the
/// page's packet X/27/0: untrusted, for the trusted core to decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FastextTarget {
    /// The page.
    pub page: u16,
    /// The sub-page, or none for any.
    pub subcode: Option<u16>,
    /// Another service the channel's descriptor lists, by its place there,
    /// or none for this channel.
    pub service: Option<u8>,
}

/// A page's Fastext: its coloured keys and Index, in
/// `interface_core::controls::FastextKey::ALL`'s order, and whether row 24
/// is shown, without which no key acts (the engine's REQ-SEC-024).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fastext {
    /// The keys' targets.
    pub keys: [Option<FastextTarget>; 5],
    /// Row 24 is drawn and not boxed away.
    pub row24: bool,
}

const FASTEXT_PAGE: u8 = 1;
const FASTEXT_SUBCODE: u8 = 2;
const FASTEXT_SERVICE: u8 = 4;
const FASTEXT_FLAGS: u8 = FASTEXT_PAGE | FASTEXT_SUBCODE | FASTEXT_SERVICE;

pub(crate) fn push_fastext(payload: &mut Vec<u8>, fastext: &Fastext) -> Result<(), ProtocolError> {
    payload.push(u8::from(fastext.row24));
    for key in &fastext.keys {
        let Some(target) = key else {
            payload.push(0);
            continue;
        };
        page_index(target.page).ok_or(ProtocolError::BadValue("Fastext page"))?;
        if target.subcode.is_some_and(|subcode| subcode >= ANY_SUBCODE) {
            return Err(ProtocolError::BadValue("Fastext sub-code"));
        }
        let flags = FASTEXT_PAGE | if target.subcode.is_some() { FASTEXT_SUBCODE } else { 0 } | if target.service.is_some() { FASTEXT_SERVICE } else { 0 };
        payload.push(flags);
        payload.extend_from_slice(&target.page.to_be_bytes());
        if let Some(subcode) = target.subcode {
            payload.extend_from_slice(&subcode.to_be_bytes());
        }
        if let Some(service) = target.service {
            payload.push(service);
        }
    }
    Ok(())
}

pub(crate) fn read_fastext(fields: &mut Fields<'_>) -> Result<Fastext, ProtocolError> {
    let row24 = match fields.u8()? {
        0 => false,
        1 => true,
        _ => return Err(ProtocolError::BadValue("Fastext row 24")),
    };
    let mut keys = [None; 5];
    for key in &mut keys {
        let flags = fields.u8()?;
        if flags == 0 {
            continue;
        }
        if flags & !FASTEXT_FLAGS != 0 || flags & FASTEXT_PAGE == 0 {
            return Err(ProtocolError::BadValue("Fastext flags"));
        }
        let page = display_page(fields)?;
        let subcode = if flags & FASTEXT_SUBCODE == 0 {
            None
        } else {
            let subcode = fields.u16()?;
            if subcode >= ANY_SUBCODE {
                return Err(ProtocolError::BadValue("Fastext sub-code"));
            }
            Some(subcode)
        };
        let service = if flags & FASTEXT_SERVICE == 0 { None } else { Some(fields.u8()?) };
        *key = Some(FastextTarget { page, subcode, service });
    }
    Ok(Fastext { keys, row24 })
}

/// The decoder's links of one page.
pub(crate) fn push_found(payload: &mut Vec<u8>, candidates: &[Candidate], numbers: &[NumberLink]) -> Result<(), ProtocolError> {
    payload.push(count(candidates.len(), MAX_RUNS, "too many links")?);
    for candidate in candidates {
        payload.extend_from_slice(&[candidate.row, candidate.column, candidate.length, flags(candidate.double, candidate.light)]);
        match &candidate.target {
            Found::Number(page) => {
                payload.push(FOUND_NUMBER);
                payload.extend_from_slice(&page.to_be_bytes());
            }
            Found::Declared(target) => {
                payload.push(FOUND_DECLARED);
                push_whole(payload, target, MAX_TARGET)?;
            }
        }
    }
    payload.push(count(numbers.len(), MAX_NUMBER_LINKS, "too many number links")?);
    for link in numbers {
        payload.extend_from_slice(&link.number.to_be_bytes());
        push_whole(payload, &link.target, MAX_TARGET)?;
    }
    Ok(())
}

pub(crate) fn read_found(fields: &mut Fields<'_>) -> Result<(Vec<Candidate>, Vec<NumberLink>), ProtocolError> {
    let candidates = (0..read_count(fields, MAX_RUNS, "too many links")?)
        .map(|_| {
            let (row, column, length) = place(fields)?;
            let (double, light) = read_flags(fields)?;
            let target = match fields.u8()? {
                FOUND_NUMBER => Found::Number(display_page(fields)?),
                FOUND_DECLARED => Found::Declared(raw_text(fields, MAX_TARGET)?),
                _ => return Err(ProtocolError::BadValue("link target kind")),
            };
            Ok::<_, ProtocolError>(Candidate { row, column, length, double, light, target })
        })
        .collect::<Result<_, _>>()?;
    let numbers = (0..read_count(fields, MAX_NUMBER_LINKS, "too many number links")?)
        .map(|_| Ok::<_, ProtocolError>(NumberLink { number: display_page(fields)?, target: raw_text(fields, MAX_TARGET)? }))
        .collect::<Result<_, _>>()?;
    Ok((candidates, numbers))
}

/// The fetcher's links of its channel.
pub(crate) fn push_channel_links(payload: &mut Vec<u8>, aliases: &[Alias], services: &[Service]) -> Result<(), ProtocolError> {
    payload.push(count(aliases.len(), MAX_ALIASES, "too many aliases")?);
    for alias in aliases {
        payload.extend_from_slice(&alias.page.to_be_bytes());
        push_whole(payload, &alias.target, MAX_ADDRESS)?;
    }
    payload.push(count(services.len(), MAX_SERVICES, "too many services")?);
    for service in services {
        push_whole(payload, &service.name, MAX_SERVICE_NAME)?;
        push_whole(payload, &service.address, MAX_ADDRESS)?;
    }
    Ok(())
}

pub(crate) fn read_channel_links(fields: &mut Fields<'_>) -> Result<(Vec<Alias>, Vec<Service>), ProtocolError> {
    let aliases = (0..read_count(fields, MAX_ALIASES, "too many aliases")?)
        .map(|_| Ok::<_, ProtocolError>(Alias { page: display_page(fields)?, target: raw_text(fields, MAX_ADDRESS)? }))
        .collect::<Result<_, _>>()?;
    let services = (0..read_count(fields, MAX_SERVICES, "too many services")?)
        .map(|_| Ok::<_, ProtocolError>(Service { name: raw_text(fields, MAX_SERVICE_NAME)?, address: raw_text(fields, MAX_ADDRESS)? }))
        .collect::<Result<_, _>>()?;
    Ok((aliases, services))
}

/// Where a page's links lie, for the interface.
fn push_page_links(payload: &mut Vec<u8>, key: PageKey, links: &PageLinks) -> Result<(), ProtocolError> {
    push_key(payload, key);
    payload.extend_from_slice(&links.version.to_be_bytes());
    payload.push(count(links.runs.len(), MAX_RUNS, "too many links")?);
    for run in &links.runs {
        payload.extend_from_slice(&[run.row, run.column, run.length, flags(run.double, run.light), run.kind as u8]);
    }
    push_numbers(payload, &links.remapped)
}

fn read_page_links(fields: &mut Fields<'_>) -> Result<(PageKey, PageLinks), ProtocolError> {
    let key = fields.key()?;
    let version = fields.u32()?;
    let runs = (0..read_count(fields, MAX_RUNS, "too many links")?)
        .map(|_| {
            let (row, column, length) = place(fields)?;
            let (double, light) = read_flags(fields)?;
            let kind = LinkKind::from_byte(fields.u8()?).ok_or(ProtocolError::BadValue("link kind"))?;
            Ok::<_, ProtocolError>(LinkRun { row, column, length, double, light, kind })
        })
        .collect::<Result<_, _>>()?;
    Ok((key, PageLinks { version, runs, remapped: read_numbers(fields)? }))
}

/// Page numbers, each a display page.
fn push_numbers(payload: &mut Vec<u8>, numbers: &[u16]) -> Result<(), ProtocolError> {
    payload.push(count(numbers.len(), MAX_REMAPPED, "too many page numbers")?);
    for number in numbers {
        page_index(*number).ok_or(ProtocolError::BadValue("page number"))?;
        payload.extend_from_slice(&number.to_be_bytes());
    }
    Ok(())
}

fn read_numbers(fields: &mut Fields<'_>) -> Result<Vec<u16>, ProtocolError> {
    (0..read_count(fields, MAX_REMAPPED, "too many page numbers")?).map(|_| display_page(fields)).collect()
}

/// What the strip shows of a link, or nothing.
fn push_pointing(payload: &mut Vec<u8>, pointing: Option<&Pointing>) -> Result<(), ProtocolError> {
    if let Some(pointing) = pointing {
        payload.push(pointing.kind as u8);
        push_text(payload, &pointing.text, MAX_DESTINATION)?;
    }
    Ok(())
}

fn read_pointing(fields: &mut Fields<'_>, empty: bool) -> Result<Option<Pointing>, ProtocolError> {
    if empty {
        return Ok(None);
    }
    let kind = LinkKind::from_byte(fields.u8()?).ok_or(ProtocolError::BadValue("link kind"))?;
    Ok(Some(Pointing { kind, text: fields.text(MAX_DESTINATION)? }))
}

/// A question for the strip, or none.
fn push_prompt(payload: &mut Vec<u8>, prompt: Option<&Prompt>) -> Result<(), ProtocolError> {
    if let Some(prompt) = prompt {
        payload.extend_from_slice(&prompt.id.to_be_bytes());
        payload.push(prompt.kind as u8);
        payload.push(if prompt.secure { SECURE } else { 0 });
        push_text(payload, &prompt.destination, MAX_DESTINATION)?;
    }
    Ok(())
}

fn read_prompt(fields: &mut Fields<'_>, empty: bool) -> Result<Option<Prompt>, ProtocolError> {
    if empty {
        return Ok(None);
    }
    let id = fields.u32()?;
    let kind = PromptKind::from_byte(fields.u8()?).ok_or(ProtocolError::BadValue("prompt kind"))?;
    let flags = fields.u8()?;
    if flags & !SECURE != 0 {
        return Err(ProtocolError::BadValue("prompt flags"));
    }
    Ok(Some(Prompt { id, kind, secure: flags & SECURE != 0, destination: fields.text(MAX_DESTINATION)? }))
}

/// A cell of a page image, or none.
pub(crate) fn push_spot(payload: &mut Vec<u8>, spot: Option<&Spot>) {
    if let Some(spot) = spot {
        push_key(payload, spot.key);
        payload.extend_from_slice(&spot.version.to_be_bytes());
        payload.extend_from_slice(&[spot.column, spot.row]);
    }
}

pub(crate) fn read_spot(fields: &mut Fields<'_>) -> Result<Spot, ProtocolError> {
    let key = fields.key()?;
    let version = fields.u32()?;
    let (column, row) = (fields.u8()?, fields.u8()?);
    if u16::from(column) >= COLUMNS || u16::from(row) >= ROWS {
        return Err(ProtocolError::BadValue("cell outside the page"));
    }
    Ok(Spot { key, version, column, row })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{FromDecoder, read_from_decoder, write_to_core as decoder_writes};
    use crate::fetcher::{FromFetcher, read_from_fetcher, write_to_core as fetcher_writes};
    use crate::{ToCore, ToInterface, read_from_core, read_from_interface, write_to_core, write_to_interface};

    const KEY: PageKey = PageKey { channel: 7, page: 0x123 };

    fn run(row: u8, kind: LinkKind) -> LinkRun {
        LinkRun { row, column: 4, length: 3, double: row.is_multiple_of(2), light: row.is_multiple_of(3), kind }
    }

    fn page_links() -> PageLinks {
        PageLinks { version: 9, runs: vec![run(3, LinkKind::Page), run(4, LinkKind::Channel), run(6, LinkKind::Web)], remapped: vec![0x302, 0x500] }
    }

    #[test]
    fn every_link_message_round_trips() {
        for message in [
            Message::Page { key: KEY, links: page_links() },
            Message::Pointing(Some(Pointing { kind: LinkKind::Web, text: "https://www.example.invalid/ (opens in your web browser)".into() })),
            Message::Pointing(None),
            Message::Prompt(Some(Prompt { id: 3, kind: PromptKind::Leave, destination: "weather.invalid".into(), secure: true })),
            Message::Prompt(None),
            Message::Show(KEY),
            Message::Aliases { channel: 7, numbers: vec![0x500] },
        ] {
            let message = ToInterface::Link(message);
            let mut bytes = Vec::new();
            write_to_interface(&mut bytes, &message).unwrap();
            assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
        let spot = Spot { key: KEY, version: 9, column: 39, row: 24 };
        for message in [
            ToCore::Point(Some(spot)),
            ToCore::Point(None),
            ToCore::Typed { number: 0x302, opened: Some((KEY, 9)) },
            ToCore::Typed { number: 0x500, opened: None },
            ToCore::Focused(false),
            ToCore::Confirm(3),
            ToCore::Cancel(3),
        ] {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &message).unwrap();
            assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
        let found = FromDecoder::Links {
            page: 0x100,
            with_image: true,
            candidates: vec![
                Candidate { row: 5, column: 10, length: 3, double: true, light: false, target: Found::Number(0x302) },
                Candidate { row: 7, column: 0, length: 12, double: false, light: true, target: Found::Declared("web:https://www.example.invalid/".into()) },
            ],
            numbers: vec![NumberLink { number: 0x302, target: "sport:302".into() }],
            fastext: Fastext::default(),
        };
        let mut bytes = Vec::new();
        decoder_writes(&mut bytes, &found).unwrap();
        assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap().unwrap(), found);
        let channel = FromFetcher::Links {
            aliases: vec![Alias { page: 0x500, target: "https://weather.invalid/".into() }],
            services: vec![Service { name: "sport".into(), address: "https://sport.invalid/".into() }],
        };
        let mut bytes = Vec::new();
        fetcher_writes(&mut bytes, &channel).unwrap();
        assert_eq!(read_from_fetcher(&mut bytes.as_slice()).unwrap().unwrap(), channel);
    }

    /// The payload of a page's links with one run of these bytes.
    fn one_run(place: [u8; 3], flags: u8, kind: u8) -> Vec<u8> {
        let mut payload = vec![0, 0, 0, 7, 0x01, 0x23, 0, 0, 0, 9, 1];
        payload.extend_from_slice(&place);
        payload.extend_from_slice(&[flags, kind, 0]);
        let mut bytes = vec![KIND_PAGE_LINKS];
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&payload);
        bytes
    }

    #[test]
    fn malformed_links_are_refused() {
        let read = |bytes: Vec<u8>| read_from_core(&mut bytes.as_slice());
        assert!(read(one_run([3, 4, 3], 0, 1)).is_ok());
        for (place, flags, kind, why) in [
            ([25, 4, 3], 0, 1, "link outside the page"),
            ([3, 38, 3], 0, 1, "link outside the page"),
            ([3, 4, 0], 0, 1, "link outside the page"),
            ([3, 4, 3], 4, 1, "link flags"),
            ([3, 4, 3], 0, 0, "link kind"),
            ([3, 4, 3], 0, 4, "link kind"),
        ] {
            assert!(matches!(read(one_run(place, flags, kind)), Err(ProtocolError::BadValue(reason)) if reason == why), "{why}");
        }
        // More runs than a page may have.
        let mut payload = vec![0, 0, 0, 7, 0x01, 0x23, 0, 0, 0, 9, u8::try_from(MAX_RUNS + 1).unwrap()];
        payload.extend(std::iter::repeat_n([3, 4, 3, 0, 1], MAX_RUNS + 1).flatten());
        payload.push(0);
        let mut bytes = vec![KIND_PAGE_LINKS];
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&payload);
        assert!(matches!(read(bytes), Err(ProtocolError::BadValue("too many links"))));
        // A remapped number that is not a display page.
        let links = PageLinks { remapped: vec![0x1A0], ..page_links() };
        let mut bytes = Vec::new();
        assert!(write_to_interface(&mut bytes, &ToInterface::Link(Message::Page { key: KEY, links })).is_err());
    }

    #[test]
    fn a_target_too_long_is_refused_never_cut() {
        // Cut short, an address can name another place.
        let target = format!("web:https://example.invalid/{}", "a".repeat(MAX_TARGET));
        let found = FromDecoder::Links {
            page: 0x100,
            with_image: false,
            candidates: vec![Candidate { row: 5, column: 0, length: 3, double: false, light: false, target: Found::Declared(target) }],
            numbers: Vec::new(),
            fastext: Fastext::default(),
        };
        assert!(matches!(decoder_writes(&mut Vec::new(), &found), Err(ProtocolError::BadValue("link target too long"))));
        // Written by hand, a reader refuses it too.
        let mut payload = vec![0x01, 0x00, 0, 1, 5, 0, 3, 0, FOUND_DECLARED];
        payload.extend_from_slice(&u16::try_from(MAX_TARGET + 1).unwrap().to_be_bytes());
        payload.extend(std::iter::repeat_n(b'a', MAX_TARGET + 1));
        payload.push(0);
        let mut bytes = vec![68];
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&payload);
        assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("text too long"))));
    }

    #[test]
    fn the_strips_text_must_be_cleaned_and_its_flags_known() {
        let hostile = Pointing { kind: LinkKind::Page, text: "Page\u{202E}203".into() };
        assert!(write_to_interface(&mut Vec::new(), &ToInterface::Link(Message::Pointing(Some(hostile)))).is_err());
        let mut bytes = vec![KIND_PROMPT, 0, 0, 0, 8, 0, 0, 0, 1, 1, 2, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("prompt flags"))));
        bytes[9] = 3;
        bytes[10] = 0;
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("prompt kind"))));
    }

    #[test]
    fn a_typed_number_must_be_a_display_page() {
        assert!(write_to_core(&mut Vec::new(), &ToCore::Typed { number: 0x1A0, opened: None }).is_err());
        let bytes = [25, 0, 0, 0, 2, 0x09, 0x00];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
    }
}
