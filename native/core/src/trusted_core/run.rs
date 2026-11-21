//! The trusted core's run (decisions 46 and 53): start-up, the event loop,
//! each event handled, and shutdown of every helper.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;

use confine::broker::{Broker, Exit, Program};
use neoteletext::address::{is_outside_link, outside_link};
use neoteletext::launch;
use neoteletext::outbox::Outbox;
use protocol::console::Level;
use protocol::sound::{FromSound, ToSound, write_to_sound};
use protocol::{ToInterface, write_to_interface};

use super::processes::{end, end_aside, read_interface, spawn, stop_decoder};
use super::startup::{load_bookmarks, load_history, load_session, load_settings, open_at_start};
use super::{Core, Event};
use crate::session::Session;

/// Handles one event; `Ok(false)` once the browser is to end.
pub(super) fn handle(core: &mut Core, event: Event) -> Result<bool, String> {
    match event {
        // Handled, then its bytes given back to its helper's budget.
        Event::Budgeted(event, ticket) => {
            let go = handle(core, *event);
            drop(ticket);
            return go;
        }
        Event::Decoded(channel, generation, message) => {
            if core.decoders.get(&channel).map(super::processes::Decoder::generation) == Some(generation) {
                core.in_window_of(channel, |core| core.decoded(channel, message))?;
            }
        }
        Event::Fetched(channel, generation, message) => {
            if core.fetchers.get(&channel).map(super::processes::Fetcher::generation) == Some(generation) {
                core.in_window_of(channel, |core| core.fetched(channel, message))?;
            }
        }
        Event::FetcherProcessEnded(routes, reason) => {
            for (channel, generation) in super::processes::places(&routes) {
                core.fetcher_ended(channel, generation, reason.clone());
            }
        }
        Event::FramePhase(phase, read_at) => core.frame_phase(phase, read_at),
        Event::Intent(intent) => {
            // What the intent changes in a tab's list is stamped with now.
            core.session.now = neoteletext::calendar::utc_now();
            return core.intent(intent);
        }
        Event::DecoderEnded(channel, generation, reason) => core.in_window_of(channel, |core| {
            core.decoder_ended(channel, generation, reason);
            Ok(())
        })?,
        #[cfg(feature = "phone-keyboard")]
        Event::Phone(generation, message) => core.phone_said(generation, message)?,
        #[cfg(feature = "phone-keyboard")]
        Event::PhoneEnded(generation, reason) => core.phone_ended(generation, reason)?,
        Event::Sound(FromSound::Sounding(channels)) => core.sounding(&channels)?,
        Event::Sound(FromSound::Metered { channel, values }) => core.metered(channel, values),
        Event::Sound(FromSound::LocalOverrun { channel, count }) => core.local_overrun(channel, count)?,
        Event::Sound(FromSound::CueHeard { channel, audio_channel, number }) => core.cue_heard(channel, audio_channel, number),
        Event::Sound(FromSound::Spent { channel, microseconds }) => core.sound_spent(channel, microseconds),
        Event::SoundEnded(reason) => {
            let broke = reason.is_some();
            core.log(Level::Error, None, &format!("no sound: the sound helper ended{}", reason.map(|reason| format!(": {reason}")).unwrap_or_default()));
            if let Some((helper, input)) = core.sound.take() {
                drop(input);
                end_aside(helper);
            }
            core.sounding(&[])?;
            // No page's rules read a sound as playing from here (RULE-27).
            core.tell_sound_heard_all();
            // It served every tab: each says so until it is reloaded
            // (REQ-683). A helper that merely ended leaves the browser silent.
            if broke {
                core.shared_helper_broke(SOUND_HELPER_BROKE)?;
            }
        }
        Event::BookmarksRead(read) => core.bookmarks_read(read)?,
        Event::PictureConverted(generation, result) => core.picture_converted(generation, result)?,
        Event::FontChecked(generation, refusal) => core.font_checked(generation, refusal)?,
        Event::SnapshotFetched(job, fetched) => core.snapshot_fetched(job, fetched),
        Event::SnapshotDrawn(job, drawn) => core.snapshot_drawn(job, *drawn)?,
        Event::Opening(opening) => {
            core.session.now = neoteletext::calendar::utc_now();
            core.opening(opening)?;
        }
        Event::Updater(message) => {
            core.updates.said(message, neoteletext::calendar::utc_now());
            core.send_update()?;
        }
        Event::UpdaterEnded(reason) => {
            core.updates.ended(reason);
            core.send_update()?;
        }
        Event::InterfaceEnded(None) => return Ok(false),
        Event::InterfaceEnded(Some(reason)) => return Err(format!("the interface broke the protocol and was closed: {reason}")),
        #[cfg(unix)]
        Event::BrokerEnded => return Err(String::from("the broker ended, and the browser ends with it")),
    }
    Ok(true)
}

