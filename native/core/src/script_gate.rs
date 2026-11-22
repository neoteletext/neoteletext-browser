//! What the trusted core's gate between the viewer and a page's program
//! decides (the script API's specification,
//! `neoteletext-common/docs/script-api.md`, sections 3.3, 4.1, 4.2, 4.4
//! and 4.6; DEC-7282 items 4 to 7; TODO-7294): which of the viewer's
//! input a script is given, the answers waited for and the defaults
//! taken without them, and whether a script's navigation is the viewer's
//! own action. The gate's state and its decisions are here, in the
//! library, with no process, no pipe and no clock of their own, so that
//! any sequence of input, answers and asks can be given to them by a
//! test or a fuzz target (rdm TODO-7439). The trusted core's program
//! (`trusted_core/script_gate.rs`) says what page is open and what its
//! scripts claim, and writes what the gate decides.
//!
//! What the gate holds to, whatever a decoder reports:
//! - a key the browser keeps for itself
//!   (`interface_core::browser::scripts::may_be_scripts`) is never given
//!   to a script, and a key is given only when a manifest names it;
//! - a key down given to a script takes its default action unless a
//!   script cancelled it, and no later than [`VERDICT_DEADLINE`] after it
//!   was given; at most [`MAX_PENDING_VERDICTS`] answers are waited for;
//! - an answer that names nothing waited for changes nothing;
//! - a script's navigation is the viewer's own action only within
//!   [`ACTIVATION_WINDOW`] of a script hearing the viewer's key press or
//!   pointer release, once for each, and at most
//!   [`MAX_NAVIGATIONS_PER_SECOND`] are asked in a second.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use interface_core::browser::scripts::may_be_scripts;
use interface_core::canvas::PageKey;
use neoteletext_input::event::{Action, KeyCode, MouseAction, Point};
use protocol::decoder::ToDecoder;
use protocol::decoder::script::{MAX_POINTERS, PointerClaim, PointerKind, ScriptOrder, ScriptPointer};
use protocol::interaction::script::KeyDefault;
use protocol::interaction::{Intent, TouchPhase};
use protocol::rules::{KeyAction, KeyAsks, PageSub, PointerAction, RuleInput};
use protocol::script::MAX_EVENT_PAYLOAD_BYTES;
use protocol::script::relay::STEP_DEADLINE;

use crate::script_keys as keys;

/// How long a script's answer to a key is waited for before the key's
/// default action is taken without it (section 4.1: "If the context is
/// stopped, absent, or passes its time limit in that handler, the default
/// happens"): the time a script host has to answer a step, and as long
/// again for the decoder's report of it. A decoder reports an event its
/// script host never answered as not cancelled when the host is ended at
/// [`STEP_DEADLINE`]; this bound holds even of a decoder that reports
/// nothing.
pub const VERDICT_DEADLINE: Duration = Duration::from_millis(500);
const _: () = assert!(VERDICT_DEADLINE.as_millis() == 2 * STEP_DEADLINE.as_millis());

/// The most events whose answer is waited for at once: what a decoder
/// queues for its scripts (section 4.7's `MAX_QUEUED_EVENTS`). One more
/// takes the oldest one's default at once.
pub const MAX_PENDING_VERDICTS: usize = neoteletext_rules::limits::MAX_QUEUED_EVENTS;

/// The most keys followed as down on a script's page, for the release
/// that follows a press whatever the modifiers have become: the engine's
/// bound of keys held (section 3.4's `MAX_KEYS_DOWN`).
pub const MAX_KEYS_DOWN: usize = neoteletext_rules::limits::MAX_KEYS_DOWN;

/// How long after a script answered the viewer's key press or pointer
/// release its navigation to another channel is still that action's
/// (section 3.3: "only while a `keydown`, `pointerup` or component
/// activation of this step is being handled"). The answer and the
/// navigation leave the decoder together, in the report of one step, so
/// the time is only what a loaded machine may put between two messages
/// of one write: the time a step may take.
pub const ACTIVATION_WINDOW: Duration = STEP_DEADLINE;

/// The mouse's number among a page's pointers (section 4.2's
/// `pointerId`); the contacts of a touch screen take the numbers after it.
pub const MOUSE_POINTER: u8 = 1;
/// The most contacts of a touch screen followed at once: every pointer
/// but the mouse (section 4.4: "at most `MAX_POINTERS` pointers are
/// followed at once, and a further touch is not reported").
pub const MAX_CONTACTS: usize = (MAX_POINTERS - MOUSE_POINTER) as usize;

