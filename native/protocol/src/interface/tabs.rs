//! The window's tabs on the wire, and the list of channels sounding, which
//! the sound helper's pipe carries in the same form.

use interface_core::browser::TabInfo;
use interface_core::theme::TAB_COLOURS;

use super::kinds::{TAB_FLAGS, TAB_MUTED, TAB_PRIVATE};
use crate::{Fields, MAX_TAB_TITLE, MAX_TABS, ProtocolError, push_text};

pub(super) fn push_tabs(payload: &mut Vec<u8>, tabs: &[TabInfo], active: usize) -> Result<(), ProtocolError> {
    if tabs.len() > MAX_TABS || (active >= tabs.len() && !tabs.is_empty()) {
        return Err(ProtocolError::BadValue("tabs"));
    }
    payload.push(u8::try_from(active).map_err(|_| ProtocolError::BadValue("tabs"))?);
    payload.push(u8::try_from(tabs.len()).map_err(|_| ProtocolError::BadValue("tabs"))?);
    for tab in tabs {
        payload.push(tab.colour);
        payload.extend_from_slice(&tab.channel.to_be_bytes());
        payload.push(if tab.muted { TAB_MUTED } else { 0 } | if tab.private { TAB_PRIVATE } else { 0 });
        push_text(payload, &tab.title, MAX_TAB_TITLE)?;
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
            Ok(TabInfo { colour, channel, muted: flags & TAB_MUTED != 0, private: flags & TAB_PRIVATE != 0, title: fields.text(MAX_TAB_TITLE)? })
        })
        .collect::<Result<_, _>>()?;
    Ok((tabs, active))
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
