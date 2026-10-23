//! The interface's intents to the trusted core: written by the interface,
//! read by the trusted core, one arm for each kind in the order of the
//! kinds. Only strip input, a separate set of kinds, can navigate.

use std::io::{Read, Write};

use super::ToCore;
#[cfg(any(feature = "links", feature = "interaction"))]
use super::kinds::KIND_FOCUSED;
use super::kinds::{
    KIND_BACK, KIND_BAR, KIND_BOOKMARK, KIND_BOOKMARKS, KIND_CELL, KIND_CHANGE_SETTING, KIND_CLOSE_TAB, KIND_CLOSE_WINDOW, KIND_CONSOLE, KIND_COOKIES,
    KIND_DRAWN, KIND_FORWARD, KIND_FROM, KIND_GO_TO, KIND_HISTORY, KIND_HOME, KIND_INSTALL_UPDATE, KIND_MOVE_TAB, KIND_MUTE_ALL, KIND_MUTE_TAB, KIND_NAVIGATE,
    KIND_NEW_PRIVATE_TAB, KIND_NEW_TAB, KIND_NEW_WINDOW, KIND_OPENED, KIND_PAGE_CONTROL, KIND_RELOAD, KIND_SELECT_TAB, KIND_STAR, KIND_SUGGEST,
    KIND_SUGGESTION, KIND_TAB_LIST, KIND_TAB_TO_WINDOW, KIND_VISIBLE, KIND_WINDOW,
};
use super::pages::{CELL_BYTES, push_cell, read_cell};
use super::windows::{push_tab_to_window, push_window_number, window_intent};
#[cfg(feature = "interaction")]
use crate::interaction::{KIND_EDITING, KIND_INPUT};
#[cfg(feature = "phone-keyboard")]
use crate::keyboard::{KIND_PHONE_FINISH, KIND_PHONE_PANEL};
#[cfg(feature = "links")]
use crate::links::{KIND_CANCEL, KIND_CONFIRM, KIND_POINT, KIND_TYPED};
use crate::{Fields, MAX_ADDRESS, MAX_TABS, ProtocolError, controls, cookies, library, push_key, push_keys, push_text, read_frame, settings, write_frame};

