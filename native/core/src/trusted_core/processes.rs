//! The helper processes the trusted core starts through the broker
//! (decision 56): a decoder, a fetcher, the bookmark reader, the
//! interface's pipe, and where a fetcher may connect.

use std::io::{BufReader, Read};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

use confine::broker::{Broker, Helper, Program};
use neoteletext::budget::Budget;
use neoteletext::outbox::Outbox;
use neoteletext::settings::{self as stored, Settings};
use protocol::decoder::{BookmarkEntry, FromDecoder, MAX_BOOKMARK_ENTRIES, MAX_CHUNK, ToDecoder, read_from_decoder, write_to_decoder};
use protocol::fetcher::read_from_fetcher;
use protocol::{ProtocolError, read_from_interface};

use super::Event;

pub(super) struct Decoder {
    pub(super) child: Helper,
    pub(super) input: Outbox,
    /// Which of the channel's decoders this is: one put to sleep may still
    /// report after the next has started, and is then not listened to.
    pub(super) generation: u64,
    /// It has said something: an end before then is a failure to start,
    /// counted against the version (finding F-12).
    pub(super) heard: bool,
}

pub(super) struct Fetcher {
    pub(super) child: Helper,
    /// As [`Decoder::generation`].
    pub(super) generation: u64,
    /// What a fetcher is given: input messages (REQ-INT-133), and the
    /// answers to its cookie questions (decision 62).
    pub(super) input: Option<Outbox>,
    /// As [`Decoder::heard`].
    pub(super) heard: bool,
}

/// Where a new fetcher may connect: the local addresses allowed, from the
/// command line and the settings, and the proxy of the settings.
pub(super) struct Reach {
    allow_local: Vec<String>,
    proxy: Option<String>,
}

impl Reach {
    pub(super) fn of(command_line: &[String], settings: &Settings) -> Self {
        let mut allow_local = command_line.to_vec();
        allow_local.extend(settings.local.iter().map(ToString::to_string));
        Self { allow_local, proxy: settings.proxy.as_ref().map(stored::Proxy::url) }
    }
}

/// Asks the broker for a helper (decision 56): it starts it confined as
/// its program's role says (decision 52), with an environment of only
/// what it needs, and passes the trusted core its pipes.
pub(super) fn spawn(broker: &Arc<Broker>, program: Program, arguments: &[String], local_network: bool) -> Result<Helper, String> {
    let helper = broker.spawn(program, arguments, local_network).map_err(|error| format!("cannot start the {}: {error}", program.name()))?;
    eprintln!("neoteletext: the {} started confined: {}", program.name(), helper.report);
    Ok(helper)
}

/// Ends a decoder: told to close, and killed rather than waited for, so
/// one that has stopped reading holds nothing up. It has nothing the
/// trusted core needs: what it was given is kept (decision 47).
pub(super) fn stop_decoder(mut decoder: Decoder) {
    let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Close);
    drop(decoder.input);
    end(&mut decoder.child);
}

/// Ends a helper: killed, then waited for, so that none is left running or
/// unreaped; what either says is of no use to a helper being given up.
pub(super) fn end(helper: &mut Helper) {
    let _ = helper.kill();
    let _ = helper.wait();
}

/// Starts a decoder for `channel`, whose messages arrive as events.
pub(super) fn start_decoder(broker: &Arc<Broker>, channel: u32, generation: u64, store_pages: usize, events: Sender<Event>) -> Result<Decoder, String> {
    let mut child = spawn(broker, Program::Decoder, &[String::from("--store-pages"), store_pages.to_string()], false)?;
    let output = child.stdout.take().ok_or("the decoder has no output")?;
    let input = Outbox::new(child.stdin.take().ok_or("the decoder has no input")?);
    relay(
        output,
        read_from_decoder,
        events,
        move |message| Event::Decoded(channel, generation, message),
        move |end| Event::DecoderEnded(channel, generation, end),
    );
    Ok(Decoder { child, input, generation, heard: false })
}

/// Starts a decoder to read another browser's bookmark file (decision
/// 71): it is given the file and reports what it holds, which arrives as
/// one event, and is ended.
pub(super) fn start_bookmark_reader(broker: &Arc<Broker>, file: Vec<u8>, events: Sender<Event>) -> Result<(), String> {
    let child = spawn(broker, Program::Decoder, &[String::from("--bookmarks")], false)?;
    read_bookmarks_with(child, file, events, BOOKMARK_READ_DEADLINE)
}

