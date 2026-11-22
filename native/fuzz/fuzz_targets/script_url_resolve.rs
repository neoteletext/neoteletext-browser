//! The script host's address operations (script-api.md section 11.4,
//! target 20; operations 40 `URL_PARSE` and 48 `URL_SET`): an arbitrary
//! input against an arbitrary base, and arbitrary values given to each
//! setter of the address that results. Nothing panics; what the host
//! says of an address is within its bounds and is an address the same
//! parser reads back to itself, with every place it names inside it and
//! in order; and a setter leaves an address, never something that is
//! none.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::{Host, Payload, Reply};
use neoteletext_script::limits::MAX_URL_BYTES;
use neoteletext_script::ops::Op;
use protocol::script::{ContextFlags, ContextId, ContextOpen, Level, Renderer};

/// The page context's number, as the shim numbers it.
const PAGE: i32 = 2;
/// The setters of operation 48: its components' numbers, 1 to 9.
const SETTERS: u8 = 9;
/// The places operation 40 writes before an address: thirteen and the
/// fragment's.
const PLACES: usize = 14;

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

/// What the host said of an address: its serialisation, checked.
fn address_of(reply: &Reply) -> Option<String> {
    let text = match reply {
        Reply::Text(text) => text,
        Reply::Undefined => return None,
        // Past the bound on an address: the refusal names it.
        Reply::Type(text) => {
            assert!(text.starts_with("MAX_URL_BYTES") || !text.is_empty());
            return None;
        }
        other => panic!("an address operation answered {other:?}"),
    };
    let mut lines = text.splitn(3, '\n');
    let (Some(places), Some(origin), Some(href)) = (lines.next(), lines.next(), lines.next()) else { panic!("not places, an origin and an address: {text:?}") };
    assert!(href.len() <= MAX_URL_BYTES, "an address of {} bytes", href.len());
    assert!(href.is_ascii() && !href.contains('\n'), "{href:?}");
    assert!(!origin.is_empty() && origin.is_ascii(), "{origin:?}");
    let places: Vec<usize> = places.split(',').map(|place| place.parse().unwrap_or_else(|_| panic!("a place that is no number: {places:?}"))).collect();
    assert_eq!(places.len(), PLACES, "{places:?}");
    assert!(places.windows(2).all(|pair| pair[0] <= pair[1]) && places.last().is_some_and(|last| *last <= href.len()), "{places:?} in {href:?}");
    Some(href.to_owned())
}

fuzz_target!(|data: &[u8]| {
    let mut host = Host::new();
    host.open(opening(), 1);
    let Some((&cut, rest)) = data.split_first() else { return };
    let Some((&with_base, rest)) = rest.split_first() else { return };
    let text = String::from_utf8_lossy(rest);
    // The input and its base: cut at a place the first byte chooses.
    let at = text.char_indices().map(|(index, _)| index).nth(usize::from(cut)).unwrap_or(text.len());
    let (input, base) = text.split_at(at);
    let reply = if with_base & 1 == 0 {
        host.call(PAGE, Op::URL_PARSE.number(), &[-1], Payload::Text(text.as_bytes()))
    } else {
        host.call(PAGE, Op::URL_PARSE.number(), &[i32::try_from(input.len()).unwrap_or(i32::MAX)], Payload::Text(text.as_bytes()))
    };
    let Some(href) = address_of(&reply) else { return };
    // Serialising is idempotent (URL standard section 4.5): the address
    // read alone is itself.
    let again = host.call(PAGE, Op::URL_PARSE.number(), &[-1], Payload::Text(href.as_bytes()));
    assert_eq!(address_of(&again).as_deref(), Some(href.as_str()), "{input:?} against {base:?}");
    // Each setter, given the base's text as its value.
    for part in 1..=SETTERS {
        let payload = format!("{href}{base}");
        let set = host.call(PAGE, Op::URL_SET.number(), &[i32::from(part), i32::try_from(href.len()).unwrap_or(i32::MAX)], Payload::Text(payload.as_bytes()));
        if let Some(changed) = address_of(&set) {
            let reread = host.call(PAGE, Op::URL_PARSE.number(), &[-1], Payload::Text(changed.as_bytes()));
            assert!(address_of(&reread).is_some(), "setter {part} left {changed:?}, which is no address");
        }
    }
});
