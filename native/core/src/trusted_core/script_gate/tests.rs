//! The trusted core's gate between the viewer and a page's program
//! (`script-api.md` sections 3.1, 3.3, 3.5, 4.1, 4.2, 4.4 and 4.6;
//! DEC-7282 items 4 to 7; TODO-7294's checks): what a script is given and
//! what it is never given, a key's default action and its deadline, a
//! script's navigation, its title, the viewer's stop, and what the
//! interface is told for the trusted indications. The decoder here is a
//! stand-in whose reports the test writes, as a hostile one could.

#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::indexing_slicing, clippy::panic, reason = "a test")]

use std::sync::mpsc;

use confine::broker::stand_in::StandIn;
use interface_core::browser::scripts::BROWSER_CHORD_MODIFIERS;
use neoteletext_input::event::{ALT, CONTROL, SHIFT, SUPER};
use neoteletext_input::keys::code;
use protocol::ToCore;
use protocol::decoder::script::ScriptReport;
use protocol::rules::{Key, KeyAsk, KeyKind};
use protocol::script::relay::{DecoderInput, DecoderOutput, read_decoder_input, read_decoder_output};
use protocol::script::{FromScriptHost, ToScriptHost, write_to_host};

use super::super::test_support::{core_watched, decoder_watched, sent_to_interface};
use std::time::Duration;

#[cfg(feature = "links")]
use neoteletext::script_gate::ACTIVATION_WINDOW;
use neoteletext::script_gate::{MAX_CONTACTS, MOUSE_POINTER};
use neoteletext_input::event::{Action, KeyCode, Point};
use protocol::decoder::ToDecoder;
use protocol::decoder::script::{MAX_POINTERS, PointerKind, ScriptOrder, ScriptPointer};
use protocol::interaction::TouchPhase;
use protocol::rules::{KeyAction, PointerAction, RuleInput};
use protocol::script::relay::STEP_DEADLINE;

use super::*;

/// The channel the tests' tab is on, and another.
const CHANNEL: &str = "https://example.test/chan/";
#[cfg(feature = "links")]
const ELSEWHERE: &str = "https://elsewhere.test/news/";
/// The page open in it.
const PAGE: u16 = 0x100;
/// The generation `decoder_watched` gives its decoder.
const DECODER: u64 = 1;

/// A trusted core with page 100 of [`CHANNEL`] open and drawn in the
/// active tab, a decoder whose input the test reads, and a script host
/// started for the channel, so that its scripts run.
struct Viewer {
    core: Core,
    /// Kept for the broker's far end, which ends with it.
    _stand_in: StandIn,
    to_interface: mpsc::Receiver<Vec<u8>>,
    to_decoder: mpsc::Receiver<Vec<u8>>,
    key: PageKey,
}

fn open_page() -> PageSub {
    PageSub::new(PAGE, 0)
}

impl Viewer {
    fn new() -> Self {
        let (mut core, stand_in, to_interface) = core_watched(CHANNEL);
        let channel = core.active_channel().unwrap();
        let key = PageKey { channel, page: PAGE };
        let to_decoder = decoder_watched(&mut core, channel);
        core.intent(ToCore::Opened(Some(key))).unwrap();
        core.intent(ToCore::Focused(true)).unwrap();
        let sends = core.typing.drawn(key, 0);
        core.deliver_typing(sends).unwrap();
        assert_eq!(core.typing.rules_open(channel), Some(open_page()));
        // The decoder greets its script host: the core starts one.
        let mut hello = Vec::new();
        write_to_host(&mut hello, &ToScriptHost::Hello { link_version: protocol::script::LINK_VERSION }).unwrap();
        let Some(DecoderOutput::ToHost(asked)) = read_decoder_output(&mut hello.as_slice()).unwrap() else { panic!("not the script link's") };
        core.script_asked(channel, DECODER, &asked);
        assert_eq!(core.script_state(channel), ScriptState::Running);
        let mut viewer = Self { core, _stand_in: stand_in, to_interface, to_decoder, key };
        viewer.interface();
        viewer.decoder();
        viewer
    }

    fn channel(&self) -> u32 {
        self.key.channel
    }

    /// The decoder reports what the open page's scripts claim, and the
    /// gate's work after an event is done.
    fn claims(&mut self, keys: &[&str], text: bool, pointer: PointerClaim, wheel: bool) {
        let claims =
            ScriptClaims { page: open_page(), running: true, network: false, text, pointer, wheel, keys: keys.iter().map(|key| (*key).to_owned()).collect() };
        self.report(ScriptReport::Claims(claims));
    }

    fn report(&mut self, report: ScriptReport) {
        let channel = self.channel();
        self.core.script_reported(channel, report).unwrap();
        self.tick();
    }

    fn tick(&mut self) {
        self.core.script_gate_tick(Instant::now()).unwrap();
    }

    /// What the interface was sent since last asked.
    fn interface(&mut self) -> Vec<ToInterface> {
        sent_to_interface(&mut self.core, &self.to_interface)
    }

    /// The key defaults the interface was sent since last asked.
    fn defaults(&mut self) -> Vec<KeyDefault> {
        self.interface().into_iter().filter_map(|message| if let ToInterface::KeyDefault(default) = message { Some(default) } else { None }).collect()
    }

    /// What the decoder was sent since last asked: the trusted core's own
    /// messages, and the script link's.
    fn decoder_input(&mut self) -> Vec<DecoderInput> {
        let channel = self.channel();
        self.core.decoders.get_mut(&channel).unwrap().flush().unwrap();
        let mut bytes = Vec::new();
        while let Ok(chunk) = self.to_decoder.recv_timeout(Duration::from_millis(100)) {
            bytes.extend(chunk);
        }
        let mut input = bytes.as_slice();
        let mut messages = Vec::new();
        while let Some(message) = read_decoder_input(&mut input).ok().flatten() {
            messages.push(message);
        }
        messages
    }

    /// The trusted core's messages to the decoder since last asked.
    fn decoder(&mut self) -> Vec<ToDecoder> {
        self.decoder_input().into_iter().filter_map(|message| if let DecoderInput::Core(message) = message { Some(message) } else { None }).collect()
    }

    /// What of the decoder's input is input for the page's scripts: the
    /// keys, and the orders that carry text, the pointer and the wheel.
    fn given(&mut self) -> Vec<ToDecoder> {
        let input = |message: &ToDecoder| {
            matches!(message, ToDecoder::RuleInput { .. } | ToDecoder::Script(_)) && !matches!(message, ToDecoder::Script(ScriptOrder::TabShown(_)))
        };
        self.decoder().into_iter().filter(input).collect()
    }

