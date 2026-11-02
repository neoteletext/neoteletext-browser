//! Tests of the settings file (requirements 19.1 and 19.2).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]

use super::*;
use crate::dropped::Dropped;
use std::fmt::Write as _;

/// Requirement 19.1: written, read back.
#[test]
fn settings_round_trip_through_their_file() {
    let settings = Settings {
        home: "https://news.example/teletext/".into(),
        sound: false,
        volume: 40,
        proxy: Some(Proxy::Socks5 { host: "127.0.0.1".into(), port: 1080 }),
        local: vec!["192.168.1.20".parse().unwrap()],
        bar: true,
        aspect: true,
        restore: false,
        check_updates: false,
        cookies: crate::cookies::Mode::Close,
        cookie_block: vec!["https://news.example/teletext/".into()],
        remote: RemotePlace { mode: RemoteMode::Floating, x: 900, y: 140 },
    };
    let (read, dropped) = Settings::read(&settings.text());
    assert_eq!((read, dropped), (settings, Dropped::default()));
    assert_eq!(Settings::read("").0, Settings::default());
}

/// REQ-737 item 2: off by default, in a file that has no line for it too;
/// kept as `aspect on` and read strictly.
#[test]
fn the_aspect_correction_is_off_until_turned_on_and_read_strictly() {
    assert!(!Settings::default().aspect);
    assert!(Settings::default().text().contains("aspect off\n"));
    let (read, dropped) = Settings::read(&format!("{VERSION_LINE}\nbar on\n"));
    assert_eq!((read.aspect, dropped.len()), (false, 0), "a file written before it");
    assert!(Settings::read(&format!("{VERSION_LINE}\naspect on\n")).0.aspect);
    let (read, dropped) = Settings::read(&format!("{VERSION_LINE}\naspect sideways\n"));
    assert_eq!((read.aspect, dropped.len()), (false, 1));
}

/// Decision 68: the remote's line, read strictly.
#[test]
fn the_remotes_place_is_read_strictly() {
    let place = |line: &str| Settings::read(&format!("{VERSION_LINE}\nremote {line}\n"));
    assert_eq!(place("docked").0.remote, RemotePlace { mode: RemoteMode::Docked, x: 0, y: 0 });
    assert_eq!(place("floating 10 20").0.remote, RemotePlace { mode: RemoteMode::Floating, x: 10, y: 20 });
    for wrong in ["floating 10", "floating 10 20 30", "docked 1 2", "floating 99999 1", "sideways", "floating -1 4"] {
        let (read, dropped) = place(wrong);
        assert_eq!((read.remote, dropped.len()), (RemotePlace::default(), 1), "{wrong}");
    }
}

/// Requirement 19.2: each rule; a broken line is dropped and the rest
/// stand.
#[test]
fn the_file_is_read_strictly() {
    let text = "neoteletext-settings 1\nvolume 101\nvolume 30\nsound maybe\nhome javascript:alert(1)\nproxy ftp p.example 21\nproxy http p.example 0\nallow-local 93.184.216.34\nallow-local 10.0.0.1\nhistory maybe\nbar off\nunknown thing\nbookmark https://a.example/\n";
    let (settings, dropped) = Settings::read(text);
    assert_eq!(settings.volume, 30);
    assert!(settings.sound, "the default stands");
    assert_eq!(settings.home, HOME_CHANNEL);
    assert_eq!(settings.proxy, None);
    assert_eq!(settings.local, ["10.0.0.1".parse::<IpAddr>().unwrap()], "only a local address may be allowed");
    assert!(!settings.bar);
    // A bookmark line is no longer a setting: bookmarks are in their own
    // file (requirement 19.2, amended by decision 57).
    assert_eq!(dropped.len(), 9, "{dropped:?}");
    assert_eq!(Settings::read("volume 30\n").0.volume, MAX_VOLUME, "no version line: the defaults");
    // Decision 65: "Remember history" is gone; its line is dropped and said.
    assert_eq!(Settings::read(&format!("{VERSION_LINE}\nhistory off\n")).1.len(), 1);
    let long = format!("{VERSION_LINE}\nhome https://{}.example/\n", "a".repeat(MAX_VALUE));
    assert_eq!(Settings::read(&long).1.len(), 1);
    let huge = "x".repeat(MAX_FILE + 1);
    assert_eq!(Settings::read(&huge).0, Settings::default());
}

