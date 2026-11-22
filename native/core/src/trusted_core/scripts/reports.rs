//! What a channel's decoder reports of its scripts, and what the trusted
//! core gives it for them (`neoteletext-common/docs/script-api.md`
//! sections 4.6, 11.1, 11.2 and 12.5; TODO-7293). A report is untrusted,
//! as everything a decoder says is.
//!
//! Built here: a script's claim of the network reaches the relay, which
//! passes a script host's requests only then; a decoder that stopped its
//! scripts has their script host ended; a script's log lines are the
//! channel's. The key claims are kept, and the verdicts, navigation and
//! titles given to the trusted core's gate (`script_gate.rs`, TODO-7294),
//! which decides what the viewer's input, the tab and the page list do
//! with them; a script's bytes are given by [`Core::give_script`], which
//! script loading calls (work package C2, TODO-7304).

use protocol::console::Level;
use protocol::decoder::ToDecoder;
use protocol::decoder::script::{ChannelFacts, ScriptClaims, ScriptOrder, ScriptReport, SourcePiece};
use protocol::rules::PageSub;
use protocol::script::{ContextId, MAX_SCRIPT_BYTES, MAX_SOURCE_PIECE_BYTES};

use super::super::Core;

/// The level of the developer log a script's line is shown at: the
/// script library's 0 is an error and 1 a warning; everything quieter is
/// information.
fn console_level(level: u8) -> Level {
    match level {
        0 => Level::Error,
        1 => Level::Warning,
        _ => Level::Info,
    }
}

impl Core {
    /// What `channel`'s scripts last claimed, as its decoder read their
    /// manifests; none when no script of it runs.
    #[cfg_attr(
        not(feature = "interaction"),
        expect(dead_code, reason = "read by the gate of a script's page (`script_gate.rs`), which a build without typing has none of")
    )]
    pub(in super::super) fn script_claims(&self, channel: u32) -> Option<&ScriptClaims> {
        self.scripts.claims.get(&channel)
    }

    /// A decoder's report of its channel's scripts.
    ///
    /// # Errors
    /// When the interface cannot be written to.
    #[cfg_attr(
        not(feature = "interaction"),
        expect(clippy::unnecessary_wraps, reason = "only the gate of a script's page writes to the interface, and a build without typing has none")
    )]
    pub(in super::super) fn script_reported(&mut self, channel: u32, report: ScriptReport) -> Result<(), String> {
        match report {
            ScriptReport::Claims(claims) => {
                // Section 11.1: the network kinds are relayed only for a
                // context the decoder reported with the network.
                self.script_network(channel, claims.running && claims.network);
                if claims.running {
                    self.scripts.claims.insert(channel, claims);
                } else {
                    self.scripts.claims.remove(&channel);
                }
            }
            // The decoder holds the authoritative rates and bounds
            // (section 10.5): a script host that passed one is ended, and
            // none is started until the channel is reloaded.
            ScriptReport::Stopped { reason, text } => {
                self.scripts.claims.remove(&channel);
                self.stop_scripts(channel, reason, &text);
            }
            ScriptReport::Log { level, context, text } => {
                let who = if context == ContextId::Channel { "channel script" } else { "page script" };
                self.channel_log(channel, console_level(level), None, &format!("{who}: {text}"));
            }
            // What became of an event the gate gave: its default is
            // taken unless a script cancelled it (section 4.1).
            #[cfg(feature = "interaction")]
            ScriptReport::Verdict { sequence, cancelled, handled } => self.script_verdict(channel, sequence, cancelled, handled)?,
            // A script's navigation, through the checks a link's goes
            // through (section 3.3).
            #[cfg(all(feature = "interaction", feature = "links"))]
            ScriptReport::Navigate { page, target } => self.script_navigate(channel, page, &target)?,
            // The page's title, where every page's title shows.
            #[cfg(feature = "interaction")]
            ScriptReport::Title { page, title } => self.script_titled(channel, page, &title)?,
            // A thing's label is the screen reader's, and nothing reads
            // one yet (section 18): it is bounded by its reader and kept
            // nowhere. In a build without typing no script is given the
            // viewer's input, and none of these has anything to act on.
            ScriptReport::Label { .. } => {}
            #[cfg(not(feature = "interaction"))]
            ScriptReport::Verdict { .. } | ScriptReport::Title { .. } => {}
            #[cfg(not(all(feature = "interaction", feature = "links")))]
            ScriptReport::Navigate { .. } => {}
        }
        Ok(())
    }

    /// Tells `channel`'s decoder what its scripts' contexts are told,
    /// before its first script.
    #[expect(dead_code, reason = "called by script loading, which work package C2 builds (TODO-7304)")]
    pub(in super::super) fn give_script_facts(&mut self, channel: u32, facts: ChannelFacts) {
        self.tell(channel, ToDecoder::Script(ScriptOrder::Channel(facts)));
    }

    /// Gives `channel`'s decoder a script as it was fetched or read,
    /// unread, in pieces: the channel's, or the one of `page`. One longer
    /// than a script may be is not given.
    #[expect(dead_code, reason = "called by script loading, which work package C2 builds (TODO-7304)")]
    pub(in super::super) fn give_script(&mut self, channel: u32, level: ContextId, page: PageSub, name: &str, bytes: &[u8]) {
        let Ok(total) = u32::try_from(bytes.len()) else { return };
        if bytes.is_empty() || bytes.len() > MAX_SCRIPT_BYTES {
            return;
        }
        let mut offset = 0_u32;
        for piece in bytes.chunks(MAX_SOURCE_PIECE_BYTES) {
            let source = SourcePiece { level, page, name: name.to_owned(), total, offset, bytes: piece.to_vec() };
            self.tell(channel, ToDecoder::Script(ScriptOrder::Source(source)));
            offset = offset.saturating_add(u32::try_from(piece.len()).unwrap_or(u32::MAX));
        }
    }
}
