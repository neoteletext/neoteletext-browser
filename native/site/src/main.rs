//! A development tool for T42 Browser, not shipped:
//!
//! - `t42-site build <folder> <sample>...` builds the fictional channel
//!   "Halcyon Teletext", every page from 100 to 899, in `<folder>/halcyon/`,
//!   from sample recordings (`.t42` files, or folders with
//!   `binaries/NNN.bin` per page).
//! - `t42-site serve <folder> [port]` serves it, with push updates to its
//!   live pages, at `http://127.0.0.1:<port>/halcyon/` (port 8070 unless
//!   given).

mod build;
mod serve;

use std::path::PathBuf;
use std::process::ExitCode;

const CHANNEL: &str = "halcyon";
const DEFAULT_PORT: u16 = 8070;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments.first().map(String::as_str) {
        Some("build") if arguments.len() >= 3 => {
            let folder = PathBuf::from(&arguments[1]).join(CHANNEL);
            let sources: Vec<PathBuf> = arguments[2..].iter().map(PathBuf::from).collect();
            let push = format!("http://127.0.0.1:{DEFAULT_PORT}/{CHANNEL}/push");
            build::build(&folder, &push, &sources).map(|built| {
                let live: Vec<String> = built.live.iter().map(|page| format!("{page:03X}")).collect();
                eprintln!("t42-site: {}: {} pages, live pages {}", folder.display(), built.pages, live.join(" "));
            })
        }
        Some("serve") if arguments.len() >= 2 => {
            let port = arguments.get(2).and_then(|port| port.parse().ok()).unwrap_or(DEFAULT_PORT);
            serve::run(&PathBuf::from(&arguments[1]), CHANNEL, port)
        }
        _ => Err("usage: t42-site build <folder> <sample>... | t42-site serve <folder> [port]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("t42-site: {error}");
            ExitCode::FAILURE
        }
    }
}
