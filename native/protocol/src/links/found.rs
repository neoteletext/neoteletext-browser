//! What a decoder finds in a page and a fetcher in its channel's
//! descriptor (`hyperlinks.md` sections 6 and 8.1): untrusted, so every
//! target travels whole as raw text for the trusted core's own parser, and
//! a run of cells is checked to lie inside the page.

use interface_core::canvas::{COLUMNS, ROWS};

use super::{MAX_ALIASES, MAX_NUMBER_LINKS, MAX_RUNS, MAX_SERVICE_NAME, MAX_SERVICES, MAX_TARGET};
use crate::{Fields, MAX_ADDRESS, ProtocolError, push_count_u8, raw_text};

// A run's flags on the wire.
const DOUBLE: u8 = 1;
const LIGHT: u8 = 2;
const RUN_FLAGS: u8 = DOUBLE | LIGHT;
// A found link's target on the wire.
const FOUND_NUMBER: u8 = 1;
pub(super) const FOUND_DECLARED: u8 = 2;

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

/// A run of cells inside the page: on one of its 25 rows, within its 40
/// columns, at least one cell long.
pub(super) fn place(fields: &mut Fields<'_>) -> Result<(u8, u8, u8), ProtocolError> {
    let (row, column, length) = (fields.u8()?, fields.u8()?, fields.u8()?);
    let end = u16::from(column).saturating_add(u16::from(length));
    if u16::from(row) >= ROWS || length == 0 || end > COLUMNS {
        return Err(ProtocolError::BadValue("link outside the page"));
    }
    Ok((row, column, length))
}

pub(super) fn flags(double: bool, light: bool) -> u8 {
    (if double { DOUBLE } else { 0 }) | (if light { LIGHT } else { 0 })
}

pub(super) fn read_flags(fields: &mut Fields<'_>) -> Result<(bool, bool), ProtocolError> {
    let flags = fields.u8()?;
    if flags & !RUN_FLAGS != 0 {
        return Err(ProtocolError::BadValue("link flags"));
    }
    Ok((flags & DOUBLE != 0, flags & LIGHT != 0))
}

/// The decoder's links of one page.
pub(crate) fn push_found(payload: &mut Vec<u8>, candidates: &[Candidate], numbers: &[NumberLink]) -> Result<(), ProtocolError> {
    push_count_u8(payload, candidates.len(), MAX_RUNS, "too many links")?;
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
    push_count_u8(payload, numbers.len(), MAX_NUMBER_LINKS, "too many number links")?;
    for link in numbers {
        payload.extend_from_slice(&link.number.to_be_bytes());
        push_whole(payload, &link.target, MAX_TARGET)?;
    }
    Ok(())
}

pub(crate) fn read_found(fields: &mut Fields<'_>) -> Result<(Vec<Candidate>, Vec<NumberLink>), ProtocolError> {
    let candidates = (0..fields.count_u8(MAX_RUNS, "too many links")?)
        .map(|_| {
            let (row, column, length) = place(fields)?;
            let (double, light) = read_flags(fields)?;
            let target = match fields.u8()? {
                FOUND_NUMBER => Found::Number(fields.display_page()?),
                FOUND_DECLARED => Found::Declared(raw_text(fields, MAX_TARGET, "text too long")?),
                _ => return Err(ProtocolError::BadValue("link target kind")),
            };
            Ok::<_, ProtocolError>(Candidate { row, column, length, double, light, target })
        })
        .collect::<Result<_, _>>()?;
    let numbers = (0..fields.count_u8(MAX_NUMBER_LINKS, "too many number links")?)
        .map(|_| Ok::<_, ProtocolError>(NumberLink { number: fields.display_page()?, target: raw_text(fields, MAX_TARGET, "text too long")? }))
        .collect::<Result<_, _>>()?;
    Ok((candidates, numbers))
}

/// The fetcher's links of its channel.
pub(crate) fn push_channel_links(payload: &mut Vec<u8>, aliases: &[Alias], services: &[Service]) -> Result<(), ProtocolError> {
    push_count_u8(payload, aliases.len(), MAX_ALIASES, "too many aliases")?;
    for alias in aliases {
        payload.extend_from_slice(&alias.page.to_be_bytes());
        push_whole(payload, &alias.target, MAX_ADDRESS)?;
    }
    push_count_u8(payload, services.len(), MAX_SERVICES, "too many services")?;
    for service in services {
        push_whole(payload, &service.name, MAX_SERVICE_NAME)?;
        push_whole(payload, &service.address, MAX_ADDRESS)?;
    }
    Ok(())
}

pub(crate) fn read_channel_links(fields: &mut Fields<'_>) -> Result<(Vec<Alias>, Vec<Service>), ProtocolError> {
    let aliases = (0..fields.count_u8(MAX_ALIASES, "too many aliases")?)
        .map(|_| Ok::<_, ProtocolError>(Alias { page: fields.display_page()?, target: raw_text(fields, MAX_ADDRESS, "text too long")? }))
        .collect::<Result<_, _>>()?;
    let services = (0..fields.count_u8(MAX_SERVICES, "too many services")?)
        .map(|_| {
            Ok::<_, ProtocolError>(Service {
                name: raw_text(fields, MAX_SERVICE_NAME, "text too long")?,
                address: raw_text(fields, MAX_ADDRESS, "text too long")?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok((aliases, services))
}