/// How long the bookmark reader has to read a file and say what it holds
/// before it is killed (finding F-85). Its largest file is 24 MiB
/// (`MAX_BOOKMARK_FILE`); its reading time is not measured, and ten
/// seconds is chosen as far above what a bounded, single-pass reader of
/// that much should need.
const BOOKMARK_READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Gives the bookmark reader `child` the file, and sends what it reports,
/// or, once `deadline` has passed with no answer, kills it and says so.
fn read_bookmarks_with(mut child: Helper, file: Vec<u8>, events: Sender<Event>, deadline: std::time::Duration) -> Result<(), String> {
    let output = child.stdout.take().ok_or("the bookmark reader has no output")?;
    let mut input = child.stdin.take().ok_or("the bookmark reader has no input")?;
    let (answer, answered) = std::sync::mpsc::channel();
    // The exchange on a thread of its own, which killing the reader ends.
    thread::spawn(move || {
        let exchange = move || -> Result<(Vec<BookmarkEntry>, u32), String> {
            // The reader takes the whole file before it says anything.
            for chunk in file.chunks(MAX_CHUNK) {
                write_to_decoder(&mut input, &ToDecoder::Data(chunk.to_vec())).map_err(|error| error.to_string())?;
            }
            write_to_decoder(&mut input, &ToDecoder::End).map_err(|error| error.to_string())?;
            let mut output = BufReader::new(output);
            let mut entries = Vec::new();
            loop {
                match read_from_decoder(&mut output).map_err(|error| error.to_string())? {
                    // Each folder's end follows its folder: twice the bound
                    // holds every honest file.
                    Some(FromDecoder::Bookmark(entry)) if entries.len() < 2 * MAX_BOOKMARK_ENTRIES => entries.push(entry),
                    Some(FromDecoder::BookmarksRead { skipped }) => return Ok((entries, skipped)),
                    Some(_) => return Err(String::from("the bookmark reader said what it may not")),
                    None => return Err(String::from("it is not a bookmark file the reader can read")),
                }
            }
        };
        let _ = answer.send(exchange());
    });
    thread::spawn(move || {
        let read = answered.recv_timeout(deadline).unwrap_or_else(|_| Err(String::from("the bookmark reader took too long")));
        end(&mut child);
        let _ = events.send(Event::BookmarksRead(read));
    });
    Ok(())
}

/// Starts a fetcher for the web channel at `address`, whose messages
/// arrive as events.
/// A fetcher is given its proxy only here, when it starts: a change of the
/// proxy reaches fetchers started after it (requirement 19.12).
pub(super) fn start_fetcher(
    broker: &Arc<Broker>,
    address: &str,
    reach: &Reach,
    channel: u32,
    generation: u64,
    events: Sender<Event>,
) -> Result<Fetcher, String> {
    let mut arguments = vec![address.to_owned()];
    for ip in &reach.allow_local {
        arguments.push(String::from("--allow-local"));
        arguments.push(ip.clone());
    }
    if let Some(proxy) = &reach.proxy {
        arguments.push(String::from("--proxy"));
        arguments.push(proxy.clone());
    }
    let mut child = spawn(broker, Program::Fetcher, &arguments, !reach.allow_local.is_empty())?;
    let output = child.stdout.take().ok_or("the fetcher has no output")?;
    relay(
        output,
        read_from_fetcher,
        events,
        move |message| Event::Fetched(channel, generation, message),
        move |end| Event::FetcherEnded(channel, generation, end),
    );
    let input = child.stdin.take().map(Outbox::new);
    Ok(Fetcher { child, generation, input, heard: false })
}

pub(super) fn read_interface(child: &mut Helper, events: Sender<Event>) -> Result<(), String> {
    let output = child.stdout.take().ok_or("the interface has no output")?;
    relay(output, read_from_interface, events, Event::Intent, Event::InterfaceEnded);
    Ok(())
}

/// A reader that counts the bytes read through it: a message's length on
/// the pipe, what its budget is charged.
struct Counting<R> {
    inner: R,
    bytes: usize,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.bytes = self.bytes.saturating_add(read);
        Ok(read)
    }
}