/// What every tab says once the sound helper broke its protocol.
const SOUND_HELPER_BROKE: &str = "The sound helper misbehaved and was closed";

pub(crate) fn run(broker: &Arc<Broker>, given: impl Iterator<Item = String>, confined: &str) -> Result<(), String> {
    // What happens before the trusted core is made, for its Log tab.
    let mut log = vec![(Level::Info, format!("trusted core: confined: {confined}"))];
    let CommandLine { addresses, allow_local, no_keychain, input_trace } = arguments(given)?;
    let mut interface = spawn(broker, Program::Interface, &[], false)?;
    log.push((Level::Info, format!("interface: confined: {}", interface.report)));
    let (events, incoming) = mpsc::channel();
    read_interface(&mut interface, events.clone())?;
    // A broker ended by a signal leaves nothing to end the core and its
    // helpers on macOS (TODO-2006); on Linux they are killed with it.
    #[cfg(unix)]
    {
        let broker_ended = events.clone();
        broker.when_ended(move || drop(broker_ended.send(Event::BrokerEnded))).map_err(|error| format!("cannot watch the broker: {error}"))?;
    }

    let (settings, settings_folder, settings_read) = load_settings(&mut log);
    let bookmarks = load_bookmarks(settings_folder.as_deref(), &mut log);
    let saved = load_session(settings_folder.as_deref(), &mut log).filter(|_| settings.restore);
    // The window's place first: the interface opens its window with the
    // first message it is sent (section 15.3).
    let mut to_interface = Outbox::new(interface.stdin.take().ok_or("the interface has no input")?);
    let window = saved.as_ref().and_then(|saved| saved.windows.first()).and_then(|window| window.geometry);
    if let Some(window) = window {
        write_to_interface(&mut to_interface, &ToInterface::Window(window)).map_err(|error| format!("cannot write to the interface: {error}"))?;
        to_interface.flush().map_err(|error| format!("cannot write to the interface: {error}"))?;
    }
    // File channels are read by the broker: the core reads no file but its
    // settings, history and bookmarks (decision 56).
    let reader = Arc::clone(broker);
    let mut session = Session::with_reader(bookmarks, Box::new(move |path: &Path| reader.read_source(path).map(crate::source::Source::from_bytes)));
    open_at_start(&mut session, saved.as_ref(), &addresses, &settings.home)?;
    let history = load_history(settings_folder.as_deref(), &mut log);
    let mut core = Core::new(session, allow_local, (settings, settings_folder), events, to_interface, Some(Arc::clone(broker)));
    core.library.history = history;
    for (level, text) in log {
        core.log(level, None, &text);
    }
    core.viewer_state.no_keychain = no_keychain;
    #[cfg(feature = "interaction")]
    if input_trace {
        core.input_trace = Some(neoteletext::input_trace::InputTrace::default());
    }
    #[cfg(not(feature = "interaction"))]
    let _ = input_trace;
    core.viewer_state_at_start();
    // Tab pictures off keep no snapshot (REQ-6886 item 9).
    if !core.settings.tab_pictures {
        core.tab_pictures_off();
    }
    core.keep_session();
    core.start_sound();
    // Updates: the launcher's state, then the updater, when the settings
    // let it check (`installation-and-updates.md` section 3.4).
    core.updates = super::updates::Updates::of_broker(broker, settings_read);
    core.start_updater();
    core.send_window_whole()?;
    // The other windows the session kept, each opened and sent whole.
    let first = core.session.window;
    for (id, geometry) in core.session.others.iter().map(|window| (window.id, window.geometry)).collect::<Vec<_>>() {
        core.send_untargeted(&ToInterface::OpenWindow { window: id, geometry })?;
        core.switch_window(id);
        core.send_window_whole()?;
    }
    core.switch_window(first);
    core.flush()?;
    core.hear_openings(broker);

    let result = event_loop(&mut core, &incoming);
    shut_down(core);
    if result.is_err() {
        let _ = interface.kill();
    }
    let status = interface.wait().map_err(|error| format!("cannot wait for the interface: {error}"))?;
    ended(result, &status)
}

