//! DEC-7225, REQ-7226: a digit of a page number is forgotten when no further
//! digit follows in time or the window shows another page or channel, from
//! the keyboard, the remote and the paired phone alike (TODO-7219).

#![allow(clippy::arithmetic_side_effects, reason = "a test's clock")]

use super::DIGITS_TIMEOUT_MILLISECONDS;
use crate::browser::fixture::{browser, channel};
use crate::browser::{Browser, Key, Modifiers};
use crate::canvas::PageKey;
use crate::channel::page_index;

/// The fixture's channel.
const CHANNEL: u32 = 7;
/// A page the fixture's channel has, typed after the stray digit.
const TYPED: u16 = 0x401;
/// Another page it has, opened between the stray digit and the number.
const OTHER: u16 = 0x103;
/// The stray digit. With the first two digits of [`TYPED`] it names page
/// 540, which the fixture's channel does not have.
const STRAY: &str = "5";
/// When the stray digit is typed.
const START: u64 = 10_000;

fn type_text(browser: &mut Browser, text: &'static str, now: u64) {
    browser.key(Key::Text(text), Modifiers::default(), now);
}

fn chosen(browser: &Browser) -> Option<u16> {
    browser.selected.and_then(|index| browser.page_key(index)).map(|key| key.page)
}

// DEC-7225 item 1: a digit followed by no other within the time is
// forgotten, and the three typed afterwards select the page they name.
// Before, the stray digit was kept for ever: 5 then 401 read as 540, which
// selected nothing, and left 1 behind for the next number.
#[test]
fn a_stray_digit_is_forgotten_after_the_time_and_the_next_number_selects_its_page() {
    let (mut browser, _) = browser();
    type_text(&mut browser, STRAY, START);
    type_text(&mut browser, "401", START + DIGITS_TIMEOUT_MILLISECONDS);
    assert_eq!(chosen(&browser), Some(TYPED));
    assert_eq!(browser.digits, "", "nothing is left for the next number");
}

// DEC-7225 item 1: the time runs from the last digit, so a number typed
// slowly, each digit just inside the time, still selects its page.
#[test]
fn each_digit_has_the_whole_time_after_the_one_before() {
    let (mut browser, _) = browser();
    let step = DIGITS_TIMEOUT_MILLISECONDS - 1;
    type_text(&mut browser, "4", START);
    type_text(&mut browser, "0", START + step);
    type_text(&mut browser, "1", START + step + step);
    assert_eq!(chosen(&browser), Some(TYPED));
}

// DEC-7225 item 2: a digit is forgotten when another page opens, here as
// the trusted core opens one for a link, an address or a redirect, well
// inside the time.
#[test]
fn a_stray_digit_is_forgotten_when_another_page_opens() {
    let (mut browser, _) = browser();
    type_text(&mut browser, STRAY, START);
    browser.show(PageKey { channel: CHANNEL, page: OTHER }, START + 1);
    assert_eq!(chosen(&browser), Some(OTHER));
    type_text(&mut browser, "401", START + 2);
    assert_eq!(chosen(&browser), Some(TYPED));
    assert_eq!(browser.digits, "");
}

// DEC-7225 item 2: and when a page is chosen in the list.
#[test]
fn a_stray_digit_is_forgotten_when_a_page_is_chosen_in_the_list() {
    let (mut browser, _) = browser();
    type_text(&mut browser, STRAY, START);
    browser.select_page(page_index(OTHER).unwrap(), true);
    type_text(&mut browser, "401", START + 2);
    assert_eq!(chosen(&browser), Some(TYPED));
}

// DEC-7225 item 2: and when the window shows another channel, as a change
// of tab or of channel brings.
#[test]
fn a_stray_digit_is_forgotten_when_the_channel_changes() {
    let (mut browser, _) = browser();
    type_text(&mut browser, STRAY, START);
    let mut next = channel();
    next.channel = CHANNEL + 1;
    browser.set_channel(next);
    type_text(&mut browser, "401", START + 2);
    assert_eq!(chosen(&browser), Some(TYPED));
}

// DEC-7225 item 4: the paired phone's digits follow the same rule, typed
// as text and as keys.
#[cfg(feature = "phone-keyboard")]
#[test]
fn the_phones_stray_digit_is_forgotten_after_the_time() {
    let (mut browser, _) = browser();
    assert!(browser.phone_text(STRAY, START));
    assert!(browser.phone_text("401", START + DIGITS_TIMEOUT_MILLISECONDS));
    assert_eq!(chosen(&browser), Some(TYPED));

    let (mut keyed, _) = crate::browser::fixture::browser();
    assert!(keyed.phone_key(true, STRAY, START));
    for (digit, at) in [("1", 0), ("0", 1), ("3", 2)] {
        assert!(keyed.phone_key(true, digit, START + DIGITS_TIMEOUT_MILLISECONDS + at));
    }
    assert_eq!(chosen(&keyed), Some(OTHER));
}

// DEC-7225 item 2: a digit made stale by another page is forgotten at the
// next look, not kept on the readout until its time is up.
#[test]
fn a_digit_made_stale_by_another_page_is_forgotten_at_once() {
    let (mut browser, _) = browser();
    type_text(&mut browser, STRAY, START);
    browser.show(PageKey { channel: CHANNEL, page: OTHER }, START + 1);
    assert_eq!(browser.pending_digits(START + 1), "");
    assert_eq!(browser.digits_deadline(START + 1), Some(1));
    assert!(browser.digits_settle(START + 1));
    assert_eq!(browser.digits_deadline(START + 1), None);
}
