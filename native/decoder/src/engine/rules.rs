//! The engine's part in the open page's rule table (page-rules.md
//! RULE-58): the calls the decoder's loop makes for it.

use neoteletext_rules::limits::SUBCODE_ANY;
use protocol::decoder::{FromDecoder, ToDecoder};
use protocol::rules::RuleInput;

use super::Engine;
use crate::rules::host::Parts;

/// How far right of the 800-pixel picture's left edge a NAPLPS picture
/// begins (RULE-13: a NAPLPS picture is 640 by 480, shown in the middle
/// of the 800 by 480 picture the trusted core gives the pointer in).
const NAPLPS_LEFT: u16 = (interface_core::canvas::REGIS_WIDTH - NAPLPS_WIDTH) / 2;
/// A NAPLPS picture's width in pixels (RULE-13).
const NAPLPS_WIDTH: u16 = neoteletext_web::records::naplps_objects::PICTURE_WIDTH;
const _: () = assert!(NAPLPS_WIDTH as usize == crate::naplps::WIDTH && NAPLPS_LEFT == 80);

/// A pixel of the 800-wide picture as a pixel of the NAPLPS picture in
/// its middle. A pixel of either margin is outside the picture and is
/// given as the first column past its right edge, which no rectangle (cut
/// to the picture, RULE-13) and no object holds: the press, release or
/// move is still followed (RULE-8), on nothing.
const fn naplps_x(x: u16) -> u16 {
    match x.checked_sub(NAPLPS_LEFT) {
        Some(inside) if inside < NAPLPS_WIDTH => inside,
        _ => NAPLPS_WIDTH,
    }
}

/// A rule input of a NAPLPS page, with its pointer in the NAPLPS
/// picture's pixels.
const fn naplps_input(input: RuleInput) -> RuleInput {
    match input {
        RuleInput::Pointer { action, button, x, y, control } => RuleInput::Pointer { action, button, x: naplps_x(x), y, control },
        RuleInput::Wheel { x, y, control } => RuleInput::Wheel { x: naplps_x(x), y, control },
        other => other,
    }
}

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
        let facts = self.picture_facts(page);
        let Self { rules, regis, naplps, animation, .. } = self;
        let input = if facts.is_naplps(regis, naplps) { naplps_input(input) } else { input };
        let picture = facts.picture(regis, naplps, animation);
        rules.input(page, sequence, input, &picture);
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
        // What the animation's repaints of the last wake cost (ANIM-16):
        // each held back for the budget is told as fault 8 (RULE-48).
        let slowed = self.take_animation_counters(page).slowed;
        let facts = self.picture_facts(page);
        let Self { rules, audio, sounds, heard, regis, naplps, animation, .. } = self;
        rules.animation_slowed(page, slowed);
        // The picture of the sub-page on screen: a rule's actions show,
        // hide, move, re-look and cycle its drawings, objects or sprites,
        // and its conditions read their bounds (RULE-12, RULE-26, ANIM-6).
        let mut picture = facts.picture(regis, naplps, animation);
        let mut parts = Parts { audio: &mut **audio, sounds, heard, picture: &mut picture, sub };
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
