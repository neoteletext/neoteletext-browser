//! A script's manifest (the script API's specification section 3.1;
//! DEC-7287 item 4, DEC-7282 item 4): the declaration at the head of a
//! script file of what the script claims, read here, in the process that
//! parses outside input, before any of the script's code runs anywhere.
//!
//! The manifest is the first thing in the file: a block comment that
//! opens with exactly `/*! nt` and holds one JSON object, at most
//! [`MAX_MANIFEST_BYTES`] from the file's start to the closing `*/`. It
//! is read, never run. The reader is strict and bounded: an unknown
//! member, a member given twice, a wrong type or a value out of its range
//! refuses the script whole, and so does a file with no manifest. Nothing
//! is taken from the script but what this returns.

use neoteletext_core::charset::language::language;
use protocol::decoder::script::PointerClaim;
use protocol::script::{MAX_KEY_ASKS, MAX_KEY_NAME_BYTES, MAX_TICK_DIVISOR, Renderer, is_language_tag};

use super::limits::{FIRST_API, MAX_MANIFEST_BYTES};

/// What opens a manifest, at the file's first byte.
const OPENING: &[u8] = b"/*! nt";
/// What closes it.
const CLOSING: &[u8] = b"*/";
/// The hexadecimal digits of a `\u` escape.
const ESCAPE_DIGITS: usize = 4;
/// The most decimal digits a whole number of the manifest has: more than
/// any member's range needs, and few enough never to overflow.
const MAX_NUMBER_DIGITS: usize = 9;

/// What a script claims, as its manifest says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The API version the script was written for, at least
    /// [`FIRST_API`]; the script host says the highest it has.
    pub api: u16,
    /// The renderer a JavaScript page draws on, when it names one.
    pub renderer: Option<Renderer>,
    /// The keys claimed, by name, each once, in the manifest's order.
    pub keys: Vec<String>,
    /// Typed text is claimed.
    pub text: bool,
    /// What of the pointer is claimed.
    pub pointer: PointerClaim,
    /// The wheel is claimed.
    pub wheel: bool,
    /// A tick event every this many ticks; 0 for none.
    pub tick: u8,
    /// The language text is encoded for, when it names one: a tag of the
    /// engine's list.
    pub language: Option<String>,
    /// The script uses the network.
    pub network: bool,
}

/// Why a script is refused for its manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// The file does not begin with a manifest.
    Missing,
    /// The manifest does not end within [`MAX_MANIFEST_BYTES`].
    TooLong,
    /// The manifest is not one JSON object as this reader takes it.
    Syntax(&'static str),
    /// A member no version of the API knows.
    UnknownMember,
    /// A member given twice.
    Duplicate(&'static str),
    /// A member's value is of the wrong type.
    Type(&'static str),
    /// A member's value is outside its range.
    Range(&'static str),
    /// The member `api` is absent.
    NoApi,
}

impl core::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missing => write!(formatter, "the script does not begin with a manifest, a comment that opens with /*! nt"),
            Self::TooLong => write!(formatter, "MAX_MANIFEST_BYTES: the manifest does not end within {MAX_MANIFEST_BYTES} bytes of the file's start"),
            Self::Syntax(what) => write!(formatter, "the manifest is not one JSON object: {what}"),
            Self::UnknownMember => write!(formatter, "the manifest has a member the API does not know"),
            Self::Duplicate(member) => write!(formatter, "the manifest gives the member {member} twice"),
            Self::Type(member) => write!(formatter, "the manifest's member {member} is of the wrong type"),
            Self::Range(member) => write!(formatter, "the manifest's member {member} is outside its range"),
            Self::NoApi => write!(formatter, "the manifest does not say which version of the API the script was written for"),
        }
    }
}

impl core::error::Error for ManifestError {}

/// A value of the manifest's JSON: the only forms a member takes.
enum Value {
    Whole(u32),
    Text(String),
    Flag(bool),
    Texts(Vec<String>),
}

