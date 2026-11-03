//! The typing page, typed into through a session as a receiver feeds it.

use crate::terminal::session::tests::{feed, named, page, press};
use crate::terminal::{Session, Settings};

#[test]
fn on_a_page_with_a_field_digits_are_text() {
    let mut session = Session::new(Settings::default());
    feed(&mut session, &[page(0x200), press('1'), press('0'), press('0')]);
    assert_eq!(session.page, 0x200);
    assert_eq!(session.fields[0].text, "100");
}

#[test]
fn backspace_takes_back_and_tab_moves_to_the_next_field() {
    let mut session = Session::new(Settings::default());
    feed(&mut session, &[page(0x200), press('a'), press('b'), named("Backspace"), named("Tab"), press('c')]);
    assert_eq!(session.fields[0].text, "a");
    assert_eq!(session.fields[1].text, "c");
}
