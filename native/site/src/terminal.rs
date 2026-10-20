//! `neoteletext-site terminal`: the reference channel that takes input
//! (`interaction.md` sections 5 to 10), as a host a terminal talks to. Its
//! descriptor declares `push.input`; each viewer's WebSocket is a session
//! with its own pages, and every key record that arrives is answered by
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

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use neoteletext_audio::Codec;
use neoteletext_audio::encode::{Command, Sound, packets};
use neoteletext_input::cursor::{ECHO, MASKED, NO_CURSOR};
use neoteletext_input::key::{self, Event, Press};
use neoteletext_input::names::{key_name, physical_name};

use crate::podcast::{self, Episode, Player};
use crate::websocket;

/// The channel's folder.
pub const CHANNEL: &str = "terminal";
/// The development port of the terminal channel.
pub const DEFAULT_PORT: u16 = 8071;
/// The longest request head read.
const MAX_HEAD: usize = 8 * 1024;
/// How long a session waits for a key before the game moves on: ten
/// frames a second (`interaction.md` section 2: "twenty times a second if
/// it likes").
const FRAME: Duration = Duration::from_millis(100);

// EN 300 706 Table 26: the spacing attributes used here.
const ALPHA_RED: u8 = 0x01;
const ALPHA_GREEN: u8 = 0x02;
const ALPHA_YELLOW: u8 = 0x03;
const ALPHA_BLUE: u8 = 0x04;
const ALPHA_MAGENTA: u8 = 0x05;
const ALPHA_CYAN: u8 = 0x06;
const ALPHA_WHITE: u8 = 0x07;
const DOUBLE_HEIGHT: u8 = 0x0D;
const MOSAIC_RED: u8 = 0x11;
const MOSAIC_BLUE: u8 = 0x14;
const MOSAIC_CYAN: u8 = 0x16;
const MOSAIC_WHITE: u8 = 0x17;
const NEW_BACKGROUND: u8 = 0x1D;
const BLACK_BACKGROUND: u8 = 0x1C;
/// TTI writes a code below 0x20 as Escape and the code plus 0x40.
const ESCAPE: char = '\x1b';
const ESCAPE_OFFSET: u8 = 0x40;
/// The page status word in TTI's `PS` form: 0x8000 a page to show, and
/// 0x4000 C4, erase page (the engine's `TTI_PS_C4`).
const PAGE_STATUS: u16 = 0x8000 | 0x4000;
/// `#` in the English national option subset (EN 300 706 Table 36).
const ENGLISH_HASH: u8 = 0x5F;
const COLUMNS: usize = 40;

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
/// The podcast's progress bar, in cells: with its two colour codes, the
/// width of a row less a margin.
const PROGRESS_CELLS: usize = 36;
/// A solid mosaic cell (EN 300 706 Table 48: all six sixels set).
const SOLID_MOSAIC: char = '\x7f';
const SECONDS_PER_MINUTE: u64 = 60;

/// A row of a page: 7-bit codes, at most 40.
#[derive(Clone, Default)]
struct Row(Vec<u8>);

impl Row {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn code(mut self, code: u8) -> Self {
        self.0.push(code);
        self
    }

    /// Text in the English national option subset (EN 300 706 Table 36),
    /// the page's default: there `#` is at 0x5F, where ASCII has `_`, and
    /// 0x23 is `£`.
    fn text(mut self, text: &str) -> Self {
        self.0.extend(text.chars().map(|character| match character {
            '#' => ENGLISH_HASH,
            character if character.is_ascii() && !character.is_ascii_control() => character as u8,
            _ => b'?',
        }));
        self
    }

    fn at(mut self, column: usize) -> Self {
        self.0.resize(column.max(self.0.len()), b' ');
        self
    }
}

/// A page as TTI: its rows, and its cursor record as a `CU` line.
struct Page {
    number: u16,
    rows: BTreeMap<u8, Row>,
    cursor: Option<(u8, u8, u8, u8)>,
    acked: u16,
    /// `LT` lines: a run of text that is a link.
    links: Vec<String>,
}

impl Page {
    fn new(number: u16, title: &str) -> Self {
        let mut page = Self { number, rows: BTreeMap::new(), cursor: None, acked: 0, links: Vec::new() };
        page.row(0, Row::new().at(8).code(ALPHA_YELLOW).text(&format!("NEOTELETEXT TERMINAL {number:03X}")));
        // Row 1 is double height, so row 2 is its lower half.
        page.row(1, Row::new().code(ALPHA_BLUE).code(NEW_BACKGROUND).code(ALPHA_YELLOW).code(DOUBLE_HEIGHT).text(&title.to_uppercase()));
        page
    }

    fn row(&mut self, number: u8, row: Row) {
        self.rows.insert(number, row);
    }

    fn tti(&self) -> String {
        // C4, erase page (EN 300 706 Table 2): each version replaces the
        // last whole, so a row left out is blank, not the row before.
        let mut out = format!("PN,{:03X}00\r\nSC,0000\r\nPS,{PAGE_STATUS:04X}\r\n", self.number);
        for (number, row) in &self.rows {
            let _ = write!(out, "OL,{number},");
            for &code in row.0.iter().take(COLUMNS) {
                if code < 0x20 {
                    out.push(ESCAPE);
                    out.push(char::from(code + ESCAPE_OFFSET));
                } else {
                    out.push(char::from(code));
                }
            }
            out.push_str("\r\n");
        }
        for link in &self.links {
            let _ = write!(out, "{link}\r\n");
        }
        // The engine's REQ-INT-006: the cursor record as a line; a page
        // with no cursor still acknowledges the keys, for the round trip.
        let (row, column, width, flags) = self.cursor.unwrap_or((NO_CURSOR, 0, 0, 0));
        let _ = write!(out, "CU,{row},{column},{width},{flags},{}\r\n", self.acked);
        out
    }
}

/// What a session sends.
enum Push {
    /// A TTI page.
    Page(String),
    /// Packets: sounds and their commands.
    Packets(Vec<u8>),
}

