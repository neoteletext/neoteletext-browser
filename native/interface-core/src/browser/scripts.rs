//! A page's program, as the browser shows and bounds it (the script API's
//! specification, `neoteletext-common/docs/script-api.md`, sections 3.1,
//! 4.1, 4.2 and 4.6; DEC-7282 items 4 to 6; TODO-7294): which keys a
//! script can never be given, the trusted indications that a script holds
//! the keys, runs or has stopped, the default action of a key a script
//! let through, and the browser's own key that stops a page's program.
//!
//! The trusted core decides everything here a second time before a key is
//! sent anywhere; this is what the interface does with the core's word.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use neoteletext_input::event::{ALT, CONTROL, HYPER, KeyCode, META, SUPER};

use crate::browser::Browser;
use crate::canvas::PageKey;
use crate::controls::FastextKey;
use crate::theme::{Colour, FIELD_INK, INSECURE};

/// The words of the trusted indications of a page's program, each in this
/// one place (section 4.6: "The wording is the owner's"; the owner
/// accepted the specification's recommendation on 22 November 2026,
/// TODO-7314, and changes them here after seeing them).
pub mod words {
    /// While a script's page holds any key, text or the pointer, the
    /// badge says where what the viewer types can go: these words, then
    /// the channel's host as the trusted strip names it (section 4.6,
    /// DEC-7282 item 5: "Keys go to host").
    pub const KEYS_GO_TO: &str = "Keys go to";
    /// Beside it while the script claims typed text (section 4.6: "a text
    /// claim shows a typing indication as well").
    pub const TYPING: &str = "Typing";
    /// Beside it on a plain connection, in the warning colour.
    pub const NOT_SECURE: &str = "Not secure";
    /// While a page's program runs (section 4.6: "The strip also shows
    /// that a page's script is running"), with the browser's own key that
    /// stops it, which no page can claim, on macOS.
    pub const RUNNING_COMMAND: &str = "Page program running \u{00B7} Command-. stops it";
    /// The same away from macOS, where the browser's key is Control.
    pub const RUNNING_CONTROL: &str = "Page program running \u{00B7} Control-. stops it";
    /// When a page's program has stopped, by its limits or by the viewer
    /// (section 3.1: "The trusted strip shows that the page's program
    /// stopped, with reload"), on macOS.
    pub const STOPPED_COMMAND: &str = "Page program stopped \u{00B7} Command-R reloads";
    /// The same away from macOS.
    pub const STOPPED_CONTROL: &str = "Page program stopped \u{00B7} Control-R reloads";
}

/// The modifiers that make a key the browser's own whatever a manifest
/// names (section 4.6: "a key pressed with the browser's own modifier",
/// "the browser's chords"): every shortcut of the browser is made with
/// Command on macOS (Super here) or Control elsewhere, so no key pressed
/// with Control, Super, Hyper or Meta is ever a script's. That keeps the
/// keys that reach the address field, switch and close tabs and windows,
/// reload, go back and forward, leave a page and stop a script.
pub const BROWSER_CHORD_MODIFIERS: u8 = CONTROL | SUPER | HYPER | META;

/// Alt is the browser's with a key that types nothing (Alt-Home goes
/// home; Alt-F4 and Alt-Tab are the system's); with a character key it
/// types, as Option does on macOS, so it is left to a script's page.
pub const BROWSER_CHORD_MODIFIER_OF_NAMED_KEYS: u8 = ALT;

/// The keys no script is ever given, by their W3C UI Events key values,
/// whatever a manifest names and whatever modifiers are held (section
/// 4.6: "Never claimable, whatever a manifest says"; PR RULE-51;
/// REQ-2944; REQ-6390): Back and the browser's own navigation, the
/// system's full-screen key, the keys that close or leave, and the
/// modifier keys themselves, which only make the browser's chords. A
/// manifest that names one is not refused: the name is ignored and logged.
/// Each is a key of the input protocol's table (DOC-1936); a name outside
/// that table is no key at all, and names nothing.
pub const NEVER_CLAIMABLE_KEYS: [&str; 28] = [
    // Back and the browser's own navigation.
    "BrowserBack",
    "BrowserForward",
    "BrowserHome",
    "BrowserRefresh",
    "BrowserStop",
    "BrowserSearch",
    "BrowserFavorites",
    // Closing and leaving.
    "Close",
    "Exit",
    "New",
    "Open",
    // The system's full-screen key away from macOS (decision 85).
    "F11",
    // The modifiers of the browser's chords, alone.
    "Control",
    "ControlLeft",
    "ControlRight",
    "Alt",
    "AltLeft",
    "AltRight",
    "AltGraph",
    "Meta",
    "MetaLeft",
    "MetaRight",
    "Super",
    "SuperLeft",
    "SuperRight",
    "Hyper",
    "HyperLeft",
    "HyperRight",
];

