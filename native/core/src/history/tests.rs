//! Tests of history (requirements 20.1 to 20.3, 20.6 and 20.11).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use super::*;

/// 2026-10-22 03:05 UTC: 14:05 in Sydney.
const NOW: i64 = 20_748 * SECONDS_PER_DAY + 3 * 3_600 + 5 * 60;
const SYDNEY: i32 = 11 * 3_600;

fn visit(address: &str, typed: bool) -> Visit {
    Visit { address: address.into(), channel: "News Example".into(), title: "WEATHER".into(), typed, reload: false }
}

/// Decision 63: a reload moves the row's last visit and counts no
/// visit; on a day with no visit yet it makes the day's row, with none
/// counted, as Chrome and Firefox record a reload without counting it.
#[test]
fn a_reload_moves_the_last_visit_and_counts_none() {
    let mut history = History::default();
    history.record(&visit("https://news.example/", true), NOW, 0);
    let reload = Visit { reload: true, ..visit("https://news.example/", true) };
    history.record(&reload, NOW + 60, 0);
    let today = &history.query("")[0];
    assert_eq!((today.visits, today.typed, today.last), (1, 1, NOW + 60));
    history.record(&reload, NOW + SECONDS_PER_DAY, 0);
    let rows = history.query("");
    assert_eq!(rows.len(), 2, "the next day has its row");
    assert_eq!((rows[0].visits, rows[0].typed, rows[0].last), (0, 0, NOW + SECONDS_PER_DAY));
}