/// A unit of the wheel's turning in an intent: 1/120th of a line (DOC-1938).
const WHEEL_UNITS_PER_LINE: i32 = 120;

/// The most navigations a channel's scripts may ask in any second
/// (sections 3.3 and 10.5's `MAX_NAVIGATIONS_PER_SECOND`). The decoder
/// holds the same bound over its script host, counted in ticks
/// (`decoder::scripts::limits`, which this crate does not link); a
/// test holds the two equal.
pub const MAX_NAVIGATIONS_PER_SECOND: usize = 2;

/// The second navigations are counted over.
pub const NAVIGATION_SECOND: Duration = Duration::from_secs(1);

/// What of an event's default is still owed while its answer is waited for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Awaited {
    /// A key down: its default is taken unless a script cancelled it.
    Key {
        /// The key.
        code: KeyCode,
        /// What it typed, when that was one character.
        typed: Option<char>,
    },
    /// A pointer's press: nothing is owed for it.
    Press,
    /// A pointer's release: nothing is owed for it, and with a key down
    /// it is what lets a script leave its channel.
    Release,
}

/// An event given to a script whose answer has not come.
#[derive(Clone, Copy, Debug)]
pub struct Pending {
    /// The event's number, as the decoder was given it.
    pub sequence: u32,
    /// The page it was given on.
    pub key: PageKey,
    /// What is owed for it.
    pub awaited: Awaited,
    /// When it was given.
    pub asked: Instant,
}

impl Pending {
    /// The default action owed for the event when no script cancelled
    /// it: a key's, which the interface acts on as on a page with no
    /// program (section 4.2). The pointer's press and release have none
    /// here: a link or a control of the page is the browser's own and is
    /// never given to a script (RULE-51 item 3).
    #[must_use]
    pub const fn default_owed(&self) -> Option<KeyDefault> {
        match self.awaited {
            Awaited::Key { code, typed } => Some(KeyDefault { key: self.key, code, typed }),
            Awaited::Press | Awaited::Release => None,
        }
    }
}

/// What the scripts of the open page are given, as the trusted core
/// reads their claims.
#[derive(Clone, Copy, Debug)]
pub struct Claimed<'a> {
    /// The page and sub-code the claims are for.
    pub page: PageSub,
    /// The keys the manifests name, held to what a script may be given
    /// (`script_keys::asks_of`), and whether text is claimed.
    pub asks: &'a KeyAsks,
    /// The pointer's claim.
    pub pointer: PointerClaim,
    /// Whether the wheel is claimed.
    pub wheel: bool,
}

/// What the gate decided of one input, in the order it is to be done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Given {
    /// The channel's decoder is told this, for its scripts.
    Decoder(ToDecoder),
    /// The interface takes this key's default action: its answer is no
    /// longer waited for.
    Default(KeyDefault),
}

/// What became of a script's asking to navigate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation {
    /// More were asked in the second than may be: it is not followed.
    TooMany,
    /// It goes on to the checks a link's goes through.
    Asked {
        /// Whether it is the viewer's own action, which alone may leave
        /// the channel: one action lets a script leave once.
        activated: bool,
    },
}

/// The gate's state. Its fields are open to read, so that a test and a
/// fuzz target can hold every bound of it.
#[derive(Debug, Default)]
pub struct Gate {
    /// The number of the last event given to a decoder for its scripts.
    pub sequence: u32,
    /// The events whose answers are waited for, oldest first.
    pub pending: VecDeque<Pending>,
    /// The keys whose press was given to the open page's scripts and that
    /// have not come up, and the page they went to.
    pub down: Vec<(PageKey, KeyCode)>,
    /// The contacts of a touch screen given to the open page's scripts,
    /// each with its number among the pointers.
    pub contacts: Vec<(u64, u8)>,
    /// The channels in whose tab the viewer has pressed a key or the
    /// pointer on a script's page: their scripts may sound (DEC-7282
    /// item 7).
    pub acted: BTreeSet<u32>,
    /// When a script of each channel last answered the viewer's key press
    /// or pointer release as heard.
    pub activated: BTreeMap<u32, Instant>,
    /// When each channel's scripts asked to navigate, within the last
    /// second.
    pub navigations: BTreeMap<u32, VecDeque<Instant>>,
}

