//! The phone keyboard itself: the instance, the QR, pairing and the keys.

use std::io::{self, BufReader, BufWriter, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use neoteletext_input::session::Limiter;
use neoteletext_qrkeyboard::config::{Layout, ScreenConfig};
use neoteletext_qrkeyboard::qr::{Correction, Matrix};
use neoteletext_qrkeyboard::screen::{Event, Screen};
use neoteletext_web::proxy::Proxy;
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

fn run(relay: &str, pin: bool, proxy: Option<Proxy>) -> Result<(), String> {
    let mut output = BufWriter::new(io::stdout().lock());
    let send = |output: &mut BufWriter<io::StdoutLock<'_>>, message: &FromKeyboard| write_to_core(output, message).map_err(|error| error.to_string());
    send(&mut output, &FromKeyboard::State(PhoneState::Preparing))?;
    let config = ScreenConfig { name: NAME.into(), layout: Layout::Terminal, pin, rejoin_seconds: Some(REJOIN_SECONDS) };
    let mut screen = match Screen::create(relay, config, proxy) {
        Ok(screen) => screen,
        Err(error) => {
            send(&mut output, &FromKeyboard::State(PhoneState::Finished))?;
            return Err(format!("no phone keyboard: {error}"));
        }
    };
    let matrix = Matrix::new(&screen.join().url, Correction::Medium).map_err(|error| error.to_string())?;
    send(&mut output, &FromKeyboard::Join { qr: qr(&matrix), pin: screen.join().pin.clone() })?;
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
                send(&mut output, &FromKeyboard::State(PhoneState::Finished))?;
            } else {
                screen.close();
            }
            return Ok(());
        }
        // What the last events produced goes before waiting.
        output.flush().map_err(|error| error.to_string())?;
        let events = screen.poll(POLL).map_err(|error| error.to_string())?;
        for event in events {
            let message = match event {
                Event::Scan => FromKeyboard::State(PhoneState::Scan),
                Event::Connected => FromKeyboard::State(PhoneState::Connected),
                Event::Disconnected(_) => FromKeyboard::State(PhoneState::Waiting),
                Event::PinFailed(left) => FromKeyboard::State(PhoneState::PinFailed(u8::try_from(left).unwrap_or(u8::MAX))),
                Event::Locked => FromKeyboard::State(PhoneState::Locked),
                Event::Finished => {
                    send(&mut output, &FromKeyboard::State(PhoneState::Finished))?;
                    return Ok(());
                }
                // At most 120 input units a second, a character of text
                // each, and never a key-up dropped (REQ-INT-109).
                Event::Down(name) if limiter.allow(clock(started), false) => FromKeyboard::Key { down: true, name },
                Event::Up(name) if limiter.allow(clock(started), true) => FromKeyboard::Key { down: false, name },
                Event::Text(text) => {
                    let end = (0..=text.len().min(MAX_PHONE_TEXT)).rev().find(|&end| text.is_char_boundary(end)).unwrap_or(0);
                    let printable: String = text
                        .get(..end)
                        .unwrap_or_default()
                        .chars()
                        .filter(|character| !character.is_control())
                        .take_while(|_| limiter.allow(clock(started), false))
                        .collect();
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
                        send(&mut output, &FromKeyboard::Key { down: true, name: key.to_owned() })?;
                        send(&mut output, &FromKeyboard::Key { down: false, name: key.to_owned() })?;
                    }
                    continue;
                }
                Event::Down(_) | Event::Up(_) => continue,
            };
            send(&mut output, &message)?;
        }
    }
}

/// What the trusted core started the keyboard with: `[relay] [--pin]
/// [--proxy URL]`. The proxy is given only at the start, as a fetcher's is
/// (requirement 19.12).
struct Arguments {
    relay: String,
    pin: bool,
    proxy: Option<Proxy>,
}

fn arguments(given: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    let mut read = Arguments { relay: neoteletext_qrkeyboard::RELAY.to_owned(), pin: false, proxy: None };
    let mut given = given.into_iter();
    while let Some(argument) = given.next() {
        match argument.as_str() {
            "--pin" => read.pin = true,
            "--proxy" => read.proxy = Some(Proxy::parse(&given.next().ok_or("--proxy needs an address")?)?),
            other if other.starts_with("--") => return Err(format!("{other}: not understood")),
            _ => read.relay = argument,
        }
    }
    Ok(read)
}

/// Runs the phone keyboard until the trusted core ends it.
pub fn main() -> ExitCode {
    // First, before any thread: the network and nothing else. What
    // confined it is not printed: this process prints one thing, why it
    // stopped (REQ-INT-137).
    match confine::apply(confine::Role::Network, None)
        .map(drop)
        .and_then(|()| arguments(std::env::args().skip(1)))
        .and_then(|read| run(&read.relay, read.pin, read.proxy))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-keyboard: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    fn read(given: &[&str]) -> Result<Arguments, String> {
        arguments(given.iter().map(|&argument| argument.to_owned()))
    }

    /// Requirement 19.12: the proxy comes only as an argument at the start.
    #[test]
    fn the_keyboard_reads_its_relay_pin_and_proxy() {
        let plain = read(&[]).unwrap();
        assert_eq!((plain.relay.as_str(), plain.pin, plain.proxy.is_none()), (neoteletext_qrkeyboard::RELAY, false, true));
        let given = read(&["wss://relay.example/", "--pin", "--proxy", "socks5://127.0.0.1:1080"]).unwrap();
        assert_eq!((given.relay.as_str(), given.pin), ("wss://relay.example/", true));
        let proxy = given.proxy.unwrap();
        assert_eq!((proxy.host.as_str(), proxy.port), ("127.0.0.1", 1080));
        assert!(read(&["--proxy"]).is_err());
        assert!(read(&["--proxy", "ftp://p.example:21"]).is_err());
        assert!(read(&["--other"]).is_err());
    }
}
