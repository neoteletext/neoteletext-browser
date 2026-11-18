//! A viewer's session: what each item of input does, the page selection
//! convention, the sounds sent and played, and the frames of the game.

use std::fmt::Write as _;

// The engine's REQ-329: the standard's page ranges are named once, there.
use neoteletext_core::limits::{FIRST_KEYABLE_PAGE, FIRST_PAGE_ADDRESS, LAST_KEYABLE_PAGE, LAST_PAGE_ADDRESS};
use neoteletext_input::control::{Control, packet_of};
use neoteletext_input::digits::decimal_digit;
use neoteletext_input::event::{ALT, Action, CONTROL, Event, Item, Key, KeyCode, MouseAction, SHIFT, units};
use neoteletext_input::keys;

use super::invaders::Invaders;
use super::pages::{index, typing_fields};
use super::sounds::{AUDIO_CHANNELS, AUDIO_SERVICE, BELL_HANDLE, CLICK_HANDLE, FIRST_HANDLE, PIANO};
use super::tti::{ALPHA_CYAN, ALPHA_MAGENTA, ALPHA_WHITE, ALPHA_YELLOW, Page, Row, USED};
use super::{PAGES, Session, Settings};
use crate::podcast::{self, Player};

/// What a session sends.
pub(super) enum Push {
    /// The open page as T42 packets, its cursor record last.
    Page(Vec<u8>),
    /// Packets: sounds and their commands.
    Packets(Vec<u8>),
}

impl Session {
    pub(super) fn new(settings: Settings) -> Self {
        Self {
            page: 0x100,
            previous: Vec::new(),
            acked: 0,
            selection: String::new(),
            last_key: String::new(),
            fields: typing_fields(),
            focus: 0,
            greeting: String::new(),
            invaders: Invaders::new(),
            piano: None,
            sounds_sent: false,
            audio_sequence: [0; AUDIO_CHANNELS as usize],
            next_channel: 0,
            record_id: 0,
            click: None,
            phone: None,
            relay: settings.relay,
            away: false,
            podcast: settings.podcast.map(|episode| (episode, Player::new())),
            shown: None,
            tables_sent: Vec::new(),
        }
    }

    /// The open page as it now stands, as the channel pushes it: T42
    /// packets with its cursor record as a packet 4/31 (neoteletext-common's
    /// DEC-1459), each cursor record a record of its own.
    pub(super) fn page_push(&mut self) -> Push {
        let mut bytes = self.rules();
        let (page, next_record_id) = self.render().packets(AUDIO_SERVICE, self.record_id);
        bytes.extend(page);
        self.record_id = next_record_id;
        Push::Page(bytes)
    }

    /// The open page's rule table, the first time the page is pushed on
    /// this connection (page-rules.md RULE-41): the table that sends every
    /// kind of input, which a server gives a channel whose pages have none
    /// of their own (RULE-53), built by the engine's one function. Without
    /// a table a receiver sends a page no input at all (RULE-50).
    fn rules(&mut self) -> Vec<u8> {
        /// Every table of this channel is its first.
        const GENERATION: u16 = 1;
        let mut bytes = Vec::new();
        if self.tables_sent.contains(&self.page) {
            return bytes;
        }
        // A table is 76 KB: kept off the stack.
        let mut table = Box::new(neoteletext_rules::Table::new());
        neoteletext_rules::default_send_all(false, &mut table);
        let mut body = vec![0_u8; neoteletext_rules::limits::MAX_TABLE_BYTES];
        let Ok(length) = neoteletext_rules::encode(self.page, GENERATION, Some(&table), &mut body) else { return bytes };
        let written = neoteletext_web::records::rules::encode(body.get(..length).unwrap_or(&[]), AUDIO_SERVICE, self.record_id, &mut |packet| {
            bytes.extend_from_slice(&packet.bytes);
        });
        if written.is_ok() {
            self.record_id = self.record_id.wrapping_add(1);
            self.tables_sent.push(self.page);
        }
        bytes
    }

    /// The open page as it now stands.
    pub(super) fn render(&self) -> Page {
        let mut page = match self.page {
            0x100 => index(),
            0x200 => self.typing(),
            0x300 => return self.invaders.page(self.acked),
            0x400 => self.piano(),
            0x500 => self.links(),
            0x600 => self.phone_page(),
            0x700 => self.podcast_page(),
            other => {
                let mut page = Page::new(other, "No such page");
                page.row(4, Row::new().code(ALPHA_WHITE).text("*100# goes back to the index."));
                page
            }
        };
        if self.away {
            page.row(23, Row::new().code(ALPHA_MAGENTA).text("Your keys went elsewhere."));
        } else if !self.selection.is_empty() {
            page.row(23, Row::new().code(ALPHA_YELLOW).text(&format!("Page {}", self.selection)));
        } else if !self.last_key.is_empty() {
            page.row(23, Row::new().code(ALPHA_CYAN).text(&format!("Last key: {}", self.last_key)));
        }
        page.set_acked(self.acked);
        page
    }