/// The piano's keys: by position, by letter, and the note's frequency in
/// hertz (equal temperament from A4 = 440 Hz: C4 to C5).
const PIANO: [(&str, char, f64); 8] = [
    ("KeyA", 'a', 261.63),
    ("KeyS", 's', 293.66),
    ("KeyD", 'd', 329.63),
    ("KeyF", 'f', 349.23),
    ("KeyG", 'g', 392.00),
    ("KeyH", 'h', 440.00),
    ("KeyJ", 'j', 493.88),
    ("KeyK", 'k', 523.25),
];
/// The notes' rate and length: half a second at 8 kHz, IMA ADPCM in
/// 256-byte blocks, the profile's smallest.
const NOTE_RATE: u32 = 8_000;
const NOTE_SAMPLES: usize = 4_000;
const NOTE_BLOCK: u16 = 256;
/// A note's peak, a quarter of full scale, and its decay.
const NOTE_PEAK: f64 = 8_000.0;
const NOTE_DECAY: f64 = 6.0;
/// The handles the notes take: 1 to 8.
const FIRST_HANDLE: u8 = 1;
/// The game's sounds: a shot, a hit, a win and a loss, after the notes.
const FIRE_HANDLE: u8 = 9;
const HIT_HANDLE: u8 = 10;
const WIN_HANDLE: u8 = 11;
const LOSE_HANDLE: u8 = 12;
/// The typing page's sounds: a key's click, and Enter's bell.
const CLICK_HANDLE: u8 = 13;
const BELL_HANDLE: u8 = 14;
/// A click: 25 ms of noise falling away fast, as a typewriter's key.
const CLICK_SAMPLES: u32 = 200;
const CLICK_DECAY: f64 = 6.0;
/// A bell: E6, ringing for half a second.
const BELL_HZ: f64 = 1_318.51;
const BELL_SAMPLES: u32 = 4_000;
/// A shot sweeps down from this pitch to this one, in hertz.
const FIRE_FROM_HZ: f64 = 1_400.0;
const FIRE_TO_HZ: f64 = 300.0;
const FIRE_SAMPLES: u32 = 1_200;
/// A hit is noise, a quarter of a second.
const HIT_SAMPLES: u32 = 2_000;
/// A win rises C E G C; a loss falls G E C, each note this long.
const TUNE_NOTE_SAMPLES: u32 = 1_000;
const WIN_TUNE: [f64; 4] = [523.25, 659.25, 783.99, 1_046.50];
const LOSE_TUNE: [f64; 3] = [392.00, 329.63, 196.00];
/// The channels the effects take in turn: the first three of the audio
/// profile's four (REQ-AUD-010), the fourth being the podcast's.
const AUDIO_CHANNELS: u8 = 3;
const _: () = assert!(podcast::CHANNEL == AUDIO_CHANNELS, "the podcast has a channel of its own");
const NOTE_GAIN: u8 = 80;
const NOTE_FADE_MS: u16 = 5;
/// The audio service the channel's sounds are on.
pub const AUDIO_SERVICE: u8 = 0;

/// A shot: a sine sweeping down, fading out.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn fire_sound() -> Vec<i16> {
    let mut phase = 0.0_f64;
    (0..FIRE_SAMPLES)
        .map(|index| {
            let share = f64::from(index) / f64::from(FIRE_SAMPLES);
            let frequency = FIRE_FROM_HZ + (FIRE_TO_HZ - FIRE_FROM_HZ) * share;
            phase += 2.0 * std::f64::consts::PI * frequency / f64::from(NOTE_RATE);
            (phase.sin() * NOTE_PEAK * (1.0 - share)).round() as i16
        })
        .collect()
}

/// Noise from a linear congruential generator (Numerical Recipes'
/// constants), shaped by `envelope` over `samples`.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn noise(samples: u32, envelope: impl Fn(f64) -> f64) -> Vec<i16> {
    const MULTIPLIER: u32 = 1_664_525;
    const INCREMENT: u32 = 1_013_904_223;
    let mut state = 1_u32;
    (0..samples)
        .map(|index| {
            state = state.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT);
            let noise = f64::from(state >> 16) / f64::from(u16::MAX) * 2.0 - 1.0;
            (noise * NOTE_PEAK * envelope(f64::from(index) / f64::from(samples))).round() as i16
        })
        .collect()
}

/// A hit: noise fading out over a quarter of a second.
fn hit_sound() -> Vec<i16> {
    noise(HIT_SAMPLES, |share| 1.0 - share)
}

/// A key's click: a short burst of noise that falls away fast.
fn click_sound() -> Vec<i16> {
    noise(CLICK_SAMPLES, |share| (-CLICK_DECAY * share).exp())
}

/// Enter's bell.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn bell_sound() -> Vec<i16> {
    (0..BELL_SAMPLES)
        .map(|index| {
            let time = f64::from(index) / f64::from(NOTE_RATE);
            ((2.0 * std::f64::consts::PI * BELL_HZ * time).sin() * NOTE_PEAK * (-NOTE_DECAY * time).exp()).round() as i16
        })
        .collect()
}

/// A short tune: each note a sine that fades.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn tune(notes: &[f64]) -> Vec<i16> {
    notes
        .iter()
        .flat_map(|frequency| {
            (0..TUNE_NOTE_SAMPLES).map(move |index| {
                let time = f64::from(index) / f64::from(NOTE_RATE);
                let fade = 1.0 - f64::from(index) / f64::from(TUNE_NOTE_SAMPLES);
                ((2.0 * std::f64::consts::PI * frequency * time).sin() * NOTE_PEAK * fade).round() as i16
            })
        })
        .collect()
}

/// One note's samples: a sine with an exponential decay.
#[allow(clippy::cast_possible_truncation, reason = "the peak is a quarter of the samples' range")]
fn note(frequency: f64) -> Vec<i16> {
    (0..u32::try_from(NOTE_SAMPLES).unwrap_or(0))
        .map(|index| {
            let time = f64::from(index) / f64::from(NOTE_RATE);
            let sample = (2.0 * std::f64::consts::PI * frequency * time).sin() * NOTE_PEAK * (-NOTE_DECAY * time).exp();
            sample.round() as i16
        })
        .collect()
}

/// The invaders' playfield.
const FIELD_LEFT: i32 = 1;
const FIELD_RIGHT: i32 = 38;
const INVADER_TOP: i32 = 5;
const SHIP_ROW: i32 = 21;
const INVADER_COLUMNS: i32 = 8;
const INVADER_ROWS: i32 = 3;
const INVADER_SPACING: i32 = 4;
/// The invaders step sideways every few frames, and down at each edge.
const INVADER_PERIOD: u32 = 4;

/// What happened in one frame of the game.
#[derive(Default)]
struct Happened {
    /// A shot hit an invader.
    hit: bool,
    /// The game ended in this frame: won, or lost.
    over: Option<bool>,
}

struct Invaders {
    alive: Vec<(i32, i32)>,
    offset: i32,
    direction: i32,
    drop: i32,
    ship: i32,
    shots: Vec<(i32, i32)>,
    left: bool,
    right: bool,
    score: u32,
    frames: u32,
    over: Option<bool>,
}

impl Invaders {
    fn new() -> Self {
        let alive = (0..INVADER_ROWS).flat_map(|row| (0..INVADER_COLUMNS).map(move |column| (column * INVADER_SPACING, row * 2))).collect();
        Self { alive, offset: 2, direction: 1, drop: 0, ship: 19, shots: Vec::new(), left: false, right: false, score: 0, frames: 0, over: None }
    }

    /// One frame of the game: what happened in it, for its sounds.
    fn tick(&mut self) -> Happened {
        let mut happened = Happened::default();
        if self.over.is_some() {
            return happened;
        }
        self.frames += 1;
        if self.left {
            self.ship = (self.ship - 1).max(FIELD_LEFT);
        }
        if self.right {
            self.ship = (self.ship + 1).min(FIELD_RIGHT - 2);
        }
        for shot in &mut self.shots {
            shot.1 -= 1;
        }
        self.shots.retain(|shot| shot.1 >= INVADER_TOP);
        let (offset, drop) = (self.offset, self.drop);
        let mut hit = Vec::new();
        for (index, shot) in self.shots.iter().enumerate() {
            if let Some(position) = self.alive.iter().position(|&(x, y)| shot.1 == INVADER_TOP + y + drop && (x + offset..x + offset + 3).contains(&shot.0)) {
                self.alive.remove(position);
                hit.push(index);
                self.score += 10;
                happened.hit = true;
            }
        }
        for index in hit.into_iter().rev() {
            self.shots.remove(index);
        }
        if self.frames.is_multiple_of(INVADER_PERIOD) {
            let edge = self.alive.iter().any(|&(x, _)| {
                let next = x + self.offset + self.direction;
                next < FIELD_LEFT || next + 2 > FIELD_RIGHT
            });
            if edge {
                self.direction = -self.direction;
                self.drop += 1;
            } else {
                self.offset += self.direction;
            }
        }
        if self.alive.is_empty() {
            self.over = Some(true);
        } else if self.alive.iter().any(|&(_, y)| INVADER_TOP + y + self.drop >= SHIP_ROW) {
            self.over = Some(false);
        }
        happened.over = self.over;
        happened
    }

