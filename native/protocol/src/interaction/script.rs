//! A page's program between the trusted core and the interface (the
//! script API's specification, `neoteletext-common/docs/script-api.md`,
//! sections 3.1, 4.1 and 4.6; TODO-7294): the trusted core's word on the
//! open page's program, which the trusted indications are drawn from; the
//! default action of a key a script let through; and the interface's
//! intent that the viewer pressed the browser's own key that stops a
//! page's program.
//!
//! None of these carries a page's text. A page's title, which a script
//! may set, goes as every page's title does (`ToInterface::Page`).

use interface_core::browser::scripts::{ScriptRun, ScriptView};
use interface_core::canvas::PageKey;
use neoteletext_input::event::KeyCode;

use crate::{Fields, ProtocolError, push_key};

// In numbers clear of those other work in flight on 22 November 2026 may
// take next (`interface/kinds.rs` lists every kind of the pipe): the
// trusted core's word and a key's default to the interface, and the
// interface's stop to the trusted core.
pub(crate) const KIND_SCRIPT_VIEW: u8 = 161;
pub(crate) const KIND_KEY_DEFAULT: u8 = 162;
pub(crate) const KIND_STOP_SCRIPTS: u8 = 163;

const RUN_NONE: u8 = 0;
const RUN_RUNNING: u8 = 1;
const RUN_STOPPED: u8 = 2;
/// A key's kind in [`KeyDefault`]: a character or a functional code, as
/// an intent's key is written (`intent.rs`).
const KEY_CHARACTER: u8 = super::intent::KEY_CHARACTER;
const KEY_FUNCTIONAL: u8 = 1;
/// No character was typed.
const NO_CHARACTER: u32 = u32::MAX;

/// A key the open page's program was given and did not cancel, or did
/// not answer for in time (section 4.1): the interface takes the default
/// action the key has on a page with no program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyDefault {
    /// The page the key was pressed on.
    pub key: PageKey,
    /// The key.
    pub code: KeyCode,
    /// What it typed, when that was one character.
    pub typed: Option<char>,
}

/// Only a program that runs holds anything, and one given text holds.
fn check_view(view: &ScriptView) -> Result<(), ProtocolError> {
    let holds_rightly = !view.holds || view.run == ScriptRun::Running;
    let text_rightly = !view.text || view.holds;
    if holds_rightly && text_rightly { Ok(()) } else { Err(ProtocolError::BadValue("script view")) }
}

pub(crate) fn push_view(payload: &mut Vec<u8>, view: &ScriptView) -> Result<(), ProtocolError> {
    check_view(view)?;
    push_key(payload, PageKey { channel: view.channel, page: view.page });
    let run = match view.run {
        ScriptRun::None => RUN_NONE,
        ScriptRun::Running => RUN_RUNNING,
        ScriptRun::Stopped => RUN_STOPPED,
    };
    payload.extend_from_slice(&[run, u8::from(view.holds), u8::from(view.text)]);
    Ok(())
}

pub(crate) fn read_view(fields: &mut Fields<'_>) -> Result<ScriptView, ProtocolError> {
    let key = fields.key()?;
    let run = match fields.u8()? {
        RUN_NONE => ScriptRun::None,
        RUN_RUNNING => ScriptRun::Running,
        RUN_STOPPED => ScriptRun::Stopped,
        _ => return Err(ProtocolError::BadValue("script view")),
    };
    let view = ScriptView { channel: key.channel, page: key.page, run, holds: fields.flag("script holds")?, text: fields.flag("script text")? };
    check_view(&view)?;
    Ok(view)
}

pub(crate) fn push_key_default(payload: &mut Vec<u8>, default: &KeyDefault) -> Result<(), ProtocolError> {
    push_key(payload, default.key);
    match default.code {
        KeyCode::Character(character) => {
            payload.push(KEY_CHARACTER);
            payload.extend_from_slice(&u32::from(character).to_be_bytes());
        }
        KeyCode::Functional(code) => {
            if !neoteletext_input::keys::is_code(code) {
                return Err(ProtocolError::BadValue("key"));
            }
            payload.push(KEY_FUNCTIONAL);
            payload.extend_from_slice(&code.to_be_bytes());
        }
    }
    payload.extend_from_slice(&default.typed.map_or(NO_CHARACTER, u32::from).to_be_bytes());
    Ok(())
}

