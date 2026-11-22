//! The component vocabulary with no renderer (script-api.md section 8):
//! the options reader against outside input, and focus, keys and events
//! against a painter that only records what it is asked to draw. What
//! these hold of is what work packages U2 and U3 may rely on when they
//! give the vocabulary their own painters.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, reason = "tests")]

use protocol::script::ContextId;

use super::limits::{COMPONENT_HANDLE_BASE, MAX_COMPONENTS, MAX_UI_ITEMS, MAX_UI_OPTION_BYTES};
use super::options::{Area, Props, read};
use super::{Component, Kind, Painter, Shown, Time, Ui, keys_json};
use crate::host::Reply;

const PAGE: ContextId = ContextId::Page;

/// A painter that draws nothing: it records each call, refuses when told
/// to, and names every component's part 7 by the component's own handle.
#[derive(Default)]
struct Recorder {
    /// The handle, whether it was focused, and its value as JSON, of each
    /// component drawn; a handle alone for an erasure.
    drawn: Vec<(i32, Option<(bool, String)>)>,
    refuse: bool,
}

impl Painter for Recorder {
    fn text_columns(&self, component: &Component) -> usize {
        usize::from(component.at.columns)
    }

    fn paint(&mut self, before: Option<&Component>, now: Option<&Shown<'_>>) -> Result<(), Reply> {
        if self.refuse && now.is_some() {
            return Err(Reply::Range("RENDERER_FULL: no room".into()));
        }
        let handle = now.map(|shown| shown.component.handle).or(before.map(|component| component.handle)).unwrap();
        self.drawn.push((
            handle,
            now.map(|shown| {
                let mut value = String::new();
                shown.component.value().write(&mut value);
                (shown.focused, value)
            }),
        ));
        Ok(())
    }

    fn part(&self, handle: i32) -> Option<(i32, usize)> {
        (handle >= COMPONENT_HANDLE_BASE).then_some((handle, 1))
    }
}

fn area() -> Area {
    Area::new(1, 0, 20, 3).unwrap()
}

fn make(ui: &mut Ui, painter: &mut Recorder, kind: Kind, options: &str) -> i32 {
    ui.create(painter, PAGE, 0, kind, area(), options.as_bytes()).unwrap()
}

const MENU: &str = r#"{"items":[{"text":"a","value":"A"},{"text":"b","disabled":true},{"text":"c","target":"103"},{"text":"d"}]}"#;