/// Writes a message to the trusted core.
///
/// # Errors
/// When writing fails or the message breaks a bound.
#[allow(clippy::too_many_lines, reason = "one arm for each kind of message, in the order of the kinds")]
pub fn write_to_core(out: &mut dyn Write, message: &ToCore) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        ToCore::Visible(keys) => {
            push_keys(&mut payload, keys)?;
            KIND_VISIBLE
        }
        ToCore::Cell { key, version, column, row } => {
            push_cell(&mut payload, *key, *version, *column, *row);
            KIND_CELL
        }
        ToCore::SelectTab(index) => {
            payload.push(*index);
            KIND_SELECT_TAB
        }
        ToCore::CloseTab(index) => {
            payload.push(*index);
            KIND_CLOSE_TAB
        }
        ToCore::MoveTab { from, to } => {
            if usize::from(*from) >= MAX_TABS || usize::from(*to) >= MAX_TABS {
                return Err(ProtocolError::BadValue("tab place"));
            }
            payload.extend_from_slice(&[*from, *to]);
            KIND_MOVE_TAB
        }
        ToCore::NewTab => KIND_NEW_TAB,
        ToCore::Opened(key) => {
            if let Some(key) = key {
                push_key(&mut payload, *key);
            }
            KIND_OPENED
        }
        ToCore::MuteTab(index) => {
            payload.push(*index);
            KIND_MUTE_TAB
        }
        ToCore::MuteAll(muted) => {
            payload.push(u8::from(*muted));
            KIND_MUTE_ALL
        }
        ToCore::Navigate(address) => {
            push_text(&mut payload, address, MAX_ADDRESS)?;
            KIND_NAVIGATE
        }
        ToCore::Back => KIND_BACK,
        ToCore::Forward => KIND_FORWARD,
        ToCore::Bookmark => KIND_BOOKMARK,
        ToCore::Home => KIND_HOME,
        ToCore::Reload => KIND_RELOAD,
        ToCore::PageControl(control) => {
            controls::push_control(&mut payload, *control);
            KIND_PAGE_CONTROL
        }
        ToCore::From(window) => {
            push_window_number(&mut payload, *window)?;
            KIND_FROM
        }
        ToCore::NewWindow { private } => {
            payload.push(u8::from(*private));
            KIND_NEW_WINDOW
        }
        ToCore::TabToWindow { tab, window, index } => {
            push_tab_to_window(&mut payload, *tab, *window, *index)?;
            KIND_TAB_TO_WINDOW
        }
        ToCore::CloseWindow => KIND_CLOSE_WINDOW,
        ToCore::Drawn => KIND_DRAWN,
        ToCore::InstallUpdate => KIND_INSTALL_UPDATE,
        ToCore::ChangeSetting(change) => {
            settings::push_change(&mut payload, change)?;
            KIND_CHANGE_SETTING
        }
        ToCore::Console(open) => {
            payload.push(u8::from(*open));
            KIND_CONSOLE
        }
        #[cfg(feature = "links")]
        ToCore::Point(spot) => {
            crate::links::push_spot(&mut payload, spot.as_ref());
            KIND_POINT
        }
        #[cfg(feature = "links")]
        ToCore::Typed { number, opened } => {
            interface_core::channel::page_index(*number).ok_or(ProtocolError::BadValue("page number"))?;
            payload.extend_from_slice(&number.to_be_bytes());
            if let Some((key, version)) = opened {
                push_key(&mut payload, *key);
                payload.extend_from_slice(&version.to_be_bytes());
            }
            KIND_TYPED
        }
        #[cfg(any(feature = "links", feature = "interaction"))]
        ToCore::Focused(focused) => {
            payload.push(u8::from(*focused));
            KIND_FOCUSED
        }
        #[cfg(feature = "interaction")]
        ToCore::Editing(editing) => {
            payload.push(u8::from(*editing));
            KIND_EDITING
        }
        #[cfg(feature = "interaction")]
        ToCore::Input(intent) => {
            crate::interaction::push_input(&mut payload, intent)?;
            KIND_INPUT
        }
        #[cfg(feature = "phone-keyboard")]
        ToCore::PhonePanel(open) => {
            payload.push(u8::from(*open));
            KIND_PHONE_PANEL
        }
        #[cfg(feature = "phone-keyboard")]
        ToCore::PhoneFinish => KIND_PHONE_FINISH,
        #[cfg(feature = "links")]
        ToCore::Confirm(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_CONFIRM
        }
        #[cfg(feature = "links")]
        ToCore::Cancel(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_CANCEL
        }
        ToCore::History(request) => {
            library::push_history_request(&mut payload, request)?;
            KIND_HISTORY
        }
        ToCore::Bookmarks(request) => {
            library::push_bookmark_request(&mut payload, request)?;
            KIND_BOOKMARKS
        }
        ToCore::Star(request) => {
            library::push_star_request(&mut payload, request)?;
            KIND_STAR
        }
        ToCore::Bar(request) => {
            library::push_bar_request(&mut payload, request);
            KIND_BAR
        }
        ToCore::Suggest(text) => {
            push_text(&mut payload, text, MAX_ADDRESS)?;
            KIND_SUGGEST
        }
        ToCore::Suggestion { number, new_tab } => {
            payload.extend_from_slice(&number.to_be_bytes());
            payload.push(u8::from(*new_tab));
            KIND_SUGGESTION
        }
        ToCore::TabList(forward) => {
            payload.push(u8::from(*forward));
            KIND_TAB_LIST
        }
        ToCore::GoTo { forward, steps } => {
            if *steps == 0 || usize::from(*steps) > library::MAX_STEPS {
                return Err(ProtocolError::BadValue("steps"));
            }
            payload.extend_from_slice(&[u8::from(*forward), *steps]);
            KIND_GO_TO
        }
        ToCore::NewPrivateTab => KIND_NEW_PRIVATE_TAB,
        ToCore::Window(window) => {
            library::push_window(&mut payload, window)?;
            KIND_WINDOW
        }
        ToCore::Cookies(request) => {
            cookies::push_request(&mut payload, request)?;
            KIND_COOKIES
        }
    };
    write_frame(out, kind, &payload)
}

