//! The channel's HTTP: the descriptor, the contents and page files, and
//! each viewer's WebSocket, its session.

use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use neoteletext_input::event::{MAX_STREAM_ITEM, StreamDecoder};

use super::pages::{contents_file, page_file};
use super::session::Push;
use super::{CHANNEL, Session, Settings};
use crate::websocket;

/// The longest request head read.
const MAX_HEAD: usize = 8 * 1024;
/// How long a session waits for a key before the game moves on: ten
/// frames a second (`interaction.md` section 2: "twenty times a second if
/// it likes").
const FRAME: Duration = Duration::from_millis(100);

fn respond(stream: &mut TcpStream, status: &str, media: &str, body: &[u8]) -> io::Result<()> {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {media}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n", body.len())?;
    stream.write_all(body)?;
    stream.flush()
}

/// A viewer's WebSocket: the input stream in, pages and sounds out, and the
/// game's frames while its page is open.
fn session(mut stream: TcpStream, settings: Settings) -> io::Result<()> {
    stream.set_read_timeout(Some(FRAME))?;
    stream.set_nodelay(true)?;
    let mut reader = websocket::Reader::default();
    let mut session = Session::new(settings);
    // The viewer's input stream, read across messages (INPUT.md section
    // 5.1); a protocol error closes the connection (section 8).
    let mut input = StreamDecoder::<MAX_STREAM_ITEM>::default();
    let mut last_frame = Instant::now();
    loop {
        let Some(frames) = reader.read(&mut stream)? else { return Ok(()) };
        for (opcode, payload) in frames {
            match opcode {
                websocket::BINARY => {
                    let mut pushes = Vec::new();
                    if let Err(error) = input.push(&payload, &mut |item| pushes.extend(session.item(&item))) {
                        eprintln!("neoteletext-site: input refused: {error:?}");
                        let _ = websocket::write(&mut stream, websocket::CLOSE, &[]);
                        return Ok(());
                    }
                    for push in pushes {
                        send(&mut stream, push)?;
                    }
                }
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

pub(super) fn serve(mut stream: TcpStream, port: u16, settings: &Settings) -> io::Result<()> {
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