/// Whether a key named by a manifest can ever be a script's (section
/// 4.6): any name but one of [`NEVER_CLAIMABLE_KEYS`].
#[must_use]
pub fn claimable_name(name: &str) -> bool {
    !NEVER_CLAIMABLE_KEYS.contains(&name)
}

/// Whether a key event may be given to a script at all (section 4.6):
/// not one of [`NEVER_CLAIMABLE_KEYS`], and not pressed with a modifier
/// that makes it the browser's own. The interface asks this before it
/// gives the trusted core a key of a script's page, and the trusted core
/// asks it again before it gives the page's decoder anything.
#[must_use]
pub fn may_be_scripts(key: KeyCode, mods: u8) -> bool {
    if mods & BROWSER_CHORD_MODIFIERS != 0 {
        return false;
    }
    match key {
        KeyCode::Character(_) => true,
        KeyCode::Functional(code) => mods & BROWSER_CHORD_MODIFIER_OF_NAMED_KEYS == 0 && neoteletext_input::keys::name(code).is_some_and(claimable_name),
    }
}

/// What the trusted core says of a channel's scripts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptRun {
    /// The channel has no script running.
    #[default]
    None,
    /// Its scripts run.
    Running,
    /// Its scripts were stopped, by a limit or by the viewer, until the
    /// channel is reloaded.
    Stopped,
}

/// The trusted core's word on the open page's program (sections 3.1 and
/// 4.6), which the trusted indications are drawn from. Nothing in it is a
/// page's text: the page's title goes to the page list, where every
/// page's title shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptView {
    /// The channel.
    pub channel: u32,
    /// Its open page.
    pub page: u16,
    /// Whether its program runs or has stopped.
    pub run: ScriptRun,
    /// The program is given a key, typed text or the pointer.
    pub holds: bool,
    /// It is given typed text.
    pub text: bool,
}

/// What the browser keeps of the open page's program.
#[derive(Debug, Default)]
pub(super) struct ScriptUi {
    view: Option<ScriptView>,
}

/// What a key a script let through does (section 4.2's table: "a digit
/// enters a page number; a Fastext colour follows its link; Index,
/// Reveal, Hold and Mix do their function; any other key nothing").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyDefault {
    /// A digit of a page number.
    Digit(char),
    /// One of the engine viewer's keys for the page's controls, by what
    /// it types.
    Control(char),
    /// A coloured key of its own.
    Colour(FastextKey),
}

/// The engine viewer's keys for a page's controls, by what they type
/// (`page_controls.rs`): the four colours, Index, Reveal, Hold and Mix.
const CONTROL_CHARACTERS: [char; 8] = ['r', 'g', 'y', 'c', 'i', '?', 'H', 'm'];

/// The keyboard's and a remote control's own coloured keys, by name.
const COLOUR_KEYS: [(&str, FastextKey); 8] = [
    ("F1", FastextKey::Red),
    ("F2", FastextKey::Green),
    ("F3", FastextKey::Yellow),
    ("F4", FastextKey::Cyan),
    ("ColorF0Red", FastextKey::Red),
    ("ColorF1Green", FastextKey::Green),
    ("ColorF2Yellow", FastextKey::Yellow),
    ("ColorF3Blue", FastextKey::Cyan),
];

/// The default action of a key by what it typed, or for a key that types
/// nothing by its name. Every key but these has none.
fn key_default(key: KeyCode, typed: Option<char>) -> Option<KeyDefault> {
    let character = typed.or(match key {
        KeyCode::Character(character) => Some(character),
        KeyCode::Functional(_) => None,
    });
    match (character, key) {
        (Some(digit), _) if digit.is_ascii_digit() => Some(KeyDefault::Digit(digit)),
        (Some(character), _) if CONTROL_CHARACTERS.contains(&character) => Some(KeyDefault::Control(character)),
        (_, KeyCode::Functional(code)) => {
            let name = neoteletext_input::keys::name(code)?;
            COLOUR_KEYS.iter().find(|(known, _)| *known == name).map(|(_, colour)| KeyDefault::Colour(*colour))
        }
        _ => None,
    }
}

