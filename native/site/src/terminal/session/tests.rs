//! The terminal channel's sessions, fed as a receiver feeds them.

use super::*;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use neoteletext_input::caps::{CURSOR_CAP, PAGE_CAP};

use crate::podcast::Episode;
use crate::terminal::invaders::INVADER_TOP;
use crate::terminal::pages::contents_file;
use neoteletext_input::event::{Mouse, Point, read_message, write};

/// One item as a receiver sends it.
fn encoded(item: &Item<'_>) -> Vec<u8> {
    let mut out = [0; neoteletext_input::MAX_MESSAGE];
    let length = write(item, &mut out).unwrap();
    out[..length].to_vec()
}

/// A printable key, as a receiver sends it outside `ALL_KEYS`: its text.
fn press(character: char) -> Vec<u8> {
    encoded(&Item::Text(character.encode_utf8(&mut [0; 4])))
}

fn key(action: Action, key: KeyCode, text: Option<&str>, base: Option<char>) -> Vec<u8> {
    encoded(&Item::Event(Event::Key(Key { action, key, mods: 0, text, base })))
}

fn named(name: &str) -> Vec<u8> {
    key(Action::Press, KeyCode::Functional(keys::code(name).unwrap()), None, None)
}

fn page(number: u16) -> Vec<u8> {
    encoded(&Item::Event(Event::Page(number)))
}

fn feed(session: &mut Session, messages: &[Vec<u8>]) -> Vec<Push> {
    let mut pushes = Vec::new();
    for message in messages {
        read_message(message, &mut |item| pushes.extend(session.item(&item))).unwrap();
    }
    pushes
}

/// INPUT.md section 6.1: HELLO is answered with WELCOME, a control
/// record of what the channel uses of what was offered.
#[test]
fn hello_is_answered_with_welcome() {
    let mut session = Session::new(Settings::default());
    let offered = PAGE_CAP | CURSOR_CAP | 0x88; // and two retired mode bits
    let pushes = feed(&mut session, &[encoded(&Item::Event(Event::Hello { version: 2, caps: offered, agent: None }))]);
    let [Push::Packets(bytes)] = pushes.as_slice() else { panic!("one packet") };
    let packet = neoteletext_core::packet::Packet { bytes: bytes.as_slice().try_into().unwrap() };
    let mut buffer = [0; neoteletext_input::control::FRAGMENT_DATA];
    let welcome = neoteletext_input::control::from_packet(&packet, &mut buffer);
    assert_eq!(welcome, Some(Ok(Control::Welcome { version: 2, caps: PAGE_CAP | CURSOR_CAP })));
    assert!(feed(&mut session, &[encoded(&Item::Event(Event::Hello { version: 1, caps: 0, agent: None }))]).is_empty(), "another version");
}

/// REQ-INT-116 rule 8: native digits select pages too.
#[test]
fn the_digits_of_any_script_choose_a_page() {
    let mut session = Session::new(Settings::default());
    feed(&mut session, &[press('٣'), press('٠'), press('٠')]);
    assert_eq!(session.page, 0x300, "Arabic-Indic three hundred");
    feed(&mut session, &[press('*'), press('५'), press('०'), press('०'), press('#')]);
    assert_eq!(session.page, 0x500, "Devanagari, the videotex way");
}

/// REQ-INT-108: `FOCUS_OUT` lets go of what is held.
#[test]
fn focus_out_releases_the_held_arrow() {
    let mut session = Session::new(Settings::default());
    feed(&mut session, &[page(0x300), named("ArrowLeft")]);
    assert!(session.invaders.left);
    feed(&mut session, &[encoded(&Item::Event(Event::FocusOut))]);
    assert!(!session.invaders.left, "no release was sent, and none was needed");
}

/// Decision E.125: no page asks for input modes.
#[test]
fn no_page_has_an_im_line() {
    let mut session = Session::new(Settings::default());
    for page_number in [0x200, 0x300, 0x400, 0x500] {
        feed(&mut session, &[page(page_number)]);
        assert!(!session.render().tti().contains("IM,"), "{page_number:X}");
    }
}

#[test]
fn pages_are_chosen_as_on_prestel() {
    let mut session = Session::new(Settings::default());
    feed(&mut session, &[press('*'), press('3'), press('0'), press('0'), press('#')]);
    assert_eq!(session.page, 0x300);
    feed(&mut session, &[press('#')]);
    assert_eq!(session.page, 0x400, "# is the next page");
    feed(&mut session, &[press('*'), press('#')]);
    assert_eq!(session.page, 0x300, "*# goes back");
    feed(&mut session, &[press('5'), press('0'), press('0')]);
    assert_eq!(session.page, 0x500, "three digits where there is no field");
    feed(&mut session, &[page(0x200), press('1'), press('0'), press('0')]);
    assert_eq!(session.page, 0x200, "on a page with a field digits are text");
    assert_eq!(session.fields[0].text, "100");
}

