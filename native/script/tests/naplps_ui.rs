//! The component vocabulary on NAPLPS as a script sees it and as a
//! receiver shows it (script-api.md section 8; work package U3;
//! DEC-7205): a page of all thirteen kinds run in the container, driven
//! by scripted keys, typed text and the pointer, its batches played by
//! the NAPLPS renderer at the pinned commit as the decoder plays them,
//! and what is then presented compared with kept pictures.
#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::indexing_slicing, clippy::panic, clippy::naive_bytecount, reason = "tests")]

mod common;
mod naplps_drawn;

use common::{Opens as _, container, ints_event, key, lines, opening, text_event, tick, view, with_events};
use naplps_drawn::{Receiver, expect, pixel};
use neoteletext_rules::animate::OperationKind;
use neoteletext_script::Container;
use neoteletext_script::limits::MAX_HOST_STATE_BYTES;
use neoteletext_script::ops::PointerAction;
use protocol::script::{Batch, ContextFlags, ContextId, ContextOpen, Event, EventKind, Renderer};

/// A page context's opening on a NAPLPS page that claims what the
/// thirteen kinds use: the keys `nt.ui.keys` names with Tab, typed text
/// and the pointer's presses.
fn claiming() -> ContextOpen {
    let mut open = opening(ContextId::Page);
    open.renderer = Renderer::Naplps;
    open.keys = ["Tab", "Enter", " ", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown", "Escape", "Backspace"]
        .map(String::from)
        .to_vec();
    open.flags = ContextFlags { text: true, pointer_press: true, ..open.flags };
    open
}

/// Steps `container` once for each of `events`, giving the lines said
/// and the batches, each played by `receiver`.
fn play(container: &mut Container, receiver: &mut Receiver, at: &mut u32, events: Vec<Event>) -> (Vec<String>, Vec<Batch>) {
    let mut said = Vec::new();
    let mut batches = Vec::new();
    for event in events {
        *at += 1;
        let batch = container.step(&with_events(*at, u64::from(*at), vec![event])).unwrap();
        // The batch is a message the link takes whole.
        view(&batch);
        said.extend(lines(&batch));
        receiver.take_batch(&batch);
        receiver.advance(1);
        batches.push(batch);
    }
    (said, batches)
}

/// The handle of the detectable object under the canvas dot (`x`, `y`),
/// as the decoder names it: kind 5 and the object's number.
fn thing_at(receiver: &Receiver, x: u32, y: u32) -> i32 {
    let numbers = receiver.terminal.objects_at(x, y + 20);
    (5 << 24) | i32::from(*numbers.last().unwrap_or_else(|| panic!("no object takes the pointer at ({x}, {y})")))
}

/// Section 8, all thirteen kinds on one NAPLPS page (work package U3's
/// "done"): what a receiver presents when the page is made, after the
/// keys, text and pointer below, is the kept picture; each component
/// reports the events section 8's table gives it, line by line; what
/// moves is moved by animation operations with nothing painted again;
/// and the parts the pointer takes are regions with the component's
/// label.
#[test]
fn a_page_of_all_thirteen_kinds_is_what_a_receiver_shows_and_reports_its_events() {
    let mut container = container();
    container.open_with(claiming(), include_bytes!("naplps_page/thirteen.js")).expect("the page opens");
    let mut receiver = Receiver::new();
    let first = container.step(&tick(0, 0)).unwrap();
    view(&first);
    // Thirteen components. Of the page's 56 objects: a region each for
    // the paragraph's prompt, four menu rows less the disabled one, three
    // table rows, the link, the button, the toggle, the slider and the
    // field; the menu's and the table's highlights; the slider's thumb;
    // and the focus ring. Of its 80 symbols: the colour the page
    // defined takes none; two outlines (the menu's and the table's
    // rows differ in width), the thumb, and the ring's two frames.
    assert_eq!(lines(&first), ["made 13 components, 16 objects, 5 symbols; focus on story"]);
    receiver.take_batch(&first);
    let record = first.naplps_objects.as_ref().expect("the components' objects");
    assert_eq!(record.objects.len(), 16);
    assert!(record.objects.iter().any(|object| object.label == "Sections"), "the menu's label is its first region's, for a screen reader");
    let diagnostics = receiver.terminal.diagnostics();
    assert_eq!((diagnostics.passed_over, diagnostics.refusals), (0, [0; naplps::REFUSAL_KINDS]), "{diagnostics:?}");
    let colours = receiver.colours();
    expect("thirteen-0-made", receiver.presented(), &colours);
    // What is drawn where: the frame's rule under its title, the band in
    // the page's own colour, the button's outline, the progress bar
    // filled to 40 of 100 of its inside, exact to a dot.
    let shown = receiver.presented().clone();
    assert!((0..480).all(|x| pixel(&shown, x, 18) == 3 && pixel(&shown, x, 19) == 3), "the frame's rule");
    assert_eq!(pixel(&shown, 2, 62), 8, "the band's paper is the entry the page defined");
    assert_eq!((pixel(&shown, 0, 280), pixel(&shown, 143, 280), pixel(&shown, 144, 280)), (7, 7, 0), "the button's outline");
    let bar: Vec<u8> = (264..480).map(|x| pixel(&shown, x, 130)).collect();
    assert_eq!(
        bar.iter().filter(|entry| **entry == 7).count(),
        2 + (216 - 4) * 40 / 100,
        "the progress bar: its outline's two sides and 40 of 100 of its inside"
    );

    let mut at = 0;
    let mut said = Vec::new();
    let down = |name: &str| key(EventKind::KeyDown, name, 0);
    // The paragraph has the focus, being the first made that takes keys.
    let (lines_said, _) = play(&mut container, &mut receiver, &mut at, vec![down(" "), down(" "), down("PageUp")]);
    said.extend(lines_said);
    // The menu: its highlight is an actor, so a move of the choice
    // paints nothing: the batch has no NAPLPS and no record, only moves.
    let (lines_said, batches) = play(&mut container, &mut receiver, &mut at, vec![down("Tab"), down("ArrowDown"), down("ArrowDown")]);
    said.extend(lines_said);
    for batch in &batches[1..] {
        assert_eq!((&batch.naplps, &batch.naplps_objects), (&None, &None), "a highlight's move paints and sends nothing");
        let moves = batch.animate.as_ref().expect("the moves");
        assert!(moves.iter().all(|operation| matches!(operation.kind, OperationKind::Move { relative: false, .. })), "{moves:?}");
        assert_eq!(moves.len(), 2, "the highlight and the focus ring");
    }
    let colours = receiver.colours();
    expect("thirteen-1-menu", receiver.presented(), &colours);
    let (lines_said, _) = play(&mut container, &mut receiver, &mut at, ["End", "Home", "Enter", "Tab", "PageDown", "ArrowDown", "Enter"].map(down).to_vec());
    said.extend(lines_said);
    // The link, the button, the toggle; the slider, whose thumb is an
    // actor moved to the dot and whose listener cancels changes past 6.
    let (lines_said, _) = play(&mut container, &mut receiver, &mut at, ["Tab", "Enter", "Tab", " ", "Tab", "Enter", "Tab"].map(down).to_vec());
    said.extend(lines_said);
    let (lines_said, batches) = play(&mut container, &mut receiver, &mut at, ["ArrowRight", "ArrowRight", "ArrowRight", "ArrowLeft"].map(down).to_vec());
    said.extend(lines_said);
    assert_eq!((&batches[0].naplps, &batches[0].naplps_objects), (&None, &None), "a thumb's move paints and sends nothing");
    assert!(batches[0].animate.is_some());
    assert_eq!(batches[2].animate, None, "a change the listener cancelled moves nothing");
    // The field: typed text is painted, and the caret is an actor the
    // receiver blinks.
    let typed = vec![down("Tab"), text_event(EventKind::Text, [0; 4], "Mary Rose"), down("Backspace"), down("ArrowLeft")];
    let (lines_said, _) = play(&mut container, &mut receiver, &mut at, typed);
    said.extend(lines_said);
    let colours = receiver.colours();
    expect("thirteen-2-field", receiver.presented(), &colours);
    let (lines_said, _) = play(&mut container, &mut receiver, &mut at, vec![down("Enter")]);
    said.extend(lines_said);
    // The pointer: a press and release on the menu's first row, which
    // is a region the decoder names by its object's number.
    let row = thing_at(&receiver, 40, 170);
    let press = ints_event(EventKind::Pointer, [PointerAction::DOWN.number(), 40, 170, row], &[1, 1, 0, 0]);
    let release = ints_event(EventKind::Pointer, [PointerAction::UP.number(), 40, 170, row], &[1, 0, 0, 0]);
    let (lines_said, _) = play(&mut container, &mut receiver, &mut at, vec![press, release]);
    said.extend(lines_said);
    // A second on, the clock has been painted over with the time.
    for _ in 0..60 {
        at += 1;
        let batch = container.step(&tick(at, u64::from(at))).unwrap();
        receiver.take_batch(&batch);
        receiver.advance(1);
    }
    let colours = receiver.colours();
    expect("thirteen-3-later", receiver.presented(), &colours);
    let diagnostics = receiver.terminal.diagnostics();
    assert_eq!((diagnostics.passed_over, diagnostics.refusals), (0, [0; naplps::REFUSAL_KINDS]), "{diagnostics:?}");
    assert!(container.host_bytes() <= MAX_HOST_STATE_BYTES);
    // Every event, line by line.
    let said: Vec<&str> = said.iter().map(String::as_str).collect();
    assert_eq!(
        said,
        [
            "page story 2",
            "page story 4",
            "page story 2",
            "move menu 1",
            "move menu 3",
            "move menu \"cam\"",
            "move menu \"ship\"",
            "activate menu \"ship\"",
            "move tides 3",
            "move tides 4",
            "activate tides 4",
            "activate link \"102\"",
            "activate refresh null",
            "change sound true",
            "change volume 5",
            "change volume 6",
            "change volume 7 cancelled",
            "change volume 5",
            "change name \"Mary Rose\"",
            "change name \"Mary Ros\"",
            "submit name \"Mary Ros\"",
            "activate menu \"ship\"",
        ],
        "{said:#?}"
    );
}

/// Section 10.1 rule 7 and section 8's "a page that runs out is told at
/// the call": a script that makes, changes and removes components
/// without end is refused by NAPLPS's own bounds by name, and the host
/// holds and sends no more than its bounds.
#[test]
fn a_page_that_makes_components_without_end_is_told_by_naplps_bounds() {
    let mut container = container();
    let mut open = claiming();
    open.tick_divisor = 1;
    container.open_with(open, include_bytes!("naplps_page/hostile-ui.js")).expect("the page opens");
    let mut said = Vec::new();
    for at in 0..6_u32 {
        let batch = container.step(&tick(at, u64::from(at))).unwrap();
        view(&batch);
        said.extend(lines(&batch));
        assert!(batch.naplps_objects.as_ref().is_none_or(|record| record.objects.len() <= 56));
        assert!(container.host_bytes() <= MAX_HOST_STATE_BYTES, "tick {at}: the host holds {}", container.host_bytes());
        assert_eq!(container.stop_reason(), None, "tick {at}");
    }
    assert!(said.iter().all(|line| line.starts_with("flood ")), "{said:?}");
    // Each bound a component can meet on NAPLPS was met, by its name.
    let all = said.join("\n");
    for bound in ["MAX_OBJECTS", "MAX_COMPONENTS", "NAPLPS_COLOURS", "COLUMNS"] {
        assert!(all.contains(bound), "no refusal by {bound}: {all}");
    }
}
