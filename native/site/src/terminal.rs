//! `neoteletext-site terminal`: the reference channel that takes input
//! (INPUT.md, protocol version 2), as a host a terminal talks to. Its
//! descriptor declares `push.input`; each viewer's WebSocket is a session
//! with its own pages, and every item of input that arrives is answered by
//! pushing the page it changed.
//!
//! - 100, the index, and the page-selection convention of a videotex
//!   host: `*nnn#` goes to a page, `#` to the next, `*#` back, `*00` shows
//!   the page again, and on a page with no field three digits go to it;
//! - 200, typing: a name field with a cursor and local echo, and a
//!   masked secret field;
//! - 300, a game with no cursor at all: invaders, pushed ten times a
//!   second, played with the arrow keys held down and Space;
//! - 400, a piano: keys A to K play notes, sent as the audio profile's
//!   packets on the same WebSocket;
//! - 500, links: page numbers and a web link work as everywhere, and a
//!   plain click is reported to the host;
//! - 600, with `--relay`, a phone as keyboard with the channel as the
//!   screen: the page draws a qrkeyboard QR in mosaics, and the phone's
//!   keys type into it;
//! - 700, with `--podcast`, an episode from a WAVE file streamed as live
//!   sound: Space plays and pauses, the arrows skip, Enter starts again.

use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use crate::podcast::{Episode, Player};

mod invaders;
mod pages;
mod phone;
mod server;
mod session;
mod sounds;
mod tti;

pub use sounds::AUDIO_SERVICE;

use invaders::Invaders;
use pages::Field;
use phone::Phone;
use sounds::AUDIO_CHANNELS;

/// The channel's folder.
pub const CHANNEL: &str = "terminal";
/// The development port of the terminal channel.
pub const DEFAULT_PORT: u16 = 8071;

/// The pages, in the contents file's order, with their titles.
const PAGES: [(u16, &str); 7] = [
    (0x100, "Index"),
    (0x200, "Type into a page"),
    (0x300, "Invaders: a game"),
    (0x400, "Piano: notes on keys"),
    (0x500, "Links and clicks"),
    (0x600, "Your phone as keyboard"),
    (0x700, "Podcast: play and skip"),
];

/// What the channel was started with, which every session shares.
#[derive(Clone, Default)]
pub struct Settings {
    /// The qrkeyboard relay of page 600.
    pub relay: Option<String>,
    /// The episode of page 700.
    pub podcast: Option<Arc<Episode>>,
}

/// One viewer's session: the page open, and every page's state.
struct Session {
    page: u16,
    previous: Vec<u16>,
    /// The input count acted on, which the pushed page acknowledges
    /// (INPUT.md section 7.1).
    acked: u16,
    /// Page selection typed so far: `*` and digits, or plain digits.
    selection: String,
    last_key: String,
    fields: [Field; 2],
    focus: usize,
    greeting: String,
    invaders: Invaders,
    piano: Option<usize>,
    sounds_sent: bool,
    audio_sequence: [u16; AUDIO_CHANNELS as usize],
    next_channel: u8,
    record_id: u8,
    click: Option<(u8, u8)>,
    phone: Option<Phone>,
    relay: Option<String>,
    /// The viewer's keys went elsewhere: to another window, or to the
    /// browser's address field (`FOCUS_OUT`).
    away: bool,
    /// The episode and this viewer's place in it.
    podcast: Option<(Arc<Episode>, Player)>,
    /// The second page 700 last showed and whether it was playing, so the
    /// page is pushed each second and when the episode ends.
    shown: Option<(u64, bool)>,
}

/// Serves the terminal channel at `http://127.0.0.1:<port>/terminal/`,
/// with the phone page's relay and the podcast's episode when given.
pub fn run(port: u16, settings: &Settings) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|error| format!("port {port}: {error}"))?;
    eprintln!("neoteletext-site: serving the terminal channel at http://127.0.0.1:{port}/{CHANNEL}/");
    for stream in listener.incoming().flatten() {
        let settings = settings.clone();
        thread::spawn(move || {
            if let Err(error) = server::serve(stream, port, &settings) {
                eprintln!("neoteletext-site: {error}");
            }
        });
    }
    Ok(())
}