/// The flag that leaves the platform's secret store alone: "Remember me"
/// is unavailable, every viewer key is a visit key, and the viewer key file
/// is neither read nor written (decision 113, VS-108, open question Q15).
pub(crate) const NO_KEYCHAIN: &str = "--no-keychain";

/// The flag that uses the platform's secret store where it is off by
/// default: the viewer key file's sealing key is read from it, or made
/// there.
pub(crate) const KEYCHAIN: &str = "--keychain";

/// Whether the secret store is used when neither flag is given. Off on
/// macOS until NeoTeletext is signed with Apple's Developer ID (the owner,
/// 28 October 2026): a build signed ad hoc is a new program to the
/// Keychain at every build, which asks for the login password at each
/// start. On; elsewhere.
pub(crate) const KEYCHAIN_BY_DEFAULT: bool = !cfg!(target_os = "macos");

/// The flag that shows each input event sent to a channel on the open
/// page's row 0, with a count of the same event running (`input_trace.rs`).
pub(crate) const INPUT_TRACE: &str = "--input-trace";

/// The browser's own arguments.
#[derive(Debug, PartialEq, Eq)]
struct CommandLine {
    /// The addresses to open.
    addresses: Vec<String>,
    /// Each local address `--allow-local` lets a fetcher reach.
    allow_local: Vec<String>,
    /// The secret store is left alone: [`NO_KEYCHAIN`] was given, or
    /// [`KEYCHAIN_BY_DEFAULT`] is off and [`KEYCHAIN`] was not.
    no_keychain: bool,
    /// [`INPUT_TRACE`] was given.
    input_trace: bool,
}

/// The addresses on a command line, for the broker: it grants the files
/// among them itself (REQ-1703 item 4).
pub(crate) fn given_addresses(given: impl Iterator<Item = String>) -> Vec<String> {
    arguments(given).map(|command_line| command_line.addresses).unwrap_or_default()
}

fn arguments(given: impl Iterator<Item = String>) -> Result<CommandLine, String> {
    let mut command_line = CommandLine { addresses: Vec::new(), allow_local: Vec::new(), no_keychain: !KEYCHAIN_BY_DEFAULT, input_trace: false };
    let mut arguments = given;
    while let Some(argument) = arguments.next() {
        if argument == "--allow-local" {
            command_line.allow_local.push(arguments.next().ok_or("--allow-local needs an address")?);
        } else if argument == NO_KEYCHAIN {
            command_line.no_keychain = true;
        } else if argument == KEYCHAIN {
            command_line.no_keychain = false;
        } else if argument == INPUT_TRACE {
            command_line.input_trace = true;
        } else if argument == launch::OPEN_LINK {
            // The broker refused any other shape of this launch (REQ-1673).
            let link = arguments.next().ok_or("--open-link needs a link")?;
            command_line.addresses.push(outside_link(&link)?.text);
        } else if argument == launch::OPEN_FILE {
            command_line.addresses.push(arguments.next().ok_or("--open-file needs a file")?);
        } else if is_outside_link(&argument) {
            command_line.addresses.push(outside_link(&argument)?.text);
        } else {
            command_line.addresses.push(argument);
        }
    }
    Ok(command_line)
}

