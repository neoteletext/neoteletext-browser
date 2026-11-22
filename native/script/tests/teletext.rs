//! The teletext API as a script sees it (script-api.md sections 5.1,
//! 5.2, 5.4, 5.5, 5.6 and 5.8; work package T1): JavaScript run in the
//! container, its batches given to the engine as a decoder would give
//! them, and what a Level 2.5 receiver then shows compared with a kept
//! picture.
mod common;
#[path = "teletext/receiver.rs"]
mod receiver;

use std::fmt::Write as _;

use common::{Opens as _, all_lines, assert_checks, container, ints_event, key, lines, opening, run, script, tick, timed, view, with_events};
use neoteletext_script::limits::{MAX_HOST_STATE_BYTES, TICKS_PER_SECOND};
use neoteletext_script::ops::PointerAction;
use neoteletext_script::teletext::limits::{LOCAL_TRIPLETS, MAX_COLOUR_SETS_PER_SECOND};
use protocol::script::batch::Label;
use protocol::script::teletext::control_handle;
use protocol::script::{Batch, ContextFlags, ContextId, EventKind};
use receiver::Receiver;

/// The kept pictures' format: raised, with the pictures made again, by
/// any change to what a receiver shows for the same script.
const PICTURE_FORMAT: u32 = 1;

