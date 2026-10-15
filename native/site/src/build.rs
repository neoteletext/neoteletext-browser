//! Builds a fictional channel of all 800 pages, laid out as the engine's
//! `docs/channels.md` describes a channel (sections 1 and 3): page 100 the
//! front page, each hundred a section with its index at its first page,
//! every page a `NNN.tti` file, `contents.txt` listing every page with its
//! title, and `channel.json` naming the push feed. The pages' bodies come
//! from sample recordings, decoded by the teletext engine; each keeps its
//! hundred where the samples allow, so news stays in 1xx and sport in 3xx.
//!
//! The samples are broadcast recordings: what this writes stays on this
//! computer and is never committed.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use decoder::{Engine, Exported};

/// The fictional channel.
pub const NAME: &str = "Halcyon Teletext";
const SHORT_NAME: &str = "HALCYON";
const SECTIONS: [&str; 8] = ["News", "Business", "Sport", "Weather and travel", "Entertainment", "Television", "Lifestyle", "Service"];
/// A contents file title is at most 32 printable ASCII characters (the
/// engine's `contents::MAX_TITLE`).
const MAX_TITLE: usize = 32;
const COLUMNS: usize = 40;
/// Text rows of a page; row 0 is the header, row 24 the Fastext row.
const ROWS: usize = 25;
/// Codes below this are spacing attributes (EN 300 706 Table 26), written
/// in TTI as an escape followed by the code plus 0x40.
const FIRST_PRINTABLE: u8 = 0x20;
const ESCAPE: char = '\x1b';
const ESCAPE_OFFSET: u8 = 0x40;
const SEVEN_BITS: u8 = 0x7F;
/// Alpha colours (Table 26), as TTI escape letters.
const WHITE: char = 'G';
const YELLOW: char = 'C';
const CYAN: char = 'F';
const GREEN: char = 'B';
const RED: char = 'A';
const MAGENTA: char = 'E';
const BANK_COLOURS: [char; 8] = [RED, GREEN, YELLOW, CYAN, MAGENTA, 'D', YELLOW, WHITE];
/// How many alternative versions of each live page are kept for the
/// server to rotate through.
const LIVE_VERSIONS: usize = 4;
/// The live pages: those whose subject changes through the day. Each is the
/// first page of a section's stories, matched by words in its sample's
/// title, or else the page at that number.
const LIVE: [(u16, &[&str]); 10] = [
    (0x101, &["NEWS", "HEADLINE"]),
    (0x102, &["NEWS", "WORLD"]),
    (0x150, &["NEWS", "FLASH"]),
    (0x201, &["SHARE", "MARKET", "FT", "CITY", "BUSINESS"]),
    (0x202, &["POUND", "CURRENC", "EXCHANGE", "RATE"]),
    (0x302, &["FOOTBALL", "RESULT", "SCORE"]),
    (0x303, &["CRICKET", "RACING", "GOLF", "TENNIS"]),
    (0x401, &["WEATHER", "FORECAST"]),
    (0x410, &["TRAVEL", "TRAFFIC", "ROAD", "RAIL"]),
    (0x601, &["NOW", "TONIGHT", "TV", "BBC1", "ITV"]),
];

/// One page of the sample pool: its rows, title and where it came from.
#[derive(Clone)]
struct Sample {
    original: u16,
    rows: Vec<[u8; COLUMNS]>,
    title: String,
}