    fn go(&mut self, page: u16) {
        if page != self.page {
            self.previous.push(self.page);
            self.page = page;
        }
    }

    /// The videotex convention (`interaction.md` section 7.6), and three
    /// plain digits where the page has no field. Returns whether the key
    /// was part of a page selection.
    fn select(&mut self, character: char) -> bool {
        let field_page = self.page == 0x200;
        // The decimal digits of any script select as 0 to 9 (DOC-1937).
        let character = decimal_digit(character).map_or(character, |digit| char::from(b'0' + digit));
        match character {
            '*' => "*".clone_into(&mut self.selection),
            '#' if self.selection == "*" => {
                if let Some(page) = self.previous.pop() {
                    self.page = page;
                }
                self.selection.clear();
            }
            '#' if self.selection.starts_with('*') => {
                if let Ok(page) = u16::from_str_radix(&self.selection[1..], 16)
                    && (FIRST_PAGE_ADDRESS..=LAST_PAGE_ADDRESS).contains(&page)
                {
                    self.go(page);
                }
                self.selection.clear();
            }
            '#' => {
                let next = PAGES.iter().position(|(number, _)| *number == self.page).map_or(0x100, |index| PAGES[(index + 1) % PAGES.len()].0);
                self.go(next);
            }
            digit @ '0'..='9' if self.selection.starts_with('*') || !field_page => {
                self.selection.push(digit);
                if self.selection == "*00" {
                    self.selection.clear();
                } else if !self.selection.starts_with('*') && self.selection.len() == 3 {
                    if let Ok(page) = u16::from_str_radix(&self.selection, 16)
                        && (FIRST_KEYABLE_PAGE..=LAST_KEYABLE_PAGE).contains(&page)
                    {
                        self.go(page);
                    }
                    self.selection.clear();
                }
            }
            _ => return false,
        }
        true
    }

    /// Acts on one item of the input stream; returns what to push. Under
    /// push protocol 9 a viewer's input comes only as a rule's report
    /// (neoteletext-common's page-rules.md RULE-46): a `RULE_EVENT` counts
    /// one, whatever it carries, and the input it carries is acted on as
    /// that input always was.
    pub(super) fn item(&mut self, item: &Item<'_>) -> Vec<Push> {
        // The input count (DOC-1943): a character or an event
        // counts one, modulo 65 536.
        self.acked = self.acked.wrapping_add(u16::try_from(units(item) & u32::from(u16::MAX)).unwrap_or(0));
        if let Item::Event(Event::RuleEvent(rule)) = item {
            return match rule.input() {
                Some((carried, _)) => self.act(&carried),
                // A firing or a value: no page of this channel asks for one.
                None => Vec::new(),
            };
        }
        self.act(item)
    }

    /// Acts on one item: the session's own, or the input a report carried.
    fn act(&mut self, item: &Item<'_>) -> Vec<Push> {
        let mut pushes = Vec::new();
        match item {
            Item::Text(text) | Item::Event(Event::Paste { text, .. }) => {
                for character in text.chars() {
                    pushes.extend(self.character(character));
                }
            }
            Item::Event(Event::Hello { version, caps, .. }) => return self.welcome(*version, *caps),
            Item::Event(Event::Select(select, _)) if select.page == 0 => {}
            Item::Event(Event::Select(select, _)) => {
                self.page = select.page;
                self.selection.clear();
            }
            Item::Event(Event::FocusIn) => self.away = false,
            // The keys went elsewhere: what was held is let go (REQ-INT-108).
            Item::Event(Event::FocusOut) => {
                self.away = true;
                self.release_all();
            }
            Item::Event(Event::Mouse(mouse)) if mouse.action == MouseAction::Press => {
                self.click = u8::try_from(mouse.at.column).ok().zip(u8::try_from(mouse.at.row).ok());
            }
            Item::Event(Event::Key(key)) if key.action == Action::Release => self.up(key),
            Item::Event(Event::Key(key)) => {
                self.last_key = describe(key);
                pushes.extend(self.down(key));
            }
            Item::Event(_) => {}
        }
        if matches!(self.page, 0x200 | 0x300 | 0x400) && !self.sounds_sent {
            self.sounds_sent = true;
            pushes.splice(0..0, self.sounds());
        }
        if self.page == 0x600 && self.phone.is_none() {
            self.start_phone();
        }
        pushes.push(self.page_push());
        pushes
    }