/// Section 8 and 10.1: a call's options are outside input. Text that is
/// not JSON, not an object, a member given twice, a number that is not
/// whole, nesting past the reader's depth, more values or bytes than the
/// bounds, and every member of the wrong type or range are refused, and
/// a kind takes only its own options.
#[test]
fn the_options_reader_refuses_what_is_not_a_kinds_options() {
    let refused = |kind: Kind, options: &str| Props::default().take(kind, &read(options.as_bytes())?).map(|_| ());
    for text in [
        "",
        "[",
        "[]",
        "7",
        "{\"text\":}",
        "{\"text\":\"a\",}",
        "{\"text\":\"a\"} x",
        "{\"text\":\"a\",\"text\":\"b\"}",
        "{\"value\":1.5}",
        "{\"value\":1e3}",
        "nul",
    ] {
        assert!(matches!(read(text.as_bytes()), Err(Reply::Type(_))), "{text}");
    }
    assert!(matches!(read(&[b'{', 0xFF, b'}']), Err(Reply::Type(_))));
    let deep = format!("{{\"items\":{}1{}}}", "[".repeat(200), "]".repeat(200));
    assert!(matches!(read(deep.as_bytes()), Err(Reply::Type(_))));
    let many = format!("{{\"lines\":[{}0]}}", "0,".repeat(MAX_UI_ITEMS * 8));
    assert!(matches!(read(many.as_bytes()), Err(Reply::Type(text)) if text.contains("more values")));
    let long = format!("{{\"text\":\"{}\"}}", "x".repeat(MAX_UI_OPTION_BYTES));
    assert!(matches!(read(long.as_bytes()), Err(Reply::Range(text)) if text.starts_with("MAX_UI_OPTION_BYTES: ")));
    for (kind, options, bound) in [
        (Kind::Menu, format!("{{\"items\":[{}{{}}]}}", "{},".repeat(MAX_UI_ITEMS)), "MAX_UI_ITEMS"),
        (Kind::Frame, r#"{"prompts":["a","b","c","d","e"]}"#.to_owned(), "MAX_UI_PROMPTS"),
        (Kind::Table, format!("{{\"columns\":[{}]}}", [r#"{"title":"t","width":1}"#; 9].join(",")), "MAX_UI_TABLE_COLUMNS"),
        (Kind::Slider, r#"{"value":1000001}"#.to_owned(), "MAX_UI_NUMBER"),
        (Kind::Slider, r#"{"step":0}"#.to_owned(), "MAX_UI_NUMBER"),
        (Kind::Field, r#"{"longest":257}"#.to_owned(), "MAX_UI_FIELD_CHARS"),
        (Kind::Text, r#"{"height":3}"#.to_owned(), "UI_TEXT_HEIGHT"),
        (Kind::Text, r#"{"ink":256}"#.to_owned(), "UI_COLOURS"),
        (Kind::Text, r#"{"at":{"row":1,"column":0,"columns":41}}"#.to_owned(), "UI_GRID_COLUMNS"),
        (Kind::Text, r#"{"at":{"row":1,"column":0,"columns":4,"rows":24}}"#.to_owned(), "UI_GRID_ROWS"),
    ] {
        assert!(matches!(refused(kind, &options), Err(Reply::Range(text)) if text.starts_with(&format!("{bound}: "))), "{bound}");
    }
    for (kind, options) in [
        (Kind::Text, r#"{"items":[]}"#),
        (Kind::Menu, r#"{"items":{}}"#),
        (Kind::Menu, r#"{"items":[{"value":true}]}"#),
        (Kind::Menu, r#"{"items":["a"]}"#),
        (Kind::Table, r#"{"columns":[{"title":"t"}]}"#),
        (Kind::Text, r#"{"at":{"row":1,"column":0}}"#),
        (Kind::Text, r#"{"at":{"row":1,"column":0,"columns":4,"depth":1}}"#),
        (Kind::Text, r#"{"align":"middle"}"#),
        (Kind::Text, r#"{"fit":true}"#),
        (Kind::Clock, r#"{"zone":"Europe/London"}"#),
        (Kind::Toggle, r#"{"on":1}"#),
        (Kind::Text, "{\"text\":\"a\\u0007\"}"),
    ] {
        assert!(matches!(refused(kind, options), Err(Reply::Type(_))), "{options}");
    }
    assert_eq!(refused(Kind::Text, r#"{"text":"café","ink":"yellow","paper":17,"align":"right","fit":"shrink","height":2}"#), Ok(()));
    assert_eq!(keys_json(Kind::Slider as i32).unwrap(), r#"["ArrowLeft","ArrowRight"]"#);
    assert!(keys_json(13).is_err() && keys_json(-1).is_err());
}

/// Section 8, "Focus and keys": the first component the focus can rest on
/// takes it; Tab moves it in the order of making, around the ends, past
/// what takes no keys and what is disabled; and each change draws the
/// component that lost it and the one that took it.
#[test]
fn the_focus_moves_in_the_order_of_making() {
    let (mut ui, mut painter) = (Ui::default(), Recorder::default());
    let band = make(&mut ui, &mut painter, Kind::Band, r#"{"text":"t"}"#);
    let button = make(&mut ui, &mut painter, Kind::Button, r#"{"text":"b"}"#);
    let off = make(&mut ui, &mut painter, Kind::Toggle, r#"{"text":"t","disabled":true}"#);
    let table = make(&mut ui, &mut painter, Kind::Table, "{}");
    let slider = make(&mut ui, &mut painter, Kind::Slider, "{}");
    assert_eq!((band, button, slider), (COMPONENT_HANDLE_BASE, COMPONENT_HANDLE_BASE + 1, COMPONENT_HANDLE_BASE + 4));
    assert_eq!(ui.focused(PAGE), button);
    assert_eq!(painter.drawn[1], (button, Some((true, "null".into()))));
    painter.drawn.clear();
    assert_eq!(ui.offer_key(&mut painter, PAGE, "Tab", false), Some(format!("{{\"h\":{slider}}}")));
    assert_eq!(painter.drawn, vec![(button, Some((false, "null".into()))), (slider, Some((true, "0".into())))]);
    ui.offer_key(&mut painter, PAGE, "Tab", false);
    assert_eq!(ui.focused(PAGE), button, "around the end, past the table that is not selectable");
    ui.offer_key(&mut painter, PAGE, "Tab", true);
    assert_eq!(ui.focused(PAGE), slider);
    // The focus rests only where keys are taken, and leaves a component
    // that is disabled or removed.
    assert!(ui.focus(&mut painter, PAGE, band).is_err() && ui.focus(&mut painter, PAGE, off).is_err() && ui.focus(&mut painter, PAGE, table).is_err());
    assert!(ui.focus(&mut painter, ContextId::Channel, slider).is_err(), "another context's component is no thing");
    ui.set(&mut painter, PAGE, slider, br#"{"disabled":true}"#).unwrap();
    assert_eq!(ui.focused(PAGE), 0);
    assert_eq!(ui.offer_key(&mut painter, PAGE, "ArrowRight", false), None, "no component has the focus: the key is the browser's");
    ui.focus(&mut painter, PAGE, button).unwrap();
    ui.remove(&mut painter, PAGE, button).unwrap();
    assert_eq!((ui.focused(PAGE), ui.owns(PAGE, button), painter.drawn.last()), (0, false, Some(&(button, None))));
}

/// Sections 4.1 and 8: what a component does with a key is the event's
/// default action. The offer changes nothing; the verdict carries the
/// change out or drops it; and a renderer that has no room to draw the
/// change leaves the component as it was.
#[test]
fn a_components_default_action_waits_for_the_scripts_verdict() {
    let (mut ui, mut painter) = (Ui::default(), Recorder::default());
    let menu = make(&mut ui, &mut painter, Kind::Menu, MENU);
    // Down passes over the disabled item: the event's value is the item's
    // own, or its place.
    assert_eq!(ui.offer_key(&mut painter, PAGE, "ArrowDown", false), Some(format!("{{\"h\":{menu},\"what\":1,\"value\":2}}")));
    assert_eq!(ui.value(PAGE, menu).unwrap(), "\"A\"", "nothing has moved yet");
    assert_eq!(ui.settle(&mut painter, PAGE, menu, false), Ok(false));
    assert_eq!(ui.value(PAGE, menu).unwrap(), "\"A\"", "cancelled");
    assert_eq!(ui.settle(&mut painter, PAGE, menu, true), Ok(false), "nothing waits any more");
    ui.offer_key(&mut painter, PAGE, "ArrowDown", false);
    painter.refuse = true;
    assert_eq!(ui.settle(&mut painter, PAGE, menu, true), Ok(false));
    assert_eq!(ui.value(PAGE, menu).unwrap(), "\"A\"", "the renderer had no room, so the highlight stays");
    painter.refuse = false;
    ui.offer_key(&mut painter, PAGE, "ArrowDown", false);
    assert_eq!(ui.settle(&mut painter, PAGE, menu, true), Ok(true));
    assert_eq!(ui.value(PAGE, menu).unwrap(), "2");
    // Enter activates the item, and says where its default navigates.
    assert_eq!(ui.offer_key(&mut painter, PAGE, "Enter", false), Some(format!("{{\"h\":{menu},\"what\":2,\"value\":2,\"target\":\"103\"}}")));
    // A key the kind does not use is not the component's; one it uses but
    // that changes nothing is, with nothing to report.
    assert_eq!(ui.offer_key(&mut painter, PAGE, "ArrowLeft", false), None);
    ui.offer_key(&mut painter, PAGE, "End", false);
    ui.settle(&mut painter, PAGE, menu, true).unwrap();
    assert_eq!(ui.offer_key(&mut painter, PAGE, "ArrowDown", false), Some(format!("{{\"h\":{menu}}}")));
    // The pointer: a thing of the renderer names its component and part;
    // a press gives the focus, a release activates the part.
    let field = make(&mut ui, &mut painter, Kind::Field, r#"{"value":"ab","longest":4}"#);
    assert_eq!(ui.offer_pointer(&mut painter, PAGE, 5, true, false), None, "no component's thing");
    assert_eq!(ui.offer_pointer(&mut painter, PAGE, field, true, false), Some(format!("{{\"h\":{field}}}")));
    assert_eq!(ui.focused(PAGE), field);
    // The recorder names part 1, which is the menu's disabled item: a
    // release there is on the menu and does nothing.
    assert_eq!(ui.offer_pointer(&mut painter, PAGE, menu, false, true), Some(format!("{{\"h\":{menu}}}")));
    // A field takes text at its caret, as much as its longest allows.
    assert_eq!(ui.offer_text(PAGE, "cdef\u{7}"), Some(format!("{{\"h\":{field},\"what\":3,\"value\":\"abcd\"}}")));
    ui.settle(&mut painter, PAGE, field, true).unwrap();
    assert_eq!(ui.offer_text(PAGE, "x"), Some(format!("{{\"h\":{field}}}")), "full: used, with nothing to report");
    ui.offer_key(&mut painter, PAGE, "ArrowLeft", false);
    assert_eq!(ui.offer_key(&mut painter, PAGE, "Backspace", false), Some(format!("{{\"h\":{field},\"what\":3,\"value\":\"abd\"}}")));
    assert_eq!(ui.offer_key(&mut painter, PAGE, "Enter", false), Some(format!("{{\"h\":{field},\"what\":5,\"value\":\"abcd\"}}")));
}

/// Section 8's bounds and 10.1 rule 3: a component past the vocabulary's
/// count, and one the renderer refuses, leave nothing behind; a clock is
/// drawn again when the second changes and nothing else is; a paragraph's
/// text is broken at spaces within the painter's columns.
#[test]
fn a_refused_component_leaves_nothing_and_clocks_follow_the_time() {
    let (mut ui, mut painter) = (Ui::default(), Recorder::default());
    painter.refuse = true;
    assert!(matches!(ui.create(&mut painter, PAGE, 0, Kind::Text, area(), b"{}"), Err(Reply::Range(text)) if text.starts_with("RENDERER_FULL")));
    painter.refuse = false;
    let mut budget = String::new();
    ui.budget(&mut budget);
    assert_eq!(budget, ",\"components\":{\"used\":0,\"limit\":64},\"uiNodes\":{\"used\":0,\"limit\":16384}");
    let clock = make(&mut ui, &mut painter, Kind::Clock, r#"{"zone":"utc"}"#);
    let text = make(&mut ui, &mut painter, Kind::Text, r#"{"text":"t"}"#);
    painter.drawn.clear();
    ui.set_time(Some(&mut painter), Time { epoch_seconds: 1, zone_minutes: 0 });
    ui.set_time(Some(&mut painter), Time { epoch_seconds: 1, zone_minutes: 0 });
    assert_eq!(painter.drawn, vec![(clock, Some((false, "null".into())))]);
    assert_eq!(Time { epoch_seconds: 86_399, zone_minutes: 600 }.parts(false), (9, 59, 59));
    assert_eq!(Time { epoch_seconds: 60, zone_minutes: -90 }.parts(false), (22, 31, 0));
    assert_eq!(Time { epoch_seconds: 3_661, zone_minutes: 600 }.parts(true), (1, 1, 1));
    for _ in 2..MAX_COMPONENTS {
        make(&mut ui, &mut painter, Kind::Band, "{}");
    }
    assert!(matches!(ui.create(&mut painter, PAGE, 0, Kind::Band, area(), b"{}"), Err(Reply::Range(text)) if text.starts_with("MAX_COMPONENTS: ")));
    assert_eq!(ui.remove_scope(Some(&mut painter), PAGE, 0).len(), MAX_COMPONENTS);
    assert!(ui.is_empty() && !ui.owns(PAGE, text));
    let story = Component {
        handle: 0,
        kind: Kind::Paragraph,
        context: PAGE,
        scope: 0,
        at: Area::new(1, 0, 9, 2).unwrap(),
        props: Props { text: "the  quick brown extraordinarily fox".into(), ..Props::default() },
        view: super::View::default(),
    };
    assert_eq!(story.lines(9), vec!["the quick", "brown", "extraordi", "narily", "fox"]);
    assert_eq!((story.page_lines(5), story.page_lines(2)), (1, 2));
}
