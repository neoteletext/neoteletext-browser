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

use std::time::Instant;

use interface_core::browser::scripts::{ScriptRun, ScriptView};
use interface_core::canvas::PageKey;
use neoteletext::script_gate::{Claimed, Given};
use neoteletext::script_keys as keys;
use neoteletext_input::event::MouseAction;
use protocol::ToInterface;
use protocol::console::Level;
use protocol::decoder::script::{PointerClaim, ScriptClaims};
use protocol::interaction::script::KeyDefault;
use protocol::interaction::{InputIntent, Intent, Typing as TypingState};
use protocol::rules::{KeyAsks, PageSub};
use protocol::script::FailReason;

use super::Core;
use super::scripts::ScriptState;

#[cfg(feature = "links")]
mod navigate;
#[cfg(all(test, unix))]
mod tests;

/// The gate's own state and its decisions, which are the library's
/// (`neoteletext::script_gate`, where a test and a fuzz target reach them
/// with no process; rdm TODO-7439), and its time for an answer.
#[cfg(all(test, unix))]
use neoteletext::script_gate::VERDICT_DEADLINE;

/// The gate: what it decides of the viewer's input, and what the
/// interface was last told.
#[derive(Default)]
pub(super) struct Gate {
    /// The events given, the answers waited for and what the viewer has
    /// done, with every decision made of them.
    pub(super) input: neoteletext::script_gate::Gate,
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

    /// Takes the default action of a key no script cancelled: it goes to
    /// the interface, which acts on it as on a page with no program
    /// (section 4.2).
    fn script_default(&mut self, default: KeyDefault) -> Result<(), String> {
        self.in_window_of(default.key.channel, |core| core.send(&ToInterface::KeyDefault(default)))
    }

    /// An intent of the viewer for a page: taken here, and nowhere else,
    /// when the page is a script's. Returns whether it was a script's
    /// page. What the page's scripts are given of it is the gate's
    /// decision (`neoteletext::script_gate::Gate::input`).
    pub(super) fn script_input(&mut self, input: &InputIntent) -> Result<bool, String> {
        let Some(held) = self.script_held(input.key) else { return Ok(false) };
        let (page, pointer, wheel, asks) = (held.claims.page, held.claims.pointer, held.claims.wheel, held.asks);
        // The page has the keyboard only while the window has it and no
        // other part of the browser has the keys (REQ-1554).
        if self.typing.keys_away() {
            return Ok(true);
        }
        // The mouse gone from the page is told by the intent of its own
        // (`Intent::Gone`), and draws nothing.
        if matches!(input.intent, Intent::Mouse { action: MouseAction::Gone, .. }) {
            return Ok(true);
        }
        let claimed = Claimed { page, asks: &asks, pointer, wheel };
        let picture = self.regis_pointing.at(input.key);
        for given in self.script_gate.input.input(&claimed, input.key, &input.intent, picture, Instant::now()) {
            match given {
                Given::Decoder(message) => self.tell(input.key.channel, message),
                Given::Default(default) => self.script_default(default)?,
            }
        }
        // A script's answer is drawn by the page's next frame, which its
        // interval does not hold (REQ-RCH-002).
        self.frames.input(input.key);
        Ok(true)
    }

    /// A decoder's report of what became of an event (section 4.1): the
    /// default of a key down is taken unless a script cancelled it
    /// (`neoteletext::script_gate::Gate::verdict`).
    pub(super) fn script_verdict(&mut self, channel: u32, sequence: u32, cancelled: bool, handled: bool) -> Result<(), String> {
        match self.script_gate.input.verdict(channel, sequence, cancelled, handled, Instant::now()) {
            Some(default) => self.script_default(default),
            None => Ok(()),
        }
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
        self.script_gate.input.due_at()
    }

    /// The gate's work after every event and at its deadlines: an answer
    /// not come by [`VERDICT_DEADLINE`], and every answer owed by scripts
    /// that no longer run, is not waited for and the default is taken; what the page's scripts no longer hold
    /// is forgotten; and the interface is told what the trusted
    /// indications say when that changed.
    pub(super) fn script_gate_tick(&mut self, now: Instant) -> Result<(), String> {
        let open = self.typing.opened_page();
        // The default is taken on the page the key was pressed on, which
        // the interface holds it to.
        let mut gate = std::mem::take(&mut self.script_gate.input);
        let defaults = gate.expire(now, |channel| self.script_state(channel) == ScriptState::Running);
        self.script_gate.input = gate;
        for default in defaults {
            self.script_default(default)?;
        }
        let holding = open.filter(|key| self.script_held(*key).is_some());
        self.script_gate.input.held_by(holding);
        // What is kept for a channel is forgotten when its scripts no
        // longer run: a reload makes a new channel.
        if self.script_gate.input.keeps_channels() {
            let running: Vec<u32> =
                self.session.all_tabs().map(|tab| tab.channel.id).filter(|channel| self.script_state(*channel) == ScriptState::Running).collect();
            self.script_gate.input.forget_channels(&running, now);
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