/// The dot of a page's picture an intent's point is at, within a `u16`.
fn dot(at: &Point, picture: Option<(u16, u16)>) -> Option<(u16, u16)> {
    picture.or_else(|| at.pixels.and_then(|(x, y)| Some((u16::try_from(x).ok()?, u16::try_from(y).ok()?))))
}

/// `text` cut to at most `limit` bytes, at a character's end.
fn cut(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or("")
}

impl Gate {
    /// The number of the next event given to a decoder for its scripts.
    const fn next_sequence(&mut self) -> u32 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    /// Waits for the answer to an event just given, within the bound: one
    /// more than the bound takes the oldest one's default now.
    fn awaits(&mut self, sequence: u32, key: PageKey, awaited: Awaited, now: Instant, given: &mut Vec<Given>) {
        while self.pending.len() >= MAX_PENDING_VERDICTS {
            let Some(oldest) = self.pending.pop_front() else { break };
            given.extend(oldest.default_owed().map(Given::Default));
        }
        self.pending.push_back(Pending { sequence, key, awaited, asked: now });
    }

    /// The viewer acted on a script's page of `channel`: its scripts may
    /// sound from now on (section 3.5; DEC-7282 item 7).
    fn viewer_acted(&mut self, channel: u32, given: &mut Vec<Given>) {
        if self.acted.insert(channel) {
            given.push(Given::Decoder(ToDecoder::Script(ScriptOrder::SoundAllowed)));
        }
    }

    /// An intent of the viewer for `key`, the open page of the active
    /// tab, whose scripts run and claim `claimed`, while the page has
    /// the keyboard: what its decoder is told for its scripts, and the
    /// defaults taken at once, in order. `picture` is where the pointer
    /// is in the page's picture, when the trusted core follows it there.
    pub fn input(&mut self, claimed: &Claimed<'_>, key: PageKey, intent: &Intent, picture: Option<(u16, u16)>, now: Instant) -> Vec<Given> {
        let mut given = Vec::new();
        let text = claimed.asks.text;
        let channel = key.channel;
        let tell = |order: ScriptOrder| Given::Decoder(ToDecoder::Script(order));
        match intent {
            Intent::Key { action, key: code, mods, text: typed, base } => {
                let followed = self.down.contains(&(key, *code));
                // A release and a repeat follow their press, whatever the
                // modifiers have become; a press is held to what a script
                // may ever be given, and to what its manifest names.
                let named = if *action == Action::Press { may_be_scripts(*code, *mods) && keys::names(claimed.asks, *code, *mods) } else { followed };
                if *action == Action::Press {
                    self.viewer_acted(channel, &mut given);
                }
                if named {
                    self.key(key, claimed.page, (*action, *code, *mods, *base), typed.as_deref(), now, &mut given);
                }
                // What the key typed is text for a script that claims
                // text, whether or not it names the key.
                let typed = typed.as_deref().filter(|_| *action != Action::Release && text && may_be_scripts(*code, *mods));
                if let Some(typed) = typed.filter(|typed| typed.chars().any(|character| !character.is_control())) {
                    let sequence = self.next_sequence();
                    given.push(tell(ScriptOrder::Text { sequence, text: cut(typed, MAX_EVENT_PAYLOAD_BYTES).to_owned() }));
                }
            }
            Intent::Text(typed) | Intent::Paste(typed) => {
                if text && !typed.is_empty() {
                    let sequence = self.next_sequence();
                    given.push(tell(ScriptOrder::Text { sequence, text: cut(typed, MAX_EVENT_PAYLOAD_BYTES).to_owned() }));
                }
            }
            Intent::Mouse { action, button, at } => {
                let action = match action {
                    MouseAction::Press => PointerAction::Press,
                    MouseAction::Release => PointerAction::Release,
                    MouseAction::Move => PointerAction::Move,
                    MouseAction::Gone => return given,
                };
                self.pointer(key, claimed.pointer, (action, *button), dot(at, picture), (MOUSE_POINTER, PointerKind::Mouse, true), now, &mut given);
            }
            Intent::Gone => {
                if claimed.pointer != PointerClaim::None {
                    let sequence = self.next_sequence();
                    let gone = ScriptPointer {
                        sequence,
                        action: PointerAction::Gone,
                        button: 0,
                        x: 0,
                        y: 0,
                        pointer: MOUSE_POINTER,
                        kind: PointerKind::Mouse,
                        primary: true,
                    };
                    given.push(tell(ScriptOrder::Pointer(gone)));
                }
            }
            Intent::Touch { phase, contact, at } => self.touch(key, claimed.pointer, *phase, *contact, dot(at, picture), now, &mut given),
            Intent::Wheel { dx, dy, at } => {
                let lines =
                    |units: i32| i16::try_from(units.checked_div(WHEEL_UNITS_PER_LINE).unwrap_or(0)).unwrap_or(if units < 0 { i16::MIN } else { i16::MAX });
                if let (true, Some((x, y))) = (claimed.wheel, dot(at, picture)) {
                    let sequence = self.next_sequence();
                    given.push(tell(ScriptOrder::Wheel { sequence, x, y, delta_x: lines(*dx), delta_y: lines(*dy) }));
                }
            }
            // An input method's preedit is not committed text, and a
            // control of the page file is the browser's own.
            Intent::Compose { .. } | Intent::Hover { .. } => {}
        }
        given
    }