    fn page(&self, acked: u16) -> Page {
        let mut page = Page::new(0x300, "Invaders");
        page.row(3, Row::new().code(ALPHA_WHITE).text(&format!("Score {:<6}", self.score)).code(ALPHA_CYAN).text("Left Right move, Space fires"));
        let mut rows: BTreeMap<i32, Vec<u8>> = BTreeMap::new();
        let mut put = |row: i32, column: i32, text: &[u8]| {
            let line = rows.entry(row).or_insert_with(|| vec![b' '; COLUMNS]);
            for (at, &byte) in (column..).zip(text) {
                if let Some(cell) = usize::try_from(at).ok().and_then(|at| line.get_mut(at)) {
                    *cell = byte;
                }
            }
        };
        for &(x, y) in &self.alive {
            put(INVADER_TOP + y + self.drop, x + self.offset, b"<o>");
        }
        for &(x, y) in &self.shots {
            put(y, x, b"!");
        }
        put(SHIP_ROW, self.ship, b"-A-");
        for (row, line) in rows {
            let colour = if row == SHIP_ROW {
                ALPHA_CYAN
            } else if line.contains(&b'!') && !line.contains(&b'o') {
                ALPHA_WHITE
            } else {
                ALPHA_GREEN
            };
            let mut cells = line;
            cells[0] = colour;
            if let Ok(row) = u8::try_from(row) {
                page.row(row, Row(cells));
            }
        }
        if let Some(won) = self.over {
            page.row(12, Row::new().at(10).code(if won { ALPHA_GREEN } else { ALPHA_RED }).code(DOUBLE_HEIGHT).text(if won { "YOU WIN" } else { "GAME OVER" }));
            page.row(14, Row::new().at(10).code(ALPHA_WHITE).text("Enter plays again"));
        }
        page.acked = acked;
        page
    }
}

/// A text field of the typing page.
struct Field {
    row: u8,
    column: u8,
    width: u8,
    text: String,
    masked: bool,
}

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
    /// The last key record's number, which the pushed page acknowledges.
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
    /// browser's address field (key record kind 6).
    away: bool,
    /// The episode and this viewer's place in it.
    podcast: Option<(Arc<Episode>, Player)>,
    /// The second page 700 last showed and whether it was playing, so the
    /// page is pushed each second and when the episode ends.
    shown: Option<(u64, bool)>,
}

/// The phone keyboard of page 600: the screen side in its own thread.
struct Phone {
    events: mpsc::Receiver<PhoneEvent>,
    rows: Vec<Vec<u8>>,
    state: String,
    typed: String,
}

enum PhoneEvent {
    Join(Vec<Vec<u8>>, Option<String>),
    State(String),
    Text(String),
    Key(String),
}

impl Session {
    fn new(settings: Settings) -> Self {
        Self {
            page: 0x100,
            previous: Vec::new(),
            acked: 0,
            selection: String::new(),
            last_key: String::new(),
            fields: [
                Field { row: 5, column: 13, width: 20, text: String::new(), masked: false },
                Field { row: 8, column: 13, width: 12, text: String::new(), masked: true },
            ],
            focus: 0,
            greeting: String::new(),
            invaders: Invaders::new(),
            piano: None,
            sounds_sent: false,
            audio_sequence: [0; AUDIO_CHANNELS as usize],
            next_channel: 0,
            record_id: 0,
            click: None,
            phone: None,
            relay: settings.relay,
            away: false,
            podcast: settings.podcast.map(|episode| (episode, Player::new())),
            shown: None,
        }
    }

    /// The open page as it now stands.
    fn render(&self) -> Page {
        let mut page = match self.page {
            0x100 => index(),
            0x200 => self.typing(),
            0x300 => return self.invaders.page(self.acked),
            0x400 => self.piano(),
            0x500 => self.links(),
            0x600 => self.phone_page(),
            0x700 => self.podcast_page(),
            other => {
                let mut page = Page::new(other, "No such page");
                page.row(4, Row::new().code(ALPHA_WHITE).text("*100# goes back to the index."));
                page
            }
        };
        if self.away {
            page.row(23, Row::new().code(ALPHA_MAGENTA).text("Your keys went elsewhere."));
        } else if !self.selection.is_empty() {
            page.row(23, Row::new().code(ALPHA_YELLOW).text(&format!("Page {}", self.selection)));
        } else if !self.last_key.is_empty() {
            page.row(23, Row::new().code(ALPHA_CYAN).text(&format!("Last key: {}", self.last_key)));
        }
        page.acked = self.acked;
        page
    }

    fn typing(&self) -> Page {
        let mut page = Page::new(0x200, "Type into a page");
        page.row(3, Row::new().code(ALPHA_WHITE).text("Type: each key goes to this channel."));
        for (index, field) in self.fields.iter().enumerate() {
            let label = if field.masked { "Secret:" } else { "Your name:" };
            let shown: String = if field.masked { "*".repeat(field.text.chars().count()) } else { field.text.clone() };
            let mut row = Row::new().code(ALPHA_YELLOW).text(label).at(usize::from(field.column) - 1).code(ALPHA_WHITE).text(&shown);
            if index == self.focus {
                row = row.at(usize::from(field.column + field.width)).code(ALPHA_CYAN).text("<");
            }
            page.row(field.row, row);
        }
        page.row(11, Row::new().code(ALPHA_GREEN).text(&self.greeting));
        page.row(14, Row::new().code(ALPHA_WHITE).text("Tab moves between fields; Enter sends."));
        page.row(15, Row::new().code(ALPHA_WHITE).text("Letters show at once, provisionally, then"));
        page.row(16, Row::new().code(ALPHA_WHITE).text("as the channel draws them: local echo."));
        page.row(18, Row::new().code(ALPHA_MAGENTA).text("*100# for the index, Esc clears."));
        let field = &self.fields[self.focus];
        let length = u8::try_from(field.text.chars().count()).unwrap_or(field.width).min(field.width);
        if length < field.width {
            page.cursor = Some((field.row, field.column + length, field.width - length, ECHO | if field.masked { MASKED } else { 0 }));
        } else {
            page.cursor = Some((field.row, field.column + length - 1, 0, 0));
        }
        page
    }

