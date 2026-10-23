//! The trusted core's messages to the interface: written by the trusted
//! core, read by the interface, one arm for each kind in the order of the
//! kinds.

use std::io::{Read, Write};

use super::ToInterface;
use super::kinds::{
    KIND_BAR_VIEW, KIND_BOOKMARKS_VIEW, KIND_CHANNEL, KIND_CLOSE, KIND_CLOSE_WINDOW_VIEW, KIND_CONSOLE_UPDATE, KIND_COOKIES_VIEW, KIND_FRAME,
    KIND_HISTORY_VIEW, KIND_OPEN_WINDOW, KIND_PAGE, KIND_PAGE_CONTROLS, KIND_REMOTE, KIND_SETTINGS, KIND_STAR_VIEW, KIND_STRIP, KIND_SUGGESTIONS,
    KIND_TAB_LIST_VIEW, KIND_TAB_SOUNDING, KIND_TABS, KIND_TARGET, KIND_UPDATE_VIEW, KIND_UPDATED, KIND_WINDOW_VIEW,
};
use super::pages::{FRAME_FIXED_BYTES, push_channel, push_frame, push_page, push_updated, read_channel, read_frame_head, read_page, read_updated};
use super::strip::{push_strip, read_strip};
use super::tabs::{push_channels, push_tabs, read_channels, read_tabs};
use super::windows::{push_open_window, read_open_window, window_number};
#[cfg(feature = "interaction")]
use crate::interaction::KIND_TYPING;
#[cfg(feature = "phone-keyboard")]
use crate::keyboard::{KIND_PHONE, KIND_PHONE_KEY};
use crate::{Fields, ProtocolError, console, controls, cookies, library, read_frame, settings, tail, write_frame};

