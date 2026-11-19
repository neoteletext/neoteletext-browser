//! The store's tests (REQ-6886) and the misses' (REQ-6883).

use super::*;

/// A folder of its own for each test, in the test's temporary folder.
fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("neoteletext-snapshots-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

fn place(page: u16) -> Place {
    Place { channel: String::from("https://a.example/tt/"), page, sub: WHOLE_PAGE }
}

fn snapshot(fetched: i64, packets: usize) -> Snapshot {
    Snapshot { fetched, drawn: 0, body: vec![0x20; packets * PACKET_BYTES] }
}

/// REQ-6886 item 3.
#[test]
fn a_snapshot_round_trips_through_its_file() {
    let place = Place { sub: 0x0002, ..place(0x123) };
    let kept = Snapshot { fetched: 1_790_000_000, drawn: 2, body: (0..=255).cycle().take(3 * PACKET_BYTES).collect() };
    let bytes = encode(&place, &kept).unwrap();
    assert!(bytes.starts_with(b"neoteletext-snapshot 1\n123\t0002\thttps://a.example/tt/\n1790000000\t0002\t126\n"));
    assert_eq!(decode(&place, &bytes), Ok(kept.clone()));
    let store = Store::new(Some(&folder("round-trip")));
    store.write(&place, &kept).unwrap();
    assert_eq!(store.read(&place), Some(kept));
    std::fs::remove_dir_all(store.folder().unwrap().parent().unwrap()).unwrap();
}

/// REQ-6886 item 5: each field wrong in turn, a short and a long body,
/// and a key for another place, each refused whole; the file is deleted.
#[test]
fn every_malformed_file_is_refused_whole_and_deleted() {
    let place = place(0x100);
    let good = String::from_utf8(encode(&place, &snapshot(5, 1)).unwrap()).unwrap();
    let body = " ".repeat(PACKET_BYTES);
    let wrong = [
        (good.replacen("snapshot 1", "snapshot 2", 1), Refused::Version),
        (good.replacen("100\t3F7F", "101\t3F7F", 1), Refused::Key),
        (good.replacen("100\t3F7F", "100\t0000", 1), Refused::Key),
        (good.replacen("a.example", "b.example", 1), Refused::Key),
        (good.replacen("\n5\t", "\n05\t", 1), Refused::Head),
        (good.replacen("\n5\t", "\n-5\t", 1), Refused::Head),
        (good.replacen("\t0000\t", "\t3F7F\t", 1), Refused::Head),
        (good.replacen("\t0000\t", "\t00a0\t", 1), Refused::Head),
        (good.replacen("\t42\n", "\t43\n", 1), Refused::Body),
        (good.replacen("\t42\n", "\t42\tx\n", 1), Refused::Head),
        (format!("{good} "), Refused::Body),
        (good.strip_suffix(' ').unwrap().to_owned(), Refused::Body),
        (good.replacen(&body, "", 1).replacen("\t42\n", "\t0\n", 1), Refused::Body),
    ];
    let store = Store::new(Some(&folder("malformed")));
    store.write(&place, &snapshot(5, 1)).unwrap();
    let path = store.folder().unwrap().join(place.file_name());
    for (text, refusal) in wrong {
        assert_eq!(decode(&place, text.as_bytes()), Err(refusal), "{text:?}");
        std::fs::write(&path, &text).unwrap();
        assert_eq!(store.read(&place), None, "{text:?}");
        assert!(!path.exists(), "deleted: {text:?}");
    }
    let mut long = good.into_bytes();
    long.resize(MAX_SNAPSHOT_FILE + 1, b' ');
    assert_eq!(decode(&place, &long), Err(Refused::Body));
    std::fs::remove_dir_all(store.folder().unwrap().parent().unwrap()).unwrap();
}

/// REQ-6886 item 2: the name is the key's SHA-256 and holds no address;
/// page and sub-code are both in the key (DEC-6599).
#[test]
fn the_file_name_is_the_keys_hash_and_holds_no_address() {
    let place = place(0x100);
    let name = place.file_name();
    assert!(is_snapshot_name(&name), "{name}");
    assert!(!name.contains("example"));
    // SHA-256 of "100\t3F7F\thttps://a.example/tt/".
    let digest = ring::digest::digest(&ring::digest::SHA256, b"100\t3F7F\thttps://a.example/tt/");
    let hex: String = digest.as_ref().iter().flat_map(|byte| [byte >> 4, byte & 0x0F]).map(|nibble| char::from_digit(u32::from(nibble), 16).unwrap()).collect();
    assert!(name.starts_with(&hex));
    assert_ne!(name, Place { sub: 1, ..place.clone() }.file_name());
    assert_ne!(name, Place { page: 0x101, ..place }.file_name());
}

/// REQ-6886 item 1.
#[cfg(unix)]
#[test]
fn files_and_folder_are_the_users_alone() {
    use std::os::unix::fs::PermissionsExt as _;
    let store = Store::new(Some(&folder("modes")));
    store.write(&place(0x100), &snapshot(1, 1)).unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(store.folder().unwrap()), 0o700);
    assert_eq!(mode(&store.folder().unwrap().join(place(0x100).file_name())), 0o600);
    std::fs::remove_dir_all(store.folder().unwrap().parent().unwrap()).unwrap();
}

