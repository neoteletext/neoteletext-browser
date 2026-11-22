//! JSON as the runner reads and writes it: a script's manifest, the
//! budget report the script library gives, and `expect.json`. The reading
//! is the engine's own bounded pull reader (`neoteletext_web::json`,
//! RFC 8259), which checks the whole grammar and limits nesting; this
//! module only gathers its tokens into a small tree, within a count of
//! values the caller names, and refuses what none of those documents
//! holds: a member given twice, and a number that is not whole.

use neoteletext_web::json::{JsonReader, JsonStr, Token, integer};

/// A JSON value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A whole number.
    Number(i64),
    /// A string.
    Text(String),
    /// An array.
    List(Vec<Self>),
    /// An object: its members in the order written, no name twice.
    Object(Vec<(String, Self)>),
}

impl Value {
    /// The members, of an object.
    #[must_use]
    pub fn members(&self) -> Option<&[(String, Self)]> {
        match self {
            Self::Object(members) => Some(members),
            _ => None,
        }
    }

    /// The member named `name`, of an object.
    #[must_use]
    pub fn member(&self, name: &str) -> Option<&Self> {
        self.members()?.iter().find(|(held, _)| held == name).map(|(_, value)| value)
    }

    /// The number, of a number.
    #[must_use]
    pub const fn number(&self) -> Option<i64> {
        match self {
            Self::Number(number) => Some(*number),
            _ => None,
        }
    }

    /// The text, of a string.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }
}

fn decoded(text: JsonStr<'_>) -> Result<String, &'static str> {
    let mut buffer = vec![0_u8; text.raw_len()];
    let length = text.decode_into(&mut buffer).map_err(|_| "string")?;
    buffer.truncate(length);
    String::from_utf8(buffer).map_err(|_| "string")
}

/// The value that begins with `first`. The engine's reader bounds the
/// nesting, and so this function's own.
fn value(reader: &mut JsonReader<'_>, first: Token<'_>, room: &mut usize) -> Result<Value, &'static str> {
    *room = room.checked_sub(1).ok_or("too many values")?;
    match first {
        Token::Null => Ok(Value::Null),
        Token::Bool(flag) => Ok(Value::Bool(flag)),
        Token::Number(number) => integer(number).map(Value::Number).ok_or("a number that is not a whole number"),
        Token::Str(text) => decoded(text).map(Value::Text),
        Token::BeginArray => {
            let mut items = Vec::new();
            loop {
                match reader.next_token().map_err(|_| "JSON")? {
                    Some(Token::EndArray) => return Ok(Value::List(items)),
                    Some(token) => items.push(value(reader, token, room)?),
                    None => return Err("JSON"),
                }
            }
        }
        Token::BeginObject => {
            let mut members: Vec<(String, Value)> = Vec::new();
            loop {
                match reader.next_token().map_err(|_| "JSON")? {
                    Some(Token::EndObject) => return Ok(Value::Object(members)),
                    Some(Token::Key(key)) => {
                        let name = decoded(key)?;
                        if members.iter().any(|(held, _)| *held == name) {
                            return Err("a member given twice");
                        }
                        let token = reader.next_token().map_err(|_| "JSON")?.ok_or("JSON")?;
                        members.push((name, value(reader, token, room)?));
                    }
                    _ => return Err("JSON"),
                }
            }
        }
        Token::EndArray | Token::EndObject | Token::Key(_) => Err("JSON"),
    }
}

/// Reads one JSON document of at most `most` values.
///
/// # Errors
/// What was wrong, in a word or two: not JSON, a member twice, a number
/// that is not whole, or more values than `most`.
pub fn parse(bytes: &[u8], most: usize) -> Result<Value, &'static str> {
    let mut reader = JsonReader::new(bytes).map_err(|_| "text that is not UTF-8")?;
    let first = reader.next_token().map_err(|_| "JSON")?.ok_or("JSON")?;
    let mut room = most;
    let read = value(&mut reader, first, &mut room)?;
    match reader.next_token() {
        Ok(None) => Ok(read),
        _ => Err("JSON"),
    }
}

/// `text` as a JSON string, quoted: the two characters JSON must escape
/// and every control character escaped, everything else as it is.
#[must_use]
pub fn quote(text: &str) -> String {
    use core::fmt::Write as _;
    let mut out = String::with_capacity(text.len().saturating_add(2));
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            control if control.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
