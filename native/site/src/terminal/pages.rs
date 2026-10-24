//! The channel's pages: the index, typing, the piano, links, the phone
//! keyboard and the podcast, each drawn from the session's state; and the
//! page and contents files a viewer fetches first.

use std::fmt::Write as _;

use neoteletext_input::cursor::{ECHO, MASKED};

use super::sounds::PIANO;
use super::tti::{
    ALPHA_CYAN, ALPHA_GREEN, ALPHA_MAGENTA, ALPHA_WHITE, ALPHA_YELLOW, BLACK_BACKGROUND, COLUMNS, DOUBLE_HEIGHT, MOSAIC_BLUE, MOSAIC_CYAN, MOSAIC_RED,
    MOSAIC_WHITE, Page, Row,
};
use super::{PAGES, Session, Settings};
use crate::podcast;

/// The podcast's progress bar, in cells: with its two colour codes, the
/// width of a row less a margin.
const PROGRESS_CELLS: usize = 36;
/// A solid mosaic cell (EN 300 706 Table 48: all six sixels set).
const SOLID_MOSAIC: char = '\x7f';
const SECONDS_PER_MINUTE: u64 = 60;

/// A text field of the typing page.
pub(super) struct Field {
    pub(super) row: u8,
    pub(super) column: u8,
    pub(super) width: u8,
    pub(super) text: String,
    pub(super) masked: bool,
}

impl Session {
    pub(super) fn typing(&self) -> Page {
        let mut page = Page::new(0x200, "Type into a page");
        page.row(3, Row::new().code(ALPHA_WHITE).text("Type: each key goes to this channel."));
        for (index, field) in self.fields.iter().enumerate() {
            let label = if field.masked { "Secret:" } else { "Your name:" };
            let shown: String = if field.masked { "*".repeat(field.text.chars().count()) } else { field.text.clone() };
            let mut row = Row::new().code(ALPHA_YELLOW).text(label).at(usize::from(field.column) - 1).code(ALPHA_WHITE).text(&shown);
            if index == self.focus {
                row = row.at(usize::from(field.column + field.width)).code(ALPHA_CYAN).text("<");
            }
            page.row(field.row, row);
        }
        page.row(11, Row::new().code(ALPHA_GREEN).text(&self.greeting));
        page.row(14, Row::new().code(ALPHA_WHITE).text("Tab moves between fields; Enter sends."));
        page.row(15, Row::new().code(ALPHA_WHITE).text("Letters show at once, provisionally, then"));
        page.row(16, Row::new().code(ALPHA_WHITE).text("as the channel draws them: local echo."));
        page.row(18, Row::new().code(ALPHA_MAGENTA).text("*100# for the index, Esc clears."));
        let field = &self.fields[self.focus];
        let length = u8::try_from(field.text.chars().count()).unwrap_or(field.width).min(field.width);
        if length < field.width {
            page.set_cursor((field.row, field.column + length, field.width - length, ECHO | if field.masked { MASKED } else { 0 }));
        } else {
            page.set_cursor((field.row, field.column + length - 1, 0, 0));
        }
        page
    }

    pub(super) fn piano(&self) -> Page {
        let mut page = Page::new(0x400, "Piano");
        page.row(3, Row::new().code(ALPHA_WHITE).text("Keys A S D F G H J K play the notes"));
        page.row(4, Row::new().code(ALPHA_WHITE).text("C to C. Each press is a sound command"));
        page.row(5, Row::new().code(ALPHA_WHITE).text("pushed on this page's own WebSocket."));
        for line in 0..6_u8 {
            let mut row = Row::new().at(2);
            for (index, (_, letter, _)) in PIANO.iter().enumerate() {
                let pressed = self.piano == Some(index);
                row = row.code(if pressed { MOSAIC_RED } else { MOSAIC_WHITE });
                if line == 5 {
                    row = row.code(ALPHA_YELLOW).text(&letter.to_uppercase().to_string()).text(" ");
                } else {
                    row = row.text("\x7f\x7f\x7f");
                }
            }
            page.row(9 + line, row.code(BLACK_BACKGROUND));
        }
        page
    }

    pub(super) fn links(&self) -> Page {
        let mut page = Page::new(0x500, "Links and clicks");
        page.row(3, Row::new().code(ALPHA_WHITE).text("Page numbers are links: 100 200 300"));
        page.row(4, Row::new().code(ALPHA_WHITE).text("Command-click one to follow it."));
        page.row(6, Row::new().code(ALPHA_CYAN).text("The teletext engine on the web"));
        page.link("LT,6,1,30,web:https://example.com/".to_owned());
        page.row(7, Row::new().code(ALPHA_WHITE).text("(Command-click opens your web browser)"));
        page.row(10, Row::new().code(ALPHA_YELLOW).text("A plain click is this channel's:"));
        let clicked = self.click.map_or_else(|| "Click anywhere on the page.".to_owned(), |(column, row)| format!("You clicked column {column}, row {row}."));
        page.row(11, Row::new().code(ALPHA_GREEN).text(&clicked));
        page
    }

