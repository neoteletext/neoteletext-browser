//! The sixel bitmap as a script sees it (script-api.md section 5.7; the
//! owner's DEC-7245; work package T3): JavaScript run in the container,
//! its batches given to the engine as a decoder would give them, and what
//! a receiver of Level 1 and of Level 2.5 then shows compared, sixel for
//! sixel, with what the script drew and with kept pictures. The hostile
//! scripts that attack the bitmap are here too (TODO-7336), and one timed
//! full-page animation through the script API (TODO-7297 step 4).
mod common;
#[allow(dead_code, reason = "the teletext tests' receiver, of which this file uses the grid and not the text picture")]
#[path = "teletext/receiver.rs"]
mod receiver;

use std::fmt::Write as _;
use std::time::Duration;

use common::{Opens as _, assert_checks, container, limited, lines, opening, script, tick, timed, view};
use neoteletext_core::charset::to_unicode;
use neoteletext_core::config::PresentationLevel;
use neoteletext_core::level1::{Glyph, Grid};
use neoteletext_script::limits::{CALLBACK_TIME_LIMIT, MAX_HOST_STATE_BYTES, STEP_TIME_BUDGET, TICKS_PER_SECOND, TIME_CHECK_MARGIN};
use neoteletext_script::teletext::limits::LOCAL_TRIPLETS;
use protocol::script::batch::FIRST_SCRIPT_ROW;
use protocol::script::{Batch, ContextId, FailReason, Level};
use receiver::Receiver;

/// The kept pictures' format: raised, with the pictures made again, by
/// any change to what a receiver shows for the same script.
const PICTURE_FORMAT: u32 = 1;

/// The bitmap's size, as section 5.7 gives it: 80 by 69 sixels over
/// columns 0 to 39 and rows 1 to 23, two a cell across and three down.
const WIDTH: usize = 80;
const HEIGHT: usize = 69;
const ACROSS: usize = 2;
const DOWN: usize = 3;
const COLUMNS: usize = WIDTH / ACROSS;
const ROWS: usize = HEIGHT / DOWN;

/// What a receiver shows in one cell of the bitmap's rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shown {
    /// Six sixels, left to right and top to bottom, each a Colour Map
    /// entry.
    Sixels([u8; ACROSS * DOWN]),
    /// A character, its foreground and its background.
    Character(char, u8, u8),
}

/// What a receiver's grid shows on rows 1 to 23, cell by cell.
fn shown(grid: &Grid) -> Vec<[Shown; COLUMNS]> {
    grid.cells
        .iter()
        .skip(usize::from(FIRST_SCRIPT_ROW))
        .take(ROWS)
        .map(|cells| {
            let mut row = [Shown::Sixels([0; ACROSS * DOWN]); COLUMNS];
            for (slot, cell) in row.iter_mut().zip(cells) {
                *slot = match cell.glyph {
                    // Bit k is sixel k (EN 300 706 clause 15.6.1; the
                    // engine's REQ-CHR-005).
                    Glyph::Mosaic { pattern, .. } => {
                        let mut sixels = [cell.background; ACROSS * DOWN];
                        for (bit, sixel) in sixels.iter_mut().enumerate() {
                            if pattern >> bit & 1 == 1 {
                                *sixel = cell.foreground;
                            }
                        }
                        Shown::Sixels(sixels)
                    }
                    glyph if glyph.is_blank() => Shown::Sixels([cell.background; ACROSS * DOWN]),
                    Glyph::Char(character) => Shown::Character(to_unicode(character).base, cell.foreground, cell.background),
                    Glyph::Space => Shown::Sixels([cell.background; ACROSS * DOWN]),
                };
            }
            row
        })
        .collect()
}

/// What a receiver shows as text a test keeps: 69 lines of 80, each
/// sixel's Colour Map entry in base 32. A cell that shows a character is
/// the character and `'` on its first line, `^` and its foreground on its
/// second, `_` and its background on its third.
fn picture(grid: &Grid) -> String {
    let digit = |entry: u8| char::from_digit(u32::from(entry), 32).unwrap();
    let mut out = String::new();
    for row in shown(grid) {
        for line in 0..DOWN {
            for cell in row {
                match cell {
                    Shown::Sixels(sixels) => out.extend([digit(sixels[line * ACROSS]), digit(sixels[line * ACROSS + 1])]),
                    Shown::Character(character, ink, paper) => out.extend([[character, '\''], ['^', digit(ink)], ['_', digit(paper)]][line]),
                }
            }
            out.push('\n');
        }
    }
    out
}

