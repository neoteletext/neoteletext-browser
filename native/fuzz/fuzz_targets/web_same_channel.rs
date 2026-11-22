//! The web client's test of whether an address is inside a channel
//! (neoteletext-common's DEC-7264 item 3; finding S-02, TODO-7267): an
//! address built from the channel's folder and arbitrary bytes, read as a
//! server that decodes a path before resolving it would read it, is never
//! judged inside the channel when that reading leaves the folder.
#![no_main]

use libfuzzer_sys::fuzz_target;

const ORIGIN: &str = "http://host.test";
const FOLDER: &str = "/home/";
/// The decodings a server is taken to make at most: one, and one more
/// behind a proxy that decodes too.
const DECODINGS: usize = 2;

fn hex(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).and_then(|digit| u8::try_from(digit).ok())
}

/// `path` percent-decoded once.
fn decoded(path: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(path.len());
    let mut bytes = path.iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        let mut ahead = bytes.clone();
        match (byte, ahead.next().and_then(hex), ahead.next().and_then(hex)) {
            (b'%', Some(high), Some(low)) => {
                out.push(high * 16 + low);
                bytes = ahead;
            }
            _ => out.push(byte),
        }
    }
    out
}

/// `path` as a lenient server resolves it: cut at a NUL, backslashes as
/// slashes, each segment without its parameters, dot segments resolved.
/// `None` when it climbs above the root.
fn resolved(path: &[u8]) -> Option<Vec<Vec<u8>>> {
    let path = path.split(|&byte| byte == 0).next().unwrap_or_default();
    let mut segments: Vec<Vec<u8>> = Vec::new();
    for segment in path.split(|&byte| byte == b'/' || byte == b'\\') {
        let segment = segment.split(|&byte| byte == b';').next().unwrap_or_default();
        match segment {
            b"." | b"" => {}
            b".." => {
                segments.pop()?;
            }
            _ => segments.push(segment.to_vec()),
        }
    }
    Some(segments)
}

fuzz_target!(|data: &[u8]| {
    let Ok(rest) = std::str::from_utf8(data) else { return };
    let Some((inside, path)) = fetcher::web_fuzzing::channel_holds(&format!("{ORIGIN}{FOLDER}"), &format!("{ORIGIN}{FOLDER}{rest}")) else { return };
    if !inside {
        return;
    }
    let mut read = path.into_bytes();
    for _ in 0..=DECODINGS {
        let left_folder = resolved(&read).is_none_or(|segments| segments.first().map(Vec::as_slice) != Some(b"home"));
        assert!(!left_folder, "{} is judged inside {FOLDER} and read outside it", String::from_utf8_lossy(&read));
        read = decoded(&read);
    }
});
