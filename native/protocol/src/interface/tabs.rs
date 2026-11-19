//! The window's tabs on the wire, and the list of channels sounding, which
//! the sound helper's pipe carries in the same form.

use interface_core::browser::TabInfo;
use interface_core::theme::TAB_COLOURS;

use super::kinds::{TAB_COSTLY, TAB_FLAGS, TAB_MUTED, TAB_PRIVATE};
use crate::library::MAX_SHOWN_ADDRESS;
use crate::{Fields, MAX_PAYLOAD, MAX_TAB_TITLE, MAX_TABS, MAX_TITLE, ProtocolError, push_text};
use neoteletext_core::limits::SUBCODE_ANY;

/// A tab's page on the wire when its address names none: no display page
/// is 0.
const NO_PAGE: u16 = 0;
/// A tab's sub-page on the wire when its address names none: 3F7F names
/// no one sub-page (EN 300 706 clause 9.6.1).
const NO_SUBPAGE: u16 = SUBCODE_ANY;

/// A tab on the wire at most: its colour, channel and flags, its title and
/// address, each with its two-byte length, its page and sub-page, and its
/// page's title with its length (REQ-6887 item 1). Its address is only
/// what its tip shows, cut as a view's is, so that every tab fits one
/// message.
const MAX_TAB_BYTES: usize = 1 + 4 + 1 + (2 + MAX_TAB_TITLE) + (2 + MAX_SHOWN_ADDRESS) + 2 + 2 + (2 + MAX_TITLE);
const _: () = assert!(2 + MAX_TABS * MAX_TAB_BYTES <= MAX_PAYLOAD, "every tab fits one message");

pub(super) fn push_tabs(payload: &mut Vec<u8>, tabs: &[TabInfo], active: usize) -> Result<(), ProtocolError> {
    if tabs.len() > MAX_TABS || (active >= tabs.len() && !tabs.is_empty()) {
        return Err(ProtocolError::BadValue("tabs"));
    }
    payload.push(u8::try_from(active).map_err(|_| ProtocolError::BadValue("tabs"))?);
    payload.push(u8::try_from(tabs.len()).map_err(|_| ProtocolError::BadValue("tabs"))?);
    for tab in tabs {
        payload.push(tab.colour);
        payload.extend_from_slice(&tab.channel.to_be_bytes());
        payload.push(if tab.muted { TAB_MUTED } else { 0 } | if tab.private { TAB_PRIVATE } else { 0 } | if tab.costly { TAB_COSTLY } else { 0 });
        push_text(payload, &tab.title, MAX_TAB_TITLE)?;
        push_text(payload, &tab.address, MAX_SHOWN_ADDRESS)?;
        push_place(payload, tab.page, tab.subpage)?;
        push_text(payload, &tab.page_title, MAX_TITLE)?;
    }
    Ok(())
}

/// The tabs and the index of the active one.
pub(super) fn read_tabs(fields: &mut Fields<'_>) -> Result<(Vec<TabInfo>, usize), ProtocolError> {
    let active = usize::from(fields.u8()?);
    let count = usize::from(fields.u8()?);
    if count > MAX_TABS || (active >= count && count > 0) {
        return Err(ProtocolError::BadValue("tabs"));
    }
    let tabs = (0..count)
        .map(|_| {
            let colour = fields.u8()?;
            if usize::from(colour) >= TAB_COLOURS.len() {
                return Err(ProtocolError::BadValue("tab colour"));
            }
            let channel = fields.u32()?;
            let flags = fields.u8()?;
            if flags & !TAB_FLAGS != 0 {
                return Err(ProtocolError::BadValue("tab flags"));
            }
            let title = fields.text(MAX_TAB_TITLE)?;
            let address = fields.text(MAX_SHOWN_ADDRESS)?;
            let (page, subpage) = read_place(fields)?;
            Ok(TabInfo {
                colour,
                channel,
                muted: flags & TAB_MUTED != 0,
                private: flags & TAB_PRIVATE != 0,
                costly: flags & TAB_COSTLY != 0,
                title,
                address,
                page,
                subpage,
                page_title: fields.text(MAX_TITLE)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok((tabs, active))
}

/// A tab's page and sub-page: a display page or none, and a sub-page only
/// with a page, naming one sub-page.
fn push_place(payload: &mut Vec<u8>, page: Option<u16>, subpage: Option<u16>) -> Result<(), ProtocolError> {
    check_place(page, subpage)?;
    payload.extend_from_slice(&page.unwrap_or(NO_PAGE).to_be_bytes());
    payload.extend_from_slice(&subpage.unwrap_or(NO_SUBPAGE).to_be_bytes());
    Ok(())
}

fn read_place(fields: &mut Fields<'_>) -> Result<(Option<u16>, Option<u16>), ProtocolError> {
    let page = match fields.u16()? {
        NO_PAGE => None,
        page => Some(page),
    };
    let subpage = match fields.u16()? {
        NO_SUBPAGE => None,
        subpage => Some(subpage),
    };
    check_place(page, subpage)?;
    Ok((page, subpage))
}

fn check_place(page: Option<u16>, subpage: Option<u16>) -> Result<(), ProtocolError> {
    if let Some(page) = page {
        crate::fields::check_page(page)?;
    }
    match subpage {
        None => Ok(()),
        Some(subpage) if page.is_some() && crate::decoder::names_one_subpage(subpage) => Ok(()),
        Some(_) => Err(ProtocolError::BadValue("tab sub-page")),
    }
}

/// The channels sounding: at most one a tab, four bytes each, to the
/// payload's end.
pub(crate) fn push_channels(payload: &mut Vec<u8>, channels: &[u32]) -> Result<(), ProtocolError> {
    if channels.len() > MAX_TABS {
        return Err(ProtocolError::BadValue("too many channels"));
    }
    for channel in channels {
        payload.extend_from_slice(&channel.to_be_bytes());
    }
    Ok(())
}

pub(crate) fn read_channels(fields: &mut Fields<'_>) -> Result<Vec<u32>, ProtocolError> {
    let mut channels = Vec::new();
    while !fields.is_empty() {
        if channels.len() == MAX_TABS {
            return Err(ProtocolError::BadValue("too many channels"));
        }
        channels.push(fields.u32()?);
    }
    Ok(channels)
}
