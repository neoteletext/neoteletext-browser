//! The decoder process itself: what the trusted core started it for, the
//! loop that feeds the engine a channel's bytes and sends the pages shown,
//! and the reader of a bookmark file. `main.rs` confines the process and
//! calls these.

use std::collections::BTreeMap;
use std::io::{self, BufReader, BufWriter, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::{Engine, Flashing, Rendered};
use protocol::ProtocolError;
use protocol::decoder::{FIRST_STORE_PAGES, FromDecoder, MAX_BOOKMARK_FILE, MAX_STORE_PAGES, ToDecoder, read_from_core, write_frame_to_core, write_to_core};

/// Microseconds in a millisecond: the audio receiver keeps milliseconds.
const MICROSECONDS_PER_MILLISECOND: u64 = 1000;

type Output<'a> = BufWriter<io::StdoutLock<'a>>;

/// What the trusted core started the decoder for.
pub enum Mode {
    /// A channel, with a store of this many pages.
    Channel(usize),
    /// To read one bookmark file (decision 71).
    Bookmarks,
}

/// The mode the trusted core asked for: a channel's store size,
/// `--store-pages N` from [`FIRST_STORE_PAGES`] to [`MAX_STORE_PAGES`], or
/// `--bookmarks`.
///
/// # Errors
/// The usage, when the arguments are none of these.
pub fn mode() -> Result<Mode, String> {
    let mut arguments = std::env::args().skip(1);
    match (arguments.next().as_deref(), arguments.next().map(|value| value.parse::<usize>())) {
        (None, _) => Ok(Mode::Channel(FIRST_STORE_PAGES)),
        (Some("--store-pages"), Some(Ok(pages))) if (FIRST_STORE_PAGES..=MAX_STORE_PAGES).contains(&pages) => Ok(Mode::Channel(pages)),
        (Some("--bookmarks"), None) => Ok(Mode::Bookmarks),
        _ => Err(format!("usage: neoteletext-decoder [--store-pages {FIRST_STORE_PAGES} to {MAX_STORE_PAGES} | --bookmarks]")),
    }
}

/// Reads a bookmark file the trusted core sends as data, to its end, and
/// reports what it holds (decision 71).
///
/// # Errors
/// When the pipe fails, or the trusted core sends anything but the file.
pub fn read_bookmarks() -> Result<(), ProtocolError> {
    let mut input = BufReader::new(io::stdin().lock());
    let mut file: Vec<u8> = Vec::new();
    loop {
        match read_from_core(&mut input)? {
            Some(ToDecoder::Data(bytes)) => {
                if file.len().saturating_add(bytes.len()) > MAX_BOOKMARK_FILE {
                    return Err(ProtocolError::TooLong(file.len().saturating_add(bytes.len())));
                }
                file.extend_from_slice(&bytes);
            }
            Some(ToDecoder::End) => break,
            Some(_) => return Err(ProtocolError::BadValue("a bookmark reader takes only data")),
            None => return Ok(()),
        }
    }
    let mut output = BufWriter::new(io::stdout().lock());
    match crate::netscape::read(&file) {
        Ok(found) => {
            for entry in found.entries {
                write_to_core(&mut output, &FromDecoder::Bookmark(entry))?;
            }
            write_to_core(&mut output, &FromDecoder::BookmarksRead { skipped: found.skipped })?;
        }
        Err(not_read) => eprintln!("neoteletext-decoder: not a bookmark file this reader reads: {not_read:?}"),
    }
    output.flush()?;
    Ok(())
}

/// A channel's decoder, with a store of `store_pages`: it reads the
/// channel's bytes and requests from the trusted core and sends back which
/// pages arrived and the pages shown, until the channel closes. Its first
/// message says it is confined, as `main.rs` has made it.
///
/// # Errors
/// When the pipe fails or a message from the trusted core is malformed.
pub fn run(store_pages: usize) -> Result<(), ProtocolError> {
    let mut output = BufWriter::new(io::stdout().lock());
    // Before it reads anything, so the trusted core can tell a decoder
    // that could not start from one its channel's data ended (finding
    // F-12).
    write_to_core(&mut output, &FromDecoder::Confined)?;
    output.flush()?;
    let mut engine = Engine::with_store(store_pages);
    let mut showing: Vec<u16> = Vec::new();
    // The last frame sent of each page shown, so an unchanged page is
    // never sent again.
    let mut sent: BTreeMap<u16, Rendered> = BTreeMap::new();
    // A page's worth of room for the next render, from the last frame not kept.
    let mut spare: Vec<u8> = Vec::new();
    let mut flashing = Flashing::default();
    // The trusted core's messages are read on a thread of their own, so
    // the loop also wakes, by itself, for flash and for its sounds' clock:
    // it sleeps until a message, the next flash phase of a shown page
    // that flashes, or the audio receiver's next tick, whichever is first,
    // and with none of these, until a message (decision 47).
    let incoming = read_messages();

    loop {
        // What the last message or deadline produced goes before waiting.
        output.flush()?;
        let now = engine.now_us();
        let audio = engine.next_audio_tick_ms().map(|ms| ms.saturating_mul(MICROSECONDS_PER_MILLISECOND));
        let deadline = [flashing.next_us(now), audio].into_iter().flatten().min();
        let received = match wait(&incoming, deadline, now) {
            Woken::Message(message) => Some(message),
            Woken::Deadline => None,
            Woken::Ended => return Ok(()),
        };
        let now = engine.now_us();
        engine.tick_audio(now.checked_div(MICROSECONDS_PER_MILLISECOND).unwrap_or(0));
        let Some(message) = received else {
            // A deadline: the sounds' clock, and the flashing pages whose
            // phase has changed.
            send_sounds(&mut engine, &mut output)?;
            draw(&engine, &flashing.due(now), now, &mut sent, &mut spare, &mut flashing, &mut output)?;
            continue;
        };
        let Some(message) = message? else { return Ok(()) };
        let arrived = match message {
            ToDecoder::Data(bytes) => engine.feed(&bytes),
            ToDecoder::Whole { packets, pushed, bytes } => engine.feed_whole(packets, pushed, &bytes),
            ToDecoder::End => engine.finish(),
            ToDecoder::Showing(pages) => {
                sent.retain(|page, _| pages.contains(page));
                flashing.retain(&pages);
                showing = pages;
                let fresh: Vec<u16> = showing.iter().copied().filter(|page| !sent.contains_key(page)).collect();
                draw(&engine, &fresh, now, &mut sent, &mut spare, &mut flashing, &mut output)?;
                continue;
            }
            ToDecoder::Close => return Ok(()),
            ToDecoder::Opened(page) => {
                engine.open_page(page);
                send_sounds(&mut engine, &mut output)?;
                continue;
            }
            // The open page's reveal, hold, mix or sub-page changed: drawn
            // again at once if on screen.
            ToDecoder::View(view) => {
                engine.set_view(view);
                if showing.contains(&view.page) {
                    draw(&engine, &[view.page], now, &mut sent, &mut spare, &mut flashing, &mut output)?;
                }
                continue;
            }
        };
        report(&mut engine, &arrived, &mut output)?;
        // A shown page drawn again only when its reception moved, or data
        // every page may draw from changed.
        let changed = engine.take_changed();
        if engine.take_store_full() {
            write_to_core(&mut output, &FromDecoder::StoreFull)?;
        }
        let pages: Vec<u16> = showing.iter().copied().filter(|page| changed.everything || changed.pages.contains(page)).collect();
        draw(&engine, &pages, now, &mut sent, &mut spare, &mut flashing, &mut output)?;
    }
}

/// A message read from the trusted core: one, the end of its pipe, or an
/// error.
type Received = Result<Option<ToDecoder>, ProtocolError>;

/// What the loop woke for.
enum Woken {
    /// A message from the trusted core.
    Message(Received),
    /// The deadline passed first.
    Deadline,
    /// The reader of the trusted core's messages ended.
    Ended,
}

/// Reads the trusted core's messages on a thread of their own, until its
/// pipe ends or a message cannot be read.
fn read_messages() -> Receiver<Received> {
    let (messages, incoming) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = BufReader::new(io::stdin().lock());
        loop {
            let message = read_from_core(&mut input);
            let last = !matches!(message, Ok(Some(_)));
            if messages.send(message).is_err() || last {
                return;
            }
        }
    });
    incoming
}

