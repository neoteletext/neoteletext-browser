//! The trusted core's run (decisions 46 and 53): start-up, the event loop,
//! each event handled, and shutdown of every helper.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;

use confine::broker::{Broker, Exit, Program};
use neoteletext::outbox::Outbox;
use protocol::console::Level;
use protocol::sound::{FromSound, ToSound, write_to_sound};
use protocol::{ToInterface, write_to_interface};

use super::processes::{end, read_interface, spawn, stop_decoder};
use super::startup::{load_bookmarks, load_history, load_session, load_settings, open_at_start};
use super::{Core, Event};
use crate::session::Session;

/// Handles one event; `Ok(false)` once the browser is to end.
fn handle(core: &mut Core, event: Event) -> Result<bool, String> {
    match event {
        // Handled, then its bytes given back to its helper's budget.
        Event::Budgeted(event, ticket) => {
            let go = handle(core, *event);
            drop(ticket);
            return go;
        }
        Event::Decoded(channel, generation, message) => {
            core.decoder_heard(channel, generation);
            if core.decoders.get(&channel).map(|decoder| decoder.generation) == Some(generation) {
                core.in_window_of(channel, |core| core.decoded(channel, message))?;
            }
        }
        Event::Fetched(channel, generation, message) => {
            core.fetcher_heard(channel, generation);
            if core.fetchers.get(&channel).map(|fetcher| fetcher.generation) == Some(generation) {
                core.in_window_of(channel, |core| core.fetched(channel, message))?;
            }
        }
        Event::FetcherEnded(channel, generation, reason) => core.fetcher_ended(channel, generation, reason),
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
        Event::SoundEnded(reason) => {
            core.log(Level::Error, None, &format!("no sound: the sound helper ended{}", reason.map(|reason| format!(": {reason}")).unwrap_or_default()));
            core.sound = None;
            core.sounding(&[])?;
        }
        Event::BookmarksRead(read) => core.bookmarks_read(read)?,
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
    }
    Ok(true)
}

pub(crate) fn run(broker: &Arc<Broker>, given: impl Iterator<Item = String>, confined: &str) -> Result<(), String> {
    // What happens before the trusted core is made, for its Log tab.
    let mut log = vec![(Level::Info, format!("trusted core: confined: {confined}"))];
    let (addresses, allow_local) = arguments(given)?;
    let mut interface = spawn(broker, Program::Interface, &[], false)?;
    log.push((Level::Info, format!("interface: confined: {}", interface.report)));
    let (events, incoming) = mpsc::channel();
    read_interface(&mut interface, events.clone())?;

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
    core.cookies_at_start();
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

    let result = event_loop(&mut core, &incoming);
    shut_down(core);
    if result.is_err() {
        let _ = interface.kill();
    }
    let status = interface.wait().map_err(|error| format!("cannot wait for the interface: {error}"))?;
    ended(result, &status)
}

/// The browser's own arguments: the addresses to open, and each local
/// address `--allow-local` lets a fetcher reach.
fn arguments(given: impl Iterator<Item = String>) -> Result<(Vec<String>, Vec<String>), String> {
    let mut addresses = Vec::new();
    let mut allow_local = Vec::new();
    let mut arguments = given;
    while let Some(argument) = arguments.next() {
        if argument == "--allow-local" {
            allow_local.push(arguments.next().ok_or("--allow-local needs an address")?);
        } else {
            addresses.push(argument);
        }
    }
    Ok((addresses, allow_local))
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
        // Timed work falls due while events are handled too: a feed that
        // just started, a notice that expired, the sleep check.
        match handle(core, event).and_then(|go| core.tick().and_then(|()| core.flush()).map(|()| go)) {
            Ok(true) => {}
            Ok(false) => break Ok(()),
            Err(error) => break Err(error),
        }
    }
}

/// Ends every helper but the interface, which is told to close: a move not
/// yet kept, and cookies not yet written or to remove, are kept first. The
/// core goes with it, before the interface is waited for.
fn shut_down(mut core: Core) {
    core.keep_window();
    core.cookies_at_exit();
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
}
