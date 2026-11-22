//! The components' drawings on the page's table (work package U2): a
//! script's own operations do not reach them, and a component the page
//! has no room for leaves the table and the step's operations exactly as
//! they were.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "tests")]

use protocol::script::{ContextFlags, ContextId, ContextOpen, Level, Renderer, SEED_BYTES};

use crate::host::{Host, Payload, Reply};
use crate::limits::regis::{CHANNEL_SHARE_DRAWINGS, REGIS_MAX_DRAWINGS};
use crate::ops::{Op, Property};
use crate::ui::Kind;

/// The page context's number in a host call.
const PAGE: i32 = 2;

fn host() -> Host {
    let mut host = Host::new();
    host.open(
        ContextOpen {
            context: ContextId::Page,
            api: 1,
            renderer: Renderer::Regis,
            level: Level::TwoFive,
            page: 0x1A0,
            subcode: 0,
            flags: ContextFlags { pointer_press: true, ..ContextFlags::default() },
            tick_divisor: 1,
            language: "en".into(),
            channel_url: "https://example.test/chan/".into(),
            page_url: "https://example.test/chan/1A0.tti".into(),
            script_url: "https://example.test/chan/page.js".into(),
            channel_title: "Example".into(),
            seed: [9; SEED_BYTES],
            epoch_seconds: 1_800_000_000,
            zone_minutes: 0,
            keys: Vec::new(),
            sounds: Vec::new(),
            source_bytes: 1,
        },
        200,
    );
    host.begin_step(0, true);
    host
}

/// The number of `kind` among the kinds, as operation `UI_CREATE` takes it.
fn kind_number(kind: Kind) -> i32 {
    i32::try_from(Kind::ALL.iter().position(|one| *one == kind).unwrap()).unwrap()
}

/// Makes a component of `kind` at row 2, 12 columns by 3 rows, with `options`.
fn create(host: &mut Host, kind: Kind, options: &str) -> Reply {
    host.call(PAGE, Op::UI_CREATE.number(), &[kind_number(kind), 2, 0, 12, 3], Payload::Text(options.as_bytes()))
}

fn call(host: &mut Host, op: Op, ints: &[i32]) -> Reply {
    host.call(PAGE, op.number(), ints, Payload::None)
}

/// A script's drawing with default attributes below `parent`.
fn drawing(host: &mut Host, parent: i32) -> Reply {
    call(host, Op::RG_DRAWING, &[parent, 0, 1, 0, 0, 0, -1, 1])
}

#[test]
fn a_scripts_own_operations_do_not_reach_a_components_drawings() {
    let mut host = host();
    assert!(matches!(create(&mut host, Kind::Button, r#"{"text":"Go"}"#), Reply::Int(_)));
    // The button's drawings are the first the page was given: 1 to 5.
    let numbers: Vec<u16> = host.regis.ui.nodes.iter().map(|node| node.number).collect();
    assert_eq!(numbers, vec![1, 2, 3, 4, 5]);
    let refusal = Reply::Type("the drawing is a part of a component: it is changed through nt.ui".to_owned());
    for number in 1..=5 {
        assert_eq!(call(&mut host, Op::RG_SET, &[number, Property::X.number(), 50]), refusal);
        assert_eq!(call(&mut host, Op::RG_REMOVE, &[number]), refusal);
        assert_eq!(call(&mut host, Op::RG_GET, &[number, Property::X.number()]), refusal);
        assert_eq!(drawing(&mut host, number), refusal, "a script's drawing does not hang from a component's");
    }
    // The script's own drawing is the next number, and is its own.
    let Reply::Int(own) = drawing(&mut host, 0) else { panic!("the script's drawing is made") };
    assert_eq!(own, 6);
    assert_eq!(call(&mut host, Op::RG_SET, &[own, Property::X.number(), 50]), Reply::Undefined);
    // Clearing deletes the script's drawings alone.
    assert!(matches!(call(&mut host, Op::RG_CLEAR, &[]), Reply::Text(_)));
    assert!(String::from_utf8_lossy(&host.regis.out).ends_with("D(K6)"), "{}", String::from_utf8_lossy(&host.regis.out));
    assert_eq!(host.regis.drawings.len(), 5);
    // What was staged is what the renderer's own validator takes.
    host.regis_commit();
    assert!(host.regis.failures.iter().all(Option::is_none) && host.regis.ready.is_some());
}

#[test]
fn a_component_the_page_has_no_room_for_changes_nothing() {
    let mut host = host();
    assert!(matches!(create(&mut host, Kind::Button, r#"{"text":"Go"}"#), Reply::Int(_)));
    // The script takes all but ten of the page's drawings.
    let share = REGIS_MAX_DRAWINGS - CHANNEL_SHARE_DRAWINGS;
    while host.regis.drawings.len() < share - 10 {
        assert!(matches!(drawing(&mut host, 0), Reply::Int(_)));
    }
    let before = (host.regis.drawings.clone(), host.regis.out.clone(), host.regis.contents, host.regis.ui.nodes.clone(), host.budget(ContextId::Page));
    // A menu of ten rows wants twenty-seven drawings: the eleventh is
    // refused, by ReGIS's own bound, and the ten made are taken back.
    let items = (0..10).map(|n| format!(r#"{{"text":"item {n}"}}"#)).collect::<Vec<_>>().join(",");
    let Reply::Range(message) = create(&mut host, Kind::Menu, &format!(r#"{{"items":[{items}]}}"#)) else { panic!("the menu is refused") };
    assert!(message.starts_with("REGIS_MAX_DRAWINGS: "), "{message}");
    let after = (host.regis.drawings.clone(), host.regis.out.clone(), host.regis.contents, host.regis.ui.nodes.clone(), host.budget(ContextId::Page));
    assert!(before == after, "a refused component changed the page's drawings, its staged operations or its budget");
    // A component that fits is still made.
    assert!(matches!(create(&mut host, Kind::Text, r#"{"text":"fits"}"#), Reply::Int(_)));
    assert_eq!(host.regis.drawings.len(), share - 9);
    host.regis_commit();
    assert!(host.regis.failures.iter().all(Option::is_none) && host.regis.ready.is_some());
}