    fn piano(&self) -> Page {
        let mut page = Page::new(0x400, "Piano");
        page.row(3, Row::new().code(ALPHA_WHITE).text("Keys A S D F G H J K play the notes"));
        page.row(4, Row::new().code(ALPHA_WHITE).text("C to C. Each press is a sound command"));
        page.row(5, Row::new().code(ALPHA_WHITE).text("pushed on this page's own WebSocket."));
        for line in 0..6_u8 {
            let mut row = Row::new().at(2);
            for (index, (_, letter, _)) in PIANO.iter().enumerate() {
                let pressed = self.piano == Some(index);
                row = row.code(if pressed { MOSAIC_RED } else { MOSAIC_WHITE });
                if line == 5 {
                    row = row.code(ALPHA_YELLOW).text(&letter.to_uppercase().to_string()).text(" ");
                } else {
                    row = row.text("\x7f\x7f\x7f");
                }
            }
            page.row(9 + line, row.code(BLACK_BACKGROUND));
        }
        page
    }

    fn links(&self) -> Page {
        let mut page = Page::new(0x500, "Links and clicks");
        page.row(3, Row::new().code(ALPHA_WHITE).text("Page numbers are links: 100 200 300"));
        page.row(4, Row::new().code(ALPHA_WHITE).text("Command-click one to follow it."));
        page.row(6, Row::new().code(ALPHA_CYAN).text("The teletext engine on the web"));
        page.links.push("LT,6,1,30,web:https://example.com/".to_owned());
        page.row(7, Row::new().code(ALPHA_WHITE).text("(Command-click opens your web browser)"));
        page.row(10, Row::new().code(ALPHA_YELLOW).text("A plain click is this channel's:"));
        let clicked = self.click.map_or_else(|| "Click anywhere on the page.".to_owned(), |(column, row)| format!("You clicked column {column}, row {row}."));
        page.row(11, Row::new().code(ALPHA_GREEN).text(&clicked));
        page
    }

    fn phone_page(&self) -> Page {
        let mut page = Page::new(0x600, "Your phone as keyboard");
        let Some(phone) = &self.phone else {
            let why = if self.relay.is_some() { "Preparing the QR code..." } else { "Start neoteletext-site with --relay to try this." };
            page.row(4, Row::new().code(ALPHA_WHITE).text(why));
            return page;
        };
        for (index, row) in phone.rows.iter().enumerate() {
            if let Ok(number) = u8::try_from(3 + index) {
                if number > 20 {
                    break;
                }
                page.row(number, Row(row.clone()));
            }
        }
        page.row(22, Row::new().code(ALPHA_WHITE).text(&phone.state));
        page.row(21, Row::new().code(ALPHA_GREEN).text(&format!("Phone typed: {}", phone.typed)));
        page
    }

    fn podcast_page(&self) -> Page {
        let mut page = Page::new(0x700, "Podcast");
        let Some((episode, player)) = &self.podcast else {
            page.row(4, Row::new().code(ALPHA_WHITE).text("Start neoteletext-site with --podcast FILE.wav"));
            page.row(5, Row::new().code(ALPHA_WHITE).text("to listen here."));
            return page;
        };
        let title: String = episode.title.chars().take(COLUMNS - 2).collect();
        page.row(3, Row::new().code(ALPHA_YELLOW).code(DOUBLE_HEIGHT).text(&title));
        let (position, length) = (player.position(episode), episode.samples.len());
        let state = match (player.playing(), position) {
            (true, _) => "Playing",
            (false, 0) => "Ready: press Space",
            (false, at) if at >= length => "Finished",
            (false, _) => "Paused",
        };
        page.row(6, Row::new().code(ALPHA_GREEN).text(state));
        let clock = |samples: usize| {
            let seconds = episode.seconds(samples);
            format!("{}:{:02}", seconds / SECONDS_PER_MINUTE, seconds % SECONDS_PER_MINUTE)
        };
        page.row(8, Row::new().code(ALPHA_WHITE).text(&format!("{} / {}", clock(position), clock(length))));
        let filled = position.saturating_mul(PROGRESS_CELLS).checked_div(length).unwrap_or(0).min(PROGRESS_CELLS);
        let bar = |cells: usize| SOLID_MOSAIC.to_string().repeat(cells);
        for row in [10, 11] {
            page.row(row, Row::new().code(MOSAIC_CYAN).text(&bar(filled)).code(MOSAIC_BLUE).text(&bar(PROGRESS_CELLS - filled)));
        }
        page.row(14, Row::new().code(ALPHA_CYAN).text("Space plays and pauses."));
        page.row(15, Row::new().code(ALPHA_CYAN).text(&format!("Left and Right skip {} seconds.", podcast::SKIP_SECONDS)));
        page.row(16, Row::new().code(ALPHA_CYAN).text("Enter starts it again."));
        page.row(18, Row::new().code(ALPHA_MAGENTA).text("It plays on while you read other pages."));
        page
    }

    fn go(&mut self, page: u16) {
        if page != self.page {
            self.previous.push(self.page);
            self.page = page;
        }
    }

    /// The videotex convention (`interaction.md` section 5.1), and three
    /// plain digits where the page has no field. Returns whether the key
    /// was part of a page selection.
    fn select(&mut self, character: char) -> bool {
        let field_page = self.page == 0x200;
        match character {
            '*' => "*".clone_into(&mut self.selection),
            '#' if self.selection == "*" => {
                if let Some(page) = self.previous.pop() {
                    self.page = page;
                }
                self.selection.clear();
            }
            '#' if self.selection.starts_with('*') => {
                if let Ok(page) = u16::from_str_radix(&self.selection[1..], 16)
                    && (0x100..=0x8FF).contains(&page)
                {
                    self.go(page);
                }
                self.selection.clear();
            }
            '#' => {
                let next = PAGES.iter().position(|(number, _)| *number == self.page).map_or(0x100, |index| PAGES[(index + 1) % PAGES.len()].0);
                self.go(next);
            }
            digit @ '0'..='9' if self.selection.starts_with('*') || !field_page => {
                self.selection.push(digit);
                if self.selection == "*00" {
                    self.selection.clear();
                } else if !self.selection.starts_with('*') && self.selection.len() == 3 {
                    if let Ok(page) = u16::from_str_radix(&self.selection, 16)
                        && (0x100..=0x899).contains(&page)
                    {
                        self.go(page);
                    }
                    self.selection.clear();
                }
            }
            _ => return false,
        }
        true
    }

    /// Acts on one key record; returns what to push.
    fn key(&mut self, record: &key::Record<'_>) -> Vec<Push> {
        self.acked = record.sequence;
        let mut pushes = Vec::new();
        match record.event {
            Event::Page(Some(page)) => {
                self.page = page;
                self.selection.clear();
            }
            Event::Focus(here) => self.away = !here,
            // A new connection: the page is pushed again below.
            Event::Page(None) | Event::Resume(_) => {}
            Event::Click { column, row } => self.click = Some((column, row)),
            Event::Text(text) => {
                for character in text.chars() {
                    self.character(character);
                }
            }
            Event::Down(press) => {
                self.last_key = describe(press);
                pushes.extend(self.down(press));
            }
            Event::Up(press) => self.up(press),
        }
        if matches!(self.page, 0x200 | 0x300 | 0x400) && !self.sounds_sent {
            self.sounds_sent = true;
            pushes.splice(0..0, self.sounds());
        }
        if self.page == 0x600 && self.phone.is_none() {
            self.start_phone();
        }
        pushes.push(Push::Page(self.render().tti()));
        pushes
    }

