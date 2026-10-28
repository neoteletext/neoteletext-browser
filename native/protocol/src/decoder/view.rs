//! How the open page is drawn (`page-controls.md`, decision 67) on the
//! wire: its page, its reveal, hold and mix, and the sub-page chosen.

use interface_core::channel::page_index;

use crate::{Fields, ProtocolError};

/// A view's flags: reveal, hold, mix, and a sub-page chosen.
const VIEW_REVEAL: u8 = 1;
const VIEW_HOLD: u8 = 2;
const VIEW_MIX: u8 = 4;
const VIEW_SUBPAGE: u8 = 8;
const VIEW_FLAGS: u8 = VIEW_REVEAL | VIEW_HOLD | VIEW_MIX | VIEW_SUBPAGE;

/// How the open page is to be drawn (`page-controls.md`, decision 67):
/// concealed text shown, held, mixed, and which sub-page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The page.
    pub page: u16,
    /// Concealed text shown (the engine's REQ-L1-010).
    pub reveal: bool,
    /// Not changed by what arrives (REQ-NAV-006).
    pub hold: bool,
    /// Cell backgrounds transparent (REQ-L1-020).
    pub mix: bool,
    /// This sub-page, or none for the latest as it arrives (REQ-NAV-005).
    pub subpage: Option<u16>,
}

pub(super) fn push_view(payload: &mut Vec<u8>, view: &View) -> Result<(), ProtocolError> {
    page_index(view.page).ok_or(ProtocolError::BadValue("page number"))?;
    let flags = if view.reveal { VIEW_REVEAL } else { 0 }
        | if view.hold { VIEW_HOLD } else { 0 }
        | if view.mix { VIEW_MIX } else { 0 }
        | if view.subpage.is_some() { VIEW_SUBPAGE } else { 0 };
    payload.extend_from_slice(&view.page.to_be_bytes());
    payload.push(flags);
    if let Some(subpage) = view.subpage {
        if !names_one_subpage(subpage) {
            return Err(ProtocolError::BadValue("sub-code"));
        }
        payload.extend_from_slice(&subpage.to_be_bytes());
    }
    Ok(())
}

pub(super) fn read_view(fields: &mut Fields<'_>) -> Result<View, ProtocolError> {
    let page = fields.display_page()?;
    let flags = fields.u8()?;
    if flags & !VIEW_FLAGS != 0 {
        return Err(ProtocolError::BadValue("view flags"));
    }
    let subpage = if flags & VIEW_SUBPAGE == 0 {
        None
    } else {
        let subpage = fields.u16()?;
        if !names_one_subpage(subpage) {
            return Err(ProtocolError::BadValue("sub-code"));
        }
        Some(subpage)
    };
    Ok(View { page, reveal: flags & VIEW_REVEAL != 0, hold: flags & VIEW_HOLD != 0, mix: flags & VIEW_MIX != 0, subpage })
}

/// True for a sub-code that names one sub-page: any of the thirteen bits'
/// values but 3F7F, which means any sub-page and so names none (EN 300
/// 706 clause 9.6.1; the engine's REQ-NUM-002 and REQ-NUM-009).
pub(crate) fn names_one_subpage(subcode: u16) -> bool {
    neoteletext_core::limits::is_sub_code(subcode) && subcode != neoteletext_core::limits::SUBCODE_ANY
}
