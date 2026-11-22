//! Teletext sprites as a script sees them and as a receiver draws them
//! (script-api.md sections 5.3, 3.1, 3.7, 3.9, 5.8 and 10.1; work package
//! T2): JavaScript run in the container, its batches given to the engine
//! as a decoder would give them, with the sprites' Object page written by
//! the engine's own authoring code, and what a Level 2.5 receiver then
//! shows compared with a kept picture.
mod common;
#[allow(dead_code, reason = "the page's own receiver, of which the sprites' uses the packets and the picture's form")]
#[path = "teletext/receiver.rs"]
mod receiver;
#[path = "teletext/sprite_receiver.rs"]
mod sprite_receiver;

use std::fmt::Write as _;

use common::{Opens as _, all_lines, assert_checks, container, ints_event, key, lines, opening, plain_event, run, script, text_event, tick, view, with_events};
use neoteletext_rules::animate::{Operation, OperationKind};
use neoteletext_rules::table::Target;
use neoteletext_script::limits::MAX_HOST_STATE_BYTES;
use neoteletext_script::ops::PointerAction;
use neoteletext_script::teletext::sprites::limits::{MAX_ANIMATED_OBJECTS, MAX_CYCLING_OBJECTS, OBJECT_PAGE_TRIPLETS, PAGE_SHARE_SPRITES};
use protocol::script::batch::{Label, MAX_OBJECT_TRIPLETS};
use protocol::script::sprites::{PAGE_SHARE_OBJECT_SUBPAGES, object_place};
use protocol::script::{Batch, ContextFlags, ContextId, EventKind, Renderer};
use sprite_receiver::SpriteReceiver;

/// The kept pictures' format: raised, with the pictures made again, by
/// any change to what a receiver shows for the same script.
const PICTURE_FORMAT: u32 = 1;

