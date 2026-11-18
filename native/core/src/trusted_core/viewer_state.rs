//! Viewer state in the running browser (neoteletext-common's
//! `docs/viewer-state.md`, DEC-6471, DEC-6478): a fetcher's challenges
//! answered from the key store, each with its own tab's channel alone
//! (VS-103); the viewer's controls (VS-109, VS-112); the strip's mark and
//! offer (VS-114, VS-208); and the remembered keys kept, sealed under a key
//! from the platform's secret store, which the broker reads (VS-108).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use neoteletext::calendar::utc_now;
use neoteletext::sealed_file::KEY_BYTES;
use neoteletext::viewer_keys::file::{self as kept, Loaded};
use neoteletext::viewer_keys::{Answer, Asked, Known, Store};
use neoteletext_web::url::Url;
use neoteletext_web::viewer_state::{Algorithm, CHALLENGE_BYTES, ChannelIdentity, NoneReason};
use protocol::console::Level;
use protocol::fetcher::{FromFetcher, StateReply, ToFetcher};
use protocol::viewer_state::{KnownRequest, KnownRow, KnownView, MAX_NOTICE, MAX_VIEW_ROWS, MAX_WHEN};
use protocol::{ToInterface, clean};

use super::Core;
use super::numbers::Numbers;
use super::run::{KEYCHAIN, KEYCHAIN_BY_DEFAULT, NO_KEYCHAIN};

/// VS-212: a key is made only within 7,000 ms of the viewer's input on
/// the tab, the server's `PROGRAM_REPLY_LIMIT_MS`, applied here
/// independently of the server.
pub(crate) const STATE_GESTURE: Duration = Duration::from_millis(7_000);
/// VS-119 and open question Q12: a fetcher is given at most 60 signatures
/// a minute.
pub(crate) const MOST_PROOFS_PER_FETCHER_PER_MINUTE: u32 = 60;
const PROOF_WINDOW: Duration = Duration::from_secs(60);
/// The challenges a fetcher's signatures are remembered by, so that one is
/// never signed twice on it (VS-119): a minute's worth.
const SIGNED_KEPT: usize = 60;
const _: () = assert!(SIGNED_KEPT >= MOST_PROOFS_PER_FETCHER_PER_MINUTE as usize, "a minute of challenges is remembered");
/// How long after a change the remembered keys are written: a burst of
/// proofs is one write.
const WRITE_AFTER: Duration = Duration::from_secs(1);
/// What the list says once after a copied or restored profile (VS-118).
const NOT_CARRIED: &str = "Channels remembered in a copied or restored profile were not carried over";

/// Viewer state in the trusted core.
#[derive(Default)]
pub(crate) struct ViewerState {
    /// Every key.
    pub(crate) store: Store,
    /// The key file's sealing key, once the broker gave it.
    key: Option<[u8; KEY_BYTES]>,
    /// Why "Remember me" is unavailable, once known (VS-108).
    pub(crate) unavailable: Option<String>,
    /// Started with `--no-keychain` (decision 113, open question Q15): the
    /// broker is never asked, and nothing is remembered.
    pub(crate) no_keychain: bool,
    /// When the remembered keys are next written.
    pub(crate) due: Option<Instant>,
    /// When the viewer last acted on each fetcher's tab (VS-212).
    gestures: BTreeMap<u32, Instant>,
    /// Each fetcher's signatures this minute: when its minute began, and
    /// how many (VS-119).
    proofs: BTreeMap<u32, (Instant, u32)>,
    /// The challenges each fetcher was given signatures for.
    signed: BTreeMap<u32, VecDeque<[u8; CHALLENGE_BYTES]>>,
    /// The fetchers whose channels offered to remember the viewer, not yet
    /// answered (VS-208).
    offers: BTreeSet<u32>,
    /// The channels whose offer was shown in this run, once each.
    offered: BTreeSet<String>,
    numbers: Numbers<String>,
    panel_open: bool,
    query: String,
    first: u32,
    notice: String,
}

fn now_unix() -> u64 {
    u64::try_from(utc_now()).unwrap_or(0)
}

/// The channel identity of an address's channel: an `http` or `https`
/// channel's canonical form (VS-102); none for a file, the start page or
/// anything else, which never has state (VS-210).
pub(crate) fn identity_of(channel: &str) -> Option<ChannelIdentity> {
    ChannelIdentity::of(&Url::parse(channel).ok()?).ok()
}

