//! Operation `DECODE` (49): the Encoding standard's decoders by the
//! pure-Rust `encoding_rs`, which implements that standard and no other
//! (script-api.md section 9.1; DEC-7260 item 5). The prelude's
//! `TextDecoder` asks here for a label's encoding and for every decoding;
//! its `TextEncoder`, which the standard makes UTF-8 alone, asks here for
//! a string's bytes.
//!
//! UTF-8 needs no state between two pieces of a stream that the prelude
//! cannot keep itself (at most three bytes of an unfinished character),
//! so its pieces are decoded whole, with nothing held here. The other
//! encodings' streams hold a decoder here, at most [`MAX_DECODERS`] for a
//! context.

use encoding_rs::{CoderResult, Decoder, DecoderResult, Encoding};

use super::super::Reply;
use super::limits::{MAX_DECODERS, MAX_ENCODING_LABEL_BYTES};

/// The bits of operation `DECODE`'s flags.
pub(super) mod flag {
    /// An error is a `TypeError`, not a replacement character.
    pub const FATAL: i32 = 1;
    /// A byte order mark is kept, not removed.
    pub const IGNORE_BOM: i32 = 2;
    /// More of the stream follows: the decoder is kept.
    pub const STREAM: i32 = 4;
    /// The payload is a label: which encoding it names is asked.
    pub const LABEL: i32 = 8;
    /// The payload is a string: its UTF-8 bytes are asked.
    pub const ENCODE: i32 = 16;
    /// A streaming decoder is asked for; its number is the result.
    pub const OPEN: i32 = 32;
    /// Every bit.
    pub const ALL: i32 = FATAL | IGNORE_BOM | STREAM | LABEL | ENCODE | OPEN;
    /// Each bit with the name the prelude knows it by.
    pub const NAMES: [(&str, i32); 6] =
        [("FATAL", FATAL), ("IGNORE_BOM", IGNORE_BOM), ("STREAM", STREAM), ("LABEL", LABEL), ("ENCODE", ENCODE), ("OPEN", OPEN)];
}

/// The encodings a `TextDecoder` may be made for, by the number the
/// prelude names each with: every encoding of the Encoding standard
/// (section 4.2, "Names and labels") but `replacement`, which the
/// standard's constructor refuses.
static ENCODINGS: [&Encoding; 39] = [
    encoding_rs::UTF_8,
    encoding_rs::IBM866,
    encoding_rs::ISO_8859_2,
    encoding_rs::ISO_8859_3,
    encoding_rs::ISO_8859_4,
    encoding_rs::ISO_8859_5,
    encoding_rs::ISO_8859_6,
    encoding_rs::ISO_8859_7,
    encoding_rs::ISO_8859_8,
    encoding_rs::ISO_8859_8_I,
    encoding_rs::ISO_8859_10,
    encoding_rs::ISO_8859_13,
    encoding_rs::ISO_8859_14,
    encoding_rs::ISO_8859_15,
    encoding_rs::ISO_8859_16,
    encoding_rs::KOI8_R,
    encoding_rs::KOI8_U,
    encoding_rs::MACINTOSH,
    encoding_rs::WINDOWS_874,
    encoding_rs::WINDOWS_1250,
    encoding_rs::WINDOWS_1251,
    encoding_rs::WINDOWS_1252,
    encoding_rs::WINDOWS_1253,
    encoding_rs::WINDOWS_1254,
    encoding_rs::WINDOWS_1255,
    encoding_rs::WINDOWS_1256,
    encoding_rs::WINDOWS_1257,
    encoding_rs::WINDOWS_1258,
    encoding_rs::X_MAC_CYRILLIC,
    encoding_rs::GBK,
    encoding_rs::GB18030,
    encoding_rs::BIG5,
    encoding_rs::EUC_JP,
    encoding_rs::ISO_2022_JP,
    encoding_rs::SHIFT_JIS,
    encoding_rs::EUC_KR,
    encoding_rs::UTF_16BE,
    encoding_rs::UTF_16LE,
    encoding_rs::X_USER_DEFINED,
];

/// One streaming decoder a context holds.
struct Held {
    id: i32,
    decoder: Decoder,
    fatal: bool,
}

/// A context's streaming decoders.
#[derive(Default)]
pub(super) struct Decoders {
    held: Vec<Held>,
    next: i32,
}