/// Compares `pictures` with the kept file `name` beside this test, or
/// writes the file when `NT_BLESS` is set.
#[track_caller]
fn expect_pictures(name: &str, pictures: &[(String, String)]) {
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

/// The `pageshow` the decoder gives the channel context for the teletext
/// page 100, which has no sub-page.
fn page_shown() -> protocol::script::Event {
    text_event(EventKind::PageShow, [0x100, 0, i32::from(Renderer::Teletext.code()), 0], "https://example.test/chan/100.tti")
}

/// Section 5.8, the worked game example, exactly as the specification
/// gives it (work package T2's "done"), run for 600 ticks with scripted
/// keys: a boat steered between rocks. The sprites are the receiver's:
/// what the engine draws for the batches is the kept picture at three
/// moments; a boat's or a rock's move leaves as one animation operation
/// and never as rows or enhancement data; and the receiver had room for
/// every operation it was sent.
#[test]
fn the_worked_game_example_is_what_a_receiver_shows() {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.tick_divisor = 6;
    open.keys = vec!["ArrowLeft".into(), "ArrowRight".into(), " ".into()];
    container.open_with(open, include_bytes!("teletext/110.js")).expect("the example opens");
    let mut receiver = SpriteReceiver::new("en");
    let mut pictures = Vec::new();
    let mut said = Vec::new();
    let mut moves = 0_usize;
    let mut navigated = None;
    for at in 0..600_u32 {
        // The scripted keys: right for a second, left for three, right
        // again, and the space bar now and then, which starts the game
        // again once it is over.
        let events = match at {
            30 | 300 => vec![key(EventKind::KeyDown, "ArrowRight", 0)],
            90 | 420 => vec![key(EventKind::KeyUp, "ArrowRight", 0)],
            120 => vec![key(EventKind::KeyDown, "ArrowLeft", 0)],
            299 => vec![key(EventKind::KeyUp, "ArrowLeft", 0)],
            200 | 400 | 599 => vec![key(EventKind::KeyDown, " ", 0)],
            _ => Vec::new(),
        };
        let batch = container.step(&with_events(at, u64::from(at), events)).unwrap();
        said.extend(lines(&batch));
        if at > 0 {
            assert_eq!(batch.teletext_enhancement, None, "tick {at}: a sprite takes none of the page's local triplets");
            assert!(batch.teletext_rows.iter().flatten().all(|row| row.row == 1), "tick {at}: only the score's row is written");
        }
        moves += batch.animate.iter().flatten().filter(|operation| matches!(operation.kind, OperationKind::Move { .. })).count();
        navigated = navigated.or(batch.navigate.clone().map(|target| (at, target)));
        receiver.take(&batch, at);
        if matches!(at, 0 | 149 | 299 | 449 | 599) {
            pictures.push((format!("tick {at}"), receiver.picture(at)));
        }
    }
    assert_eq!(said, Vec::<String>::new());
    assert_eq!(receiver.refused, Vec::<String>::new());
    assert!(moves > 100, "{moves} moves were the receiver's to make");
    println!("the game: {moves} moves, navigated {navigated:?}, the receiver's states and cycles {:?}", receiver.states());
    expect_pictures("110.expect.txt", &pictures);
}

/// Section 5.3 through `nt.teletext`: each call's result and each
/// refusal's kind and bound, as JavaScript is given them; then what the
/// batch carries for it, and a sprite's events (sections 4.4 and 3.9).
#[test]
fn the_sprite_api_as_a_script_sees_it() {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.flags = ContextFlags { pointer_press: true, wheel: true, ..open.flags };
    let source = script(include_str!("teletext/sprites_api.js"));
    container.open_with(open, source.as_bytes()).expect("the page opens");
    let first = container.step(&tick(0, 0)).unwrap();
    assert_checks(&lines(&first), 59);
    let _ = view(&first);
    // Two sprites are left, declared as they stand: the boat (1) moved,
    // in the overlay's layer at the lowest order, and the flagship (2)
    // shown, with its cycle sent after the declaration. Their labels and
    // the wheel's list name them by their numbers.
    let declared = first.sprites.clone().unwrap();
    let seen: Vec<_> =
        declared.iter().map(|one| (one.number, one.row, one.column, one.layer, one.order, one.shown, one.detectable, one.width, one.height)).collect();
    assert_eq!(seen, vec![(1, 6, 6, 3, -128, true, false, 3, 2), (2, 5, 5, 2, 3, true, true, 2, 2)]);
    assert_eq!(declared[1].objects.len(), 3);
    assert_eq!(declared[1].objects[1], declared[1].objects[2], "frames that are the same are one object");
    let objects = first.teletext_objects.clone().unwrap();
    assert_eq!(objects.len(), 3);
    assert!(objects.iter().all(|object| object_place(object.number).unwrap().0 < PAGE_SHARE_OBJECT_SUBPAGES && object.triplets.len() <= MAX_OBJECT_TRIPLETS));
    let cycle = Operation { target: Target::object(2), kind: OperationKind::Cycle { first: 1, last: 2, period: 4, once: true } };
    assert_eq!(first.animate, Some(vec![cycle]));
    assert_eq!(first.labels, Some(vec![Label { handle: 1, label: "Boat".into() }]));
    assert_eq!(first.watch.as_ref().and_then(|watch| watch.wheel.clone()), Some(vec![2]));
    assert_eq!((&first.teletext_enhancement, &first.teletext_rows), (&None, &None));

    // What a receiver shows: the flagship's second frame over the boat.
    let mut receiver = SpriteReceiver::new("en");
    receiver.take(&first, 0);
    let picture = receiver.picture(0);
    assert_eq!(receiver.refused, Vec::<String>::new());
    assert!(picture.contains("05 text |     #  "), "{picture}");

    // A release on the flagship is heard by it, then by nt, which is told
    // every thing under the pointer.
    let release = ints_event(EventKind::Pointer, [PointerAction::UP.number(), 5, 5, 2], &[1, 0, 0, 0, 1]);
    let pressed = container.step(&with_events(1, 1, vec![release])).unwrap();
    assert_eq!(lines(&pressed), vec!["sprite nt true 2"]);
    // The once cycle ends at its last frame: the decoder says so, and the
    // script's own count agrees.
    let mut ended = tick(2, 9);
    ended.cycle_ended = vec![2];
    let after = container.step(&ended).unwrap();
    assert_eq!(lines(&after), vec!["cycle ended at frame 2, cycling false"]);
    assert_eq!(receiver.frame(2, 9), 2);
    assert_eq!(after.sprites, None);
}

/// Section 3.1: a page with no script of its own still has the channel
/// context's sprites. The channel context draws them when it is shown
/// the page, in the overlay's layer alone and within its share, and they
/// are dropped when the page is hidden; nothing else of the page is its
/// to draw.
#[test]
fn the_channel_context_draws_its_sprites_on_a_page_with_no_script() {
    let mut container = container();
    let channel = script(
        r#"
        const tt = nt.teletext;
        throws("nothing to draw on before pageshow", TypeError, "the page shown is not a teletext page", () => tt.sprite({ frames: [tt.block(1, 1)], row: 1, column: 1 }));
        let badge;
        nt.addEventListener("pageshow", () => {
          const star = tt.block(1, 1).cell(0, 0, { char: "*", ink: 3, paper: 4 });
          badge = tt.sprite({ frames: [star], row: 1, column: 39, label: "Channel" });
          equal("the overlay's layer by default", badge.layer, 3);
          throws("the overlay's layer alone", TypeError, "the channel context draws in the overlay's layer, 3, alone", () => tt.sprite({ frames: [star], row: 1, column: 1, layer: 2 }));
          throws("and stays there", TypeError, "the channel context draws in the overlay's layer", () => { badge.layer = 1; });
          throws("the plane is the page's", TypeError, "only the page context draws", () => tt.page.put(1, 0, "x"));
          throws("controls are the page's", TypeError, "only the page context draws", () => tt.control(1, 0, 4));
          const more = [];
          for (let n = 0; n < 3; n++) more.push(tt.sprite({ frames: [star], row: 2, column: n }));
          throws("its share", RangeError, "CHANNEL_SHARE_SPRITES: the channel context has at most 4 of a page's 32 sprites", () => tt.sprite({ frames: [star], row: 1, column: 1 }));
          for (const one of more) one.remove();
          badge.moveBy(1, -1);
          done();
        });
        nt.addEventListener("pagehide", () => {
          throws("dropped with the page", TypeError, "the sprite was removed", () => badge.moveBy(0, 1));
          equal("no bounds", badge.bounds(), null);
          throws("nothing to draw on", TypeError, "the page shown is not a teletext page", () => tt.sprite({ frames: [tt.block(1, 1)], row: 1, column: 1 }));
          done();
        });
        done();
        "#,
    );
    container.open_with(opening(ContextId::Channel), channel.as_bytes()).unwrap();
    let opened = container.step(&tick(0, 0)).unwrap();
    assert_checks(&lines(&opened), 1);
    let shown = container.step(&with_events(1, 1, vec![page_shown()])).unwrap();
    assert_checks(&lines(&shown), 7);
    let declared = shown.sprites.clone().unwrap();
    assert_eq!(declared.iter().map(|one| (one.number, one.row, one.column, one.layer)).collect::<Vec<_>>(), vec![(1, 2, 38, 3)]);
    let objects = shown.teletext_objects.clone().unwrap();
    assert_eq!(objects.iter().map(|object| object_place(object.number).unwrap()).collect::<Vec<_>>(), vec![(PAGE_SHARE_OBJECT_SUBPAGES, 0)]);
    assert_eq!(shown.labels, Some(vec![Label { handle: 1, label: "Channel".into() }]));
    let mut receiver = SpriteReceiver::new("en");
    receiver.take(&shown, 1);
    let picture = receiver.picture(1);
    assert!(picture.contains(&format!("02 text |{}* |", " ".repeat(38))), "{picture}");
    assert_eq!(receiver.refused, Vec::<String>::new());

    let hidden = container.step(&with_events(2, 2, vec![plain_event(EventKind::PageHide, [0x100, 0, 0, 0])])).unwrap();
    assert_checks(&lines(&hidden), 10);
    // Shown the page again, it draws again, on a state of its own.
    let again = container.step(&with_events(3, 3, vec![page_shown()])).unwrap();
    assert_checks(&lines(&again), 16);
    assert_eq!(again.sprites.as_ref().map(Vec::len), Some(1));
}

/// Section 3.1: the two levels on one page. Each has its share of the
/// page's sprites and of the Object page, neither can name the other's
/// sprite, and the page context's end leaves the channel context nothing
/// to draw on until it is shown a page again.
#[test]
fn the_two_levels_share_a_pages_sprites_without_reaching_each_others() {
    let mut container = container();
    let channel = script(
        r#"
        nt.addEventListener("pageshow", () => {
          const made = nt.teletext.sprite({ frames: [nt.teletext.block(1, 1).cell(0, 0, { char: "c" })], row: 3, column: 3 });
          nt.shared.set("channel", 1);
          equal("the page's sprites are counted with its own", nt.budget().sprites.used, 3);
          done();
        });
        "#,
    );
    let page = script(
        r#"
        const tt = nt.teletext;
        const p = tt.block(1, 1).cell(0, 0, { char: "p" });
        const one = tt.sprite({ frames: [p], row: 2, column: 2 });
        const two = tt.sprite({ frames: [p], row: 2, column: 4, layer: 3 });
        equal("numbers rise", JSON.stringify(nt.budget().sprites), '{"used":2,"limit":32}');
        nt.addEventListener("tick", () => {
          const made = [];
          try { for (;;) made.push(tt.sprite({ frames: [p], row: 5, column: 5 })); } catch (error) { equal("refused by name", error.message.split(":")[0], "MAX_SPRITES"); }
          equal("the page's share with the channel's one beside it", made.length + 2, 28);
          equal("all of them", nt.budget().sprites.used, 29);
          done();
        });
        done();
        "#,
    );
    container.open_with(opening(ContextId::Channel), channel.as_bytes()).unwrap();
    let mut open = opening(ContextId::Page);
    open.tick_divisor = 1;
    container.open_with(open, page.as_bytes()).unwrap();
    let first = container.step(&with_events(0, 0, vec![page_shown()])).unwrap();
    assert_checks(&lines(&first), 6);
    let declared = first.sprites.clone().unwrap();
    assert_eq!(declared.len(), PAGE_SHARE_SPRITES + 1);
    let places: Vec<usize> = first.teletext_objects.iter().flatten().map(|object| object_place(object.number).unwrap().0).collect();
    assert_eq!(places, vec![0, PAGE_SHARE_OBJECT_SUBPAGES], "each level's frames are on its own sub-pages");
    let mut receiver = SpriteReceiver::new("en");
    receiver.take(&first, 0);
    let picture = receiver.picture(0);
    assert!(picture.contains("02 text |  p p") && picture.contains("03 text |   c"), "{picture}");
    assert_eq!(receiver.refused, Vec::<String>::new());
}

/// The security review's leads on host memory and on bounds (TODO-7272,
/// as TODO-7336 gives them to this work package; section 10.1 rules 3, 4
/// and 7): a page that asks on every tick for far more sprites, frames
/// and Object page than a page has, and works dead handles and values
/// past every range, is refused each by name; the host holds no more for
/// it than a page's sprites and one Object page; every batch is a message
/// the link takes; the Object page the engine writes from it always
/// fits; and the receiver has room for every operation it is sent.
#[test]
fn a_hostile_page_cannot_pass_the_sprites_or_the_object_page_or_grow_the_host() {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.tick_divisor = 1;
    container.open_with(open, include_bytes!("teletext/hostile_sprites.js")).unwrap();
    let before = container.host_bytes();
    let mut receiver = SpriteReceiver::new("en");
    let mut said = Vec::new();
    for at in 0..12_u32 {
        let batch = container.step(&tick(at, u64::from(at))).unwrap();
        let _ = view(&batch);
        assert!(batch.sprites.as_ref().is_none_or(|sprites| sprites.len() <= PAGE_SHARE_SPRITES));
        let triplets: usize = batch.teletext_objects.iter().flatten().map(|object| object.triplets.len() + 2).sum();
        assert!(triplets <= OBJECT_PAGE_TRIPLETS, "{triplets} triplets of objects");
        receiver.take(&batch, at);
        let _ = receiver.grid(at);
        let (states, cycling) = receiver.states();
        assert!(states <= MAX_ANIMATED_OBJECTS && cycling <= MAX_CYCLING_OBJECTS, "tick {at}: {states} states, {cycling} cycling");
        said.extend(lines(&batch));
    }
    assert_eq!(receiver.refused, Vec::<String>::new());
    assert_eq!(said.len(), 12, "{said:#?}");
    let last = said.last().unwrap();
    for bound in [
        "MAX_SPRITES",
        "OBJECT_BODY_TRIPLETS",
        "OBJECT_PAGE_TRIPLETS",
        "MAX_OBJECT_FRAMES",
        "MAX_SPRITE_ROW",
        "MAX_CYCLING_OBJECTS",
        "MAX_WHEEL_THINGS",
        "LAST_LAYER",
        "SPRITE_ORDER",
        "TypeError",
    ] {
        assert!(last.contains(&format!("\"{bound}\":")), "nothing was refused by {bound}: {last}");
    }
    println!("{last}");
    assert!(
        container.host_bytes() <= before + 4096 && container.host_bytes() <= MAX_HOST_STATE_BYTES,
        "{} bytes held, {before} before",
        container.host_bytes()
    );
    assert!(container.stopped().is_none());
}

/// ANIM-6 as section 5.3 surfaces it: a page's sprites and controls share
/// the receiver's 32 animation states, and 16 may cycle. A page that asks
/// for more is refused by name, and the engine, given the batches, never
/// meets an operation it has no state or no cycle for, though sprites are
/// made and removed throughout.
#[test]
fn a_hostile_page_cannot_ask_the_receiver_for_more_animation_state_than_it_has() {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.tick_divisor = 1;
    container.open_with(open, include_bytes!("teletext/hostile_states.js")).unwrap();
    let mut receiver = SpriteReceiver::new("en");
    let batches: Vec<Batch> = run(&mut container, 0, 40).into_iter().map(Result::unwrap).collect();
    let mut most = (0, 0);
    for (at, batch) in (0_u32..).zip(&batches) {
        let _ = view(batch);
        receiver.take(batch, at);
        let (states, cycling) = receiver.states();
        most = (most.0.max(states), most.1.max(cycling));
    }
    assert_eq!(receiver.refused, Vec::<String>::new());
    let said = all_lines(&batches.into_iter().map(Ok).collect::<Vec<_>>());
    let last = said.last().unwrap();
    assert!(last.contains("\"MAX_ANIMATED_OBJECTS\":") && last.contains("\"MAX_CYCLING_OBJECTS\":"), "{last}");
    assert!(most.0 <= MAX_ANIMATED_OBJECTS && most.1 == MAX_CYCLING_OBJECTS, "the receiver held {most:?} states and cycles at most");
    println!("{last}; the receiver held {most:?} states and cycles at most");
}
