//! A page's program in the browser (`script-api.md` sections 3.1, 4.1,
//! 4.2 and 4.6; DEC-7282 items 4 to 6; TODO-7294): the keys no script is
//! ever given, the trusted indications, a key's default action and the
//! browser's own key that stops a page's program.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]

use alloc::string::String;
use alloc::vec::Vec;

use neoteletext_input::event::{ALT, Action, CAPS_LOCK, CONTROL, HYPER, KeyCode, META, NUM_LOCK, SHIFT, SUPER};
use neoteletext_input::keys::code;

use super::*;
use crate::browser::{AskedKey, AskedKeys, Frame, Key, KeyNamed, Modifiers, Output, TypingView};
use crate::channel::{Channel, PageInfo, Status};
use crate::controls::{PageControl, PageControls};
use crate::geometry::Rect;
use crate::test_canvas::Recorder;

const KEY: PageKey = PageKey { channel: 4, page: 0x100 };
const NOW: u64 = 10_000;

/// A browser with page 100 of a channel open, its controls told, on the
/// platform with (`mac` false) or without the frame's own window buttons.
fn browser_on(mac: bool) -> Browser {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: !mac });
    let mut channel = Channel::empty();
    channel.channel = KEY.channel;
    channel.pages[0] = Some(PageInfo { title: "Game".into(), status: Status::Loaded, subpages: 0 });
    browser.set_channel(channel);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.wall.zoom(KEY, 0);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, NOW);
    browser.set_page_controls(PageControls { channel: KEY.channel, page: Some(KEY.page), subpages: 1, fastext: [true; 5], ..PageControls::default() });
    browser.take_outputs();
    assert_eq!(browser.opened(), Some(KEY));
    browser
}

fn browser() -> Browser {
    browser_on(false)
}

/// The trusted core's typing state for a page whose keys are the page's.
fn typing() -> TypingView {
    TypingView {
        channel: KEY.channel,
        page: KEY.page,
        on: true,
        secure: true,
        refused: false,
        cursor: None,
        round_trip_us: None,
        wheel: false,
        asks: false,
        keys: true,
    }
}

fn view(run: ScriptRun, holds: bool, text: bool) -> ScriptView {
    ScriptView { channel: KEY.channel, page: KEY.page, run, holds, text }
}

/// A browser whose open page's program runs and is given every key the
/// trusted core could name and typed text: the most a page could ask.
fn with_program() -> Browser {
    let mut browser = browser();
    browser.set_script_view(view(ScriptRun::Running, true, true), NOW);
    browser.set_typing(typing(), NOW);
    browser.set_asked_keys(AskedKeys::everything());
    browser
}

fn named(name: &str) -> KeyCode {
    KeyCode::Functional(code(name).unwrap())
}

/// What the badge says, as one string.
fn badge(browser: &Browser) -> String {
    let mut canvas = Recorder::new();
    browser.draw_typing_badge(&mut canvas, Rect::new(0.0, 0.0, 1200.0, 40.0));
    canvas.texts.iter().map(|(_, text)| text.as_str()).collect()
}

/// Section 4.6, "Never claimable, whatever a manifest says": the exact
/// set of keys no script is ever given, each a key of the engine's table
/// (so that naming it could otherwise have claimed it), and the exact
/// modifiers that make any key the browser's own.
#[test]
fn the_keys_no_script_is_given_are_exactly_these() {
    let mut never: Vec<&str> = NEVER_CLAIMABLE_KEYS.to_vec();
    never.sort_unstable();
    let expected = [
        "Alt",
        "AltGraph",
        "AltLeft",
        "AltRight",
        "BrowserBack",
        "BrowserFavorites",
        "BrowserForward",
        "BrowserHome",
        "BrowserRefresh",
        "BrowserSearch",
        "BrowserStop",
        "Close",
        "Control",
        "ControlLeft",
        "ControlRight",
        "Exit",
        "F11",
        "Hyper",
        "HyperLeft",
        "HyperRight",
        "Meta",
        "MetaLeft",
        "MetaRight",
        "New",
        "Open",
        "Super",
        "SuperLeft",
        "SuperRight",
    ];
    assert_eq!(never, expected);
    for name in NEVER_CLAIMABLE_KEYS {
        assert!(code(name).is_some(), "{name} is a key of the input protocol's table");
        assert!(!claimable_name(name));
        // Whatever the modifiers, and with none.
        for mods in [0, SHIFT, ALT, CONTROL, SUPER, CAPS_LOCK | NUM_LOCK] {
            assert!(!may_be_scripts(named(name), mods), "{name} with {mods}");
        }
    }
    assert_eq!(BROWSER_CHORD_MODIFIERS, CONTROL | SUPER | HYPER | META);
    assert_eq!(BROWSER_CHORD_MODIFIER_OF_NAMED_KEYS, ALT);
}

