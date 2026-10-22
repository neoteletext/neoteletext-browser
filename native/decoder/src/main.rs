//! The decoder process: it reads a channel's bytes and requests from the
//! trusted core on standard input, and writes which pages arrived and the
//! pages it renders on standard output. It opens no file and no network
//! connection; on macOS the trusted core starts it in a sandbox that
//! denies both.

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::collections::BTreeMap;
use std::io::{self, BufReader, BufWriter, Write};
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use decoder::{Engine, Flashing, Rendered};
use protocol::ProtocolError;
use protocol::decoder::{FIRST_STORE_PAGES, FromDecoder, MAX_STORE_PAGES, ToDecoder, read_from_core, write_frame_to_core, write_to_core};

/// Microseconds in a millisecond: the audio receiver keeps milliseconds.
const MICROSECONDS_PER_MILLISECOND: u64 = 1000;

type Output<'a> = BufWriter<io::StdoutLock<'a>>;

/// The store size the trusted core asked for: `--store-pages N`, from
/// [`FIRST_STORE_PAGES`] to [`MAX_STORE_PAGES`].
fn store_pages() -> Result<usize, String> {
    let mut arguments = std::env::args().skip(1);
    match (arguments.next().as_deref(), arguments.next().map(|value| value.parse::<usize>())) {
        (None, _) => Ok(FIRST_STORE_PAGES),
        (Some("--store-pages"), Some(Ok(pages))) if (FIRST_STORE_PAGES..=MAX_STORE_PAGES).contains(&pages) => Ok(pages),
        _ => Err(format!("usage: neoteletext-decoder [--store-pages {FIRST_STORE_PAGES} to {MAX_STORE_PAGES}]")),
    }
}

fn run(store_pages: usize) -> Result<(), ProtocolError> {
    let mut output = BufWriter::new(io::stdout().lock());
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

    loop {
        // What the last message or deadline produced goes before waiting.
        output.flush()?;
        let now = engine.now_us();
        let audio = engine.next_audio_tick_ms().map(|ms| ms.saturating_mul(MICROSECONDS_PER_MILLISECOND));
        let deadline = [flashing.next_us(now), audio].into_iter().flatten().min();
        let received = match deadline {
            Some(at) => match incoming.recv_timeout(Duration::from_micros(at.saturating_sub(now))) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            },
            None => match incoming.recv() {
                Ok(message) => Some(message),
                Err(_) => return Ok(()),
            },
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
        };
        send_sounds(&mut engine, &mut output)?;
        for (page, title) in &arrived {
            write_to_core(&mut output, &FromDecoder::Arrived { page: *page, title: title.clone() })?;
        }
        for page in engine.take_updated() {
            write_to_core(&mut output, &FromDecoder::Updated { page })?;
        }
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
        // Rendered into the room kept from the last frame not sent.
        let Some(rendered) = engine.render_into(page, now_us, core::mem::take(spare)) else { continue };
        flashing.drawn(page, &rendered);
        let before = sent.get(&page);
        if before == Some(&rendered) {
            *spare = rendered.indices;
            continue;
        }
        let new_image = before.is_none_or(|before| before.palette != rendered.palette || before.indices != rendered.indices);
        #[cfg(feature = "links")]
        if before.map(|before| &before.links) != Some(&rendered.links) {
            let links = rendered.links.clone();
            write_to_core(output, &FromDecoder::Links { page, with_image: new_image, candidates: links.candidates, numbers: links.numbers })?;
        }
        if new_image {
            write_frame_to_core(output, page, &rendered.palette, &rendered.indices)?;
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

fn main() -> ExitCode {
    // First, before any thread: its pipes and nothing else (requirement 3.4).
    match confine::apply(confine::Role::Decoder, None) {
        Ok(report) => eprintln!("neoteletext-decoder: confined: {report}"),
        Err(error) => {
            eprintln!("neoteletext-decoder: {error}");
            return ExitCode::FAILURE;
        }
    }
    let pages = match store_pages() {
        Ok(pages) => pages,
        Err(usage) => {
            eprintln!("neoteletext-decoder: {usage}");
            return ExitCode::FAILURE;
        }
    };
    match run(pages) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-decoder: {error}");
            ExitCode::FAILURE
        }
    }
}
