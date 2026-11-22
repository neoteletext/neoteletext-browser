//! What a channel's decoder reports of its scripts, and what the trusted
//! core gives it for them (`neoteletext-common/docs/script-api.md`
//! sections 4.6, 11.1, 11.2 and 12.5; TODO-7293). A report is untrusted,
//! as everything a decoder says is.
//!
//! Built here: a script's claim of the network reaches the relay, which
//! passes a script host's requests only then; a decoder that stopped its
//! scripts has their script host ended; a script's log lines are the
//! channel's. The key claims, the verdicts, navigation, titles and labels
//! are kept or passed over for the trusted strip and the key routing,
//! which are work package F5's (TODO-7294), and a script's bytes are
//! given by [`Core::give_script`], which script loading calls (work
//! package C2, TODO-7304).

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
        not(all(test, unix, feature = "interaction")),
        expect(dead_code, reason = "read by the key routing and the trusted strip, which work package F5 builds (TODO-7294)")
    )]
    pub(in super::super) fn script_claims(&self, channel: u32) -> Option<&ScriptClaims> {
        self.scripts.claims.get(&channel)
    }

    /// A decoder's report of its channel's scripts.
    pub(in super::super) fn script_reported(&mut self, channel: u32, report: ScriptReport) {
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
            // Each event's verdict, a script's navigation, a page's title
            // and a thing's label: taken by the key routing, the
            // navigation gate and the screen reader's tree, which work
            // package F5 builds (TODO-7294). Until then every default
            // stands, as with no script.
            ScriptReport::Verdict { .. } | ScriptReport::Navigate { .. } | ScriptReport::Title { .. } | ScriptReport::Label { .. } => {}
            ScriptReport::Wanted { page, name } => self.script_wanted(channel, page, &name),
        }
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
            // A file opened from disk: its script is read from the file's
            // own folder by the broker (section 12.7, DEC-7315), which is
            // not built yet (TODO-7304 step 6).
            return self.log(
                Level::Warning,
                Some(channel),
                "a file opened from disk names a script; reading it from the file's folder is not built yet, so the page runs without it",
            );
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
        let Some(tab) = self.session.all_tabs().find(|tab| tab.channel.id == channel) else { return };
        let time = self.local_time();
        let facts = ChannelFacts {
            // A file's own address is no folder a script's address is made
            // from: a file page's origin is `null` (DEC-7315 item 3).
            channel_url: tab.channel.web.clone().unwrap_or_default(),
            channel_title: clean(tab.channel.name.as_deref().unwrap_or(&tab.channel.title), protocol::script::MAX_TITLE_BYTES),
            from_disk: tab.channel.web.is_none(),
            epoch_seconds: u64::try_from(time.utc).unwrap_or(0),
            zone_minutes: i16::try_from(time.offset.checked_div(SECONDS_PER_MINUTE).unwrap_or(0)).unwrap_or(0),
        };
        self.scripts.told.insert(channel, decoder);
        self.give_script_facts(channel, facts);
    }

    /// What `channel`'s fetcher said of a script: a piece of one it
    /// fetched, or that it could not fetch one.
    pub(in super::super) fn script_message(&mut self, channel: u32, message: protocol::fetcher::FromFetcher) {
        match message {
            protocol::fetcher::FromFetcher::Script(piece) => self.script_fetched(channel, piece),
            protocol::fetcher::FromFetcher::ScriptFailed { level, page, name } => self.script_failed(channel, level, page, &name),
            _ => {}
        }
    }

    /// A piece of a script `channel`'s fetcher fetched (section 12.5 step
    /// 1): passed to the channel's decoder unread, after what its
    /// contexts are told.
    fn script_fetched(&mut self, channel: u32, piece: SourcePiece) {
        self.script_facts_once(channel);
        self.tell(channel, ToDecoder::Script(ScriptOrder::Source(piece)));
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