/// Every sample recording given: a `.t42` file, or a folder of per-page
/// T42 files (`binaries/NNN.bin`), decoded by the engine.
fn samples(sources: &[PathBuf]) -> Vec<Sample> {
    let mut pool = Vec::new();
    for source in sources {
        let files: Vec<PathBuf> = if source.join("binaries").is_dir() {
            let mut files: Vec<PathBuf> = fs::read_dir(source.join("binaries")).map(|entries| entries.flatten().map(|entry| entry.path()).collect()).unwrap_or_default();
            files.sort();
            files
        } else {
            vec![source.clone()]
        };
        let mut engine = Engine::new();
        for file in files {
            if let Ok(bytes) = fs::read(&file) {
                engine.feed(&bytes);
            }
        }
        engine.finish();
        for Exported { page, rows, title } in engine.export() {
            // Pages with almost no text make poor samples.
            if title.chars().filter(char::is_ascii_alphanumeric).count() >= 3 {
                pool.push(Sample { original: page, rows, title });
            }
        }
        eprintln!("t42-site: {}: {} pages so far", source.display(), pool.len());
    }
    pool
}

/// A title for the contents file: printable ASCII, at most 32 characters.
fn contents_title(title: &str) -> String {
    let ascii: String = title.chars().map(|character| if character.is_ascii_graphic() || character == ' ' { character } else { ' ' }).collect();
    let words: Vec<&str> = ascii.split_whitespace().collect();
    let mut out = String::new();
    for word in words {
        if out.len() + word.len() + 1 > MAX_TITLE {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() { String::from("Page") } else { out }
}

/// One row of 7-bit codes as TTI text.
fn row_text(row: &[u8; COLUMNS]) -> String {
    let mut out = String::new();
    for &code in row {
        let code = code & SEVEN_BITS;
        if code < FIRST_PRINTABLE {
            out.push(ESCAPE);
            out.push(char::from(code + ESCAPE_OFFSET));
        } else {
            out.push(char::from(code));
        }
    }
    out.trim_end().to_owned()
}

/// The channel's own header row: its name and the page number. The
/// receiver draws columns 0 to 7 and the clock itself.
fn header(page: u16) -> String {
    format!("        {ESCAPE}{CYAN}{SHORT_NAME} {page:03X}")
}

fn tti(page: u16, rows: &[String]) -> String {
    let mut out = format!("DE,{NAME} page {page:03X}\r\nPN,{page:03X}00\r\nSC,0000\r\nPS,8000\r\nOL,0,{}\r\n", header(page));
    for (number, row) in rows.iter().enumerate().skip(1) {
        if !row.is_empty() {
            let _ = write!(out, "OL,{number},{row}\r\n");
        }
    }
    out
}

/// A sample's rows as TTI rows, row 0 replaced by the channel's header.
fn sample_rows(sample: &Sample) -> Vec<String> {
    sample.rows.iter().take(ROWS).map(row_text).collect()
}

/// An index page of the channel's own: a coloured title and a list of
/// page numbers and titles.
fn index_page(title: &str, colour: char, entries: &[(u16, String)]) -> Vec<String> {
    let mut rows = vec![String::new(); ROWS];
    rows[1] = format!("{ESCAPE}{colour}{ESCAPE}\x4d{}", title.to_uppercase());
    rows[3] = format!("{ESCAPE}{WHITE}{NAME}: every page from 100 to 899");
    for (slot, (page, name)) in entries.iter().take(18).enumerate() {
        let short: String = name.chars().take(30).collect();
        rows[5 + slot] = format!("{ESCAPE}{YELLOW}{page:03X}{ESCAPE}{WHITE}{short}");
    }
    rows[24] = format!("{ESCAPE}{RED}News {ESCAPE}{GREEN}Sport {ESCAPE}{YELLOW}Weather {ESCAPE}{CYAN}Index");
    rows
}

pub struct Built {
    pub pages: usize,
    pub live: Vec<u16>,
}

/// Builds the channel into `out`, whose push feed is at `push`.
///
/// # Errors
/// When the samples give no pages, or a file cannot be written.
pub fn build(out: &Path, push: &str, sources: &[PathBuf]) -> Result<Built, String> {
    let pool = samples(sources);
    if pool.is_empty() {
        return Err("the samples gave no pages".into());
    }
    // Samples by their original hundred, each list in a fixed order, and
    // a cursor into every list so each sample is used before any repeats.
    let mut by_hundred: BTreeMap<u16, Vec<&Sample>> = BTreeMap::new();
    for sample in &pool {
        by_hundred.entry(sample.original >> 8).or_default().push(sample);
    }
    let mut used: BTreeMap<u16, usize> = BTreeMap::new();
    let mut everything = 0_usize;
    let mut next_for = |hundred: u16| -> &Sample {
        if let Some(list) = by_hundred.get(&hundred) {
            let cursor = used.entry(hundred).or_insert(0);
            if *cursor < list.len() {
                *cursor += 1;
                return list[*cursor - 1];
            }
        }
        everything += 1;
        &pool[(everything - 1) % pool.len()]
    };

    fs::create_dir_all(out.join("live")).map_err(|error| format!("{}: {error}", out.display()))?;
    let write = |name: &str, text: &str| fs::write(out.join(name), text).map_err(|error| format!("{name}: {error}"));
    let mut contents = format!("# {NAME}: every page from 100 to 899\n");
    let mut sections: Vec<(String, Vec<(u16, String)>)> = Vec::new();
    let mut live = Vec::new();
    for (bank, section) in SECTIONS.iter().enumerate() {
        let magazine = u16::try_from(bank + 1).unwrap_or(1);
        let mut entries = Vec::new();
        for slot in 1..100_u16 {
            let (tens, ones) = (slot / 10, slot % 10);
            let page = magazine << 8 | tens << 4 | ones;
            let rule = LIVE.iter().find(|(number, _)| *number == page);
            let sample = match rule {
                // A live page takes a sample whose title fits its subject.
                Some((_, words)) => pool
                    .iter()
                    .find(|sample| words.iter().any(|word| sample.title.to_uppercase().contains(word)))
                    .unwrap_or_else(|| next_for(magazine)),
                None => next_for(magazine),
            };
            let title = contents_title(&sample.title);
            write(&format!("{page:03X}.tti"), &tti(page, &sample_rows(sample)))?;
            if let Some((_, words)) = rule {
                // Alternative versions for the server to rotate through.
                let versions: Vec<&Sample> = pool
                    .iter()
                    .filter(|candidate| words.iter().any(|word| candidate.title.to_uppercase().contains(word)))
                    .chain(by_hundred.get(&magazine).into_iter().flatten().copied())
                    .take(LIVE_VERSIONS)
                    .collect();
                for (version, candidate) in versions.iter().enumerate() {
                    write(&format!("live/{page:03X}-{version}.tti"), &tti(page, &sample_rows(candidate)))?;
                }
                live.push(page);
            }
            entries.push((page, title));
        }
        let first = magazine << 8;
        let colour = BANK_COLOURS[bank];
        let index = if first == 0x100 {
            // The front page lists the sections.
            let listed: Vec<(u16, String)> = SECTIONS.iter().zip(1_u16..).map(|(name, magazine)| (magazine << 8, (*name).to_owned())).collect();
            index_page(NAME, colour, &listed)
        } else {
            index_page(section, colour, &entries)
        };
        write(&format!("{first:03X}.tti"), &tti(first, &index))?;
        entries.insert(0, (first, if first == 0x100 { String::from("Front page") } else { format!("{section} index") }));
        sections.push(((*section).to_owned(), entries));
    }
    for (section, entries) in &sections {
        let _ = writeln!(contents, "s\t{section}");
        for (page, title) in entries {
            let _ = writeln!(contents, "p\t{page:03X}\t{title}");
        }
    }
    write("contents.txt", &contents)?;
    write("channel.json", &format!("{{\n  \"name\": \"{NAME}\",\n  \"initial_page\": \"100\",\n  \"push\": {{ \"address\": \"{push}\", \"method\": \"sse\" }}\n}}\n"))?;
    let pages = sections.iter().map(|(_, entries)| entries.len()).sum();
    Ok(Built { pages, live })
}
