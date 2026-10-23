//! The open page's controls between the trusted core and the interface
//! (`page-controls.md` section 5, decision 67): what the page has and
//! which are on, and the user's key for one of them, as strip input.

use interface_core::channel::page_index;
use interface_core::controls::{FastextKey, MAX_SUBPAGES, PageControl, PageControls};

use super::{Fields, ProtocolError};

const CONTROL_FASTEXT: u8 = 1;
const CONTROL_REVEAL: u8 = 2;
const CONTROL_HOLD: u8 = 3;
const CONTROL_MIX: u8 = 4;
const CONTROL_NEXT: u8 = 5;
const CONTROL_PREVIOUS: u8 = 6;
const CONTROL_LATEST: u8 = 7;

/// The controls' flags: a page open, reveal, hold, mix, a sub-page chosen.
const OPEN: u8 = 1;
const REVEAL: u8 = 2;
const HOLD: u8 = 4;
const MIX: u8 = 8;
const CHOSEN: u8 = 16;
const FLAGS: u8 = OPEN | REVEAL | HOLD | MIX | CHOSEN;

pub(crate) fn push_control(payload: &mut Vec<u8>, control: PageControl) {
    match control {
        PageControl::Fastext(key) => payload.extend_from_slice(&[CONTROL_FASTEXT, u8::try_from(key.index()).unwrap_or(u8::MAX)]),
        PageControl::Reveal => payload.push(CONTROL_REVEAL),
        PageControl::Hold => payload.push(CONTROL_HOLD),
        PageControl::Mix => payload.push(CONTROL_MIX),
        PageControl::SubPage(true) => payload.push(CONTROL_NEXT),
        PageControl::SubPage(false) => payload.push(CONTROL_PREVIOUS),
        PageControl::Latest => payload.push(CONTROL_LATEST),
    }
}

pub(crate) fn read_control(fields: &mut Fields<'_>) -> Result<PageControl, ProtocolError> {
    Ok(match fields.u8()? {
        CONTROL_FASTEXT => PageControl::Fastext(FastextKey::at(usize::from(fields.u8()?)).ok_or(ProtocolError::BadValue("Fastext key"))?),
        CONTROL_REVEAL => PageControl::Reveal,
        CONTROL_HOLD => PageControl::Hold,
        CONTROL_MIX => PageControl::Mix,
        CONTROL_NEXT => PageControl::SubPage(true),
        CONTROL_PREVIOUS => PageControl::SubPage(false),
        CONTROL_LATEST => PageControl::Latest,
        _ => return Err(ProtocolError::BadValue("page control")),
    })
}

pub(crate) fn push_controls(payload: &mut Vec<u8>, controls: &PageControls) -> Result<(), ProtocolError> {
    if usize::from(controls.subpages) > MAX_SUBPAGES || controls.chosen.is_some_and(|chosen| chosen >= controls.subpages) {
        return Err(ProtocolError::BadValue("sub-pages"));
    }
    let flags = if controls.page.is_some() { OPEN } else { 0 }
        | if controls.reveal { REVEAL } else { 0 }
        | if controls.hold { HOLD } else { 0 }
        | if controls.mix { MIX } else { 0 }
        | if controls.chosen.is_some() { CHOSEN } else { 0 };
    payload.push(flags);
    if let Some(page) = controls.page {
        page_index(page).ok_or(ProtocolError::BadValue("page number"))?;
        payload.extend_from_slice(&page.to_be_bytes());
    }
    let keys = controls.fastext.iter().enumerate().fold(0_u8, |bits, (index, on)| if *on { bits | (1 << index) } else { bits });
    payload.extend_from_slice(&[keys, controls.subpages]);
    if let Some(chosen) = controls.chosen {
        payload.push(chosen);
    }
    payload.extend_from_slice(&controls.subcode.to_be_bytes());
    Ok(())
}

pub(crate) fn read_controls(fields: &mut Fields<'_>) -> Result<PageControls, ProtocolError> {
    let flags = fields.u8()?;
    if flags & !FLAGS != 0 {
        return Err(ProtocolError::BadValue("control flags"));
    }
    let page = if flags & OPEN == 0 {
        None
    } else {
        let page = fields.u16()?;
        page_index(page).ok_or(ProtocolError::BadValue("page number"))?;
        Some(page)
    };
    let keys = fields.u8()?;
    if usize::from(keys) >> FastextKey::ALL.len() != 0 {
        return Err(ProtocolError::BadValue("Fastext keys"));
    }
    let subpages = fields.u8()?;
    if usize::from(subpages) > MAX_SUBPAGES {
        return Err(ProtocolError::BadValue("sub-pages"));
    }
    let chosen = if flags & CHOSEN == 0 {
        None
    } else {
        let chosen = fields.u8()?;
        if chosen >= subpages {
            return Err(ProtocolError::BadValue("sub-page chosen"));
        }
        Some(chosen)
    };
    let subcode = fields.u16()?;
    Ok(PageControls {
        page,
        fastext: core::array::from_fn(|index| keys & (1 << index) != 0),
        reveal: flags & REVEAL != 0,
        hold: flags & HOLD != 0,
        mix: flags & MIX != 0,
        subpages,
        chosen,
        subcode,
    })
}
