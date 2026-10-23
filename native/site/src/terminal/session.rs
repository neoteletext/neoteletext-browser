//! A viewer's session: what each item of input does, the page selection
//! convention, the sounds sent and played, and the frames of the game.

use std::fmt::Write as _;

use neoteletext_audio::Codec;
use neoteletext_audio::encode::{Command, Sound, packets};
use neoteletext_input::control::{Control, packet_of};
use neoteletext_input::digits::decimal_digit;
use neoteletext_input::event::{ALT, Action, CONTROL, Event, Item, Key, KeyCode, MouseAction, SHIFT, units};
use neoteletext_input::keys;

use super::invaders::{FIELD_LEFT, FIELD_RIGHT, Invaders, SHIP_ROW};
use super::pages::{Field, index};
use super::sounds::{
    AUDIO_CHANNELS, AUDIO_SERVICE, BELL_HANDLE, CLICK_HANDLE, FIRE_HANDLE, FIRST_HANDLE, HIT_HANDLE, LOSE_HANDLE, NOTE_BLOCK, NOTE_FADE_MS, NOTE_GAIN,
    NOTE_RATE, PIANO, WIN_HANDLE, library,
};
use super::tti::{ALPHA_CYAN, ALPHA_MAGENTA, ALPHA_WHITE, ALPHA_YELLOW, Page, Row, USED};
use super::{PAGES, Session, Settings};
use crate::podcast::{self, Episode, Player};

