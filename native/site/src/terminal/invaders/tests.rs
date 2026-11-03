//! The game, played through a session as a receiver feeds it.

use super::*;
use crate::terminal::Settings;
use crate::terminal::session::tests::{encoded, feed, named, page, press};
use crate::terminal::sounds::PIANO;
use neoteletext_input::event::{Event, Item};

/// REQ-INT-108: `FOCUS_OUT` lets go of what is held.
#[test]
fn focus_out_releases_the_held_arrow() {
    let mut session = Session::new(Settings::default());
    feed(&mut session, &[page(0x300), named("ArrowLeft")]);
    assert!(session.invaders.left);
    feed(&mut session, &[encoded(&Item::Event(Event::FocusOut))]);
    assert!(!session.invaders.left, "no release was sent, and none was needed");
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
