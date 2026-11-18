//! The decoder process itself: what the trusted core started it for, the
//! loop that feeds the engine a channel's bytes and sends the pages shown,
//! and the reader of a bookmark file. `main.rs` confines the process and
//! calls these.

use std::collections::BTreeMap;
use std::io::{self, BufReader, BufWriter, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::{Engine, Flashing, Rendered};
use protocol::ProtocolError;
use protocol::decoder::{FIRST_STORE_PAGES, FromDecoder, MAX_BOOKMARK_FILE, MAX_STORE_PAGES, ToDecoder, read_from_core, write_frame_to_core, write_to_core};

/// Microseconds in a millisecond: the audio receiver keeps milliseconds.
const MICROSECONDS_PER_MILLISECOND: u64 = 1000;

type Output<'a> = BufWriter<io::StdoutLock<'a>>;

/// What the trusted core started the decoder for.
pub enum Mode {
    /// A channel, with a store of this many pages.
    Channel(usize),
    /// To read one bookmark file (decision 71).
    Bookmarks,
    /// To convert one picture to a page (`docs/picture-tools.md`).
    Picture,
}

/// The mode the trusted core asked for: a channel's store size,
/// `--store-pages N` from [`FIRST_STORE_PAGES`] to [`MAX_STORE_PAGES`], or
/// `--bookmarks`.
///
/// # Errors
/// The usage, when the arguments are none of these.
pub fn mode() -> Result<Mode, String> {
    let mut arguments = std::env::args().skip(1);
    match (arguments.next().as_deref(), arguments.next().map(|value| value.parse::<usize>())) {
        (None, _) => Ok(Mode::Channel(FIRST_STORE_PAGES)),
        (Some("--store-pages"), Some(Ok(pages))) if (FIRST_STORE_PAGES..=MAX_STORE_PAGES).contains(&pages) => Ok(Mode::Channel(pages)),
        (Some("--bookmarks"), None) => Ok(Mode::Bookmarks),
        (Some("--picture"), None) => Ok(Mode::Picture),
        _ => Err(format!("usage: neoteletext-decoder [--store-pages {FIRST_STORE_PAGES} to {MAX_STORE_PAGES} | --bookmarks | --picture]")),
    }
}

/// Reads a bookmark file the trusted core sends as data, to its end, and
/// reports what it holds (decision 71).
///
/// # Errors
/// When the pipe fails, or the trusted core sends anything but the file.
pub fn read_bookmarks() -> Result<(), ProtocolError> {
    let mut input = BufReader::new(io::stdin().lock());
    let mut file: Vec<u8> = Vec::new();
    loop {
        match read_from_core(&mut input)? {
            Some(ToDecoder::Data(bytes)) => {
                if file.len().saturating_add(bytes.len()) > MAX_BOOKMARK_FILE {
                    return Err(ProtocolError::TooLong(file.len().saturating_add(bytes.len())));
                }
                file.extend_from_slice(&bytes);
            }
            Some(ToDecoder::End) => break,
            Some(_) => return Err(ProtocolError::BadValue("a bookmark reader takes only data")),
            None => return Ok(()),
        }
    }
    let mut output = BufWriter::new(io::stdout().lock());
    match crate::netscape::read(&file) {
        Ok(found) => {
            for entry in found.entries {
                write_to_core(&mut output, &FromDecoder::Bookmark(entry))?;
            }
            write_to_core(&mut output, &FromDecoder::BookmarksRead { skipped: found.skipped })?;
        }
        Err(not_read) => eprintln!("neoteletext-decoder: not a bookmark file this reader reads: {not_read:?}"),
    }
    output.flush()?;
    Ok(())
}

/// Converts one picture the trusted core sends: its choices, then the
/// picture as data to its end, at most the engine's `MAX_INPUT_BYTES`; then
/// sends the page made, or why there is none, and ends.
///
/// # Errors
/// When the pipe fails, or the trusted core sends anything else.
pub fn convert_picture() -> Result<(), ProtocolError> {
    let mut input = BufReader::new(io::stdin().lock());
    let Some(ToDecoder::Picture(choice)) = read_from_core(&mut input)? else {
        return Err(ProtocolError::BadValue("a picture converter takes its choices first"));
    };
    let mut picture: Vec<u8> = Vec::new();
    loop {
        match read_from_core(&mut input)? {
            Some(ToDecoder::Data(bytes)) => {
                if picture.len().saturating_add(bytes.len()) > neoteletext_picture::MAX_INPUT_BYTES {
                    return Err(ProtocolError::TooLong(picture.len().saturating_add(bytes.len())));
                }
                picture.extend_from_slice(&bytes);
            }
            Some(ToDecoder::End) => break,
            Some(_) => return Err(ProtocolError::BadValue("a picture converter takes only data after its choices")),
            None => return Ok(()),
        }
    }
    let mut output = BufWriter::new(io::stdout().lock());
    write_to_core(&mut output, &crate::picture::converted(&picture, choice))?;
    output.flush()?;
    Ok(())
}

/// The receiver's figures as the protocol carries them: each saturates at
/// the field's bound, which no receiver this process builds reaches.
fn audio_receiver(limits: neoteletext_audio::ReceiverLimits) -> protocol::decoder::AudioReceiver {
    let figure = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
    protocol::decoder::AudioReceiver {
        store_bytes: figure(limits.store_bytes),
        objects: figure(limits.objects),
        records_in_flight: figure(limits.records_in_flight),
    }
}

/// A channel's decoder, with a store of `store_pages`: it reads the
/// channel's bytes and requests from the trusted core and sends back which
/// pages arrived and the pages shown, until the channel closes. Its first
/// message says it is confined, as `main.rs` has made it.
///
/// # Errors
/// When the pipe fails or a message from the trusted core is malformed.
#[allow(clippy::too_many_lines, reason = "the one loop that takes every message of the pipe in its order, each arm a line or two")]
pub fn run(store_pages: usize) -> Result<(), ProtocolError> {
    let mut output = BufWriter::new(io::stdout().lock());
    // Before it reads anything, so the trusted core can tell a decoder
    // that could not start from one its channel's data ended (finding
    // F-12).
    write_to_core(&mut output, &FromDecoder::Confined)?;
    output.flush()?;
    let mut engine = Engine::with_store(store_pages);
    // The timed test's settings, for its figures before DEC-6580 (RULE-66).
    #[cfg(feature = "timed-test")]
    let mut held = timed::Held::new(&mut engine);
    write_to_core(&mut output, &FromDecoder::AudioReceiver(audio_receiver(engine.audio_limits())))?;
    let mut showing: Vec<u16> = Vec::new();
    // The clock the pages on screen were last drawn with.
    let mut shown_clock = None;
    // The last frame sent of each page shown, so an unchanged page is
    // never sent again.
    let mut sent: BTreeMap<u16, Rendered> = BTreeMap::new();
    // A page's worth of room for the next render, from the last frame not kept.
    let mut spare: Vec<u8> = Vec::new();
    let mut flashing = Flashing::default();
    // The pages on screen the trusted core was last told are missing.
    let mut missing: Vec<u16> = Vec::new();
    // The trusted core's messages are read on a thread of their own, so
    // the loop also wakes, by itself, for flash and for its sounds' clock:
    // it sleeps until a message, the next flash phase of a shown page
    // that flashes, or the audio receiver's next tick, whichever is first,
    // and with none of these, until a message (decision 47).
    let incoming = read_messages();

    // The paint point painted at in the last wake, and when on the
    // engine's own clock the painting began (RULE-98 item 5).
    let mut painting: Option<u64> = None;
    loop {
        // What the last message or deadline produced goes before waiting.
        output.flush()?;
        #[cfg(feature = "timed-test")]
        protocol::timed_test::flushed("decoder");
        if let Some(began) = painting.take() {
            engine.painted(began);
        }
        // RULE-98 item 3: the clock moves toward the interface's frames
        // before the loop waits for the next tick.
        engine.slew_clock();
        let now = engine.now_us();
        let deadline = next_deadline(&mut engine, &flashing, &showing, &sent, now);
        let received = match wait(&incoming, deadline, now) {
            Woken::Message(message) => Some(message),
            Woken::Deadline => None,
            Woken::Ended => return Ok(()),
        };
        // RULE-66's hops: how late the loop woke for its deadline.
        #[cfg(feature = "timed-test")]
        let woke_late_us = if received.is_none() { deadline.map(|deadline| engine.now_us().saturating_sub(deadline)) } else { None };
        #[cfg(feature = "timed-test")]
        let received = held.hold(&engine, received);
        let received = take_rule_message(&mut engine, received, &mut output)?;
        let now = engine.now_us();
        tick(&mut engine, now, &mut output)?;
        #[cfg(feature = "timed-test")]
        held.release(&mut engine, now, &mut output)?;
        // RULE-65: what changed the picture is painted at the next paint
        // point, just before one of the interface's compositions, so that
        // every change before it costs one repaint; with no `FramePhase`
        // in force, at once.
        let paint = engine.paint_due();
        if paint {
            painting = Some(engine.real_us());
            // RULE-66's hops: the paint, how long after its paint point,
            // and how late the loop woke.
            #[cfg(feature = "timed-test")]
            if let Some(point) = engine.paint_point_real_us() {
                let real = engine.real_us();
                protocol::timed_test::mark(
                    "decoder",
                    "paint",
                    std::time::Instant::now(),
                    format_args!("after-point {} woke-late {}", real.saturating_sub(point), woke_late_us.map_or(-1, |late| i64::try_from(late).unwrap_or(-1))),
                );
            }
            // The animation of the pages on screen, after the rules ran
            // (animation.md ANIM-22, page-rules.md RULE-65).
            let animated = animate(&mut engine, &showing, now, &mut output)?;
            #[cfg(feature = "timed-test")]
            protocol::timed_test::mark("decoder", "animated", std::time::Instant::now(), format_args!(""));
            draw(&engine, &animated, now, &mut sent, &mut spare, &mut flashing, &mut output)?;
            #[cfg(feature = "timed-test")]
            protocol::timed_test::mark("decoder", "drawn", std::time::Instant::now(), format_args!("{}", animated.len()));
        }
        let Some(message) = received else {
            // A deadline: the sounds' clock, and the flashing pages whose
            // phase has changed.
            send_sounds(&mut engine, &mut output)?;
            draw(&engine, &flashing.due(now, engine.config()), now, &mut sent, &mut spare, &mut flashing, &mut output)?;
            if paint {
                draw_regis(&mut engine, &showing, &sent, now, &mut output)?;
            }
            continue;
        };
        let Some(message) = message? else { return Ok(()) };
        let arrived = match message {
            ToDecoder::Data(bytes) => engine.feed(&bytes),
            ToDecoder::Whole { packets, pushed, bytes } => engine.feed_whole(packets, pushed, &bytes),
            ToDecoder::End => engine.finish(),
            ToDecoder::Showing(pages) => {
                sent.retain(|page, _| pages.contains(page));
                flashing.retain(&pages);
                showing = pages;
                engine.screen_changed();
                // A frame ends when its page stops being shown (ANIM-2).
                engine.animation_showing(&showing);
                let fresh: Vec<u16> = showing.iter().copied().filter(|page| !sent.contains_key(page)).collect();
                draw(&engine, &fresh, now, &mut sent, &mut spare, &mut flashing, &mut output)?;
                // A page that came on screen is sent its ReGIS or NAPLPS picture again.
                for page in &fresh {
                    engine.show_pictures_again(*page);
                }
                draw_regis(&mut engine, &showing, &sent, now, &mut output)?;
                say_missing(&engine, &showing, now, &mut missing, &mut output)?;
                continue;
            }
            ToDecoder::Close => return Ok(()),
            // A channel's decoder converts no picture: the trusted core
            // sends choices only to one started with `--picture`.
            ToDecoder::Picture(_) => return Err(ProtocolError::BadValue("a channel's decoder converts no picture")),
            ToDecoder::Region(region) => {
                engine.set_region(region);
                continue;
            }
            ToDecoder::Opened(page) => {
                engine.open_page(page);
                send_sounds(&mut engine, &mut output)?;
                continue;
            }
            // The open page's reveal, hold, mix or sub-page changed: drawn
            // again at once if on screen.
            ToDecoder::View(view) => {
                engine.set_view(view);
                if showing.contains(&view.page) {
                    draw(&engine, &[view.page], now, &mut sent, &mut spare, &mut flashing, &mut output)?;
                }
                continue;
            }
            // The sub-page list scrolled rows of a page into view.
            ToDecoder::SubPageRows { page, range } => {
                write_to_core(&mut output, &FromDecoder::SubPageRows { page, rows: engine.subpage_rows(page, range) })?;
                continue;
            }
            // What the viewer's pointer is doing to a ReGIS or NAPLPS
            // picture changed (REQ-6374; DEC-6638 item 2): repainted at
            // once if on screen and its budget allows. Only one of the two
            // holds the sub-page.
            ToDecoder::RegisViewer { page, sub, viewer } => {
                engine.regis().set_viewer(page, sub, viewer);
                engine.naplps().set_viewer(page, sub, viewer);
                // Nothing arrived; what is owed is drawn below, as after data.
                Vec::new()
            }
            // The viewer's setting "Paint drawings gradually" (ANIM-15):
            // off, what was being painted gradually is whole at the next
            // draw, below.
            ToDecoder::PaintGradually(gradual) => {
                engine.set_paint_gradually(gradual);
                Vec::new()
            }
            // The viewer's input for the open page's rule table, its
            // metered values and a state query (RULE-52, RULE-33,
            // RULE-47): each waits for the next tick.
            // Taken above, as each is read.
            ToDecoder::RuleInput { .. }
            | ToDecoder::Metered { .. }
            | ToDecoder::RulesQuery { .. }
            | ToDecoder::RulesPage(_)
            | ToDecoder::ConnectionEnded
            | ToDecoder::CueHeard { .. }
            | ToDecoder::FramePhase(_) => continue,
            // The look under the pointer changed: drawn again at once.
            #[cfg(feature = "links")]
            ToDecoder::Look { page, look } => {
                engine.set_look(page, look);
                if showing.contains(&page) {
                    draw(&engine, &[page], now, &mut sent, &mut spare, &mut flashing, &mut output)?;
                }
                continue;
            }
            // Built without links, the decoder draws no look; the message
            // exists when another crate of the build turned links on.
            #[cfg(not(feature = "links"))]
            #[allow(unreachable_patterns, reason = "reachable only when the protocol has links and this crate does not")]
            _ => continue,
        };
        report(&mut engine, &arrived, &mut output)?;
        let pages = changed_pages(&mut engine, &showing, &mut shown_clock, &mut output)?;
        draw(&engine, &pages, now, &mut sent, &mut spare, &mut flashing, &mut output)?;
        if paint {
            draw_regis(&mut engine, &showing, &sent, now, &mut output)?;
        }
        say_missing(&engine, &showing, now, &mut missing, &mut output)?;
    }
}

/// When the loop next wakes by itself: the next flash phase of a shown
/// page that flashes, the audio receiver's next tick, a ReGIS picture whose
/// repaint its budget held back (REQ-6380), the open page's next tick
/// while it has a rule table (page-rules.md RULE-58), or the next tick the
/// animation of a page on screen is due at (animation.md ANIM-1),
/// whichever is first.
/// RULE-96: a message of the open page's rule table, an input above all,
/// is handled the moment it is read, before a record waiting, a repaint or
/// a tick, and what it produced (a group's sound commands first) is
/// written and flushed at once. Any other message is given back.
fn take_rule_message(engine: &mut Engine, received: Option<Received>, output: &mut Output<'_>) -> Result<Option<Received>, ProtocolError> {
    let Some(Ok(Some(message))) = received else { return Ok(received) };
    match engine.rule_message(message) {
        Ok(produced) => {
            write_rule_messages(&produced, output)?;
            Ok(None)
        }
        Err(other) => Ok(Some(Ok(Some(other)))),
    }
}

/// Writes what the rule table produced, and flushes it: a group's sound
/// commands leave before any other work (RULE-96).
fn write_rule_messages(produced: &[FromDecoder], output: &mut Output<'_>) -> Result<(), ProtocolError> {
    for message in produced {
        write_to_core(output, message)?;
    }
    output.flush()?;
    // RULE-66: when the decoder wrote a local sound's MIDI message.
    #[cfg(feature = "timed-test")]
    for message in produced {
        if let FromDecoder::Audio(protocol::audio::Audio::LocalMidi { status, data1, .. }) = message {
            protocol::timed_test::mark("decoder", "local-midi", std::time::Instant::now(), format_args!("{status:02X} {data1}"));
        }
    }
    Ok(())
}

/// The timed test's settings in the decoder (RULE-66), only in its own
/// build.
#[cfg(feature = "timed-test")]
mod timed {
    use super::{Engine, Output, ProtocolError, Received, ToDecoder, write_rule_messages};

    /// "Events held for the tick": each input event waits for the tick
    /// due when it arrived, and is handled only after that tick's step,
    /// as before DEC-6580 item 1, so its sound and its picture both wait.
    pub(super) struct Held {
        on: bool,
        waiting: Vec<(u64, ToDecoder)>,
    }

    impl Held {
        /// The settings read; "the tick left unlocked" set on the engine.
        pub(super) fn new(engine: &mut Engine) -> Self {
            if protocol::timed_test::setting(protocol::timed_test::UNLOCKED) {
                engine.leave_unlocked();
            }
            Self { on: protocol::timed_test::setting(protocol::timed_test::EVENTS_AT_THE_TICK), waiting: Vec::new() }
        }

        /// Keeps an input event for the tick, when the setting is on.
        pub(super) fn hold(&mut self, engine: &Engine, received: Option<Received>) -> Option<Received> {
            match received {
                Some(Ok(Some(message @ ToDecoder::RuleInput { .. }))) if self.on => {
                    let due = engine.rules_due_us().unwrap_or_else(|| engine.now_us());
                    self.waiting.push((due, message));
                    None
                }
                other => other,
            }
        }

        /// Hands each kept event whose tick has run to the evaluator.
        pub(super) fn release(&mut self, engine: &mut Engine, now_us: u64, output: &mut Output<'_>) -> Result<(), ProtocolError> {
            while self.waiting.first().is_some_and(|(due, _)| *due <= now_us) {
                let (_, message) = self.waiting.remove(0);
                if let Ok(produced) = engine.rule_message(message) {
                    write_rule_messages(&produced, output)?;
                }
            }
            Ok(())
        }
    }
}

fn next_deadline(engine: &mut Engine, flashing: &Flashing, showing: &[u16], sent: &BTreeMap<u16, Rendered>, now: u64) -> Option<u64> {
    let audio = engine.next_audio_tick_ms().map(|ms| ms.saturating_mul(MICROSECONDS_PER_MILLISECOND));
    // A repaint owed waits for the next paint point (RULE-65).
    let paint = engine.next_paint_us();
    let at_paint = |due: Option<u64>| due.map(|due| paint.map_or(due, |paint| due.max(paint)));
    let animation = at_paint(engine.animation_due_us(showing, now));
    let regis = at_paint(regis_owed(engine, showing, sent, now));
    [flashing.next_us(now, engine.config()), audio, regis, engine.rules_due_us(), animation].into_iter().flatten().min()
}

/// What every wake does first: the sounds' clock, then the open page's
/// rule table for each tick due. What its rules produced is written at
/// once: a `midi` action's message is on its way to the sound helper
/// before this wake draws anything (RULE-64).
fn tick(engine: &mut Engine, now_us: u64, output: &mut Output<'_>) -> Result<(), ProtocolError> {
    engine.tick_audio(now_us.checked_div(MICROSECONDS_PER_MILLISECOND).unwrap_or(0));
    let produced = engine.run_rules(now_us);
    if produced.is_empty() {
        return Ok(());
    }
    write_rule_messages(&produced, output)
}

/// The pages on screen to draw again after data arrived: a shown page is
/// drawn again only when its reception moved, or data every page may draw
/// from changed, or the clock every page shows moved on from
/// `shown_clock` (the engine's decision E.121), or its sprites or its
/// animation state changed (animation.md ANIM-6). The trusted core is
/// told when the store is full.
fn changed_pages(
    engine: &mut Engine,
    showing: &[u16],
    shown_clock: &mut Option<[u8; neoteletext_core::consts::CLOCK_BYTES]>,
    output: &mut Output<'_>,
) -> Result<Vec<u16>, ProtocolError> {
    let changed = engine.take_changed();
    if engine.take_store_full() {
        write_to_core(output, &FromDecoder::StoreFull)?;
    }
    let clock = engine.latest_clock();
    let ticked = clock != *shown_clock;
    *shown_clock = clock;
    let animated = engine.take_animated(showing);
    Ok(showing.iter().copied().filter(|page| ticked || changed.everything || changed.pages.contains(page) || animated.contains(page)).collect())
}

/// Brings the animation of the pages on screen to the tick due at
/// `now_us` (animation.md ANIM-1, ANIM-22): cycles, the paint rate, WAIT
/// and BLINK, and the frames that end by the clock (ANIM-2), whose pages
/// are reported as arrived. It runs after the open page's rules, so that
/// what a rule changed is painted in the tick that ran it (page-rules.md
/// RULE-65). The teletext pages on screen to draw again: those that
/// arrived and those whose sprites or animation state changed.
fn animate(engine: &mut Engine, showing: &[u16], now_us: u64, output: &mut Output<'_>) -> Result<Vec<u16>, ProtocolError> {
    let late = engine.animation_step(showing, now_us);
    report(engine, &late, output)?;
    let mut pages: Vec<u16> = showing.iter().copied().filter(|page| late.iter().any(|(arrived, _)| arrived == page)).collect();
    pages.extend(engine.take_animated(showing));
    Ok(pages)
}

/// The sub-page of a page on screen whose teletext page was drawn, if one
/// was: the sub-page whose ReGIS picture is shown.
fn shown_subpage(sent: &BTreeMap<u16, Rendered>, page: u16) -> Option<u16> {
    sent.get(&page).map(|rendered| rendered.subpages.shown)
}

/// When the earliest repaint owed to a ReGIS picture on screen may be made.
fn regis_owed(engine: &mut Engine, showing: &[u16], sent: &BTreeMap<u16, Rendered>, now_us: u64) -> Option<u64> {
    showing.iter().filter_map(|&page| engine.regis().owed_us(page, shown_subpage(sent, page), now_us)).min()
}

/// Sends the picture of each page on screen that the channel draws in
/// ReGIS, when what it shows changed (DEC-6335) and its repaint budget
/// allows (REQ-6380): of the sub-page whose teletext page was drawn, or
/// with none drawn the page's first with any.
fn draw_regis(engine: &mut Engine, showing: &[u16], sent: &BTreeMap<u16, Rendered>, now_us: u64, output: &mut Output<'_>) -> Result<(), ProtocolError> {
    for &page in showing {
        if let Some(drawn) = engine.regis().take(page, shown_subpage(sent, page), now_us) {
            // RULE-66's hops: the repaint written, and the areas it holds.
            #[cfg(feature = "timed-test")]
            {
                let area =
                    |picture: &protocol::regis::RegisPicture| format!("{},{},{},{}", picture.area.x, picture.area.y, picture.area.width, picture.area.height);
                let areas: Vec<String> = core::iter::once(&drawn.picture).chain(&drawn.more).map(area).collect();
                protocol::timed_test::mark("decoder", "regis", std::time::Instant::now(), format_args!("{}", areas.join(";")));
                protocol::timed_test::owe_flush();
            }
            write_to_core(output, &FromDecoder::Regis { page: drawn.page, sub: drawn.sub, picture: drawn.picture, pick: drawn.pick })?;
            // The other areas of the same repaint, each a part of the one before (REQ-6399).
            for picture in drawn.more {
                write_to_core(output, &FromDecoder::Regis { page: drawn.page, sub: drawn.sub, picture, pick: None })?;
            }
        }
        // A page drawn in NAPLPS is sent the same way (DEC-6407).
        if let Some(drawn) = engine.naplps().take(page, shown_subpage(sent, page)) {
            write_to_core(output, &FromDecoder::Regis { page: drawn.page, sub: drawn.sub, picture: drawn.picture, pick: drawn.pick })?;
            for picture in drawn.more {
                write_to_core(output, &FromDecoder::Regis { page: drawn.page, sub: drawn.sub, picture, pick: None })?;
            }
        }
    }
    Ok(())
}

/// Tells the trusted core which pages on screen have nothing to draw, when
/// that changed since `said`.
fn say_missing(engine: &Engine, showing: &[u16], now_us: u64, said: &mut Vec<u16>, output: &mut Output<'_>) -> Result<(), ProtocolError> {
    let missing = engine.missing(showing, now_us);
    if missing != *said {
        write_to_core(output, &FromDecoder::Missing(missing.clone()))?;
        *said = missing;
    }
    Ok(())
}

/// A message read from the trusted core: one, the end of its pipe, or an
/// error.
type Received = Result<Option<ToDecoder>, ProtocolError>;

/// What the loop woke for.
enum Woken {
    /// A message from the trusted core.
    Message(Received),
    /// The deadline passed first.
    Deadline,
    /// The reader of the trusted core's messages ended.
    Ended,
}

/// Reads the trusted core's messages on a thread of their own, until its
/// pipe ends or a message cannot be read.
fn read_messages() -> Receiver<Received> {
    let (messages, incoming) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = BufReader::new(io::stdin().lock());
        loop {
            let message = read_from_core(&mut input);
            // RULE-66's hops: when the input event was read from the pipe.
            #[cfg(feature = "timed-test")]
            if let Ok(Some(ToDecoder::RuleInput { sequence, .. })) = &message {
                protocol::timed_test::mark("decoder", "input-read", std::time::Instant::now(), format_args!("{sequence}"));
            }
            let last = !matches!(message, Ok(Some(_)));
            if messages.send(message).is_err() || last {
                return;
            }
        }
    });
    incoming
}

