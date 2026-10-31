//! A page's links at the longest address (neoteletext-common's REQ-1661):
//! a page file's, and a LINKS record's, which do not outlive their message.

use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_formats::page::{LinkDestination, LinkKey, LinkLine, LinkText, TextLink};
use neoteletext_formats::text::Text;
use neoteletext_formats::tti::link_destination;
use neoteletext_web::records::links::{SubPageLinks, encode};
use neoteletext_web::records::{ChannelRecord, Read, RecordReader};

use super::{Lines, LinkLines, PageFile};

const PAGE: u16 = 0x100;
/// An address of RFC 9110's 8,000 octets, written out so that this says the
/// requirement.
const LONGEST: usize = 8_000;
const SERVICE: u8 = 2;

fn address(mark: char) -> String {
    let mut text = String::from("https://a.example/");
    text.push(mark);
    text.extend(core::iter::repeat_n('a', LONGEST.saturating_sub(text.len())));
    text
}

fn key(key: LinkKey, destination: LinkDestination) -> LinkLine {
    let LinkDestination { service, address, subcode, url } = destination;
    LinkLine { key, service, address, subcode, url, label: Text::new(), label_truncated: false, line: 0 }
}

fn text_link(row: u8, destination: LinkDestination) -> TextLink {
    TextLink { row, column: 0, length: 4, destination, line: 0 }
}

/// Reads the records of `links` as a decoder does, each record dropped once
/// it is read.
fn read_all(links: SubPageLinks<'_>) -> Lines {
    let page = PageAddress::from_mpp(PAGE).unwrap();
    let mut packets = Vec::new();
    encode(page, SubCode::new(0), links, SERVICE, 0, &mut |packet| packets.push(packet)).unwrap();
    let (mut held, mut reader) = (Lines::default(), RecordReader::new());
    for packet in &packets {
        if let Read::Record(ChannelRecord::Links(record)) = reader.push(packet) {
            held.read_record(&record);
        }
    }
    held
}

fn keys_addresses(lines: &LinkLines) -> Vec<Vec<u8>> {
    let file = lines.key_file().unwrap();
    file.links.as_slice().iter().map(|line| file.link_text.address(line.url).to_vec()).collect()
}

#[test]
fn a_page_files_link_of_the_longest_address_is_kept() {
    let long = address('x');
    let mut file = PageFile::new();
    let destination = link_destination(long.as_bytes(), &mut file.link_text).unwrap();
    file.text_links.push(text_link(2, destination));
    file.address = PageAddress::from_mpp(PAGE).unwrap();
    let mut lines = Lines::default();
    lines.read(&file);
    assert_eq!(lines.get(PAGE, 0).unwrap().text[0].3, long);
}

/// REQ-1661 and DEC-1668: every link of a sub-page at the bound, as records
/// that have gone by the time the links are used.
#[test]
fn a_records_links_keep_their_addresses_when_the_record_has_gone() {
    let mut text = LinkText::new();
    let targets: Vec<String> = ('a'..='r').map(address).collect();
    let mut destinations = targets.iter().map(|target| link_destination(target.as_bytes(), &mut text).unwrap());
    let keys = [LinkKey::Red, LinkKey::Green].map(|colour| key(colour, destinations.next().unwrap()));
    let texts: Vec<TextLink> = (2..10).map(|row| text_link(row, destinations.next().unwrap())).collect();
    let lines = read_all(SubPageLinks { keys: &keys, numbers: &[], texts: &texts, addresses: &text });
    let held = lines.get(PAGE, 0).unwrap();
    assert_eq!(held.text.iter().map(|(.., target)| target.as_str()).collect::<Vec<_>>(), targets[2..10].iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(keys_addresses(held), [targets[0].as_bytes(), targets[1].as_bytes()]);
}

/// A web link keeps its mark, which makes the target longer than the
/// longest address, and which the trusted core reads.
#[test]
fn a_web_link_of_the_longest_address_keeps_its_mark() {
    let web = format!("web:{}", address('x'));
    let mut text = LinkText::new();
    let texts = [text_link(2, link_destination(web.as_bytes(), &mut text).unwrap())];
    let lines = read_all(SubPageLinks { keys: &[], numbers: &[], texts: &texts, addresses: &text });
    assert_eq!(lines.get(PAGE, 0).unwrap().text[0].3, web);
}
