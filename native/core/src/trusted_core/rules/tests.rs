//! The gate through the whole trusted core (page-rules.md RULE-50 to
//! RULE-52, RULE-33; REQ-6523, REQ-6534, REQ-6535): what reaches a page's
//! decoder, its fetcher and the sound helper, read from their pipes.

use std::sync::mpsc;
use std::time::Duration;

use interface_core::canvas::PageKey;
use neoteletext_input::event::{Action, Event, Item, KeyCode, read_message};
use protocol::ToCore;
use protocol::audio::{Audio, WatchSlot};
use protocol::decoder::{FromDecoder, ToDecoder};
use protocol::fetcher::{Entry, FromFetcher, ToFetcher};
use protocol::interaction::{InputIntent, Intent};
use protocol::rules::{INTEREST_KEYS, PageSub, Report, Reported, RuleInput, RulesOpen};
use protocol::settings::SettingChange;
use protocol::sound::ToSound;
use protocol::subpages::SubPageStep;

use super::super::test_support::{Sink, Test, decoder_watched, sent_to_fetcher, sleeping_helper, wire};
use neoteletext::outbox::Outbox;

const LIVE: &str = "https://live.invalid/channel/";
const PAGE: u16 = 0x100;
/// [`PAGE`] as drawn, sub-code 0: the page its table is for (RULE-112).
const OPEN: PageSub = PageSub::new(PAGE, 0);

/// A tab on a live channel with page 100 open and drawn, its session
/// welcomed, and what the core writes to its decoder and its fetcher.
fn open_page(name: &str) -> (Test, u32, mpsc::Receiver<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
    let mut test = Test::new(&format!("rules {name}"));
    test.core.session.open(LIVE).unwrap();
    let channel = test.core.active_channel().unwrap();
    let decoder = decoder_watched(&mut test.core, channel);
    test.core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
    test.core.fetched(channel, FromFetcher::Connecting(false)).unwrap();
    test.core.fetched(channel, FromFetcher::Contents(vec![Entry::Page { page: PAGE, title: String::from("Page 100"), subpages: 0 }])).unwrap();
    test.core.fetched(channel, FromFetcher::InitialPage(PAGE)).unwrap();
    test.core.intent(ToCore::Opened(Some(PageKey { channel, page: PAGE }))).unwrap();
    let fetcher = wire(&mut test.core, channel);
    let step = SubPageStep { count: 1, shown: 0, place: Some(0), previous: None, next: None };
    test.core.decoded(channel, FromDecoder::SubPages { page: PAGE, step }).unwrap();
    test.sent();
    (test, channel, decoder, fetcher)
}

/// Every message the trusted core wrote to the decoder since the last look.
fn sent_to_decoder(test: &mut Test, channel: u32, written: &mpsc::Receiver<Vec<u8>>) -> Vec<ToDecoder> {
    test.core.decoders.get_mut(&channel).unwrap().flush().unwrap();
    let mut bytes = Vec::new();
    while let Ok(chunk) = written.recv_timeout(Duration::from_millis(200)) {
        bytes.extend(chunk);
    }
    let mut input = bytes.as_slice();
    let mut messages = Vec::new();
    while let Some(message) = protocol::decoder::read_from_core(&mut input).unwrap() {
        messages.push(message);
    }
    messages
}

fn press(test: &mut Test, channel: u32, character: char) {
    let intent = Intent::Key { action: Action::Press, key: KeyCode::Character(character), mods: 0, text: Some(character.to_string()), base: None };
    test.core.intent(ToCore::Input(InputIntent { key: PageKey { channel, page: PAGE }, intent })).unwrap();
}

/// The viewer's input events the fetcher was given, each as the rule that
/// reported it and the event it carries; anything else it was given that
/// is a viewer's input outside a report fails the test in `read_message`.
fn input_sent(messages: &[ToFetcher]) -> Vec<(u8, String)> {
    let mut sent = Vec::new();
    for message in messages {
        if let ToFetcher::Send { bytes, .. } = message {
            read_message(bytes, &mut |item| {
                if let Item::Event(Event::RuleEvent(rule)) = item {
                    sent.push((rule.rule, rule.input().map(|(carried, _)| format!("{carried:?}")).unwrap_or_default()));
                }
            })
            .unwrap();
        }
    }
    sent
}

