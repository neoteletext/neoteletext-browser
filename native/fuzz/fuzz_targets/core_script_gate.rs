//! The trusted core's gate between the viewer and a page's scripts
//! (script-api.md sections 3.3, 4.1, 4.2, 4.4 and 4.6; DEC-7282 items 4
//! to 7; `neoteletext::script_gate::Gate`, opened by TODO-7439): any
//! interleaving of the viewer's input, a decoder's answers (which a
//! compromised decoder may make up), the clock's steps, scripts' asks to
//! navigate and channels ending. Whatever the sequence:
//! - a key the browser keeps for itself is never given to a script, nor
//!   is what it typed;
//! - after the gate's own deadline work, no key's default is held past
//!   `VERDICT_DEADLINE`;
//! - a navigation is the viewer's own action (and so may leave the
//!   channel) only within `ACTIVATION_WINDOW` of a script hearing a real
//!   press or release, once for each, as an independent statement of the
//!   rule computes it;
//! - every bound of the gate holds.
#![no_main]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use interface_core::browser::scripts::may_be_scripts;
use interface_core::canvas::PageKey;
use libfuzzer_sys::fuzz_target;
use neoteletext::script_gate::{
    ACTIVATION_WINDOW, Awaited, Claimed, Gate, Given, MAX_CONTACTS, MAX_KEYS_DOWN, MAX_NAVIGATIONS_PER_SECOND, MAX_PENDING_VERDICTS, NAVIGATION_SECOND,
    Navigation, VERDICT_DEADLINE,
};
use neoteletext_input::event::{Action, KeyCode, MouseAction, Point};
use protocol::decoder::ToDecoder;
use protocol::decoder::script::{PointerClaim, ScriptOrder};
use protocol::interaction::{Intent, TouchPhase};
use protocol::rules::{Key, KeyAsk, KeyAsks, KeyKind, PageSub};

/// The most commands one input gives.
const MAX_COMMANDS: usize = 512;
/// The channels whose pages are open, in turn.
const CHANNELS: [u32; 3] = [4, 5, 6];
/// Keys of every kind: characters, keys a script may claim, and keys the
/// browser keeps (`NEVER_CLAIMABLE_KEYS`).
const NAMED: [&str; 10] = ["Enter", "ArrowUp", "F1", "Escape", "Tab", "BrowserBack", "MetaLeft", "F11", "ControlLeft", "Backspace"];
/// Characters a key types.
const CHARACTERS: [char; 5] = ['a', '1', '/', 'é', ' '];

/// Reads the input a piece at a time; past its end every read is zero.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((first, rest)) = self.0.split_first() else { return 0 };
        self.0 = rest;
        *first
    }

    fn code(&mut self) -> KeyCode {
        let choice = self.byte();
        if choice & 1 == 0 {
            KeyCode::Character(CHARACTERS[usize::from(choice >> 1) % CHARACTERS.len()])
        } else {
            neoteletext_input::keys::code(NAMED[usize::from(choice >> 1) % NAMED.len()]).map_or(KeyCode::Character('x'), KeyCode::Functional)
        }
    }

    fn point(&mut self) -> Point {
        let (column, row) = (u32::from(self.byte() % 40), u32::from(self.byte() % 25));
        let pixels = if self.byte() & 1 == 0 { None } else { Some((column * 12 + u32::from(self.byte() % 12), row * 20 + u32::from(self.byte() % 20))) };
        Point { column, row, pixels, mods: self.byte() }
    }

    fn intent(&mut self) -> Intent {
        match self.byte() % 9 {
            0..=2 => Intent::Key {
                action: [Action::Press, Action::Repeat, Action::Release][usize::from(self.byte() % 3)],
                key: self.code(),
                mods: self.byte(),
                text: if self.byte() & 1 == 0 { None } else { Some(CHARACTERS[usize::from(self.byte()) % CHARACTERS.len()].to_string()) },
                base: None,
            },
            3 => Intent::Text("typed".into()),
            4 => Intent::Mouse {
                action: [MouseAction::Press, MouseAction::Release, MouseAction::Move, MouseAction::Gone][usize::from(self.byte() % 4)],
                button: self.byte() % 3,
                at: self.point(),
            },
            5 => Intent::Gone,
            6 => Intent::Touch {
                phase: [TouchPhase::Began, TouchPhase::Moved, TouchPhase::Ended, TouchPhase::Cancelled][usize::from(self.byte() % 4)],
                contact: u64::from(self.byte() % 16),
                at: self.point(),
            },
            7 => Intent::Wheel { dx: i32::from(i8::from_ne_bytes([self.byte()])) * 40, dy: i32::from(i8::from_ne_bytes([self.byte()])) * 40, at: self.point() },
            _ => Intent::Paste("pasted".into()),
        }
    }
}

/// The asks a manifest may make, and some no manifest's reader makes
/// (any modifiers, any key), so that the gate's own refusal is reached.
fn asks_of(byte: u8) -> KeyAsks {
    let ask = |code: KeyCode, any_mods: bool| {
        let key = match code {
            KeyCode::Character(character) => Key::of(KeyKind::Character, u32::from(character)),
            KeyCode::Functional(number) => Key::of(KeyKind::Named, number),
        };
        KeyAsk { key: Some(key), mods_mask: if any_mods { 0 } else { interface_core::browser::scripts::BROWSER_CHORD_MODIFIERS }, mods_value: 0 }
    };
    let mut keys: Vec<KeyAsk> = CHARACTERS.iter().map(|&character| ask(KeyCode::Character(character), byte & 2 != 0)).collect();
    keys.extend(NAMED.iter().filter_map(|name| neoteletext_input::keys::code(name)).map(|code| ask(KeyCode::Functional(code), byte & 2 != 0)));
    if byte & 4 != 0 {
        keys.push(KeyAsk { key: None, mods_mask: 0, mods_value: 0 });
    }
    KeyAsks { text: byte & 1 != 0, keys }
}