/// Waits for a message until `deadline` (microseconds by the engine's
/// clock, which reads `now`), or with none for as long as it takes.
fn wait(incoming: &Receiver<Received>, deadline: Option<u64>, now: u64) -> Woken {
    match deadline {
        Some(at) => match incoming.recv_timeout(Duration::from_micros(at.saturating_sub(now))) {
            Ok(message) => Woken::Message(message),
            Err(RecvTimeoutError::Timeout) => Woken::Deadline,
            Err(RecvTimeoutError::Disconnected) => Woken::Ended,
        },
        None => incoming.recv().map_or(Woken::Ended, Woken::Message),
    }
}

/// Sends what the bytes fed produced: the sounds, the pages that arrived,
/// the pages received again and the channel's log lines.
fn report(engine: &mut Engine, arrived: &[(u16, String)], output: &mut Output<'_>) -> Result<(), ProtocolError> {
    send_sounds(engine, output)?;
    // The drawing links of ReGIS sub-pages that arrived whole (REQ-6376).
    #[cfg(feature = "links")]
    for (page, sub, links) in engine.regis().take_links() {
        write_to_core(output, &FromDecoder::RegisLinks { page, sub, links })?;
    }
    for (page, title) in arrived {
        write_to_core(output, &FromDecoder::Arrived { page: *page, title: title.clone() })?;
    }
    // A picture's title that came after its page (TODO-6748).
    for (page, title) in engine.take_retitled() {
        write_to_core(output, &FromDecoder::PictureTitle { page, title })?;
    }
    let updated = engine.take_updated();
    for &page in &updated {
        write_to_core(output, &FromDecoder::Updated { page })?;
    }
    // The sub-pages of the pages that came, for the page list.
    let pages: Vec<u16> = arrived.iter().map(|(page, _)| *page).chain(updated).collect();
    for (page, count) in engine.subpage_changes(&pages) {
        write_to_core(output, &FromDecoder::SubPageCount { page, count })?;
    }
    for log in engine.take_logs() {
        write_to_core(output, &FromDecoder::ChannelLog { level: log.level, page: log.page, text: log.text })?;
    }
    Ok(())
}

