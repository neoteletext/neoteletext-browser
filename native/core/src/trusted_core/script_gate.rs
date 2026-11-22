//! The trusted core's gate between the viewer and a page's program (the
//! script API's specification, `neoteletext-common/docs/script-api.md`,
//! principle 1's fifth boundary and sections 3.1, 3.3, 4.1, 4.2, 4.4 and
//! 4.6; DEC-7282 items 4 to 7; TODO-7294). A script is exactly as
//! untrusted as the channel that served it, and so is the decoder that
//! speaks for it: nothing here is decided by what either says beyond the
//! claims a manifest made, and those are held to what a script may ever
//! be given.
//!
//! - **What a script is given.** Only the keys its manifest names, typed
//!   text, the pointer and the wheel when it claims them, only on the
//!   open page of the active tab while the page has the keyboard, and
//!   never a key the browser keeps for itself
//!   (`interface_core::browser::scripts::may_be_scripts`): the interface
//!   decides the same before it sends a key, and this decides again.
//! - **Default actions.** A key down given to a script takes its default
//!   action unless the script cancelled it. The script's answer is
//!   waited for at most [`VERDICT_DEADLINE`]; a script that is stopped,
//!   or whose page is left, answers nothing and the default is taken at
//!   once, so a script never holds a key hostage (principle 5).
//! - **Navigation.** A script's `nt.navigate` is followed by the rules a
//!   link is, to a page of its own channel at most
//!   `MAX_NAVIGATIONS_PER_SECOND` times a second, and to another
//!   channel only while the viewer's own key press or pointer release is
//!   being handled (`navigate.rs`).
//! - **What the viewer sees.** The interface is told whether the open
//!   page's program runs, holds input or has stopped, for the trusted
//!   indications, and the viewer's own key stops it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use interface_core::browser::scripts::{ScriptRun, ScriptView, may_be_scripts};
use interface_core::canvas::PageKey;
use neoteletext::script_keys as keys;
use neoteletext_input::event::{Action, KeyCode, MouseAction, Point};
use protocol::ToInterface;
use protocol::console::Level;
use protocol::decoder::ToDecoder;
use protocol::decoder::script::{MAX_POINTERS, PointerClaim, PointerKind, ScriptClaims, ScriptOrder, ScriptPointer};
use protocol::interaction::script::KeyDefault;
use protocol::interaction::{InputIntent, Intent, TouchPhase, Typing as TypingState};
use protocol::rules::{KeyAction, KeyAsks, PageSub, PointerAction, RuleInput};
use protocol::script::relay::STEP_DEADLINE;
use protocol::script::{FailReason, MAX_EVENT_PAYLOAD_BYTES};

use super::Core;
use super::scripts::ScriptState;

#[cfg(feature = "links")]
mod navigate;
#[cfg(all(test, unix))]
mod tests;

/// How long a script's answer to a key is waited for before the key's
/// default action is taken without it (section 4.1: "If the context is
/// stopped, absent, or passes its time limit in that handler, the default
/// happens"): the time a script host has to answer a step, and as long
/// again for the decoder's report of it. A decoder reports an event its
/// script host never answered as not cancelled when the host is ended at
/// [`STEP_DEADLINE`]; this bound holds even of a decoder that reports
/// nothing.
pub(super) const VERDICT_DEADLINE: Duration = Duration::from_millis(500);
const _: () = assert!(VERDICT_DEADLINE.as_millis() == 2 * STEP_DEADLINE.as_millis());

/// The most events whose answer is waited for at once: what a decoder
/// queues for its scripts (section 4.7's `MAX_QUEUED_EVENTS`). One more
/// takes the oldest one's default at once.
pub(super) const MAX_PENDING_VERDICTS: usize = neoteletext_rules::limits::MAX_QUEUED_EVENTS;

/// The most keys followed as down on a script's page, for the release
/// that follows a press whatever the modifiers have become: the engine's
/// bound of keys held (section 3.4's `MAX_KEYS_DOWN`).
const MAX_KEYS_DOWN: usize = neoteletext_rules::limits::MAX_KEYS_DOWN;