impl Core {
    /// The channel a fetcher's tab shows, and whether the tab is private:
    /// taken from the tab, never from the fetcher (VS-103).
    fn fetcher_identity(&self, channel: u32) -> Option<(ChannelIdentity, bool)> {
        let tab = self.session.all_tabs().find(|tab| tab.channel.id == channel)?;
        Some((identity_of(tab.address()?.channel_text())?, tab.private))
    }

    /// Tells every fetcher of `identity` in a jar but `except` to send
    /// `message` unprompted (VS-111, VS-113).
    fn tell_channel(&mut self, identity: &str, private: bool, except: Option<u32>, message: &ToFetcher) {
        let fetchers: Vec<u32> = self
            .session
            .all_tabs()
            .filter(|tab| tab.private == private && Some(tab.channel.id) != except)
            .filter(|tab| tab.address().and_then(|address| identity_of(address.channel_text())).is_some_and(|each| each.as_str() == identity))
            .map(|tab| tab.channel.id)
            .collect();
        for fetcher in fetchers {
            self.tell_fetcher(fetcher, message);
        }
    }

    /// The viewer acted on the tab whose fetcher is `channel` (VS-212).
    /// Only input to a channel is such an act, so a build with typing
    /// compiled out has no caller but the tests.
    #[cfg(any(feature = "interaction", test))]
    pub(crate) fn viewer_gesture(&mut self, channel: u32) {
        self.viewer_state.gestures.insert(channel, Instant::now());
    }

    /// A fetcher's state message (VS-119), answered at once when it is a
    /// challenge.
    pub(crate) fn state_message(&mut self, channel: u32, message: &FromFetcher) -> Result<(), String> {
        if let Some(answer) = self.state_reply(channel, message)? {
            self.tell_fetcher(channel, &answer);
        }
        Ok(())
    }

    /// What a fetcher's state message does, and the answer it is owed.
    pub(crate) fn state_reply(&mut self, channel: u32, message: &FromFetcher) -> Result<Option<ToFetcher>, String> {
        let owner = self.fetcher_identity(channel);
        match message {
            FromFetcher::StateChallenged { request, challenge, create } => {
                let reply = self.challenged(channel, owner.as_ref(), challenge, *create);
                self.send(&ToInterface::Strip(self.strip()))?;
                return Ok(Some(ToFetcher::StateAnswer { request: *request, reply }));
            }
            FromFetcher::StateEndReceived { key_id } => {
                if let Some((identity, private)) = owner
                    && self.viewer_state.store.end(private, identity.as_str(), key_id)
                {
                    self.viewer_state_changed();
                    self.send(&ToInterface::Strip(self.strip()))?;
                    self.send_known()?;
                }
            }
            // Ignored in a private window, for a blocked channel, without
            // the keychain, once shown in this run, and for a channel with
            // no visit key (VS-208).
            FromFetcher::StateKeepOffered => {
                if let Some((identity, false)) = owner
                    && self.can_remember()
                    && self.viewer_state.store.known(false, identity.as_str()) == Known::Visit
                    && !self.viewer_state.store.blocked.contains(identity.as_str())
                    && self.viewer_state.offered.insert(identity.as_str().to_owned())
                {
                    self.viewer_state.offers.insert(channel);
                    self.send(&ToInterface::Strip(self.strip()))?;
                }
            }
            _ => {}
        }
        Ok(None)
    }

    /// The answer to one challenge on the fetcher of `channel`: refused
    /// when its channel has no identity, beyond the fetcher's signatures a
    /// minute, or for a challenge already signed on it (VS-119).
    fn challenged(&mut self, channel: u32, owner: Option<&(ChannelIdentity, bool)>, challenge: &[u8; CHALLENGE_BYTES], create: bool) -> StateReply {
        let Some((identity, private)) = owner else { return StateReply::None(NoneReason::NoKey) };
        let now = Instant::now();
        let state = &mut self.viewer_state;
        let (began, count) = state.proofs.get(&channel).copied().filter(|(began, _)| now.saturating_duration_since(*began) < PROOF_WINDOW).unwrap_or((now, 0));
        let signed = state.signed.entry(channel).or_default();
        if count >= MOST_PROOFS_PER_FETCHER_PER_MINUTE || signed.contains(challenge) {
            return StateReply::None(NoneReason::NoKey);
        }
        let gesture = state.gestures.get(&channel).is_some_and(|at| now.saturating_duration_since(*at) < STATE_GESTURE);
        let before = state.store.known(*private, identity.as_str());
        let asked = Asked { private: *private, identity, challenge, create, gesture };
        match state.store.answer(&asked, now_unix()) {
            Answer::Proof { lifetime, public_key, signature } => {
                state.proofs.insert(channel, (began, count.saturating_add(1)));
                if signed.len() >= SIGNED_KEPT {
                    signed.pop_front();
                }
                signed.push_back(*challenge);
                // A key just made: the channel's other tabs offer, so the
                // server challenges them and sees one viewer (VS-111).
                if before == Known::Not {
                    self.tell_channel(identity.as_str(), *private, Some(channel), &ToFetcher::StateSendOffer);
                }
                self.viewer_state_changed();
                StateReply::Proof { algorithm: Algorithm::Ed25519, lifetime, public_key, signature }
            }
            Answer::None(reason) => StateReply::None(reason),
        }
    }

