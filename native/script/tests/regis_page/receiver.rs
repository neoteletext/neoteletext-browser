//! What the decoder and its renderer are to a script's batches on a
//! ReGIS page, for the tests of the ReGIS API and of the components on
//! ReGIS: the renderer's own `Terminal`, fed exactly the bytes and
//! animation operations each batch carries, and the kept pictures its
//! pictures are compared with.
#![allow(dead_code, reason = "each test file uses its own part of this receiver")]

use std::fmt::Write as _;
use std::path::PathBuf;

use neoteletext_rules::animate::OperationKind;
use neoteletext_script::limits::MAX_BATCH_BYTES;
use protocol::script::{Batch, Bounds};
use regis::{Event as RegisEvent, ForcedLook, Picture, SCREEN_HEIGHT, SCREEN_WIDTH, Screen, Terminal};

use super::common::view;

/// The version of the kept pictures' format: the digest of a picture's
/// 800 by 480 colour-map addresses followed by its sixteen colours. A
/// change to what is painted raises it and writes the file again.
pub const GOLDEN_FORMAT: u32 = 1;

/// What the decoder and its renderer are to a script's batches: the
/// renderer's own terminal, fed each batch's ReGIS and animation
/// operations, and asked for the picture, the painted report, bounds and
/// picks.
pub struct Receiver {
    pub terminal: Terminal,
    pub screen: Screen,
    /// The commits applied.
    pub commits: u32,
    /// Every byte of ReGIS taken.
    pub bytes: usize,
}

impl Receiver {
    pub fn new() -> Self {
        // The fonts are the receiver's, given as the decoder gives them.
        let mut terminal = Terminal::new();
        super::fonts::give(&mut terminal);
        Self { terminal, screen: Screen::new(), commits: 0, bytes: 0 }
    }

    /// Takes a batch as the decoder does: its ReGIS appended to the
    /// page's one stream, then its animation operations, then its watch
    /// list. The receiver never refuses a commit: the host's validator is
    /// the renderer's own.
    #[track_caller]
    pub fn take(&mut self, batch: &Batch) {
        let _ = view(batch);
        if let Some(bytes) = &batch.regis {
            assert!(bytes.len() <= MAX_BATCH_BYTES);
            self.bytes += bytes.len();
            let events = self.terminal.feed_regis(&mut self.screen, bytes);
            assert!(!events.iter().any(|event| matches!(event, RegisEvent::CommitFailed { .. })), "the receiver refused what the host sent: {events:?}");
            self.commits += u32::try_from(events.iter().filter(|event| matches!(event, RegisEvent::Committed(_))).count()).unwrap();
        }
        for operation in batch.animate.iter().flatten() {
            let mut state = self.terminal.animation(operation.target.number).unwrap_or_default();
            match operation.kind {
                OperationKind::Frame(frame) | OperationKind::Cycle { first: frame, .. } => state.frame = frame,
                OperationKind::Look(look) => {
                    state.look = [ForcedLook::None, ForcedLook::Hover, ForcedLook::Pressed, ForcedLook::Alternative][usize::from(look)];
                }
                _ => {}
            }
            assert!(self.terminal.set_animation(operation.target.number, state), "{operation:?}");
        }
        if let Some(painted) = batch.watch.as_ref().and_then(|watch| watch.painted.as_ref()) {
            let numbers: Vec<u16> = painted.iter().map(|handle| u16::try_from(*handle).unwrap()).collect();
            assert!(self.terminal.watch_painted(&numbers));
        }
    }

    pub fn picture(&mut self) -> Picture {
        let mut picture = Picture::new();
        self.terminal.compose(&self.screen, &mut picture);
        picture
    }

    /// The bounds of the drawings `numbers`, as a step reports them.
    pub fn bounds(&self, numbers: &[u16]) -> Vec<Bounds> {
        numbers
            .iter()
            .filter_map(|number| {
                let found = self.terminal.bounds(*number)?;
                let int = |value: u32| i32::try_from(value).unwrap();
                Some(Bounds { handle: i32::from(*number), x: int(found.x), y: int(found.y), width: int(found.width), height: int(found.height) })
            })
            .collect()
    }
}

/// The digest of a picture: its pixels, then its colours.
pub fn digest(picture: &Picture, terminal: &Terminal) -> String {
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    context.update(picture.canvas().pixels());
    for colour in terminal.palette().colours() {
        context.update(&colour);
    }
    context.finish().as_ref().iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

/// A PNG chunk: its length, type, data and CRC-32 (the PNG
/// specification, section 5.3; the CRC is ISO 3309's, polynomial
/// 0xEDB88320 reflected).
fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    let start = out.len();
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let mut crc = u32::MAX;
    for byte in &out[start..] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

/// Writes a picture as a PNG with a palette, its pixel data stored and
/// not compressed (RFC 1950 and RFC 1951 stored blocks), where a person
/// can look at it; gives the file's path.
pub fn write_png(name: &str, picture: &Picture, terminal: &Terminal) -> PathBuf {
    let (width, height) = (usize::try_from(SCREEN_WIDTH).unwrap(), usize::try_from(SCREEN_HEIGHT).unwrap());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&u32::try_from(width).unwrap().to_be_bytes());
    header.extend_from_slice(&u32::try_from(height).unwrap().to_be_bytes());
    // Eight bits a pixel, colour type 3 (a palette), no interlace.
    header.extend_from_slice(&[8, 3, 0, 0, 0]);
    chunk(&mut png, *b"IHDR", &header);
    chunk(&mut png, *b"PLTE", &terminal.palette().colours().concat());
    let mut raw = Vec::new();
    for row in picture.canvas().pixels().chunks(width) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut data = vec![0x78, 0x01];
    let (mut low, mut high) = (1_u32, 0_u32);
    let blocks = raw.chunks(65_535).count();
    for (index, block) in raw.chunks(65_535).enumerate() {
        let length = u16::try_from(block.len()).unwrap();
        data.push(u8::from(index + 1 == blocks));
        data.extend_from_slice(&length.to_le_bytes());
        data.extend_from_slice(&(!length).to_le_bytes());
        data.extend_from_slice(block);
        for byte in block {
            low = (low + u32::from(*byte)) % 65_521;
            high = (high + low) % 65_521;
        }
    }
    data.extend_from_slice(&((high << 16) | low).to_be_bytes());
    chunk(&mut png, *b"IDAT", &data);
    chunk(&mut png, *b"IEND", &[]);
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("regis-{name}.png"));
    std::fs::write(&path, png).expect("the picture is written");
    path
}

/// Compares the receiver's picture with the one named `name` among the
/// kept pictures `golden`, and writes it as a PNG. The digests that
/// differ are gathered in `wrong`, so that one run shows every picture
/// that changed.
pub fn keep(golden: &str, name: &str, receiver: &mut Receiver, wrong: &mut Vec<String>) {
    let picture = receiver.picture();
    let found = digest(&picture, &receiver.terminal);
    let path = write_png(name, &picture, &receiver.terminal);
    let mut kept = golden.lines().filter(|line| !line.starts_with('#'));
    assert_eq!(kept.next(), Some(format!("format {GOLDEN_FORMAT}").as_str()), "the kept pictures' format");
    let line = format!("{name} {found}");
    if !kept.any(|kept| kept == line) {
        wrong.push(format!("{line}    (see {})", path.display()));
    }
}