    pub(super) fn phone_page(&self) -> Page {
        let mut page = Page::new(0x600, "Your phone as keyboard");
        let Some(phone) = &self.phone else {
            let why = if self.relay.is_some() { "Preparing the QR code..." } else { "Start neoteletext-site with --relay to try this." };
            page.row(4, Row::new().code(ALPHA_WHITE).text(why));
            return page;
        };
        for (index, row) in phone.rows().iter().enumerate() {
            if let Ok(number) = u8::try_from(3 + index) {
                if number > 20 {
                    break;
                }
                page.row(number, Row::from_codes(row.clone()));
            }
        }
        page.row(22, Row::new().code(ALPHA_WHITE).text(phone.state()));
        page.row(21, Row::new().code(ALPHA_GREEN).text(&format!("Phone typed: {}", phone.typed())));
        page
    }

    pub(super) fn podcast_page(&self) -> Page {
        let mut page = Page::new(0x700, "Podcast");
        let Some((episode, player)) = &self.podcast else {
            page.row(4, Row::new().code(ALPHA_WHITE).text("Start neoteletext-site with --podcast FILE.wav"));
            page.row(5, Row::new().code(ALPHA_WHITE).text("to listen here."));
            return page;
        };
        let title: String = episode.title.chars().take(COLUMNS - 2).collect();
        page.row(3, Row::new().code(ALPHA_YELLOW).code(DOUBLE_HEIGHT).text(&title));
        let (position, length) = (player.position(episode), episode.samples.len());
        let state = match (player.playing(), position) {
            (true, _) => "Playing",
            (false, 0) => "Ready: press Space",
            (false, at) if at >= length => "Finished",
            (false, _) => "Paused",
        };
        page.row(6, Row::new().code(ALPHA_GREEN).text(state));
        let clock = |samples: usize| {
            let seconds = episode.seconds(samples);
            format!("{}:{:02}", seconds / SECONDS_PER_MINUTE, seconds % SECONDS_PER_MINUTE)
        };
        page.row(8, Row::new().code(ALPHA_WHITE).text(&format!("{} / {}", clock(position), clock(length))));
        let filled = position.saturating_mul(PROGRESS_CELLS).checked_div(length).unwrap_or(0).min(PROGRESS_CELLS);
        let bar = |cells: usize| SOLID_MOSAIC.to_string().repeat(cells);
        for row in [10, 11] {
            page.row(row, Row::new().code(MOSAIC_CYAN).text(&bar(filled)).code(MOSAIC_BLUE).text(&bar(PROGRESS_CELLS - filled)));
        }
        page.row(14, Row::new().code(ALPHA_CYAN).text("Space plays and pauses."));
        page.row(15, Row::new().code(ALPHA_CYAN).text(&format!("Left and Right skip {} seconds.", podcast::SKIP_SECONDS)));
        page.row(16, Row::new().code(ALPHA_CYAN).text("Enter starts it again."));
        page.row(18, Row::new().code(ALPHA_MAGENTA).text("It plays on while you read other pages."));
        page
    }
}

pub(super) fn index() -> Page {
    let mut page = Page::new(0x100, "Terminal channel");
    page.row(3, Row::new().code(ALPHA_WHITE).text("This channel takes your keys, as a"));
    page.row(4, Row::new().code(ALPHA_WHITE).text("terminal's host does."));
    for (index, (number, title)) in PAGES.iter().skip(1).enumerate() {
        let row = u8::try_from(6 + index).unwrap_or(6);
        page.row(row, Row::new().code(ALPHA_YELLOW).text(&format!("{number:03X}")).code(ALPHA_WHITE).text(title));
    }
    page.row(13, Row::new().code(ALPHA_CYAN).text("Choose as on Prestel: *nnn# for a page,"));
    page.row(14, Row::new().code(ALPHA_CYAN).text("# the next, *# back, *00 again; or type"));
    page.row(15, Row::new().code(ALPHA_CYAN).text("three digits."));
    page.row(17, Row::new().code(ALPHA_GREEN).text("Command-Escape leaves; so does closing"));
    page.row(18, Row::new().code(ALPHA_GREEN).text("the page."));
    page
}

/// The page files a viewer fetches before the session starts.
pub(super) fn page_file(number: u16, settings: &Settings) -> Option<String> {
    PAGES.iter().any(|(page, _)| *page == number).then(|| {
        let mut session = Session::new(settings.clone());
        session.page = number;
        session.render().tti()
    })
}

/// The channel's contents in the engine's format (its REQ-WEB-025): a
/// section, then each page.
pub(super) fn contents_file() -> String {
    let mut contents = String::from("s\tTerminal\n");
    for (number, title) in PAGES {
        let _ = writeln!(contents, "p\t{number:03X}\t{title}");
    }
    contents
}