    /// Whether "Remember me" can be offered: the keychain is in use and has
    /// not refused (VS-108, open question Q15).
    fn can_remember(&self) -> bool {
        !self.viewer_state.no_keychain && self.viewer_state.unavailable.is_none() && self.settings_folder.is_some()
    }

    /// The strip's viewer state for the active tab (VS-109, VS-114,
    /// VS-208).
    pub(super) fn viewer_strip(&self, strip: &mut interface_core::browser::StripState) {
        let Some(tab) = self.session.active_tab() else { return };
        let Some(identity) = tab.address().and_then(|address| identity_of(address.channel_text())) else { return };
        strip.known = self.viewer_state.store.known(tab.private, identity.as_str());
        strip.can_remember = !tab.private && strip.known == Known::Visit && self.can_remember();
        strip.keep_offer = strip.can_remember && self.viewer_state.offers.contains(&tab.channel.id);
    }

    /// Remembered keys changed: written soon, once; the list shows them now.
    fn viewer_state_changed(&mut self) {
        if self.viewer_state.store.changed() && self.viewer_state.key.is_some() && self.viewer_state.due.is_none() {
            self.viewer_state.due = Instant::now().checked_add(WRITE_AFTER);
        }
    }

    /// Writes the remembered keys, sealed, when they changed; none left
    /// behind when none is remembered.
    pub(crate) fn keep_viewer_keys(&mut self) {
        self.viewer_state.due = None;
        let (Some(folder), Some(key)) = (self.settings_folder.clone(), self.viewer_state.key) else { return };
        if !self.viewer_state.store.changed() {
            return;
        }
        let remembered = self.viewer_state.store.listed().iter().any(|(_, listed)| listed.remembered);
        let written = if remembered { kept::save(&folder, &key, &self.viewer_state.store) } else { kept::remove(&folder) };
        match written {
            Ok(()) => self.viewer_state.store.written(),
            Err(error) => self.log(Level::Error, None, &format!("the remembered channels could not be kept in {}: {error}", folder.display())),
        }
    }

    /// At start: the settings' choice and blocked channels; the cookie file
    /// deleted, once (VS-603); and the remembered keys, when there is a key
    /// file and the keychain may be asked.
    pub(crate) fn viewer_state_at_start(&mut self) {
        self.viewer_state.store.never = !self.settings.channels_may_know;
        self.viewer_state.store.blocked = self.settings.state_block.iter().cloned().collect();
        let Some(folder) = self.settings_folder.clone() else { return };
        // Cookies are gone: their file is credentials no code reads (VS-603).
        if let Err(error) = neoteletext::sealed_file::remove(&folder, "cookies.bin") {
            self.log(Level::Error, None, &format!("the old cookie file could not be removed: {error}"));
        }
        if self.viewer_state.no_keychain {
            let why =
                if KEYCHAIN_BY_DEFAULT { format!("started with {NO_KEYCHAIN}") } else { format!("the Keychain is used only when started with {KEYCHAIN}") };
            self.viewer_state.unavailable = Some(format!("Remember me is unavailable: {why}"));
            return;
        }
        // The secret store is asked only when there is something to open:
        // a viewer who never remembers is never asked for it.
        if folder.join(kept::FILE_NAME).exists() {
            self.open_viewer_keys(&folder);
        }
    }

