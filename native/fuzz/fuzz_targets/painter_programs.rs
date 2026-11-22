//! Painter programs (script-api.md section 11.4, target 22; section 10.2):
//! arbitrary whole numbers as a NAPLPS painter program, for the backdrop,
//! for a symbol and for a pattern. Nothing panics; a program is taken or
//! refused whole; what a program that is taken writes is within the room
//! it was given, and is NAPLPS the renderer reads with no byte passed over
//! and nothing refused but for its pixel work, which a picture may spend.
//! The ReGIS half of this target is work package R2's.
#![no_main]

use libfuzzer_sys::fuzz_target;
use naplps::{Refusal, Validator};
use neoteletext_script::host::naplps::limits::MAX_PAINTER_WORDS;
use neoteletext_script::host::naplps::program::Paint;
use neoteletext_script::host::naplps::writer::{NoDefinitions, Painter, Target};

/// The most calls one input gives.
const MAX_CALLS: usize = 256;
/// Whole numbers at the edges of the painter's ranges: the canvas's
/// sides, the places an operand reaches, the bounds on counts.
const EDGES: [i32; 20] = [0, 1, -1, 2, 3, 15, 16, 63, 64, 128, 459, 460, 479, 480, -480, 600, 919, 1_024, 4_096, i32::MAX];
/// The rooms a painting is given.
const ROOMS: [usize; 4] = [0, 64, 4_096, 65_536];
/// The languages text is written in.
const LANGUAGES: [&str; 4] = ["en", "", "fa", "no-such-language"];

/// Reads the input a piece at a time; past its end every read is zero.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((first, rest)) = self.0.split_first() else { return 0 };
        self.0 = rest;
        *first
    }

    fn int(&mut self) -> i32 {
        let choice = self.byte();
        match choice {
            0..100 => i32::from(choice),
            100..180 => EDGES[usize::from(choice) % EDGES.len()],
            180..220 => -i32::from(self.byte()),
            _ => i32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()]),
        }
    }
}

/// A program: calls of the table with whole numbers after them, lists and
/// strings with true and false lengths, and bare numbers.
fn program(input: &mut Bytes<'_>) -> Vec<i32> {
    let mut words = Vec::new();
    for _ in 0..MAX_CALLS {
        if input.0.is_empty() || words.len() >= MAX_PAINTER_WORDS {
            break;
        }
        let choice = input.byte();
        if choice < 230 {
            words.push(Paint::ALL[usize::from(choice) % Paint::ALL.len()].1.number());
        } else {
            words.push(input.int());
        }
        for _ in 0..input.byte() % 12 {
            words.push(input.int());
        }
        // A list or a string: a length, true or not, then that many words.
        let lists = input.byte() % 4;
        for _ in 0..lists.saturating_sub(1) {
            let length = usize::from(input.byte());
            words.push(if input.byte().is_multiple_of(8) { input.int() } else { i32::try_from(length).unwrap_or(0) });
            let character = input.byte().is_multiple_of(4);
            for _ in 0..length {
                words.push(if character { i32::from(b"xyd<^un aZ[{"[usize::from(input.byte()) % 12]) } else { input.int() });
            }
        }
    }
    words
}

fuzz_target!(|data: &[u8]| {
    let mut input = Bytes(data);
    let target = [Target::Backdrop, Target::Symbol, Target::Pattern][usize::from(input.byte()) % 3];
    let room = ROOMS[usize::from(input.byte()) % ROOMS.len()];
    let language = LANGUAGES[usize::from(input.byte()) % LANGUAGES.len()];
    let words = program(&mut input);
    let mut stream = Vec::new();
    Painter::start_generation(&mut stream);
    let mut painter = if target == Target::Backdrop { Painter::backdrop() } else { Painter::definition(target) };
    if target == Target::Backdrop {
        painter.end(&mut stream);
    } else {
        Painter::backdrop().end(&mut stream);
    }
    let mut out = Vec::new();
    if painter.run(&words, language, &NoDefinitions, &mut out, room, "ROOM").is_err() {
        return;
    }
    assert!(out.len() <= room, "{} bytes written into a room of {room}", out.len());
    stream.extend_from_slice(&out);
    let mut validator = Validator::new();
    validator.feed(&stream);
    validator.finish();
    let diagnostics = validator.diagnostics();
    assert_eq!(diagnostics.passed_over, 0, "the renderer passed bytes over: {diagnostics:?}\n{stream:02X?}");
    for kind in Refusal::ALL {
        assert!(kind == Refusal::WorkLimit || diagnostics.count(kind) == 0, "the renderer refused {kind:?}: {diagnostics:?}\n{stream:02X?}");
    }
});
