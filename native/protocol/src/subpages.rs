//! A page's sub-pages on the wire (decisions 111, 112 and 119): a count,
//! the place and neighbours of the one drawn, and runs of rows asked for by
//! range, so that no message grows with the standard's 8,190 sub-pages a
//! page (`neoteletext-common/STANDARD-LIMITS.md` section 4). Every count
//! and place is 16-bit; every sub-code is any value of the thirteen bits a
//! sub-code has (the engine's REQ-NUM-002 and REQ-NUM-006), and nothing
//! else.

use neoteletext_core::limits::{MAX_SUBPAGES, is_sub_code};

use crate::{Fields, ProtocolError};

/// A page's sub-pages from the one drawn: how many it has, which is drawn,
/// its place among them and the sub-pages either side of it, round the
/// list as a set's sub-page keys go (the engine's REQ-NAV-005). The
/// trusted core steps by it, holding no list (decision 112 item 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubPageStep {
    /// How many sub-pages are listed: at most 8,190.
    pub count: u16,
    /// The sub-code of the one drawn.
    pub shown: u16,
    /// Its place among them, from 0, when it is listed.
    pub place: Option<u16>,
    /// The sub-page before it, round the list; none with fewer than two.
    pub previous: Option<u16>,
    /// The sub-page after it.
    pub next: Option<u16>,
}

/// A run of a page's sub-pages in the page's order, each with its title:
/// how many the page has, the place of the first row, and the rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubPageRows {
    /// How many sub-pages the page has: at most 8,190.
    pub count: u16,
    /// The place of the first row, from 0.
    pub first: u16,
    /// Each row's sub-code and title.
    pub rows: Vec<(u16, String)>,
}

/// A run of rows asked for: the place of the first, from 0, and how many.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubPageRange {
    /// The first row's place.
    pub first: u16,
    /// How many rows, at most [`interface_core::controls::SUBPAGE_ROWS`].
    pub count: u16,
}

const STEP_PLACE: u8 = 1;
const STEP_PREVIOUS: u8 = 2;
const STEP_NEXT: u8 = 4;
const STEP_FLAGS: u8 = STEP_PLACE | STEP_PREVIOUS | STEP_NEXT;

/// A sub-code read, refused unless every bit set is one of the thirteen.
pub(crate) fn read_subcode(fields: &mut Fields<'_>) -> Result<u16, ProtocolError> {
    let subcode = fields.u16()?;
    if is_sub_code(subcode) { Ok(subcode) } else { Err(ProtocolError::BadValue("sub-code")) }
}

/// A count of sub-pages read, refused above the standard's maximum.
pub(crate) fn read_count(fields: &mut Fields<'_>) -> Result<u16, ProtocolError> {
    let count = fields.u16()?;
    if usize::from(count) > MAX_SUBPAGES { Err(ProtocolError::BadValue("too many sub-pages")) } else { Ok(count) }
}

fn checked_count(count: u16) -> Result<u16, ProtocolError> {
    if usize::from(count) > MAX_SUBPAGES { Err(ProtocolError::BadValue("too many sub-pages")) } else { Ok(count) }
}

fn checked_subcode(subcode: u16) -> Result<u16, ProtocolError> {
    if is_sub_code(subcode) { Ok(subcode) } else { Err(ProtocolError::BadValue("sub-code")) }
}

pub(crate) fn push_step(payload: &mut Vec<u8>, step: &SubPageStep) -> Result<(), ProtocolError> {
    checked_count(step.count)?;
    checked_subcode(step.shown)?;
    if step.place.is_some_and(|place| place >= step.count) {
        return Err(ProtocolError::BadValue("sub-page place"));
    }
    for neighbour in [step.previous, step.next].into_iter().flatten() {
        checked_subcode(neighbour)?;
    }
    let flags = if step.place.is_some() { STEP_PLACE } else { 0 }
        | if step.previous.is_some() { STEP_PREVIOUS } else { 0 }
        | if step.next.is_some() { STEP_NEXT } else { 0 };
    payload.extend_from_slice(&step.count.to_be_bytes());
    payload.extend_from_slice(&step.shown.to_be_bytes());
    payload.push(flags);
    for value in [step.place, step.previous, step.next].into_iter().flatten() {
        payload.extend_from_slice(&value.to_be_bytes());
    }
    Ok(())
}

