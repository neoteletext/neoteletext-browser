//! The recording of the open tab's traffic (REQ-6406, DEC-6405), kept by
//! the trusted core while the developer console is open. Nothing is
//! recorded until the user asks for it with the console's Record button:
//! opening the console records nothing. A recording is of one tab; it is
//! paused and stopped from the console's bar, and ends when the console
//! closes or another tab is opened.

use neoteletext::recording::{Direction, Payload, Recording};
use protocol::ToInterface;
use protocol::console::{Level, RecordingRequest, RecordingState, RecordingView};
use protocol::fetcher::{FromFetcher, Origin, ToFetcher};

use super::Core;

/// What the trusted core holds of a recording.
#[derive(Debug, Default)]
pub(super) enum Slot {
    /// None: the console is closed, or the user has not asked for one, or
    /// one could not be started.
    #[default]
    Off,
    /// The open tab is private and is not recorded.
    Private(u32),
    /// One is open, of this tab.
    On(Recording),
    /// The last one ended: its tab, its file's name and its size.
    Stopped { channel: u32, name: String, bytes: u64 },
}

/// The kind an entry of received bytes is written with, by how they came.
const fn origin_kind(origin: Origin) -> &'static str {
    match origin {
        Origin::Fetched => "fetched",
        Origin::FetchedPage => "fetched-page",
        Origin::FetchedPackets => "fetched-packets",
        Origin::PushedPackets => "pushed-packets",
    }
}

impl Core {
    /// The console opened or closed. Opening it records nothing: a
    /// recording is the user's to start (the owner, 16 November 2026:
    /// "recording must only happen when specific switched on"). Closing
    /// it ends one.
    pub(super) fn recording_console(&mut self, open: bool) {
        if open {
            self.recording = self.idle();
        } else {
            self.end_recording("the console closed");
            self.recording = Slot::Off;
        }
    }

    /// What is held while nothing is recorded: that the open tab is
    /// private, so the bar can say it is never recorded, or nothing.
    fn idle(&self) -> Slot {
        match self.session.active_tab() {
            Some(tab) if tab.private => Slot::Private(tab.channel.id),
            _ => Slot::Off,
        }
    }

    /// Starts a recording of the open tab, ending any that is open.
    fn start_recording(&mut self) {
        self.end_recording("another began");
        let Some(tab) = self.session.active_tab() else {
            self.recording = Slot::Off;
            return;
        };
        let channel = tab.channel.id;
        if tab.private {
            self.recording = Slot::Private(channel);
            return;
        }
        let (host, address) = tab.address().map(|address| (address.host.clone(), address.text.clone())).unwrap_or_default();
        let Some(folder) = self.settings_folder.clone() else {
            self.recording = Slot::Off;
            self.note(Level::Warning, None, "The tab is not recorded: the browser has no folder to keep recordings in");
            return;
        };
        match Recording::start(&folder, channel, &host, &address, crate::VERSION, self.now(), neoteletext::calendar::utc_now()) {
            Ok(recording) => {
                let path = recording.path().display().to_string();
                self.recording = Slot::On(recording);
                self.note(Level::Info, Some(channel), &format!("Recording this tab's traffic to {path}. What is typed into the page is in the file."));
            }
            Err(error) => {
                self.recording = Slot::Off;
                self.note(Level::Warning, Some(channel), &format!("The tab is not recorded: {error}"));
            }
        }
    }

    /// Ends the open recording, if there is one, saying where it is.
    fn end_recording(&mut self, why: &str) {
        let Slot::On(mut recording) = std::mem::take(&mut self.recording) else { return };
        let _ = recording.mark(self.now(), &format!("stopped: {why}"));
        let (channel, name, bytes, path) = (recording.channel(), recording.name(), recording.bytes(), recording.path().display().to_string());
        drop(recording);
        self.recording = Slot::Stopped { channel, name, bytes };
        self.note(Level::Info, Some(channel), &format!("The recording ended ({why}): {path}"));
    }