#[test]
fn typing_echoes_and_acknowledges() {
    let mut session = Session::new(Settings::default());
    let pushes = feed(&mut session, &[page(0x200), press('A'), press('n'), named("Enter")]);
    let Some(Push::Page(last)) = pushes.last() else { panic!() };
    assert!(last.contains("Hello, An!"), "{last}");
    assert!(last.contains("CU,5,15,18,1,4"), "the cursor after the name, echo on, acknowledging an input count of 4: {last}");
    feed(&mut session, &[named("Tab"), press('x')]);
    let page = session.render().tti();
    assert!(page.contains("CU,8,14,11,3,"), "the secret field is masked: {page}");
}

/// The channels the receiver started sounds on.
struct Count(Vec<u8>);
impl neoteletext_audio::port::VoiceSink for Count {
    fn define(&mut self, _: neoteletext_audio::port::SoundId, _: neoteletext_audio::port::SoundFormat) {}
    fn samples(&mut self, _: neoteletext_audio::port::SoundId, _: u32, _: &[i16]) {}
    fn forget(&mut self, _: neoteletext_audio::port::SoundId) {}
    fn stream(&mut self, _: neoteletext_audio::port::SoundId, _: u32) {}
    fn stream_samples(&mut self, _: neoteletext_audio::port::SoundId, _: &[i16]) {}
    fn start(&mut self, channel: u8, _: neoteletext_audio::port::Start) {
        self.0.push(channel);
    }
    fn stop(&mut self, _: u8, _: u16) {}
    fn gain(&mut self, _: u8, _: u8, _: u16) {}
}

#[test]
fn the_typing_page_clicks_on_keys_and_rings_on_enter() {
    let mut session = Session::new(Settings::default());
    feed(&mut session, &[page(0x200)]);
    let commands = |pushes: &[Push]| pushes.iter().filter(|push| matches!(push, Push::Packets(bytes) if bytes.len() == 42)).count();
    let key = feed(&mut session, &[press('a')]);
    assert_eq!(commands(&key), 1, "a click");
    let enter = feed(&mut session, &[named("Enter")]);
    assert_eq!(commands(&enter), 1, "a bell");
    // The game's page does not click.
    feed(&mut session, &[page(0x300)]);
    assert_eq!(commands(&feed(&mut session, &[press('a')])), 0);
}

#[test]
fn the_piano_sends_its_sounds_then_a_command_a_key() {
    let mut session = Session::new(Settings::default());
    let pushes = feed(&mut session, &[page(0x400)]);
    let sounds = pushes.iter().filter(|push| matches!(push, Push::Packets(bytes) if !bytes.is_empty() && bytes.len() % 42 == 0)).count();
    assert_eq!(sounds, PIANO.len() + 6, "the notes, the game's four sounds and the typing page's two");
    // In ALL_KEYS, the page's own mode: `q` on AZERTY, where A is on a
    // United States layout, plays the first note (REQ-INT-116).
    let pushes = feed(&mut session, &[key(Action::Press, KeyCode::Character('q'), Some("q"), Some('a'))]);
    assert!(matches!(pushes.first(), Some(Push::Packets(bytes)) if bytes.len() == 42), "one command, one packet");
    assert_eq!(session.piano, Some(0));
    feed(&mut session, &[key(Action::Release, KeyCode::Character('q'), None, Some('a'))]);
    assert_eq!(session.piano, None, "the release ends the note");
    // The sounds decode with the engine's own receiver.
    let mut arena = vec![0_u8; 1 << 20];
    let mut receiver = neoteletext_audio::receiver::Receiver::<16, 4>::new(&mut arena);
    let mut sink = Count(Vec::new());
    let mut session = Session::new(Settings::default());
    let mut all = feed(&mut session, &[page(0x400)]);
    all.extend(feed(&mut session, &[key(Action::Press, KeyCode::Character('s'), Some("s"), None)]));
    hear(&mut receiver, &mut sink, all);
    assert_eq!(sink.0.len(), 1, "the note played");
}

fn hear(receiver: &mut neoteletext_audio::receiver::Receiver<'_, 16, 4>, sink: &mut Count, pushes: Vec<Push>) {
    for push in pushes {
        if let Push::Packets(bytes) = push {
            for chunk in bytes.as_chunks::<42>().0 {
                receiver.push_packet(&neoteletext_core::packet::Packet { bytes: *chunk }, sink);
            }
        }
    }
}

