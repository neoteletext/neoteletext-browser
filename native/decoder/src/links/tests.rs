//! A page's links at the longest address (neoteletext-common's REQ-1661):
//! a page file's, and a LINKS record's, which do not outlive their message.

use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_formats::page::{LinkDestination, LinkKey, LinkLine, LinkText, NumberLink, TextLink};
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

/// The packets of the LINKS records of `links`, for page 100's sub-page
/// 0000, from record id `first_id`.
fn encoded(links: SubPageLinks<'_>, first_id: u8) -> Vec<neoteletext_core::packet::Packet> {
    let page = PageAddress::from_mpp(PAGE).unwrap();
    let mut packets = Vec::new();
    encode(page, SubCode::new(0), links, SERVICE, first_id, &mut |packet| packets.push(packet)).unwrap();
    packets
}

/// Reads the records in `packets` into `held` as a decoder does, each
/// record dropped once it is read.
fn read_into(held: &mut Lines, packets: &[neoteletext_core::packet::Packet]) {
    let mut reader = RecordReader::new();
    for packet in packets {
        if let Read::Record(ChannelRecord::Links(record)) = reader.push(packet) {
            held.read_record(&record);
        }
    }
}

/// Reads the records of `links` as a decoder does, each record dropped once
/// it is read.
fn read_all(links: SubPageLinks<'_>) -> Lines {
    let mut held = Lines::default();
    read_into(&mut held, &encoded(links, 0));
    held
}

/// `count` text links, each to an address of the longest kind, so that
/// each takes a LINKS record of its own.
fn long_links(count: u8, mark: char) -> (LinkText, Vec<TextLink>) {
    let mut text = LinkText::new();
    let target = address(mark);
    let texts = (2..).take(usize::from(count)).map(|row| text_link(row, link_destination(target.as_bytes(), &mut text).unwrap())).collect();
    (text, texts)
}

/// The packets of each record of `links`, one record a group.
fn records(links: SubPageLinks<'_>, first_id: u8) -> Vec<Vec<neoteletext_core::packet::Packet>> {
    let mut groups: Vec<Vec<neoteletext_core::packet::Packet>> = Vec::new();
    let mut reader = RecordReader::new();
    let mut group = Vec::new();
    for packet in encoded(links, first_id) {
        group.push(packet);
        if matches!(reader.push(&packet), Read::Record(_)) {
            groups.push(core::mem::take(&mut group));
        }
    }
    groups
}

/// DOC-1952 and DOC-1958: a LINKS part that is not the next of the sending
/// part 0 began, or that gives the sending another number of parts, is
/// refused, and the sub-page is left with no Link Extension links. On the
/// code before, each was added to what part 0 gave.
#[test]
fn a_part_that_breaks_its_sending_leaves_no_links() {
    let (text, texts) = long_links(3, 'a');
    let three = records(SubPageLinks { keys: &[], numbers: &[], texts: &texts, addresses: &text }, 0);
    let (other_text, other_texts) = long_links(4, 'b');
    let four = records(SubPageLinks { keys: &[], numbers: &[], texts: &other_texts, addresses: &other_text }, 10);
    assert_eq!((three.len(), four.len()), (3, 4), "a record a link");
    let held = |groups: &[&Vec<neoteletext_core::packet::Packet>]| {
        let mut lines = Lines::default();
        for group in groups {
            read_into(&mut lines, group);
        }
        lines.get(PAGE, 0).map(|lines| lines.text.len())
    };
    assert_eq!(held(&[&three[0], &three[1], &three[2]]), Some(3), "a sound sending");
    assert_eq!(held(&[&three[0], &four[1]]), None, "a part of four after a part 0 of three");
    assert_eq!(held(&[&three[0], &three[2]]), None, "a part skipped");
    assert_eq!(held(&[&three[1]]), None, "no part 0");
    assert_eq!(held(&[&three[0], &three[1], &three[2], &three[2]]), None, "a part again");
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

/// DOC-1958: over a whole LINKS sending each key and each page number comes
/// once, at most five key links, eight page-number links and eight text
/// links. Parts that are each sound alone and follow one another, but give
/// a key or a page number a second time between them, are refused, and the
/// sub-page is left with no Link Extension links. On the code before, the
/// second part was added to the first.
#[test]
fn a_sending_that_breaks_its_totals_across_parts_leaves_no_links() {
    let mut text = LinkText::new();
    let (first, second) = (address('a'), address('b'));
    let one = link_destination(first.as_bytes(), &mut text).unwrap();
    let two = link_destination(second.as_bytes(), &mut text).unwrap();
    let held = |groups: [&Vec<neoteletext_core::packet::Packet>; 2]| {
        let mut lines = Lines::default();
        for group in groups {
            read_into(&mut lines, group);
        }
        lines.get(PAGE, 0).map(|lines| (lines.keys.len(), lines.numbers.len()))
    };
    // Two sendings of two parts each, a key link a part, in the opposite
    // order: part 0 of one and part 1 of the other give red twice.
    let red_green = [key(LinkKey::Red, one), key(LinkKey::Green, two)];
    let green_red = [key(LinkKey::Green, two), key(LinkKey::Red, one)];
    let a = records(SubPageLinks { keys: &red_green, numbers: &[], texts: &[], addresses: &text }, 0);
    let b = records(SubPageLinks { keys: &green_red, numbers: &[], texts: &[], addresses: &text }, 10);
    assert_eq!((a.len(), b.len()), (2, 2), "a record a link");
    assert_eq!(held([&a[0], &a[1]]), Some((2, 0)), "a sound sending");
    assert_eq!(held([&a[0], &b[1]]), None, "red twice");
    // The same with page numbers.
    let number = |page: u16, destination| NumberLink { number: PageAddress::from_mpp(page).unwrap(), destination, line: 0 };
    let up = [number(0x101, one), number(0x102, two)];
    let down = [number(0x102, two), number(0x101, one)];
    let c = records(SubPageLinks { keys: &[], numbers: &up, texts: &[], addresses: &text }, 20);
    let d = records(SubPageLinks { keys: &[], numbers: &down, texts: &[], addresses: &text }, 30);
    assert_eq!(held([&c[0], &c[1]]), Some((0, 2)), "a sound sending");
    assert_eq!(held([&c[0], &d[1]]), None, "page 101 twice");
}
