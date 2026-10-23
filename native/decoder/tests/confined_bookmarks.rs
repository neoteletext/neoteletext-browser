//! A decoder, confined as the broker confines it, reads another browser's
//! bookmark file and reports what it holds (decision 71): the trusted core
//! never reads the file's HTML itself (requirement 3.2).

use std::ffi::OsString;
use std::io::{BufReader, Write as _};
use std::path::Path;

use confine::{Role, Spawn};
use protocol::decoder::{BookmarkEntry, FromDecoder, ToDecoder, read_from_decoder, write_to_decoder};

/// As Firefox writes it.
const FIREFOX: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'none'; img-src data: *; object-src 'none'"></meta>
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks Menu</H1>
<DL><p>
    <DT><H3 ADD_DATE="1700000000" LAST_MODIFIED="1700000000" PERSONAL_TOOLBAR_FOLDER="true">Bookmarks Toolbar</H3>
    <DL><p>
        <DT><A HREF="https://teastop.co.uk/teefax/" ADD_DATE="1700000000" LAST_MODIFIED="1700000000">Teefax</A>
        <DT><A HREF="place:sort=8&maxResults=10">Most Visited</A>
    </DL><p>
</DL>
"#;

#[test]
fn a_confined_decoder_reads_another_browsers_bookmarks() {
    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-decoder"));
    let arguments = [OsString::from("--bookmarks")];
    let environment = confine::environment(Role::Decoder);
    let mut process =
        confine::spawn(&Spawn { program, arguments: &arguments, role: Role::Decoder, environment: &environment, local_network: false, writable: None })
            .unwrap();
    eprintln!("decoder: {}", process.report);
    let mut input = process.stdin.take().unwrap();
    write_to_decoder(&mut input, &ToDecoder::Data(FIREFOX.as_bytes().to_vec())).unwrap();
    write_to_decoder(&mut input, &ToDecoder::End).unwrap();
    input.flush().unwrap();
    let mut output = BufReader::new(process.stdout.take().unwrap());
    let mut said = Vec::new();
    while let Some(message) = read_from_decoder(&mut output).unwrap() {
        said.push(message);
    }
    let link = |name: &str, address: &str| FromDecoder::Bookmark(BookmarkEntry::Link { name: name.into(), address: address.into() });
    // The decoder says what is there; the trusted core decides what a
    // channel's address is, and refuses `place:`.
    assert_eq!(
        said,
        [
            FromDecoder::Bookmark(BookmarkEntry::Folder("Bookmarks Toolbar".into())),
            link("Teefax", "https://teastop.co.uk/teefax/"),
            link("Most Visited", "place:sort=8&maxResults=10"),
            FromDecoder::Bookmark(BookmarkEntry::FolderEnd),
            FromDecoder::BookmarksRead { skipped: 0 },
        ]
    );
    assert!(process.wait().unwrap().success());
}
