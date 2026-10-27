//! What the trusted core tells the interface about a page's links: where
//! they lie and what kind of place each leads to, never a target; the
//! page numbers typed that lead elsewhere; and the strip's text and
//! question.

use interface_core::canvas::PageKey;
use interface_core::channel::page_index;

use super::found::{flags, place, push_area, read_area, read_flags};
use super::{LinkKind, LinkRun, MAX_CONTROLS, MAX_DESTINATION, MAX_POINTER_AREAS, MAX_REMAPPED, MAX_RUNS, PageLinks, Pointing, Prompt, PromptKind};
use crate::{Fields, ProtocolError, push_count_u8, push_key, push_text};

// A prompt's flags.
const SECURE: u8 = 1;

/// Where a page's links lie, for the interface.
pub(super) fn push_page_links(payload: &mut Vec<u8>, key: PageKey, links: &PageLinks) -> Result<(), ProtocolError> {
    push_key(payload, key);
    payload.extend_from_slice(&links.version.to_be_bytes());
    push_count_u8(payload, links.runs.len(), MAX_RUNS, "too many links")?;
    for run in &links.runs {
        payload.extend_from_slice(&[run.row, run.column, run.length, flags(run.double, run.light), run.kind as u8]);
    }
    push_count_u8(payload, links.controls.len(), MAX_CONTROLS, "too many controls")?;
    for control in &links.controls {
        push_area(payload, *control);
    }
    push_count_u8(payload, links.pointer_areas.len(), MAX_POINTER_AREAS, "too many pointer areas")?;
    for area in &links.pointer_areas {
        push_area(payload, *area);
    }
    push_numbers(payload, &links.remapped)
}

pub(super) fn read_page_links(fields: &mut Fields<'_>) -> Result<(PageKey, PageLinks), ProtocolError> {
    let key = fields.key()?;
    let version = fields.u32()?;
    let runs = (0..fields.count_u8(MAX_RUNS, "too many links")?)
        .map(|_| {
            let (row, column, length) = place(fields)?;
            let (double, light) = read_flags(fields)?;
            let kind = LinkKind::from_byte(fields.u8()?).ok_or(ProtocolError::BadValue("link kind"))?;
            Ok::<_, ProtocolError>(LinkRun { row, column, length, double, light, kind })
        })
        .collect::<Result<_, _>>()?;
    let controls = (0..fields.count_u8(MAX_CONTROLS, "too many controls")?).map(|_| read_area(fields)).collect::<Result<_, _>>()?;
    let pointer_areas = (0..fields.count_u8(MAX_POINTER_AREAS, "too many pointer areas")?).map(|_| read_area(fields)).collect::<Result<_, _>>()?;
    Ok((key, PageLinks { version, runs, controls, pointer_areas, remapped: read_numbers(fields)? }))
}

/// Page numbers, each a display page.
pub(super) fn push_numbers(payload: &mut Vec<u8>, numbers: &[u16]) -> Result<(), ProtocolError> {
    push_count_u8(payload, numbers.len(), MAX_REMAPPED, "too many page numbers")?;
    for number in numbers {
        page_index(*number).ok_or(ProtocolError::BadValue("page number"))?;
        payload.extend_from_slice(&number.to_be_bytes());
    }
    Ok(())
}

pub(super) fn read_numbers(fields: &mut Fields<'_>) -> Result<Vec<u16>, ProtocolError> {
    (0..fields.count_u8(MAX_REMAPPED, "too many page numbers")?).map(|_| fields.display_page()).collect()
}

/// What the strip shows of a link, or nothing.
pub(super) fn push_pointing(payload: &mut Vec<u8>, pointing: Option<&Pointing>) -> Result<(), ProtocolError> {
    if let Some(pointing) = pointing {
        payload.push(pointing.kind as u8);
        push_text(payload, &pointing.text, MAX_DESTINATION)?;
    }
    Ok(())
}

pub(super) fn read_pointing(fields: &mut Fields<'_>, empty: bool) -> Result<Option<Pointing>, ProtocolError> {
    if empty {
        return Ok(None);
    }
    let kind = LinkKind::from_byte(fields.u8()?).ok_or(ProtocolError::BadValue("link kind"))?;
    Ok(Some(Pointing { kind, text: fields.text(MAX_DESTINATION)? }))
}

/// A question for the strip, or none.
pub(super) fn push_prompt(payload: &mut Vec<u8>, prompt: Option<&Prompt>) -> Result<(), ProtocolError> {
    if let Some(prompt) = prompt {
        payload.extend_from_slice(&prompt.id.to_be_bytes());
        payload.push(prompt.kind as u8);
        payload.push(if prompt.secure { SECURE } else { 0 });
        push_text(payload, &prompt.destination, MAX_DESTINATION)?;
    }
    Ok(())
}

pub(super) fn read_prompt(fields: &mut Fields<'_>, empty: bool) -> Result<Option<Prompt>, ProtocolError> {
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