/// REQ-6523, RULE-50, RULE-52, RULE-58 (REQ-6534): through the whole
/// trusted core, a key on a page with no table reaches neither its
/// decoder nor its fetcher; once the decoder says the page's table asks
/// for keys the key is given to the decoder with a sequence number, and it
/// reaches the fetcher only as the `RULE_EVENT` of a report the decoder
/// asks for, built from the core's own copy.
#[test]
fn a_key_leaves_only_as_the_report_its_pages_table_asks_for() {
    let (mut test, channel, decoder, fetcher) = open_page("key");
    let told = sent_to_decoder(&mut test, channel, &decoder);
    assert!(told.contains(&ToDecoder::RulesPage(Some(OPEN))), "the decoder is told which page is the open page: {told:?}");
    sent_to_fetcher(&mut test.core, channel, &fetcher);

    press(&mut test, channel, 'a');
    assert_eq!(sent_to_decoder(&mut test, channel, &decoder), [], "no table: the decoder is given nothing");
    assert_eq!(input_sent(&sent_to_fetcher(&mut test.core, channel, &fetcher)), [], "and the channel is sent nothing");

    let asks = RulesOpen { page: OPEN, generation: 3, interest: INTEREST_KEYS, watch: Vec::new(), wheel: Vec::new() };
    test.core.decoded(channel, FromDecoder::RulesOpen(asks)).unwrap();
    // The key is given under the gate's first sequence number, and its
    // report asked for at once, as a decoder's next tick does: this core's
    // clock runs, and a report is taken only within `SEND_WINDOW_TICKS`.
    press(&mut test, channel, 'b');
    let report = Report {
        page: OPEN,
        generation: 3,
        rule: 4,
        row: neoteletext_rules::limits::NO_RULE,
        tick: 9,
        target_kind: 0,
        target_number: 0,
        what: Reported::Input { sequence: 0 },
    };
    test.core.decoded(channel, FromDecoder::Report(report)).unwrap();
    // A decoder cannot have it sent again, nor a key it was never given.
    test.core.decoded(channel, FromDecoder::Report(report)).unwrap();
    let never = Report { what: Reported::Input { sequence: 7 }, ..report };
    test.core.decoded(channel, FromDecoder::Report(never)).unwrap();
    let given = sent_to_decoder(&mut test, channel, &decoder);
    let [ToDecoder::RuleInput { page: OPEN, sequence: 0, input: RuleInput::Key { code, named: false, .. } }] = given.as_slice() else { panic!("{given:?}") };
    assert_eq!(*code, u32::from('b'));
    let sent = input_sent(&sent_to_fetcher(&mut test.core, channel, &fetcher));
    assert!(matches!(sent.as_slice(), [(4, key)] if key.contains("Character('b')")), "once, as rule 4's report: {sent:?}");
    // Long after, the same request is too old to be answered.
    test.core.decoded(channel, FromDecoder::Report(Report { rule: 5, ..report })).unwrap();
    assert_eq!(input_sent(&sent_to_fetcher(&mut test.core, channel, &fetcher)), [], "past the window");
}

/// What the trusted core wrote to the sound helper since the last look.
fn sent_to_sound(test: &mut Test, written: &mpsc::Receiver<Vec<u8>>) -> Vec<ToSound> {
    if let Some((_, input)) = test.core.sound.as_mut() {
        std::io::Write::flush(input).unwrap();
    }
    let mut bytes = Vec::new();
    while let Ok(chunk) = written.recv_timeout(Duration::from_millis(200)) {
        bytes.extend(chunk);
    }
    let mut input = bytes.as_slice();
    let mut messages = Vec::new();
    while let Some(message) = protocol::sound::read_to_sound(&mut input).unwrap() {
        messages.push(message);
    }
    messages
}

/// RULE-33 items 2 and 4 (REQ-6535): the watch list of the open page's
/// table is passed to the sound helper tagged with the view's channel; a
/// metered report is given to that view's decoder only at the length of
/// the list passed, and a report of another length, or for a view with no
/// list, is dropped; when the table ends the helper is given an empty
/// list.
#[test]
fn the_watch_list_goes_to_the_sound_helper_and_a_report_of_its_length_comes_back() {
    let (mut test, channel, decoder, _fetcher) = open_page("watch");
    let (sender, sound) = mpsc::channel();
    let (helper, _output) = sleeping_helper(confine::Role::Sound);
    test.core.sound = Some((helper, Outbox::new(Sink(sender))));
    sent_to_decoder(&mut test, channel, &decoder);

    test.core.metered(channel, vec![1, 2]);
    assert_eq!(sent_to_decoder(&mut test, channel, &decoder), [], "no list was passed for this view");

    let watch = vec![WatchSlot { channel: 0, kind: 1, part: 0 }, WatchSlot { channel: 0, kind: 3, part: 2 }];
    let asks = RulesOpen { page: OPEN, generation: 1, interest: 0, watch: watch.clone(), wheel: Vec::new() };
    test.core.decoded(channel, FromDecoder::RulesOpen(asks)).unwrap();
    assert_eq!(sent_to_sound(&mut test, &sound), [ToSound::Audio { channel, audio: Audio::Watch(watch) }]);

    test.core.metered(channel, vec![9]);
    test.core.metered(channel, vec![9, 9, 9]);
    test.core.metered(channel.wrapping_add(1), vec![9, 9]);
    assert_eq!(sent_to_decoder(&mut test, channel, &decoder), [], "a report that is not the list's length, or another view's");
    test.core.metered(channel, vec![200, 3]);
    assert_eq!(sent_to_decoder(&mut test, channel, &decoder), [ToDecoder::Metered { page: OPEN, values: vec![200, 3] }]);

    // A decoder cannot have another page's table watched.
    let other =
        RulesOpen { page: PageSub::new(0x200, 0), generation: 1, interest: 0, watch: vec![WatchSlot { channel: 1, kind: 1, part: 0 }], wheel: Vec::new() };
    test.core.decoded(channel, FromDecoder::RulesOpen(other)).unwrap();
    assert_eq!(
        sent_to_sound(&mut test, &sound),
        [ToSound::Audio { channel, audio: Audio::Watch(Vec::new()) }],
        "the page is not the open page: nothing is watched"
    );
    test.core.metered(channel, vec![1]);
    assert_eq!(sent_to_decoder(&mut test, channel, &decoder), []);
}

