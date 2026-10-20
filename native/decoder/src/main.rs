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
use std::io::{self, BufReader, BufWriter};
use std::process::ExitCode;
use std::time::Instant;

use decoder::{Engine, Rendered};
use protocol::ProtocolError;
use protocol::decoder::{FromDecoder, ToDecoder, read_from_core, write_to_core};

fn run() -> Result<(), ProtocolError> {
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = BufWriter::new(io::stdout().lock());
    let mut engine = Engine::new();
    let mut showing: Vec<u16> = Vec::new();
    // The last frame sent of each page shown, so an unchanged page is
    // never sent again.
    let mut sent: BTreeMap<u16, Rendered> = BTreeMap::new();
    // The audio receiver's clock (the engine's REQ-AUD-014).
    let started = Instant::now();
    let now_ms = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let render =
        |engine: &Engine, pages: &[u16], sent: &mut BTreeMap<u16, Rendered>, output: &mut BufWriter<io::StdoutLock<'_>>| -> Result<(), ProtocolError> {
            for &page in pages {
                let Some(rendered) = engine.render(page) else { continue };
                let before = sent.get(&page);
                if before == Some(&rendered) {
                    continue;
                }
                // A page's links go before its image, so the trusted core
                // has them for the image when it arrives (`hyperlinks.md`
                // section 8.1).
                let new_image = before.is_none_or(|before| before.palette != rendered.palette || before.indices != rendered.indices);
                #[cfg(feature = "links")]
                if before.map(|before| &before.links) != Some(&rendered.links) {
                    let links = rendered.links.clone();
                    write_to_core(output, &FromDecoder::Links { page, with_image: new_image, candidates: links.candidates, numbers: links.numbers })?;
                }
                if new_image {
                    write_to_core(output, &FromDecoder::Frame { page, palette: rendered.palette.clone(), indices: rendered.indices.clone() })?;
                }
                sent.insert(page, rendered);
            }
            Ok(())
        };

    while let Some(message) = read_from_core(&mut input)? {
        engine.tick_audio(now_ms());
        let arrived = match message {
            ToDecoder::Data(bytes) => engine.feed(&bytes),
            ToDecoder::Whole { packets, pushed, bytes } => engine.feed_whole(packets, pushed, &bytes),
            ToDecoder::End => engine.finish(),
            ToDecoder::Showing(pages) => {
                sent.retain(|page, _| pages.contains(page));
                showing = pages;
                render(&engine, &showing, &mut sent, &mut output)?;
                continue;
            }
            ToDecoder::Tick => {
                send_sounds(&mut engine, &mut output)?;
                render(&engine, &showing, &mut sent, &mut output)?;
                continue;
            }
            ToDecoder::Close => break,
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
        // A shown page that just arrived, or changed, is drawn at once.
        render(&engine, &showing, &mut sent, &mut output)?;
    }
    Ok(())
}

/// Sends the voice operations the audio receiver produced, and the
/// cursor records found.
fn send_sounds(engine: &mut Engine, output: &mut BufWriter<io::StdoutLock<'_>>) -> Result<(), ProtocolError> {
    for audio in engine.take_sounds() {
        write_to_core(output, &FromDecoder::Audio(audio))?;
    }
    #[cfg(feature = "interaction")]
    for body in engine.take_cursors() {
        write_to_core(output, &FromDecoder::Cursor { body })?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("t42-decoder: {error}");
            ExitCode::FAILURE
        }
    }
}
