//! The decoder process: it reads a channel's bytes and requests from the
//! trusted core on standard input, and writes which pages arrived and the
//! pages it renders on standard output. It opens no file and no network
//! connection; on macOS the trusted core starts it in a sandbox that
//! denies both.

use std::collections::BTreeMap;
use std::io::{self, BufReader, BufWriter};
use std::process::ExitCode;

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

    let render =
        |engine: &Engine, pages: &[u16], sent: &mut BTreeMap<u16, Rendered>, output: &mut BufWriter<io::StdoutLock<'_>>| -> Result<(), ProtocolError> {
            for &page in pages {
                let Some(rendered) = engine.render(page) else { continue };
                if sent.get(&page) == Some(&rendered) {
                    continue;
                }
                write_to_core(output, &FromDecoder::Frame { page, palette: rendered.palette.clone(), indices: rendered.indices.clone() })?;
                sent.insert(page, rendered);
            }
            Ok(())
        };

    while let Some(message) = read_from_core(&mut input)? {
        let arrived = match message {
            ToDecoder::Data(bytes) => engine.feed(&bytes),
            ToDecoder::End => engine.finish(),
            ToDecoder::Showing(pages) => {
                sent.retain(|page, _| pages.contains(page));
                showing = pages;
                render(&engine, &showing, &mut sent, &mut output)?;
                continue;
            }
            ToDecoder::Tick => {
                render(&engine, &showing, &mut sent, &mut output)?;
                continue;
            }
            ToDecoder::Close => break,
        };
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

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("t42-decoder: {error}");
            ExitCode::FAILURE
        }
    }
}