/// Section 4.6: a key pressed with a modifier of the browser's chords is
/// never a script's, so the keys that reach the address field (Command or
/// Control and L), switch tabs (and a digit, or with Alt an arrow), close
/// (and W), reload (and R), go back (and `[`), leave a page (and Escape)
/// and stop a script (and the full stop) cannot be claimed; every other
/// key can be, Escape and the digits among them, by name.
#[test]
fn a_chord_of_the_browsers_is_never_a_scripts() {
    let chords = [('l', "the address field"), ('1', "a tab"), ('w', "closing"), ('r', "reload"), ('[', "back"), ('.', "stopping a script"), ('t', "a new tab")];
    for modifier in [CONTROL, SUPER, HYPER, META, CONTROL | SHIFT, SUPER | ALT, CONTROL | ALT] {
        for (character, what) in chords {
            assert!(!may_be_scripts(KeyCode::Character(character), modifier), "{what}: {character} with {modifier}");
        }
        for name in ["Escape", "ArrowLeft", "ArrowRight", "Home", "Enter", "F1"] {
            assert!(!may_be_scripts(named(name), modifier), "{name} with {modifier}");
        }
    }
    // Alt with a key that types nothing is the browser's or the system's:
    // Alt-Home goes home, Alt and an arrow go back and forward elsewhere.
    for name in ["Home", "ArrowLeft", "F4", "Tab"] {
        assert!(!may_be_scripts(named(name), ALT), "Alt-{name}");
    }
    // What a script may be given, by name.
    for mods in [0, SHIFT, CAPS_LOCK, NUM_LOCK] {
        assert!(may_be_scripts(KeyCode::Character('1'), mods) && may_be_scripts(named("Escape"), mods) && may_be_scripts(named("ArrowLeft"), mods));
    }
    assert!(may_be_scripts(KeyCode::Character('e'), ALT), "Option and a letter types");
}

/// Section 4.6: on a script's page the interface gives the page no key a
/// script is never given, though the page asks for every key and for
/// text: the chord stays the browser's and acts. On a page with no
/// program (a live channel's, REQ-7037) nothing changes.
#[test]
fn a_script_pages_keys_are_held_to_what_a_script_may_be_given() {
    let mut browser = with_program();
    let target = |browser: &Browser, key: KeyCode, mods: u8, text: Option<&str>| browser.key_target(Action::Press, key, mods, text, None);
    assert_eq!(target(&browser, KeyCode::Character('l'), 0, Some("l")), Some(KEY), "a letter is the page's");
    assert_eq!(target(&browser, named("Escape"), 0, None), Some(KEY), "Escape by name");
    assert_eq!(target(&browser, KeyCode::Character('l'), CONTROL, Some("\u{c}")), None, "Control-L is the address field's");
    assert_eq!(target(&browser, KeyCode::Character('l'), SUPER, Some("l")), None, "Command-L, though it types");
    assert_eq!(target(&browser, KeyCode::Character('.'), CONTROL, Some(".")), None, "the key that stops it");
    assert_eq!(target(&browser, named("F11"), 0, None), None);
    assert_eq!(target(&browser, named("BrowserBack"), 0, None), None);
    assert_eq!(target(&browser, named("ArrowLeft"), CONTROL | ALT, None), None, "the tab before");
    // The chord acts as the browser's own: Control-L starts the address.
    assert!(browser.key(Key::Text("l"), Modifiers { command: true, control: true, ..Modifiers::default() }, NOW));
    assert!(browser.editing, "the address field has the keys");
    // A page with no program, whose table asks for every key: as before.
    let mut live = browser_on(false);
    live.set_typing(typing(), NOW);
    live.set_asked_keys(AskedKeys::everything());
    assert_eq!(target(&live, KeyCode::Character('l'), CONTROL, Some("\u{c}")), Some(KEY), "a terminal's Control-L is its host's");
    assert_eq!(target(&live, named("F11"), 0, None), Some(KEY));
}

