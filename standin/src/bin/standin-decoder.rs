//! The stand-in decoder process: reads `Show`, `Tick` and `Close` on
//! standard input and writes a `Frame` for every page shown, and again for
//! every shown page when the flash phase changes. It has no network and no
//! file access of its own; it only renders.

use std::collections::BTreeSet;
use std::io::{self, BufReader, BufWriter};
use std::process::ExitCode;

use standin::page::render;
use standin::pipe::{self, Message};

// A channel has at most 2,048 page numbers (eight magazines of 256), so
// the set of shown pages is bounded by that.
const MAX_SHOWN: usize = 8 * 256;

fn run() -> Result<(), pipe::PipeError> {
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = BufWriter::new(io::stdout().lock());
    let mut shown: BTreeSet<(u32, u16)> = BTreeSet::new();
    let mut flash_on = false;

    while let Some(message) = pipe::read(&mut input)? {
        match message {
            Message::Show { channel, page, subcode: _ } => {
                if shown.len() < MAX_SHOWN {
                    shown.insert((channel, page));
                }
                pipe::write(&mut output, &Message::Frame(render(channel, page, flash_on)))?;
            }
            Message::Tick { flash_on: phase } => {
                flash_on = phase;
                for &(channel, page) in &shown {
                    pipe::write(&mut output, &Message::Frame(render(channel, page, flash_on)))?;
                }
            }
            Message::Close => break,
            // A decoder never receives frames; one that arrives is a fault
            // in the sender, and the decoder stops (requirement 4.3).
            Message::Frame(_) => return Err(pipe::PipeError::BadValue("frame sent to the decoder")),
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("standin-decoder: {error}");
            ExitCode::FAILURE
        }
    }
}