    fn down(&mut self, press: Press) -> Vec<Push> {
        let name = key_name(press.key);
        let character = char::from_u32(u32::from(press.key)).filter(|_| name.is_none());
        let control = press.modifiers & key::CONTROL != 0;
        // The typing page clicks on every key, as a typewriter does, and
        // rings on Enter.
        let sound = (self.page == 0x200).then_some(if name == Some("Enter") { BELL_HANDLE } else { CLICK_HANDLE });
        match (self.page, name, character) {
            // A press moves at once; held, the ship keeps moving each frame.
            (0x300, Some("ArrowLeft"), _) => {
                self.invaders.left = true;
                self.invaders.ship = (self.invaders.ship - 1).max(FIELD_LEFT);
            }
            (0x300, Some("ArrowRight"), _) => {
                self.invaders.right = true;
                self.invaders.ship = (self.invaders.ship + 1).min(FIELD_RIGHT - 2);
            }
            (0x300, _, Some(' ')) if press.modifiers & key::REPEAT == 0 => {
                if self.invaders.shots.len() < 3 && self.invaders.over.is_none() {
                    self.invaders.shots.push((self.invaders.ship + 1, SHIP_ROW - 1));
                    return vec![self.play(FIRE_HANDLE)];
                }
            }
            (0x300, Some("Enter"), _) if self.invaders.over.is_some() => self.invaders = Invaders::new(),
            (0x400, _, _) => {
                let position = physical_name(press.physical);
                let index = PIANO.iter().position(|(code, letter, _)| position == Some(code) || character.map(|c| c.to_ascii_lowercase()) == Some(*letter));
                if let Some(index) = index
                    && press.modifiers & key::REPEAT == 0
                {
                    self.piano = Some(index);
                    return vec![self.play(FIRST_HANDLE + u8::try_from(index).unwrap_or(0))];
                }
                if let Some(character) = character {
                    self.select(character);
                }
            }
            (0x700, Some("ArrowLeft"), _) => return self.listen(|player, episode, record_id| player.skip(episode, -podcast::SKIP_SECONDS, record_id)),
            (0x700, Some("ArrowRight"), _) => return self.listen(|player, episode, record_id| player.skip(episode, podcast::SKIP_SECONDS, record_id)),
            (0x700, Some("Enter"), _) => return self.listen(Player::restart),
            (0x700, _, Some(' ')) if press.modifiers & key::REPEAT == 0 => return self.listen(Player::toggle),
            (0x200, Some("Tab"), _) => self.focus = (self.focus + 1) % self.fields.len(),
            (0x200, Some("Backspace"), _) => {
                self.fields[self.focus].text.pop();
            }
            (0x200, Some("Escape"), _) => {
                for field in &mut self.fields {
                    field.text.clear();
                }
                self.greeting.clear();
            }
            (0x200, Some("Enter"), _) => {
                let name = self.fields[0].text.trim().to_owned();
                self.greeting = if name.is_empty() { "Type your name first.".to_owned() } else { format!("Hello, {name}!") };
            }
            (_, _, Some(character)) if !control => self.character(character),
            _ => {}
        }
        sound.map(|handle| vec![self.play(handle)]).unwrap_or_default()
    }

    fn up(&mut self, press: Press) {
        match key_name(press.key) {
            Some("ArrowLeft") => self.invaders.left = false,
            Some("ArrowRight") => self.invaders.right = false,
            _ => {}
        }
        if self.page == 0x400 {
            self.piano = None;
        }
    }

    fn character(&mut self, character: char) {
        if self.select(character) {
            return;
        }
        if self.page == 0x200 {
            let field = &mut self.fields[self.focus];
            if field.text.chars().count() < usize::from(field.width) && character.is_ascii() && !character.is_ascii_control() {
                field.text.push(character);
            }
        }
    }

    /// Every sound of the channel, as packets, one message a sound: the
    /// piano's notes and the game's effects.
    fn sounds(&mut self) -> Vec<Push> {
        let notes = PIANO.iter().enumerate().filter_map(|(index, (_, _, frequency))| Some((FIRST_HANDLE + u8::try_from(index).ok()?, note(*frequency))));
        let effects = [
            (FIRE_HANDLE, fire_sound()),
            (HIT_HANDLE, hit_sound()),
            (WIN_HANDLE, tune(&WIN_TUNE)),
            (LOSE_HANDLE, tune(&LOSE_TUNE)),
            (CLICK_HANDLE, click_sound()),
            (BELL_HANDLE, bell_sound()),
        ];
        let all: Vec<(u8, Vec<i16>)> = notes.chain(effects).collect();
        all.into_iter()
            .filter_map(|(handle, pcm)| {
                let sound = Sound::new(handle, 1, Codec::ImaAdpcm, NOTE_RATE, NOTE_BLOCK, &pcm).ok()?;
                let mut records = vec![sound.object_record().ok()?];
                records.extend(sound.data_records());
                let (packets, next) = packets(AUDIO_SERVICE, self.record_id, &records);
                self.record_id = next;
                Some(Push::Packets(packets.iter().flat_map(|packet| packet.bytes).collect()))
            })
            .collect()
    }

    /// The command that plays a sound, on the next of the effects' channels.
    fn play(&mut self, handle: u8) -> Push {
        let channel = self.next_channel;
        self.next_channel = (self.next_channel + 1) % AUDIO_CHANNELS;
        let sequence = &mut self.audio_sequence[usize::from(channel)];
        *sequence = sequence.wrapping_add(1);
        let command = Command {
            channel,
            sequence: *sequence,
            operation: 1,
            handle,
            generation: 1,
            gain: NOTE_GAIN,
            fade_ms: NOTE_FADE_MS,
            flags: 0,
            duck: 0,
            page: None,
        };
        let (packets, next) = packets(AUDIO_SERVICE, self.record_id, &[command.record()]);
        self.record_id = next;
        Push::Packets(packets.iter().flat_map(|packet| packet.bytes).collect())
    }

    /// Does `action` to this viewer's podcast: what it sends, pushed.
    fn listen(&mut self, action: impl FnOnce(&mut Player, &Episode, &mut u8) -> Vec<u8>) -> Vec<Push> {
        let Some((episode, player)) = &mut self.podcast else { return Vec::new() };
        let bytes = action(player, episode, &mut self.record_id);
        if bytes.is_empty() { Vec::new() } else { vec![Push::Packets(bytes)] }
    }

    /// The podcast's next blocks, wherever the viewer is, and page 700
    /// again each second and when it stops, while it is open.
    fn podcast_frame(&mut self) -> Vec<Push> {
        let mut pushes = self.listen(Player::stream);
        if self.page == 0x700
            && let Some((episode, player)) = &self.podcast
        {
            let now = (episode.seconds(player.position(episode)), player.playing());
            if self.shown.replace(now) != Some(now) {
                pushes.push(Push::Page(self.render().tti()));
            }
        }
        pushes
    }