/// What a session sends.
pub(super) enum Push {
    /// A TTI page.
    Page(String),
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
            fields: [
                Field { row: 5, column: 13, width: 20, text: String::new(), masked: false },
                Field { row: 8, column: 13, width: 12, text: String::new(), masked: true },
            ],
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
        }
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
        page.acked = self.acked;
        page
    }

    fn go(&mut self, page: u16) {
        if page != self.page {
            self.previous.push(self.page);
            self.page = page;
        }
    }

    /// The videotex convention (INPUT.md section 9.4), and three
    /// plain digits where the page has no field. Returns whether the key
    /// was part of a page selection.
    fn select(&mut self, character: char) -> bool {
        let field_page = self.page == 0x200;
        // The decimal digits of any script select as 0 to 9 (INPUT.md
        // section 5.7 rule 8).
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
                    && (0x100..=0x8FF).contains(&page)
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
                        && (0x100..=0x899).contains(&page)
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

    /// Acts on one item of the input stream; returns what to push.
    pub(super) fn item(&mut self, item: &Item<'_>) -> Vec<Push> {
        // The input count (INPUT.md section 7.1): a character or an event
        // counts one, modulo 65 536.
        self.acked = self.acked.wrapping_add(u16::try_from(units(item) & u32::from(u16::MAX)).unwrap_or(0));
        let mut pushes = Vec::new();
        match item {
            Item::Text(text) | Item::Event(Event::Paste(text)) => {
                for character in text.chars() {
                    pushes.extend(self.character(character));
                }
            }
            Item::Event(Event::Hello { version, caps, .. }) => return self.welcome(*version, *caps),
            Item::Event(Event::Page(0)) => {}
            Item::Event(Event::Page(page)) => {
                self.page = *page;
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
        pushes.push(Push::Page(self.render().tti()));
        pushes
    }

    /// HELLO: WELCOME back with what the channel uses of what is offered,
    /// as a control record (INPUT.md section 7.2).
    fn welcome(&mut self, version: u32, caps: u32) -> Vec<Push> {
        if version != neoteletext_input::VERSION {
            return Vec::new();
        }
        let welcome = Control::Welcome { version, caps: caps & USED };
        let Ok(packet) = packet_of(&welcome, AUDIO_SERVICE, self.record_id) else { return Vec::new() };
        self.record_id = self.record_id.wrapping_add(1);
        vec![Push::Packets(packet.bytes.to_vec())]
    }

    /// Everything held is released: the game's ship stops, the note ends.
    fn release_all(&mut self) {
        self.invaders.left = false;
        self.invaders.right = false;
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
            // A press moves at once; held, the ship keeps moving each frame.
            (0x300, Some("ArrowLeft"), _) => {
                self.invaders.left = true;
                self.invaders.ship = (self.invaders.ship - 1).max(FIELD_LEFT);
            }
            (0x300, Some("ArrowRight"), _) => {
                self.invaders.right = true;
                self.invaders.ship = (self.invaders.ship + 1).min(FIELD_RIGHT - 2);
            }
            (0x300, Some("Enter"), _) if self.invaders.over.is_some() => self.invaders = Invaders::new(),
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
            (0x200, Some("Tab"), _) => self.focus = (self.focus + 1) % self.fields.len(),
            (0x200, Some("Backspace"), _) => {
                self.fields[self.focus].text.pop();
            }
            (0x200, Some("Escape"), _) => {
                for field in &mut self.fields {
                    field.text.clear();
                }
                self.greeting.clear();
            }
            (0x200, Some("Enter"), _) => {
                let name = self.fields[0].text.trim().to_owned();
                self.greeting = if name.is_empty() { "Type your name first.".to_owned() } else { format!("Hello, {name}!") };
            }
            // A KEY with text, when every key is asked for.
            (_, _, Some(_)) if !control && !repeat => {
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
        match key.key {
            KeyCode::Functional(code) if keys::name(code) == Some("ArrowLeft") => self.invaders.left = false,
            KeyCode::Functional(code) if keys::name(code) == Some("ArrowRight") => self.invaders.right = false,
            _ => {}
        }
        if self.page == 0x400 {
            self.piano = None;
        }
    }

    /// A character typed: Space fires and plays, a digit or `*` or `#`
    /// selects a page, and on the typing page it goes into the field.
    fn character(&mut self, character: char) -> Vec<Push> {
        match (self.page, character) {
            (0x300, ' ') => {
                if self.invaders.shots.len() < 3 && self.invaders.over.is_none() {
                    self.invaders.shots.push((self.invaders.ship + 1, SHIP_ROW - 1));
                    return vec![self.play(FIRE_HANDLE)];
                }
                return Vec::new();
            }
            (0x700, ' ') => return self.listen(Player::toggle),
            _ => {}
        }
        if self.select(character) {
            return Vec::new();
        }
        if self.page == 0x200 {
            let field = &mut self.fields[self.focus];
            if field.text.chars().count() < usize::from(field.width) && character.is_ascii() && !character.is_ascii_control() {
                field.text.push(character);
            }
            return vec![self.play(CLICK_HANDLE)];
        }
        Vec::new()
    }

    /// Every sound of the channel, as packets, one message a sound: the
    /// piano's notes and the game's effects.
    fn sounds(&mut self) -> Vec<Push> {
        library()
            .into_iter()
            .filter_map(|(handle, pcm)| {
                let sound = Sound::new(handle, 1, Codec::ImaAdpcm, NOTE_RATE, NOTE_BLOCK, &pcm).ok()?;
                let mut records = vec![sound.object_record().ok()?];
                records.extend(sound.data_records());
                let (packets, next) = packets(AUDIO_SERVICE, self.record_id, &records);
                self.record_id = next;
                Some(Push::Packets(packets.iter().flat_map(|packet| packet.bytes).collect()))
            })
            .collect()
    }

    /// The command that plays a sound, on the next of the effects' channels.
    fn play(&mut self, handle: u8) -> Push {
        let channel = self.next_channel;
        self.next_channel = (self.next_channel + 1) % AUDIO_CHANNELS;
        let sequence = &mut self.audio_sequence[usize::from(channel)];
        *sequence = sequence.wrapping_add(1);
        let command = Command {
            channel,
            sequence: *sequence,
            operation: 1,
            handle,
            generation: 1,
            gain: NOTE_GAIN,
            fade_ms: NOTE_FADE_MS,
            flags: 0,
            duck: 0,
            page: None,
        };
        let (packets, next) = packets(AUDIO_SERVICE, self.record_id, &[command.record()]);
        self.record_id = next;
        Push::Packets(packets.iter().flat_map(|packet| packet.bytes).collect())
    }

    /// Does `action` to this viewer's podcast: what it sends, pushed.
    fn listen(&mut self, action: impl FnOnce(&mut Player, &Episode, &mut u8) -> Vec<u8>) -> Vec<Push> {
        let Some((episode, player)) = &mut self.podcast else { return Vec::new() };
        let bytes = action(player, episode, &mut self.record_id);
        if bytes.is_empty() { Vec::new() } else { vec![Push::Packets(bytes)] }
    }

    /// The podcast's next blocks, wherever the viewer is, and page 700
    /// again each second and when it stops, while it is open.
    pub(super) fn podcast_frame(&mut self) -> Vec<Push> {
        let mut pushes = self.listen(Player::stream);
        if self.page == 0x700
            && let Some((episode, player)) = &self.podcast
        {
            let now = (episode.seconds(player.position(episode)), player.playing());
            if self.shown.replace(now) != Some(now) {
                pushes.push(Push::Page(self.render().tti()));
            }
        }
        pushes
    }

    /// One frame of the game: its sounds, then its page.
    pub(super) fn frame(&mut self) -> Vec<Push> {
        let happened = self.invaders.tick();
        let mut pushes = Vec::new();
        if happened.hit {
            pushes.push(self.play(HIT_HANDLE));
        }
        match happened.over {
            Some(true) => pushes.push(self.play(WIN_HANDLE)),
            Some(false) => pushes.push(self.play(LOSE_HANDLE)),
            None => {}
        }
        pushes.push(Push::Page(self.render().tti()));
        pushes
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

#[cfg(test)]
mod tests;