/// Where the reader is in the manifest's JSON.
struct Cursor<'a> {
    rest: &'a [u8],
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.rest.first().copied()
    }

    fn next(&mut self) -> Option<u8> {
        let (&first, rest) = self.rest.split_first()?;
        self.rest = rest;
        Some(first)
    }

    /// Passes JSON's white space (RFC 8259 section 2).
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.next();
        }
    }

    fn expect(&mut self, byte: u8, what: &'static str) -> Result<(), ManifestError> {
        self.space();
        if self.next() == Some(byte) { Ok(()) } else { Err(ManifestError::Syntax(what)) }
    }

    fn word(&mut self, word: &[u8]) -> Result<(), ManifestError> {
        match self.rest.strip_prefix(word) {
            Some(rest) => {
                self.rest = rest;
                Ok(())
            }
            None => Err(ManifestError::Syntax("a value that is no number, text, true, false or list")),
        }
    }

    /// The four hexadecimal digits of a `\u` escape.
    fn escape_unit(&mut self) -> Result<u32, ManifestError> {
        let mut unit = 0_u32;
        for _ in 0..ESCAPE_DIGITS {
            let digit =
                self.next().and_then(|byte| char::from(byte).to_digit(16)).ok_or(ManifestError::Syntax("a \\u escape without four hexadecimal digits"))?;
            unit = unit.saturating_mul(16).saturating_add(digit);
        }
        Ok(unit)
    }

    /// A JSON string (RFC 8259 section 7): its opening quote is next.
    /// Its text is at most the manifest's own length, which is bounded.
    fn text(&mut self) -> Result<String, ManifestError> {
        const BAD: ManifestError = ManifestError::Syntax("a text that is not a JSON string");
        if self.next() != Some(b'"') {
            return Err(BAD);
        }
        let mut bytes = Vec::new();
        loop {
            match self.next().ok_or(BAD)? {
                b'"' => break,
                b'\\' => {
                    let character = match self.next().ok_or(BAD)? {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.escaped()?,
                        _ => return Err(BAD),
                    };
                    bytes.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                }
                // RFC 8259: a control character is written only escaped.
                byte if byte < b' ' => return Err(BAD),
                byte => bytes.push(byte),
            }
        }
        String::from_utf8(bytes).map_err(|_| ManifestError::Syntax("a text that is not UTF-8"))
    }

    /// The character of a `\u` escape whose `\u` was read: one unit, or a
    /// surrogate pair. A lone surrogate is no character and is refused.
    fn escaped(&mut self) -> Result<char, ManifestError> {
        const LONE: ManifestError = ManifestError::Syntax("a \\u escape that is half a surrogate pair");
        const HIGH: core::ops::RangeInclusive<u32> = 0xD800..=0xDBFF;
        const LOW: core::ops::RangeInclusive<u32> = 0xDC00..=0xDFFF;
        /// The first scalar value a surrogate pair gives, and the bits
        /// each half carries (the Unicode Standard, section 3.9, UTF-16).
        const PAIR_BASE: u32 = 0x1_0000;
        const HALF_BITS: u32 = 10;
        let unit = self.escape_unit()?;
        if !HIGH.contains(&unit) {
            return char::from_u32(unit).ok_or(LONE);
        }
        if self.next() != Some(b'\\') || self.next() != Some(b'u') {
            return Err(LONE);
        }
        let low = self.escape_unit()?;
        if !LOW.contains(&low) {
            return Err(LONE);
        }
        let scalar = PAIR_BASE.saturating_add(unit.saturating_sub(*HIGH.start()) << HALF_BITS).saturating_add(low.saturating_sub(*LOW.start()));
        char::from_u32(scalar).ok_or(LONE)
    }

    /// A whole number that is not negative (RFC 8259 section 6, without a
    /// fraction, an exponent or a sign: no member takes any).
    fn whole(&mut self) -> Result<u32, ManifestError> {
        const BAD: ManifestError = ManifestError::Syntax("a number that is not a whole number without sign, fraction or exponent");
        let digits = self.rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
        let (number, rest) = self.rest.split_at_checked(digits).ok_or(BAD)?;
        // JSON has no leading zero, and a number ends at its last digit.
        if digits == 0 || digits > MAX_NUMBER_DIGITS || (digits > 1 && number.first() == Some(&b'0')) || matches!(rest.first(), Some(b'.' | b'e' | b'E')) {
            return Err(BAD);
        }
        self.rest = rest;
        Ok(number.iter().fold(0_u32, |value, digit| value.saturating_mul(10).saturating_add(u32::from(digit.saturating_sub(b'0')))))
    }

    /// A list of texts, whose `[` is next: at most `most`.
    fn texts(&mut self, most: usize, member: &'static str) -> Result<Vec<String>, ManifestError> {
        self.next();
        let mut texts = Vec::new();
        self.space();
        if self.peek() == Some(b']') {
            self.next();
            return Ok(texts);
        }
        loop {
            self.space();
            if self.peek() != Some(b'"') {
                return Err(ManifestError::Type(member));
            }
            if texts.len() >= most {
                return Err(ManifestError::Range(member));
            }
            texts.push(self.text()?);
            self.space();
            match self.next() {
                Some(b',') => {}
                Some(b']') => return Ok(texts),
                _ => return Err(ManifestError::Syntax("a list that does not end")),
            }
        }
    }

    /// The value of `member`: one of the forms a member takes.
    fn value(&mut self, member: &'static str) -> Result<Value, ManifestError> {
        self.space();
        match self.peek() {
            Some(b'"') => self.text().map(Value::Text),
            Some(b't') => self.word(b"true").map(|()| Value::Flag(true)),
            Some(b'f') => self.word(b"false").map(|()| Value::Flag(false)),
            Some(b'[') => self.texts(MAX_KEY_ASKS, member).map(Value::Texts),
            Some(b'0'..=b'9') => self.whole().map(Value::Whole),
            // A negative number, null and an object are JSON, and no
            // member's type.
            Some(b'-' | b'n' | b'{') => Err(ManifestError::Type(member)),
            _ => Err(ManifestError::Syntax("a member without a value")),
        }
    }
}