/// Sections 3.1 and 4.6 (DEC-7282 items 5 and 6): what the trusted badge
/// says in each state of a page's program, in the words kept in one
/// place, with the host as the trusted strip names it and nothing a page
/// wrote.
#[test]
fn the_badge_says_what_a_pages_program_does() {
    let mut browser = browser();
    assert_eq!(badge(&browser), "", "no program, nothing said");
    // It runs and is given nothing.
    browser.set_script_view(view(ScriptRun::Running, false, false), NOW);
    assert_eq!(badge(&browser), words::RUNNING_CONTROL);
    // It holds keys or the pointer.
    browser.set_script_view(view(ScriptRun::Running, true, false), NOW);
    browser.set_typing(typing(), NOW);
    let said = badge(&browser);
    let identity = browser.strip.identity.clone();
    assert!(said.starts_with(&alloc::format!("{} {identity}", words::KEYS_GO_TO)) && said.ends_with(words::RUNNING_CONTROL), "{said}");
    // The fixture's channel is on a plain connection: said for as long
    // as the badge says keys go to it, whoever has the keys just then
    // (TODO-7407).
    assert!(!said.contains(words::TYPING) && said.contains(words::NOT_SECURE), "{said}");
    browser.start_editing();
    assert_eq!(badge(&browser), said, "the address field having the keys changes nothing of it");
    browser.stop_editing();
    browser.set_typing(TypingView { on: false, keys: false, ..typing() }, NOW);
    assert_eq!(badge(&browser), said, "nor does the typing state");
    browser.set_typing(typing(), NOW);
    // It is given typed text.
    browser.set_script_view(view(ScriptRun::Running, true, true), NOW);
    assert!(badge(&browser).contains(words::TYPING), "{}", badge(&browser));
    // Over an encrypted connection.
    browser.strip.encrypted = true;
    assert!(!badge(&browser).contains(words::NOT_SECURE), "{}", badge(&browser));
    browser.strip.encrypted = false;
    // It stopped: nothing is the page's any more.
    browser.set_script_view(view(ScriptRun::Stopped, false, false), NOW);
    browser.set_typing(TypingView { on: false, keys: false, ..typing() }, NOW);
    assert_eq!(badge(&browser), words::STOPPED_CONTROL);
    // The word on another page says nothing of this one.
    browser.set_script_view(ScriptView { page: 0x101, ..view(ScriptRun::Running, true, false) }, NOW);
    assert_eq!(badge(&browser), "");
    // On macOS the browser's key is Command.
    let mut mac = browser_on(true);
    mac.set_script_view(view(ScriptRun::Running, false, false), NOW);
    assert_eq!(badge(&mac), words::RUNNING_COMMAND);
    mac.set_script_view(view(ScriptRun::Stopped, false, false), NOW);
    assert_eq!(badge(&mac), words::STOPPED_COMMAND);
}

/// Section 4.2's table: the default action of a key a script let through
/// is what the key does on a page with no program, for a digit, a
/// Fastext colour, Index, Reveal, Hold and Mix, and nothing for any
/// other key.
#[test]
fn a_key_a_script_let_through_takes_its_default() {
    let controls = |browser: &mut Browser| -> Vec<PageControl> {
        browser.take_outputs().into_iter().filter_map(|output| if let Output::PageControl(control) = output { Some(control) } else { None }).collect()
    };
    let mut browser = with_program();
    for (key, typed, control) in [
        (KeyCode::Character('r'), Some('r'), PageControl::Fastext(FastextKey::Red)),
        (KeyCode::Character('g'), Some('g'), PageControl::Fastext(FastextKey::Green)),
        (KeyCode::Character('y'), Some('y'), PageControl::Fastext(FastextKey::Yellow)),
        (KeyCode::Character('c'), Some('c'), PageControl::Fastext(FastextKey::Cyan)),
        (KeyCode::Character('i'), Some('i'), PageControl::Fastext(FastextKey::Index)),
        (KeyCode::Character('/'), Some('?'), PageControl::Reveal),
        (KeyCode::Character('h'), Some('H'), PageControl::Hold),
        (KeyCode::Character('m'), Some('m'), PageControl::Mix),
        (named("F1"), None, PageControl::Fastext(FastextKey::Red)),
        (named("F4"), None, PageControl::Fastext(FastextKey::Cyan)),
        (named("ColorF1Green"), None, PageControl::Fastext(FastextKey::Green)),
    ] {
        browser.script_key_default(KEY, key, typed, NOW);
        assert_eq!(controls(&mut browser), [control], "{key:?} typing {typed:?}");
    }
    // A digit enters a page number.
    assert_eq!(browser.digits.as_str(), "");
    browser.script_key_default(KEY, KeyCode::Character('2'), Some('2'), NOW);
    assert_eq!(browser.digits.as_str(), "2");
    // Any other key: nothing, and never the browser's other keys for it.
    for (key, typed) in [
        (KeyCode::Character('x'), Some('x')),
        (KeyCode::Character('h'), Some('h')),
        (named("Escape"), None),
        (named("ArrowLeft"), None),
        (named("ArrowDown"), None),
        (named("Enter"), None),
        (named("Tab"), None),
        (named("PageDown"), None),
    ] {
        assert!(!browser.script_key_default(KEY, key, typed, NOW), "{key:?}");
        assert!(browser.take_outputs().is_empty(), "{key:?}");
    }
    assert_eq!((browser.opened(), browser.digits.as_str()), (Some(KEY), "2"), "the page is still open and nothing more was typed");
}