/// Writes a message to the interface.
///
/// # Errors
/// When writing fails, or the message breaks a bound or carries text that
/// was not cleaned.
#[allow(clippy::too_many_lines, reason = "one arm for each kind of message, in the order of the kinds")]
pub fn write_to_interface(out: &mut dyn Write, message: &ToInterface) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        ToInterface::Tabs { tabs, active } => {
            push_tabs(&mut payload, tabs, *active)?;
            KIND_TABS
        }
        ToInterface::Strip(strip) => {
            push_strip(&mut payload, strip)?;
            KIND_STRIP
        }
        ToInterface::Channel(channel) => {
            push_channel(&mut payload, channel)?;
            KIND_CHANNEL
        }
        ToInterface::Page { channel, page, title } => {
            push_page(&mut payload, *channel, *page, title)?;
            KIND_PAGE
        }
        ToInterface::Updated { channel, page } => {
            push_updated(&mut payload, *channel, *page);
            KIND_UPDATED
        }
        ToInterface::Frame { key, version, image } => {
            push_frame(&mut payload, *key, *version, image);
            KIND_FRAME
        }
        ToInterface::Sounding(channels) => {
            push_channels(&mut payload, channels)?;
            KIND_TAB_SOUNDING
        }
        #[cfg(feature = "links")]
        ToInterface::Link(message) => crate::links::push_message(&mut payload, message)?,
        #[cfg(feature = "interaction")]
        ToInterface::Typing(typing) => {
            crate::interaction::push_typing(&mut payload, typing)?;
            KIND_TYPING
        }
        #[cfg(feature = "phone-keyboard")]
        ToInterface::Phone(view) => {
            crate::keyboard::push_view(&mut payload, view)?;
            KIND_PHONE
        }
        #[cfg(feature = "phone-keyboard")]
        ToInterface::PhoneKey(key) => {
            crate::keyboard::push_phone_key(&mut payload, key)?;
            KIND_PHONE_KEY
        }
        ToInterface::Settings(view) => {
            settings::push_view(&mut payload, view)?;
            KIND_SETTINGS
        }
        ToInterface::Update(view) => {
            settings::push_update(&mut payload, view)?;
            KIND_UPDATE_VIEW
        }
        ToInterface::Console(update) => {
            console::push_update(&mut payload, update)?;
            KIND_CONSOLE_UPDATE
        }
        ToInterface::History(view) => {
            library::push_history_view(&mut payload, view)?;
            KIND_HISTORY_VIEW
        }
        ToInterface::Suggestions(suggestions) => {
            library::push_suggestions(&mut payload, suggestions)?;
            KIND_SUGGESTIONS
        }
        ToInterface::TabList(list) => {
            library::push_tab_list(&mut payload, list)?;
            KIND_TAB_LIST_VIEW
        }
        ToInterface::Bookmarks(view) => {
            library::push_bookmarks_view(&mut payload, view)?;
            KIND_BOOKMARKS_VIEW
        }
        ToInterface::Star(view) => {
            library::push_star_view(&mut payload, view)?;
            KIND_STAR_VIEW
        }
        ToInterface::Bar(view) => {
            library::push_bar_view(&mut payload, view)?;
            KIND_BAR_VIEW
        }
        ToInterface::Window(window) => {
            library::push_window(&mut payload, window)?;
            KIND_WINDOW_VIEW
        }
        ToInterface::Cookies(view) => {
            cookies::push_view(&mut payload, view)?;
            KIND_COOKIES_VIEW
        }
        ToInterface::PageControls(view) => {
            controls::push_controls(&mut payload, view)?;
            KIND_PAGE_CONTROLS
        }
        ToInterface::Remote(place) => {
            settings::push_remote(&mut payload, *place)?;
            KIND_REMOTE
        }
        ToInterface::Target(window) => {
            payload.extend_from_slice(&window.to_be_bytes());
            KIND_TARGET
        }
        ToInterface::OpenWindow { window, geometry } => {
            push_open_window(&mut payload, *window, geometry.as_ref())?;
            KIND_OPEN_WINDOW
        }
        ToInterface::CloseWindow(window) => {
            payload.extend_from_slice(&window.to_be_bytes());
            KIND_CLOSE_WINDOW_VIEW
        }
        ToInterface::Close => KIND_CLOSE,
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind the trusted core may not send, and any payload
/// that is malformed or out of bounds.
pub fn read_from_core(input: &mut dyn Read) -> Result<Option<ToInterface>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_TABS => {
            let (tabs, active) = read_tabs(&mut fields)?;
            ToInterface::Tabs { tabs, active }
        }
        KIND_STRIP => ToInterface::Strip(read_strip(&mut fields)?),
        KIND_CHANNEL => ToInterface::Channel(read_channel(&mut fields)?),
        KIND_PAGE => {
            let (channel, page, title) = read_page(&mut fields)?;
            ToInterface::Page { channel, page, title }
        }
        KIND_UPDATED => {
            let (channel, page) = read_updated(&mut fields)?;
            ToInterface::Updated { channel, page }
        }
        KIND_FRAME => {
            if payload.len() < FRAME_FIXED_BYTES {
                return Err(ProtocolError::BadLength { kind, length: payload.len() });
            }
            let (key, version) = read_frame_head(&mut fields)?;
            let image = tail(payload, FRAME_FIXED_BYTES);
            return Ok(Some(ToInterface::Frame { key, version, image }));
        }
        KIND_TAB_SOUNDING => ToInterface::Sounding(read_channels(&mut fields)?),
        #[cfg(feature = "links")]
        kind if crate::links::is_message(kind) => ToInterface::Link(crate::links::read_message(kind, &mut fields)?),
        #[cfg(feature = "interaction")]
        KIND_TYPING => ToInterface::Typing(crate::interaction::read_typing(&mut fields)?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE => ToInterface::Phone(crate::keyboard::read_view(&mut fields)?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE_KEY => ToInterface::PhoneKey(crate::keyboard::read_phone_key(&mut fields)?),
        KIND_SETTINGS => ToInterface::Settings(settings::read_view(&mut fields)?),
        KIND_UPDATE_VIEW => ToInterface::Update(settings::read_update(&mut fields)?),
        KIND_CONSOLE_UPDATE => ToInterface::Console(console::read_update(&mut fields)?),
        KIND_HISTORY_VIEW => ToInterface::History(library::read_history_view(&mut fields)?),
        KIND_SUGGESTIONS => ToInterface::Suggestions(library::read_suggestions(&mut fields)?),
        KIND_TAB_LIST_VIEW => ToInterface::TabList(library::read_tab_list(&mut fields)?),
        KIND_BOOKMARKS_VIEW => ToInterface::Bookmarks(library::read_bookmarks_view(&mut fields)?),
        KIND_STAR_VIEW => ToInterface::Star(library::read_star_view(&mut fields)?),
        KIND_BAR_VIEW => ToInterface::Bar(library::read_bar_view(&mut fields)?),
        KIND_WINDOW_VIEW => ToInterface::Window(library::read_window(&mut fields)?),
        KIND_COOKIES_VIEW => ToInterface::Cookies(cookies::read_view(&mut fields)?),
        KIND_PAGE_CONTROLS => ToInterface::PageControls(controls::read_controls(&mut fields)?),
        KIND_REMOTE => ToInterface::Remote(settings::read_remote(&mut fields)?),
        KIND_TARGET => ToInterface::Target(window_number(&mut fields)?),
        KIND_OPEN_WINDOW => {
            let (window, geometry) = read_open_window(&mut fields)?;
            ToInterface::OpenWindow { window, geometry }
        }
        KIND_CLOSE_WINDOW_VIEW => ToInterface::CloseWindow(window_number(&mut fields)?),
        KIND_CLOSE => ToInterface::Close,
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}