pub(crate) fn read_step(fields: &mut Fields<'_>) -> Result<SubPageStep, ProtocolError> {
    let count = read_count(fields)?;
    let shown = read_subcode(fields)?;
    let flags = fields.u8()?;
    if flags & !STEP_FLAGS != 0 {
        return Err(ProtocolError::BadValue("sub-page flags"));
    }
    let place = if flags & STEP_PLACE == 0 { None } else { Some(fields.u16()?) };
    if place.is_some_and(|place| place >= count) {
        return Err(ProtocolError::BadValue("sub-page place"));
    }
    let previous = if flags & STEP_PREVIOUS == 0 { None } else { Some(read_subcode(fields)?) };
    let next = if flags & STEP_NEXT == 0 { None } else { Some(read_subcode(fields)?) };
    Ok(SubPageStep { count, shown, place, previous, next })
}

pub(crate) fn push_range(payload: &mut Vec<u8>, range: SubPageRange, max_rows: usize) -> Result<(), ProtocolError> {
    if usize::from(range.count) > max_rows || usize::from(range.first) >= MAX_SUBPAGES {
        return Err(ProtocolError::BadValue("sub-page range"));
    }
    payload.extend_from_slice(&range.first.to_be_bytes());
    payload.extend_from_slice(&range.count.to_be_bytes());
    Ok(())
}

pub(crate) fn read_range(fields: &mut Fields<'_>, max_rows: usize) -> Result<SubPageRange, ProtocolError> {
    let (first, count) = (fields.u16()?, fields.u16()?);
    if usize::from(count) > max_rows || usize::from(first) >= MAX_SUBPAGES {
        return Err(ProtocolError::BadValue("sub-page range"));
    }
    Ok(SubPageRange { first, count })
}

/// Writes `rows`, at most `max_rows` of them, each title by `text`.
pub(crate) fn push_rows(
    payload: &mut Vec<u8>,
    rows: &SubPageRows,
    max_rows: usize,
    mut text: impl FnMut(&mut Vec<u8>, &str) -> Result<(), ProtocolError>,
) -> Result<(), ProtocolError> {
    checked_count(rows.count)?;
    let length = rows.rows.len();
    if length > max_rows || usize::from(rows.first).saturating_add(length) > usize::from(rows.count) {
        return Err(ProtocolError::BadValue("sub-page rows"));
    }
    payload.extend_from_slice(&rows.count.to_be_bytes());
    payload.extend_from_slice(&rows.first.to_be_bytes());
    payload.extend_from_slice(&u16::try_from(length).map_err(|_| ProtocolError::BadValue("sub-page rows"))?.to_be_bytes());
    for (subcode, title) in &rows.rows {
        payload.extend_from_slice(&checked_subcode(*subcode)?.to_be_bytes());
        text(payload, title)?;
    }
    Ok(())
}

