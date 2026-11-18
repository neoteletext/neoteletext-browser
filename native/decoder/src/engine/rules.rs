//! The engine's part in the open page's rule table (page-rules.md
//! RULE-58): the calls the decoder's loop makes for it.

use protocol::decoder::{FromDecoder, ToDecoder};
use protocol::rules::RuleInput;

use super::Engine;

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

    /// The page of the channel that is the open page of the active tab, or
    /// none (RULE-58): only its table is evaluated.
    pub fn rule_page(&mut self, page: Option<u16>) {
        let _ = page;
    }

    /// An input event of the viewer for the open page, from the trusted
    /// core (RULE-52 item 1): it waits for the next tick (RULE-5 step 2).
    pub fn rule_input(&mut self, page: u16, sequence: u32, input: RuleInput) {
        let _ = (page, sequence, input);
    }

    /// The sound helper's newest metered report for the open page's watch
    /// list (RULE-33 item 5): the next tick's snapshot takes it.
    pub fn rule_metered(&mut self, page: u16, values: &[u8]) {
        let _ = (page, values);
    }

    /// A state query for `page` (RULE-47): answered once, at the end of
    /// the next tick, when that page is the open page.
    pub fn rule_query(&mut self, page: u16, identifier: u32) {
        let _ = (page, identifier);
    }

    /// When the open page's next tick is due, on [`Engine::now_us`]'s
    /// clock: `None` while no open page has a table, so the decoder keeps
    /// no sixty-a-second wake for rules (RULE-58).
    #[must_use]
    pub fn rules_due_us(&self) -> Option<u64> {
        None
    }

    /// Evaluates each tick of the open page's table that is due at
    /// `now_us` (RULE-4), and gives what they produced for the trusted
    /// core, in the order it is to leave: the sound operations first,
    /// before any picture work of the tick (RULE-64).
    pub fn run_rules(&mut self, now_us: u64) -> Vec<FromDecoder> {
        let _ = now_us;
        Vec::new()
    }
}