    fn input(&mut self, intent: Intent) {
        let key = self.key;
        self.core.intent(ToCore::Input(InputIntent { key, intent })).unwrap();
    }

    fn key(&mut self, action: Action, key: KeyCode, mods: u8, text: Option<&str>) {
        self.input(Intent::Key { action, key, mods, text: text.map(str::to_owned), base: None });
    }

    /// Presses a character key that types itself, and gives the number
    /// the decoder was told the event by.
    fn press(&mut self, character: char) -> u32 {
        self.key(Action::Press, KeyCode::Character(character), 0, Some(&character.to_string()));
        let given = self.given();
        given
            .iter()
            .find_map(|message| {
                if let ToDecoder::RuleInput { sequence, input: RuleInput::Key { action: KeyAction::Press, .. }, .. } = message { Some(*sequence) } else { None }
            })
            .unwrap_or_else(|| panic!("{character} was not given: {given:?}"))
    }

    #[cfg(feature = "links")]
    fn address(&self) -> String {
        self.core.session.active_tab().unwrap().address().unwrap().text.clone()
    }
}

fn named(name: &str) -> KeyCode {
    KeyCode::Functional(code(name).unwrap())
}

fn point(x: u32, y: u32) -> Point {
    Point { column: x / 12, row: y / 20, pixels: Some((x, y)), mods: 0 }
}

/// Section 4.6 ("Which keys a script may claim"; DEC-7282 item 4): a
/// manifest's names become the page's key asks, each compared under the
/// modifiers that make a key the browser's own, and a name no script is
/// ever given, or that is no key, becomes none. The interface is told
/// that the page's program runs and holds the keys, and that it types.
#[test]
fn a_manifests_keys_become_the_pages_asks_and_never_a_key_of_the_browsers() {
    let mut viewer = Viewer::new();
    viewer.claims(&["a", "1", "ArrowLeft", "Escape", "F11", "BrowserBack", "Control", "NoSuchKey", "ab", "a"], true, PointerClaim::None, false);
    let sent = viewer.interface();
    let view = sent.iter().find_map(|message| if let ToInterface::Script(view) = message { Some(*view) } else { None });
    assert_eq!(view, Some(ScriptView { channel: viewer.channel(), page: PAGE, run: ScriptRun::Running, holds: true, text: true }));
    let typing = sent.iter().rev().find_map(|message| if let ToInterface::Typing(typing) = message { Some(typing.clone()) } else { None }).unwrap();
    assert!(typing.on && typing.keys && typing.secure && !typing.wheel && !typing.asks, "{typing:?}");
    let character =
        |character: char| KeyAsk { key: Some(Key::of(KeyKind::Character, u32::from(character))), mods_mask: BROWSER_CHORD_MODIFIERS, mods_value: 0 };
    let functional = |name: &str| KeyAsk { key: Some(Key::of(KeyKind::Named, code(name).unwrap())), mods_mask: BROWSER_CHORD_MODIFIERS | ALT, mods_value: 0 };
    assert_eq!(typing.asked, KeyAsks { text: true, keys: vec![character('a'), character('1'), functional("ArrowLeft"), functional("Escape")] });
    // The names that became none, for the channel's developer.
    let claims = viewer.core.script_claims(viewer.channel()).unwrap();
    assert_eq!(keys::asks_of(claims).1, ["F11", "BrowserBack", "Control", "NoSuchKey", "ab"]);
    // Every name of the list, whatever else a manifest names.
    let every: Vec<String> = interface_core::browser::scripts::NEVER_CLAIMABLE_KEYS.iter().map(|name| (*name).to_owned()).collect();
    let all = ScriptClaims { keys: every, ..ScriptClaims::none(open_page()) };
    assert_eq!(keys::asks_of(&all).0.keys, vec![], "none of them is ever asked for");
}

/// Section 4.6: the trusted core decides again what the interface
/// decided. A key the manifest names goes to the page's decoder; a key
/// it does not name, a key pressed with a modifier of the browser's
/// chords, and a key no script is ever given go nowhere, whatever the
/// interface sent; a release follows its press.
#[test]
fn only_a_key_the_manifest_names_and_a_script_may_have_is_given() {
    let mut viewer = Viewer::new();
    viewer.claims(&["a", "ArrowLeft", "F11", "l", "."], false, PointerClaim::None, false);
    viewer.given();
    viewer.key(Action::Press, KeyCode::Character('a'), SHIFT, Some("A"));
    let given = viewer.given();
    assert!(matches!(given.first(), Some(ToDecoder::Script(ScriptOrder::SoundAllowed))), "the viewer acted: {given:?}");
    let key = |action: KeyAction, mods: u8| RuleInput::Key { action, named: false, code: u32::from('a'), base: None, mods };
    assert!(
        matches!(given.get(1), Some(ToDecoder::RuleInput { page, input, .. }) if *page == open_page() && *input == key(KeyAction::Press, SHIFT)),
        "{given:?}"
    );
    assert_eq!(given.len(), 2);
    // Its repeat and its release follow it, whatever is held by then.
    viewer.key(Action::Repeat, KeyCode::Character('a'), 0, Some("a"));
    viewer.key(Action::Release, KeyCode::Character('a'), CONTROL, None);
    let given = viewer.given();
    assert!(
        matches!(given.as_slice(), [ToDecoder::RuleInput { input: first, .. }, ToDecoder::RuleInput { input: second, .. }]
        if *first == key(KeyAction::Repeat, 0) && *second == key(KeyAction::Release, CONTROL)),
        "{given:?}"
    );
    // Not named; the browser's chords, though the manifest names the
    // key; the system's key, though the manifest names it; a release
    // with no press.
    viewer.interface();
    viewer.key(Action::Press, KeyCode::Character('b'), 0, Some("b"));
    viewer.key(Action::Press, KeyCode::Character('l'), CONTROL, Some("\u{c}"));
    viewer.key(Action::Press, KeyCode::Character('l'), SUPER, Some("l"));
    viewer.key(Action::Press, KeyCode::Character('.'), CONTROL, Some("."));
    viewer.key(Action::Press, named("ArrowLeft"), ALT, None);
    viewer.key(Action::Press, named("ArrowLeft"), CONTROL | ALT, None);
    viewer.key(Action::Press, named("F11"), 0, None);
    viewer.key(Action::Release, KeyCode::Character('l'), 0, None);
    assert_eq!(viewer.given(), [], "none of these is a script's");
    assert_eq!(viewer.core.script_gate.input.pending.len(), 2, "and none waits for an answer: only a's press and repeat do");
    // While another part of the browser has the keys, nothing is given.
    viewer.core.intent(ToCore::Editing(true)).unwrap();
    viewer.key(Action::Press, KeyCode::Character('a'), 0, Some("a"));
    assert_eq!(viewer.given(), []);
    // Nor for a page that is not the open one.
    viewer.core.intent(ToCore::Editing(false)).unwrap();
    let other = PageKey { channel: viewer.channel(), page: 0x101 };
    let press = Intent::Key { action: Action::Press, key: KeyCode::Character('a'), mods: 0, text: None, base: None };
    viewer.core.intent(ToCore::Input(InputIntent { key: other, intent: press })).unwrap();
    assert_eq!(viewer.given(), []);
}