    /// A request from the console's bar.
    pub(super) fn recording_request(&mut self, request: RecordingRequest) {
        if !self.console.is_open() {
            return;
        }
        let now = self.now();
        match (request, &mut self.recording) {
            (RecordingRequest::Pause | RecordingRequest::Resume, Slot::On(recording)) => {
                let wrote = recording.set_paused(request == RecordingRequest::Pause, now).map(|()| true);
                self.recording_wrote(wrote);
            }
            (RecordingRequest::Stop, Slot::On(_)) => self.end_recording("stopped from the console"),
            (RecordingRequest::Record, Slot::Off | Slot::Stopped { .. }) => self.start_recording(),
            _ => {}
        }
    }

    /// A recording is of the tab it was started on: when another tab is
    /// the open one, it ends, and the new tab is not recorded until the
    /// user asks.
    pub(super) fn follow_recording(&mut self) {
        if !self.console.is_open() {
            return;
        }
        let recorded = match &self.recording {
            Slot::Off => None,
            Slot::Private(channel) | Slot::Stopped { channel, .. } => Some(*channel),
            Slot::On(recording) => Some(recording.channel()),
        };
        if recorded.is_none() || recorded != self.active_channel() {
            self.end_recording("another tab was opened");
            self.recording = self.idle();
        }
    }

    /// What a write to the file came to: an error, or the bound reached,
    /// ends the recording and says so; the tab is not disturbed.
    fn recording_wrote(&mut self, wrote: std::io::Result<bool>) {
        match wrote {
            Ok(true) => {}
            Ok(false) => self.end_recording("it reached its largest size"),
            Err(error) => self.end_recording(&format!("it could not be written: {error}")),
        }
    }

    /// The open recording of `channel`, if it is being written.
    fn recording_of(&mut self, channel: u32) -> Option<&mut Recording> {
        match &mut self.recording {
            Slot::On(recording) if recording.channel() == channel && !recording.paused() => Some(recording),
            _ => None,
        }
    }

    /// A message from `channel`'s fetcher: one entry. Bytes are written
    /// whole; viewer state's messages only as their kind, never a challenge,
    /// key or signature (VS-209); the rest as the report's text.
    pub(super) fn record_received(&mut self, channel: u32, message: &FromFetcher) {
        let now = self.now();
        let Some(recording) = self.recording_of(channel) else { return };
        let wrote = match message {
            FromFetcher::Page { origin, bytes } => recording.entry(now, Direction::Received, origin_kind(*origin), Payload::Bytes(bytes)),
            FromFetcher::StateChallenged { .. } => recording.entry(now, Direction::Received, "state", Payload::Text("challenge")),
            FromFetcher::StateEndReceived { .. } => recording.entry(now, Direction::Received, "state", Payload::Text("end")),
            FromFetcher::StateKeepOffered => recording.entry(now, Direction::Received, "state", Payload::Text("keep offer")),
            other => recording.entry(now, Direction::Received, "report", Payload::Text(&format!("{other:?}"))),
        };
        self.recording_wrote(wrote);
    }

    /// A message to `channel`'s fetcher: one entry.
    pub(super) fn record_sent(&mut self, channel: u32, message: &ToFetcher) {
        let now = self.now();
        let Some(recording) = self.recording_of(channel) else { return };
        let wrote = match message {
            // Input is the only thing sent as bytes, and a build without it has no such message.
            #[cfg(feature = "interaction")]
            ToFetcher::Send { bytes, .. } => recording.entry(now, Direction::Sent, "send", Payload::Bytes(bytes)),
            ToFetcher::StateAnswer { .. } => recording.entry(now, Direction::Sent, "state", Payload::Text("answer")),
            ToFetcher::StateSendOffer | ToFetcher::StateSendNone(_) => recording.entry(now, Direction::Sent, "state", Payload::Text("unprompted")),
            other => recording.entry(now, Direction::Sent, "request", Payload::Text(&format!("{other:?}"))),
        };
        self.recording_wrote(wrote);
    }

