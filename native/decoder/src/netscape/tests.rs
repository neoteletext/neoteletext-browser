//! The bookmark file reader's tests: what it reads, what it leaves out, and
//! the bounds it keeps.

use super::*;

/// As Chrome writes it, with Firefox's comments and descriptions.
const CHROME: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<!-- This is an automatically generated file.
     It will be read and overwritten.
     DO NOT EDIT! -->
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3 ADD_DATE="1700000000" LAST_MODIFIED="1700000001" PERSONAL_TOOLBAR_FOLDER="true">Bookmarks bar</H3>
    <DL><p>
        <DT><A HREF="https://teastop.co.uk/teefax/" ADD_DATE="1700000002" ICON="data:image/png;base64,iVBOR">Teefax &amp; friends</A>
        <DT><H3>News &#x26; sport</H3>
        <DL><p>
            <DT><A HREF='http://example.com/ceefax/?a=1&amp;b=2'>Ceefax
               again</A>
            <DD>A description, not kept
        </DL><p>
        <DT><A>No address</A>
    </DL><p>
    <DT><A HREF="https://example.org/">&lt;Other&gt; &#169;</A>
</DL><p>
"#;

#[test]
fn a_browsers_file_gives_its_folders_and_bookmarks_in_order() {
    let found = read(CHROME.as_bytes()).unwrap();
    let link = |name: &str, address: &str| BookmarkEntry::Link { name: name.into(), address: address.into() };
    assert_eq!(
        found.entries,
        [
            BookmarkEntry::Folder("Bookmarks bar".into()),
            link("Teefax & friends", "https://teastop.co.uk/teefax/"),
            BookmarkEntry::Folder("News & sport".into()),
            link("Ceefax again", "http://example.com/ceefax/?a=1&b=2"),
            BookmarkEntry::FolderEnd,
            BookmarkEntry::FolderEnd,
            link("<Other> ©", "https://example.org/"),
        ]
    );
    assert_eq!(found.skipped, 1, "the bookmark with no address");
}

#[test]
fn only_a_netscape_file_is_read() {
    assert_eq!(read(b"<html><body><a href=\"https://x/\">x</a>"), Err(NotRead::NotNetscape));
    assert_eq!(read(b"bookmark\t1\t0\tName\thttps://x/"), Err(NotRead::NotNetscape));
}

#[test]
fn folders_deeper_than_the_bookmarks_keep_join_the_folder_around_them() {
    let depth = MAX_FOLDER_DEPTH + 3;
    let mut file = String::from("<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL>");
    for level in 0..depth {
        file += &["<DT><H3>Level ", &level.to_string(), "</H3><DL>"].concat();
    }
    file.push_str("<DT><A HREF=\"https://deep/\">Deep</A>");
    for _ in 0..depth {
        file.push_str("</DL>");
    }
    let found = read(file.as_bytes()).unwrap();
    let folders = found.entries.iter().filter(|entry| matches!(entry, BookmarkEntry::Folder(_))).count();
    let ends = found.entries.iter().filter(|entry| matches!(entry, BookmarkEntry::FolderEnd)).count();
    assert_eq!((folders, ends, found.skipped), (MAX_FOLDER_DEPTH, MAX_FOLDER_DEPTH, 3));
    assert!(found.entries.contains(&BookmarkEntry::Link { name: "Deep".into(), address: "https://deep/".into() }));
}

#[test]
fn a_file_cut_short_still_closes_its_folders() {
    let found = read(b"<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><DT><H3>Open</H3><DL><DT><A HREF=\"https://a/\">A</A><DT><A HREF=\"https://b/\">B").unwrap();
    assert_eq!(found.entries.last(), Some(&BookmarkEntry::FolderEnd));
    assert_eq!(found.skipped, 1, "the bookmark whose name never ended");
}

#[test]
fn lengths_and_counts_are_bounded() {
    let long_address = format!("https://x/{}", "a".repeat(MAX_ATTRIBUTE));
    let long_name = "n".repeat(MAX_BOOKMARK_NAME * 2);
    let file = format!("<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><DT><A HREF=\"{long_address}\">x</A><DT><A HREF=\"https://y/\">{long_name}</A>");
    let found = read(file.as_bytes()).unwrap();
    assert_eq!(found.skipped, 1, "the address past its bound");
    let [BookmarkEntry::Link { name, .. }] = found.entries.as_slice() else { panic!("one bookmark: {:?}", found.entries) };
    assert_eq!(name.len(), MAX_BOOKMARK_NAME);
    let many = format!("<!DOCTYPE NETSCAPE-Bookmark-file-1><DL>{}", "<DT><A HREF=\"https://z/\">z</A>".repeat(MAX_BOOKMARK_ENTRIES + 5));
    let found = read(many.as_bytes()).unwrap();
    assert_eq!((found.entries.len(), found.skipped), (MAX_BOOKMARK_ENTRIES, 5));
    assert_eq!(read(&vec![b' '; MAX_BOOKMARK_FILE + 1]), Err(NotRead::TooLarge));
}

#[test]
fn a_quoted_greater_than_sign_does_not_end_a_tag() {
    let found = read(b"<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><DT><A TITLE=\"a > b\" HREF=\"https://q/\">Q</A>").unwrap();
    assert_eq!(found.entries, [BookmarkEntry::Link { name: "Q".into(), address: "https://q/".into() }]);
}

#[test]
fn unknown_entities_are_kept_as_written() {
    let mut out = String::new();
    decode_into("a &bogus; b &#xZZ; &#65; &", &mut out, 100);
    assert_eq!(out, "a &bogus; b &#xZZ; A &");
}