/// Section 4.1 ("Each event has a default action"): a key down given to
/// a script takes its default action unless a handler cancelled it, once
/// and only for an event the gate gave: a report naming any other number,
/// the same number twice or another channel's makes no key act.
#[test]
fn a_key_takes_its_default_unless_the_script_cancelled_it() {
    let mut viewer = Viewer::new();
    viewer.claims(&["1", "/"], false, PointerClaim::None, false);
    viewer.interface();
    let cancelled = viewer.press('1');
    viewer.report(ScriptReport::Verdict { sequence: cancelled, cancelled: true, handled: true });
    assert_eq!(viewer.defaults(), [], "a game cancels");
    let passed = viewer.press('1');
    assert_ne!(passed, cancelled);
    viewer.report(ScriptReport::Verdict { sequence: passed, cancelled: false, handled: true });
    assert_eq!(viewer.defaults(), [KeyDefault { key: viewer.key, code: KeyCode::Character('1'), typed: Some('1') }], "a page that watches lets it through");
    // The default goes by what the key typed: Shift and / is Reveal's.
    viewer.key(Action::Press, KeyCode::Character('/'), SHIFT, Some("?"));
    let Some(ToDecoder::RuleInput { sequence, .. }) = viewer.given().pop() else { panic!("not given") };
    viewer.report(ScriptReport::Verdict { sequence, cancelled: false, handled: false });
    assert_eq!(viewer.defaults(), [KeyDefault { key: viewer.key, code: KeyCode::Character('/'), typed: Some('?') }]);
    // Answered already, never given, and not this channel's.
    viewer.report(ScriptReport::Verdict { sequence: passed, cancelled: false, handled: true });
    viewer.report(ScriptReport::Verdict { sequence: passed.wrapping_add(1_000), cancelled: false, handled: true });
    viewer.key(Action::Release, KeyCode::Character('1'), 0, None);
    let waiting = viewer.press('1');
    let other = viewer.channel() + 1;
    viewer.core.script_reported(other, ScriptReport::Verdict { sequence: waiting, cancelled: false, handled: true }).unwrap();
    viewer.tick();
    assert_eq!(viewer.defaults(), [], "no key acts that the viewer did not press here, and none twice");
    // A release has no default and is not waited for.
    viewer.key(Action::Release, KeyCode::Character('1'), 0, None);
    assert_eq!(viewer.core.script_gate.input.pending.len(), 1, "only the press waits");
}

/// Sections 4.1 and 1 principle 5 ("Fail open"), timed end to end at the
/// gate: a script that does not answer never holds a key. With no report
/// at all, as from a decoder that has stopped too, the default is taken
/// at [`VERDICT_DEADLINE`] after the key was given and not before; a
/// late answer then changes nothing.
#[test]
fn a_script_that_does_not_answer_never_holds_a_key() {
    let mut viewer = Viewer::new();
    viewer.claims(&["1"], false, PointerClaim::None, false);
    viewer.interface();
    let before = Instant::now();
    let sequence = viewer.press('1');
    let after = Instant::now();
    let due = viewer.core.script_gate_due_at().expect("the gate has a deadline to keep");
    assert!(due >= before + VERDICT_DEADLINE && due <= after + VERDICT_DEADLINE, "the deadline is the key's own time and the bound");
    assert_eq!(VERDICT_DEADLINE, 2 * STEP_DEADLINE, "the script host's time to answer, and as long again");
    // The trusted core's timers wake it then.
    assert!(viewer.core.next_wake().is_some_and(|next| next <= due));
    viewer.core.script_gate_tick(due.checked_sub(Duration::from_millis(1)).unwrap()).unwrap();
    assert_eq!(viewer.defaults(), [], "the script still has time");
    viewer.core.script_gate_tick(due).unwrap();
    assert_eq!(viewer.defaults(), [KeyDefault { key: viewer.key, code: KeyCode::Character('1'), typed: Some('1') }]);
    assert_eq!(viewer.core.script_gate_due_at(), None);
    viewer.report(ScriptReport::Verdict { sequence, cancelled: false, handled: true });
    assert_eq!(viewer.defaults(), [], "the late answer is no second default");
}

/// Section 4.1: "If the context is stopped, absent, or passes its time
/// limit in that handler, the default happens": when the scripts stop,
/// by a limit or by their decoder's word, every key they were given and
/// had not answered takes its default at once, in the order pressed, and
/// the interface is told that the page's program has stopped and holds
/// nothing.
#[test]
fn a_stopped_scripts_keys_take_their_defaults_at_once() {
    let mut viewer = Viewer::new();
    viewer.claims(&["1", "2"], false, PointerClaim::None, false);
    viewer.interface();
    viewer.press('1');
    viewer.press('2');
    let pressed = Instant::now();
    let channel = viewer.channel();
    viewer.core.script_reported(channel, ScriptReport::Stopped { reason: FailReason::Time, text: "STEP_DEADLINE".into() }).unwrap();
    viewer.tick();
    assert!(pressed.elapsed() < VERDICT_DEADLINE, "the test did not simply wait");
    assert_eq!(viewer.core.script_state(channel), ScriptState::Stopped(FailReason::Time));
    let sent = viewer.interface();
    let defaults: Vec<KeyCode> =
        sent.iter().filter_map(|message| if let ToInterface::KeyDefault(default) = message { Some(default.code) } else { None }).collect();
    assert_eq!(defaults, [KeyCode::Character('1'), KeyCode::Character('2')]);
    let view = sent.iter().find_map(|message| if let ToInterface::Script(view) = message { Some(*view) } else { None });
    assert_eq!(view, Some(ScriptView { channel, page: PAGE, run: ScriptRun::Stopped, holds: false, text: false }));
    let typing = sent.iter().rev().find_map(|message| if let ToInterface::Typing(typing) = message { Some(typing.clone()) } else { None }).unwrap();
    assert!(!typing.on && !typing.keys && typing.asked == KeyAsks::default(), "every key is the viewer's again: {typing:?}");
    // The defaults come before the word that the keys are the viewer's.
    let place = |wanted: fn(&ToInterface) -> bool| sent.iter().position(wanted).unwrap();
    assert!(place(|message| matches!(message, ToInterface::KeyDefault(_))) < place(|message| matches!(message, ToInterface::Typing(_))));
    // And a key now is no script's.
    viewer.given();
    viewer.key(Action::Press, KeyCode::Character('1'), 0, Some("1"));
    assert_eq!(viewer.given(), []);
}

