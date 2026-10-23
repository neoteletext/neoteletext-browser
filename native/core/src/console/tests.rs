//! Tests of the developer console's entries.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]

use super::*;
use neoteletext_core::ecc::hamming84_encode;

/// A packet of magazine 1, row `row`, its data `data`, as the engine
/// writes one.
fn packet(row: u8, data: &[u8]) -> Vec<u8> {
    let mut bytes = vec![hamming84_encode(1 | ((row & 1) << 3)), hamming84_encode(row >> 1)];
    bytes.extend(data.iter().copied().chain(std::iter::repeat(b' ')).take(DATA_BYTES));
    bytes
}

/// Requirement 19.9: each packet's address read by the engine, its
/// bytes shown as text and hexadecimal, never anything else.
#[test]
fn packets_are_read_with_the_engine_and_shown_as_text() {
    let mut console = Console::default();
    let mut header = vec![hamming84_encode(0), hamming84_encode(1)];
    header.extend(std::iter::repeat_n(hamming84_encode(0), 6));
    header.extend(b"HEADER".iter().copied().chain(std::iter::repeat(b' ')).take(DATA_BYTES - 8));
    let mut bytes = packet(0, &[]);
    bytes.splice(DATA_START.., header);
    bytes.extend(packet(5, b"\x1b\x07Hello\r\n<script>"));
    // A byte with two bits wrong cannot be corrected.
    let broken = (0..=u8::MAX).find(|&byte| neoteletext_core::ecc::hamming84(byte).is_none()).unwrap();
    bytes.extend([broken; PACKET_BYTES]);
    console.packets(1, 7, true, &bytes);
    console.set_open(true);
    let updates = console.updates(Some(1));
    let texts: Vec<&str> = updates[0].entries.iter().map(|entry| entry.text.as_str()).collect();
    assert!(texts[0].starts_with("pushed M1 R00 page 110"), "{texts:?}");
    assert!(texts[1].starts_with("pushed M1 R05  ..Hello..<script>"), "{texts:?}");
    assert!(texts[1].contains(" 1B 07 48"), "{texts:?}");
    assert!(texts[2].starts_with("pushed bad address"), "{texts:?}");
    assert!(texts.iter().all(|text| text.len() <= MAX_ENTRY && text.chars().all(|character| (' '..='~').contains(&character))));
}

#[test]
fn each_tab_keeps_its_newest_1000() {
    let mut console = Console::default();
    for n in 0..1005 {
        console.log(Some(1), Level::Warning, n, &format!("notice {n}"));
    }
    console.add(1, ConsoleTab::Network, 9, "GET");
    console.set_open(true);
    let updates = console.updates(Some(1));
    let entries: Vec<&ConsoleEntry> = updates.iter().flat_map(|update| &update.entries).collect();
    assert_eq!(entries.len(), KEPT + 1);
    assert_eq!(entries[0].text, "notice 5", "the oldest went");
    assert!(updates.iter().all(|update| update.entries.len() <= MAX_ENTRIES));
    assert_eq!(updates.iter().filter(|update| update.replace).count(), 1, "the first replaces, the rest follow");
    assert!(updates[0].replace);
}

/// Requirement 19.9: sent only while open, the new ones only, and
/// afresh when another channel is shown.
#[test]
fn only_new_entries_go_and_only_while_open() {
    let mut console = Console::default();
    console.add(1, ConsoleTab::Pages, 0, "page 100 arrived");
    assert_eq!(console.updates(Some(1)), [], "closed");
    console.set_open(true);
    assert_eq!(console.updates(Some(1))[0].entries.len(), 1);
    assert!(console.updates(Some(1)).iter().all(|update| update.entries.is_empty()));
    console.add(1, ConsoleTab::Pages, 1, "page 101 arrived");
    console.add(2, ConsoleTab::Pages, 1, "other channel");
    let updates = console.updates(Some(1));
    assert_eq!((updates[0].replace, updates[0].entries[0].text.as_str()), (false, "page 101 arrived"));
    let updates = console.updates(Some(2));
    assert_eq!((updates[0].replace, updates[0].entries.len()), (true, 1), "another channel replaces");
    let updates = console.updates(Some(3));
    assert_eq!((updates[0].replace, updates[0].entries.len()), (true, 0), "an empty channel clears what was shown");
    console.forget(2);
    assert_eq!(console.updates(Some(2))[0].entries, []);
    console.set_open(false);
    assert_eq!(console.updates(Some(1)), []);
}

/// Decision 70: a line of the browser's own log shows in every
/// channel's Log tab, among the channel's own, in order.
#[test]
fn the_browsers_lines_show_in_every_channels_log() {
    let mut console = Console::default();
    console.set_open(true);
    console.log(Some(1), Level::Warning, 1, "fetcher: 404");
    console.log(None, Level::Info, 2, "decoder: confined: sandbox-exec");
    let texts = |updates: Vec<ConsoleUpdate>| updates.into_iter().flat_map(|update| update.entries).map(|entry| (entry.level, entry.text)).collect::<Vec<_>>();
    assert_eq!(texts(console.updates(Some(1))), [(Level::Warning, "fetcher: 404".into()), (Level::Info, "decoder: confined: sandbox-exec".into())]);
    assert_eq!(texts(console.updates(Some(2))), [(Level::Info, "decoder: confined: sandbox-exec".into())], "another channel has the browser's line too");
    console.log(None, Level::Error, 3, "sound helper: ended");
    assert_eq!(texts(console.updates(Some(2))), [(Level::Error, "sound helper: ended".into())], "only what is new");
}

#[test]
fn hidden_characters_are_cleaned() {
    let mut console = Console::default();
    console.log(Some(1), Level::Warning, 0, &format!("a\u{202E}b{}", "x".repeat(400)));
    console.set_open(true);
    let text = &console.updates(Some(1))[0].entries[0].text;
    assert!(text.starts_with("ab") && text.len() <= MAX_ENTRY);
    assert_eq!(exchange_text("GET", "https://a.example/100.tti", 200, 512, 12_345), "GET https://a.example/100.tti 200 512 bytes 12 ms");
}
