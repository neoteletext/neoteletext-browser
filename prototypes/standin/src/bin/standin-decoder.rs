//! The stand-in decoder as a program of its own; see [`standin::decoder`].

use std::process::ExitCode;

fn main() -> ExitCode {
    match standin::decoder::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("standin-decoder: {error}");
            ExitCode::FAILURE
        }
    }
}