fn scratch(name: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("neoteletext-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Requirement 20.1: one row per address per day, its visits counted,
/// its typed visits apart; the browser's own page is not kept.
#[test]
fn a_page_is_one_row_a_day() {
    let mut history = History::default();
    assert!(history.record(&visit("https://news.example/teletext/#302", true), NOW, SYDNEY).is_some());
    history.record(&visit("https://news.example/teletext/#302", false), NOW + 60, SYDNEY);
    history.record(&visit("https://news.example/teletext/#302", false), NOW + SECONDS_PER_DAY, SYDNEY);
    assert_eq!(history.record(&visit("neoteletext:home", true), NOW, SYDNEY), None);
    assert_eq!(history.record(&visit("not an address", true), NOW, SYDNEY), None);
    assert_eq!(history.len(), 2);
    let today = history.row(20_748, "https://news.example/teletext/#302").unwrap();
    assert_eq!((today.visits, today.typed, today.last), (2, 1, NOW + 60));
}

/// Requirement 20.2: written, appended, read back; the last line of a
/// row stands; a file of many lines wants compacting.
#[test]
fn history_round_trips_through_its_file() {
    let folder = scratch("history-round-trip");
    let mut history = History::default();
    for minute in 0..3 {
        let line = history.record(&visit("https://news.example/teletext/", minute == 0), NOW + minute * 60, SYDNEY).unwrap();
        History::append(&folder, &line).unwrap();
    }
    let (read, dropped) = History::load(&folder).unwrap();
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(read.query(""), history.query(""));
    assert!(read.wants_compacting(), "three lines for one row");
    let mut read = read;
    read.save(&folder).unwrap();
    let text = std::fs::read_to_string(folder.join(FILE_NAME)).unwrap();
    assert_eq!(text.lines().count(), 2, "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(folder.join(FILE_NAME)).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

/// Requirement 20.3: each rule; a broken line is dropped and the rest
/// stand.
#[test]
fn the_file_is_read_strictly() {
    let good = Row {
        day: 20_748,
        last: NOW,
        offset: SYDNEY,
        visits: 2,
        typed: 1,
        address: "https://news.example/teletext/#302".into(),
        channel: "News".into(),
        title: "WEATHER".into(),
    };
    let line = good.line();
    let broken = [
        line.replace("2026-10-22", "2026-02-30"),
        line.replace(&NOW.to_string(), "-5"),
        line.replace(&SYDNEY.to_string(), "90000"),
        line.replacen("\t2\t", "\t0\t", 1),
        line.replacen("\t2\t1\t", "\t1\t2\t", 1),
        line.replace("https://news.example/teletext/#302", "neoteletext:home"),
        line.replace("https://news.example/teletext/#302", "https://news.example/a b"),
        line.replace("WEATHER", "WEA\u{202E}THER"),
        format!("{line}\textra"),
        line.replace("2026-10-22", "2026-10-21"),
        String::new(),
    ];
    let mut text = format!("{VERSION_LINE}\n{line}\n");
    for line in &broken {
        text.push_str(line);
        text.push('\n');
    }
    let (history, dropped) = History::read(&text);
    assert_eq!(history.query(""), vec![&good]);
    assert_eq!(dropped.len(), broken.len(), "{dropped:?}");
    assert!(History::read("no version\n").0.is_empty());
    assert!(History::read(&"x".repeat(MAX_FILE + 1)).0.is_empty());
}

/// Requirement 20.2: 90 days kept, today among them; at most
/// `MAX_ROWS`, the oldest going first.
#[test]
fn history_is_kept_ninety_days_and_bounded() {
    let mut history = History::default();
    history.record(&visit("https://old.example/", false), NOW - 90 * SECONDS_PER_DAY, 0);
    history.record(&visit("https://kept.example/", false), NOW - 89 * SECONDS_PER_DAY, 0);
    assert!(history.expire(local_day(NOW, 0)));
    let addresses: Vec<&str> = history.query("").iter().map(|row| row.address.as_str()).collect();
    assert_eq!(addresses, ["https://kept.example/"]);
    let mut history = History::default();
    for n in 0..=MAX_ROWS {
        history.record(&visit(&format!("https://p{n}.example/"), false), NOW + i64::try_from(n).unwrap(), 0);
    }
    history.expire(local_day(NOW, 0));
    assert_eq!(history.len(), MAX_ROWS);
    assert!(history.row(local_day(NOW, 0), "https://p0.example/").is_none(), "the oldest went");
}

/// Requirement 20.6: a row, a channel, a range, everything.
#[test]
fn history_is_removed_as_asked() {
    let mut history = History::default();
    history.record(&visit("https://news.example/teletext/#100", false), NOW - 2 * HOUR, 0);
    history.record(&visit("https://news.example/teletext/#302", false), NOW - 30 * 60, 0);
    history.record(&visit("https://sport.example/", false), NOW - 10, 0);
    assert_eq!(history.clear(History::since(NOW, HOUR)), 2);
    assert_eq!(history.remove(&[(local_day(NOW, 0), "https://news.example/teletext/#100".into())]), ["https://news.example/teletext/#100"]);
    assert!(history.is_empty());
    history.record(&visit("https://sport.example/", false), NOW, 0);
    assert_eq!(history.query("host:SPORT.example").len(), 1, "host: names one host, in any case");
    assert!(history.query("host:example").is_empty(), "the whole host");
    assert!(history.has_address("https://sport.example/"));
    assert_eq!(history.remove(&[(local_day(NOW, 0), "https://sport.example/".into())]), ["https://sport.example/"]);
    assert!(!history.has_address("https://sport.example/"));
    history.record(&visit("https://sport.example/", false), NOW, 0);
    assert_eq!(history.clear(Range::All), 1);
}

/// Requirement 20.11: inline completion only from addresses typed;
/// suggestions from every visit, the most visited first.
#[test]
fn completion_comes_only_from_what_was_typed() {
    let mut history = History::default();
    history.record(&visit("https://news.example/teletext/", false), NOW, 0);
    assert_eq!(history.completion("new"), None, "a link followed never completes the field");
    history.record(&visit("https://news.example/teletext/", true), NOW, 0);
    assert_eq!(history.completion("New").as_deref(), Some("News.example/teletext/"));
    assert_eq!(history.completion("https://n").as_deref(), Some("https://news.example/teletext/"));
    assert_eq!(history.completion("sport"), None);
    history.record(&visit("https://newer.example/", false), NOW + 5, 0);
    let suggested: Vec<String> = history.suggest("new", 8).into_iter().map(|row| row.address).collect();
    assert_eq!(suggested, ["https://news.example/teletext/", "https://newer.example/"]);
    assert_eq!(history.suggest("", 8), Vec::new());
}
