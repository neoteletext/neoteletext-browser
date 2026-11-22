//! The component vocabulary as a script sees it and as a teletext
//! receiver shows it (script-api.md section 8; work package U1): a page
//! of all thirteen kinds run in the container, driven by scripted keys,
//! typed text and the pointer, its batches given to the engine as a
//! decoder would give them, and what a Level 2.5 receiver then shows
//! compared with kept pictures.
mod common;
#[path = "teletext/receiver.rs"]
mod receiver;

use std::fmt::Write as _;

use common::{Opens as _, assert_checks, container, ints_event, key, lines, opening, script, text_event, tick, view, with_events};
use neoteletext_script::limits::{MAX_HOST_STATE_BYTES, TICKS_PER_SECOND};
use neoteletext_script::ops::PointerAction;
use neoteletext_script::teletext::limits::{LOCAL_TRIPLETS, MAX_CONTROLS};
use neoteletext_script::ui::limits::MAX_COMPONENTS;
use protocol::script::batch::Label;
use protocol::script::teletext::control_handle;
use protocol::script::{Batch, ContextFlags, ContextId, ContextOpen, Event, EventKind};
use receiver::Receiver;

/// The kept pictures' format: raised, with the pictures made again, by
/// any change to what a receiver shows for the same script.
const PICTURE_FORMAT: u32 = 1;
/// The modifier bit of Shift in a key event.
const SHIFT: i32 = 1;