    /// HELLO: WELCOME back with what the channel uses of what is offered,
    /// as a control record (DOC-1953).
    fn welcome(&mut self, version: u32, caps: u32) -> Vec<Push> {
        if version != neoteletext_input::VERSION {
            return Vec::new();
        }
        let welcome = Control::Welcome { version, caps: caps & USED, resume: None };
        let Ok(packet) = packet_of(&welcome, AUDIO_SERVICE, self.record_id) else { return Vec::new() };
        self.record_id = self.record_id.wrapping_add(1);
        vec![Push::Packets(packet.bytes.to_vec())]
    }

    /// Everything held is released: the game's ship stops, the note ends.
    fn release_all(&mut self) {
        self.invaders.release();
        self.piano = None;
    }

    fn down(&mut self, key: &Key<'_>) -> Vec<Push> {
        let name = match key.key {
            KeyCode::Functional(code) => keys::name(code),
            KeyCode::Character(_) => None,
        };
        let character = match key.key {
            KeyCode::Character(character) => Some(character),
            KeyCode::Functional(_) => None,
        };
        let repeat = key.action == Action::Repeat;
        let control = key.mods & CONTROL != 0;
        // The typing page clicks on every key, as a typewriter does, and
        // rings on Enter.
        let sound = (self.page == 0x200).then_some(if name == Some("Enter") { BELL_HANDLE } else { CLICK_HANDLE });
        match (self.page, name, character) {
            (0x300, _, _) if self.invaders.key_down(name) => {}
            (0x200, _, _) if self.typing_key(name) => {}
            (0x400, _, _) => {
                // By position on any layout, or by letter.
                let letter = key.base.or(character).map(|character| character.to_ascii_lowercase());
                let index = PIANO.iter().position(|(_, piano, _)| letter == Some(*piano));
                if let Some(index) = index
                    && !repeat
                {
                    self.piano = Some(index);
                    return vec![self.play(FIRST_HANDLE + u8::try_from(index).unwrap_or(0))];
                }
                for typed in key.text.unwrap_or_default().chars() {
                    self.select(typed);
                }
            }
            (0x700, Some("ArrowLeft"), _) => return self.listen(|player, episode, record_id| player.skip(episode, -podcast::SKIP_SECONDS, record_id)),
            (0x700, Some("ArrowRight"), _) => return self.listen(|player, episode, record_id| player.skip(episode, podcast::SKIP_SECONDS, record_id)),
            (0x700, Some("Enter"), _) => return self.listen(Player::restart),
            // A key's text types, held keys repeating it (DOC-1935).
            (_, _, Some(_)) if !control => {
                let mut pushes = Vec::new();
                for typed in key.text.unwrap_or_default().chars() {
                    pushes.extend(self.character(typed));
                }
                return pushes;
            }
            _ => {}
        }
        sound.map(|handle| vec![self.play(handle)]).unwrap_or_default()
    }

    fn up(&mut self, key: &Key<'_>) {
        if let KeyCode::Functional(code) = key.key {
            self.invaders.key_up(keys::name(code));
        }
        if self.page == 0x400 {
            self.piano = None;
        }
    }

    /// A character typed: Space fires and plays, a digit or `*` or `#`
    /// selects a page, and on the typing page it goes into the field.
    fn character(&mut self, character: char) -> Vec<Push> {
        match (self.page, character) {
            (0x300, ' ') => return self.fire(),
            (0x700, ' ') => return self.listen(Player::toggle),
            _ => {}
        }
        if self.select(character) {
            return Vec::new();
        }
        if self.page == 0x200 {
            self.fields[self.focus].type_character(character);
            return vec![self.play(CLICK_HANDLE)];
        }
        Vec::new()
    }
}

/// A key's name for the page, as the channel heard it.
fn describe(key: &Key<'_>) -> String {
    let name = match key.key {
        KeyCode::Functional(code) => keys::name(code).unwrap_or("Unidentified").to_owned(),
        // The space bar types a space, which on the page would be nothing.
        KeyCode::Character(' ') => "Space".to_owned(),
        KeyCode::Character(character) => String::from(character),
    };
    let mut text = String::new();
    for (bit, label) in [(CONTROL, "Ctrl-"), (ALT, "Alt-"), (SHIFT, "Shift-")] {
        if key.mods & bit != 0 {
            text.push_str(label);
        }
    }
    text.push_str(&name);
    if let Some(base) = key.base {
        let _ = write!(text, " at {}", base.to_ascii_uppercase());
    }
    text
}

mod audio;
#[cfg(test)]
pub(super) mod tests;