/// Reads one intent from the interface, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind the interface may not send, and any payload
/// that is malformed or out of bounds.
pub fn read_from_interface(input: &mut dyn Read) -> Result<Option<ToCore>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_VISIBLE => ToCore::Visible(fields.keys()?),
        KIND_CELL => {
            if payload.len() != CELL_BYTES {
                return Err(ProtocolError::BadLength { kind, length: payload.len() });
            }
            let (key, version, column, row) = read_cell(&mut fields)?;
            ToCore::Cell { key, version, column, row }
        }
        KIND_SELECT_TAB => ToCore::SelectTab(fields.u8()?),
        KIND_CLOSE_TAB => ToCore::CloseTab(fields.u8()?),
        KIND_MOVE_TAB => {
            let (from, to) = (fields.u8()?, fields.u8()?);
            if usize::from(from) >= MAX_TABS || usize::from(to) >= MAX_TABS {
                return Err(ProtocolError::BadValue("tab place"));
            }
            ToCore::MoveTab { from, to }
        }
        KIND_NEW_TAB => ToCore::NewTab,
        KIND_OPENED => ToCore::Opened(if payload.is_empty() { None } else { Some(fields.key()?) }),
        KIND_MUTE_TAB => ToCore::MuteTab(fields.u8()?),
        KIND_MUTE_ALL => ToCore::MuteAll(fields.flag("mute")?),
        KIND_NAVIGATE => ToCore::Navigate(fields.text(MAX_ADDRESS)?),
        KIND_BACK => ToCore::Back,
        KIND_FORWARD => ToCore::Forward,
        KIND_BOOKMARK => ToCore::Bookmark,
        KIND_HOME => ToCore::Home,
        KIND_RELOAD => ToCore::Reload,
        KIND_PAGE_CONTROL => ToCore::PageControl(controls::read_control(&mut fields)?),
        KIND_FROM | KIND_NEW_WINDOW | KIND_TAB_TO_WINDOW | KIND_CLOSE_WINDOW => window_intent(kind, &mut fields)?,
        KIND_DRAWN | KIND_INSTALL_UPDATE => update_intent(kind),
        KIND_CHANGE_SETTING => ToCore::ChangeSetting(settings::read_change(&mut fields)?),
        KIND_CONSOLE => ToCore::Console(fields.flag("flag")?),
        #[cfg(feature = "links")]
        KIND_POINT => ToCore::Point(if payload.is_empty() { None } else { Some(crate::links::read_spot(&mut fields)?) }),
        #[cfg(feature = "links")]
        KIND_TYPED => {
            let number = fields.display_page()?;
            let opened = if fields.is_empty() { None } else { Some((fields.key()?, fields.u32()?)) };
            ToCore::Typed { number, opened }
        }
        #[cfg(feature = "interaction")]
        KIND_INPUT => ToCore::Input(crate::interaction::read_input(&mut fields)?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE_PANEL => ToCore::PhonePanel(fields.flag("phone panel")?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE_FINISH => ToCore::PhoneFinish,
        #[cfg(any(feature = "links", feature = "interaction"))]
        KIND_FOCUSED => ToCore::Focused(fields.flag("focus")?),
        #[cfg(feature = "interaction")]
        KIND_EDITING => ToCore::Editing(fields.flag("editing")?),
        #[cfg(feature = "links")]
        KIND_CONFIRM => ToCore::Confirm(fields.u32()?),
        #[cfg(feature = "links")]
        KIND_CANCEL => ToCore::Cancel(fields.u32()?),
        KIND_HISTORY => ToCore::History(library::read_history_request(&mut fields)?),
        KIND_BOOKMARKS => ToCore::Bookmarks(library::read_bookmark_request(&mut fields)?),
        KIND_STAR => ToCore::Star(library::read_star_request(&mut fields)?),
        KIND_BAR => ToCore::Bar(library::read_bar_request(&mut fields)?),
        KIND_SUGGEST => ToCore::Suggest(fields.text(MAX_ADDRESS)?),
        KIND_SUGGESTION => ToCore::Suggestion { number: fields.u32()?, new_tab: fields.flag("flag")? },
        KIND_TAB_LIST => ToCore::TabList(fields.flag("flag")?),
        KIND_GO_TO => {
            let forward = fields.flag("flag")?;
            ToCore::GoTo { forward, steps: library::read_steps(&mut fields)? }
        }
        KIND_NEW_PRIVATE_TAB => ToCore::NewPrivateTab,
        KIND_WINDOW => ToCore::Window(library::read_window(&mut fields)?),
        KIND_COOKIES => ToCore::Cookies(cookies::read_request(&mut fields)?),
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// One of the intents of updates (`installation-and-updates.md` section
/// 3.2), which carry nothing.
fn update_intent(kind: u8) -> ToCore {
    if kind == KIND_DRAWN { ToCore::Drawn } else { ToCore::InstallUpdate }
}