/// Compares `pictures` with the kept file `name` beside this test, or
/// writes the file when `NT_BLESS` is set.
#[track_caller]
fn expect_pictures(name: &str, pictures: &[(&str, String)]) {
    let mut text = format!("format {PICTURE_FORMAT}\n");
    for (title, picture) in pictures {
        let _ = write!(text, "\n== {title}\n{picture}");
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui").join(name);
    if std::env::var_os("NT_BLESS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("no kept pictures at {}: {error}", path.display()));
    assert!(kept == text, "what the receiver shows is not the kept picture {}:\n{text}", path.display());
}

/// A page context's opening that claims what the thirteen kinds use: the
/// keys `nt.ui.keys` names with Tab, typed text and the pointer's presses.
fn claiming() -> ContextOpen {
    let mut open = opening(ContextId::Page);
    open.keys = ["Tab", "Enter", " ", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown", "Escape", "Backspace"]
        .map(String::from)
        .to_vec();
    open.flags = ContextFlags { text: true, pointer_press: true, ..open.flags };
    open
}

/// A release of the pointer on the thing `handle` at `row`, `column`.
fn release(row: u8, column: u8, handle: i32) -> Event {
    ints_event(EventKind::Pointer, [PointerAction::UP.number(), i32::from(column), i32::from(row), handle], &[1, 0, 0, 0])
}

/// A press of the pointer on the thing `handle`.
fn press(row: u8, column: u8, handle: i32) -> Event {
    ints_event(EventKind::Pointer, [PointerAction::DOWN.number(), i32::from(column), i32::from(row), handle], &[1, 1, 0, 0])
}

/// Steps `container` once for each of `events`, giving the lines said and
/// each event's verdict, and the batches to `receiver`.
fn play(container: &mut neoteletext_script::Container, receiver: &mut Receiver, at: &mut u32, events: Vec<Event>) -> (Vec<String>, Vec<u8>) {
    let mut said = Vec::new();
    let mut verdicts = Vec::new();
    for event in events {
        *at += 1;
        let batch = container.step(&with_events(*at, u64::from(*at), vec![event])).unwrap();
        let seen = view(&batch);
        verdicts.extend(seen.verdicts);
        said.extend(lines(&batch));
        check_bounds(&batch);
        receiver.take(&batch);
    }
    (said, verdicts)
}

/// What every batch of a page of components holds to: teletext's own.
fn check_bounds(batch: &Batch) {
    let triplets = batch.teletext_enhancement.as_ref().map_or(0, Vec::len);
    assert!(triplets <= LOCAL_TRIPLETS, "{triplets} triplets of local enhancement data");
}

/// Section 8, all thirteen kinds on one teletext page (work package U1's
/// "done"): what a receiver shows when the page is made, after the keys,
/// text and pointer below, and a second later, is the kept picture; each
/// component reports the events section 8's table gives it, with its
/// value; a listener cancels one default action and replaces another; a
/// key a component uses is not left to the browser, and one it does not
/// use is.
#[test]
fn a_page_of_all_thirteen_kinds_is_what_a_receiver_shows_and_reports_its_events() {
    let mut container = container();
    container.open_with(claiming(), include_bytes!("ui/thirteen.js")).expect("the page opens");
    let mut receiver = Receiver::new("en");
    let first = container.step(&tick(0, 0)).unwrap();
    assert_eq!(lines(&first), vec!["made 13 components, 12 controls, 31 triplets; focus on story"]);
    check_bounds(&first);
    // A component's label is its first control's, for a screen reader.
    assert_eq!(first.labels, Some(vec![Label { handle: control_handle(9, 0).unwrap(), label: "Sections".into() }]));
    receiver.take(&first);
    let made = receiver.picture();
    let mut at = 0;
    let down = |name: &str| key(EventKind::KeyDown, name, 0);

    // The paragraph has the focus, being the first made that takes keys:
    // the space bar shows its next part, PageUp the part before, and at
    // its start PageUp is used and reports nothing.
    let (said, verdicts) =
        play(&mut container, &mut receiver, &mut at, vec![down(" "), down(" "), down("PageUp"), down("PageUp"), down("PageUp"), down("ArrowDown")]);
    assert_eq!(said, vec!["page story 2", "page story 4", "page story 2", "page story 0"]);
    // Bit 0 is "not the browser's" and bit 1 that a listener heard it, as
    // the page's own keydown listener on nt always does. The fifth key is
    // used though it reports nothing; the arrow is not a paragraph's key,
    // so it is left to the browser.
    assert_eq!(verdicts, vec![0b11, 0b11, 0b11, 0b11, 0b11, 0b10]);

    // Tab moves the focus in the order of making. The menu: the highlight
    // passes over the disabled item, Home and End go to the ends, and
    // Enter activates the item, navigating when it has a target.
    let keys = ["Tab", "ArrowDown", "ArrowDown", "End", "Home", "ArrowUp", "ArrowDown", "Enter"].map(down).to_vec();
    let (said, _) = play(&mut container, &mut receiver, &mut at, keys);
    assert_eq!(said, vec!["move menu 1", "move menu 3", "move menu \"cam\"", "move menu \"ship\"", "move menu 1", "activate menu 1"]);
    let after_menu = receiver.picture();

    // The table: rows scroll under the headings; its value is the row.
    let keys = ["Tab", "PageDown", "ArrowDown", "Enter"].map(down).to_vec();
    let (said, _) = play(&mut container, &mut receiver, &mut at, keys);
    assert_eq!(said, vec!["move tides 3", "move tides 4", "activate tides 4"]);

    // The link navigates; the button's listener replaces nothing but adds
    // to its default; the toggle flips; the slider's listener cancels
    // every change past 6.
    let keys = ["Tab", "Enter", "Tab", " ", "Tab", "Enter", "Tab", "ArrowRight", "ArrowRight", "ArrowRight", "ArrowLeft", "Home"].map(down).to_vec();
    let (said, verdicts) = play(&mut container, &mut receiver, &mut at, keys);
    assert_eq!(
        said,
        vec![
            "activate link \"102\"",
            "activate refresh null",
            "change sound true",
            "change volume 5",
            "change volume 6",
            "change volume 7 cancelled",
            "change volume 5"
        ]
    );
    assert_eq!(verdicts.last(), Some(&0b10), "Home is no key of a slider's: it is left to the browser");

    // The field takes typed text at its caret, within its longest; the
    // caret's own moves report nothing; Enter submits and Escape cancels.
    let typed = |text: &str| text_event(EventKind::Text, [0; 4], text);
    let events = vec![
        down("Tab"),
        typed("Mary Rose"),
        down("Backspace"),
        down("ArrowLeft"),
        down("ArrowLeft"),
        typed("-"),
        typed(" of Portsmouth"),
        down("Enter"),
        down("Escape"),
    ];
    let (said, _) = play(&mut container, &mut receiver, &mut at, events);
    assert_eq!(
        said,
        vec![
            "change name \"Mary Rose\"",
            "change name \"Mary Ros\"",
            "change name \"Mary R-os\"",
            "change name \"Mary R- ofos\"",
            "submit name \"Mary R- ofos\"",
            "cancel name \"Mary R- ofos\""
        ]
    );
    // Tab goes round to the first, and with Shift back to the last.
    let (said, _) = play(&mut container, &mut receiver, &mut at, vec![down("Tab"), key(EventKind::KeyDown, "Tab", SHIFT), down("Escape"), typed("!")]);
    // The field is full: the text is used and nothing is reported.
    assert_eq!(said, vec!["cancel name \"Mary R- ofos\""]);

    // The pointer: a release on a row of the menu activates that item,
    // and a press on the toggle gives it the focus, where the space bar
    // then flips it.
    let webcam = control_handle(12, 0).unwrap();
    let toggle = control_handle(15, 14).unwrap();
    let events = vec![press(12, 3, webcam), release(12, 3, webcam), press(15, 16, toggle), release(15, 16, toggle), down(" ")];
    let (said, _) = play(&mut container, &mut receiver, &mut at, events);
    assert_eq!(said, vec!["activate menu 3", "change sound false", "change sound true"]);
    let after_input = receiver.picture();

    // A second later the clock, and nothing else, is written again.
    let mut later = Vec::new();
    for tick_at in (1..=TICKS_PER_SECOND).map(|more| u64::from(at) + more) {
        let batch = container.step(&tick(u32::try_from(tick_at).unwrap(), tick_at)).unwrap();
        later.extend(batch.teletext_rows.iter().flatten().map(|row| row.row));
        assert_eq!((&batch.teletext_enhancement, lines(&batch)), (&None, Vec::<String>::new()));
        receiver.take(&batch);
    }
    assert_eq!(later, vec![5]);
    expect_pictures(
        "thirteen.expect.txt",
        &[
            ("as made", made),
            ("after the menu's keys", after_menu),
            ("after every key, the text and the pointer", after_input),
            ("a second later", receiver.picture()),
        ],
    );
    assert!(container.stopped().is_none());
}

/// Sections 8, 3.7 and 10.1 through `nt.ui`: each call's result and each
/// refusal's kind and bound, as JavaScript is given them, the
/// vocabulary's own and teletext's.
#[test]
fn the_component_api_as_a_script_sees_it() {
    let mut container = container();
    let source = script(include_str!("ui/api.js"));
    container.open_with(claiming(), source.as_bytes()).expect("the page opens");
    let first = container.step(&tick(0, 0)).unwrap();
    assert_checks(&lines(&first), 52);
    let _ = view(&first);
}

/// Section 10.1 rules 3 and 7 and section 8's bounds (the security
/// review's leads on host memory, TODO-7272): a page that asks on every
/// tick for more components, items, option bytes, controls and triplets
/// than there are is refused each by name, the page's local enhancement
/// data stays within its 208 triplets and its controls within 32, and
/// the host holds no more for it than the vocabulary's bound.
#[test]
fn a_hostile_page_cannot_pass_the_components_bounds_or_grow_the_host() {
    let mut container = container();
    let mut open = claiming();
    open.tick_divisor = 1;
    container.open_with(open, include_bytes!("ui/hostile.js")).unwrap();
    let mut said = Vec::new();
    for at in 0..6_u32 {
        let batch = container.step(&tick(at, u64::from(at))).unwrap();
        let _ = view(&batch);
        check_bounds(&batch);
        said.extend(lines(&batch));
    }
    let last = said.last().unwrap();
    println!("{last}; {} bytes held", container.host_bytes());
    for bound in [
        "MAX_COMPONENTS",
        "MAX_UI_ITEMS",
        "MAX_UI_OPTION_BYTES",
        "MAX_UI_STATE_BYTES",
        "MAX_CONTROLS",
        "LOCAL_TRIPLETS",
        "UI_GRID_ROWS",
        "UI_GRID_COLUMNS",
        "TypeError",
    ] {
        assert!(last.contains(&format!("\"{bound}\":")), "nothing was refused by {bound}: {last}");
    }
    assert!(last.contains(&format!("components {MAX_COMPONENTS} ")) && last.contains(&format!("controls {MAX_CONTROLS} ")), "{last}");
    assert!(container.host_bytes() <= MAX_HOST_STATE_BYTES, "{} bytes held", container.host_bytes());
    assert!(container.stopped().is_none());
}
