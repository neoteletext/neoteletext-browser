//! The trusted core as the gate of a page's rule table (neoteletext-common's
//! `docs/page-rules.md` RULE-52): what a decoder says of the open page's
//! table, the reports it asks for, and what the sound helper and the
//! fetcher say for it. The trusted core reads no table: the typing rules
//! (`neoteletext::typing`) hold the gate, and this passes its decisions on.
//! Rules bind a viewer's input, so they are built with typing.

use protocol::decoder::FromDecoder;

use super::Core;

#[cfg(feature = "interaction")]
impl Core {
    /// A rule message from the decoder of `channel`, untrusted.
    pub(super) fn rules_decoded(&mut self, channel: u32, message: FromDecoder) -> Result<(), String> {
        use protocol::audio::Audio;
        use protocol::console::Level;
        use protocol::sound::ToSound;
        let now_us = self.now_us();
        match message {
            // What the page's table asks for: its interest mask and wheel
            // rectangles stay with the gate; its watch list goes to the
            // sound helper, for the open page alone (RULE-33 item 2).
            FromDecoder::RulesOpen(told) => {
                let (sends, open) = self.typing.rules_open(channel, &told, now_us);
                let had = self.watching.remove(&channel);
                if open && !told.watch.is_empty() {
                    self.watching.insert(channel, (told.page, told.watch.len()));
                    self.tell_sound(&ToSound::Audio { channel, audio: Audio::Watch(told.watch) });
                } else if had.is_some() {
                    self.tell_sound(&ToSound::Audio { channel, audio: Audio::Watch(Vec::new()) });
                }
                self.deliver_typing(sends)?;
            }
            FromDecoder::Report(report) => {
                let sends = self.typing.report(channel, &report, now_us);
                self.deliver_typing(sends)?;
            }
            FromDecoder::PageState { page, identifier, body } => {
                let sends = self.typing.page_state(channel, page, identifier, &body, now_us);
                self.deliver_typing(sends)?;
            }
            // Passed to the page's channel, and written where the page's
            // author reads the channel's log (RULE-48, RULE-72).
            FromDecoder::RulesFault { page, generation, fault, count } => {
                let name = neoteletext_input::rules::FaultCode::from_code(fault).map_or("unknown", neoteletext_input::rules::FaultCode::name);
                self.note(Level::Warning, Some(channel), &format!("page {page:03X}: rule table fault {name}, {count} time(s), generation {generation}"));
                let sends = self.typing.rules_fault(channel, page, generation, fault, count, now_us);
                self.deliver_typing(sends)?;
            }
            // The receiver's own counts, for the page's author (RULE-72):
            // noted when something was skipped, dropped or overran since
            // they were last noted, and as the page's rules first run.
            FromDecoder::RuleCounters(counters) => {
                let troubles = (counters.skipped, counters.events_dropped, counters.overruns);
                let before = self.rule_troubles.insert(channel, (counters.page, troubles));
                if before != Some((counters.page, troubles)) {
                    self.note(
                        Level::Info,
                        Some(channel),
                        &format!(
                            "page {:03X} rules: {} ticks, {} fired, {} actions, {} skipped, {} events dropped, {} sends asked, {} overruns",
                            counters.page,
                            counters.ticks,
                            counters.fired,
                            counters.actions,
                            counters.skipped,
                            counters.events_dropped,
                            counters.sends,
                            counters.overruns
                        ),
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The sound helper's metered report for the view of `channel`
    /// (RULE-33 item 4): given to that view's decoder only at the length
    /// of the watch list passed for it; any other is dropped.
    pub(super) fn metered(&mut self, channel: u32, values: Vec<u8>) {
        let Some(&(page, length)) = self.watching.get(&channel) else { return };
        if values.len() == length {
            self.tell(channel, protocol::decoder::ToDecoder::Metered { page, values });
        }
    }

    /// A state query the fetcher of `channel` received (RULE-47): the
    /// decoder is asked only for the open page, and its one answer is
    /// passed only for a query noted here (RULE-52 item 6).
    pub(super) fn rules_query(&mut self, channel: u32, page: u16, identifier: u32) {
        if self.typing.rules_query(channel, page, identifier) {
            self.tell(channel, protocol::decoder::ToDecoder::RulesQuery { page, identifier });
        }
    }

    /// The sound helper stopped the local sounds of the view of `channel`
    /// for overrunning their time (RULE-62): fault 7 for the open page's
    /// table.
    pub(super) fn local_overrun(&mut self, channel: u32) -> Result<(), String> {
        /// Fault 7 (RULE-48).
        const FAULT_LOCAL_OVERRUN: u8 = 7;
        let Some((page, generation)) = self.typing.rules_asked(channel) else { return Ok(()) };
        self.note(protocol::console::Level::Warning, Some(channel), &format!("page {page:03X}: its local sounds were stopped for overrunning their time"));
        let sends = self.typing.rules_fault(channel, page, generation, FAULT_LOCAL_OVERRUN, 1, self.now_us());
        self.deliver_typing(sends)
    }

    /// The decoder of `channel` ended or is replaced: its page's table
    /// asks for nothing until the next decoder says so, and nothing is
    /// metered for it.
    pub(super) fn rules_decoder_gone(&mut self, channel: u32) {
        self.typing.decoder_gone(channel);
        self.rule_troubles.remove(&channel);
        if self.watching.remove(&channel).is_some() {
            self.tell_sound(&protocol::sound::ToSound::Audio { channel, audio: protocol::audio::Audio::Watch(Vec::new()) });
        }
    }
}

// Built without typing, no input is taken for any page, so no table is
// evaluated and none of these messages is acted on.
#[cfg(not(feature = "interaction"))]
#[expect(clippy::unused_self, clippy::unnecessary_wraps, reason = "the same calls as with typing, which do nothing without it")]
impl Core {
    pub(super) fn rules_decoded(&mut self, _channel: u32, _message: FromDecoder) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn metered(&mut self, _channel: u32, _values: Vec<u8>) {}

    pub(super) fn rules_query(&mut self, _channel: u32, _page: u16, _identifier: u32) {}

    pub(super) fn local_overrun(&mut self, _channel: u32) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn rules_decoder_gone(&mut self, _channel: u32) {}
}

#[cfg(all(test, unix, feature = "interaction"))]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]
mod tests;