/// Section 4.6 (DEC-7282 item 6): the viewer's own key stops the program
/// of the active tab's page for good: its script host is ended, its
/// decoder told with the reason, the interface told it stopped, and
/// nothing more is given to it.
#[test]
fn the_viewers_own_key_stops_a_pages_program() {
    let mut viewer = Viewer::new();
    viewer.claims(&["a"], true, PointerClaim::Move, true);
    viewer.interface();
    viewer.decoder_input();
    viewer.core.intent(ToCore::StopScripts).unwrap();
    viewer.tick();
    assert_eq!(viewer.core.script_state(viewer.channel()), ScriptState::Stopped(FailReason::Viewer));
    let told = viewer.decoder_input();
    assert!(
        told.iter().any(|message| matches!(message, DecoderInput::Link(FromScriptHost::Failed { reason: FailReason::Viewer, .. }))),
        "the decoder hears why: {told:?}"
    );
    let sent = viewer.interface();
    assert!(sent.iter().any(|message| matches!(message, ToInterface::Script(view) if view.run == ScriptRun::Stopped && !view.holds)), "{sent:?}");
    assert!(sent.iter().any(|message| matches!(message, ToInterface::Typing(typing) if !typing.on)), "{sent:?}");
    viewer.key(Action::Press, KeyCode::Character('a'), 0, Some("a"));
    viewer.input(Intent::Mouse { action: MouseAction::Press, button: 0, at: point(100, 100) });
    assert_eq!(viewer.given(), []);
    // With no program running the key stops nothing.
    viewer.core.intent(ToCore::StopScripts).unwrap();
    assert_eq!(viewer.core.script_state(viewer.channel()), ScriptState::Stopped(FailReason::Viewer));
}

/// Sections 4.2 and 14 (DOC-1794's amendment): a static channel's script
/// page is given the typed text, the pointer and the wheel its manifest
/// claims, and none it does not claim.
#[test]
fn a_script_page_is_given_the_text_pointer_and_wheel_it_claims() {
    let mut viewer = Viewer::new();
    // Nothing claimed but one key: no text, pointer or wheel is given.
    viewer.claims(&["a"], false, PointerClaim::None, false);
    viewer.input(Intent::Text("été".into()));
    viewer.input(Intent::Paste("pasted".into()));
    viewer.input(Intent::Mouse { action: MouseAction::Press, button: 0, at: point(100, 100) });
    viewer.input(Intent::Wheel { dx: 0, dy: 240, at: point(100, 100) });
    viewer.input(Intent::Gone);
    assert_eq!(viewer.given(), []);
    // Text, presses and the wheel claimed; moves not.
    viewer.claims(&[], true, PointerClaim::Press, true);
    viewer.input(Intent::Text("été".into()));
    viewer.input(Intent::Paste("pasted".into()));
    viewer.key(Action::Press, KeyCode::Character('e'), 0, Some("e"));
    viewer.input(Intent::Compose { cursor: 0, text: "preedit".into() });
    let texts: Vec<String> = viewer
        .given()
        .into_iter()
        .filter_map(|message| if let ToDecoder::Script(ScriptOrder::Text { text, .. }) = message { Some(text) } else { None })
        .collect();
    assert_eq!(texts, ["été", "pasted", "e"], "committed text, a paste and what a key typed; never a preedit");
    assert!(viewer.core.script_gate.input.pending.is_empty(), "a key that only types is not waited for");
    viewer.input(Intent::Mouse { action: MouseAction::Move, button: 0, at: point(10, 10) });
    viewer.input(Intent::Mouse { action: MouseAction::Press, button: 0, at: point(100, 120) });
    viewer.input(Intent::Mouse { action: MouseAction::Release, button: 0, at: point(101, 121) });
    viewer.input(Intent::Wheel { dx: -120, dy: 240, at: point(100, 120) });
    let given = viewer.given();
    let pointers: Vec<ScriptPointer> =
        given.iter().filter_map(|message| if let ToDecoder::Script(ScriptOrder::Pointer(pointer)) = message { Some(*pointer) } else { None }).collect();
    // The decoder is told the move too: a claim of presses includes the
    // pointer coming onto a thing and off it, which the decoder tells
    // from where the pointer is; it gives a script no move event.
    assert_eq!(pointers.len(), 3, "{given:?}");
    let mouse = |pointer: &ScriptPointer| (pointer.pointer, pointer.kind, pointer.primary) == (MOUSE_POINTER, PointerKind::Mouse, true);
    assert!(pointers.iter().all(mouse));
    assert_eq!((pointers[0].action, pointers[0].x, pointers[0].y), (PointerAction::Move, 10, 10));
    assert_eq!((pointers[1].action, pointers[1].x, pointers[1].y), (PointerAction::Press, 100, 120));
    assert_eq!((pointers[2].action, pointers[2].x, pointers[2].y), (PointerAction::Release, 101, 121));
    assert!(given.iter().any(|message| matches!(message, ToDecoder::Script(ScriptOrder::Wheel { x: 100, y: 120, delta_x: -1, delta_y: 2, .. }))), "{given:?}");
    // A press and a release are waited for, and have no default here.
    viewer.interface();
    assert_eq!(viewer.core.script_gate.input.pending.len(), 2, "the press and the release");
    viewer.core.script_gate_tick(Instant::now() + VERDICT_DEADLINE).unwrap();
    assert_eq!(viewer.defaults(), [], "a pointer's press and release have no default of the browser's here");
    assert!(viewer.core.script_gate.input.pending.is_empty());
    // With moves claimed, a move and the pointer's leaving are given.
    viewer.claims(&[], false, PointerClaim::Move, false);
    viewer.given();
    viewer.input(Intent::Mouse { action: MouseAction::Move, button: 1, at: point(10, 10) });
    viewer.input(Intent::Gone);
    let actions: Vec<PointerAction> = viewer
        .given()
        .into_iter()
        .filter_map(|message| if let ToDecoder::Script(ScriptOrder::Pointer(pointer)) = message { Some(pointer.action) } else { None })
        .collect();
    assert_eq!(actions, [PointerAction::Move, PointerAction::Gone]);
}