    /// Gives a key to the scripts of the open page, and waits for the
    /// answer to a key down.
    fn key(
        &mut self,
        key: PageKey,
        page: PageSub,
        (action, code, mods, base): (Action, KeyCode, u8, Option<char>),
        typed: Option<&str>,
        now: Instant,
        given: &mut Vec<Given>,
    ) {
        match action {
            Action::Press if !self.down.contains(&(key, code)) => {
                // One more key than is followed is not given: its release
                // could not be.
                if self.down.len() >= MAX_KEYS_DOWN {
                    return;
                }
                self.down.push((key, code));
            }
            Action::Release => self.down.retain(|held| *held != (key, code)),
            Action::Press | Action::Repeat => {}
        }
        let (named, number) = match code {
            KeyCode::Character(character) => (false, u32::from(character)),
            KeyCode::Functional(number) => (true, number),
        };
        let action_told = match action {
            Action::Press => KeyAction::Press,
            Action::Repeat => KeyAction::Repeat,
            Action::Release => KeyAction::Release,
        };
        let sequence = self.next_sequence();
        if action != Action::Release {
            let mut characters = typed.unwrap_or("").chars();
            let typed = characters.next().filter(|_| characters.next().is_none());
            self.awaits(sequence, key, Awaited::Key { code, typed }, now, given);
        }
        let input = RuleInput::Key { action: action_told, named, code: number, base: base.map(u32::from), mods };
        given.push(Given::Decoder(ToDecoder::RuleInput { page, sequence, input }));
    }

    /// Gives a pointer's event to the scripts of the open page, when
    /// their manifests claim it, and waits for the answer to a press and
    /// a release.
    #[expect(clippy::too_many_arguments, reason = "a pointer event's fields, each named where it is read")]
    fn pointer(
        &mut self,
        key: PageKey,
        claim: PointerClaim,
        (action, button): (PointerAction, u8),
        at: Option<(u16, u16)>,
        which: (u8, PointerKind, bool),
        now: Instant,
        given: &mut Vec<Given>,
    ) {
        // A claim of presses includes the pointer coming onto a thing and
        // off it (section 3.1's manifest: "press, release, enter, leave
        // and cancel events"), which the decoder tells from where the
        // pointer is: so it is told the moves too, and gives a script a
        // move event only when a manifest claims moves.
        let Some((x, y)) = at.filter(|_| claim >= PointerClaim::Press) else { return };
        let sequence = self.next_sequence();
        match action {
            PointerAction::Press => {
                self.viewer_acted(key.channel, given);
                self.awaits(sequence, key, Awaited::Press, now, given);
            }
            PointerAction::Release => self.awaits(sequence, key, Awaited::Release, now, given),
            PointerAction::Move | PointerAction::Gone => {}
        }
        let (pointer, kind, primary) = which;
        given.push(Given::Decoder(ToDecoder::Script(ScriptOrder::Pointer(ScriptPointer { sequence, action, button, x, y, pointer, kind, primary }))));
    }