/// How long after a script answered the viewer's key press or pointer
/// release its navigation to another channel is still that action's
/// (section 3.3: "only while a `keydown`, `pointerup` or component
/// activation of this step is being handled"). The answer and the
/// navigation leave the decoder together, in the report of one step, so
/// the time is only what a loaded machine may put between two messages
/// of one write: the time a step may take.
const ACTIVATION_WINDOW: Duration = STEP_DEADLINE;

/// The mouse's number among a page's pointers (section 4.2's
/// `pointerId`); the contacts of a touch screen take the numbers after it.
const MOUSE_POINTER: u8 = 1;
/// The most contacts of a touch screen followed at once: every pointer
/// but the mouse (section 4.4: "at most `MAX_POINTERS` pointers are
/// followed at once, and a further touch is not reported").
const MAX_CONTACTS: usize = (MAX_POINTERS - MOUSE_POINTER) as usize;

/// A unit of the wheel's turning in an intent: 1/120th of a line (DOC-1938).
const WHEEL_UNITS_PER_LINE: i32 = 120;

/// What of an event's default is still owed while its answer is waited for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Awaited {
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
struct Pending {
    sequence: u32,
    key: PageKey,
    awaited: Awaited,
    asked: Instant,
}

/// The gate's state.
#[derive(Default)]
pub(super) struct Gate {
    /// The number of the last event given to a decoder for its scripts.
    sequence: u32,
    /// The events whose answers are waited for, oldest first.
    pending: VecDeque<Pending>,
    /// The keys whose press was given to the open page's scripts and that
    /// have not come up, and the page they went to.
    down: Vec<(PageKey, KeyCode)>,
    /// The contacts of a touch screen given to the open page's scripts,
    /// each with its number among the pointers.
    contacts: Vec<(u64, u8)>,
    /// The channels in whose tab the viewer has pressed a key or the
    /// pointer on a script's page: their scripts may sound (DEC-7282
    /// item 7).
    acted: BTreeSet<u32>,
    /// When a script of each channel last answered the viewer's key press
    /// or pointer release as heard.
    activated: BTreeMap<u32, Instant>,
    /// When each channel's scripts asked to navigate, within the last
    /// second.
    navigations: BTreeMap<u32, VecDeque<Instant>>,
    /// What the interface was last told of the open page's program.
    told: Option<ScriptView>,
    /// What the open page's scripts held when the interface was last told
    /// the typing state for them: the page, its key asks and the wheel.
    told_held: Option<(PageKey, KeyAsks, bool)>,
    /// The page whose manifests' keys that no script is given were last
    /// logged, so that each page's are logged once.
    logged: Option<(u32, PageSub)>,
    /// The channel whose own script's navigation is being carried out.
    #[cfg(feature = "links")]
    navigating_itself: Option<u32>,
}

/// What a script of the open page is given, as the gate reads its claims.
struct Held<'a> {
    claims: &'a ScriptClaims,
    asks: KeyAsks,
}

impl Held<'_> {
    /// Whether the script is given any of the viewer's input: a key,
    /// text or the pointer (section 4.6: "holds any key or the pointer").
    fn holds(&self) -> bool {
        self.asks.text || !self.asks.keys.is_empty() || self.claims.pointer != PointerClaim::None
    }
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