impl Browser {
    /// The trusted core's word on the open page's program. A page whose
    /// program starts to hold keys, or stops, while the page fills the
    /// screen has the strip shown as a key of the browser's own does, so
    /// that the indication is seen (section 18: "full-screen page-only
    /// mode must end or show the indication when a script starts holding
    /// keys"). Returns whether the window needs drawing.
    pub fn set_script_view(&mut self, view: ScriptView, now: u64) -> bool {
        let before = self.scripts.view.replace(view);
        if before == Some(view) {
            return false;
        }
        let was = before.filter(|old| (old.channel, old.page) == (view.channel, view.page)).unwrap_or_default();
        if (view.holds && !was.holds) || (view.run != was.run && view.run != ScriptRun::None) {
            self.full_screen_key(now);
        }
        true
    }

    /// The trusted core's word on the program of the page open now, if
    /// it said one of that page.
    fn script_view(&self) -> Option<ScriptView> {
        let opened = self.opened()?;
        self.scripts.view.filter(|view| PageKey { channel: view.channel, page: view.page } == opened && view.run != ScriptRun::None)
    }

    /// Whether the open page's program runs, so that its keys are a
    /// script's and held to what a script may be given.
    pub(super) fn script_runs(&self) -> bool {
        self.script_view().is_some_and(|view| view.run == ScriptRun::Running)
    }

    /// Whether a key that the open page asks for is kept from it all the
    /// same, because the page's keys are a script's and this one is never
    /// a script's (section 4.6).
    pub(super) fn kept_from_script(&self, key: KeyCode, mods: u8) -> bool {
        self.script_view().is_some() && !may_be_scripts(key, mods)
    }

    /// What the trusted indication says of the open page's program, in
    /// the parts the badge draws, each with its ink: none when the page
    /// has no program. Every word is the browser's own or the host as the
    /// trusted strip names it; nothing a page wrote is in it.
    pub(super) fn script_indication(&self) -> Option<(bool, Vec<(String, Colour)>)> {
        let view = self.script_view()?;
        // The frame draws its own window buttons on every platform but
        // macOS (`keys.rs`), where the browser's modifier is Command.
        let command = !self.frame.window_buttons;
        if view.run == ScriptRun::Stopped {
            let stopped = if command { words::STOPPED_COMMAND } else { words::STOPPED_CONTROL };
            return Some((true, alloc::vec![(String::from(stopped), FIELD_INK)]));
        }
        let mut parts = Vec::new();
        if view.holds {
            parts.push((format!("{} {}", words::KEYS_GO_TO, self.strip.identity), FIELD_INK));
            if view.text {
                parts.push((String::from(words::TYPING), FIELD_INK));
            }
            // For as long as the badge says keys go to the channel, it
            // says when the channel's connection is a plain one, by the
            // strip's own word on it (TODO-7407).
            if !self.strip.encrypted {
                parts.push((String::from(words::NOT_SECURE), INSECURE));
            }
        }
        parts.push((String::from(if command { words::RUNNING_COMMAND } else { words::RUNNING_CONTROL }), FIELD_INK));
        Some((false, parts))
    }

    /// The browser's own key that stops the open page's program: Command
    /// and the full stop on macOS, Control and the full stop elsewhere,
    /// which no page can claim (section 4.6; DEC-7282 item 6). Returns
    /// whether a program was there to stop.
    pub(super) fn stop_page_program(&mut self) -> bool {
        if !self.script_runs() {
            return false;
        }
        self.outputs.push(crate::browser::Output::StopScripts);
        true
    }

    /// The default action of a key the open page's program was given and
    /// did not cancel, as the trusted core says once the program has
    /// answered or has not in time (section 4.1). It is taken only while
    /// `key` is still the page the keyboard goes to, so that what the
    /// viewer pressed on a page never acts on another page, a field or a
    /// panel opened since. Returns whether the window needs drawing.
    pub fn script_key_default(&mut self, key: PageKey, code: KeyCode, typed: Option<char>, now: u64) -> bool {
        if self.opened() != Some(key) || self.editing || self.strip_has_keys() {
            return false;
        }
        match key_default(code, typed) {
            Some(KeyDefault::Digit(digit)) => {
                self.type_digit(digit, now);
                true
            }
            Some(KeyDefault::Control(character)) => {
                let mut typed = [0_u8; 4];
                self.control_key(crate::browser::Key::Text(character.encode_utf8(&mut typed))).unwrap_or(false)
            }
            Some(KeyDefault::Colour(colour)) => self.fastext_key(colour),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests;