    /// One frame of the game: its sounds, then its page.
    fn frame(&mut self) -> Vec<Push> {
        let happened = self.invaders.tick();
        let mut pushes = Vec::new();
        if happened.hit {
            pushes.push(self.play(HIT_HANDLE));
        }
        match happened.over {
            Some(true) => pushes.push(self.play(WIN_HANDLE)),
            Some(false) => pushes.push(self.play(LOSE_HANDLE)),
            None => {}
        }
        pushes.push(Push::Page(self.render().tti()));
        pushes
    }

    fn start_phone(&mut self) {
        let Some(relay) = self.relay.clone() else { return };
        let (sender, events) = mpsc::channel();
        thread::spawn(move || phone(&relay, &sender));
        self.phone = Some(Phone { events, rows: Vec::new(), state: "Preparing...".to_owned(), typed: String::new() });
    }

    /// What the phone did since the last look; whether the page changed.
    fn phone_events(&mut self) -> bool {
        let Some(phone) = &mut self.phone else { return false };
        let mut changed = false;
        while let Ok(event) = phone.events.try_recv() {
            changed = true;
            match event {
                PhoneEvent::Join(rows, pin) => {
                    phone.rows = rows;
                    phone.state = pin.map_or_else(|| "Scan with your phone camera to type here.".to_owned(), |pin| format!("Scan, then enter PIN {pin}"));
                }
                PhoneEvent::State(state) => phone.state = state,
                PhoneEvent::Text(text) => phone.typed.push_str(&text),
                PhoneEvent::Key(key) if key == "Backspace" => {
                    phone.typed.pop();
                }
                PhoneEvent::Key(key) => {
                    let _ = write!(phone.typed, "({key})");
                }
            }
            let excess = phone.typed.chars().count().saturating_sub(24);
            phone.typed = phone.typed.chars().skip(excess).collect();
        }
        changed && self.page == 0x600
    }
}

/// The channel as the screen of qrkeyboard (`interaction.md` section 9):
/// the instance, its QR as mosaic rows, and the phone's keys.
fn phone(relay: &str, events: &mpsc::Sender<PhoneEvent>) {
    use neoteletext_qrkeyboard::config::{Layout, ScreenConfig};
    use neoteletext_qrkeyboard::qr::{Correction, Matrix};
    use neoteletext_qrkeyboard::screen::{Event as ScreenEvent, Screen};
    let config = ScreenConfig { name: "NeoTeletext terminal channel".into(), layout: Layout::Terminal, pin: false, rejoin_seconds: Some(30) };
    let mut screen = match Screen::create(relay, config) {
        Ok(screen) => screen,
        Err(error) => {
            let _ = events.send(PhoneEvent::State(format!("No relay: {error}")));
            return;
        }
    };
    let rows = Matrix::new(&screen.join().url, Correction::Low).map(|matrix| matrix.mosaic_rows()).unwrap_or_default();
    if events.send(PhoneEvent::Join(rows, screen.join().pin.clone())).is_err() {
        return;
    }
    loop {
        let Ok(happened) = screen.poll(Duration::from_millis(200)) else { return };
        for event in happened {
            let sent = match event {
                ScreenEvent::Connected => events.send(PhoneEvent::State("Phone connected: type on it.".into())),
                ScreenEvent::Text(text) => events.send(PhoneEvent::Text(text)),
                ScreenEvent::Special { key, .. } => events.send(PhoneEvent::Key(key.to_owned())),
                ScreenEvent::Down(key) => events.send(PhoneEvent::Key(key)),
                ScreenEvent::Disconnected(_) => events.send(PhoneEvent::State("Phone gone: scan again.".into())),
                ScreenEvent::Finished => {
                    let _ = events.send(PhoneEvent::State("Session finished.".into()));
                    return;
                }
                _ => Ok(()),
            };
            if sent.is_err() {
                screen.close();
                return;
            }
        }
    }
}

/// A key's name for the page, as the channel heard it.
fn describe(press: Press) -> String {
    let name = match (key_name(press.key), char::from_u32(u32::from(press.key))) {
        (Some(name), _) => name.to_owned(),
        // The space bar types a space, which on the page would be nothing.
        (None, Some(' ')) => "Space".to_owned(),
        (None, character) => character.map(String::from).unwrap_or_default(),
    };
    let mut text = String::new();
    for (bit, label) in [(key::CONTROL, "Ctrl-"), (key::ALT, "Alt-"), (key::SHIFT, "Shift-")] {
        if press.modifiers & bit != 0 {
            text.push_str(label);
        }
    }
    text.push_str(&name);
    if let Some(position) = physical_name(press.physical) {
        let _ = write!(text, " at {position}");
    }
    text
}

fn index() -> Page {
    let mut page = Page::new(0x100, "Terminal channel");
    page.row(3, Row::new().code(ALPHA_WHITE).text("This channel takes your keys, as a"));
    page.row(4, Row::new().code(ALPHA_WHITE).text("terminal's host does."));
    for (index, (number, title)) in PAGES.iter().skip(1).enumerate() {
        let row = u8::try_from(6 + index).unwrap_or(6);
        page.row(row, Row::new().code(ALPHA_YELLOW).text(&format!("{number:03X}")).code(ALPHA_WHITE).text(title));
    }
    page.row(13, Row::new().code(ALPHA_CYAN).text("Choose as on Prestel: *nnn# for a page,"));
    page.row(14, Row::new().code(ALPHA_CYAN).text("# the next, *# back, *00 again; or type"));
    page.row(15, Row::new().code(ALPHA_CYAN).text("three digits."));
    page.row(17, Row::new().code(ALPHA_GREEN).text("Command-Escape leaves; so does closing"));
    page.row(18, Row::new().code(ALPHA_GREEN).text("the page."));
    page
}

/// The page files a viewer fetches before the session starts.
fn page_file(number: u16, settings: &Settings) -> Option<String> {
    PAGES.iter().any(|(page, _)| *page == number).then(|| {
        let mut session = Session::new(settings.clone());
        session.page = number;
        session.render().tti()
    })
}

/// The channel's contents in the engine's format (its REQ-WEB-025): a
/// section, then each page.
fn contents_file() -> String {
    let mut contents = String::from("s\tTerminal\n");
    for (number, title) in PAGES {
        let _ = writeln!(contents, "p\t{number:03X}\t{title}");
    }
    contents
}

fn respond(stream: &mut TcpStream, status: &str, media: &str, body: &[u8]) -> io::Result<()> {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {media}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n", body.len())?;
    stream.write_all(body)?;
    stream.flush()
}

/// A viewer's WebSocket: key records in, pages and sounds out, and the
/// game's frames while its page is open.
fn session(mut stream: TcpStream, settings: Settings) -> io::Result<()> {
    stream.set_read_timeout(Some(FRAME))?;
    stream.set_nodelay(true)?;
    let mut reader = websocket::Reader::default();
    let mut session = Session::new(settings);
    let mut last_frame = Instant::now();
    loop {
        let Some(frames) = reader.read(&mut stream)? else { return Ok(()) };
        for (opcode, payload) in frames {
            match opcode {
                websocket::BINARY => match key::read(&payload) {
                    Ok(record) => {
                        for push in session.key(&record) {
                            send(&mut stream, push)?;
                        }
                    }
                    Err(error) => eprintln!("neoteletext-site: a record refused: {error:?}"),
                },
                websocket::PING => websocket::write(&mut stream, websocket::PONG, &payload)?,
                websocket::CLOSE => {
                    let _ = websocket::write(&mut stream, websocket::CLOSE, &payload);
                    return Ok(());
                }
                // The subscription, as text: everything, always.
                _ => {}
            }
        }
        if session.phone_events() {
            send(&mut stream, Push::Page(session.render().tti()))?;
        }
        for push in session.podcast_frame() {
            send(&mut stream, push)?;
        }
        if session.page == 0x300 && last_frame.elapsed() >= FRAME {
            last_frame = Instant::now();
            for push in session.frame() {
                send(&mut stream, push)?;
            }
        }
    }
}