    /// The sealing key from the broker, and the key file opened with it.
    /// Without the key nothing is read or written and "Remember me" is
    /// unavailable, with the reason (VS-108).
    fn open_viewer_keys(&mut self, folder: &std::path::Path) -> bool {
        let key = match self.broker.as_ref().map(|broker| broker.viewer_keys_key()) {
            Some(Ok(key)) => key,
            Some(Err(reason)) => {
                self.log(Level::Warning, None, &format!("remembered channels are unavailable: {reason}"));
                self.viewer_state.unavailable = Some(format!("Remember me is unavailable: {reason}"));
                return false;
            }
            None => return false,
        };
        self.viewer_state.key = Some(key);
        match kept::load(folder, &key) {
            Ok(Loaded::None) => {}
            Ok(Loaded::Text(text)) => {
                for dropped in self.viewer_state.store.import(&text, now_unix()) {
                    self.log(Level::Warning, None, &dropped);
                }
            }
            // A copied or restored profile: its keys are not carried over,
            // and the viewer is told once (VS-118).
            Ok(Loaded::NotOpened) => {
                self.log(Level::Warning, None, NOT_CARRIED);
                self.viewer_state.notice = String::from(NOT_CARRIED);
                if let Err(error) = kept::remove(folder) {
                    self.log(Level::Error, None, &format!("the unreadable viewer key file could not be removed: {error}"));
                }
            }
            Err(error) => self.log(Level::Error, None, &format!("the remembered channels could not be read: {error}")),
        }
        true
    }

    /// At exit: the remembered keys written.
    pub(crate) fn viewer_state_at_exit(&mut self) {
        self.keep_viewer_keys();
    }

    /// The last private tab closed, in every window: the private keys go
    /// (VS-110).
    pub(crate) fn private_keys_closed(&mut self) {
        if !self.session.all_tabs().any(|tab| tab.private) {
            self.viewer_state.store.close_private();
        }
    }

    /// The settings' "Channels may know you" changed (VS-112).
    pub(crate) fn channels_may_know_changed(&mut self, allowed: bool) {
        self.viewer_state.store.never = !allowed;
    }

    /// Forgets the ordinary keys made since `since`, seconds since 1970,
    /// with history (VS-112): each channel's connections are told.
    pub(crate) fn forget_made_since(&mut self, since: Option<i64>) -> usize {
        let from = since.map_or(0, |since| u64::try_from(since).unwrap_or(0));
        let gone = self.viewer_state.store.forget_made_since(from);
        self.forgotten(&gone);
        gone.len()
    }

    /// The ordinary jar's keys of `channels` were forgotten: each one's
    /// connections send `STATE_NONE` `forgotten`, which names no key, so
    /// nothing relates the old key to a later one (VS-113).
    fn forgotten(&mut self, channels: &[String]) {
        for identity in channels {
            self.tell_channel(identity, false, None, &ToFetcher::StateSendNone(NoneReason::Forgotten));
        }
        self.viewer_state_changed();
    }

    /// The active tab's channel remembered, or no longer (VS-109): strip
    /// input from the menu or the strip's question. The channel's
    /// connections offer, so the server learns the lifetime (VS-111).
    fn remember_active(&mut self, remembered: bool) -> Result<(), String> {
        let Some(tab) = self.session.active_tab() else { return Ok(()) };
        let (private, id) = (tab.private, tab.channel.id);
        let Some(identity) = tab.address().and_then(|address| identity_of(address.channel_text())) else { return Ok(()) };
        self.viewer_state.offers.remove(&id);
        if private {
            return self.send(&ToInterface::Strip(self.strip()));
        }
        self.remember_channel(identity.as_str(), remembered)?;
        self.send(&ToInterface::Strip(self.strip()))
    }

    /// `identity` remembered, or no longer, when it can be.
    fn remember_channel(&mut self, identity: &str, remembered: bool) -> Result<(), String> {
        let changed = if remembered {
            if !self.can_remember() || (self.viewer_state.key.is_none() && !self.settings_folder.clone().is_some_and(|folder| self.open_viewer_keys(&folder))) {
                let why = self.viewer_state.unavailable.clone().unwrap_or_else(|| String::from("Remember me is unavailable"));
                self.viewer_state.notice.clone_from(&why);
                return self.say(why);
            }
            self.viewer_state.store.remember(identity, now_unix())
        } else {
            self.viewer_state.store.stop_remembering(identity, now_unix())
        };
        if changed {
            self.tell_channel(identity, false, None, &ToFetcher::StateSendOffer);
            self.viewer_state_changed();
        }
        Ok(())
    }