/// RULE-47, RULE-52 item 6 (REQ-6534): a state query the fetcher received
/// is passed to the decoder for the open page only, and the decoder's
/// answer goes to the channel once.
#[test]
fn a_state_query_is_passed_for_the_open_page_and_answered_once() {
    let (mut test, channel, decoder, fetcher) = open_page("query");
    sent_to_decoder(&mut test, channel, &decoder);
    sent_to_fetcher(&mut test.core, channel, &fetcher);
    test.core.fetched(channel, FromFetcher::RulesQuery { page: PageSub::new(0x200, 0), identifier: 4 }).unwrap();
    assert_eq!(sent_to_decoder(&mut test, channel, &decoder), [], "a page that is not open");
    test.core.fetched(channel, FromFetcher::RulesQuery { page: OPEN, identifier: 5 }).unwrap();
    assert_eq!(sent_to_decoder(&mut test, channel, &decoder), [ToDecoder::RulesQuery { page: OPEN, identifier: 5 }]);
    let mut body = [0_u8; 32];
    let length = neoteletext_rules::state::encode(&neoteletext_rules::state::PageState { identifier: 5, table: None }, &mut body).unwrap();
    let answer = FromDecoder::PageState { page: OPEN, identifier: 5, body: body[..length].to_vec() };
    let states = |messages: &[ToFetcher]| {
        let mut count = 0;
        for message in messages {
            if let ToFetcher::Send { bytes, .. } = message {
                read_message(bytes, &mut |item| count += usize::from(matches!(item, Item::Event(Event::PageState(_))))).unwrap();
            }
        }
        count
    };
    test.core.decoded(channel, answer.clone()).unwrap();
    assert_eq!(states(&sent_to_fetcher(&mut test.core, channel, &fetcher)), 1);
    test.core.decoded(channel, answer).unwrap();
    assert_eq!(states(&sent_to_fetcher(&mut test.core, channel, &fetcher)), 0, "once");
}

/// The `SoundHeard` messages among `messages`, in order.
fn heard(messages: &[ToDecoder]) -> Vec<bool> {
    messages.iter().filter_map(|message| if let ToDecoder::SoundHeard(heard) = message { Some(*heard) } else { None }).collect()
}

/// Gives the core a sound helper that runs and reads nothing, as one with
/// an output device does between its messages.
fn run_sound_helper(test: &mut Test) {
    let (sender, _sound) = mpsc::channel();
    let (helper, _output) = sleeping_helper(confine::Role::Sound);
    test.core.sound = Some((helper, Outbox::new(Sink(sender))));
}

fn change(test: &mut Test, change: SettingChange) {
    test.core.intent(ToCore::ChangeSetting(change)).unwrap();
}

/// RULE-27 (DEC-7143 items 2 and 3; TODO-7141 item 3): with the sound
/// helper running and Sound on at full volume, a channel's decoder is told
/// `SoundHeard(false)` when Sound is turned off, the volume set to 0, its
/// tab muted or the sound helper ends, and `SoundHeard(true)` at each
/// opposite; a change of volume between two values above 0 tells it
/// nothing, nor does Sound turned on again while the volume is 0.
#[test]
fn a_decoder_is_told_whenever_its_sound_stops_or_starts_reaching_the_viewer() {
    let (mut test, channel, decoder, _fetcher) = open_page("sound heard");
    run_sound_helper(&mut test);
    sent_to_decoder(&mut test, channel, &decoder);

    change(&mut test, SettingChange::Sound(false));
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [false], "Sound off");
    change(&mut test, SettingChange::Sound(true));
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [true], "Sound on");

    change(&mut test, SettingChange::Volume(0));
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [false], "volume 0");
    change(&mut test, SettingChange::Sound(false));
    change(&mut test, SettingChange::Sound(true));
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [] as [bool; 0], "Sound off and on at volume 0: nothing heard either way");
    change(&mut test, SettingChange::Volume(40));
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [true], "volume above 0");
    change(&mut test, SettingChange::Volume(70));
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [] as [bool; 0], "from one volume above 0 to another");

    test.core.intent(ToCore::MuteTab(0)).unwrap();
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [false], "the tab muted");
    test.core.intent(ToCore::MuteTab(0)).unwrap();
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [true], "the tab unmuted");

    super::super::run::handle(&mut test.core, super::super::Event::SoundEnded(None)).unwrap();
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [false], "the sound helper ended");
    assert!(test.core.sound.is_none());
    change(&mut test, SettingChange::Sound(false));
    change(&mut test, SettingChange::Sound(true));
    assert_eq!(heard(&sent_to_decoder(&mut test, channel, &decoder)), [] as [bool; 0], "with no sound helper nothing is heard, whatever the setting");
}