fn send(stream: &mut TcpStream, push: Push) -> io::Result<()> {
    match push {
        Push::Page(tti) => websocket::write(stream, websocket::TEXT, tti.as_bytes()),
        Push::Packets(bytes) => websocket::write(stream, websocket::BINARY, &bytes),
    }
}

fn serve(mut stream: TcpStream, port: u16, settings: &Settings) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        let mut head = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            head.push_str(&line);
            if head.len() > MAX_HEAD {
                return respond(&mut stream, "431 Request Header Fields Too Large", "text/plain", b"");
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        let target = head.split(' ').nth(1).unwrap_or_default().to_owned();
        let path = target.split('?').next().unwrap_or_default();
        eprintln!("neoteletext-site: GET {target}");
        let folder = format!("/{CHANNEL}/");
        if path == format!("{folder}push") {
            let Some(accept) = websocket::accept(&head) else { return respond(&mut stream, "426 Upgrade Required", "text/plain", b"") };
            websocket::handshake(&mut stream, &accept)?;
            return session(stream, settings.clone());
        }
        let Some(name) = path.strip_prefix(&folder) else {
            respond(&mut stream, "404 Not Found", "text/plain", b"not found")?;
            continue;
        };
        match name {
            "channel.json" => {
                let json = format!(
                    r#"{{"name":"NeoTeletext terminal channel","push":{{"address":"ws://127.0.0.1:{port}/{CHANNEL}/push","method":"websocket","input":true}}}}"#
                );
                respond(&mut stream, "200 OK", "application/json", json.as_bytes())?;
            }
            "contents.txt" => respond(&mut stream, "200 OK", "text/plain", contents_file().as_bytes())?,
            _ => match name.strip_suffix(".tti").and_then(|number| u16::from_str_radix(number, 16).ok()).and_then(|number| page_file(number, settings)) {
                Some(tti) => respond(&mut stream, "200 OK", "text/x-teletext-tti", tti.as_bytes())?,
                None => respond(&mut stream, "404 Not Found", "text/plain", b"not found")?,
            },
        }
    }
}

