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

use neoteletext_core::consts::MAX_SCRIPT_NAME;
use protocol::clean;
use protocol::console::Level;
use protocol::decoder::ToDecoder;
use protocol::decoder::script::{ChannelFacts, ScriptClaims, ScriptOrder, ScriptReport, SourcePiece};
use protocol::fetcher::ToFetcher;
use protocol::rules::PageSub;
use protocol::script::{ContextId, MAX_SCRIPT_BYTES, MAX_SOURCE_PIECE_BYTES};

/// Seconds in a minute: a zone's offset is told in minutes.
const SECONDS_PER_MINUTE: i32 = 60;

use super::super::Core;

/// The most page scripts kept for each channel in case its decoder is
/// replaced: the pages a viewer has most lately opened. With the
/// channel's own script, at a megabyte each at most, a channel's kept
/// scripts are bounded.
const MAX_PAGE_SCRIPTS_KEPT: usize = 8;

/// A script kept as its decoder was given it.
pub(super) struct Kept {
    level: ContextId,
    page: PageSub,
    pieces: Vec<SourcePiece>,
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
                self.script_stop_explained(channel, reason, &text);
                self.stop_scripts(channel, reason, &text);
            }
            ScriptReport::Log { level, context, text } => {
                let who = if context == ContextId::Channel { "channel script" } else { "page script" };
                self.channel_log(channel, protocol::script::batch::console_level(level), None, &format!("{who}: {text}"));
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
            ScriptReport::Wanted { page, name } => self.script_wanted(channel, page, &name),
        }
        Ok(())
    }

    /// A page file of `channel` names a script its decoder does not hold
    /// (section 12.5 step 2). The name is the decoder's word and
    /// untrusted: it is asked of the channel's fetcher only when it is a
    /// script name and the engine's one rule says the file of that name
    /// in the channel's own folder may be loaded (`scripts::may_load`,
    /// DEC-7264 item 5). A live channel runs no script (DEC-7282 item 10).
    fn script_wanted(&mut self, channel: u32, page: PageSub, name: &str) {
        let Some(script) = neoteletext_core::types::ScriptName::parse(name.as_bytes()) else {
            return self.log(Level::Warning, Some(channel), "a page file names a script by what is not a script's name; it is not loaded");
        };
        let Some(tab) = self.session.all_tabs().find(|tab| tab.channel.id == channel) else { return };
        let Some(web) = tab.channel.web.clone() else {
            // A file opened from disk (section 12.7; DEC-7315 version 2):
            // its script is read by the broker from the file's own folder
            // and nowhere else, and runs at once. The script itself has no
            // access to files.
            let Some(file) = tab.address().and_then(|address| address.file.clone()) else { return };
            return match self.broker().and_then(|broker| broker.read_script(&file, name)) {
                Ok(bytes) => self.give_script(channel, ContextId::Page, page, name, &bytes),
                Err(reason) => {
                    let name = clean(name, MAX_SCRIPT_NAME);
                    self.log(
                        Level::Warning,
                        Some(channel),
                        &format!("the script {name} a file names could not be read from its folder ({reason}); the page runs without it"),
                    );
                    self.tell(channel, ToDecoder::Script(ScriptOrder::NoScript(page)));
                }
            };
        };
        let allowed = neoteletext_web::url::Url::parse(&web).is_ok_and(|folder| {
            neoteletext_web::scripts::script_url(&folder, script).is_ok_and(|address| neoteletext_web::scripts::may_load(&folder, script, &address))
        });
        if !allowed {
            return self.log(Level::Warning, Some(channel), "a page file names a script that is not in its channel's own folder; it is not loaded");
        }
        self.tell_fetcher(channel, &ToFetcher::FetchScript { page, name: name.to_owned() });
    }

    /// Tells `channel`'s decoder what its scripts' contexts are told,
    /// once for each decoder, before its first script: the channel's
    /// address and name, whether it is a file opened from disk, and the
    /// time and zone, which a confined decoder cannot read.
    fn script_facts_once(&mut self, channel: u32) {
        let Some(decoder) = self.decoders.get(&channel).map(super::super::processes::Decoder::generation) else { return };
        if self.scripts.told.get(&channel) == Some(&decoder) {
            return;
        }
        let Some(facts) = self.script_facts(channel) else { return };
        self.scripts.told.insert(channel, decoder);
        self.give_script_facts(channel, facts);
    }

    /// What `channel`'s scripts' contexts are told.
    fn script_facts(&self, channel: u32) -> Option<ChannelFacts> {
        let tab = self.session.all_tabs().find(|tab| tab.channel.id == channel)?;
        let time = self.local_time();
        Some(ChannelFacts {
            // A file's own address is no folder a script's address is made
            // from: a file page's origin is `null` (DEC-7315 item 3).
            channel_url: tab.channel.web.clone().unwrap_or_default(),
            channel_title: clean(tab.channel.name.as_deref().unwrap_or(&tab.channel.title), protocol::script::MAX_TITLE_BYTES),
            from_disk: tab.channel.web.is_none(),
            epoch_seconds: u64::try_from(time.utc).unwrap_or(0),
            zone_minutes: i16::try_from(time.offset.checked_div(SECONDS_PER_MINUTE).unwrap_or(0)).unwrap_or(0),
        })
    }

    /// What `channel`'s fetcher said of a script: a piece of one it
    /// fetched, or that it could not fetch one.
    pub(in super::super) fn script_message(&mut self, channel: u32, message: protocol::fetcher::FromFetcher) {
        match message {
            protocol::fetcher::FromFetcher::Script(piece) => self.script_fetched(channel, piece),
            protocol::fetcher::FromFetcher::ScriptFailed { level, page, name } => {
                // No script comes for the page: its refusal stands.
                self.refusal_stands(channel, page.page);
                self.script_failed(channel, level, page, &name);
            }
            _ => {}
        }
    }

    /// A piece of a script `channel`'s fetcher fetched (section 12.5 step
    /// 1): passed to the channel's decoder unread, after what its
    /// contexts are told.
    fn script_fetched(&mut self, channel: u32, piece: SourcePiece) {
        // Section 12.3: a page with a script and no page file is a
        // JavaScript page, not a page that does not exist. Its page
        // file's 404 was said just before its script came, since the
        // fetcher asks for the page first: that refusal is withdrawn.
        if piece.level == ContextId::Page && piece.offset == 0 {
            // The warning held for that 404 is never said (TODO-7419).
            self.refusal_withdrawn(channel, piece.page.page);
            let refused = self
                .session
                .all_tabs_mut()
                .find(|tab| tab.channel.id == channel)
                .filter(|tab| tab.channel.failed.is_some_and(|(_, page)| page == Some(piece.page.page)));
            if let Some(tab) = refused {
                tab.channel.failed = None;
                if self.active_channel() == Some(channel) {
                    let strip = self.strip();
                    let _ = self.send(&protocol::ToInterface::Strip(strip));
                }
            }
        }
        self.script_facts_once(channel);
        self.keep_script(channel, &piece);
        self.tell(channel, ToDecoder::Script(ScriptOrder::Source(piece)));
    }

    /// Keeps a piece of a script given to `channel`'s decoder, for a
    /// decoder that replaces it: a first piece replaces the script kept
    /// for the same level and page, and the page scripts kept are the
    /// newest [`MAX_PAGE_SCRIPTS_KEPT`].
    fn keep_script(&mut self, channel: u32, piece: &SourcePiece) {
        let kept = self.scripts.kept.entry(channel).or_default();
        if piece.offset == 0 {
            // The channel's script is one, whatever page it came with.
            kept.retain(|held| held.level != piece.level || (piece.level == ContextId::Page && held.page != piece.page));
            while kept.iter().filter(|held| held.level == ContextId::Page).count() >= MAX_PAGE_SCRIPTS_KEPT {
                let Some(oldest) = kept.iter().position(|held| held.level == ContextId::Page) else { break };
                kept.remove(oldest);
            }
            kept.push(Kept { level: piece.level, page: piece.page, pieces: Vec::new() });
        }
        if let Some(held) = kept.iter_mut().rev().find(|held| held.level == piece.level && held.page == piece.page) {
            held.pieces.push(piece.clone());
        }
    }

    /// A decoder of generation `decoder` is being started for `channel`:
    /// what it is told first when it replaces another (its store grew, or
    /// the channel woke): what its scripts' contexts are told, and every
    /// script kept for the channel, so that a page whose decoder was
    /// restarted does not lose its script. A channel no tab holds is
    /// forgotten.
    pub(in super::super) fn scripts_for_new_decoder(&mut self, channel: u32, decoder: u64) -> Vec<ToDecoder> {
        let open: Vec<u32> = self.session.all_tabs().map(|tab| tab.channel.id).collect();
        self.scripts.kept.retain(|kept, _| open.contains(kept));
        self.scripts.told.retain(|told, _| open.contains(told));
        let pieces: Vec<SourcePiece> = self.scripts.kept.get(&channel).into_iter().flatten().flat_map(|held| held.pieces.iter().cloned()).collect();
        let Some(facts) = self.script_facts(channel).filter(|_| !pieces.is_empty()) else { return Vec::new() };
        self.scripts.told.insert(channel, decoder);
        core::iter::once(ScriptOrder::Channel(facts)).chain(pieces.into_iter().map(ScriptOrder::Source)).map(ToDecoder::Script).collect()
    }

    /// A script of `channel` could not be fetched: its page runs without
    /// it, as with no script (section 1 principle 5).
    fn script_failed(&mut self, channel: u32, level: ContextId, page: PageSub, name: &str) {
        self.log(
            Level::Warning,
            Some(channel),
            &format!("the script {} of page {:03X} could not be loaded; the page runs without it", clean(name, MAX_SCRIPT_NAME), page.page),
        );
        if level == ContextId::Page {
            if let Some(kept) = self.scripts.kept.get_mut(&channel) {
                kept.retain(|held| held.level != ContextId::Page || held.page != page);
            }
            self.tell(channel, ToDecoder::Script(ScriptOrder::NoScript(page)));
        }
    }

    /// Tells `channel`'s decoder what its scripts' contexts are told,
    /// before its first script.
    pub(in super::super) fn give_script_facts(&mut self, channel: u32, facts: ChannelFacts) {
        self.tell(channel, ToDecoder::Script(ScriptOrder::Channel(facts)));
    }

    /// Gives `channel`'s decoder a script as it was fetched or read,
    /// unread, in pieces: the channel's, or the one of `page`. One longer
    /// than a script may be is not given.
    pub(in super::super) fn give_script(&mut self, channel: u32, level: ContextId, page: PageSub, name: &str, bytes: &[u8]) {
        let Ok(total) = u32::try_from(bytes.len()) else { return };
        if bytes.is_empty() || bytes.len() > MAX_SCRIPT_BYTES {
            return;
        }
        let mut offset = 0_u32;
        for piece in bytes.chunks(MAX_SOURCE_PIECE_BYTES) {
            self.script_fetched(channel, SourcePiece { level, page, name: name.to_owned(), total, offset, bytes: piece.to_vec() });
            offset = offset.saturating_add(u32::try_from(piece.len()).unwrap_or(u32::MAX));
        }
    }
}