/// Compares `pictures` with the kept file `name` beside this test, or
/// writes the file when `NT_BLESS` is set.
#[track_caller]
fn expect_pictures(name: &str, pictures: &[(String, String)]) {
    let mut text = format!("format {PICTURE_FORMAT}\n");
    for (title, picture) in pictures {
        let _ = write!(text, "\n== {title}\n{picture}");
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/bitmap").join(name);
    if std::env::var_os("NT_BLESS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("no kept pictures at {}: {error}", path.display()));
    assert!(kept == text, "what the receiver shows is not the kept picture {}:\n{text}", path.display());
}

/// The opening of a teletext page shown at `level`, a tick on every tick.
fn opening_at(level: Level) -> protocol::script::ContextOpen {
    let mut open = opening(ContextId::Page);
    open.level = level;
    open.tick_divisor = 1;
    open
}

/// The receiver's level for a context's.
fn receiver_level(level: Level) -> PresentationLevel {
    match level {
        Level::One => PresentationLevel::Level1,
        Level::OneFive => PresentationLevel::Level15,
        Level::TwoFive => PresentationLevel::Level25,
        Level::ThreeFive => PresentationLevel::Level35,
    }
}

/// The triplets a batch stages, none when it stages no enhancement data.
fn staged_triplets(batch: &Batch) -> usize {
    batch.teletext_enhancement.as_ref().map_or(0, Vec::len)
}

/// Section 5.7 through `nt.teletext.bitmap`: each call's result and each
/// refusal's kind and bound, as JavaScript is given them.
#[test]
fn the_bitmap_api_as_a_script_sees_it() {
    let mut container = container();
    let source = script(include_str!("bitmap/api.js"));
    container.open_with(opening(ContextId::Page), source.as_bytes()).expect("the page opens");
    let first = container.step(&tick(0, 0)).unwrap();
    assert_checks(&lines(&first), 68);
}

/// What `figure.js` draws, made here the same way: each sixel's colour,
/// and the cells its text reserves.
struct Figure {
    sixels: [[u8; WIDTH]; HEIGHT],
    text: Vec<(usize, usize, char, u8, u8)>,
}

impl Figure {
    fn rect(&mut self, (x, y): (usize, usize), (width, height): (usize, usize), colour: u8) {
        for line in &mut self.sixels[y..y + height] {
            line[x..x + width].fill(colour);
        }
    }

    /// The figure of `figure.js` for a receiver of Level 1 (`amber` none)
    /// or of Level 2.5 (`amber` the entry the script was given).
    fn new(amber: Option<u8>) -> Self {
        let mut figure = Self { sixels: [[0; WIDTH]; HEIGHT], text: Vec::new() };
        figure.rect((4, 6), (20, 12), 1);
        figure.rect((2, 68), (78, 1), 7);
        if let Some(amber) = amber {
            figure.rect((40, 30), (17, 10), amber);
            figure.rect((60, 3), (6, 6), 4);
            figure.rect((62, 5), (2, 2), 6);
        }
        let (ink, paper) = if amber.is_some() { (3, 4) } else { (2, 0) };
        figure.text = (10..).zip("SIXELS".chars()).map(|(column, character)| (12, column, character, ink, paper)).collect();
        figure
    }

    /// What a receiver that shows the figure exactly shows in each cell.
    fn cells(&self) -> Vec<[Shown; COLUMNS]> {
        (0..ROWS)
            .map(|row| {
                core::array::from_fn(|column| {
                    if let Some((_, _, character, ink, paper)) =
                        self.text.iter().find(|(text_row, text_column, ..)| (*text_row, *text_column) == (row + 1, column))
                    {
                        return Shown::Character(*character, *ink, *paper);
                    }
                    Shown::Sixels(core::array::from_fn(|sixel| self.sixels[row * DOWN + sixel / ACROSS][column * ACROSS + sixel % ACROSS]))
                })
            })
            .collect()
    }
}

/// Asserts a receiver shows `wanted`, naming the first cell that differs.
#[track_caller]
fn assert_shows(grid: &Grid, wanted: &[[Shown; COLUMNS]]) {
    let got = shown(grid);
    for (row, (got, wanted)) in got.iter().zip(wanted).enumerate() {
        for (column, (got, wanted)) in got.iter().zip(wanted).enumerate() {
            assert_eq!(got, wanted, "row {}, column {column}; the receiver shows:\n{}", row + 1, picture(grid));
        }
    }
}

/// Section 5.7 and DEC-7245, "a double buffered sixel bitmap which gets
/// transformed into characters": nothing a script draws is shown before
/// `present()`; what is then shown is, sixel for sixel and character for
/// character, what the script drew, on the engine's own receiver at the
/// level the page is shown at, when no cell asks for more than its level
/// can show (`lossy` is 0). At Level 2.5 that takes X/26 triplets, the
/// script's defined colour among them; at Level 1 it takes none. The
/// overlay lies over the bitmap: its cells show its own characters and
/// colours and the bitmap's cells after them are as they were.
#[test]
fn what_a_script_draws_is_what_a_receiver_shows_once_presented() {
    for level in [Level::One, Level::TwoFive] {
        let level_1 = level == Level::One;
        let mut container = container();
        let source = format!("const LEVEL_1 = {level_1};\n{}", include_str!("bitmap/figure.js"));
        // No tick yet: the script has drawn, and nothing leaves.
        let mut open = opening_at(level);
        open.tick_divisor = 0;
        container.open_with(open, source.as_bytes()).expect("the page opens");
        let drawn = container.step(&common::with_events(0, 0, Vec::new())).unwrap();
        assert_eq!((&drawn.teletext_rows, &drawn.teletext_enhancement), (&None, &None), "{level:?}: shown before present()");
        drop(container);

        let mut container = common::container();
        container.open_with(opening_at(level), source.as_bytes()).expect("the page opens");
        let mut receiver = Receiver::new("en");
        let first = container.step(&tick(0, 0)).unwrap();
        receiver.take(&first);
        let said = lines(&first);
        let numbers: Vec<usize> = said[0].strip_prefix("first ").unwrap().split(' ').map(|number| number.parse().unwrap()).collect();
        let [cells, lossy, used] = numbers[..] else { panic!("{said:?}") };
        assert_eq!(lossy, 0, "{level:?}: {said:?}");
        assert_eq!(used, staged_triplets(&first), "{level:?}: the budget's triplets are the enhancement data staged");
        let figure = Figure::new((!level_1).then_some(16));
        let wanted = figure.cells();
        // Every cell that is not black is a cell that changed.
        let changed = wanted.iter().flatten().filter(|cell| **cell != Shown::Sixels([0; ACROSS * DOWN])).count();
        assert_eq!(cells, changed, "{level:?}");
        assert_shows(&receiver.grid_at(receiver_level(level)), &wanted);
        if level_1 {
            assert_eq!(used, 0, "Level 1 takes no triplet");
            continue;
        }
        assert!(used > 0 && used <= LOCAL_TRIPLETS, "{used} triplets");

        // The overlay over two cells of the red block.
        receiver.take(&container.step(&tick(1, 1)).unwrap());
        let over = container.step(&tick(2, 2)).unwrap();
        assert_eq!(over.teletext_rows, None, "the overlay touches no row");
        let used: usize = lines(&over)[0].strip_prefix("overlay ").unwrap().parse().unwrap();
        assert_eq!(used, staged_triplets(&over));
        receiver.take(&over);
        let mut wanted = wanted;
        wanted[3][5] = Shown::Character('O', 7, 2);
        wanted[3][6] = Shown::Character('K', 7, 2);
        assert_shows(&receiver.grid(), &wanted);
    }
}

/// TODO-7297's "done": a bitmap animation at a Level 1 and at a Level
/// 2.5 setting, with kept pictures of what the engine's receiver shows
/// at its first tick and half a second later, and the cells each present
/// changed and could not show. The pictures are made by the engine in
/// this test, as the teletext API's are (DEC-7360 item 18).
#[test]
fn a_bitmap_animation_at_level_1_and_at_level_2_5_is_its_kept_pictures() {
    const LATER: u64 = TICKS_PER_SECOND / 2;
    let mut pictures = Vec::new();
    for (level, name) in [(Level::One, "Level 1"), (Level::TwoFive, "Level 2.5")] {
        let mut container = container();
        container.open_with(opening_at(level), include_bytes!("bitmap/animation.js")).expect("the animation opens");
        let mut receiver = Receiver::new("en");
        let mut said = String::new();
        for at in 0..=LATER {
            let batch = container.step(&tick(u32::try_from(at).unwrap(), at)).unwrap();
            let log = lines(&batch);
            assert_eq!(log.len(), 1, "{name}, tick {at}: {log:?}");
            assert!(staged_triplets(&batch) <= LOCAL_TRIPLETS);
            if level == Level::One {
                assert_eq!(batch.teletext_enhancement, None, "Level 1 stages no enhancement data");
            }
            let _ = writeln!(said, "{}", log[0]);
            receiver.take(&batch);
            if at == 0 || at == LATER {
                pictures.push((format!("{name}, tick {at}"), picture(&receiver.grid_at(receiver_level(level)))));
            }
        }
        pictures.push((format!("{name}: the tick, the cells changed and the cells that lost detail"), said));
    }
    expect_pictures("animation.expect.txt", &pictures);
}

/// Section 5.7, "within what `LOCAL_TRIPLETS` leaves", section 10.1 rule
/// 7 and the security review's lead on host memory (TODO-7272 as
/// TODO-7336 gives it to the renderer packages): a page that fights for
/// the page's triplets and rows on every tick, by the bitmap, the
/// overlay, the controls, the page plane and the settings at once, never
/// stages more than the page's 208 local triplets, makes the host hold
/// no more than a page, and every batch it causes is one the link and
/// the engine's receiver take.
#[test]
fn a_hostile_page_cannot_pass_the_pages_triplets_through_the_bitmap() {
    const TICKS: u64 = 12;
    let mut container = container();
    container.open_with(opening_at(Level::TwoFive), include_bytes!("bitmap/hostile_budget.js")).unwrap();
    let before = container.host_bytes();
    let mut receiver = Receiver::new("en");
    let mut most = 0;
    for at in 0..TICKS {
        let batch = container.step(&tick(u32::try_from(at).unwrap(), at)).unwrap();
        most = most.max(staged_triplets(&batch));
        assert!(staged_triplets(&batch) <= LOCAL_TRIPLETS, "tick {at}: {} triplets staged", staged_triplets(&batch));
        let _ = view(&batch);
        receiver.take(&batch);
        let _ = receiver.grid();
        let said = lines(&batch);
        let [line] = &said[..] else { panic!("tick {at}: {said:?}") };
        let used: usize = line.split(" used ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
        assert!(line.starts_with(&format!("tick {at} used ")) && used <= LOCAL_TRIPLETS && line.contains(" refused true "), "{line}");
        // The first tick turns substitution on before the bitmap has a
        // row, so the bitmap is refused its rows; from the second it
        // shows, and loses detail.
        assert_eq!(line.ends_with(" lossy true"), at > 0, "{line}");
    }
    assert!(most > LOCAL_TRIPLETS / 2, "the page was not held to nothing either: {most} triplets at most");
    // Controls are held in a list that grows to the page's sixteen.
    assert!(
        container.host_bytes() <= before + 4096 && container.host_bytes() <= MAX_HOST_STATE_BYTES,
        "{} bytes held, {before} before",
        container.host_bytes()
    );
    assert!(container.stopped().is_none());
}

/// Section 10.1 rules 1 to 4 for the bitmap's operations: a page that
/// gives every call whole numbers at the ends of their range, numbers
/// that are not whole, objects that would run code if read, and the
/// largest arrays, thousands of times a tick, is thrown a `RangeError`
/// or a `TypeError` each time or has bounded work done; no code of its
/// objects is run; nothing is staged, since nothing was presented; and
/// the host holds no more.
#[test]
fn a_hostile_pages_arguments_are_refused_or_bounded() {
    let mut container = container();
    container.open_with(opening_at(Level::TwoFive), include_bytes!("bitmap/hostile_arguments.js")).unwrap();
    let before = container.host_bytes();
    let mut calls = 0;
    for at in 0..3 {
        let (batch, took) = timed(|| container.step(&tick(at, u64::from(at))));
        let batch = batch.unwrap();
        let said = lines(&batch);
        let [line] = &said[..] else { panic!("tick {at}: {said:?}") };
        let numbers: Vec<u64> = line.split(' ').filter_map(|word| word.parse().ok()).collect();
        let [made, other, ran] = numbers[..] else { panic!("{line}") };
        assert_eq!((other, ran), (0, 0), "{line}");
        assert!(made > calls, "{line}");
        calls = made;
        println!("tick {at}: {made} hostile bitmap calls so far; the step took {:?} ({:?} running)", took.wall, took.running);
        assert_eq!((&batch.teletext_rows, &batch.teletext_enhancement, &batch.teletext_settings), (&None, &None, &None));
    }
    assert_eq!(container.host_bytes(), before);
    assert!(container.stopped().is_none());
}

/// Section 3.2 and section 10.1 rule 2: `present()` is bounded work, so
/// a callback that redraws and presents for ever is ended by the
/// callback's time limit as any other is, under the limits the browser's
/// script host runs with, no later than the margin and one present past
/// it.
#[test]
fn a_callback_that_presents_for_ever_is_stopped_at_the_callback_limit() {
    // The longest one present may take past the limit's own margin: three
    // conversions (the bitmap's `PRESENT_ATTEMPTS`), measured at under
    // 4 ms each optimised and under 60 ms each with debug assertions.
    let one_present = if cfg!(debug_assertions) { Duration::from_millis(180) } else { Duration::from_millis(12) };
    let mut container = limited();
    container.open_with(opening_at(Level::TwoFive), include_bytes!("bitmap/hostile_presents.js")).unwrap();
    let (batch, took) = timed(|| container.step(&tick(1, 0)));
    let batch = batch.expect("the step in which the container stopped is answered");
    println!("a callback that presents for ever was stopped after {:?} ({:?} running)", took.wall, took.running);
    assert!(took.wall >= CALLBACK_TIME_LIMIT, "stopped after {:?}, before its limit", took.wall);
    assert!(took.running <= CALLBACK_TIME_LIMIT + TIME_CHECK_MARGIN + one_present, "ran {:?} under a limit of {CALLBACK_TIME_LIMIT:?}", took.running);
    assert_eq!(view(&batch).states.iter().map(|state| state.stopped).collect::<Vec<_>>(), vec![Some(FailReason::Time)]);
    assert_eq!(container.stop_reason(), Some(FailReason::Time));
    assert!(staged_triplets(&batch) <= LOCAL_TRIPLETS);
}

/// TODO-7297 step 4 and DEC-7245, which asks for this figure: a
/// full-page animation at 60 ticks a second through the script API, every
/// cell of every row changing on every frame, at Level 1 and at Level
/// 2.5. Each step is one tick's callback: a whole-page `blit` and a
/// `present()`. The time of a step is printed, mean and worst, less the
/// time its thread waited for a processor; the mean must be within the
/// step's time budget, which is under a tick.
#[test]
#[cfg_attr(debug_assertions, ignore = "the budget is for an optimised build: run with --release")]
fn a_full_page_animation_through_the_script_api_keeps_to_the_steps_budget() {
    const FRAMES: u32 = 120;
    let tick_length = Duration::from_nanos(1_000_000_000 / TICKS_PER_SECOND);
    for level in [Level::One, Level::TwoFive] {
        let mut container = container();
        container.open_with(opening_at(level), include_bytes!("bitmap/timed.js")).expect("the animation opens");
        let (mut total, mut worst) = (Duration::ZERO, Duration::ZERO);
        let mut said = Vec::new();
        let mut rows = 0;
        for at in 0..FRAMES {
            let (batch, took) = timed(|| container.step(&tick(at, u64::from(at))));
            let batch = batch.unwrap();
            total += took.running;
            worst = worst.max(took.running);
            rows += batch.teletext_rows.as_ref().map_or(0, Vec::len);
            said.extend(lines(&batch));
        }
        let mean = total / FRAMES;
        println!(
            "{level:?}: {FRAMES} full-page frames through the script API: a step (blit and present) mean {mean:?}, worst {worst:?}; {} rows a frame; {said:?}; a tick is {tick_length:?}, the step's budget {STEP_TIME_BUDGET:?}",
            rows / usize::try_from(FRAMES).unwrap(),
        );
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(mean <= STEP_TIME_BUDGET, "{level:?}: a step takes {mean:?} on average, more than the step's budget of {STEP_TIME_BUDGET:?}");
    }
}
