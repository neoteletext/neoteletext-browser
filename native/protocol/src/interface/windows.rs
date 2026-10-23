//! Windows on the wire (`multiple-windows.md`, decision 72): the number a
//! message is for, a window opened or closed, and the intents that open,
//! close and move tabs between windows.

use interface_core::library::WindowGeometry;

use super::ToCore;
use super::kinds::{KIND_CLOSE_WINDOW, KIND_FROM, KIND_NEW_WINDOW, KIND_TAB_TO_WINDOW};
use crate::library::{push_window, read_window};
use crate::{Fields, MAX_TABS, NEW_WINDOW, ProtocolError};

/// A window's number, never [`NEW_WINDOW`].
pub(super) fn window_number(fields: &mut Fields<'_>) -> Result<u32, ProtocolError> {
    let window = fields.u32()?;
    if window == NEW_WINDOW { Err(ProtocolError::BadValue("window number")) } else { Ok(window) }
}

/// A window's number, refused when it is [`NEW_WINDOW`].
pub(super) fn push_window_number(payload: &mut Vec<u8>, window: u32) -> Result<(), ProtocolError> {
    if window == NEW_WINDOW {
        return Err(ProtocolError::BadValue("window number"));
    }
    payload.extend_from_slice(&window.to_be_bytes());
    Ok(())
}

/// A window to open: its number and, when kept, its place and size.
pub(super) fn push_open_window(payload: &mut Vec<u8>, window: u32, geometry: Option<&WindowGeometry>) -> Result<(), ProtocolError> {
    push_window_number(payload, window)?;
    if let Some(geometry) = geometry {
        push_window(payload, geometry)?;
    }
    Ok(())
}

pub(super) fn read_open_window(fields: &mut Fields<'_>) -> Result<(u32, Option<WindowGeometry>), ProtocolError> {
    let window = window_number(fields)?;
    let geometry = if fields.is_empty() { None } else { Some(read_window(fields)?) };
    Ok((window, geometry))
}

/// A tab moved into another window: its place, the window, its place there.
pub(super) fn push_tab_to_window(payload: &mut Vec<u8>, tab: u8, window: u32, index: u8) -> Result<(), ProtocolError> {
    if usize::from(tab) >= MAX_TABS || usize::from(index) > MAX_TABS {
        return Err(ProtocolError::BadValue("tab place"));
    }
    payload.push(tab);
    payload.extend_from_slice(&window.to_be_bytes());
    payload.push(index);
    Ok(())
}

/// One of the window intents (`multiple-windows.md`).
pub(super) fn window_intent(kind: u8, fields: &mut Fields<'_>) -> Result<ToCore, ProtocolError> {
    Ok(match kind {
        KIND_FROM => ToCore::From(window_number(fields)?),
        KIND_NEW_WINDOW => ToCore::NewWindow { private: fields.flag("flag")? },
        KIND_TAB_TO_WINDOW => {
            let tab = fields.u8()?;
            let window = fields.u32()?;
            let index = fields.u8()?;
            if usize::from(tab) >= MAX_TABS || usize::from(index) > MAX_TABS {
                return Err(ProtocolError::BadValue("tab place"));
            }
            ToCore::TabToWindow { tab, window, index }
        }
        KIND_CLOSE_WINDOW => ToCore::CloseWindow,
        other => return Err(ProtocolError::UnknownKind(other)),
    })
}