pub(crate) fn read_key_default(fields: &mut Fields<'_>) -> Result<KeyDefault, ProtocolError> {
    let key = fields.key()?;
    let (kind, code) = (fields.u8()?, fields.u32()?);
    let code = match kind {
        KEY_CHARACTER => KeyCode::Character(char::from_u32(code).ok_or(ProtocolError::BadValue("key"))?),
        KEY_FUNCTIONAL if neoteletext_input::keys::is_code(code) => KeyCode::Functional(code),
        _ => return Err(ProtocolError::BadValue("key")),
    };
    let typed = match fields.u32()? {
        NO_CHARACTER => None,
        character => Some(char::from_u32(character).ok_or(ProtocolError::BadValue("typed character"))?),
    };
    Ok(KeyDefault { key, code, typed })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a test")]

    use super::*;
    use crate::{ToCore, ToInterface, read_from_core, read_from_interface, write_to_core, write_to_interface};

    const KEY: PageKey = PageKey { channel: 7, page: 0x1A0 };

    /// Sections 3.1, 4.1 and 4.6 (TODO-7294): the trusted core's word on a
    /// page's program, a key's default and the stop intent are read back
    /// as they were written.
    #[test]
    fn the_script_messages_round_trip() {
        let views = [
            ScriptView { channel: 7, page: 0x1A0, run: ScriptRun::None, holds: false, text: false },
            ScriptView { channel: 7, page: 0x1A0, run: ScriptRun::Running, holds: false, text: false },
            ScriptView { channel: 7, page: 0x1A0, run: ScriptRun::Running, holds: true, text: true },
            ScriptView { channel: 7, page: 0x1A0, run: ScriptRun::Stopped, holds: false, text: false },
        ];
        for view in views {
            let mut wire = Vec::new();
            write_to_interface(&mut wire, &ToInterface::Script(view)).unwrap();
            assert_eq!(read_from_core(&mut wire.as_slice()).unwrap(), Some(ToInterface::Script(view)));
        }
        let escape = neoteletext_input::keys::code("Escape").unwrap();
        for default in
            [KeyDefault { key: KEY, code: KeyCode::Character('/'), typed: Some('?') }, KeyDefault { key: KEY, code: KeyCode::Functional(escape), typed: None }]
        {
            let mut wire = Vec::new();
            write_to_interface(&mut wire, &ToInterface::KeyDefault(default)).unwrap();
            assert_eq!(read_from_core(&mut wire.as_slice()).unwrap(), Some(ToInterface::KeyDefault(default)));
        }
        let mut wire = Vec::new();
        write_to_core(&mut wire, &ToCore::StopScripts).unwrap();
        assert_eq!(read_from_interface(&mut wire.as_slice()).unwrap(), Some(ToCore::StopScripts));
    }

    /// Section 4.6: a program that does not run holds nothing, so a word
    /// that says a stopped program holds the keys is refused both ways,
    /// and a functional code that names no key is no key's default.
    #[test]
    fn a_script_view_that_cannot_be_is_refused() {
        for view in [
            ScriptView { channel: 7, page: 0x1A0, run: ScriptRun::Stopped, holds: true, text: false },
            ScriptView { channel: 7, page: 0x1A0, run: ScriptRun::None, holds: true, text: false },
            ScriptView { channel: 7, page: 0x1A0, run: ScriptRun::Running, holds: false, text: true },
        ] {
            assert!(write_to_interface(&mut Vec::new(), &ToInterface::Script(view)).is_err(), "{view:?}");
        }
        let mut wire = Vec::new();
        write_to_interface(&mut wire, &ToInterface::Script(ScriptView { channel: 7, page: 0x1A0, run: ScriptRun::Running, holds: true, text: false })).unwrap();
        let run = wire.len() - 3;
        wire[run] = RUN_STOPPED;
        assert!(read_from_core(&mut wire.as_slice()).is_err());
        let unnamed = KeyDefault { key: KEY, code: KeyCode::Functional(u32::MAX - 1), typed: None };
        assert!(write_to_interface(&mut Vec::new(), &ToInterface::KeyDefault(unnamed)).is_err());
    }
}