/// Serves the terminal channel at `http://127.0.0.1:<port>/terminal/`,
/// with the phone page's relay and the podcast's episode when given.
pub fn run(port: u16, settings: &Settings) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|error| format!("port {port}: {error}"))?;
    eprintln!("neoteletext-site: serving the terminal channel at http://127.0.0.1:{port}/{CHANNEL}/");
    for stream in listener.incoming().flatten() {
        let settings = settings.clone();
        thread::spawn(move || {
            if let Err(error) = serve(stream, port, &settings) {
                eprintln!("neoteletext-site: {error}");
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use neoteletext_input::key::{MAX_RECORD, Record};
    use neoteletext_input::names::named_key;

    fn record(sequence: u16, event: Event<'_>) -> Vec<u8> {
        let mut out = [0; MAX_RECORD];
        let length = Record { sequence, clock_ms: 0, event }.write(&mut out).unwrap();
        out[..length].to_vec()
    }

    fn press(character: char) -> Event<'static> {
        Event::Down(Press { key: u16::try_from(u32::from(character)).unwrap(), modifiers: 0, physical: 0 })
    }

    fn named(name: &str) -> Event<'static> {
        Event::Down(Press { key: named_key(name).unwrap(), modifiers: 0, physical: 0 })
    }

    fn feed(session: &mut Session, events: &[Event<'_>]) -> Vec<Push> {
        let mut pushes = Vec::new();
        for (index, event) in events.iter().enumerate() {
            let bytes = record(u16::try_from(index + 1).unwrap(), *event);
            pushes.extend(session.key(&key::read(&bytes).unwrap()));
        }
        pushes
    }

    #[test]
    fn pages_are_chosen_as_on_prestel() {
        let mut session = Session::new(Settings::default());
        feed(&mut session, &[press('*'), press('3'), press('0'), press('0'), press('#')]);
        assert_eq!(session.page, 0x300);
        feed(&mut session, &[press('#')]);
        assert_eq!(session.page, 0x400, "# is the next page");
        feed(&mut session, &[press('*'), press('#')]);
        assert_eq!(session.page, 0x300, "*# goes back");
        feed(&mut session, &[press('5'), press('0'), press('0')]);
        assert_eq!(session.page, 0x500, "three digits where there is no field");
        feed(&mut session, &[Event::Page(Some(0x200)), press('1'), press('0'), press('0')]);
        assert_eq!(session.page, 0x200, "on a page with a field digits are text");
        assert_eq!(session.fields[0].text, "100");
    }

    #[test]
    fn typing_echoes_and_acknowledges() {
        let mut session = Session::new(Settings::default());
        let pushes = feed(&mut session, &[Event::Page(Some(0x200)), press('A'), press('n'), named("Enter")]);
        let Some(Push::Page(last)) = pushes.last() else { panic!() };
        assert!(last.contains("Hello, An!"), "{last}");
        assert!(last.contains("CU,5,15,18,1,4"), "the cursor after the name, echo on, acknowledging key 4: {last}");
        feed(&mut session, &[named("Tab"), press('x')]);
        let page = session.render().tti();
        assert!(page.contains("CU,8,14,11,3,"), "the secret field is masked: {page}");
    }

    /// The channels the receiver started sounds on.
    struct Count(Vec<u8>);
    impl neoteletext_audio::port::VoiceSink for Count {
        fn define(&mut self, _: neoteletext_audio::port::SoundId, _: neoteletext_audio::port::SoundFormat) {}
        fn samples(&mut self, _: neoteletext_audio::port::SoundId, _: u32, _: &[i16]) {}
        fn forget(&mut self, _: neoteletext_audio::port::SoundId) {}
        fn stream(&mut self, _: neoteletext_audio::port::SoundId, _: u32) {}
        fn stream_samples(&mut self, _: neoteletext_audio::port::SoundId, _: &[i16]) {}
        fn start(&mut self, channel: u8, _: neoteletext_audio::port::Start) {
            self.0.push(channel);
        }
        fn stop(&mut self, _: u8, _: u16) {}
        fn gain(&mut self, _: u8, _: u8, _: u16) {}
    }

    #[test]
    fn the_typing_page_clicks_on_keys_and_rings_on_enter() {
        let mut session = Session::new(Settings::default());
        feed(&mut session, &[Event::Page(Some(0x200))]);
        let commands = |pushes: &[Push]| pushes.iter().filter(|push| matches!(push, Push::Packets(bytes) if bytes.len() == 42)).count();
        let key = feed(&mut session, &[press('a')]);
        assert_eq!(commands(&key), 1, "a click");
        let enter = feed(&mut session, &[named("Enter")]);
        assert_eq!(commands(&enter), 1, "a bell");
        // The game's page does not click.
        feed(&mut session, &[Event::Page(Some(0x300))]);
        assert_eq!(commands(&feed(&mut session, &[press('a')])), 0);
    }

    #[test]
    fn the_piano_sends_its_sounds_then_a_command_a_key() {
        let mut session = Session::new(Settings::default());
        let pushes = feed(&mut session, &[Event::Page(Some(0x400))]);
        let sounds = pushes.iter().filter(|push| matches!(push, Push::Packets(bytes) if !bytes.is_empty() && bytes.len() % 42 == 0)).count();
        assert_eq!(sounds, PIANO.len() + 6, "the notes, the game's four sounds and the typing page's two");
        let pushes = feed(&mut session, &[Event::Down(Press { key: u16::from(b'a'), modifiers: 0, physical: neoteletext_input::names::physical_key("KeyA") })]);
        assert!(matches!(pushes.first(), Some(Push::Packets(bytes)) if bytes.len() == 42), "one command, one packet");
        assert_eq!(session.piano, Some(0));
        // The sounds decode with the engine's own receiver.
        let mut arena = vec![0_u8; 1 << 20];
        let mut receiver = neoteletext_audio::receiver::Receiver::<16, 4>::new(&mut arena);
        let mut sink = Count(Vec::new());
        let mut session = Session::new(Settings::default());
        let mut all = feed(&mut session, &[Event::Page(Some(0x400))]);
        all.extend(feed(&mut session, &[press('s')]));
        hear(&mut receiver, &mut sink, all);
        assert_eq!(sink.0.len(), 1, "the note played");
    }

    fn hear(receiver: &mut neoteletext_audio::receiver::Receiver<'_, 16, 4>, sink: &mut Count, pushes: Vec<Push>) {
        for push in pushes {
            if let Push::Packets(bytes) = push {
                for chunk in bytes.as_chunks::<42>().0 {
                    receiver.push_packet(&neoteletext_core::packet::Packet { bytes: *chunk }, sink);
                }
            }
        }
    }

    #[test]
    fn the_podcast_plays_on_space_on_a_channel_of_its_own() {
        let rate = 8_000;
        let episode = Episode { title: "Pilot".to_owned(), rate, samples: vec![500; usize::try_from(rate).unwrap() * 3] };
        let mut session = Session::new(Settings { relay: None, podcast: Some(Arc::new(episode)) });
        let mut arena = vec![0_u8; 1 << 20];
        let mut receiver = neoteletext_audio::receiver::Receiver::<16, 4>::new(&mut arena);
        let mut sink = Count(Vec::new());
        let mut pushes = feed(&mut session, &[Event::Page(Some(0x700))]);
        assert!(session.render().tti().contains("Ready: press Space"));
        pushes.extend(feed(&mut session, &[press(' ')]));
        assert!(session.render().tti().contains("Playing"));
        assert!(session.render().tti().contains("0:00 / 0:03"));
        // The typing page clicks on the effects' channels, never the podcast's.
        pushes.extend(feed(&mut session, &[Event::Page(Some(0x200)), press('a'), press('b'), press('c'), press('d')]));
        hear(&mut receiver, &mut sink, pushes);
        assert_eq!(sink.0, [podcast::CHANNEL, 0, 1, 2, 0], "the podcast, then four clicks in turn on the other three");
        feed(&mut session, &[press(' ')]);
        assert!(session.render().tti().contains("Last key: Space\r\n"), "the space bar named");
    }

    #[test]
    fn the_podcast_page_says_when_the_episode_ends_within_a_second() {
        // Ten and a half seconds: skipped to ten, it ends half a second later,
        // in the second it began.
        let rate = 8_000;
        let samples = usize::try_from(rate).unwrap() * 21 / 2;
        let episode = Episode { title: "Short".to_owned(), rate, samples: vec![500; samples] };
        let mut session = Session::new(Settings { relay: None, podcast: Some(Arc::new(episode)) });
        feed(&mut session, &[Event::Page(Some(0x700)), named("ArrowRight"), press(' ')]);
        session.podcast_frame();
        assert!(session.render().tti().contains("0:10 / 0:10"));
        thread::sleep(Duration::from_millis(700));
        let pushes = session.podcast_frame();
        assert!(matches!(pushes.last(), Some(Push::Page(tti)) if tti.contains("Finished")), "the stop, then the page");
    }

    #[test]
    fn the_game_has_no_cursor_and_moves_while_a_key_is_held() {
        let mut session = Session::new(Settings::default());
        let start = session.invaders.ship;
        feed(&mut session, &[Event::Page(Some(0x300)), named("ArrowLeft")]);
        assert_eq!(session.invaders.ship, start - 1, "a press moves at once");
        session.invaders.tick();
        session.invaders.tick();
        assert_eq!(session.invaders.ship, start - 3, "and held, each frame");
        assert!(session.render().tti().contains("CU,255,0,0,0,"));
    }

    #[test]
    fn the_game_sounds_a_shot_and_a_hit() {
        let mut session = Session::new(Settings::default());
        let opened = feed(&mut session, &[Event::Page(Some(0x300))]);
        assert_eq!(opened.iter().filter(|push| matches!(push, Push::Packets(_))).count(), PIANO.len() + 6, "its sounds first");
        let fired = feed(&mut session, &[Event::Down(Press { key: u16::from(b' '), modifiers: 0, physical: 0 })]);
        assert!(matches!(fired.first(), Some(Push::Packets(bytes)) if bytes.len() == 42), "the shot's command");
        // A shot one row below the bottom row's first invader hits it next frame.
        session.invaders.shots.clear();
        let (x, y) = *session.invaders.alive.last().unwrap();
        let (offset, drop) = (session.invaders.offset, session.invaders.drop);
        session.invaders.shots.push((x + offset + 1, INVADER_TOP + y + drop + 1));
        let frame = session.frame();
        assert!(matches!(frame.first(), Some(Push::Packets(bytes)) if bytes.len() == 42), "a hit sounds");
        assert_eq!(session.invaders.score, 10);
    }

    #[test]
    fn the_contents_file_is_the_engines_format() {
        let contents = contents_file();
        let read = neoteletext_web::contents::read(contents.as_bytes()).unwrap();
        assert_eq!(read.flatten().count(), PAGES.len() + 1);
    }

    #[test]
    fn the_page_says_when_the_keys_went_elsewhere() {
        let mut session = Session::new(Settings::default());
        feed(&mut session, &[Event::Page(Some(0x200)), Event::Focus(false)]);
        assert!(session.render().tti().contains("Your keys went elsewhere."));
        feed(&mut session, &[Event::Focus(true)]);
        assert!(!session.render().tti().contains("elsewhere"));
    }

    #[test]
    fn a_click_is_the_hosts() {
        let mut session = Session::new(Settings::default());
        feed(&mut session, &[Event::Page(Some(0x500)), Event::Click { column: 7, row: 11 }]);
        assert!(session.render().tti().contains("You clicked column 7, row 11."));
    }
}