/// Section 4.1: a key's default is the key's on the page it was pressed
/// on. Once another part of the browser has the keys, or another page is
/// open, it acts on nothing: a digit pressed on a page never types into
/// the address field.
#[test]
fn a_keys_default_never_acts_off_its_page() {
    let mut browser = with_program();
    assert!(!browser.script_key_default(PageKey { channel: KEY.channel, page: 0x101 }, KeyCode::Character('r'), Some('r'), NOW), "another page");
    browser.start_editing();
    assert!(browser.editing);
    assert!(!browser.script_key_default(KEY, KeyCode::Character('2'), Some('2'), NOW));
    assert!(!browser.script_key_default(KEY, KeyCode::Character('r'), Some('r'), NOW));
    assert_eq!(browser.digits.as_str(), "", "no digit of a page number");
    assert!(!browser.take_outputs().iter().any(|output| matches!(output, Output::PageControl(_))));
}

/// Section 4.6 (DEC-7282 item 6): the browser's own key stops a running
/// program, Command or Control and the full stop, which no page can
/// claim; with no program running it is no key of the browser's.
#[test]
fn the_browsers_own_key_stops_a_pages_program() {
    let stop = Modifiers { command: true, ..Modifiers::default() };
    let stops = |browser: &mut Browser| browser.take_outputs().iter().filter(|output| matches!(output, Output::StopScripts)).count();
    let mut browser = with_program();
    // The page is given every key it could be, and still not this one.
    assert_eq!(browser.key_target(Action::Press, KeyCode::Character('.'), CONTROL, Some("."), None), None);
    assert_eq!(browser.key_target(Action::Press, KeyCode::Character('.'), SUPER, Some("."), None), None);
    assert!(browser.key(Key::Text("."), stop, NOW));
    assert_eq!(stops(&mut browser), 1);
    // A program that claims nothing is stopped the same way.
    let mut quiet = browser_on(false);
    quiet.set_script_view(view(ScriptRun::Running, false, false), NOW);
    assert!(quiet.key(Key::Text("."), stop, NOW));
    assert_eq!(stops(&mut quiet), 1);
    // Stopped already, or no program: nothing to stop.
    quiet.set_script_view(view(ScriptRun::Stopped, false, false), NOW);
    assert!(!quiet.key(Key::Text("."), stop, NOW));
    let mut plain = browser_on(false);
    assert!(!plain.key(Key::Text("."), stop, NOW));
    assert_eq!(stops(&mut quiet) + stops(&mut plain), 0);
}

/// Section 4.6: Escape is a script's only by name, and while it is the
/// full-screen notice says to hold it (REQ-7037 item 7); holding it is
/// the browser's.
#[test]
fn escape_is_a_scripts_only_when_its_manifest_names_it() {
    let escape = AskedKey { key: KeyNamed::Named(code("Escape").unwrap()), mods_mask: BROWSER_CHORD_MODIFIERS | ALT, mods_value: 0 };
    let mut browser = with_program();
    browser.set_asked_keys(AskedKeys { text: true, keys: alloc::vec![AskedKey { key: KeyNamed::Character('a'), ..escape }] });
    assert!(!browser.escape_is_pages(), "a manifest that names other keys and text leaves Escape the browser's");
    browser.set_asked_keys(AskedKeys { text: false, keys: alloc::vec![escape] });
    assert!(browser.escape_is_pages());
    assert_eq!(browser.key_target(Action::Press, named("Escape"), SUPER, None, None), None, "Command-Escape still leaves the page");
}

/// Section 18 (TODO-7285 item 6): a page that fills the screen has the
/// strip, with its indication, shown when its program starts to hold the
/// keys and when it stops, as a key of the browser's own shows it.
#[test]
fn full_screen_shows_the_strip_when_a_program_takes_the_keys_or_stops() {
    let mut browser = browser();
    assert!(browser.toggle_full_screen(NOW));
    let later = NOW + 60_000;
    assert!(!browser.full_screen_strip(later), "the page alone");
    browser.set_script_view(view(ScriptRun::Running, false, false), later);
    assert!(browser.full_screen_strip(later + 1), "a program started");
    let later = later + 60_000;
    assert!(!browser.full_screen_strip(later));
    browser.set_script_view(view(ScriptRun::Running, true, false), later);
    assert!(browser.full_screen_strip(later + 1), "it holds the keys");
    let later = later + 60_000;
    browser.set_script_view(view(ScriptRun::Stopped, false, false), later);
    assert!(browser.full_screen_strip(later + 1), "it stopped");
    // The same word again changes nothing.
    let later = later + 60_000;
    assert!(!browser.set_script_view(view(ScriptRun::Stopped, false, false), later));
    assert!(!browser.full_screen_strip(later + 1));
}