/// Compares `pictures` with the kept file `name` beside this test, or
/// writes the file when `NT_BLESS` is set.
#[track_caller]
fn expect_pictures(name: &str, pictures: &[(&str, String)]) {
    let mut text = format!("format {PICTURE_FORMAT}\n");
    for (title, picture) in pictures {
        let _ = write!(text, "\n== {title}\n{picture}");
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/teletext").join(name);
    if std::env::var_os("NT_BLESS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("no kept pictures at {}: {error}", path.display()));
    assert!(kept == text, "what the receiver shows is not the kept picture {}:\n{text}", path.display());
}

/// The worked page example's manifest, as the decoder would read it into
/// the context's opening: a tick every 60, the keys 1 and 2, the pointer's
/// presses.
fn example_opening() -> protocol::script::ContextOpen {
    let mut open = opening(ContextId::Page);
    open.tick_divisor = 60;
    open.keys = vec!["1".into(), "2".into()];
    open.flags = ContextFlags { pointer_press: true, ..open.flags };
    open
}

/// Section 5.8, the worked page example, exactly as the specification
/// gives it (work package T1's "done"): a double-height title, an index,
/// an amber band on the overlay, a control, and a clock that needs no
/// server. What a Level 2.5 receiver shows for it is the kept picture, at
/// its first tick and a second later; after the first step only the
/// clock's row leaves; and the control and the keys navigate.
#[test]
fn the_worked_page_example_is_what_a_receiver_shows() {
    let mut container = container();
    container.open_with(example_opening(), include_bytes!("teletext/101.js")).expect("the example opens");
    let mut receiver = Receiver::new("en");
    let first = container.step(&tick(0, 0)).unwrap();
    assert_eq!(lines(&first), Vec::<String>::new());
    receiver.take(&first);
    let rows: Vec<u8> = first.teletext_rows.iter().flatten().map(|row| row.row).collect();
    assert_eq!(rows, vec![1, 4, 5, 22]);
    let forecast = control_handle(4, 1).unwrap();
    assert_eq!(first.labels, Some(vec![Label { handle: forecast, label: "Shipping forecast".into() }]));
    assert_eq!(first.teletext_settings.as_ref().unwrap().colours, vec![(16, 0xFA0)]);
    let at_first_tick = receiver.picture();

    // A second passes: the clock's row and nothing else.
    let mut later = Vec::new();
    for at in 1..=TICKS_PER_SECOND {
        let batch = container.step(&tick(u32::try_from(at).unwrap(), at)).unwrap();
        assert_eq!(lines(&batch), Vec::<String>::new());
        assert_eq!((&batch.teletext_enhancement, &batch.teletext_settings, &batch.animate, &batch.labels), (&None, &None, &None, &None), "tick {at}");
        later.extend(batch.teletext_rows.iter().flatten().map(|row| (at, row.row)));
        receiver.take(&batch);
    }
    assert_eq!(later, vec![(TICKS_PER_SECOND, 22)]);
    expect_pictures("101.expect.txt", &[("the first tick", at_first_tick), ("a second later", receiver.picture())]);

    // The control answers a release of the pointer on it, and the keys
    // replace their default.
    let release = ints_event(EventKind::Pointer, [PointerAction::UP.number(), 5, 4, forecast], &[1, 0, 4, 0]);
    let pressed = container.step(&with_events(100, 61, vec![release])).unwrap();
    assert_eq!((pressed.navigate.as_deref(), view(&pressed).verdicts), (Some("102"), vec![0b10]));
    let typed = container.step(&with_events(101, 130, vec![key(EventKind::KeyDown, "2", 0)])).unwrap();
    assert_eq!((typed.navigate.as_deref(), view(&typed).verdicts), (Some("103"), vec![0b11]));
}

/// Sections 5.1 to 5.6 through `nt.teletext`: each call's result and each
/// refusal's kind and bound, as JavaScript is given them.
#[test]
fn the_teletext_api_as_a_script_sees_it() {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.flags = ContextFlags { pointer_press: true, ..open.flags };
    let source = script(include_str!("teletext/api.js"));
    container.open_with(open, source.as_bytes()).expect("the page opens");
    let first = container.step(&tick(0, 0)).unwrap();
    assert_checks(&lines(&first), 63);
    // A release on the control is heard by the control, then by nt.
    let forecast = control_handle(4, 1).unwrap();
    let release = ints_event(EventKind::Pointer, [PointerAction::UP.number(), 5, 4, forecast], &[1, 0, 4, 0]);
    let pressed = container.step(&with_events(1, 1, vec![release])).unwrap();
    assert_eq!(lines(&pressed), vec!["control true, nt true"]);
}

/// Section 3.1: a page context has the API of its page's renderer and no
/// other, and the channel context may ask teletext's level, languages and
/// measure but draws nothing of the page.
#[test]
fn nt_teletext_is_the_teletext_pages_and_the_channel_context_draws_nothing() {
    let mut regis = container();
    let mut open = opening(ContextId::Page);
    open.renderer = protocol::script::Renderer::Regis;
    regis.open_with(open, script(r#"equal("no teletext on a ReGIS page", nt.teletext, undefined); done();"#).as_bytes()).unwrap();
    assert_checks(&all_lines(&run(&mut regis, 0, 1)), 1);

    let mut container = container();
    let channel = script(
        r#"
        nt.addEventListener("pageshow", () => {
          equal("level", nt.teletext.level, 25);
          equal("measure", nt.teletext.measure("abc"), 3);
          throws("the plane is the page's", TypeError, "only the page context draws", () => nt.teletext.page.put(1, 0, "x"));
          throws("the overlay is the page's", TypeError, "only the page context draws", () => nt.teletext.overlay.clear());
          done();
        });
        "#,
    );
    container.open_with(opening(ContextId::Channel), channel.as_bytes()).unwrap();
    container.open_with(opening(ContextId::Page), b"nt.teletext.page.put(1, 0, 'page');").unwrap();
    let shown = common::text_event(EventKind::PageShow, [0x100, 0, 1, 0], "https://example.test/chan/100.tti");
    let batch = container.step(&with_events(0, 0, vec![shown])).unwrap();
    assert_checks(&lines(&batch), 4);
    assert_eq!(batch.teletext_rows.map(|rows| rows.len()), Some(1));
}

/// The security review's finding S-10 (TODO-7276; script-api.md section
/// 10.5): a page that redefines a colour on every tick, flashing
/// everything drawn in it thirty times a second in the spike, changes the
/// entry at most `MAX_COLOUR_SETS_PER_SECOND` times in any second here,
/// and is not stopped for it: nothing throws.
#[test]
fn a_hostile_page_cannot_flash_a_colour_faster_than_three_times_a_second() {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.tick_divisor = 1;
    container.open_with(open, include_bytes!("teletext/hostile_flash.js")).unwrap();
    let batches: Vec<Batch> = run(&mut container, 0, 3 * TICKS_PER_SECOND).into_iter().map(Result::unwrap).collect();
    assert_eq!(batches.iter().flat_map(lines).collect::<Vec<_>>(), Vec::<String>::new());
    let changes: Vec<u64> = (0..).zip(&batches).filter(|(_, batch)| batch.teletext_settings.is_some()).map(|(at, _)| at).collect();
    for (at, tick) in changes.iter().enumerate() {
        let within = changes[at..].iter().filter(|later| **later < tick + TICKS_PER_SECOND).count();
        assert!(within <= MAX_COLOUR_SETS_PER_SECOND, "{within} changes in the second from tick {tick}: {changes:?}");
    }
    assert!(changes.len() > MAX_COLOUR_SETS_PER_SECOND, "the page was not held still either: {changes:?}");
    assert!(container.stopped().is_none());
}

/// Section 5.2 and 10.1 rule 7 (the security review's lead on host
/// memory, TODO-7272 as TODO-7336 item 1 gives it to this work package):
/// a page that asks on every tick for far more overlay cells, marks and
/// controls than a page holds is refused each one past the bound, never
/// stages more than the page's 208 local triplets, and the host holds no
/// more for it than a page.
#[test]
fn a_hostile_page_cannot_pass_the_pages_local_triplets_or_grow_the_host() {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.tick_divisor = 1;
    container.open_with(open, include_bytes!("teletext/hostile_triplets.js")).unwrap();
    let before = container.host_bytes();
    let (batches, took) = timed(|| run(&mut container, 0, 3));
    println!("three ticks of 920 refused or granted teletext calls each took {:?} ({:?} running)", took.wall, took.running);
    let batches: Vec<Batch> = batches.into_iter().map(Result::unwrap).collect();
    for batch in &batches {
        let triplets = batch.teletext_enhancement.as_ref().map_or(0, Vec::len);
        assert!(triplets <= LOCAL_TRIPLETS, "{triplets} triplets staged");
        let _ = view(batch);
    }
    let said = batches.iter().flat_map(lines).collect::<Vec<_>>();
    assert_eq!(said.len(), 3, "{said:?}");
    assert!(said.iter().all(|line| line.starts_with("refused ") && line.ends_with(" used 208")), "{said:?}");
    assert!(
        container.host_bytes() <= before + 4096 && container.host_bytes() <= MAX_HOST_STATE_BYTES,
        "{} bytes held, {before} before",
        container.host_bytes()
    );
}
