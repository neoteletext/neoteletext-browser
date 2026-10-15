//! Proof C. The stand-in decoder runs as a child process. Rust asks it for
//! `pages` pages, then sends flash ticks so that every page is rendered
//! again on every tick: the worst case. Each frame is encoded as an indexed
//! image and sent to the page over a channel as raw bytes:
//! [page: u16][encode microseconds: u32][image bytes], all big-endian.

use std::io::{BufReader, BufWriter};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use standin::pipe::{self, FIRST_PAGE, LAST_PAGE, Message};
use tauri::ipc::{Channel, InvokeResponseBody};

const CHANNEL_NUMBER: u32 = 1;
const MAX_PAGES: u16 = LAST_PAGE - FIRST_PAGE + 1;

#[tauri::command]
fn start(frames: Channel<InvokeResponseBody>, pages: u16, ticks: u32, tick_milliseconds: u64) -> Result<(), String> {
    let pages = pages.min(MAX_PAGES);
    let decoder = std::env::var("STANDIN_DECODER").map_err(|_| "STANDIN_DECODER is not set".to_owned())?;
    let mut child = Command::new(decoder).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().map_err(|error| error.to_string())?;
    let stdin = child.stdin.take().ok_or("no decoder input")?;
    let stdout = child.stdout.take().ok_or("no decoder output")?;

    std::thread::spawn(move || {
        let mut writer = BufWriter::new(stdin);
        for offset in 0..pages {
            let show = Message::Show { channel: CHANNEL_NUMBER, page: FIRST_PAGE + offset, subcode: 0 };
            if pipe::write(&mut writer, &show).is_err() {
                return;
            }
        }
        for tick in 0..ticks {
            std::thread::sleep(Duration::from_millis(tick_milliseconds));
            if pipe::write(&mut writer, &Message::Tick { flash_on: tick % 2 == 0 }).is_err() {
                return;
            }
        }
        let _ = pipe::write(&mut writer, &Message::Close);
    });

    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let started = Instant::now();
        let (mut count, mut image_bytes, mut encode_micros) = (0_u64, 0_u64, 0_u128);
        while let Ok(Some(Message::Frame(frame))) = pipe::read(&mut reader) {
            let encode_started = Instant::now();
            let image = standin::png::encode(&frame);
            let micros = encode_started.elapsed().as_micros();
            encode_micros += micros;
            image_bytes += image.len() as u64;
            count += 1;
            let mut message = Vec::with_capacity(6 + image.len());
            message.extend_from_slice(&(frame.page - FIRST_PAGE).to_be_bytes());
            message.extend_from_slice(&u32::try_from(micros).unwrap_or(u32::MAX).to_be_bytes());
            message.extend_from_slice(&image);
            if frames.send(InvokeResponseBody::Raw(message)).is_err() {
                break;
            }
        }
        let _ = child.wait();
        if let (Some(mean_bytes), Some(mean_micros)) = (image_bytes.checked_div(count), encode_micros.checked_div(u128::from(count))) {
            println!(
                "RUST frames sent {count} in {:.2} s; mean image {mean_bytes} bytes; mean encode {mean_micros} microseconds",
                started.elapsed().as_secs_f64()
            );
        }
    });
    Ok(())
}

/// The run's parameters, from the environment: pages, ticks, tick interval.
#[tauri::command]
fn parameters() -> (u16, u32, u64) {
    let read = |name: &str, default: u64| std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default);
    let pages = u16::try_from(read("SPIKE_PAGES", 50)).unwrap_or(MAX_PAGES);
    let ticks = u32::try_from(read("SPIKE_TICKS", 10)).unwrap_or(u32::MAX);
    (pages, ticks, read("SPIKE_TICK_MS", 500))
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value, reason = "Tauri extracts command arguments by value")]
fn report(app: tauri::AppHandle, lines: Vec<String>) {
    for line in &lines {
        println!("PAGE {line}");
    }
    app.exit(0);
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![start, parameters, report])
        .run(tauri::generate_context!())
        .expect("Tauri runtime failed to start");
}
