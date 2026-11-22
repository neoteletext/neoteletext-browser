//! A script's manifest (script-api.md section 3.1): the declaration at
//! the head of the file, which is read and never run. It is the first
//! thing in the file, a block comment that opens with exactly `/*! nt`
//! and holds one JSON object, at most [`MAX_MANIFEST_BYTES`] from the
//! file's start to the closing `*/`. An unknown member, a member given
//! twice, a wrong type or a value out of its range refuses the script
//! whole; so does a file with no manifest.
//!
//! In the browser the decoder reads the manifest (work package F4). Until
//! that reader exists this is the runner's own, written to the same
//! section; when F4's lands the runner calls it and this file goes, so
//! that one reader decides what both accept.

use neoteletext_script::API_VERSION;
use protocol::script::{MAX_KEY_ASKS, MAX_KEY_NAME_BYTES, MAX_LANGUAGE_BYTES, MAX_TICK_DIVISOR, Renderer};

use crate::json::{self, Value};
use crate::limits::MAX_MANIFEST_BYTES;

/// What a manifest opens with, exactly, at the file's first byte.
pub const OPENING: &[u8] = b"/*! nt";
/// What ends the comment.
const CLOSING: &[u8] = b"*/";
/// The most JSON values a manifest holds: its nine members, their values
/// and every key name.
const MAX_MANIFEST_VALUES: usize = 32 + MAX_KEY_ASKS;

/// What the manifest's `pointer` claims.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pointer {
    /// Nothing.
    #[default]
    None,
    /// Presses, releases, enters, leaves and cancels.
    Press,
    /// Those and moves.
    Move,
}

/// A manifest, read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The API version the script was written for.
    pub api: u16,
    /// The renderer it draws on, if it says.
    pub renderer: Option<Renderer>,
    /// The keys it claims.
    pub keys: Vec<String>,
    /// Whether it claims typed text.
    pub text: bool,
    /// What it claims of the pointer.
    pub pointer: Pointer,
    /// Whether it claims the wheel.
    pub wheel: bool,
    /// One tick event every this many ticks; 0 for none.
    pub tick: u8,
    /// The language its text is encoded for, if it says.
    pub language: Option<String>,
    /// Whether it uses the network.
    pub network: bool,
}

/// The JSON text of the manifest at the head of `source`.
fn text_of(source: &[u8]) -> Result<&[u8], String> {
    let rest = source.strip_prefix(OPENING).ok_or("the script does not begin with its manifest, a comment that opens with exactly `/*! nt`")?;
    let head = rest.get(..MAX_MANIFEST_BYTES.saturating_sub(OPENING.len()).min(rest.len())).unwrap_or(rest);
    let end = head
        .windows(CLOSING.len())
        .position(|window| window == CLOSING)
        .ok_or_else(|| format!("MAX_MANIFEST_BYTES: the manifest's comment does not close within {MAX_MANIFEST_BYTES} bytes of the file's start"))?;
    Ok(head.get(..end).unwrap_or_default())
}

fn flag(value: &Value, name: &str) -> Result<bool, String> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        _ => Err(format!("the manifest's `{name}` is not true or false")),
    }
}

/// Reads the manifest at the head of `source`.
///
/// # Errors
/// Why the script is refused, naming the member.
pub fn read(source: &[u8]) -> Result<Manifest, String> {
    let document = json::parse(text_of(source)?, MAX_MANIFEST_VALUES).map_err(|what| format!("the manifest is not one JSON object: {what}"))?;
    let members = document.members().ok_or("the manifest is not a JSON object")?;
    let mut manifest = Manifest::default();
    let mut has_api = false;
    for (name, value) in members {
        match name.as_str() {
            "api" => {
                manifest.api = value
                    .number()
                    .and_then(|number| u16::try_from(number).ok())
                    .filter(|api| (1..=API_VERSION).contains(api))
                    .ok_or_else(|| format!("the manifest's `api` is not a version from 1 to {API_VERSION}"))?;
                has_api = true;
            }
            "renderer" => {
                manifest.renderer = Some(match value.text() {
                    Some("teletext") => Renderer::Teletext,
                    Some("regis") => Renderer::Regis,
                    Some("naplps") => Renderer::Naplps,
                    _ => return Err("the manifest's `renderer` is not \"teletext\", \"regis\" or \"naplps\"".into()),
                });
            }
            "keys" => {
                let Value::List(names) = value else { return Err("the manifest's `keys` is not an array".into()) };
                if names.len() > MAX_KEY_ASKS {
                    return Err(format!("MAX_KEY_ASKS: the manifest's `keys` names more than {MAX_KEY_ASKS} keys"));
                }
                for name in names {
                    let key = name
                        .text()
                        .filter(|key| !key.is_empty() && key.len() <= MAX_KEY_NAME_BYTES && !key.chars().any(char::is_control))
                        .ok_or_else(|| format!("MAX_KEY_NAME_BYTES: a name in the manifest's `keys` is not 1 to {MAX_KEY_NAME_BYTES} bytes of plain text"))?;
                    if !manifest.keys.iter().any(|held| held == key) {
                        manifest.keys.push(key.to_owned());
                    }
                }
            }
            "text" => manifest.text = flag(value, name)?,
            "wheel" => manifest.wheel = flag(value, name)?,
            "network" => manifest.network = flag(value, name)?,
            "pointer" => {
                manifest.pointer = match value.text() {
                    Some("none") => Pointer::None,
                    Some("press") => Pointer::Press,
                    Some("move") => Pointer::Move,
                    _ => return Err("the manifest's `pointer` is not \"none\", \"press\" or \"move\"".into()),
                };
            }
            "tick" => {
                manifest.tick = value
                    .number()
                    .and_then(|number| u8::try_from(number).ok())
                    .filter(|tick| *tick <= MAX_TICK_DIVISOR)
                    .ok_or_else(|| format!("MAX_TICK_DIVISOR: the manifest's `tick` is not a whole number from 0 to {MAX_TICK_DIVISOR}"))?;
            }
            "language" => {
                let tag = value
                    .text()
                    .filter(|tag| !tag.is_empty() && tag.len() <= MAX_LANGUAGE_BYTES && tag.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
                    .ok_or_else(|| format!("the manifest's `language` is not a language tag of at most {MAX_LANGUAGE_BYTES} bytes"))?;
                manifest.language = Some(tag.to_owned());
            }
            other => return Err(format!("the manifest has a member this version does not know: `{other}`")),
        }
    }
    if has_api { Ok(manifest) } else { Err("the manifest does not say its `api` version".into()) }
}