/// REQ-6886 item 4: past `MAX_SNAPSHOTS` the snapshot fetched longest ago
/// goes, never the one being stored; and past the bytes' bound too.
#[test]
fn the_store_is_bounded_in_count_and_bytes_dropping_the_oldest() {
    let store = Store::new(Some(&folder("bounded")));
    for index in 0..=MAX_SNAPSHOTS {
        let page = 0x100 + u16::try_from(index).unwrap();
        // The first is the newest, so the second is the oldest.
        let fetched = if index == 0 { 10_000 } else { i64::try_from(index).unwrap() };
        store.write(&place(page), &snapshot(fetched, 1)).unwrap();
    }
    assert!(store.read(&place(0x100)).is_some(), "the newest stays");
    assert!(store.read(&place(0x101)).is_none(), "the oldest went");
    assert_eq!(store.kept().len(), MAX_SNAPSHOTS);
    // Bytes: the largest bodies, past 32 MiB all together.
    store.clear(None);
    let large = MAX_SNAPSHOT_BYTES / PACKET_BYTES;
    let fit = usize::try_from(MAX_SNAPSHOT_STORE_BYTES).unwrap() / (large * PACKET_BYTES + MAX_HEAD);
    for index in 0..=fit {
        store.write(&place(0x200 + u16::try_from(index).unwrap()), &snapshot(i64::try_from(index).unwrap(), large)).unwrap();
    }
    let total: u64 = store.kept().iter().map(|kept| kept.bytes).sum();
    assert!(total <= MAX_SNAPSHOT_STORE_BYTES, "{total}");
    assert!(store.read(&place(0x200)).is_none(), "the oldest went");
    assert!(store.read(&place(0x200 + u16::try_from(fit).unwrap())).is_some(), "the one stored stays");
    std::fs::remove_dir_all(store.folder().unwrap().parent().unwrap()).unwrap();
}

/// REQ-6886 items 5 and 7: a place no tab shows is deleted, and so is
/// anything in the folder that is not a snapshot file.
#[test]
fn a_place_no_tab_shows_is_deleted_and_strays_with_it() {
    let store = Store::new(Some(&folder("unshown")));
    store.write(&place(0x100), &snapshot(1, 1)).unwrap();
    store.write(&place(0x101), &snapshot(1, 1)).unwrap();
    let stray = store.folder().unwrap().join("notes.txt");
    std::fs::write(&stray, b"x").unwrap();
    std::fs::create_dir(store.folder().unwrap().join("sub")).unwrap();
    store.keep_only([&place(0x101)]);
    assert!(store.read(&place(0x100)).is_none());
    assert!(store.read(&place(0x101)).is_some());
    assert!(!stray.exists() && !store.folder().unwrap().join("sub").exists(), "strays are deleted");
    std::fs::remove_dir_all(store.folder().unwrap().parent().unwrap()).unwrap();
}

/// REQ-6886 item 7: history cleared since a time deletes the snapshots
/// fetched in it; the setting off deletes them all.
#[test]
fn clearing_history_deletes_the_snapshots_of_its_time_and_the_setting_all() {
    let store = Store::new(Some(&folder("cleared")));
    store.write(&place(0x100), &snapshot(100, 1)).unwrap();
    store.write(&place(0x101), &snapshot(200, 1)).unwrap();
    store.clear(Some(150));
    assert!(store.read(&place(0x100)).is_some());
    assert!(store.read(&place(0x101)).is_none());
    store.clear(None);
    assert!(store.read(&place(0x100)).is_none());
    std::fs::remove_dir_all(store.folder().unwrap().parent().unwrap()).unwrap();
}

/// REQ-6883: the place of an address, page and sub-code together; none
/// for a file or a built-in page.
#[test]
fn a_tabs_place_is_its_channel_page_and_sub_code() {
    let at = |typed: &str| Place::of(&crate::address::parse(typed).unwrap());
    let bare = at("https://a.example/tt/").unwrap();
    assert_eq!((bare.page, bare.sub, bare.page_asked()), (INITIAL_PAGE, WHOLE_PAGE, INITIAL_PAGE_ASKED));
    let page = at("https://a.example/tt/302").unwrap();
    assert_eq!((page.channel.as_str(), page.page, page.sub), ("https://a.example/tt/", 0x302, WHOLE_PAGE));
    let sub = at("https://a.example/tt/302/0002").unwrap();
    assert_eq!((sub.page, sub.sub), (0x302, 2));
    assert_eq!(at(crate::address::START_PAGE), None);
}

/// REQ-6883: a miss is remembered for its interval, the oldest forgotten
/// past the bound.
#[test]
fn a_miss_is_remembered_for_its_interval() {
    let mut misses = Misses::default();
    let now = Instant::now();
    misses.missed(&place(0x100), now);
    assert!(misses.recent(&place(0x100), (now + SNAPSHOT_MISS_RETRY).checked_sub(Duration::from_secs(1)).unwrap()));
    assert!(!misses.recent(&place(0x100), now + SNAPSHOT_MISS_RETRY));
    assert!(!misses.recent(&place(0x101), now));
    for index in 0..MAX_MISSES_REMEMBERED {
        misses.missed(&place(0x200 + u16::try_from(index).unwrap()), now);
    }
    assert!(!misses.recent(&place(0x100), now), "the oldest forgotten");
    misses.clear();
    assert!(!misses.recent(&place(0x200), now));
}