/// The `TypeError` the standard's `decode` throws for malformed input
/// when the decoder is fatal (Encoding standard section 7.1).
fn malformed() -> Reply {
    Reply::Type("the encoded data was not valid".to_owned())
}

impl Decoders {
    /// The bytes held for the decoders.
    pub(super) fn state_bytes(&self) -> usize {
        self.held.len().saturating_mul(size_of::<Held>())
    }

    /// One call of operation `DECODE`.
    pub(super) fn operate(&mut self, index: i32, flags: i32, id: i32, text: Option<&str>, bytes: Option<&[u8]>) -> Reply {
        if flags & !flag::ALL != 0 {
            return Reply::Type("no such flag of a decoding".to_owned());
        }
        if flags & flag::LABEL != 0 {
            let Some(label) = text else { return Reply::Type("a label is a string".to_owned()) };
            return label_of(label);
        }
        if flags & flag::ENCODE != 0 {
            // The shim wrote the string as UTF-8 and the host checked that
            // it is: its bytes are the standard's encoding of it.
            return text.map_or_else(|| Reply::Type("what is encoded is a string".to_owned()), |text| Reply::Bytes(text.as_bytes().to_vec()));
        }
        let Some(encoding) = usize::try_from(index).ok().and_then(|index| ENCODINGS.get(index)).copied() else {
            return Reply::Type("no such encoding".to_owned());
        };
        let (fatal, ignore_bom) = (flags & flag::FATAL != 0, flags & flag::IGNORE_BOM != 0);
        if flags & flag::OPEN != 0 {
            if self.held.len() >= MAX_DECODERS {
                return Reply::Range("MAX_DECODERS: the context holds as many unfinished decodings as it may".to_owned());
            }
            self.next = self.next.checked_add(1).unwrap_or(1);
            let decoder = if ignore_bom { encoding.new_decoder_without_bom_handling() } else { encoding.new_decoder_with_bom_removal() };
            self.held.push(Held { id: self.next, decoder, fatal });
            return Reply::Int(self.next);
        }
        let Some(bytes) = bytes else { return Reply::Type("what is decoded is bytes".to_owned()) };
        let last = flags & flag::STREAM == 0;
        if id == 0 {
            // A whole decoding, or a piece of UTF-8 the prelude cut at a
            // character's edge: nothing is kept.
            let mut decoder = if ignore_bom { encoding.new_decoder_without_bom_handling() } else { encoding.new_decoder_with_bom_removal() };
            return decode(&mut decoder, bytes, true, fatal);
        }
        let Some(place) = self.held.iter().position(|held| held.id == id) else { return Reply::Type("no such decoder".to_owned()) };
        let Some(held) = self.held.get_mut(place) else { return Reply::Type("no such decoder".to_owned()) };
        let reply = decode(&mut held.decoder, bytes, last, held.fatal);
        // A decoder that reached its stream's end, or failed, is done.
        if last || !matches!(reply, Reply::Text(_)) {
            self.held.remove(place);
        }
        reply
    }
}

/// The encoding a label names, as its number, a line feed and its name
/// in lower case, as `TextDecoder.encoding` gives it (Encoding standard
/// section 7.1); undefined for no encoding and for `replacement`.
fn label_of(label: &str) -> Reply {
    if label.len() > MAX_ENCODING_LABEL_BYTES {
        return Reply::Undefined;
    }
    let Some(encoding) = Encoding::for_label(label.as_bytes()) else { return Reply::Undefined };
    ENCODINGS
        .iter()
        .position(|one| *one == encoding)
        .map_or(Reply::Undefined, |index| Reply::Text(format!("{index}\n{}", encoding.name().to_ascii_lowercase())))
}