/// Waits for a message until `deadline` (microseconds by the engine's
/// clock, which reads `now`), or with none for as long as it takes.
fn wait(incoming: &Receiver<Received>, deadline: Option<u64>, now: u64) -> Woken {
    match deadline {
        Some(at) => match incoming.recv_timeout(Duration::from_micros(at.saturating_sub(now))) {
            Ok(message) => Woken::Message(message),
            Err(RecvTimeoutError::Timeout) => Woken::Deadline,
            Err(RecvTimeoutError::Disconnected) => Woken::Ended,
        },
        None => incoming.recv().map_or(Woken::Ended, Woken::Message),
    }
}

/// Sends what the bytes fed produced: the sounds, the pages that arrived,
/// the pages received again and the channel's log lines.
fn report(engine: &mut Engine, arrived: &[(u16, String)], output: &mut Output<'_>) -> Result<(), ProtocolError> {
    send_sounds(engine, output)?;
    for (page, title) in arrived {
        write_to_core(output, &FromDecoder::Arrived { page: *page, title: title.clone() })?;
    }
    for page in engine.take_updated() {
        write_to_core(output, &FromDecoder::Updated { page })?;
    }
    for log in engine.take_logs() {
        write_to_core(output, &FromDecoder::ChannelLog { level: log.level, page: log.page, text: log.text })?;
    }
    Ok(())
}

