//! The phone keyboard itself: the instance, the QR, pairing and the keys.

use std::io::{self, BufReader, BufWriter};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use neoteletext_input::key::Limiter;
use neoteletext_qrkeyboard::config::{Layout, ScreenConfig};
use neoteletext_qrkeyboard::qr::{Correction, Matrix};
use neoteletext_qrkeyboard::screen::{Event, Screen};
use protocol::keyboard::{FromKeyboard, MAX_PHONE_TEXT, PhoneState, Qr, ToKeyboard, read_to_keyboard, write_to_core};

/// What the phone shows as the screen's name.
const NAME: &str = "NeoTeletext";
/// A dropped phone keeps the browser for at most this long (requirement
/// 18.9), not qrkeyboard's default five minutes.
const REJOIN_SECONDS: u32 = 30;
/// How long one wait on the relay lasts: short, so Finish acts at once.
const POLL: Duration = Duration::from_millis(50);

/// The QR's modules, one bit each.
fn qr(matrix: &Matrix) -> Qr {
    let size = matrix.size;
    let mut bits = vec![0_u8; size.saturating_mul(size).div_ceil(8)];
    for y in 0..size {
        for x in 0..size {
            if matrix.dark(x, y) {
                let index = y.saturating_mul(size).saturating_add(x);
                if let Some(byte) = bits.get_mut(index / 8) {
                    *byte |= 0x80 >> (index % 8);
                }
            }
        }
    }
    Qr { size, bits }
}

/// The milliseconds the rate limit counts, from the process's start.
fn clock(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis() & u128::from(u32::MAX)).unwrap_or(0)
}

fn run(relay: &str, pin: bool) -> Result<(), String> {
    let mut output = BufWriter::new(io::stdout().lock());
    let mut send = |message: &FromKeyboard| write_to_core(&mut output, message).map_err(|error| error.to_string());
    send(&FromKeyboard::State(PhoneState::Preparing))?;
    let config = ScreenConfig { name: NAME.into(), layout: Layout::Terminal, pin, rejoin_seconds: Some(REJOIN_SECONDS) };
    let mut screen = match Screen::create(relay, config) {
        Ok(screen) => screen,
        Err(error) => {
            send(&FromKeyboard::State(PhoneState::Finished))?;
            return Err(format!("no phone keyboard: {error}"));
        }
    };
    let matrix = Matrix::new(&screen.join().url, Correction::Medium).map_err(|error| error.to_string())?;
    send(&FromKeyboard::Join { qr: qr(&matrix), pin: screen.join().pin.clone() })?;
    // The trusted core's Finish, or the end of its pipe.
    let (finish, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = BufReader::new(io::stdin().lock());
        let finish_asked = matches!(read_to_keyboard(&mut input), Ok(Some(ToKeyboard::Finish)));
        let _ = finish.send(finish_asked);
    });
    let started = Instant::now();
    let mut limiter = Limiter::new();
    loop {
        if let Ok(asked) = finished.try_recv() {
            if asked {
                let _ = screen.finish();
                send(&FromKeyboard::State(PhoneState::Finished))?;
            } else {
                screen.close();
            }
            return Ok(());
        }
        let events = screen.poll(POLL).map_err(|error| error.to_string())?;
        for event in events {
            let message = match event {
                Event::Scan => FromKeyboard::State(PhoneState::Scan),
                Event::Connected => FromKeyboard::State(PhoneState::Connected),
                Event::Disconnected(_) => FromKeyboard::State(PhoneState::Waiting),
                Event::PinFailed(left) => FromKeyboard::State(PhoneState::PinFailed(u8::try_from(left).unwrap_or(u8::MAX))),
                Event::Locked => FromKeyboard::State(PhoneState::Locked),
                Event::Finished => {
                    send(&FromKeyboard::State(PhoneState::Finished))?;
                    return Ok(());
                }
                // At most 120 keys a second, and never a key-up dropped.
                Event::Down(name) if limiter.allow(clock(started), false) => FromKeyboard::Key { down: true, name },
                Event::Up(name) if limiter.allow(clock(started), true) => FromKeyboard::Key { down: false, name },
                Event::Text(text) if limiter.allow(clock(started), false) => {
                    let end = (0..=text.len().min(MAX_PHONE_TEXT)).rev().find(|&end| text.is_char_boundary(end)).unwrap_or(0);
                    let printable: String = text.get(..end).unwrap_or_default().chars().filter(|character| !character.is_control()).collect();
                    if printable.is_empty() {
                        continue;
                    }
                    FromKeyboard::Text(printable)
                }
                Event::Special { key, count } => {
                    // A bulk Backspace is that many presses, within the rate.
                    for _ in 0..count {
                        if !limiter.allow(clock(started), false) {
                            break;
                        }
                        send(&FromKeyboard::Key { down: true, name: key.to_owned() })?;
                        send(&FromKeyboard::Key { down: false, name: key.to_owned() })?;
                    }
                    continue;
                }
                Event::Down(_) | Event::Up(_) | Event::Text(_) => continue,
            };
            send(&message)?;
        }
    }
}

/// Runs the phone keyboard until the trusted core ends it.
pub fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let pin = arguments.iter().any(|argument| argument == "--pin");
    let relay = arguments.iter().find(|argument| !argument.starts_with("--")).map_or(neoteletext_qrkeyboard::RELAY, String::as_str);
    match run(relay, pin) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-keyboard: {error}");
            ExitCode::FAILURE
        }
    }
}