/// Handles each event, and the timed work, until the browser is to end.
fn event_loop(core: &mut Core, incoming: &mpsc::Receiver<Event>) -> Result<(), String> {
    loop {
        // An event, or the time for timed work, whichever is first.
        let event = match core.next_wake() {
            Some(at) => match incoming.recv_timeout(at.saturating_duration_since(std::time::Instant::now())) {
                Ok(event) => Some(event),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
            },
            None => match incoming.recv() {
                Ok(event) => Some(event),
                Err(_) => break Ok(()),
            },
        };
        let Some(event) = event else {
            if let Err(error) = core.tick().and_then(|()| core.flush()) {
                break Err(error);
            }
            continue;
        };
        // RULE-66's hops: what the core's thread was doing when an event
        // waited for it.
        #[cfg(feature = "timed-test")]
        let (began, name) = (std::time::Instant::now(), timed::name(&event));
        // Timed work falls due while events are handled too: a feed that
        // just started, a notice that expired, the sleep check.
        let handled = handle(core, event).and_then(|go| core.tick().and_then(|()| core.flush()).map(|()| go));
        #[cfg(feature = "timed-test")]
        protocol::timed_test::flushed("core");
        #[cfg(feature = "timed-test")]
        timed::slow(began, &name);
        match handled {
            Ok(true) => {}
            Ok(false) => break Ok(()),
            Err(error) => break Err(error),
        }
    }
}

/// Ends every helper but the interface, which is told to close: a move not
/// yet kept, and remembered channels not yet written, are kept first. The
/// core goes with it, before the interface is waited for.
pub(super) fn shut_down(mut core: Core) {
    core.keep_window();
    core.viewer_state_at_exit();
    for id in core.fetchers.keys().copied().collect::<Vec<_>>() {
        core.stop_fetcher(id);
    }
    for (_, decoder) in std::mem::take(&mut core.decoders) {
        stop_decoder(decoder);
    }
    if let Some((mut helper, mut input)) = core.sound.take() {
        let _ = write_to_sound(&mut input, &ToSound::Close);
        let _ = input.flush();
        drop(input);
        end(&mut helper);
    }
    let _ = write_to_interface(&mut core.to_interface, &ToInterface::Close);
    let _ = core.to_interface.flush();
}