    /// Bytes sent on `channel`'s push connection: one entry.
    #[cfg_attr(
        all(not(feature = "interaction"), not(test)),
        expect(dead_code, reason = "only input is sent as bytes on the push connection, and this build has none; the tests still write entries with it")
    )]
    pub(super) fn record_sent_bytes(&mut self, channel: u32, bytes: &[u8]) {
        let now = self.now();
        let Some(recording) = self.recording_of(channel) else { return };
        let wrote = recording.entry(now, Direction::Sent, "send", Payload::Bytes(bytes));
        self.recording_wrote(wrote);
    }

    /// The recording as the console's bar shows it.
    pub(super) fn recording_view(&self) -> RecordingView {
        match &self.recording {
            Slot::Off => RecordingView::default(),
            Slot::Private(_) => RecordingView { state: RecordingState::Private, ..RecordingView::default() },
            Slot::On(recording) => RecordingView {
                state: if recording.paused() { RecordingState::Paused } else { RecordingState::Recording },
                bytes: recording.bytes(),
                name: recording.name(),
            },
            Slot::Stopped { name, bytes, .. } => RecordingView { state: RecordingState::Stopped, bytes: *bytes, name: name.clone() },
        }
    }

    /// Tells the interface what the recording is doing, when that changed
    /// since it was last told and the console is open.
    pub(super) fn send_recording(&mut self) -> Result<(), String> {
        if !self.console.is_open() {
            self.recording_sent = None;
            return Ok(());
        }
        let view = self.recording_view();
        if self.recording_sent.as_ref() != Some(&view) {
            self.send(&ToInterface::Recording(view.clone()))?;
            self.recording_sent = Some(view);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test reports a failure by panicking, and indexes what it made")]

    use neoteletext::recording::{EXTENSION, FOLDER};
    use protocol::ToCore;

    use super::super::test_support::Test;
    use super::*;

    /// The recordings in the test's folder, by name, each with its text.
    fn recordings(test: &Test) -> Vec<(String, String)> {
        let Ok(entries) = std::fs::read_dir(test.folder.join(FOLDER)) else { return Vec::new() };
        let mut found: Vec<(String, String)> = entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == EXTENSION))
            .map(|path| (path.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&path).unwrap()))
            .collect();
        found.sort();
        found
    }

    /// The entries of a recording's text: its lines after the header,
    /// each without its time, which differs from run to run: an entry's
    /// first word, and a mark's second.
    fn entries(text: &str) -> Vec<String> {
        let timeless = |line: &str| {
            let mut words: Vec<&str> = line.split(' ').collect();
            words.remove(usize::from(line.starts_with('#')));
            words.join(" ")
        };
        text.lines().skip(4).map(timeless).collect()
    }

    /// REQ-6406 items 1 to 3: opening the console records nothing; Record
    /// starts a recording of the open tab; what the fetcher gives and is
    /// given is written, each way; pause holds it; the console's closing
    /// ends it.
    #[test]
    fn the_console_records_the_open_tab_until_it_closes() {
        let mut test = Test::new("recording-console");
        let channel = test.tab("https://example.invalid/teletext/", false);
        test.core.record_sent_bytes(channel, &[9]);
        assert_eq!(recordings(&test), [], "nothing is recorded with the console closed");
        test.core.settings_intent(ToCore::Console(true)).unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Off, "the console alone records nothing");
        test.core.record_sent_bytes(channel, &[0x44]);
        assert_eq!(recordings(&test), [], "nothing is recorded until the user asks");
        test.core.settings_intent(ToCore::Recording(RecordingRequest::Record)).unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Recording);
        test.core.record_sent_bytes(channel, &[0x82, 0x01]);
        test.core.record_received(channel, &FromFetcher::Page { origin: Origin::PushedPackets, bytes: vec![0xaa, 0xbb] });
        test.core.record_received(channel, &FromFetcher::Pushing(true));
        test.core.record_sent(channel, &ToFetcher::FetchPage { page: 0x100 });
        test.core.record_sent_bytes(channel + 1, &[0x77]);
        test.core.settings_intent(ToCore::Recording(RecordingRequest::Pause)).unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Paused);
        test.core.record_sent_bytes(channel, &[0x55]);
        test.core.settings_intent(ToCore::Recording(RecordingRequest::Resume)).unwrap();
        test.core.record_sent_bytes(channel, &[0x66]);
        test.core.settings_intent(ToCore::Console(false)).unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Off);
        let found = recordings(&test);
        assert_eq!(found.len(), 1);
        assert!(found[0].0.ends_with("-example.invalid.ntrec"), "{}", found[0].0);
        assert!(found[0].1.contains("# address https://example.invalid/teletext/\n"));
        assert_eq!(
            entries(&found[0].1),
            [
                "> send 8201",
                "< pushed-packets aabb",
                "< report Pushing(true)",
                "> request FetchPage { page: 256 }",
                "# paused",
                "# resumed",
                "> send 66",
                "# stopped: the console closed"
            ],
            "each way in order, nothing of another tab, nothing while paused"
        );
    }

    /// REQ-6406 item 3 and VS-209: viewer state's messages are recorded
    /// as their kind alone, never a challenge, key or signature.
    #[test]
    fn viewer_state_is_recorded_only_as_its_kind() {
        let mut test = Test::new("recording-state");
        let channel = test.tab("https://example.invalid/teletext/", false);
        test.core.settings_intent(ToCore::Console(true)).unwrap();
        test.core.settings_intent(ToCore::Recording(RecordingRequest::Record)).unwrap();
        test.core.record_received(channel, &FromFetcher::StateChallenged { request: 1, challenge: [0xAB; 32], create: true });
        let reply = protocol::fetcher::StateReply::Proof {
            algorithm: protocol::fetcher::Algorithm::Ed25519,
            lifetime: protocol::fetcher::Lifetime::Visit,
            public_key: [0xCD; 32],
            signature: [0xEF; 64],
        };
        test.core.record_sent(channel, &ToFetcher::StateAnswer { request: 1, reply });
        test.core.settings_intent(ToCore::Console(false)).unwrap();
        let written = &recordings(&test)[0].1;
        for secret in ["171", "205", "239", "AB", "CD", "EF", "ab", "cd", "ef"] {
            assert!(!written.contains(&format!("{secret}, {secret}")), "{written}");
        }
        assert_eq!(entries(written)[..2], ["< state challenge", "> state answer"]);
    }

    /// REQ-6406 items 1 and 2: a private tab is never recorded, even when
    /// asked; Stop ends a recording; and another tab made the open one ends
    /// the recording, the new tab not being recorded until the user asks.
    #[test]
    fn a_private_tab_is_not_recorded_and_each_tab_has_its_own_file() {
        let mut test = Test::new("recording-tabs");
        test.tab("https://private.invalid/teletext/", true);
        test.core.settings_intent(ToCore::Console(true)).unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Private);
        test.core.settings_intent(ToCore::Recording(RecordingRequest::Record)).unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Private, "asking changes nothing");
        assert_eq!(recordings(&test), []);
        let first = test.tab("https://first.invalid/teletext/", false);
        test.core.send_console().unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Off, "the tab now open is not recorded unasked");
        test.core.settings_intent(ToCore::Recording(RecordingRequest::Record)).unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Recording);
        test.core.record_sent_bytes(first, &[1]);
        test.core.settings_intent(ToCore::Recording(RecordingRequest::Stop)).unwrap();
        let stopped = test.core.recording_view();
        assert_eq!(stopped.state, RecordingState::Stopped);
        assert!(stopped.name.ends_with("-first.invalid.ntrec") && stopped.bytes > 0);
        test.core.record_sent_bytes(first, &[2]);
        assert_eq!(recordings(&test).len(), 1);
        assert!(recordings(&test)[0].1.ends_with("stopped: stopped from the console\n"), "nothing after it stopped");
        // A recording does not follow the user to another tab.
        // A recording begun within the second of the last has a name of its own.
        test.core.settings_intent(ToCore::Recording(RecordingRequest::Record)).unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Recording);
        assert_eq!(recordings(&test).len(), 2);
        let second = test.tab("https://second.invalid/teletext/", false);
        test.core.send_console().unwrap();
        assert_eq!(test.core.recording_view().state, RecordingState::Off, "the new tab is not recorded");
        test.core.record_sent_bytes(second, &[3]);
        test.core.record_sent_bytes(first, &[4]);
        let found = recordings(&test);
        assert!(found.iter().all(|(name, _)| name.ends_with("-first.invalid.ntrec")), "no recording was started for the new tab");
        let second_recording = &found.iter().find(|(name, _)| name.contains("-2-first.invalid")).unwrap().1;
        assert!(second_recording.ends_with("stopped: another tab was opened\n"), "{second_recording}");
    }
}