/// Section 4.4 ("Touch is delivered as pointer events with `pointerType:
/// "touch"`; at most `MAX_POINTERS` pointers are followed at once, and a
/// further touch is not reported"): each contact is a pointer with its
/// own number from its press to its release, the first down the primary
/// one, beside the mouse's.
#[test]
fn the_contacts_of_a_touch_screen_are_pointers_of_their_own() {
    let mut viewer = Viewer::new();
    viewer.claims(&[], false, PointerClaim::Move, false);
    viewer.given();
    let touched = |viewer: &mut Viewer, phase: TouchPhase, contact: u64| {
        viewer.input(Intent::Touch { phase, contact, at: point(u32::try_from(contact).unwrap(), 40) });
        viewer
            .given()
            .into_iter()
            .filter_map(|message| if let ToDecoder::Script(ScriptOrder::Pointer(pointer)) = message { Some(pointer) } else { None })
            .collect::<Vec<ScriptPointer>>()
    };
    let first = touched(&mut viewer, TouchPhase::Began, 71);
    assert!(
        matches!(first.as_slice(), [ScriptPointer { action: PointerAction::Press, button: 0, pointer: 2, kind: PointerKind::Touch, primary: true, .. }]),
        "{first:?}"
    );
    let second = touched(&mut viewer, TouchPhase::Began, 72);
    assert!(matches!(second.as_slice(), [ScriptPointer { pointer: 3, kind: PointerKind::Touch, primary: false, .. }]), "{second:?}");
    let third = touched(&mut viewer, TouchPhase::Began, 73);
    assert!(matches!(third.as_slice(), [ScriptPointer { pointer: 4, .. }]), "{third:?}");
    assert_eq!(usize::from(MAX_POINTERS), 1 + MAX_CONTACTS, "the mouse and three contacts are every pointer");
    assert_eq!(touched(&mut viewer, TouchPhase::Began, 74), [], "a further touch is not reported");
    assert_eq!(touched(&mut viewer, TouchPhase::Moved, 74), []);
    assert_eq!(touched(&mut viewer, TouchPhase::Ended, 74), []);
    // A move keeps its contact's number; a release ends it and frees it.
    let moved = touched(&mut viewer, TouchPhase::Moved, 72);
    assert!(matches!(moved.as_slice(), [ScriptPointer { action: PointerAction::Move, button: 1, pointer: 3, primary: false, .. }]), "{moved:?}");
    let lifted = touched(&mut viewer, TouchPhase::Ended, 72);
    assert!(matches!(lifted.as_slice(), [ScriptPointer { action: PointerAction::Release, pointer: 3, .. }]), "{lifted:?}");
    let cancelled = touched(&mut viewer, TouchPhase::Cancelled, 73);
    assert!(matches!(cancelled.as_slice(), [ScriptPointer { action: PointerAction::Gone, pointer: 4, .. }]), "{cancelled:?}");
    let again = touched(&mut viewer, TouchPhase::Began, 75);
    assert!(matches!(again.as_slice(), [ScriptPointer { action: PointerAction::Press, pointer: 3, primary: false, .. }]), "{again:?}");
    // The mouse is still pointer 1 beside them.
    viewer.input(Intent::Mouse { action: MouseAction::Press, button: 0, at: point(5, 5) });
    let given = viewer.given();
    assert!(given.iter().any(|message| matches!(message, ToDecoder::Script(ScriptOrder::Pointer(ScriptPointer { pointer: 1, kind: PointerKind::Mouse, .. })))));
    // A page that claims no pointer is given no contact.
    viewer.claims(&["a"], false, PointerClaim::None, false);
    viewer.given();
    assert_eq!(touched(&mut viewer, TouchPhase::Began, 80), []);
}

/// Section 3.5 (DEC-7282 item 7): a script page makes no sound until the
/// viewer has pressed a key or the pointer in its tab; the decoder is
/// told once, at the first such press.
#[test]
fn sound_is_allowed_once_the_viewer_has_pressed_a_key_or_the_pointer() {
    let allowed = |given: &[ToDecoder]| given.iter().filter(|message| matches!(message, ToDecoder::Script(ScriptOrder::SoundAllowed))).count();
    let mut viewer = Viewer::new();
    viewer.claims(&["a"], false, PointerClaim::Move, false);
    assert_eq!(allowed(&viewer.given()), 0, "running and claiming is not the viewer acting");
    viewer.input(Intent::Mouse { action: MouseAction::Move, button: 0, at: point(5, 5) });
    assert_eq!(allowed(&viewer.given()), 0, "nor is the pointer moving");
    viewer.input(Intent::Mouse { action: MouseAction::Press, button: 0, at: point(5, 5) });
    assert_eq!(allowed(&viewer.given()), 1);
    viewer.key(Action::Press, KeyCode::Character('a'), 0, Some("a"));
    viewer.input(Intent::Mouse { action: MouseAction::Press, button: 0, at: point(5, 5) });
    assert_eq!(allowed(&viewer.given()), 0, "told once");
}

/// Section 3.3 (`nt.page.title`): a script's title is untrusted text,
/// bounded and cleaned, that names its page where every page's title
/// shows, in the page list, and only for the open page of a channel
/// whose scripts run. Nothing of it reaches the strip.
#[test]
fn a_scripts_title_names_its_page_in_the_page_list_and_nowhere_else() {
    let mut viewer = Viewer::new();
    viewer.claims(&[], false, PointerClaim::None, false);
    viewer.interface();
    let channel = viewer.channel();
    // The page has arrived, as a page shown has.
    viewer.core.session.tabs.iter_mut().find(|tab| tab.channel.id == channel).unwrap().channel.arrived(PAGE, "Page 100");
    viewer.report(ScriptReport::Title { page: open_page(), title: "Keys go to bank.example".into() });
    let sent = viewer.interface();
    assert!(
        sent.iter()
            .any(|message| matches!(message, ToInterface::Page { channel: told, page: PAGE, title } if *told == channel && title == "Keys go to bank.example")),
        "{sent:?}"
    );
    assert!(sent.iter().all(|message| matches!(message, ToInterface::Page { .. })), "the page list alone hears a page's own words: {sent:?}");
    assert!(!viewer.core.strip().identity.contains("bank"), "the strip's identity is the channel's address");
    // Not the open page; and once the scripts have stopped, no title.
    viewer.report(ScriptReport::Title { page: PageSub::new(0x101, 0), title: "Other".into() });
    viewer.report(ScriptReport::Title { page: PageSub::new(PAGE, 1), title: "Another sub-page".into() });
    assert_eq!(viewer.interface(), []);
    viewer.report(ScriptReport::Stopped { reason: FailReason::Memory, text: "stopped".into() });
    viewer.interface();
    viewer.report(ScriptReport::Title { page: open_page(), title: "After".into() });
    assert!(!viewer.interface().iter().any(|message| matches!(message, ToInterface::Page { .. })));
    // The reader bounds it: a longer title, or one with a control
    // character, never reaches the trusted core.
    let long = ScriptReport::Title { page: open_page(), title: "t".repeat(protocol::script::MAX_TITLE_BYTES + 1) };
    assert!(protocol::decoder::write_to_core(&mut Vec::new(), &protocol::decoder::FromDecoder::Script(long)).is_err());
}