impl Core {
    /// What the scripts of `key` are given, when `key` is the open page
    /// of the active tab, its scripts run, and their claims are for the
    /// page and sub-code its decoder was told is open. None for any other
    /// page: a script is given nothing off the open page.
    fn script_held(&self, key: PageKey) -> Option<Held<'_>> {
        if self.active_channel() != Some(key.channel) || self.typing.opened_page() != Some(key) || self.script_state(key.channel) != ScriptState::Running {
            return None;
        }
        let claims = self.script_claims(key.channel).filter(|claims| claims.running && claims.page.page == key.page)?;
        if self.typing.rules_open(key.channel) != Some(claims.page) {
            return None;
        }
        Some(Held { claims, asks: keys::asks_of(claims).0 })
    }

    /// Whether `key` is the open page and its scripts are given any of
    /// the viewer's input.
    pub(super) fn script_holds_input(&self, key: PageKey) -> bool {
        self.script_held(key).is_some_and(|held| held.holds())
    }

    /// The typing state the interface is told, with what the open page's
    /// scripts hold put in when the page is a script's (section 4.6): the
    /// keys their manifests name are then the page's, as a table's are
    /// (REQ-7037), on a channel that takes no input by itself (DOC-1794's
    /// amendment, section 14). A channel whose push connection takes
    /// input runs no script (DEC-7282 item 10), and its state is its own.
    pub(super) fn script_typing(&self, mut state: TypingState) -> TypingState {
        if state.on {
            return state;
        }
        let key = PageKey { channel: state.channel, page: state.page };
        let Some(held) = self.script_held(key).filter(Held::holds) else { return state };
        state.on = true;
        state.secure = self.session.all_tabs().find(|tab| tab.channel.id == key.channel).and_then(|tab| tab.address()).is_some_and(|address| address.encrypted);
        state.keys = held.asks.text || !held.asks.keys.is_empty();
        // The wheel over the page is the script's only when it claims it.
        state.wheel = held.claims.wheel;
        state.asks = held.claims.wheel;
        state.asked = held.asks;
        state
    }

    /// What the trusted indications say of the program of the active
    /// tab's open page (sections 3.1 and 4.6).
    fn script_view(&self) -> Option<ScriptView> {
        // The page open in the active tab; and while the trusted core is
        // told of none (between a tab chosen and its page reported open
        // again), the page the interface was last told of, so that a
        // program that stops then is not left shown as running
        // (TODO-7407).
        let active = self.active_channel()?;
        let told = self.script_gate.told.map(|view| PageKey { channel: view.channel, page: view.page });
        let key = self.typing.opened_page().or(told).filter(|key| key.channel == active)?;
        let run = match self.script_state(key.channel) {
            ScriptState::None => ScriptRun::None,
            ScriptState::Running => ScriptRun::Running,
            ScriptState::Stopped(_) => ScriptRun::Stopped,
        };
        let held = self.script_held(key);
        let holds = held.as_ref().is_some_and(Held::holds);
        let text = held.as_ref().is_some_and(|held| held.asks.text);
        Some(ScriptView { channel: key.channel, page: key.page, run, holds, text })
    }

    /// The number of the next event given to a decoder for its scripts.
    fn script_sequence(&mut self) -> u32 {
        self.script_gate.sequence = self.script_gate.sequence.wrapping_add(1);
        self.script_gate.sequence
    }

    /// Waits for the answer to an event just given, within the bound: one
    /// more than the bound takes the oldest one's default now.
    fn script_awaits(&mut self, sequence: u32, key: PageKey, awaited: Awaited) -> Result<(), String> {
        while self.script_gate.pending.len() >= MAX_PENDING_VERDICTS {
            let Some(oldest) = self.script_gate.pending.pop_front() else { break };
            self.script_default(&oldest)?;
        }
        self.script_gate.pending.push_back(Pending { sequence, key, awaited, asked: Instant::now() });
        Ok(())
    }

    /// Takes the default action of an event a script did not cancel: a
    /// key's goes to the interface, which acts on it as on a page with no
    /// program (section 4.2). The pointer's press and release have none
    /// here: a link or a control of the page is the browser's own and is
    /// never given to a script (RULE-51 item 3).
    fn script_default(&mut self, pending: &Pending) -> Result<(), String> {
        let Awaited::Key { code, typed } = pending.awaited else { return Ok(()) };
        self.in_window_of(pending.key.channel, |core| core.send(&ToInterface::KeyDefault(KeyDefault { key: pending.key, code, typed })))
    }

    /// The viewer acted on a script's page of `channel`: its scripts may
    /// sound from now on (section 3.5; DEC-7282 item 7).
    fn script_viewer_acted(&mut self, channel: u32) {
        if self.script_gate.acted.insert(channel) {
            self.tell(channel, ToDecoder::Script(ScriptOrder::SoundAllowed));
        }
    }

    /// An intent of the viewer for a page: taken here, and nowhere else,
    /// when the page is a script's. Returns whether it was a script's
    /// page.
    pub(super) fn script_input(&mut self, input: &InputIntent) -> Result<bool, String> {
        let Some(held) = self.script_held(input.key) else { return Ok(false) };
        let (page, text, pointer, wheel, asks) = (held.claims.page, held.asks.text, held.claims.pointer, held.claims.wheel, held.asks);
        // The page has the keyboard only while the window has it and no
        // other part of the browser has the keys (REQ-1554).
        if self.typing.keys_away() {
            return Ok(true);
        }
        let channel = input.key.channel;
        match &input.intent {
            Intent::Key { action, key, mods, text: typed, base } => {
                let followed = self.script_gate.down.contains(&(input.key, *key));
                // A release and a repeat follow their press, whatever the
                // modifiers have become; a press is held to what a script
                // may ever be given, and to what its manifest names.
                let named = if *action == Action::Press { may_be_scripts(*key, *mods) && keys::names(&asks, *key, *mods) } else { followed };
                if *action == Action::Press {
                    self.script_viewer_acted(channel);
                }
                if named {
                    self.script_key(input.key, page, *action, *key, *mods, *base, typed.as_deref())?;
                }
                // What the key typed is text for a script that claims
                // text, whether or not it names the key.
                let typed = typed.as_deref().filter(|_| *action != Action::Release && text && may_be_scripts(*key, *mods));
                if let Some(typed) = typed.filter(|typed| typed.chars().any(|character| !character.is_control())) {
                    let sequence = self.script_sequence();
                    self.tell(channel, ToDecoder::Script(ScriptOrder::Text { sequence, text: cut(typed, MAX_EVENT_PAYLOAD_BYTES).to_owned() }));
                }
            }
            Intent::Text(typed) | Intent::Paste(typed) => {
                if text && !typed.is_empty() {
                    let sequence = self.script_sequence();
                    self.tell(channel, ToDecoder::Script(ScriptOrder::Text { sequence, text: cut(typed, MAX_EVENT_PAYLOAD_BYTES).to_owned() }));
                }
            }
            Intent::Mouse { action, button, at } => {
                let action = match action {
                    MouseAction::Press => PointerAction::Press,
                    MouseAction::Release => PointerAction::Release,
                    MouseAction::Move => PointerAction::Move,
                    MouseAction::Gone => return Ok(true),
                };
                self.script_pointer(input.key, pointer, action, *button, at, (MOUSE_POINTER, PointerKind::Mouse, true))?;
            }
            Intent::Gone => {
                if pointer != PointerClaim::None {
                    let sequence = self.script_sequence();
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
                    self.tell(channel, ToDecoder::Script(ScriptOrder::Pointer(gone)));
                }
            }
            Intent::Touch { phase, contact, at } => self.script_touch(input.key, pointer, *phase, *contact, at)?,
            Intent::Wheel { dx, dy, at } => {
                let lines =
                    |units: i32| i16::try_from(units.checked_div(WHEEL_UNITS_PER_LINE).unwrap_or(0)).unwrap_or(if units < 0 { i16::MIN } else { i16::MAX });
                if let (true, Some((x, y))) = (wheel, dot(at, self.regis_pointing.at(input.key))) {
                    let sequence = self.script_sequence();
                    self.tell(channel, ToDecoder::Script(ScriptOrder::Wheel { sequence, x, y, delta_x: lines(*dx), delta_y: lines(*dy) }));
                }
            }
            // An input method's preedit is not committed text, and a
            // control of the page file is the browser's own.
            Intent::Compose { .. } | Intent::Hover { .. } => {}
        }
        // A script's answer is drawn by the page's next frame, which its
        // interval does not hold (REQ-RCH-002).
        self.frames.input(input.key);
        Ok(true)
    }

    /// Gives a key to the scripts of the open page, and waits for the
    /// answer to a key down.
    #[expect(clippy::too_many_arguments, reason = "a key event's fields, each named where it is read")]
    fn script_key(
        &mut self,
        key: PageKey,
        page: PageSub,
        action: Action,
        code: KeyCode,
        mods: u8,
        base: Option<char>,
        typed: Option<&str>,
    ) -> Result<(), String> {
        let down = &mut self.script_gate.down;
        match action {
            Action::Press if !down.contains(&(key, code)) => {
                // One more key than is followed is not given: its release
                // could not be.
                if down.len() >= MAX_KEYS_DOWN {
                    return Ok(());
                }
                down.push((key, code));
            }
            Action::Release => down.retain(|held| *held != (key, code)),
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
        let sequence = self.script_sequence();
        if action != Action::Release {
            let mut characters = typed.unwrap_or("").chars();
            let typed = characters.next().filter(|_| characters.next().is_none());
            self.script_awaits(sequence, key, Awaited::Key { code, typed })?;
        }
        let input = RuleInput::Key { action: action_told, named, code: number, base: base.map(u32::from), mods };
        self.tell(key.channel, ToDecoder::RuleInput { page, sequence, input });
        Ok(())
    }

    /// Gives a pointer's event to the scripts of the open page, when
    /// their manifests claim it, and waits for the answer to a press and
    /// a release.
    fn script_pointer(
        &mut self,
        key: PageKey,
        claim: PointerClaim,
        action: PointerAction,
        button: u8,
        at: &Point,
        which: (u8, PointerKind, bool),
    ) -> Result<(), String> {
        // A claim of presses includes the pointer coming onto a thing and
        // off it (section 3.1's manifest: "press, release, enter, leave
        // and cancel events"), which the decoder tells from where the
        // pointer is: so it is told the moves too, and gives a script a
        // move event only when a manifest claims moves.
        let Some((x, y)) = dot(at, self.regis_pointing.at(key)).filter(|_| claim >= PointerClaim::Press) else { return Ok(()) };
        let sequence = self.script_sequence();
        match action {
            PointerAction::Press => {
                self.script_viewer_acted(key.channel);
                self.script_awaits(sequence, key, Awaited::Press)?;
            }
            PointerAction::Release => self.script_awaits(sequence, key, Awaited::Release)?,
            PointerAction::Move | PointerAction::Gone => {}
        }
        let (pointer, kind, primary) = which;
        self.tell(key.channel, ToDecoder::Script(ScriptOrder::Pointer(ScriptPointer { sequence, action, button, x, y, pointer, kind, primary })));
        Ok(())
    }

    /// A contact of a touch screen on a script's page, as a pointer of
    /// its own with its number (section 4.4): at most [`MAX_CONTACTS`]
    /// are followed, and a further one is not reported. The first down is
    /// the primary one.
    fn script_touch(&mut self, key: PageKey, claim: PointerClaim, phase: TouchPhase, contact: u64, at: &Point) -> Result<(), String> {
        let contacts = &mut self.script_gate.contacts;
        let known = contacts.iter().position(|(id, _)| *id == contact);
        let place = match (phase, known) {
            (TouchPhase::Began, None) if contacts.len() < MAX_CONTACTS => {
                let taken: Vec<u8> = contacts.iter().map(|(_, number)| *number).collect();
                let Some(number) = (MOUSE_POINTER.saturating_add(1)..=MAX_POINTERS).find(|number| !taken.contains(number)) else { return Ok(()) };
                contacts.push((contact, number));
                contacts.len().saturating_sub(1)
            }
            (TouchPhase::Moved | TouchPhase::Ended | TouchPhase::Cancelled, Some(place)) => place,
            // A contact not followed, and a contact that began twice.
            _ => return Ok(()),
        };
        let Some(&(_, number)) = contacts.get(place) else { return Ok(()) };
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
        self.script_pointer(key, claim, action, button, at, (number, PointerKind::Touch, primary))
    }

    /// A decoder's report of what became of an event (section 4.1): the
    /// default of a key down is taken unless a script cancelled it. A
    /// number that names nothing waited for changes nothing, so a decoder
    /// can make no key act that the viewer did not press, and none twice.
    pub(super) fn script_verdict(&mut self, channel: u32, sequence: u32, cancelled: bool, handled: bool) -> Result<(), String> {
        let Some(place) = self.script_gate.pending.iter().position(|pending| pending.sequence == sequence && pending.key.channel == channel) else {
            return Ok(());
        };
        let Some(pending) = self.script_gate.pending.remove(place) else { return Ok(()) };
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
            self.script_gate.activated.insert(channel, Instant::now());
        }
        if cancelled { Ok(()) } else { self.script_default(&pending) }
    }

    /// A page context named its page (section 3.3): the title of the
    /// open page of a channel whose scripts run, bounded and cleaned as
    /// every page's title is, shown where every page's title is, in the
    /// page list, and never in the trusted strip.
    pub(super) fn script_titled(&mut self, channel: u32, page: PageSub, title: &str) -> Result<(), String> {
        if self.script_state(channel) != ScriptState::Running || self.typing.rules_open(channel) != Some(page) {
            return Ok(());
        }
        self.picture_titled(channel, page.page, title)
    }

    /// The viewer pressed the browser's own key that stops a page's
    /// program (section 4.6; DEC-7282 item 6): the scripts of the active
    /// tab's channel are stopped until it is reloaded, the last picture
    /// stays and every default is in force.
    pub(super) fn script_stop_asked(&mut self) {
        let Some(channel) = self.active_channel().filter(|channel| self.script_state(*channel) == ScriptState::Running) else { return };
        self.stop_scripts(channel, FailReason::Viewer, "the viewer stopped the page's program");
    }

    /// When the gate has timed work: the deadline of the oldest answer
    /// waited for.
    pub(super) fn script_gate_due_at(&self) -> Option<Instant> {
        self.script_gate.pending.iter().filter_map(|pending| pending.asked.checked_add(VERDICT_DEADLINE)).min()
    }

    /// The gate's work after every event and at its deadlines: an answer
    /// not come by [`VERDICT_DEADLINE`], and every answer owed by scripts
    /// that no longer run, is not waited for and the default is taken; what the page's scripts no longer hold
    /// is forgotten; and the interface is told what the trusted
    /// indications say when that changed.
    pub(super) fn script_gate_tick(&mut self, now: Instant) -> Result<(), String> {
        let open = self.typing.opened_page();
        let mut waiting = VecDeque::new();
        for pending in std::mem::take(&mut self.script_gate.pending) {
            let late = pending.asked.checked_add(VERDICT_DEADLINE).is_none_or(|deadline| now >= deadline);
            let gone = self.script_state(pending.key.channel) != ScriptState::Running;
            if late || gone {
                // The default is taken on the page the key was pressed on,
                // which the interface holds it to.
                self.script_default(&pending)?;
            } else {
                waiting.push_back(pending);
            }
        }
        self.script_gate.pending = waiting;
        let holding = open.filter(|key| self.script_held(*key).is_some());
        self.script_gate.down.retain(|(key, _)| Some(*key) == holding);
        if holding.is_none() {
            self.script_gate.contacts.clear();
        }
        // What is kept for a channel is forgotten when its scripts no
        // longer run: a reload makes a new channel.
        let gate = &self.script_gate;
        if !(gate.acted.is_empty() && gate.activated.is_empty() && gate.navigations.is_empty()) {
            let running: Vec<u32> =
                self.session.all_tabs().map(|tab| tab.channel.id).filter(|channel| self.script_state(*channel) == ScriptState::Running).collect();
            let gate = &mut self.script_gate;
            gate.acted.retain(|channel| running.contains(channel));
            gate.activated.retain(|channel, at| running.contains(channel) && now.saturating_duration_since(*at) <= ACTIVATION_WINDOW);
            gate.navigations.retain(|channel, _| running.contains(channel));
        }
        self.script_log_claims();
        let view = self.script_view();
        if view != self.script_gate.told {
            self.script_gate.told = view;
            // A page left has nothing said of it: the next page's word,
            // or none, replaces it in the interface.
            if let Some(view) = view {
                self.send(&ToInterface::Script(view))?;
            }
        }
        // What the page's scripts hold changed: the keys the interface
        // gives the page follow, by the typing state told again.
        let held = holding.and_then(|key| self.script_held(key)).filter(Held::holds).map(|held| (held.asks, held.claims.wheel));
        let held = holding.zip(held).map(|(key, (asks, wheel))| (key, asks, wheel));
        if held != self.script_gate.told_held {
            self.script_gate.told_held = held;
            let sends = self.typing.interface_state().map(neoteletext::typing::Send::Interface).into_iter().collect();
            self.deliver_typing(sends)?;
        }
        Ok(())
    }

    /// Logs once, for the channel's developer, the keys the open page's
    /// manifests name that no script is given (section 4.6: "A manifest
    /// that names one is not refused; the name is ignored and logged").
    fn script_log_claims(&mut self) {
        let Some(channel) = self.active_channel() else { return };
        let Some(claims) = self.script_claims(channel).filter(|claims| claims.running) else { return };
        if self.script_gate.logged == Some((channel, claims.page)) {
            return;
        }
        let (page, ignored) = (claims.page, keys::asks_of(claims).1.join(", "));
        self.script_gate.logged = Some((channel, page));
        if !ignored.is_empty() {
            let text = format!("page {:03X}: a script is never given these keys its manifest names: {ignored}", page.page);
            self.channel_log(channel, Level::Warning, None, &text);
        }
    }
}
