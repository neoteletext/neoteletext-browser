//! The script host's address operations in sequence (script-api.md
//! section 11.4, target 20; operations 40 `URL_PARSE` and 48 `URL_SET`;
//! TODO-7390 step 5 item 1): an arbitrary address, then arbitrary
//! setters with arbitrary values, each on the address the one before
//! gave, as a script's `URL` object is changed again and again. After
//! each, the address the host gives is one its own parser reads, and
//! reading it gives the same eleven fields (URL standard section 4.5:
//! serialising is idempotent). Nothing panics, whatever the `url` crate
//! does with the text (its faults are guarded in the host, TODO-7390).
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::{Host, Payload, Reply};
use neoteletext_script::ops::Op;
use protocol::script::{ContextFlags, ContextId, ContextOpen, Level, Renderer};

/// The page context's number, as the shim numbers it.
const PAGE: i32 = 2;
/// The setters of operation 48: its components' numbers, 1 to 9.
const SETTERS: u8 = 9;
/// The fields operation 40 gives of an address, a line each: the
/// serialisation, the origin and the API's nine components.
const FIELDS: usize = 11;
/// What parts the input: the address, then each setter's number and value.
const SEPARATOR: u8 = 0;
/// The most setters one input gives.
const MAX_SETTERS: usize = 32;

fn opening() -> ContextOpen {
    ContextOpen {
        context: ContextId::Page,
        api: 1,
        renderer: Renderer::Teletext,
        level: Level::TwoFive,
        page: 0x100,
        subcode: 0,
        flags: ContextFlags::default(),
        tick_divisor: 0,
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [1; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: Vec::new(),
        source_bytes: 1,
    }
}

/// The address's fields, when the reply is one.
fn fields(reply: &Reply) -> Option<String> {
    match reply {
        Reply::Text(text) => {
            assert_eq!(text.split('\n').count(), FIELDS, "not an address's fields: {text:?}");
            Some(text.clone())
        }
        Reply::Undefined | Reply::Type(_) | Reply::Range(_) => None,
        other => panic!("an address operation answered {other:?}"),
    }
}

/// Holds the address `text` to its own reading: its first field parses,
/// to the same fields.
fn holds(host: &mut Host, text: &str, after: &str) {
    let href = text.split('\n').next().unwrap_or_default();
    let again = fields(&host.call(PAGE, Op::URL_PARSE.number(), &[-1], Payload::Text(href.as_bytes())));
    assert_eq!(again.as_deref(), Some(text), "after {after}, the address {href:?} does not read back to its own fields");
}

fuzz_target!(|data: &[u8]| {
    let mut host = Host::new();
    host.open(opening(), 1);
    let mut parts = data.split(|&byte| byte == SEPARATOR);
    let Some(address) = parts.next() else { return };
    let address = String::from_utf8_lossy(address);
    let Some(mut text) = fields(&host.call(PAGE, Op::URL_PARSE.number(), &[-1], Payload::Text(address.as_bytes()))) else { return };
    holds(&mut host, &text, "parsing");
    for part in parts.take(MAX_SETTERS) {
        let Some((&setter, value)) = part.split_first() else { continue };
        let setter = setter % SETTERS + 1;
        let value = String::from_utf8_lossy(value);
        let href = text.split('\n').next().unwrap_or_default().to_owned();
        let payload = format!("{href}{value}");
        let reply =
            host.call(PAGE, Op::URL_SET.number(), &[i32::from(setter), i32::try_from(href.len()).unwrap_or(i32::MAX)], Payload::Text(payload.as_bytes()));
        let Some(changed) = fields(&reply) else { continue };
        holds(&mut host, &changed, &format!("setter {setter} with {value:?} on {href:?}"));
        text = changed;
    }
});