/// Sections 3.1 and 4.6 (TODO-7407): the interface is told what a page's
/// program does in every change of it, also one that comes while the
/// trusted core has been told of no open page, as between a tab chosen
/// and its page reported open again: a program that stops then is told
/// as stopped, for the page the interface was last told of.
#[test]
fn a_program_that_stops_while_no_page_is_reported_open_is_told_as_stopped() {
    let views = |sent: &[ToInterface]| -> Vec<(ScriptRun, bool)> {
        sent.iter().filter_map(|message| if let ToInterface::Script(view) = message { Some((view.run, view.holds)) } else { None }).collect()
    };
    let mut viewer = Viewer::new();
    viewer.claims(&["a"], false, PointerClaim::None, false);
    assert_eq!(views(&viewer.interface()), [(ScriptRun::Running, true)]);
    viewer.core.intent(ToCore::Opened(None)).unwrap();
    viewer.tick();
    // Still the page's program, running; it holds nothing off the page.
    assert_eq!(views(&viewer.interface()), [(ScriptRun::Running, false)]);
    viewer.report(ScriptReport::Stopped { reason: FailReason::Link, text: "refused".into() });
    assert_eq!(views(&viewer.interface()), [(ScriptRun::Stopped, false)]);
    // The page reported open again changes nothing of it.
    let key = viewer.key;
    viewer.core.intent(ToCore::Opened(Some(key))).unwrap();
    viewer.tick();
    assert_eq!(views(&viewer.interface()), []);
    assert_eq!(viewer.core.script_gate.told.map(|view| view.run), Some(ScriptRun::Stopped));
}

/// TODO-7410: the Log tab says why a channel's scripts stopped, in the
/// browser's own words for the reason, with the script host's text
/// beside it cleaned and bounded; also when the trusted core's relay,
/// which reads no payload, noted the stop first and knew no reason.
#[test]
fn the_log_says_why_a_channels_scripts_stopped() {
    let logged = |viewer: &mut Viewer| {
        let channel = viewer.channel();
        viewer.core.console.set_open(true);
        format!("{:?}", viewer.core.console.updates(Some(channel)))
    };
    // The decoder's word alone: a time limit.
    let mut viewer = Viewer::new();
    viewer.report(ScriptReport::Stopped { reason: FailReason::Time, text: "CALLBACK_TIME_LIMIT".into() });
    let said = logged(&mut viewer);
    assert!(said.contains("a time limit was passed") && said.contains("CALLBACK_TIME_LIMIT"), "{said}");
    // The relay saw the script host say it stopped, and then the decoder
    // says why: the reason kept is the decoder's, and the log has it.
    let mut viewer = Viewer::new();
    let channel = viewer.channel();
    let mut failed = Vec::new();
    let said_failed = FromScriptHost::Failed { context: protocol::script::ContextId::Channel, reason: FailReason::Memory, text: "heap".into() };
    protocol::script::write_to_decoder(&mut failed, &said_failed).unwrap();
    let Some(protocol::script::relay::HostOutput::ToDecoder(relayed)) = protocol::script::relay::read_host_output(&mut failed.as_slice()).unwrap() else {
        panic!("not for the decoder")
    };
    let generation = viewer.core.generation;
    viewer.core.script_said(channel, generation, protocol::script::relay::HostOutput::ToDecoder(relayed));
    assert!(matches!(viewer.core.script_state(channel), ScriptState::Stopped(_)));
    viewer.report(ScriptReport::Stopped { reason: FailReason::Memory, text: "QUICKJS_HEAP_BYTES\u{7} passed".into() });
    assert_eq!(viewer.core.script_state(channel), ScriptState::Stopped(FailReason::Memory));
    let said = logged(&mut viewer);
    assert!(said.contains("a memory limit was passed") && said.contains("QUICKJS_HEAP_BYTES passed"), "{said}");
    assert_eq!(said.matches("were stopped").count(), 1, "one line for one stop: {said}");
    // The viewer's stop says so.
    let mut viewer = Viewer::new();
    viewer.core.intent(ToCore::StopScripts).unwrap();
    let said = logged(&mut viewer);
    assert_eq!(said.matches("the viewer stopped the page's program").count(), 1, "said once: {said}");
}

/// Section 3.1's table and TODO-7405: only a hidden tab pauses a
/// channel's scripts. A change of the open page, the page chosen again,
/// and the tab's own key pressed for the tab already in front tell the
/// decoder that the tab is shown, whatever they say of the open page.
#[test]
fn only_a_hidden_tab_is_told_as_hidden() {
    let shown = |told: &[ToDecoder]| -> Vec<bool> {
        told.iter().filter_map(|message| if let ToDecoder::Script(ScriptOrder::TabShown(shown)) = message { Some(*shown) } else { None }).collect()
    };
    let mut viewer = Viewer::new();
    viewer.claims(&["a"], false, PointerClaim::None, false);
    viewer.decoder();
    // Another page of the channel is chosen and drawn.
    let next = PageKey { channel: viewer.channel(), page: 0x101 };
    viewer.core.intent(ToCore::Opened(Some(next))).unwrap();
    let sends = viewer.core.typing.drawn(next, 0);
    viewer.core.deliver_typing(sends).unwrap();
    let told = viewer.decoder();
    assert!(told.iter().any(|message| matches!(message, ToDecoder::RulesPage(Some(_)))), "{told:?}");
    assert!(!shown(&told).contains(&false), "a change of page hides nothing: {told:?}");
    // The tab's own key, for the tab already in front.
    viewer.core.intent(ToCore::SelectTab(0)).unwrap();
    assert_eq!(viewer.decoder(), [], "the tab in front chosen again tells its decoder nothing");
    // The tab's own key changes nothing at all: the page is still the
    // open one, and the decoder is told nothing (TODO-7423). Another tab
    // in front, and this one coming back, is tested with the real
    // decoder and script host (`scripts/loading_tests.rs`).
    assert_eq!(viewer.core.typing.opened_page(), Some(next));
    assert_eq!(viewer.core.script_state(viewer.channel()), ScriptState::Running);
}

