//! A development tool for T42 Browser, not shipped:
//!
//! - `t42-site build <folder> <sample>...` builds the fictional channel
//!   "Halcyon Teletext", every page from 100 to 899, in `<folder>/halcyon/`,
//!   from sample recordings (`.t42` files, or folders with
//!   `binaries/NNN.bin` per page).
//! - `t42-site serve <folder> [port]` serves it, with push updates to its
//!   live pages, at `http://127.0.0.1:<port>/halcyon/` (port 8070 unless
//!   given).
//! - `t42-site terminal [port] [--relay <address>]` serves the terminal
//!   channel, which takes the viewer's keys (`interaction.md`), at
//!   `http://127.0.0.1:<port>/terminal/` (port 8071 unless given); with a
//!   qrkeyboard relay, its page 600 takes a phone's keys too.

mod build;
mod serve;
mod terminal;
mod websocket;

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
        Some("terminal") => {
            let relay = arguments.iter().position(|argument| argument == "--relay").and_then(|at| arguments.get(at + 1)).cloned();
            let port = arguments.get(1).filter(|argument| !argument.starts_with("--")).and_then(|port| port.parse().ok()).unwrap_or(terminal::DEFAULT_PORT);
            terminal::run(port, relay.as_deref())
        }
        _ => Err("usage: t42-site build <folder> <sample>... | t42-site serve <folder> [port] | t42-site terminal [port] [--relay <address>]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("t42-site: {error}");
            ExitCode::FAILURE
        }
    }
}
