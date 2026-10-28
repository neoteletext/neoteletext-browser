//! A channel and its pages on the wire: the channel's banks and page
//! statuses, one page's status, a page updated, a page image's head, and
//! a click on one cell of a page.

use interface_core::canvas::{COLUMNS, PageKey, ROWS};
use interface_core::channel::{BANKS, Channel, PAGES, PageInfo, Status, page_index};
use interface_core::controls::SUBPAGE_ROWS;

use crate::subpages::SubPageRows;
use crate::{Fields, KEY_BYTES, MAX_BANK_NAME, MAX_PAGE_TITLE, MAX_TITLE, ProtocolError, push_key, push_text};

pub(super) const FRAME_FIXED_BYTES: usize = KEY_BYTES + 4;
pub(super) const CELL_BYTES: usize = KEY_BYTES + 4 + 2;

pub(super) fn push_channel(payload: &mut Vec<u8>, channel: &Channel) -> Result<(), ProtocolError> {
    if channel.banks.len() != BANKS || channel.pages.len() != PAGES {
        return Err(ProtocolError::BadValue("channel shape"));
    }
    payload.extend_from_slice(&channel.channel.to_be_bytes());
    push_text(payload, &channel.title, MAX_TITLE)?;
    for name in &channel.banks {
        push_text(payload, name, MAX_BANK_NAME)?;
    }
    let present = channel.pages.iter().flatten().count();
    payload.extend_from_slice(&u16::try_from(present).map_err(|_| ProtocolError::BadValue("pages"))?.to_be_bytes());
    for (index, page) in channel.pages.iter().enumerate() {
        let Some(page) = page else { continue };
        let number = interface_core::channel::page_number(index).ok_or(ProtocolError::BadValue("pages"))?;
        payload.extend_from_slice(&number.to_be_bytes());
        payload.push(page.status as u8);
        push_text(payload, &page.title, MAX_PAGE_TITLE)?;
    }
    Ok(())
}

pub(super) fn read_channel(fields: &mut Fields<'_>) -> Result<Channel, ProtocolError> {
    let mut channel = Channel::empty();
    channel.channel = fields.u32()?;
    channel.title = fields.text(MAX_TITLE)?;
    for name in &mut channel.banks {
        *name = fields.text(MAX_BANK_NAME)?;
    }
    let count = usize::from(fields.u16()?);
    if count > PAGES {
        return Err(ProtocolError::BadValue("too many pages"));
    }
    for _ in 0..count {
        let index = page_index(fields.u16()?).ok_or(ProtocolError::BadValue("page number"))?;
        let status = Status::from_byte(fields.u8()?).ok_or(ProtocolError::BadValue("page status"))?;
        let title = fields.text(MAX_PAGE_TITLE)?;
        let slot = channel.pages.get_mut(index).ok_or(ProtocolError::BadValue("page number"))?;
        if slot.replace(PageInfo { title, status }).is_some() {
            return Err(ProtocolError::BadValue("page listed twice"));
        }
    }
    Ok(channel)
}

/// One page's new status: its channel, number and title.
pub(super) fn push_page(payload: &mut Vec<u8>, channel: u32, page: u16, title: &str) -> Result<(), ProtocolError> {
    payload.extend_from_slice(&channel.to_be_bytes());
    payload.extend_from_slice(&page.to_be_bytes());
    push_text(payload, title, MAX_PAGE_TITLE)
}

pub(super) fn read_page(fields: &mut Fields<'_>) -> Result<(u32, u16, String), ProtocolError> {
    let channel = fields.u32()?;
    let page = fields.display_page()?;
    Ok((channel, page, fields.text(MAX_PAGE_TITLE)?))
}

/// A page with a new version: its channel and number.
pub(super) fn push_updated(payload: &mut Vec<u8>, channel: u32, page: u16) {
    payload.extend_from_slice(&channel.to_be_bytes());
    payload.extend_from_slice(&page.to_be_bytes());
}

pub(super) fn read_updated(fields: &mut Fields<'_>) -> Result<(u32, u16), ProtocolError> {
    let channel = fields.u32()?;
    let page = fields.display_page()?;
    Ok((channel, page))
}

/// A page image: its key and version, then the image's bytes as they are.
pub(super) fn push_frame(payload: &mut Vec<u8>, key: PageKey, version: u32, image: &[u8]) {
    payload.reserve_exact(FRAME_FIXED_BYTES.saturating_add(image.len()));
    push_key(payload, key);
    payload.extend_from_slice(&version.to_be_bytes());
    payload.extend_from_slice(image);
}

/// A page image's key and version; the image is the payload's tail.
pub(super) fn read_frame_head(fields: &mut Fields<'_>) -> Result<(PageKey, u32), ProtocolError> {
    let key = fields.key()?;
    let version = fields.u32()?;
    Ok((key, version))
}

/// A click on one cell of a page image.
pub(super) fn push_cell(payload: &mut Vec<u8>, key: PageKey, version: u32, column: u8, row: u8) {
    push_key(payload, key);
    payload.extend_from_slice(&version.to_be_bytes());
    payload.extend_from_slice(&[column, row]);
}

pub(super) fn read_cell(fields: &mut Fields<'_>) -> Result<(PageKey, u32, u8, u8), ProtocolError> {
    let key = fields.key()?;
    let version = fields.u32()?;
    let (column, row) = (fields.u8()?, fields.u8()?);
    if u16::from(column) >= COLUMNS || u16::from(row) >= ROWS {
        return Err(ProtocolError::BadValue("cell outside the page"));
    }
    Ok((key, version, column, row))
}

/// How many sub-pages a page has: its channel, number and count.
pub(super) fn push_subpage_count(payload: &mut Vec<u8>, channel: u32, page: u16, count: u16) -> Result<(), ProtocolError> {
    if usize::from(count) > neoteletext_core::limits::MAX_SUBPAGES {
        return Err(ProtocolError::BadValue("too many sub-pages"));
    }
    payload.extend_from_slice(&channel.to_be_bytes());
    payload.extend_from_slice(&page.to_be_bytes());
    payload.extend_from_slice(&count.to_be_bytes());
    Ok(())
}

/// Reads [`push_subpage_count`].
pub(super) fn read_subpage_count(fields: &mut Fields<'_>) -> Result<(u32, u16, u16), ProtocolError> {
    let channel = fields.u32()?;
    let page = fields.display_page()?;
    Ok((channel, page, crate::subpages::read_count(fields)?))
}

/// A run of a page's sub-pages: its channel, number and rows, each title
/// cleaned.
pub(super) fn push_subpage_rows(payload: &mut Vec<u8>, channel: u32, page: u16, rows: &SubPageRows) -> Result<(), ProtocolError> {
    page_index(page).ok_or(ProtocolError::BadValue("page number"))?;
    payload.extend_from_slice(&channel.to_be_bytes());
    payload.extend_from_slice(&page.to_be_bytes());
    crate::subpages::push_rows(payload, rows, SUBPAGE_ROWS, |payload, title| push_text(payload, title, MAX_PAGE_TITLE))
}

/// Reads [`push_subpage_rows`].
pub(super) fn read_subpage_rows(fields: &mut Fields<'_>) -> Result<(u32, u16, SubPageRows), ProtocolError> {
    let channel = fields.u32()?;
    let page = fields.display_page()?;
    let rows = crate::subpages::read_rows(fields, SUBPAGE_ROWS, |fields| fields.text(MAX_PAGE_TITLE))?;
    Ok((channel, page, rows))
}
