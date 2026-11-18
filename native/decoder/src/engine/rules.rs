//! The engine's part in the open page's rule table (page-rules.md
//! RULE-58): the calls the decoder's loop makes for it.

use neoteletext_rules::limits::SUBCODE_ANY;
use protocol::decoder::{FromDecoder, ToDecoder};
use protocol::rules::RuleInput;

use super::Engine;
use crate::rules::host::Parts;
use crate::rules::{Picture, Still};

/// A teletext page's picture, in pixels (RULE-13).
const TELETEXT_PICTURE: (u16, u16) = (interface_core::canvas::PAGE_WIDTH, interface_core::canvas::PAGE_HEIGHT);
/// A ReGIS picture, in pixels (RULE-13).
const REGIS_PICTURE: (u16, u16) = (interface_core::canvas::REGIS_WIDTH, interface_core::canvas::REGIS_HEIGHT);
/// A NAPLPS picture, in pixels (RULE-13).
const NAPLPS_PICTURE: (u16, u16) = (640, 480);
const _: () = assert!(NAPLPS_PICTURE.0 as usize == crate::naplps::WIDTH && NAPLPS_PICTURE.1 as usize == crate::naplps::HEIGHT);

impl Engine {
    /// Takes a message of the open page's rule table (RULE-63) and gives
    /// it back; any other message is given back untouched. The loop then
    /// has nothing more to do for a rule message: it waits for the next
    /// tick.
    pub fn rule_message(&mut self, message: ToDecoder) -> ToDecoder {
        match &message {
            ToDecoder::RuleInput { page, sequence, input } => self.rule_input(*page, *sequence, *input),
            ToDecoder::Metered { page, values } => self.rule_metered(*page, values),
            ToDecoder::RulesQuery { page, identifier } => self.rule_query(*page, *identifier),
            ToDecoder::RulesPage(page) => self.rule_page(*page),
            _ => {}
        }
        message
    }

    /// The picture of `page` as its rule table sees it (RULE-12, RULE-13):
    /// the one place the evaluator's host obtains it. Every page's picture
    /// is still, of its renderer's size, until its animation state is
    /// given here.
    fn rules_picture(&mut self, page: u16) -> &mut dyn Picture {
        let size = if self.regis.has(page) {
            REGIS_PICTURE
        } else if self.naplps.has(page) {
            NAPLPS_PICTURE
        } else {
            TELETEXT_PICTURE
        };
        self.still = Still::new(size);
        &mut self.still
    }

    /// The sub-code of the sub-page of `page` shown, as the evaluator
    /// takes it: 0 while the store holds no sub-page of it.
    fn rules_subcode(&self, page: u16) -> u16 {
        self.shown_subcode(page).unwrap_or(0).min(SUBCODE_ANY)
    }

    /// The page of the channel that is the open page of the active tab, or
    /// none (RULE-58): only its table is evaluated.
    pub fn rule_page(&mut self, page: Option<u16>) {
        self.rules.set_page(page);
    }

    /// An input event of the viewer for the open page, from the trusted
    /// core (RULE-52 item 1): it waits for the next tick (RULE-5 step 2).
    pub fn rule_input(&mut self, page: u16, sequence: u32, input: RuleInput) {
        if self.rules.open_page() != Some(page) {
            return;
        }
        self.rules_picture(page);
        self.rules.input(page, sequence, input, &self.still);
    }

    /// The sound helper's newest metered report for the open page's watch
    /// list (RULE-33 item 5): the next tick's snapshot takes it.
    pub fn rule_metered(&mut self, page: u16, values: &[u8]) {
        self.rules.metered(page, values);
    }

    /// A state query for `page` (RULE-47): answered once, at the end of
    /// the next tick, when that page is the open page.
    pub fn rule_query(&mut self, page: u16, identifier: u32) {
        let now_us = self.now_us();
        self.rules.query(page, identifier, now_us);
    }

    /// When the open page's next tick is due, on [`Engine::now_us`]'s
    /// clock: `None` while no open page has a table, so the decoder keeps
    /// no sixty-a-second wake for rules (RULE-58).
    #[must_use]
    pub fn rules_due_us(&self) -> Option<u64> {
        self.rules.due_us()
    }

    /// Evaluates each tick of the open page's table that is due at
    /// `now_us` (RULE-4), and gives what they produced for the trusted
    /// core, in the order it is to leave: a rule's MIDI messages first,
    /// before any picture work of the tick (RULE-64).
    pub fn run_rules(&mut self, now_us: u64) -> Vec<FromDecoder> {
        let Some(page) = self.rules.page() else {
            // No page of the channel is the open page: an evaluator still
            // running ends here.
            return self.run_rules_for(0, now_us);
        };
        self.run_rules_for(page, now_us)
    }

    fn run_rules_for(&mut self, page: u16, now_us: u64) -> Vec<FromDecoder> {
        let sub = self.rules_subcode(page);
        self.rules_picture(page);
        let Self { rules, audio, sounds, heard, still, .. } = self;
        let mut parts = Parts { audio: &mut **audio, sounds, heard, picture: still, sub };
        let mut produced = rules.run(now_us, &mut parts);
        // What a rule's `play`, `stop` or `gain` made of the channel's
        // sounds follows at once, with what the tick reported.
        if !produced.is_empty() {
            produced.extend(sounds.drain(..).map(FromDecoder::Audio));
        }
        produced
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::arithmetic_side_effects, reason = "tests")]
mod tests;