/// The browser's outcome once the interface has ended: an interface that
/// crashed or failed is a failure even when it closed its pipe cleanly, as
/// a crashing process does.
pub(super) fn ended(result: Result<(), String>, interface: &Exit) -> Result<(), String> {
    result?;
    if interface.success { Ok(()) } else { Err(format!("the interface ended abnormally ({interface})")) }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    /// A wait status as the kernel reports it: a signal number in the low
    /// bits, or an exit code shifted into the second byte.
    const SEGMENTATION_FAULT: i32 = 11;
    const EXIT_CODE_SHIFT: u32 = 8;

    #[test]
    fn a_crashed_or_failed_interface_fails_the_browser() {
        let clean = Exit::from(std::process::ExitStatus::from_raw(0));
        assert_eq!(ended(Ok(()), &clean), Ok(()));
        let crashed = Exit::from(std::process::ExitStatus::from_raw(SEGMENTATION_FAULT));
        assert!(ended(Ok(()), &crashed).is_err_and(|error| error.contains("abnormally")), "a crash is not a clean exit");
        let failed = Exit::from(std::process::ExitStatus::from_raw(1 << EXIT_CODE_SHIFT));
        assert!(ended(Ok(()), &failed).is_err());
        assert_eq!(ended(Err("first".into()), &crashed), Err("first".into()), "the first error is the one reported");
    }

    /// TODO-2006: the broker's end ends the core, which then ends its
    /// helpers, rather than leaving it running with no one to end it.
    #[cfg(unix)]
    #[test]
    fn the_brokers_end_ends_the_core() {
        let mut core = super::super::test_support::test_core();
        assert!(handle(&mut core, Event::BrokerEnded).is_err_and(|error| error.contains("broker ended")));
    }

    /// REQ-683, TODO-2071: a fetcher that broke the protocol and keeps
    /// running is killed, whether its reader saw a bad message or only its
    /// output closed, and nothing waits for it: the core's one event loop
    /// goes on. The line is its channel's, and no other channel's. On the
    /// code before, `handle` waited on the broker for as long as the
    /// fetcher chose to sleep.
    #[test]
    fn a_fetcher_still_running_holds_nothing_up_and_is_killed() {
        use super::super::processes::Fetcher;
        use super::super::test_support::{HELPER_LIMIT, ended_soon, sleeping_helper, test_core};
        use confine::Role;
        use protocol::console::ConsoleTab;

        for reason in [Some(String::from("a malformed message")), None] {
            let (child, output) = sleeping_helper(Role::Network);
            let mut core = test_core();
            core.console.set_open(true);
            core.fetchers.insert(1, Fetcher::new(child, 1, None));
            let begun = std::time::Instant::now();
            handle(&mut core, Event::FetcherProcessEnded(super::super::processes::places_of(&[(1, 1)]), reason)).unwrap();
            assert!(begun.elapsed() < HELPER_LIMIT, "the event loop waited {:?} for a fetcher still running", begun.elapsed());
            assert!(ended_soon(output), "killed");
            assert!(core.fetchers.is_empty());
            let stopped = |core: &mut Core, channel| {
                core.console
                    .updates(Some(channel))
                    .iter()
                    .flat_map(|update| update.entries.clone())
                    .any(|entry| entry.tab == ConsoleTab::Log && entry.text.contains("fetcher for channel 1 stopped"))
            };
            assert!(stopped(&mut core, 1), "in the channel's Log");
            assert!(!stopped(&mut core, 2), "and not the browser-wide one, which every channel's Log shows");
        }
    }

    /// REQ-683, TODO-2071: a sound helper that broke the protocol and keeps
    /// running is killed. On the code before, it was dropped and ran on.
    #[test]
    fn a_sound_helper_that_broke_the_protocol_is_killed() {
        use super::super::test_support::{ended_soon, sleeping_helper, test_core};
        use confine::Role;
        let (child, output) = sleeping_helper(Role::Sound);
        let mut core = test_core();
        core.sound = Some((child, Outbox::new(Vec::new())));
        handle(&mut core, Event::SoundEnded(Some(String::from("a malformed message")))).unwrap();
        assert!(ended_soon(output), "killed");
        assert!(core.sound.is_none());
        assert_eq!(core.strip().notice, SOUND_HELPER_BROKE, "said to every tab, whichever is in front");
    }

    /// REQ-683, requirement 4.3: a sound helper that broke its protocol served
    /// every tab, so every tab open says so, and goes on saying it after a
    /// notice's time, until it is reloaded. On the code before, the notice
    /// lasted ten seconds in whichever tab was in front.
    #[test]
    fn every_tab_a_broken_sound_helper_served_says_so_until_reloaded() {
        use super::super::test_support::{Test, sleeping_helper};
        use confine::Role;
        let mut test = Test::new("sound-broke-tabs");
        test.tab("https://one.example/teletext/", false);
        test.tab("https://two.example/teletext/", false);
        let (child, _output) = sleeping_helper(Role::Sound);
        test.core.sound = Some((child, Outbox::new(Vec::new())));
        handle(&mut test.core, Event::SoundEnded(Some(String::from("a malformed message")))).unwrap();
        // The notice's time is up.
        test.core.notice = None;
        for active in 0..test.core.session.tabs.len() {
            test.core.session.active = active;
            assert_eq!(test.core.strip().notice, SOUND_HELPER_BROKE, "tab {active}");
        }
        test.core.session.open("https://three.example/teletext/").unwrap();
        test.core.session.active = test.core.session.tabs.len() - 1;
        assert_eq!(test.core.strip().notice, "", "a tab opened after, which the helper never served");
    }

    /// Finding F-12: a helper is heard once it says it is confined, its
    /// first message, so one that ends after that, before its channel
    /// gave it a page or an answer, is not counted against the version; a
    /// helper that ends without having said so is, whatever else it said.
    #[test]
    fn a_helper_is_heard_once_it_says_it_is_confined() {
        use super::super::processes::{Decoder, Fetcher};
        use super::super::test_support::{healthy_once_settled, stand_in, test_core};
        use confine::Role;
        use protocol::decoder::FromDecoder;
        use protocol::fetcher::FromFetcher;

        let decoder = |core: &mut Core| {
            let mut child = stand_in("/usr/bin/false", &[], Role::Decoder);
            let input = Outbox::new(child.stdin.take().unwrap());
            core.decoders.insert(1, Decoder::new(child, input, 1));
        };
        let fetcher = |core: &mut Core| {
            core.fetchers.insert(1, Fetcher::new(stand_in("/usr/bin/false", &[], Role::Network), 1, None));
        };

        let mut core = test_core();
        decoder(&mut core);
        handle(&mut core, Event::Decoded(1, 1, FromDecoder::Confined)).unwrap();
        handle(&mut core, Event::DecoderEnded(1, 1, None)).unwrap();
        assert!(healthy_once_settled(&mut core), "a decoder that ended after it was confined is not counted");

        let mut core = test_core();
        decoder(&mut core);
        handle(&mut core, Event::Decoded(1, 1, FromDecoder::Updated { page: 0x100 })).unwrap();
        handle(&mut core, Event::DecoderEnded(1, 1, None)).unwrap();
        assert!(!healthy_once_settled(&mut core), "a decoder that never said it was confined failed the version");

        let mut core = test_core();
        fetcher(&mut core);
        handle(&mut core, Event::Fetched(1, 1, FromFetcher::Confined)).unwrap();
        handle(&mut core, Event::FetcherProcessEnded(super::super::processes::places_of(&[(1, 1)]), None)).unwrap();
        assert!(healthy_once_settled(&mut core), "a fetcher that ended after it was confined is not counted");

        let mut core = test_core();
        fetcher(&mut core);
        handle(&mut core, Event::Fetched(1, 1, FromFetcher::Notice("unreachable".into()))).unwrap();
        handle(&mut core, Event::FetcherProcessEnded(super::super::processes::places_of(&[(1, 1)]), None)).unwrap();
        assert!(!healthy_once_settled(&mut core), "a fetcher that never said it was confined failed the version");
    }
}