/// Draws `pages` at `now_us` and sends those that changed: a page's links
/// go before its image, so the trusted core has them for the image when it
/// arrives (`hyperlinks.md` section 8.1).
fn draw(
    engine: &Engine,
    pages: &[u16],
    now_us: u64,
    sent: &mut BTreeMap<u16, Rendered>,
    spare: &mut Vec<u8>,
    flashing: &mut Flashing,
    output: &mut Output<'_>,
) -> Result<(), ProtocolError> {
    for &page in pages {
        // Rendered into the room kept from the last frame not sent, over
        // the page's last frame sent: only the rows that changed since.
        let last = sent.get(&page).map(|last| last.indices.as_slice());
        let Some(rendered) = engine.render_over(page, now_us, last, core::mem::take(spare)) else { continue };
        flashing.drawn(page, &rendered);
        let before = sent.get(&page);
        if before == Some(&rendered) {
            *spare = rendered.indices;
            continue;
        }
        let new_image =
            before.is_none_or(|before| before.palette != rendered.palette || before.screen != rendered.screen || before.indices != rendered.indices);
        #[cfg(feature = "links")]
        if before.map(|before| &before.links) != Some(&rendered.links) {
            let links = rendered.links.clone();
            let message = FromDecoder::Links {
                page,
                with_image: new_image,
                candidates: links.candidates,
                numbers: links.numbers,
                controls: links.controls,
                fastext: links.fastext,
            };
            write_to_core(output, &message)?;
        }
        if before.map(|before| before.subpages) != Some(rendered.subpages) {
            write_to_core(output, &FromDecoder::SubPages { page, step: rendered.subpages })?;
        }
        if new_image {
            // ANIM-5: only the rectangle that changed, when the rest is the
            // frame last sent of the page in the same colours.
            match before.and_then(|before| changed_area(before, &rendered)) {
                Some((area, indices)) => {
                    let message = FromDecoder::FrameArea { page, palette: rendered.palette.clone(), screen: rendered.screen, area, indices };
                    write_to_core(output, &message)?;
                }
                None => write_frame_to_core(output, page, &rendered.palette, rendered.screen, &rendered.indices)?,
            }
        }
        if let Some(replaced) = sent.insert(page, rendered) {
            *spare = replaced.indices;
        }
    }
    Ok(())
}