#[test]
fn the_podcast_plays_on_space_on_a_channel_of_its_own() {
    let rate = 8_000;
    let episode = Episode { title: "Pilot".to_owned(), rate, samples: vec![500; usize::try_from(rate).unwrap() * 3] };
    let mut session = Session::new(Settings { relay: None, podcast: Some(Arc::new(episode)) });
    let mut arena = vec![0_u8; 1 << 20];
    let mut receiver = neoteletext_audio::receiver::Receiver::<16, 4>::new(&mut arena);
    let mut sink = Count(Vec::new());
    let mut pushes = feed(&mut session, &[page(0x700)]);
    assert!(session.render().tti().contains("Ready: press Space"));
    pushes.extend(feed(&mut session, &[press(' ')]));
    assert!(session.render().tti().contains("Playing"));
    assert!(session.render().tti().contains("0:00 / 0:03"));
    // The typing page clicks on the effects' channels, never the podcast's.
    pushes.extend(feed(&mut session, &[page(0x200), press('a'), press('b'), press('c'), press('d')]));
    hear(&mut receiver, &mut sink, pushes);
    assert_eq!(sink.0, [podcast::CHANNEL, 0, 1, 2, 0], "the podcast, then four clicks in turn on the other three");
    feed(&mut session, &[key(Action::Press, KeyCode::Character(' '), None, None)]);
    assert!(session.render().tti().contains("Last key: Space\r\n"), "the space bar named");
}

#[test]
fn the_podcast_page_says_when_the_episode_ends_within_a_second() {
    // Ten and a half seconds: skipped to ten, it ends half a second later,
    // in the second it began.
    let rate = 8_000;
    let samples = usize::try_from(rate).unwrap() * 21 / 2;
    let episode = Episode { title: "Short".to_owned(), rate, samples: vec![500; samples] };
    let mut session = Session::new(Settings { relay: None, podcast: Some(Arc::new(episode)) });
    feed(&mut session, &[page(0x700), named("ArrowRight"), press(' ')]);
    session.podcast_frame();
    assert!(session.render().tti().contains("0:10 / 0:10"));
    thread::sleep(Duration::from_millis(700));
    let pushes = session.podcast_frame();
    assert!(matches!(pushes.last(), Some(Push::Page(tti)) if tti.contains("Finished")), "the stop, then the page");
}

#[test]
fn the_game_has_no_cursor_and_moves_while_a_key_is_held() {
    let mut session = Session::new(Settings::default());
    let start = session.invaders.ship;
    feed(&mut session, &[page(0x300), named("ArrowLeft")]);
    assert_eq!(session.invaders.ship, start - 1, "a press moves at once");
    session.invaders.tick();
    session.invaders.tick();
    assert_eq!(session.invaders.ship, start - 3, "and held, each frame");
    assert!(session.render().tti().contains("CU,255,0,0,0,"));
}

#[test]
fn the_game_sounds_a_shot_and_a_hit() {
    let mut session = Session::new(Settings::default());
    let opened = feed(&mut session, &[page(0x300)]);
    assert_eq!(opened.iter().filter(|push| matches!(push, Push::Packets(_))).count(), PIANO.len() + 6, "its sounds first");
    let fired = feed(&mut session, &[press(' ')]);
    assert!(matches!(fired.first(), Some(Push::Packets(bytes)) if bytes.len() == 42), "the shot's command");
    // A shot one row below the bottom row's first invader hits it next frame.
    session.invaders.shots.clear();
    let (x, y) = *session.invaders.alive.last().unwrap();
    let (offset, drop) = (session.invaders.offset, session.invaders.drop);
    session.invaders.shots.push((x + offset + 1, INVADER_TOP + y + drop + 1));
    let frame = session.frame();
    assert!(matches!(frame.first(), Some(Push::Packets(bytes)) if bytes.len() == 42), "a hit sounds");
    assert_eq!(session.invaders.score, 10);
}

#[test]
fn the_contents_file_is_the_engines_format() {
    let contents = contents_file();
    let read = neoteletext_web::contents::read(contents.as_bytes()).unwrap();
    assert_eq!(read.flatten().count(), PAGES.len() + 1);
}

#[test]
fn the_page_says_when_the_keys_went_elsewhere() {
    let mut session = Session::new(Settings::default());
    feed(&mut session, &[page(0x200), encoded(&Item::Event(Event::FocusOut))]);
    assert!(session.render().tti().contains("Your keys went elsewhere."));
    feed(&mut session, &[encoded(&Item::Event(Event::FocusIn))]);
    assert!(!session.render().tti().contains("elsewhere"));
}

#[test]
fn a_click_is_the_hosts() {
    let mut session = Session::new(Settings::default());
    let at = Point { column: 7, row: 11, pixels: None, mods: 0 };
    feed(&mut session, &[page(0x500), encoded(&Item::Event(Event::Mouse(Mouse { action: MouseAction::Press, button: 0, at })))]);
    assert!(session.render().tti().contains("You clicked column 7, row 11."));
}