/// Whether a decoder's answer to `awaited` is a script hearing the
/// viewer's own action (section 3.3): a key down but Escape, or a release.
fn is_action(awaited: Awaited) -> bool {
    let escape = neoteletext_input::keys::code("Escape").map(KeyCode::Functional);
    match awaited {
        Awaited::Key { code, .. } => Some(code) != escape,
        Awaited::Release => true,
        Awaited::Press => false,
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Bytes(data);
    let asks = asks_of(input.byte());
    let pointer = [PointerClaim::None, PointerClaim::Press, PointerClaim::Move][usize::from(input.byte() % 3)];
    let claimed = Claimed { page: PageSub::new(0x100, 0), asks: &asks, pointer, wheel: input.byte() & 1 != 0 };
    let began = Instant::now();
    let mut now = began;
    let mut gate = Gate::default();
    let mut channel = CHANNELS[0];
    let mut running: Vec<u32> = CHANNELS.to_vec();
    // When a script of each channel last heard the viewer's own action,
    // by the rule's own words.
    let mut heard: BTreeMap<u32, Instant> = BTreeMap::new();
    for _ in 0..MAX_COMMANDS {
        if input.0.is_empty() {
            break;
        }
        match input.byte() % 8 {
            0..=2 => {
                let intent = input.intent();
                let given = gate.input(&claimed, PageKey { channel, page: 0x100 }, &intent, None, now);
                if let Intent::Key { action: Action::Press, key, mods, .. } = &intent
                    && !may_be_scripts(*key, *mods)
                {
                    for one in &given {
                        assert!(
                            matches!(one, Given::Default(_) | Given::Decoder(ToDecoder::Script(ScriptOrder::SoundAllowed))),
                            "{key:?} with {mods:#04x} is the browser's, but the gate gave {one:?}"
                        );
                    }
                }
            }
            3 => {
                // A decoder's answer: to an event waited for, or made up.
                let sequence = if !gate.pending.is_empty() && input.byte() < 200 {
                    gate.pending[usize::from(input.byte()) % gate.pending.len()].sequence
                } else {
                    u32::from(input.byte())
                };
                let answering = if input.byte() & 7 == 0 { CHANNELS[usize::from(input.byte()) % CHANNELS.len()] } else { channel };
                let (cancelled, handled) = (input.byte() & 1 != 0, input.byte() & 1 != 0);
                let waited =
                    gate.pending.iter().find(|pending| pending.sequence == sequence && pending.key.channel == answering).map(|pending| pending.awaited);
                let _ = gate.verdict(answering, sequence, cancelled, handled, now);
                if handled && waited.is_some_and(is_action) {
                    heard.insert(answering, now);
                }
            }
            4 => {
                // The clock moves, and the gate does its timed work.
                now += Duration::from_millis(u64::from(input.byte()) * 8);
                let _ = gate.expire(now, |channel| running.contains(&channel));
                for pending in &gate.pending {
                    assert!(now < pending.asked + VERDICT_DEADLINE, "a default waited {:?}", now - pending.asked);
                }
                assert!(gate.due_at().is_none_or(|due| due <= now + VERDICT_DEADLINE));
            }
            5 => {
                let asking = if input.byte() & 3 == 0 { CHANNELS[usize::from(input.byte()) % CHANNELS.len()] } else { channel };
                let counted =
                    gate.navigations.get(&asking).map_or(0, |asked| asked.iter().filter(|at| now.saturating_duration_since(**at) < NAVIGATION_SECOND).count());
                match gate.navigation_asked(asking, now) {
                    Navigation::TooMany => assert!(counted >= MAX_NAVIGATIONS_PER_SECOND, "refused with {counted} asked in the second"),
                    Navigation::Asked { activated } => {
                        let expected = heard.remove(&asking).is_some_and(|at| now.saturating_duration_since(at) <= ACTIVATION_WINDOW);
                        assert_eq!(activated, expected, "a navigation of channel {asking} at {:?}", now - began);
                    }
                }
            }
            6 => {
                // Another page is opened, or the open one's scripts end.
                channel = CHANNELS[usize::from(input.byte()) % CHANNELS.len()];
                gate.held_by(if input.byte() & 1 == 0 { Some(PageKey { channel, page: 0x100 }) } else { None });
            }
            _ => {
                let ended = CHANNELS[usize::from(input.byte()) % CHANNELS.len()];
                if input.byte() & 1 == 0 {
                    running.retain(|one| *one != ended);
                } else if !running.contains(&ended) {
                    running.push(ended);
                }
                gate.forget_channels(&running, now);
                heard.retain(|channel, _| running.contains(channel));
            }
        }
        assert!(gate.pending.len() <= MAX_PENDING_VERDICTS, "{} answers waited for", gate.pending.len());
        assert!(gate.down.len() <= MAX_KEYS_DOWN, "{} keys down", gate.down.len());
        assert!(gate.contacts.len() <= MAX_CONTACTS, "{} contacts", gate.contacts.len());
        assert!(gate.down.iter().all(|(_, code)| may_be_scripts(*code, 0)), "a key the browser keeps is held down for a script: {:?}", gate.down);
        assert!(gate.navigations.values().all(|asked| asked.len() <= MAX_NAVIGATIONS_PER_SECOND));
    }
});