/// The rectangle of `after` that differs from `before` and its dots, when
/// both are whole pages in the same palette and screen colour and the
/// rectangle is not the whole page (animation.md ANIM-5).
fn changed_area(before: &Rendered, after: &Rendered) -> Option<(protocol::decoder::PageArea, Vec<u8>)> {
    let width = usize::from(interface_core::canvas::PAGE_WIDTH);
    if before.palette != after.palette || before.screen != after.screen || before.indices.len() != after.indices.len() || width == 0 {
        return None;
    }
    let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0_usize, 0_usize);
    for (row, (was, now)) in before.indices.chunks_exact(width).zip(after.indices.chunks_exact(width)).enumerate() {
        let Some(first) = was.iter().zip(now).position(|(was, now)| was != now) else { continue };
        let last = was.iter().zip(now).rposition(|(was, now)| was != now).unwrap_or(first);
        left = left.min(first);
        right = right.max(last.saturating_add(1));
        top = top.min(row);
        bottom = row.saturating_add(1);
    }
    let (across, down) = (right.checked_sub(left)?, bottom.checked_sub(top)?);
    if across.saturating_mul(down) >= after.indices.len() {
        return None;
    }
    let area = protocol::decoder::PageArea {
        x: u16::try_from(left).ok()?,
        y: u16::try_from(top).ok()?,
        width: u16::try_from(across).ok()?,
        height: u16::try_from(down).ok()?,
    };
    let mut dots = Vec::with_capacity(area.dots());
    for row in top..bottom {
        let start = row.saturating_mul(width).saturating_add(left);
        dots.extend_from_slice(after.indices.get(start..start.saturating_add(across))?);
    }
    Some((area, dots))
}

