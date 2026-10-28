//! A page's Fastext as a decoder reads it from packet X/27/0: where each
//! coloured key and Index lead, untrusted, for the trusted core to decide.

use interface_core::channel::page_index;

use crate::decoder::names_one_subpage;
use crate::{Fields, ProtocolError};

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
        if target.subcode.is_some_and(|subcode| !names_one_subpage(subcode)) {
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
    let row24 = fields.flag("Fastext row 24")?;
    let mut keys = [None; 5];
    for key in &mut keys {
        let flags = fields.u8()?;
        if flags == 0 {
            continue;
        }
        if flags & !FASTEXT_FLAGS != 0 || flags & FASTEXT_PAGE == 0 {
            return Err(ProtocolError::BadValue("Fastext flags"));
        }
        let page = fields.display_page()?;
        let subcode = if flags & FASTEXT_SUBCODE == 0 {
            None
        } else {
            let subcode = fields.u16()?;
            if !names_one_subpage(subcode) {
                return Err(ProtocolError::BadValue("Fastext sub-code"));
            }
            Some(subcode)
        };
        let service = if flags & FASTEXT_SERVICE == 0 { None } else { Some(fields.u8()?) };
        *key = Some(FastextTarget { page, subcode, service });
    }
    Ok(Fastext { keys, row24 })
}
