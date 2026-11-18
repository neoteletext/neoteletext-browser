//! The typing rules' part in a page's rule table (page-rules.md RULE-52):
//! what a decoder says of the open page's table, and the reports, state
//! answers and faults it asks this core to pass to the page's channel.

use interface_core::canvas::PageKey;
use protocol::rules::{Report, RulesOpen};

use super::{Send, Typing, together};

impl Typing {
    /// Whether `page` of `channel` is the open page of the active tab,
    /// with its channel's session welcomed: the one page whose table is
    /// evaluated and whose reports may go (RULE-52 item 4, RULE-58).
    fn rules_page_open(&self, channel: u32, page: u16) -> bool {
        self.on(channel) && self.opened == Some(PageKey { channel, page })
    }

    /// The page of `channel` that is the open page of the active tab, if
    /// one is: what a decoder just started is told (RULE-58).
    #[must_use]
    pub fn rules_page(&self, channel: u32) -> Option<u16> {
        self.opened.filter(|opened| opened.channel == channel && self.active == Some(channel)).map(|opened| opened.page)
    }

    /// The decoder of `channel` said what the table of one of its pages
    /// asks for (RULE-52 item 1): taken only for the open page. A page
    /// whose table is newly taken while it holds the keyboard is given the
    /// focus, so that its `focus` rule knows (RULE-8). The watch list the
    /// trusted core may pass to the sound helper: the list told, for the
    /// open page, else none.
    pub fn rules_open(&mut self, channel: u32, told: &RulesOpen, now_us: u64) -> (Vec<Send>, bool) {
        let mut sends = Vec::new();
        let open = self.rules_page_open(channel, told.page);
        let Some(state) = self.channels.get_mut(&channel) else { return (sends, false) };
        let before = state.gate.wheel_here();
        state.rules_told(told, open);
        state.give_focus_owed(channel, now_us, &mut sends);
        if state.gate.wheel_here() != before {
            self.tell_interface(&mut sends);
        }
        (together(sends), open && told.generation != 0)
    }

    /// A report the decoder of `channel` asks for (RULE-52 items 3 to 5).
    pub fn report(&mut self, channel: u32, report: &Report, now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        // Where: on that page's own channel, whose session is welcomed.
        if !self.on_session(channel) {
            return sends;
        }
        if let Some(state) = self.channels.get_mut(&channel) {
            state.report(channel, report, now_us, &mut sends);
        }
        together(sends)
    }

    /// Whether `channel`'s input socket is open with its session welcomed.
    fn on_session(&self, channel: u32) -> bool {
        self.channels.get(&channel).is_some_and(|state| state.open && state.session.welcomed())
    }

    /// A state query the fetcher of `channel` received for `page`
    /// (RULE-47): whether the decoder is to be asked, which is only for
    /// the open page.
    pub fn rules_query(&mut self, channel: u32, page: u16, identifier: u32) -> bool {
        if !self.rules_page_open(channel, page) {
            return false;
        }
        if let Some(state) = self.channels.get_mut(&channel) {
            state.gate.query(page, identifier);
        }
        true
    }

    /// The decoder's answer to a state query (RULE-52 item 6).
    pub fn page_state(&mut self, channel: u32, page: u16, identifier: u32, body: &[u8], now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        if !self.rules_page_open(channel, page) {
            return sends;
        }
        if let Some(state) = self.channels.get_mut(&channel) {
            state.page_state(channel, page, identifier, body, now_us, &mut sends);
        }
        sends
    }

    /// A fault of a page's table (RULE-48), from its decoder or from the
    /// sound helper: passed to the channel for the open page, at most once
    /// a second for each code.
    pub fn rules_fault(&mut self, channel: u32, page: u16, generation: u16, fault: u8, count: u16, now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        if !self.rules_page_open(channel, page) {
            return sends;
        }
        if let Some(state) = self.channels.get_mut(&channel) {
            state.fault(channel, page, generation, fault, count, now_us, &mut sends);
        }
        sends
    }

    /// The table the open page of `channel` is asked for, if any: its page
    /// and generation.
    #[must_use]
    pub fn rules_asked(&self, channel: u32) -> Option<(u16, u16)> {
        self.channels.get(&channel)?.gate.asked.as_ref().map(|asked| (asked.page, asked.generation))
    }

    /// How many reports the decoder of `channel` asked for that named no
    /// event it could report (RULE-52 item 3): what a faulty or hostile
    /// decoder leaves behind, for the trusted core's log.
    #[must_use]
    pub fn rules_refused(&self, channel: u32) -> u32 {
        self.channels.get(&channel).map_or(0, |state| state.gate.refused)
    }

    /// What the trusted core derived for the pointer event about to be
    /// given for `channel` (RULE-52 item 1): the pixel of the page's
    /// picture, when a renderer other than teletext's draws it, and the
    /// teletext control under its cell, numbered from 1, or 0.
    pub fn pointer_context(&mut self, channel: u32, pixel: Option<(u16, u16)>, control: u16) {
        if let Some(state) = self.channels.get_mut(&channel) {
            state.gate.pixel = pixel;
            state.gate.control = control;
        }
    }

    /// The decoder of `channel` ended or was replaced: what it said its
    /// page asks for is forgotten, until its next decoder says it.
    pub fn decoder_gone(&mut self, channel: u32) {
        if let Some(state) = self.channels.get_mut(&channel) {
            state.gate.forget();
        }
    }
}