/// Reads a helper's messages on a thread of its own, each sent to the
/// trusted core's thread as the event `said` makes of it, until the pipe
/// ends or breaks the protocol, which is sent as the event `ended` makes
/// of why; it stops early once the trusted core has gone. Each message's
/// bytes are taken from the helper's own budget before it is sent, and
/// given back once the core has handled it, so a helper that writes
/// faster than the core handles stalls on its own pipe (finding F-10).
pub(super) fn relay<M: 'static>(
    output: impl Read + Send + 'static,
    read: fn(&mut dyn Read) -> Result<Option<M>, ProtocolError>,
    events: Sender<Event>,
    said: impl Fn(M) -> Event + Send + 'static,
    ended: impl FnOnce(Option<String>) -> Event + Send + 'static,
) {
    thread::spawn(move || {
        let budget = Budget::default();
        let mut output = Counting { inner: BufReader::new(output), bytes: 0 };
        let end = loop {
            let before = output.bytes;
            match read(&mut output) {
                Ok(Some(message)) => {
                    let ticket = budget.take(output.bytes.saturating_sub(before));
                    if events.send(Event::Budgeted(Box::new(said(message)), ticket)).is_err() {
                        return;
                    }
                }
                Ok(None) => break None,
                Err(error) => break Some(error.to_string()),
            }
        };
        let _ = events.send(ended(end));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    /// The dots of a page image (`FromDecoder::Frame`).
    const FRAME_DOTS: usize = 480 * 500;
    /// How long the trusted core handles nothing in the flood test.
    const HOLD: Duration = Duration::from_secs(1);
    /// Where a flood is stopped when nothing holds it: four times the
    /// budget, far past what the bound lets through.
    const FLOOD_CAP: usize = 4 * neoteletext::outbox::MAX_QUEUED_BYTES;
    /// What a pipe and the reader's buffer hold beyond the budget: Linux's
    /// largest pipe without privileges (`/proc/sys/fs/pipe-max-size`, 1 MiB)
    /// is more than macOS's 64 KiB and the reader's 8 KiB together.
    const PIPE_AND_READER_BUFFERS: usize = 1024 * 1024;

    /// Finding F-85: a bookmark reader that never answers is ended at its
    /// deadline, and the import says it was not imported.
    #[cfg(unix)]
    #[test]
    fn a_bookmark_reader_that_never_answers_is_ended() {
        const DEADLINE: Duration = Duration::from_millis(200);
        let program = std::path::PathBuf::from("/bin/sleep");
        let arguments = [std::ffi::OsString::from("60")];
        let spawn =
            confine::Spawn { program: &program, arguments: &arguments, role: confine::Role::Decoder, environment: &[], local_network: false, writable: None };
        let child = Helper::local(confine::spawn(&spawn).unwrap());
        let (events, incoming) = mpsc::channel();
        read_bookmarks_with(child, b"<!DOCTYPE NETSCAPE-Bookmark-file-1>".to_vec(), events, DEADLINE).unwrap();
        let answer = incoming.recv_timeout(Duration::from_secs(5));
        assert!(matches!(&answer, Ok(Event::BookmarksRead(Err(reason))) if reason == "the bookmark reader took too long"), "no answer in five seconds");
    }

    /// Finding F-10: a helper that writes faster than the trusted core
    /// handles its messages is held at its budget, on its own pipe, so the
    /// core's queue of its messages does not grow without bound.
    #[test]
    fn a_helper_that_floods_the_core_is_held_at_its_budget() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let (events, incoming) = mpsc::channel();
        relay(reader, read_from_decoder, events, |message| Event::Decoded(1, 0, message), |end| Event::DecoderEnded(1, 0, end));
        let mut frame = Vec::new();
        protocol::decoder::write_frame_to_core(&mut frame, 0x100, &[[0, 0, 0]], 0, &vec![0; FRAME_DOTS]).unwrap();
        let frame_bytes = frame.len();
        let written = Arc::new(AtomicUsize::new(0));
        let theirs = Arc::clone(&written);
        let flood = thread::spawn(move || {
            while theirs.load(Ordering::Relaxed) < FLOOD_CAP && writer.write_all(&frame).is_ok() {
                theirs.fetch_add(frame.len(), Ordering::Relaxed);
            }
        });
        thread::sleep(HOLD);
        let held_at = written.load(Ordering::Relaxed);
        // The core takes its events, then goes: the helper's writes end.
        drop(incoming);
        flood.join().unwrap();
        let most = neoteletext::outbox::MAX_QUEUED_BYTES + PIPE_AND_READER_BUFFERS + frame_bytes;
        assert!(held_at <= most, "the helper wrote {held_at} bytes while the core handled nothing; at most {most}");
    }

    /// Requirement 19.12: a fetcher is started with the proxy and local
    /// addresses of the settings.
    #[test]
    fn a_fetcher_is_given_the_proxy_and_the_local_addresses() {
        let settings =
            Settings { proxy: stored::Proxy::from_parts("http", "p.example", 3128), local: vec!["10.0.0.2".parse().unwrap()], ..Settings::default() };
        let reach = Reach::of(&["192.168.0.9".to_owned()], &settings);
        assert_eq!(reach.allow_local, ["192.168.0.9", "10.0.0.2"]);
        assert_eq!(reach.proxy.as_deref(), Some("http://p.example:3128"));
        assert_eq!(Reach::of(&[], &Settings::default()).proxy, None);
    }
}