    /// A contact of a touch screen on a script's page, as a pointer of
    /// its own with its number (section 4.4): at most [`MAX_CONTACTS`]
    /// are followed, and a further one is not reported. The first down is
    /// the primary one.
    #[expect(clippy::too_many_arguments, reason = "a contact's fields, each named where it is read")]
    fn touch(&mut self, key: PageKey, claim: PointerClaim, phase: TouchPhase, contact: u64, at: Option<(u16, u16)>, now: Instant, given: &mut Vec<Given>) {
        let contacts = &mut self.contacts;
        let known = contacts.iter().position(|(id, _)| *id == contact);
        let place = match (phase, known) {
            (TouchPhase::Began, None) if contacts.len() < MAX_CONTACTS => {
                let taken: Vec<u8> = contacts.iter().map(|(_, number)| *number).collect();
                let Some(number) = (MOUSE_POINTER.saturating_add(1)..=MAX_POINTERS).find(|number| !taken.contains(number)) else { return };
                contacts.push((contact, number));
                contacts.len().saturating_sub(1)
            }
            (TouchPhase::Moved | TouchPhase::Ended | TouchPhase::Cancelled, Some(place)) => place,
            // A contact not followed, and a contact that began twice.
            _ => return,
        };
        let Some(&(_, number)) = contacts.get(place) else { return };
        let primary = place == 0;
        // A contact's press is its main button, and a move holds it
        // (W3C Pointer Events: a touch contact is button 0, buttons 1).
        let (action, button) = match phase {
            TouchPhase::Began => (PointerAction::Press, 0),
            TouchPhase::Moved => (PointerAction::Move, 1),
            TouchPhase::Ended => (PointerAction::Release, 0),
            TouchPhase::Cancelled => (PointerAction::Gone, 0),
        };
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            contacts.remove(place);
        }
        self.pointer(key, claim, (action, button), at, (number, PointerKind::Touch, primary), now, given);
    }

    /// A decoder's report of what became of an event (section 4.1): the
    /// default of a key down is owed unless a script cancelled it, and is
    /// what is returned. A number that names nothing waited for on
    /// `channel` changes nothing, so a decoder can make no key act that
    /// the viewer did not press, and none twice.
    pub fn verdict(&mut self, channel: u32, sequence: u32, cancelled: bool, handled: bool, now: Instant) -> Option<KeyDefault> {
        let place = self.pending.iter().position(|pending| pending.sequence == sequence && pending.key.channel == channel)?;
        let pending = self.pending.remove(place)?;
        // A script heard the viewer's own action: what it asks in the same
        // step may leave the channel (section 3.3).
        // Escape is never that action, as on the web (HTML's activation
        // triggering input events leave it out): it is the key a viewer
        // presses to get away.
        let escape = neoteletext_input::keys::code("Escape").map(KeyCode::Functional);
        let acted = match pending.awaited {
            Awaited::Key { code, .. } => Some(code) != escape,
            Awaited::Release => true,
            Awaited::Press => false,
        };
        if handled && acted {
            self.activated.insert(channel, now);
        }
        if cancelled { None } else { pending.default_owed() }
    }

    /// When the gate has timed work: the deadline of the oldest answer
    /// waited for.
    #[must_use]
    pub fn due_at(&self) -> Option<Instant> {
        self.pending.iter().filter_map(|pending| pending.asked.checked_add(VERDICT_DEADLINE)).min()
    }

    /// Ceases to wait, at `now`, for every answer not come by
    /// [`VERDICT_DEADLINE`] and every answer owed by scripts that no
    /// longer run (`running` says whether a channel's do): the defaults
    /// owed for them, oldest first, each taken on the page its key was
    /// pressed on.
    pub fn expire(&mut self, now: Instant, running: impl Fn(u32) -> bool) -> Vec<KeyDefault> {
        let mut defaults = Vec::new();
        let mut waiting = VecDeque::new();
        for pending in std::mem::take(&mut self.pending) {
            let late = pending.asked.checked_add(VERDICT_DEADLINE).is_none_or(|deadline| now >= deadline);
            if late || !running(pending.key.channel) {
                defaults.extend(pending.default_owed());
            } else {
                waiting.push_back(pending);
            }
        }
        self.pending = waiting;
        defaults
    }

    /// Forgets what the page's scripts no longer hold: the keys down on
    /// any page but `holding`, the open page whose scripts are given
    /// input, and every contact when there is none.
    pub fn held_by(&mut self, holding: Option<PageKey>) {
        self.down.retain(|(key, _)| Some(*key) == holding);
        if holding.is_none() {
            self.contacts.clear();
        }
    }

    /// Whether anything is kept for a channel, which
    /// [`Self::forget_channels`] would look through.
    #[must_use]
    pub fn keeps_channels(&self) -> bool {
        !(self.acted.is_empty() && self.activated.is_empty() && self.navigations.is_empty())
    }

    /// Forgets what is kept for a channel whose scripts no longer run (a
    /// reload makes a new channel), and an action of the viewer's older
    /// than [`ACTIVATION_WINDOW`] at `now`.
    pub fn forget_channels(&mut self, running: &[u32], now: Instant) {
        self.acted.retain(|channel| running.contains(channel));
        self.activated.retain(|channel, at| running.contains(channel) && now.saturating_duration_since(*at) <= ACTIVATION_WINDOW);
        self.navigations.retain(|channel, _| running.contains(channel));
    }

    /// The scripts of `channel` asked to navigate at `now` (section
    /// 3.3): counted whether followed or not, at most
    /// [`MAX_NAVIGATIONS_PER_SECOND`] in a second, and the viewer's own
    /// action only within [`ACTIVATION_WINDOW`] of a script hearing it,
    /// once.
    pub fn navigation_asked(&mut self, channel: u32, now: Instant) -> Navigation {
        let asked = self.navigations.entry(channel).or_default();
        while asked.front().is_some_and(|at| now.saturating_duration_since(*at) >= NAVIGATION_SECOND) {
            asked.pop_front();
        }
        if asked.len() >= MAX_NAVIGATIONS_PER_SECOND {
            return Navigation::TooMany;
        }
        asked.push_back(now);
        // One action of the viewer's lets a script leave once.
        let activated = self.activated.remove(&channel).is_some_and(|at| now.saturating_duration_since(at) <= ACTIVATION_WINDOW);
        Navigation::Asked { activated }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::panic, reason = "a test")]
    use neoteletext_input::event::CONTROL;
    use protocol::rules::{Key, KeyAsk, KeyKind};

    use super::*;

    const PAGE: PageKey = PageKey { channel: 4, page: 0x100 };

    /// A press of `code` with `mods` on [`PAGE`], which types nothing.
    fn press(code: KeyCode, mods: u8) -> Intent {
        Intent::Key { action: Action::Press, key: code, mods, text: None, base: None }
    }

    /// Asks that name `code` whatever the modifiers, as no manifest's
    /// asks do: the gate's own refusal is what is tested.
    fn asks_naming(codes: &[KeyCode]) -> KeyAsks {
        let ask = |code: &KeyCode| {
            let key = match *code {
                KeyCode::Character(character) => Key::of(KeyKind::Character, u32::from(character)),
                KeyCode::Functional(number) => Key::of(KeyKind::Named, number),
            };
            KeyAsk { key: Some(key), mods_mask: 0, mods_value: 0 }
        };
        KeyAsks { text: false, keys: codes.iter().map(ask).collect() }
    }

    fn keys_given(given: &[Given]) -> usize {
        given.iter().filter(|given| matches!(given, Given::Decoder(ToDecoder::RuleInput { .. }))).count()
    }

    /// Sections 4.1 and 4.6, with no trusted core around the gate (rdm
    /// TODO-7439): a key the browser keeps is never given whatever the
    /// asks name; a key given is waited for no longer than
    /// [`VERDICT_DEADLINE`], and its default is taken once, by the answer
    /// or by the deadline; an answer that names nothing waited for, or
    /// another channel's, changes nothing; and one more answer than may
    /// be waited for takes the oldest one's default at once.
    #[test]
    fn a_key_the_browser_keeps_is_never_given_and_a_default_is_never_held_past_its_deadline() {
        let back = KeyCode::Functional(neoteletext_input::keys::code("BrowserBack").unwrap());
        let a = KeyCode::Character('a');
        let asks = asks_naming(&[back, a]);
        let claimed = Claimed { page: PageSub::new(0x100, 0), asks: &asks, pointer: PointerClaim::None, wheel: false };
        let began = Instant::now();
        let mut gate = Gate::default();
        // The viewer's first press lets the scripts sound, and that is all
        // the decoder is told of a key that is the browser's.
        assert_eq!(gate.input(&claimed, PAGE, &press(back, 0), None, began), vec![Given::Decoder(ToDecoder::Script(ScriptOrder::SoundAllowed))]);
        assert_eq!(keys_given(&gate.input(&claimed, PAGE, &press(a, CONTROL), None, began)), 0);
        assert!(gate.pending.is_empty() && gate.down.is_empty());

        assert_eq!(keys_given(&gate.input(&claimed, PAGE, &press(a, 0), None, began)), 1);
        let sequence = gate.sequence;
        assert_eq!(gate.due_at(), Some(began + VERDICT_DEADLINE));
        assert_eq!(gate.verdict(PAGE.channel + 1, sequence, false, true, began), None, "another channel's decoder answers nothing of this one's");
        assert_eq!(gate.verdict(PAGE.channel, sequence + 1, false, true, began), None, "a number never given");
        let just_before = (began + VERDICT_DEADLINE).checked_sub(Duration::from_millis(1)).unwrap();
        assert_eq!(gate.expire(just_before, |_| true), Vec::new());
        let default = KeyDefault { key: PAGE, code: a, typed: None };
        assert_eq!(gate.expire(began + VERDICT_DEADLINE, |_| true), vec![default]);
        assert_eq!((gate.verdict(PAGE.channel, sequence, false, true, began), gate.due_at()), (None, None), "taken once");

        // Scripts that no longer run are not waited for at all.
        let release = Intent::Key { action: Action::Release, key: a, mods: 0, text: None, base: None };
        gate.input(&claimed, PAGE, &release, None, began);
        gate.input(&claimed, PAGE, &press(a, 0), None, began);
        assert_eq!(gate.expire(began, |_| false), vec![default]);

        // One more than may be waited for: the oldest one's default now.
        let repeat = Intent::Key { action: Action::Repeat, key: a, mods: 0, text: None, base: None };
        for _ in 0..MAX_PENDING_VERDICTS {
            assert!(!gate.input(&claimed, PAGE, &repeat, None, began).contains(&Given::Default(default)));
        }
        assert!(gate.input(&claimed, PAGE, &repeat, None, began).contains(&Given::Default(default)));
        assert_eq!(gate.pending.len(), MAX_PENDING_VERDICTS);
    }

    /// Section 3.3 (rdm TODO-7439): a script's navigation is the viewer's
    /// own action only when a script heard the viewer's key press within
    /// [`ACTIVATION_WINDOW`], once for each press, never for a press no
    /// script heard, never for Escape; and at most
    /// [`MAX_NAVIGATIONS_PER_SECOND`] are asked in a second.
    #[test]
    fn a_navigation_is_the_viewers_action_once_and_only_soon_after_a_script_heard_it() {
        let (a, escape) = (KeyCode::Character('a'), KeyCode::Functional(neoteletext_input::keys::code("Escape").unwrap()));
        let asks = asks_naming(&[a, escape]);
        let claimed = Claimed { page: PageSub::new(0x100, 0), asks: &asks, pointer: PointerClaim::None, wheel: false };
        let began = Instant::now();
        let mut gate = Gate::default();
        let heard = |gate: &mut Gate, code: KeyCode, handled: bool, at: Instant| {
            gate.down.clear();
            gate.input(&claimed, PAGE, &press(code, 0), None, at);
            let sequence = gate.sequence;
            gate.verdict(PAGE.channel, sequence, true, handled, at);
        };
        assert_eq!(gate.navigation_asked(PAGE.channel, began), Navigation::Asked { activated: false }, "no action of the viewer's");
        heard(&mut gate, a, true, began);
        assert_eq!(gate.navigation_asked(PAGE.channel, began), Navigation::Asked { activated: true });
        assert_eq!(gate.navigation_asked(PAGE.channel, began), Navigation::TooMany, "the third in a second");
        let later = began + NAVIGATION_SECOND;
        assert_eq!(gate.navigation_asked(PAGE.channel, later), Navigation::Asked { activated: false }, "one action, one leaving");
        heard(&mut gate, a, false, later);
        heard(&mut gate, escape, true, later);
        assert_eq!(gate.navigation_asked(PAGE.channel, later), Navigation::Asked { activated: false }, "a press no script heard, and Escape");
        let last = later + NAVIGATION_SECOND;
        heard(&mut gate, a, true, last);
        let late = last + ACTIVATION_WINDOW + Duration::from_millis(1);
        assert_eq!(gate.navigation_asked(PAGE.channel, late), Navigation::Asked { activated: false }, "the action is long over");
        gate.forget_channels(&[], late);
        assert!(!gate.keeps_channels());
    }
}