/// Sends the voice operations the audio receiver produced, and the
/// channel's records for its input found.
fn send_sounds(engine: &mut Engine, output: &mut Output<'_>) -> Result<(), ProtocolError> {
    for audio in engine.take_sounds() {
        write_to_core(output, &FromDecoder::Audio(audio))?;
    }
    #[cfg(feature = "interaction")]
    {
        let input = engine.take_input();
        for body in input.cursors {
            write_to_core(output, &FromDecoder::Cursor { body })?;
        }
        for body in input.controls {
            write_to_core(output, &FromDecoder::InputControl { body })?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::naive_bytecount, reason = "a test")]
mod tests {
    use super::*;

    fn rendered(indices: Vec<u8>) -> Rendered {
        Rendered {
            palette: vec![[0, 0, 0], [255, 255, 255]],
            screen: 0,
            indices,
            flash: None,
            #[cfg(feature = "links")]
            links: crate::Found::default(),
            subpages: protocol::subpages::SubPageStep::default(),
        }
    }

    /// ANIM-5: a one-cell change sends a frame of that cell's rectangle
    /// alone; a change of colours, or of the whole page, sends it whole.
    #[test]
    fn a_one_cell_change_sends_that_cells_rectangle() {
        let width = usize::from(interface_core::canvas::PAGE_WIDTH);
        let before = rendered(vec![0; protocol::frame::PAGE_DOTS]);
        // The cell at column 3, row 2: 12 dots by 20 from (36, 40).
        let mut dots = before.indices.clone();
        for y in 40..60 {
            for x in 36..48 {
                dots[y * width + x] = 1;
            }
        }
        dots[45 * width + 40] = 0;
        let after = rendered(dots);
        let (area, indices) = changed_area(&before, &after).unwrap();
        assert_eq!(area, protocol::decoder::PageArea { x: 36, y: 40, width: 12, height: 20 });
        assert_eq!(indices.len(), 240);
        assert_eq!(indices.iter().filter(|&&index| index == 1).count(), 239, "the cell's dots as drawn");
        let mut recoloured = after.clone();
        recoloured.palette[1] = [255, 0, 0];
        assert!(changed_area(&before, &recoloured).is_none(), "other colours: whole");
        assert!(changed_area(&before, &rendered(vec![1; protocol::frame::PAGE_DOTS])).is_none(), "the whole page: whole");
    }
}