/// What the stand-in broker reads as the tab's file channel.
const FILE: &str = "/tmp/news.tti";
/// Ceiling on the stand-in's account once the core has gone.
const FINISH_WITHIN: Duration = Duration::from_secs(10);

/// The messages the core wrote to each decoder the stand-in broker
/// started for it, once the core has gone.
fn decoders_told(stand_in: confine::broker::stand_in::StandIn) -> Vec<Vec<ToDecoder>> {
    let written = stand_in.finish(FINISH_WITHIN).unwrap();
    written
        .iter()
        .filter(|written| written.helper.program == confine::broker::Program::Decoder)
        .map(|written| {
            let mut input = written.bytes.as_slice();
            let mut messages = Vec::new();
            while let Some(message) = protocol::decoder::read_from_core(&mut input).unwrap() {
                messages.push(message);
            }
            messages
        })
        .collect()
}

/// RULE-27 (DEC-7143 item 3): a decoder started while no sound helper
/// runs, as on a machine with no output device, is told
/// `SoundHeard(false)` first of all, before anything of its channel; one
/// started while the helper runs with Sound on is told nothing of it, a
/// decoder beginning with its sound heard.
#[test]
fn a_new_decoder_is_told_first_when_its_sound_is_not_heard() {
    let (mut core, stand_in) = super::super::test_support::core_on(FILE);
    core.match_decoders().unwrap();
    core.flush().unwrap();
    drop(core);
    let told = decoders_told(stand_in);
    assert_eq!(told.len(), 1, "one decoder: {told:?}");
    assert_eq!(told[0].first(), Some(&ToDecoder::SoundHeard(false)), "first of all: {:?}", told[0]);
    assert!(
        told[0][1..].iter().any(|message| matches!(message, ToDecoder::Data(_) | ToDecoder::Whole { .. })),
        "and the channel's data after it: {:?}",
        told[0]
    );
    assert_eq!(heard(&told[0]), [false], "once");

    let (mut core, stand_in) = super::super::test_support::core_on(FILE);
    let (sender, _sound) = mpsc::channel();
    let (helper, _output) = sleeping_helper(confine::Role::Sound);
    core.sound = Some((helper, Outbox::new(Sink(sender))));
    core.match_decoders().unwrap();
    core.flush().unwrap();
    core.sound = None;
    drop(core);
    let told = decoders_told(stand_in);
    assert_eq!(told.len(), 1, "one decoder: {told:?}");
    assert_eq!(heard(&told[0]), [] as [bool; 0], "the sound helper runs: nothing to tell: {:?}", told[0]);
}

/// RULE-27 (DEC-7143 item 3): the sound helper started tells every
/// decoder its sound is heard, and its end that it is not.
#[test]
fn the_sound_helper_starting_and_ending_is_told_to_every_decoder() {
    let (mut core, stand_in) = super::super::test_support::core_on(FILE);
    let channel = core.active_channel().unwrap();
    let decoder = decoder_watched(&mut core, channel);
    let told = |core: &mut super::super::Core| {
        core.decoders.get_mut(&channel).unwrap().flush().unwrap();
        let mut bytes = Vec::new();
        while let Ok(chunk) = decoder.recv_timeout(Duration::from_millis(200)) {
            bytes.extend(chunk);
        }
        let mut input = bytes.as_slice();
        let mut messages = Vec::new();
        while let Some(message) = protocol::decoder::read_from_core(&mut input).unwrap() {
            messages.push(message);
        }
        heard(&messages)
    };
    told(&mut core);
    core.start_sound();
    assert!(core.sound.is_some(), "the stand-in broker started a sound helper");
    assert_eq!(told(&mut core), [true], "the sound helper started");
    super::super::run::handle(&mut core, super::super::Event::SoundEnded(None)).unwrap();
    assert_eq!(told(&mut core), [false], "the sound helper ended");
    drop(core);
    drop(stand_in);
}