/// The members, by the names the specification gives them.
const MEMBERS: [&str; 9] = ["api", "renderer", "keys", "text", "pointer", "wheel", "tick", "language", "network"];

/// What has been read of a manifest's members.
#[derive(Default)]
struct Members {
    api: Option<u16>,
    renderer: Option<Renderer>,
    keys: Option<Vec<String>>,
    text: Option<bool>,
    pointer: Option<PointerClaim>,
    wheel: Option<bool>,
    tick: Option<u8>,
    language: Option<String>,
    network: Option<bool>,
}

fn flag(value: &Value, member: &'static str) -> Result<bool, ManifestError> {
    if let Value::Flag(flag) = value { Ok(*flag) } else { Err(ManifestError::Type(member)) }
}

fn text(value: Value, member: &'static str) -> Result<String, ManifestError> {
    if let Value::Text(text) = value { Ok(text) } else { Err(ManifestError::Type(member)) }
}

fn whole(value: &Value, member: &'static str) -> Result<u32, ManifestError> {
    if let Value::Whole(whole) = value { Ok(*whole) } else { Err(ManifestError::Type(member)) }
}

/// Keeps `value` as `slot`'s, which must be empty.
fn keep<T>(slot: &mut Option<T>, value: T, member: &'static str) -> Result<(), ManifestError> {
    if slot.replace(value).is_some() { Err(ManifestError::Duplicate(member)) } else { Ok(()) }
}

