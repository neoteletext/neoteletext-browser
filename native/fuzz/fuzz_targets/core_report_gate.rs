//! The trusted core as the gate of a page's rule table (neoteletext-common's
//! page-rules.md RULE-52, RULE-83; REQ-6523): any sequence of decoder
//! messages and viewer events never makes the trusted core send an event
//! it did not receive from the viewer for that page, send anything for a
//! page whose decoder told no table, or exceed its budgets.
//!
//! The input is a script. Each step is a command byte and its operands:
//! the viewer presses a key, the decoder says what a table asks for, asks
//! for a report, answers a state query or reports a fault, time passes,
//! or the viewer opens another page. The decoder's steps are a hostile
//! decoder's: every field is the script's.
#![no_main]

use std::collections::BTreeSet;

use interface_core::canvas::PageKey;
use libfuzzer_sys::fuzz_target;
use neoteletext::typing::{Send, Typing};
use neoteletext_input::control::{Control, MAX_BODY, write_body};
use neoteletext_input::event::{Action, Event, Item, KeyCode, is_page_input, read_message};
use neoteletext_input::rules::RuleBody;
use protocol::interaction::{InputIntent, Intent};
use protocol::rules::{INTEREST_ALL, NO_RULE, PageSub, Report, Reported, RuleInput, RulesOpen, SEND_BUDGET, SEND_WINDOW_TICKS, TIMED_SEND_BUDGET};

/// The view's channel.
const CHANNEL: u32 = 7;
/// The pages the viewer opens and the decoder names: two sub-pages of one
/// page, each exactly a page of its own (RULE-112, DEC-6599).
const PAGES: [PageSub; 2] = [PageSub::new(0x100, 0), PageSub::new(0x100, 1)];
/// Microseconds in a second.
const SECOND: u64 = 1_000_000;

/// What the script's run has seen.
#[derive(Default)]
struct Seen {
    /// Each key the trusted core gave a decoder: the page and the character.
    given: BTreeSet<(PageSub, u32)>,
    /// The pages whose decoder was given that the page gained or lost the
    /// keyboard.
    focus_given: BTreeSet<PageSub>,
    /// Reports sent that carry an input event, and those that carry none.
    with_input: u64,
    without: u64,
    /// The pages a decoder was accepted as having a table for.
    asked: BTreeSet<PageSub>,
    /// The pages that stopped being the open page while they had a table,
    /// and when: their decoder's final tick may still report, for
    /// `SEND_WINDOW_TICKS` (RULE-5's `close` rules).
    closing: Vec<(PageSub, u64)>,
    /// The script's time.
    now_us: u64,
}

/// The time after a page stops being the open page within which its
/// decoder's final tick may report: `SEND_WINDOW_TICKS` sixtieths of a
/// second, rounded up.
const CLOSING_US: u64 = SEND_WINDOW_TICKS as u64 * SECOND / 60 + 1;