/// Section 4.2 (`navigate`, cause `"browser"`): before the decoder of a
/// channel whose scripts run is told that another page of it is the open
/// one, its scripts are told `navigate` with where the tab goes, by a
/// cause no script can cancel.
#[cfg(feature = "links")]
#[test]
fn the_scripts_of_a_page_the_tab_leaves_hear_navigate_first() {
    use protocol::decoder::script::NavigationCause;
    let mut viewer = Viewer::new();
    viewer.claims(&["a"], false, PointerClaim::None, false);
    viewer.decoder();
    let next = PageKey { channel: viewer.channel(), page: 0x101 };
    viewer.core.intent(ToCore::Opened(Some(next))).unwrap();
    let sends = viewer.core.typing.drawn(next, 2);
    viewer.core.deliver_typing(sends).unwrap();
    let told = viewer.decoder();
    let navigating = told
        .iter()
        .position(|message| matches!(message, ToDecoder::Script(ScriptOrder::Navigating { cause: NavigationCause::Browser, target }) if target == "101/0002"));
    let opened = told.iter().position(|message| matches!(message, ToDecoder::RulesPage(Some(page)) if *page == PageSub::new(0x101, 2)));
    assert!(navigating.is_some() && navigating < opened, "{told:?}");
    assert_eq!(told.iter().filter(|message| matches!(message, ToDecoder::Script(ScriptOrder::Navigating { .. }))).count(), 1);
}

#[cfg(feature = "links")]
mod navigation {
    use super::super::navigate::{MAX_NAVIGATIONS_PER_SECOND, NAVIGATION_SECOND};
    use super::*;

    fn navigate(viewer: &mut Viewer, target: &str) {
        viewer.report(ScriptReport::Navigate { page: open_page(), target: target.into() });
    }

    /// The decoder holds the same bound over its script host.
    #[test]
    fn the_rate_of_navigations_is_the_decoders() {
        assert_eq!(MAX_NAVIGATIONS_PER_SECOND, decoder::scripts::limits::MAX_NAVIGATIONS_PER_SECOND);
    }

    /// Section 3.3: a script's `nt.navigate` to a page of its own channel
    /// is followed at any time, as a link to it is: the tab's address
    /// names the page and the interface opens it. The script that asked
    /// heard its own `navigate`, so its decoder is not told one for it,
    /// and once the tab has left its page it asks nothing more.
    #[test]
    fn a_script_goes_to_a_page_of_its_own_channel() {
        let mut viewer = Viewer::new();
        viewer.claims(&["a"], false, PointerClaim::None, false);
        viewer.interface();
        viewer.decoder();
        navigate(&mut viewer, "200");
        assert_eq!(viewer.address(), format!("{CHANNEL}200"));
        let shown = PageKey { channel: viewer.channel(), page: 0x200 };
        assert!(viewer.interface().iter().any(|message| matches!(message, ToInterface::Show(key) if *key == shown)));
        assert!(!viewer.decoder().iter().any(|message| matches!(message, ToDecoder::Script(ScriptOrder::Navigating { .. }))));
        // Page 100 is no longer the open page: its scripts' next ask,
        // though the rate allows it, goes nowhere.
        navigate(&mut viewer, "201");
        assert_eq!(viewer.address(), format!("{CHANNEL}200"));
    }

    /// Sections 3.3 and 10.5: at most `MAX_NAVIGATIONS_PER_SECOND` asks
    /// in any second are looked at, followed or not, whatever the decoder
    /// reports; the next second allows as many again.
    #[test]
    fn a_scripts_navigations_are_held_to_their_rate() {
        let mut viewer = Viewer::new();
        viewer.claims(&["a"], false, PointerClaim::None, false);
        let (home, channel) = (viewer.address(), viewer.channel());
        assert_eq!(MAX_NAVIGATIONS_PER_SECOND, 2);
        // Two asks that are refused use the second up.
        navigate(&mut viewer, ELSEWHERE);
        navigate(&mut viewer, "9FF");
        assert_eq!(viewer.core.script_gate.input.navigations[&channel].len(), MAX_NAVIGATIONS_PER_SECOND);
        for target in ["200", "201", "202"] {
            navigate(&mut viewer, target);
        }
        assert_eq!(viewer.address(), home, "no more than the bound in a second");
        assert_eq!(viewer.core.script_gate.input.navigations[&channel].len(), MAX_NAVIGATIONS_PER_SECOND, "and nothing kept past it");
        // A second later.
        for at in viewer.core.script_gate.input.navigations.get_mut(&channel).unwrap() {
            *at = at.checked_sub(NAVIGATION_SECOND).unwrap();
        }
        navigate(&mut viewer, "200");
        assert_eq!(viewer.address(), format!("{CHANNEL}200"));
    }

    /// Section 3.3: to another channel only while the viewer's key press
    /// or pointer release is being handled. A script with no such action
    /// stays in its channel whatever its decoder reports.
    #[test]
    fn a_script_does_not_leave_its_channel_without_the_viewers_own_action() {
        let mut viewer = Viewer::new();
        viewer.claims(&["a"], false, PointerClaim::Press, false);
        let home = viewer.address();
        navigate(&mut viewer, ELSEWHERE);
        assert_eq!(viewer.address(), home, "no action of the viewer's");
        // A key the script did not hear and a pointer's press are not
        // that action.
        let unheard = viewer.press('a');
        viewer.report(ScriptReport::Verdict { sequence: unheard, cancelled: false, handled: false });
        viewer.core.script_gate.input.navigations.clear();
        navigate(&mut viewer, ELSEWHERE);
        viewer.input(Intent::Mouse { action: MouseAction::Press, button: 0, at: point(5, 5) });
        let Some(ToDecoder::Script(ScriptOrder::Pointer(press))) = viewer.given().pop() else { panic!("no press given") };
        viewer.report(ScriptReport::Verdict { sequence: press.sequence, cancelled: false, handled: true });
        navigate(&mut viewer, ELSEWHERE);
        assert_eq!(viewer.address(), home);
        // A report of an event the gate never gave is no action either.
        viewer.core.script_gate.input.navigations.clear();
        viewer.report(ScriptReport::Verdict { sequence: 9_999, cancelled: true, handled: true });
        navigate(&mut viewer, ELSEWHERE);
        assert_eq!(viewer.address(), home);
        assert!(viewer.core.script_gate.input.activated.is_empty());
    }

    /// The other half of the rule: the viewer's key press, heard by the
    /// script, lets it leave once, by the tab's own navigation.
    #[test]
    fn the_viewers_key_press_lets_a_script_leave_once() {
        let mut viewer = Viewer::new();
        viewer.claims(&["a"], false, PointerClaim::None, false);
        let heard = viewer.press('a');
        viewer.report(ScriptReport::Verdict { sequence: heard, cancelled: true, handled: true });
        assert!(viewer.core.script_gate.input.activated.contains_key(&viewer.channel()));
        navigate(&mut viewer, ELSEWHERE);
        assert_eq!(viewer.address(), ELSEWHERE, "the tab left for the other channel");
        assert!(viewer.core.script_gate.input.activated.is_empty(), "one action, one leaving");
    }