impl Members {
    /// Takes the value of the member `name`.
    fn take(&mut self, name: &'static str, value: Value) -> Result<(), ManifestError> {
        let range = ManifestError::Range(name);
        match name {
            "api" => {
                let api = u16::try_from(whole(&value, name)?).ok().filter(|api| *api >= FIRST_API).ok_or(range)?;
                keep(&mut self.api, api, name)
            }
            "renderer" => {
                let renderer = match text(value, name)?.as_str() {
                    "teletext" => Renderer::Teletext,
                    "regis" => Renderer::Regis,
                    "naplps" => Renderer::Naplps,
                    _ => return Err(range),
                };
                keep(&mut self.renderer, renderer, name)
            }
            "keys" => {
                let Value::Texts(names) = value else { return Err(ManifestError::Type(name)) };
                let mut keys: Vec<String> = Vec::new();
                for key in names {
                    // A key's name is plain text of the link's bound
                    // (section 10.6); a name given twice is one claim.
                    if key.is_empty() || key.len() > MAX_KEY_NAME_BYTES || key.chars().any(char::is_control) {
                        return Err(range);
                    }
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
                keep(&mut self.keys, keys, name)
            }
            "text" => keep(&mut self.text, flag(&value, name)?, name),
            "pointer" => {
                let pointer = match text(value, name)?.as_str() {
                    "none" => PointerClaim::None,
                    "press" => PointerClaim::Press,
                    "move" => PointerClaim::Move,
                    _ => return Err(range),
                };
                keep(&mut self.pointer, pointer, name)
            }
            "wheel" => keep(&mut self.wheel, flag(&value, name)?, name),
            "tick" => {
                let tick = u8::try_from(whole(&value, name)?).ok().filter(|tick| *tick <= MAX_TICK_DIVISOR).ok_or(range)?;
                keep(&mut self.tick, tick, name)
            }
            "language" => {
                // Section 5.6: a tag of the engine's list, and no other.
                let tag = text(value, name)?;
                // The tag's form by the link's own rule, the one definition
                // of it, before the engine's list is asked: the list's
                // lookup takes a tag by its beginning, so alone it would
                // let through text the context's opening then refuses.
                if !is_language_tag(&tag) || language(&tag).is_none() {
                    return Err(range);
                }
                keep(&mut self.language, tag, name)
            }
            _ => keep(&mut self.network, flag(&value, name)?, name),
        }
    }
}

/// The manifest's JSON within `source`, a script file: what lies between
/// the opening at the file's first byte and the first `*/` after it.
fn body(source: &[u8]) -> Result<&[u8], ManifestError> {
    let after = source.strip_prefix(OPENING).ok_or(ManifestError::Missing)?;
    // Only the head of the file is searched: a manifest that does not
    // close within its bound is refused unread.
    let head = after.get(..MAX_MANIFEST_BYTES.saturating_sub(OPENING.len()).min(after.len())).unwrap_or(after);
    let end = head.windows(CLOSING.len()).position(|window| window == CLOSING).ok_or(ManifestError::TooLong)?;
    head.get(..end).ok_or(ManifestError::TooLong)
}

/// Reads the manifest of the script file `source`.
///
/// # Errors
/// Why the script is refused: see [`ManifestError`].
pub fn read(source: &[u8]) -> Result<Manifest, ManifestError> {
    let mut cursor = Cursor { rest: body(source)? };
    cursor.expect(b'{', "it does not open with {")?;
    let mut members = Members::default();
    cursor.space();
    if cursor.peek() == Some(b'}') {
        cursor.next();
    } else {
        loop {
            cursor.space();
            let name = cursor.text()?;
            let name = MEMBERS.iter().copied().find(|member| *member == name).ok_or(ManifestError::UnknownMember)?;
            cursor.expect(b':', "a member without a colon")?;
            let value = cursor.value(name)?;
            members.take(name, value)?;
            cursor.space();
            match cursor.next() {
                Some(b',') => {}
                Some(b'}') => break,
                _ => return Err(ManifestError::Syntax("the object does not end")),
            }
        }
    }
    cursor.space();
    if !cursor.rest.is_empty() {
        return Err(ManifestError::Syntax("something follows the object"));
    }
    Ok(Manifest {
        api: members.api.ok_or(ManifestError::NoApi)?,
        renderer: members.renderer,
        keys: members.keys.unwrap_or_default(),
        text: members.text.unwrap_or(false),
        pointer: members.pointer.unwrap_or_default(),
        wheel: members.wheel.unwrap_or(false),
        tick: members.tick.unwrap_or(0),
        language: members.language,
        network: members.network.unwrap_or(false),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::panic, reason = "a test")]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Result<Manifest, ManifestError> {
        read(format!("/*! nt {json} */\nnt.log('x');").as_bytes())
    }

    /// Section 3.1's table: every member, read as written, and each
    /// member's default when it is absent.
    #[test]
    fn every_member_is_read_and_an_absent_one_has_its_default() {
        let full = r#"{ "api": 1, "renderer": "regis", "keys": ["1", "ArrowLeft", "1", "é"], "text": true,
            "pointer": "move", "wheel": true, "tick": 60, "language": "de", "network": true }"#;
        assert_eq!(
            manifest(full).unwrap(),
            Manifest {
                api: 1,
                renderer: Some(Renderer::Regis),
                keys: vec!["1".into(), "ArrowLeft".into(), "é".into()],
                text: true,
                pointer: PointerClaim::Move,
                wheel: true,
                tick: 60,
                language: Some("de".into()),
                network: true,
            }
        );
        assert_eq!(
            manifest(r#"{"api":3}"#).unwrap(),
            Manifest { api: 3, renderer: None, keys: vec![], text: false, pointer: PointerClaim::None, wheel: false, tick: 0, language: None, network: false }
        );
        assert_eq!(manifest(r#"{"api":1,"pointer":"press","keys":[]}"#).unwrap().pointer, PointerClaim::Press);
        // The comment's close is the first one: what follows is the script.
        assert_eq!(read(b"/*! nt{\"api\":1}*/ /* a later */").unwrap().api, 1);
    }

    /// Section 3.1: "an unknown member, a duplicate member, a wrong type
    /// or a value out of range refuses the script whole. A file with no
    /// manifest is refused."
    #[test]
    fn an_unknown_duplicate_mistyped_or_out_of_range_member_refuses_the_script() {
        use ManifestError::{Duplicate, Missing, NoApi, Range, Syntax, Type, UnknownMember};
        let too_many: String = (0..=MAX_KEY_ASKS).map(|key| format!("\"k{key}\"")).collect::<Vec<_>>().join(",");
        let long_key = "k".repeat(MAX_KEY_NAME_BYTES + 1);
        let cases: Vec<(String, ManifestError)> = vec![
            (r#"{"api":1,"any":true}"#.into(), UnknownMember),
            (r#"{"api":1,"API":1}"#.into(), UnknownMember),
            (r#"{"api":1,"api":1}"#.into(), Duplicate("api")),
            (r#"{"api":1,"text":true,"text":true}"#.into(), Duplicate("text")),
            (r"{}".into(), NoApi),
            (r#"{"text":true}"#.into(), NoApi),
            (r#"{"api":"1"}"#.into(), Type("api")),
            (r#"{"api":null}"#.into(), Type("api")),
            (r#"{"api":-1}"#.into(), Type("api")),
            (r#"{"api":{}}"#.into(), Type("api")),
            (r#"{"api":0}"#.into(), Range("api")),
            (r#"{"api":65536}"#.into(), Range("api")),
            (r#"{"api":1,"renderer":"sixel"}"#.into(), Range("renderer")),
            (r#"{"api":1,"renderer":1}"#.into(), Type("renderer")),
            (r#"{"api":1,"keys":"1"}"#.into(), Type("keys")),
            (r#"{"api":1,"keys":[1]}"#.into(), Type("keys")),
            (r#"{"api":1,"keys":[""]}"#.into(), Range("keys")),
            (format!(r#"{{"api":1,"keys":["{long_key}"]}}"#), Range("keys")),
            (format!(r#"{{"api":1,"keys":[{too_many}]}}"#), Range("keys")),
            (r#"{"api":1,"keys":["a\u0007"]}"#.into(), Range("keys")),
            (r#"{"api":1,"text":1}"#.into(), Type("text")),
            (r#"{"api":1,"pointer":"any"}"#.into(), Range("pointer")),
            (r#"{"api":1,"pointer":true}"#.into(), Type("pointer")),
            (r#"{"api":1,"wheel":"yes"}"#.into(), Type("wheel")),
            (r#"{"api":1,"tick":61}"#.into(), Range("tick")),
            (r#"{"api":1,"tick":true}"#.into(), Type("tick")),
            (r#"{"api":1,"language":"tlh"}"#.into(), Range("language")),
            (r#"{"api":1,"language":"aaaaaaaaaaaaaaaaa"}"#.into(), Range("language")),
            (r#"{"api":1,"network":0}"#.into(), Type("network")),
        ];
        for (json, error) in cases {
            assert_eq!(manifest(&json), Err(error), "{json}");
        }
        for json in [
            r#"{"api":1,}"#,
            r#"{"api":1} {}"#,
            r#"{"api":01}"#,
            r#"{"api":1.0}"#,
            r#"{"api":1e0}"#,
            r#"{"api":1234567890}"#,
            r#"{"api" 1}"#,
            r"{api:1}",
            r#"["api"]"#,
            r#"{"api":1,"keys":["a" "b"]}"#,
            r#"{"api":1,"keys":["\ud800"]}"#,
            r#"{"api":1,"keys":["\x"]}"#,
            r#"{"api":1,"text":tru}"#,
            "{\"api\":1,\"language\":\"e\nn\"}",
            "",
        ] {
            assert!(matches!(manifest(json), Err(Syntax(_))), "{json}: {:?}", manifest(json));
        }
        assert_eq!(manifest(r#"{"api":1,"keys":["a",]}"#), Err(Type("keys")), "a list that ends in a comma");
        for source in [&b""[..], b"nt.log(1)", b" /*! nt {\"api\":1} */", b"/* nt {\"api\":1} */", b"\xEF\xBB\xBF/*! nt {\"api\":1} */", b"/*! n"] {
            assert_eq!(read(source), Err(Missing), "{}", String::from_utf8_lossy(source));
        }
        assert!(matches!(read(b"/*! nt {\"api\":1,\"keys\":[\"\xFF\"]} */"), Err(Syntax(_))), "not UTF-8");
    }

    /// Section 3.1 and 10.6: the manifest ends within `MAX_MANIFEST_BYTES`
    /// of the file's start, to the byte, and a file that never closes its
    /// comment is refused whatever its length.
    #[test]
    fn the_manifest_ends_within_its_bound_to_the_byte() {
        let json = r#"{"api":1}"#;
        let fits = |padding: usize| format!("/*! nt {json}{}*/", " ".repeat(padding));
        let exact = MAX_MANIFEST_BYTES - fits(0).len();
        assert_eq!(fits(exact).len(), MAX_MANIFEST_BYTES);
        assert_eq!(read(fits(exact).as_bytes()).unwrap().api, 1);
        assert_eq!(read(fits(exact + 1).as_bytes()), Err(ManifestError::TooLong));
        assert_eq!(read(b"/*! nt {\"api\":1}"), Err(ManifestError::TooLong));
        let mut endless = b"/*! nt ".to_vec();
        endless.resize(protocol::script::MAX_SCRIPT_BYTES, b' ');
        assert_eq!(read(&endless), Err(ManifestError::TooLong));
    }
    /// TODO-7441, found by the fuzz target `script_manifest` on 23
    /// November 2026 (its input, kept here): a `language` whose beginning
    /// is a tag of the engine's list and whose rest is not a tag at all
    /// is refused by the manifest's reader, with the manifest's reason,
    /// and never reaches a context's opening, whose own check is the same
    /// rule. A tag of the list with a region is still taken.
    #[test]
    fn a_language_that_only_begins_as_a_tag_is_refused_by_the_manifest() {
        let found = "/*! nt {\"api\":1,\"renderer\":\"regis\",\"keys\":[\"Enter\",\"Arrop\",\"a\",\"F1\"],\"text\":true,\"pointer\":\"move\",\"wheel\":true,\"tick\":60,\"language\":\"de-0\u{74f}\",\"network\":true} */";
        assert_eq!(read(found.as_bytes()), Err(ManifestError::Range("language")));
        for tag in ["de-\u{e9}", "de_AT", "de AT", "de-AT!", "d\u{435}"] {
            assert_eq!(manifest(&format!(r#"{{"api":1,"language":"{tag}"}}"#)), Err(ManifestError::Range("language")), "{tag}");
        }
        for tag in ["de", "de-AT", "en"] {
            let taken = manifest(&format!(r#"{{"api":1,"language":"{tag}"}}"#)).unwrap().language.unwrap();
            assert!(is_language_tag(&taken), "{taken}");
        }
    }
}