    /// A request of the viewer's controls, strip input (VS-109, VS-112):
    /// the list's rows are acted on by the numbers it was shown.
    pub(crate) fn known_request(&mut self, request: KnownRequest) -> Result<(), String> {
        let row = |number: u32, state: &ViewerState| state.numbers.get(number).cloned();
        match request {
            KnownRequest::Panel(open) => {
                self.viewer_state.panel_open = open;
                if !open {
                    self.viewer_state.notice.clear();
                    return Ok(());
                }
            }
            KnownRequest::Query { text, first } => {
                self.viewer_state.query = clean(&text, protocol::viewer_state::MAX_QUERY);
                self.viewer_state.first = first;
            }
            KnownRequest::Forget(number) => {
                let Some(identity) = row(number, &self.viewer_state) else { return self.send_known() };
                if self.viewer_state.store.forget(false, &identity) {
                    self.forgotten(core::slice::from_ref(&identity));
                }
                self.viewer_state.notice = String::from("Forgotten");
            }
            KnownRequest::Remember { number, remembered } => {
                let Some(identity) = row(number, &self.viewer_state) else { return self.send_known() };
                self.remember_channel(&identity, remembered)?;
            }
            KnownRequest::Block { number, blocked } => {
                let Some(identity) = row(number, &self.viewer_state) else { return self.send_known() };
                if self.viewer_state.store.block(&identity, blocked) {
                    self.forgotten(core::slice::from_ref(&identity));
                }
                self.settings.state_block = self.viewer_state.store.blocked.iter().take(neoteletext::settings::MAX_BLOCKED).cloned().collect();
                self.save_settings();
                self.viewer_state.notice = if blocked { String::from("Blocked, and forgotten") } else { String::from("Unblocked") };
            }
            KnownRequest::ForgetAll => {
                let gone = self.viewer_state.store.forget_all();
                self.forgotten(&gone);
                self.viewer_state.notice = String::from("Every channel forgotten");
            }
            KnownRequest::RememberActive(remembered) => return self.remember_active(remembered),
            KnownRequest::DismissOffer => {
                if let Some(id) = self.active_channel() {
                    self.viewer_state.offers.remove(&id);
                }
                return self.send(&ToInterface::Strip(self.strip()));
            }
        }
        self.send(&ToInterface::Strip(self.strip()))?;
        self.send_known()
    }

    /// The list's view, while it is open: each channel with a key and each
    /// blocked channel, matching the search, never a key (VS-112, VS-116).
    pub(crate) fn send_known(&mut self) -> Result<(), String> {
        if !self.viewer_state.panel_open {
            return Ok(());
        }
        let offset = self.local_time().offset;
        let when = |at: u64| {
            let day = neoteletext::calendar::local_day(i64::try_from(at).unwrap_or(i64::MAX), offset);
            clean(&neoteletext::calendar::day_words(day), MAX_WHEN)
        };
        let query = self.viewer_state.query.to_lowercase();
        let mut rows: Vec<(String, KnownRow)> = self
            .viewer_state
            .store
            .listed()
            .into_iter()
            .map(|(identity, listed)| {
                let row = KnownRow {
                    number: 0,
                    label: clean(&identity, protocol::viewer_state::MAX_LABEL),
                    has_key: true,
                    remembered: listed.remembered,
                    blocked: false,
                    created: when(listed.created),
                    used: when(listed.used),
                };
                (identity, row)
            })
            .collect();
        for identity in &self.viewer_state.store.blocked {
            rows.push((identity.clone(), KnownRow { label: clean(identity, protocol::viewer_state::MAX_LABEL), blocked: true, ..KnownRow::default() }));
        }
        rows.retain(|(identity, _)| query.is_empty() || identity.to_lowercase().contains(&query));
        rows.sort_by(|(left, _), (right, _)| left.cmp(right));
        let total = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        let first = self.viewer_state.first.min(total);
        self.viewer_state.numbers.fresh();
        let shown: Vec<KnownRow> = rows
            .into_iter()
            .skip(usize::try_from(first).unwrap_or(usize::MAX))
            .take(MAX_VIEW_ROWS)
            .map(|(identity, mut row)| {
                row.number = self.viewer_state.numbers.number(identity);
                row
            })
            .collect();
        let notice =
            if self.viewer_state.notice.is_empty() { self.viewer_state.unavailable.clone().unwrap_or_default() } else { self.viewer_state.notice.clone() };
        let view = KnownView {
            query: self.viewer_state.query.clone(),
            first,
            total,
            rows: shown,
            can_remember: self.can_remember(),
            notice: clean(&notice, MAX_NOTICE),
        };
        // Said once (VS-118), or the result of one change.
        self.viewer_state.notice.clear();
        self.send(&ToInterface::Known(view))
    }
}

#[cfg(test)]
mod tests;