/// Draws `pages` at `now_us` and sends those that changed: a page's links
/// go before its image, so the trusted core has them for the image when it
/// arrives (`hyperlinks.md` section 8.1).
fn draw(
    engine: &Engine,
    pages: &[u16],
    now_us: u64,
    sent: &mut BTreeMap<u16, Rendered>,
    spare: &mut Vec<u8>,
    flashing: &mut Flashing,
    output: &mut Output<'_>,
) -> Result<(), ProtocolError> {
    for &page in pages {
        // Rendered into the room kept from the last frame not sent, over
        // the page's last frame sent: only the rows that changed since.
        let last = sent.get(&page).map(|last| last.indices.as_slice());
        let Some(rendered) = engine.render_over(page, now_us, last, core::mem::take(spare)) else { continue };
        flashing.drawn(page, &rendered);
        let before = sent.get(&page);
        if before == Some(&rendered) {
            *spare = rendered.indices;
            continue;
        }
        let new_image =
            before.is_none_or(|before| before.palette != rendered.palette || before.screen != rendered.screen || before.indices != rendered.indices);
        #[cfg(feature = "links")]
        if before.map(|before| &before.links) != Some(&rendered.links) {
            let links = rendered.links.clone();
            let message = FromDecoder::Links { page, with_image: new_image, candidates: links.candidates, numbers: links.numbers, fastext: links.fastext };
            write_to_core(output, &message)?;
        }
        if before.map(|before| &before.subpages) != Some(&rendered.subpages) {
            let (subcodes, shown) = rendered.subpages.clone();
            write_to_core(output, &FromDecoder::SubPages { page, subcodes, shown })?;
        }
        if new_image {
            write_frame_to_core(output, page, &rendered.palette, rendered.screen, &rendered.indices)?;
        }
        if let Some(replaced) = sent.insert(page, rendered) {
            *spare = replaced.indices;
        }
    }
    Ok(())
}

/// Sends the voice operations the audio receiver produced, and the
/// channel's records for its input found.
fn send_sounds(engine: &mut Engine, output: &mut Output<'_>) -> Result<(), ProtocolError> {
    for audio in engine.take_sounds() {
        write_to_core(output, &FromDecoder::Audio(audio))?;
    }
    #[cfg(feature = "interaction")]
    {
        let input = engine.take_input();
        for body in input.cursors {
            write_to_core(output, &FromDecoder::Cursor { body })?;
        }
        for body in input.controls {
            write_to_core(output, &FromDecoder::InputControl { body })?;
        }
    }
    Ok(())
}