/// Decodes `bytes` with `decoder`, the stream ending here when `last`.
fn decode(decoder: &mut Decoder, bytes: &[u8], last: bool, fatal: bool) -> Reply {
    let room = if fatal { decoder.max_utf8_buffer_length_without_replacement(bytes.len()) } else { decoder.max_utf8_buffer_length(bytes.len()) };
    let Some(room) = room else { return Reply::Range("MAX_HOST_BYTES: too much to decode at once".to_owned()) };
    let mut out = String::with_capacity(room);
    if fatal {
        match decoder.decode_to_string_without_replacement(bytes, &mut out, last) {
            (DecoderResult::InputEmpty, _) => Reply::Text(out),
            // The room asked for is the decoder's own worst case, so it is
            // never short; were it, the text would be short, and that is
            // an error and not a result.
            (DecoderResult::Malformed(_, _) | DecoderResult::OutputFull, _) => malformed(),
        }
    } else {
        match decoder.decode_to_string(bytes, &mut out, last) {
            (CoderResult::InputEmpty, _, _) => Reply::Text(out),
            (CoderResult::OutputFull, _, _) => Reply::Error("a decoding did not fit the room its decoder asked for".to_owned()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, reason = "tests")]
mod tests {
    use super::*;

    fn text(reply: Reply) -> String {
        match reply {
            Reply::Text(text) => text,
            other => panic!("{other:?}"),
        }
    }

    /// Encoding standard section 4.2: a label names its encoding without
    /// case and with white space around it; `replacement` and what is no
    /// label name none.
    #[test]
    fn labels_name_the_standards_encodings() {
        let mut decoders = Decoders::default();
        let mut label = |label: &str| decoders.operate(0, flag::LABEL, 0, Some(label), None);
        assert_eq!(text(label(" UTF8 ")), "0\nutf-8");
        assert_eq!(text(label("latin1")), "21\nwindows-1252");
        assert_eq!(text(label("sjis")), "34\nshift_jis");
        assert_eq!(text(label("utf-16")), "37\nutf-16le");
        assert_eq!(label("replacement"), Reply::Undefined);
        assert_eq!(label("iso-2022-kr"), Reply::Undefined);
        assert_eq!(label("no-such-encoding"), Reply::Undefined);
        // Every encoding's own name is a label of it, at its own number.
        for (index, encoding) in ENCODINGS.iter().enumerate() {
            assert_eq!(text(label(encoding.name())), format!("{index}\n{}", encoding.name().to_ascii_lowercase()));
        }
    }

    /// Encoding standard section 7.1: a byte order mark is removed unless
    /// it is to be ignored; malformed input is U+FFFD, or a `TypeError`
    /// when fatal.
    #[test]
    fn a_whole_decoding_follows_its_flags() {
        let mut decoders = Decoders::default();
        let mut whole = |index: i32, flags: i32, bytes: &[u8]| decoders.operate(index, flags, 0, None, Some(bytes));
        assert_eq!(text(whole(0, 0, b"\xEF\xBB\xBFa\xC3\xA9")), "a\u{e9}");
        assert_eq!(text(whole(0, flag::IGNORE_BOM, b"\xEF\xBB\xBFa")), "\u{feff}a");
        assert_eq!(text(whole(0, 0, b"a\xFFb")), "a\u{fffd}b");
        assert!(matches!(whole(0, flag::FATAL, b"a\xFFb"), Reply::Type(_)));
        assert_eq!(text(whole(21, 0, b"\x80\xE9")), "\u{20ac}\u{e9}");
        assert_eq!(text(whole(34, 0, b"\x83\x65")), "\u{30c6}");
        assert!(matches!(whole(99, 0, b"a"), Reply::Type(_)));
        assert!(matches!(whole(0, 64, b"a"), Reply::Type(_)));
    }

    /// A stream of an encoding with state: a character cut between two
    /// pieces is decoded whole, the decoder is freed at the stream's end,
    /// and a context holds at most [`MAX_DECODERS`].
    #[test]
    fn a_stream_keeps_its_decoder_until_it_ends() {
        let mut decoders = Decoders::default();
        let Reply::Int(id) = decoders.operate(34, flag::OPEN, 0, None, None) else { panic!() };
        assert_eq!(text(decoders.operate(34, flag::STREAM, id, None, Some(b"\x83"))), "");
        assert_eq!(text(decoders.operate(34, 0, id, None, Some(b"\x65"))), "\u{30c6}");
        assert!(matches!(decoders.operate(34, 0, id, None, Some(b"a")), Reply::Type(_)), "the decoder was freed at its stream's end");
        for _ in 0..MAX_DECODERS {
            assert!(matches!(decoders.operate(34, flag::OPEN, 0, None, None), Reply::Int(_)));
        }
        assert!(matches!(decoders.operate(34, flag::OPEN, 0, None, None), Reply::Range(text) if text.starts_with("MAX_DECODERS")));
    }
}
