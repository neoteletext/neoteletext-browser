//! The component vocabulary on a ReGIS page (script-api.md section 8;
//! work package U2, rdm TODO-7306): the page of all thirteen kinds run in
//! the container, driven by scripted keys, typed text and the pointer,
//! its batches given to the renderer's own terminal as a decoder gives
//! them, and what that receiver then shows compared with kept pictures.
//! The events are the vocabulary's, the same as on teletext; the drawing
//! is ReGIS's own, and the tests hold it to what ReGIS is good at: a
//! change is an attribute of a numbered drawing wherever one will do.
mod common;
#[path = "regis_page/fonts.rs"]
mod fonts;
#[path = "regis_page/receiver.rs"]
mod receiver;

use common::{Opens as _, assert_checks, container, ints_event, key, lines, opening, script, text_event, tick, view, with_events};
use neoteletext_script::Container;
use neoteletext_script::limits::regis::{CHANNEL_SHARE_DRAWINGS, REGIS_MAX_DRAWINGS};
use neoteletext_script::limits::{MAX_HOST_STATE_BYTES, TICKS_PER_SECOND};
use neoteletext_script::ops::PointerAction;
use neoteletext_script::ui::limits::MAX_COMPONENTS;
use protocol::script::{Batch, ContextFlags, ContextId, ContextOpen, Event, EventKind, Renderer};
use receiver::{GOLDEN_FORMAT, Receiver, keep};

const GOLDEN: &str = include_str!("golden/ui-regis.txt");
/// The modifier bit of Shift in a key event.
const SHIFT: i32 = 1;
/// A cell's side in pixels on ReGIS (section 8, "Placement").
const CELL: i32 = 20;