/// The command line's tests run on every platform; those above need Unix's wait statuses.
#[cfg(test)]
mod command_line_tests;

/// The timed test's marks of the core's own thread (page-rules.md
/// RULE-66), only in its build.
#[cfg(feature = "timed-test")]
mod timed {
    use super::Event;

    /// An event handled for longer than this is marked, with what it was:
    /// a third of a hop's two milliseconds in RULE-64.
    const SLOW_MICROSECONDS: u128 = 300;
    /// How much of a message's debugging form names it.
    const NAME_BYTES: usize = 48;

    /// A writer that keeps the first [`NAME_BYTES`] and then refuses, so
    /// that a frame's debugging form is never formatted whole.
    struct First(String);

    impl core::fmt::Write for First {
        fn write_str(&mut self, text: &str) -> core::fmt::Result {
            for character in text.chars() {
                if self.0.len() >= NAME_BYTES {
                    return Err(core::fmt::Error);
                }
                self.0.push(if character.is_whitespace() { '_' } else { character });
            }
            Ok(())
        }
    }

    fn first(what: &str, message: &dyn core::fmt::Debug) -> String {
        let mut name = First(format!("{what}:"));
        let _ = core::fmt::write(&mut name, format_args!("{message:?}"));
        name.0
    }

    /// What the event is, in a word or two.
    pub(super) fn name(event: &Event) -> String {
        match event {
            Event::Budgeted(event, _) => name(event),
            Event::Decoded(_, _, message) => first("decoded", message),
            Event::Fetched(_, _, message) => first("fetched", message),
            Event::Intent(intent) => first("intent", intent),
            _ => String::from("other"),
        }
    }

    /// Marks an event whose handling, with the timed work and the flush
    /// after it, took longer than [`SLOW_MICROSECONDS`].
    pub(super) fn slow(began: std::time::Instant, name: &str) {
        let took = began.elapsed().as_micros();
        if took > SLOW_MICROSECONDS {
            protocol::timed_test::mark("core", "slow-event", began, format_args!("{took} {name}"));
        }
    }
}