    /// Escape is never the action that lets a script leave: it is the
    /// key a viewer presses to get away (HTML's activation triggering
    /// input events leave it out).
    #[test]
    fn escape_is_not_the_viewers_action() {
        let mut viewer = Viewer::new();
        viewer.claims(&["Escape"], false, PointerClaim::None, false);
        let home = viewer.address();
        viewer.key(Action::Press, named("Escape"), 0, None);
        let Some(ToDecoder::RuleInput { sequence, .. }) = viewer.given().pop() else { panic!("Escape was not given") };
        viewer.report(ScriptReport::Verdict { sequence, cancelled: true, handled: true });
        navigate(&mut viewer, ELSEWHERE);
        assert_eq!(viewer.address(), home);
    }

    /// The pointer's release is the viewer's action too.
    #[test]
    fn the_viewers_pointer_release_lets_a_script_leave() {
        let mut viewer = Viewer::new();
        viewer.claims(&[], false, PointerClaim::Press, false);
        viewer.input(Intent::Mouse { action: MouseAction::Release, button: 0, at: point(5, 5) });
        let Some(ToDecoder::Script(ScriptOrder::Pointer(release))) = viewer.given().pop() else { panic!("no release given") };
        viewer.report(ScriptReport::Verdict { sequence: release.sequence, cancelled: false, handled: true });
        navigate(&mut viewer, ELSEWHERE);
        assert_eq!(viewer.address(), ELSEWHERE);
    }

    /// The viewer's action is the script's only for the time one step may
    /// take: a navigation reported later is not that action's.
    #[test]
    fn the_viewers_action_does_not_last() {
        let mut viewer = Viewer::new();
        viewer.claims(&["a"], false, PointerClaim::None, false);
        let heard = viewer.press('a');
        viewer.report(ScriptReport::Verdict { sequence: heard, cancelled: true, handled: true });
        let (home, channel) = (viewer.address(), viewer.channel());
        let long_ago = Instant::now().checked_sub(ACTIVATION_WINDOW + Duration::from_millis(1)).unwrap();
        viewer.core.script_gate.input.activated.insert(channel, long_ago);
        let report = ScriptReport::Navigate { page: open_page(), target: ELSEWHERE.into() };
        viewer.core.script_reported(channel, report).unwrap();
        assert_eq!(viewer.address(), home);
    }

    /// Section 3.3 and principle 1: a navigation is the open page's
    /// scripts' alone. One reported for another page, for a channel that
    /// is not in front, while the page does not have the keyboard, after
    /// the scripts stopped, or with a target that is no page and no
    /// address the address field would open, goes nowhere.
    #[test]
    fn a_navigation_is_followed_only_for_the_open_page_of_running_scripts() {
        let mut viewer = Viewer::new();
        viewer.claims(&["a"], false, PointerClaim::None, false);
        let (home, channel) = (viewer.address(), viewer.channel());
        viewer.report(ScriptReport::Navigate { page: PageSub::new(0x101, 0), target: "200".into() });
        viewer.core.script_reported(channel + 1, ScriptReport::Navigate { page: open_page(), target: "200".into() }).unwrap();
        for target in ["", "9FF", "100:0002", "file:///etc/passwd", "web:https://example.test/", "javascript://x/%0Aalert(1)"] {
            viewer.core.script_gate.input.navigations.clear();
            navigate(&mut viewer, target);
        }
        assert_eq!(viewer.address(), home);
        viewer.core.script_gate.input.navigations.clear();
        viewer.core.intent(ToCore::Editing(true)).unwrap();
        navigate(&mut viewer, "200");
        assert_eq!(viewer.address(), home, "the address field has the keys");
        viewer.core.intent(ToCore::Editing(false)).unwrap();
        viewer.report(ScriptReport::Stopped { reason: FailReason::Failures, text: "stopped".into() });
        navigate(&mut viewer, "200");
        assert_eq!(viewer.address(), home, "stopped scripts ask nothing");
    }
}

/// REQ-6374 on a script's page (rdm TODO-7425): the receiver draws a
/// detectable drawing's hover look while the pointer is over it and its
/// pressed look while it is pressed, so the decoder of a page whose
/// scripts hold the viewer's input is told which drawing that is, as the
/// decoder of a page keys go to is. A page whose scripts hold nothing is
/// told of none.
#[test]
fn the_decoder_of_a_script_page_is_told_the_drawing_hovered_and_pressed() {
    use protocol::regis::{PictureLanguage, RegisPick, RegisPointer, RegisViewer};
    /// The drawing that fills the picture's first hundred lines.
    const BUTTON: u16 = 7;
    let mut viewer = Viewer::new();
    let key = viewer.key;
    let language = PictureLanguage::Regis;
    let lines = 100 * usize::from(language.width());
    let record: Vec<u16> = (0..language.pixels()).map(|pixel| if pixel < lines { BUTTON } else { 0 }).collect();
    viewer.core.regis.hold(key, 0, 1, language, Some(RegisPick::of(&record, [(BUTTON, 0, 0)].into_iter(), language)));
    let pointer = |at: (u16, u16), pressed: bool| ToCore::RegisPointer(RegisPointer { key, at: Some(at), pressed, finger: false, span: 1 });
    let told = |viewer: &mut Viewer| -> Vec<RegisViewer> {
        viewer.decoder().into_iter().filter_map(|message| if let ToDecoder::RegisViewer { viewer, .. } = message { Some(viewer) } else { None }).collect()
    };
    // No script holds any input: the pointer over the drawing says nothing.
    viewer.core.intent(pointer((50, 50), false)).unwrap();
    assert_eq!(told(&mut viewer), []);
    viewer.core.intent(pointer((50, 300), false)).unwrap();
    // The page's script claims the pointer's presses.
    viewer.claims(&[], false, PointerClaim::Press, false);
    viewer.decoder();
    viewer.core.intent(pointer((50, 50), false)).unwrap();
    assert_eq!(told(&mut viewer), [RegisViewer { hovered: BUTTON, pressed: 0, typing: true }]);
    viewer.core.intent(pointer((50, 50), true)).unwrap();
    assert_eq!(told(&mut viewer), [RegisViewer { hovered: BUTTON, pressed: BUTTON, typing: true }]);
    // Pressed until the release, wherever the pointer goes meanwhile.
    viewer.core.intent(pointer((50, 300), true)).unwrap();
    assert_eq!(told(&mut viewer), [RegisViewer { hovered: 0, pressed: BUTTON, typing: true }]);
    viewer.core.intent(pointer((50, 300), false)).unwrap();
    assert_eq!(told(&mut viewer), [RegisViewer { hovered: 0, pressed: 0, typing: true }]);
}