impl Seen {
    /// Reads what one call sent, and holds it to the gate's rules.
    fn take(&mut self, sends: &[Send]) {
        for send in sends {
            match send {
                Send::Decoder { page, input: RuleInput::Key { code, .. }, .. } => {
                    assert!(self.asked.contains(page), "input given for a page with no table");
                    self.given.insert((*page, *code));
                }
                Send::Decoder { page, input: RuleInput::Focus | RuleInput::Blur, .. } => {
                    self.focus_given.insert(*page);
                }
                Send::Fetcher { bytes, .. } => {
                    let read = read_message(bytes, &mut |item: Item<'_>| {
                        assert!(!is_page_input(&item), "a viewer's input outside a report");
                        let Item::Event(Event::RuleEvent(rule)) = item else { return };
                        let on = PageSub::new(rule.on.page, rule.on.sub);
                        let closing = self.closing.iter().any(|(page, at)| *page == on && self.now_us.saturating_sub(*at) <= CLOSING_US);
                        assert!(self.asked.contains(&on) || closing, "a report for a page with no table");
                        match rule.body {
                            RuleBody::Input(_) => {
                                self.with_input += 1;
                                // Built from the core's own copy: a key the
                                // viewer pressed on that page or the page's
                                // own gain or loss of the keyboard, and
                                // nothing else.
                                match rule.input() {
                                    Some((Item::Event(Event::Key(key)), None)) => {
                                        let KeyCode::Character(character) = key.key else { panic!("a key the viewer did not press") };
                                        assert_eq!(key.on, rule.on);
                                        assert!(self.given.contains(&(on, u32::from(character))), "a key that was not given for the page");
                                    }
                                    Some((Item::Event(Event::FocusIn | Event::FocusOut), None)) => {
                                        assert!(self.focus_given.contains(&on), "a focus change that was not given for the page");
                                    }
                                    other => panic!("a report of something the viewer did not do: {other:?}"),
                                }
                            }
                            RuleBody::Fired | RuleBody::Value { .. } => self.without += 1,
                        }
                    });
                    assert!(read.is_ok(), "the trusted core wrote what the channel cannot read");
                }
                _ => {}
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let mut typing = Typing::default();
    typing.input(CHANNEL, true, true, 0);
    let mut open = Some(PAGES[0]);
    typing.opened(Some(CHANNEL), Some(PageKey { channel: CHANNEL, page: PAGES[0].page }), 0);
    let welcome = Control::Welcome { version: neoteletext_input::VERSION, caps: neoteletext_input::caps::SELECT_CAP, resume: None };
    let mut body = [0_u8; MAX_BODY];
    let Ok(length) = write_body(&welcome, &mut body) else { return };
    typing.control(CHANNEL, body.get(..length).unwrap_or(&[]), 0);
    typing.drawn(PageKey { channel: CHANNEL, page: PAGES[0].page }, PAGES[0].sub);

    let mut seen = Seen::default();
    let mut now_us = 1_u64;
    let mut script = data.iter().copied();
    let mut next = || script.next();
    while let Some(command) = next() {
        seen.now_us = now_us;
        let page = PAGES[usize::from(command >> 7)];
        match command & 0x07 {
            // The viewer presses a key on a page.
            0 => {
                let Some(letter) = next() else { break };
                let character = char::from(b'a' + letter % 26);
                let intent = Intent::Key { action: Action::Press, key: KeyCode::Character(character), mods: 0, text: Some(character.to_string()), base: None };
                let sends = typing.input_intent(&InputIntent { key: PageKey { channel: CHANNEL, page: page.page }, intent }, now_us);
                seen.take(&sends);
            }
            // The decoder says what a page's table asks for.
            1 => {
                let Some(generation) = next() else { break };
                let told = RulesOpen {
                    page,
                    generation: u16::from(generation),
                    interest: if generation == 0 { 0 } else { INTEREST_ALL },
                    watch: Vec::new(),
                    wheel: Vec::new(),
                    keys: Vec::new(),
                };
                let (sends, accepted) = typing.rules_told(CHANNEL, &told, now_us);
                // Taken only for the open page: a decoder cannot make
                // another page the one whose input is given.
                assert!(!accepted || open == Some(page));
                if accepted {
                    seen.asked.insert(page);
                }
                // The end of a table ends what was asked for its page.
                if generation == 0 {
                    seen.asked.remove(&page);
                    seen.closing.retain(|(closing, _)| *closing != page);
                }
                seen.take(&sends);
            }
            // The decoder asks for a report: every field its own.
            2 => {
                let (Some(generation), Some(rule), Some(kind), Some(sequence), Some(row)) = (next(), next(), next(), next(), next()) else { break };
                let what = match kind % 3 {
                    0 => Reported::Fired,
                    1 => Reported::Input { sequence: u32::from(sequence) },
                    _ => Reported::Value { value: [1, sequence % 32, 0, 0, 0, 0, 0, 0], reading: i32::from(kind) },
                };
                // RULE-46: a run no rule made has neither rule nor row.
                let (rule, row) = if rule == NO_RULE { (NO_RULE, NO_RULE) } else { (rule % 64, row % 64) };
                let report = Report { page, generation: u16::from(generation), rule, row, tick: u32::from(kind), target_kind: 0, target_number: 0, what };
                seen.take(&typing.report(CHANNEL, &report, now_us));
            }
            // The decoder answers a state query nobody asked, or one asked.
            3 => {
                let Some(identifier) = next() else { break };
                if identifier & 1 == 0 {
                    typing.rules_query(CHANNEL, page, u32::from(identifier));
                }
                let mut answer = u32::from(identifier).to_be_bytes().to_vec();
                answer.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
                seen.take(&typing.page_state(CHANNEL, page, u32::from(identifier), &answer, now_us));
            }
            // The decoder reports a fault.
            4 => {
                let (Some(fault), Some(count)) = (next(), next()) else { break };
                seen.take(&typing.rules_fault(CHANNEL, page, 1, fault, u16::from(count), now_us));
            }
            // Time passes: up to a quarter of a second.
            5 => {
                let Some(ticks) = next() else { break };
                now_us += u64::from(ticks) * 1_000;
            }
            // The viewer opens the other page, or none.
            _ => {
                let before = open;
                open = (command & 0x40 == 0).then_some(page);
                let key = open.map(|page| PageKey { channel: CHANNEL, page: page.page });
                let sends = typing.opened(Some(CHANNEL), key, now_us);
                // What is held on the page left goes up, lost, for its
                // table's rules (RULE-8), before the page has no table.
                seen.take(&sends);
                // A page that stopped being the open page has no table
                // until its decoder says so again; the same page opened
                // again changes nothing.
                if open != before {
                    let left = core::mem::take(&mut seen.asked);
                    seen.closing.extend(left.into_iter().map(|page| (page, now_us)));
                }
                // The sub-page on screen is the one the page is drawn with.
                if let (Some(key), Some(page)) = (key, open) {
                    seen.take(&typing.drawn(key, page.sub));
                }
            }
        }
        // The budgets (RULE-52 item 5): each holds at most a second's
        // worth, and earns its rate.
        let seconds = now_us / SECOND + 1;
        assert!(seen.with_input + seen.without <= u64::from(SEND_BUDGET) * (seconds + 1), "more reports than the budget");
        assert!(seen.without <= u64::from(TIMED_SEND_BUDGET) * (seconds + 1), "more reports with no input than the budget");
    }
});