/// A page context's opening on a ReGIS page that claims what the
/// thirteen kinds use.
fn claiming() -> ContextOpen {
    let mut open = opening(ContextId::Page);
    open.renderer = Renderer::Regis;
    open.keys = ["Tab", "Enter", " ", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown", "Escape", "Backspace"]
        .map(String::from)
        .to_vec();
    open.flags = ContextFlags { text: true, pointer_press: true, ..open.flags };
    open
}

/// The ReGIS a batch carries, as text.
fn regis_of(batch: &Batch) -> String {
    String::from_utf8_lossy(batch.regis.as_deref().unwrap_or_default()).into_owned()
}

/// Steps `container` once for each of `events`, giving the lines said,
/// each event's verdict and each step's ReGIS, and the batches to
/// `receiver`.
fn play(container: &mut Container, receiver: &mut Receiver, at: &mut u32, events: Vec<Event>) -> (Vec<String>, Vec<u8>, Vec<String>) {
    let (mut said, mut verdicts, mut written) = (Vec::new(), Vec::new(), Vec::new());
    for event in events {
        *at += 1;
        let batch = container.step(&with_events(*at, u64::from(*at), vec![event])).unwrap();
        verdicts.extend(view(&batch).verdicts);
        said.extend(lines(&batch));
        written.push(regis_of(&batch));
        receiver.take(&batch);
    }
    (said, verdicts, written)
}

/// A press or a release of the pointer at the middle of the cell `row`,
/// `column`, on the drawing the receiver picks there, as the decoder
/// names it: by its number.
fn pointer(receiver: &mut Receiver, action: PointerAction, row: i32, column: i32) -> Event {
    let (x, y) = (column * CELL + CELL / 2, row * CELL + CELL / 2);
    let picture = receiver.picture();
    let pick = picture.pick(u32::try_from(x).unwrap(), u32::try_from(y).unwrap());
    assert_ne!(pick.drawing, 0, "nothing takes the pointer at row {row}, column {column}");
    println!("the pointer at row {row}, column {column} is on drawing {}, at {},{} of it", pick.drawing, pick.x, pick.y);
    let buttons = i32::from(action == PointerAction::DOWN);
    ints_event(EventKind::Pointer, [action.number(), x, y, i32::from(pick.drawing)], &[1, buttons, pick.x, pick.y])
}

/// How many drawings' contents `written` gives, and how many it deletes.
fn defined_and_deleted(written: &str) -> (usize, usize) {
    (written.matches("D(B").count(), written.matches("D(K").count())
}

/// Section 8, all thirteen kinds on one ReGIS page (work package U2's
/// "done"): the same page as teletext's reports the same events, line by
/// line; what a receiver built on the renderer shows when the page is
/// made, after the keys, text and pointer, and a second later, is the
/// kept picture; and each change is written as ReGIS does it best: a
/// menu's highlight, a scroll, a slider's thumb, a toggle and the focus
/// are attributes alone, with no drawing's contents written again.
#[test]
#[expect(clippy::too_many_lines, reason = "one page followed through every kind's keys, as the teletext test follows it")]
fn a_page_of_all_thirteen_kinds_is_what_a_receiver_shows_and_reports_its_events() {
    let mut container = container();
    container.open_with(claiming(), include_bytes!("ui/thirteen-regis.js")).expect("the page opens");
    let mut receiver = Receiver::new();
    let mut wrong = Vec::new();
    let first = container.step(&tick(0, 0)).unwrap();
    let made = lines(&first);
    println!("{made:?}; {} bytes of ReGIS", regis_of(&first).len());
    assert_eq!(made.len(), 1);
    assert!(made[0].starts_with("made 13 components, ") && made[0].ends_with("; focus on story"), "{made:?}");
    receiver.take(&first);
    // A component's label is its top drawing's, for a screen reader.
    let labels: Vec<String> = receiver.terminal.labels().into_iter().map(|(_, label)| label).collect();
    assert_eq!(labels, vec!["Sections"]);
    keep(GOLDEN, "thirteen-as-made", &mut receiver, &mut wrong);
    let mut at = 0;
    let down = |name: &str| key(EventKind::KeyDown, name, 0);

    // The paragraph has the focus, being the first made that takes keys.
    let (said, verdicts, written) =
        play(&mut container, &mut receiver, &mut at, vec![down(" "), down(" "), down("PageUp"), down("PageUp"), down("PageUp"), down("ArrowDown")]);
    assert_eq!(said, vec!["page story 2", "page story 4", "page story 2", "page story 0"]);
    assert_eq!(verdicts, vec![0b11, 0b11, 0b11, 0b11, 0b11, 0b10]);
    // A part of a paragraph is its text written again, in one drawing.
    assert_eq!(defined_and_deleted(&written[0]).0, 1, "{}", written[0]);
    assert_eq!(written[4], "", "a key that changes nothing writes nothing");

    // Tab moves the focus: the ring of one component is hidden and the
    // next one's shown. The menu's highlight is the source of two rows.
    let keys = ["Tab", "ArrowDown", "ArrowDown", "End", "Home", "ArrowUp", "ArrowDown", "Enter"].map(down).to_vec();
    let (said, _, written) = play(&mut container, &mut receiver, &mut at, keys);
    assert_eq!(said, vec!["move menu 1", "move menu 3", "move menu \"cam\"", "move menu \"ship\"", "move menu 1", "activate menu 1"]);
    for (step, changes) in written.iter().enumerate().take(7) {
        assert_eq!(defined_and_deleted(changes), (0, 0), "step {step} of the menu's keys wrote contents: {changes}");
    }
    assert!(written[0].contains("(V0)") && written[0].contains("(V1)"), "the focus is two rings' visibility: {}", written[0]);
    assert!(written[1].matches("(R").count() == 2, "a highlight moved is two rows' source: {}", written[1]);
    // End scrolls the menu: the group its rows hang from is moved.
    assert!(written[3].contains("(Y-40)"), "End moves the rows up two: {}", written[3]);
    keep(GOLDEN, "thirteen-after-the-menus-keys", &mut receiver, &mut wrong);

    // The table: rows scroll under the headings; its value is the row.
    let keys = ["Tab", "PageDown", "ArrowDown", "Enter"].map(down).to_vec();
    let (said, _, written) = play(&mut container, &mut receiver, &mut at, keys);
    assert_eq!(said, vec!["move tides 3", "move tides 4", "activate tides 4"]);
    assert!(written.iter().all(|changes| defined_and_deleted(changes) == (0, 0)), "{written:?}");

    // The link, the button, the toggle and the slider.
    let keys = ["Tab", "Enter", "Tab", " ", "Tab", "Enter", "Tab", "ArrowRight", "ArrowRight", "ArrowRight", "ArrowLeft", "Home"].map(down).to_vec();
    let (said, verdicts, written) = play(&mut container, &mut receiver, &mut at, keys);
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
    // The toggle: one knob hidden and the other shown. The slider: its
    // thumb's x. The progress bar the button fills: its text written
    // again, and its filling shown further by its clip rectangle.
    assert_eq!(defined_and_deleted(&written[5]), (0, 0), "{}", written[5]);
    assert!(written[5].contains("(V0)") && written[5].contains("(V1)"), "{}", written[5]);
    assert!(written[7].starts_with("D(S") && written[7].contains("(X") && defined_and_deleted(&written[7]) == (0, 0), "{}", written[7]);
    assert_eq!(written[9], "", "a change the script cancelled writes nothing");
    assert!(written[3].contains("C(X0,Y0,W") && defined_and_deleted(&written[3]) == (1, 0), "{}", written[3]);

    // The field.
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
    let (said, _, written) = play(&mut container, &mut receiver, &mut at, events);
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
    // The caret is made with the focus, with its two frames, and is
    // moved by its x alone.
    assert!(written[0].contains("F("), "the caret's frames: {}", written[0]);
    assert!(written[3].starts_with("D(S") && defined_and_deleted(&written[3]) == (0, 0), "a caret moved is its x: {}", written[3]);
    let (said, _, _) = play(&mut container, &mut receiver, &mut at, vec![down("Tab"), key(EventKind::KeyDown, "Tab", SHIFT), down("Escape"), typed("!")]);
    assert_eq!(said, vec!["cancel name \"Mary R- ofos\""]);

    // The pointer: the receiver picks the drawing under it, to the
    // pixel; a release on a row of the menu activates that item, and a
    // press on the toggle gives it the focus.
    let events = vec![pointer(&mut receiver, PointerAction::DOWN, 12, 3), pointer(&mut receiver, PointerAction::UP, 12, 3)];
    let (said, _, _) = play(&mut container, &mut receiver, &mut at, events);
    assert_eq!(said, vec!["activate menu 3"]);
    let events = vec![pointer(&mut receiver, PointerAction::DOWN, 15, 16), pointer(&mut receiver, PointerAction::UP, 15, 16), down(" ")];
    let (said, _, _) = play(&mut container, &mut receiver, &mut at, events);
    assert_eq!(said, vec!["change sound false", "change sound true"]);
    keep(GOLDEN, "thirteen-after-every-key-the-text-and-the-pointer", &mut receiver, &mut wrong);

    // A second later the clock's one drawing, and nothing else, is
    // written again.
    let mut later = Vec::new();
    for tick_at in (1..=TICKS_PER_SECOND).map(|more| u64::from(at) + more) {
        let batch = container.step(&tick(u32::try_from(tick_at).unwrap(), tick_at)).unwrap();
        assert_eq!(lines(&batch), Vec::<String>::new());
        later.extend(Some(regis_of(&batch)).filter(|written| !written.is_empty()));
        receiver.take(&batch);
    }
    assert_eq!(later.len(), 1, "{later:?}");
    assert!(later[0].starts_with("D(B") && defined_and_deleted(&later[0]) == (1, 0) && later[0].ends_with("D(E)D(C)"), "{later:?}");
    keep(GOLDEN, "thirteen-a-second-later", &mut receiver, &mut wrong);
    assert!(wrong.is_empty(), "pictures differ from tests/golden/ui-regis.txt (format {GOLDEN_FORMAT}):\n{}", wrong.join("\n"));
    assert!(container.stopped().is_none());
}

/// Sections 8, 3.7 and 10.1 through `nt.ui` on a ReGIS page: ReGIS's own
/// refusals by ReGIS's names, each leaving no drawing and no byte of
/// contents; what each kind costs in drawings; and the script's own
/// drawings beside the components' on the page's one table.
#[test]
fn the_component_api_on_regis_as_a_script_sees_it() {
    let mut container = container();
    let source = script(include_str!("ui/api-regis.js"));
    container.open_with(claiming(), source.as_bytes()).expect("the page opens");
    let first = container.step(&tick(0, 0)).unwrap();
    assert_checks(&lines(&first), 34);
    // What the page staged is what the renderer takes.
    let mut receiver = Receiver::new();
    receiver.take(&first);
    assert_eq!(receiver.commits, 1);
}

/// Section 10.1 rules 3 and 7 and section 8's bounds on ReGIS (the
/// security review's leads on host memory, TODO-7272): a page that asks
/// on every tick for more components, rows, option bytes, colours and
/// labels than there are is refused each by name, its drawings stay
/// within the page's share, the host holds no more for it than its
/// bound, and no step's commit is one the renderer refuses.
#[test]
fn a_hostile_page_cannot_pass_the_components_bounds_on_regis_or_grow_the_host() {
    let mut container = container();
    let mut open = claiming();
    open.tick_divisor = 1;
    container.open_with(open, include_bytes!("ui/hostile-regis.js")).unwrap();
    let mut receiver = Receiver::new();
    let mut said = Vec::new();
    for at in 0..6_u32 {
        let batch = container.step(&tick(at, u64::from(at))).unwrap();
        receiver.take(&batch);
        said.extend(lines(&batch));
    }
    let last = said.last().unwrap();
    println!("{last}; {} bytes held; {} bytes of ReGIS in {} commits", container.host_bytes(), receiver.bytes, receiver.commits);
    for bound in [
        "MAX_COMPONENTS",
        "MAX_UI_ITEMS",
        "MAX_UI_OPTION_BYTES",
        "MAX_UI_STATE_BYTES",
        "REGIS_MAX_DRAWINGS",
        "REGIS_COLOURS",
        "MAX_LABEL_BYTES",
        "UI_GRID_ROWS",
        "UI_GRID_COLUMNS",
        "TypeError",
    ] {
        assert!(last.contains(&format!("\"{bound}\":")), "nothing was refused by {bound}: {last}");
    }
    assert!(last.contains(&format!("components {MAX_COMPONENTS} ")) && last.contains(" failed 0 "), "{last}");
    let drawings: usize = last.split(" drawings ").nth(1).and_then(|rest| rest.split(' ').next()).and_then(|count| count.parse().ok()).unwrap();
    assert!(drawings <= REGIS_MAX_DRAWINGS - CHANNEL_SHARE_DRAWINGS && drawings > 1500, "{drawings} drawings");
    assert!(container.host_bytes() <= MAX_HOST_STATE_BYTES, "{} bytes held", container.host_bytes());
    assert!(container.stopped().is_none());
}

/// A commit the renderer refuses takes the whole step with it, the
/// components' changes too (`regis-v2.md`, "Failed commits"). The
/// components are then drawn again as they are: a page whose script
/// staged a cycle of drawings in the step that changed, made and removed
/// components ends with the picture of the same page without the cycle.
#[test]
fn components_changed_in_a_step_whose_commit_is_refused_are_drawn_again() {
    let page = |faulty: bool| {
        let body = format!(
            r#"
const ui = nt.ui;
const menu = ui.menu({{ at: {{ row: 2, column: 0, columns: 12, rows: 3 }}, items: [{{ text: "one" }}, {{ text: "two" }}, {{ text: "three" }}] }});
const gone = ui.button({{ at: {{ row: 6, column: 0, columns: 8 }}, text: "Gone" }});
const bar = ui.progress({{ at: {{ row: 8, column: 0, columns: 12 }}, value: 10 }});
const a = nt.regis.drawing({{ x: 300 }});
const b = nt.regis.drawing({{ parent: a }});
let failures = 0;
addEventListener("error", (e) => {{ failures++; e.preventDefault(); }});
nt.addEventListener("tick", (e) => {{
  if (e.tick === 1) {{
    menu.set({{ selected: 2 }});
    gone.remove();
    bar.set({{ value: 90 }});
    ui.toggle({{ at: {{ row: 10, column: 0, columns: 12 }}, text: "New", on: true }});
    if ({faulty}) b.source = a;
  }}
  if (e.tick === 2 && {faulty}) b.source = null;
  if (e.tick === 4) console.log("failures " + failures + " components " + nt.budget().components.used + " drawings " + nt.budget().drawings.used);
}});
"#
        );
        let mut container = container();
        let mut open = claiming();
        open.tick_divisor = 1;
        container.open_with(open, script(&body).as_bytes()).expect("the page opens");
        let mut receiver = Receiver::new();
        let mut said = Vec::new();
        for at in 0..5_u32 {
            let batch = container.step(&tick(at, u64::from(at))).unwrap();
            said.extend(lines(&batch));
            receiver.take(&batch);
        }
        let picture = receiver.picture();
        (said, receiver::digest(&picture, &receiver.terminal), receiver.commits)
    };
    let (said, picture, commits) = page(false);
    let (said_faulty, picture_faulty, commits_faulty) = page(true);
    assert_eq!(said, vec!["failures 0 components 3 drawings 24"]);
    assert!(said_faulty[0].starts_with("the step's ReGIS commit would fail (cycle, code 4, "), "{said_faulty:?}");
    assert_eq!(said_faulty[1..], ["failures 1 components 3 drawings 24"]);
    assert_eq!(picture_faulty, picture, "the components are not drawn as the page without the refused commit draws them");
    // The refused step sent nothing; the step after it drew the changes.
    assert_eq!((commits, commits_faulty), (2, 2));
}