/// Reads [`push_rows`]: at most `max_rows` rows, within the page's count,
/// each title by `text`.
pub(crate) fn read_rows(
    fields: &mut Fields<'_>,
    max_rows: usize,
    mut text: impl FnMut(&mut Fields<'_>) -> Result<String, ProtocolError>,
) -> Result<SubPageRows, ProtocolError> {
    let count = read_count(fields)?;
    let first = fields.u16()?;
    let length = fields.count_u16(max_rows, "too many sub-page rows")?;
    if usize::from(first).saturating_add(length) > usize::from(count) {
        return Err(ProtocolError::BadValue("sub-page rows"));
    }
    let mut rows = Vec::with_capacity(length);
    for _ in 0..length {
        let subcode = read_subcode(fields)?;
        rows.push((subcode, text(fields)?));
    }
    Ok(SubPageRows { count, first, rows })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    #[allow(clippy::unnecessary_wraps, reason = "the writer push_rows takes, which may refuse")]
    fn raw(payload: &mut Vec<u8>, text: &str) -> Result<(), ProtocolError> {
        crate::push_raw_text(payload, text, crate::MAX_PAGE_TITLE);
        Ok(())
    }

    fn read_raw(fields: &mut Fields<'_>) -> Result<String, ProtocolError> {
        crate::raw_text(fields, crate::MAX_PAGE_TITLE, "title too long")
    }

    /// Decision 112 item 4: a count and a place go as 16-bit values, to
    /// the standard's 8,190 sub-pages and no further.
    #[test]
    fn counts_and_places_are_sixteen_bit_to_the_standards_maximum() {
        let most = u16::try_from(MAX_SUBPAGES).unwrap();
        let step = SubPageStep { count: most, shown: 0x3F7E, place: Some(most - 1), previous: Some(0x3F7D), next: Some(0x0001) };
        let mut payload = Vec::new();
        push_step(&mut payload, &step).unwrap();
        assert_eq!(read_step(&mut Fields::new(&payload)).unwrap(), step);
        assert!(push_step(&mut Vec::new(), &SubPageStep { count: most + 1, ..step }).is_err(), "8,191 is refused");
        let mut over = payload.clone();
        over[..2].copy_from_slice(&(most + 1).to_be_bytes());
        assert!(read_step(&mut Fields::new(&over)).is_err(), "8,191 is refused when read");
        assert!(push_step(&mut Vec::new(), &SubPageStep { place: Some(most), ..step }).is_err(), "a place past the count");
    }

    /// The engine's REQ-NUM-002 and REQ-NUM-006: every value of the
    /// thirteen bits is a sub-code, 3F7F included; a value with a bit
    /// outside them is not.
    #[test]
    fn every_sub_code_of_the_thirteen_bits_and_nothing_else() {
        let mut rows = SubPageRows { count: 3, first: 0, rows: vec![(0x0000, String::new()), (0x3F7F, "Any".into()), (0x0A5C, "Hex".into())] };
        let mut payload = Vec::new();
        push_rows(&mut payload, &rows, 3, raw).unwrap();
        assert_eq!(read_rows(&mut Fields::new(&payload), 3, read_raw).unwrap(), rows);
        rows.rows[1].0 = 0x0080;
        assert!(push_rows(&mut Vec::new(), &rows, 3, raw).is_err(), "bit 7 is control bit C4, not the sub-code's");
        let mut forged = payload.clone();
        // The second row's sub-code, after the three counts and the first
        // row's sub-code and empty title.
        forged[6 + 2 + 2..6 + 2 + 2 + 2].copy_from_slice(&0x4000_u16.to_be_bytes());
        assert!(read_rows(&mut Fields::new(&forged), 3, read_raw).is_err(), "S4's byte holds C5 and C6 above two bits");
    }

    /// A run of rows stays within its budget and its page's count.
    #[test]
    fn rows_stay_within_their_budget_and_the_page() {
        let rows = SubPageRows { count: 8_190, first: 8_189, rows: vec![(1, "Last".into())] };
        let mut payload = Vec::new();
        push_rows(&mut payload, &rows, 1, raw).unwrap();
        assert_eq!(read_rows(&mut Fields::new(&payload), 1, read_raw).unwrap(), rows);
        assert!(read_rows(&mut Fields::new(&payload), 0, read_raw).is_err(), "more rows than the budget");
        let past = SubPageRows { first: 8_190, ..rows };
        assert!(push_rows(&mut Vec::new(), &past, 1, raw).is_err(), "a row past the page's last");
        assert!(push_range(&mut Vec::new(), SubPageRange { first: 0, count: 65 }, 64).is_err());
        let mut range = Vec::new();
        push_range(&mut range, SubPageRange { first: 8_189, count: 64 }, 64).unwrap();
        assert_eq!(read_range(&mut Fields::new(&range), 64).unwrap(), SubPageRange { first: 8_189, count: 64 });
    }
}
