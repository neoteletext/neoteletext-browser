//! The trusted core as the gate of a page's rule table (neoteletext-common's
//! `docs/page-rules.md` RULE-52): what a decoder says of the open page's
//! table, the reports it asks for, and what the sound helper and the
//! fetcher say for it. The trusted core reads no table.

use protocol::decoder::FromDecoder;

use super::Core;

impl Core {
    /// A rule message from the decoder of `channel`, untrusted.
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "the messages are defined first; the gate of RULE-52 that handles them follows in its own commits"
    )]
    pub(super) fn rules_decoded(&mut self, channel: u32, message: FromDecoder) -> Result<(), String> {
        let _ = (channel, message);
        Ok(())
    }

    /// The sound helper's metered report for the view of `channel`
    /// (RULE-33 item 4).
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "the messages are defined first; the gate of RULE-52 that handles them follows in its own commits"
    )]
    pub(super) fn metered(&mut self, channel: u32, values: Vec<u8>) -> Result<(), String> {
        let _ = (channel, values);
        Ok(())
    }

    /// A state query the fetcher of `channel` received (RULE-47).
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "the messages are defined first; the gate of RULE-52 that handles them follows in its own commits"
    )]
    pub(super) fn rules_query(&mut self, channel: u32, page: u16, identifier: u32) -> Result<(), String> {
        let _ = (channel, page, identifier);
        Ok(())
    }
}