/// Requirement 19.1: a home as long as the address field allows is
/// kept between runs; its line is its name and the address.
#[test]
fn the_longest_address_is_kept() {
    let query = "a".repeat(protocol::MAX_ADDRESS - "https://a.example/?q=".len() - "302/0002".len());
    let address = format!("https://a.example/?q={query}");
    assert_eq!(checked_address(&address).as_deref(), Some(address.as_str()), "the address rules take it");
    let settings = Settings { home: address, ..Settings::default() };
    assert_eq!(Settings::read(&settings.text()), (settings, Dropped::default()));
}

/// `installation-and-updates.md` section 3.3: this format's file is
/// the one the profile rule names, and "Check for updates" is kept.
#[test]
fn the_settings_are_this_formats_file_and_keep_the_update_setting() {
    assert_eq!(update::profile::file_name(PROFILE, FORMAT), FILE_NAME);
    assert_eq!(VERSION_LINE, format!("neoteletext-settings {FORMAT}"));
    let off = Settings { check_updates: false, ..Settings::default() };
    assert_eq!(Settings::read(&off.text()), (off, Dropped::default()));
    assert!(Settings::default().check_updates, "on by default");
}

/// DEC-1761: Noto Teletext is the only font, so a settings file written
/// while there was a choice loads whole. Its `font` line names nothing known
/// and is dropped and logged as any such line is; the next save leaves it out.
#[test]
fn a_stored_font_line_is_dropped_and_the_rest_still_loads() {
    let (read, dropped) = Settings::read(&format!("{VERSION_LINE}\nfont bedstead\nvolume 3\nfont noto-teletext\nsound off\n"));
    assert_eq!((read.volume, read.sound, dropped.len()), (3, false, 2));
    assert!(read.text().lines().all(|line| !line.starts_with("font")), "not written back");
}

/// neoteletext-common's DEC-1652: a new browser's home is the home channel,
/// a home the settings file holds stays the user's, and the built-in page
/// can still be chosen.
#[test]
fn the_home_is_the_home_channel_unless_the_user_has_one() {
    assert_eq!(Settings::default().home, HOME_CHANNEL);
    assert_eq!(Settings::read(&format!("{VERSION_LINE}\n")).0.home, HOME_CHANNEL, "no home line");
    let own = "https://news.example/teletext/";
    assert_eq!(Settings::read(&format!("{VERSION_LINE}\nhome {own}\n")).0.home, own);
    assert_eq!(Settings::read(&format!("{VERSION_LINE}\nhome {START_PAGE}\n")).0.home, START_PAGE);
}

#[test]
fn at_most_16_local_addresses() {
    let mut text = String::from("neoteletext-settings 1\n");
    for n in 0..20 {
        let _ = writeln!(text, "allow-local 10.0.0.{n}");
    }
    let (settings, _) = Settings::read(&text);
    assert_eq!(settings.local.len(), MAX_LOCAL);
}

/// Requirement 19.1: a write that stops before its rename leaves the
/// old file whole.
#[test]
fn the_file_is_written_whole_or_not_at_all() {
    let folder = std::env::temp_dir().join(format!("neoteletext-settings-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let first = Settings { volume: 10, ..Settings::default() };
    first.save(&folder).unwrap();
    assert_eq!(Settings::load(&folder).unwrap().0, first);
    // The user's alone: before this the file took the default mode.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(folder.join(FILE_NAME)).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // A temporary file left by a crash before the rename is ignored.
    std::fs::write(folder.join(TEMPORARY_NAME), "neoteletext-settings 1\nvolume 9").unwrap();
    assert_eq!(Settings::load(&folder).unwrap().0, first);
    let second = Settings { volume: 20, ..Settings::default() };
    second.save(&folder).unwrap();
    assert_eq!(Settings::load(&folder).unwrap().0, second);
    std::fs::remove_dir_all(&folder).unwrap();
    assert_eq!(Settings::load(&folder).unwrap().0, Settings::default(), "no file: the defaults");
}

#[test]
fn proxies_are_written_as_the_engine_reads_them() {
    assert_eq!(Proxy::Http { host: "p.example".into(), port: 3128 }.url(), "http://p.example:3128");
    assert_eq!(Proxy::Socks5 { host: "::1".into(), port: 1080 }.url(), "socks5://[::1]:1080");
    assert_eq!(Proxy::from_parts("http", "p example", 80), None);
}
