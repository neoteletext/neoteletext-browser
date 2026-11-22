//! Runs of the standalone runner (script-api.md section 16), made in this
//! process through the function the worker calls.

mod common;

use common::{CLAIMS, Given, log, message, run, status, text};
use script_run::Status;
use script_run::blobs::find;
use script_run::report::sha256;
use script_run::run::{BUDGET_FILE, LOG_FILE, SOUND_FILE, is_kept};

/// A page that uses everything a run must hold still: the random
/// sources, the clock, timers, ticks and keys.
fn busy() -> String {
    format!(
        r#"{CLAIMS}
        const bytes = new Uint8Array(8); crypto.getRandomValues(bytes);
        console.log("start", Math.random(), Array.from(bytes).join(","), Date.now(), performance.now(), crypto.randomUUID());
        setTimeout(() => console.log("timer at", nt.now(), Date.now()), 250);
        let ticks = 0;
        nt.addEventListener("tick", (event) => {{ ticks++; if (event.tick % 20 === 0) console.log("tick", event.tick, Math.random()); }});
        nt.addEventListener("keydown", (event) => {{ console.log("key", event.key, ticks); nt.synth.start(0, "synthd"); nt.synth.note(0, 60, {{ ticks: 20 }}); }});
        nt.addEventListener("close", () => console.log("closing after", ticks));
        "#
    )
}

/// Section 16, "Steps time deterministically": two runs with one seed
/// give identical frames, sound, log and budget, by the SHA-256 of every
/// kept file; another seed gives another log.
#[test]
fn two_runs_with_one_seed_give_identical_frames() {
    let page = busy();
    let given =
        Given { arguments: &["--ticks", "90", "--png-every", "30", "--png-at", "7"], page: Some(&page), input: Some("10 key Enter\n"), ..Given::default() };
    let digests = |made: &[script_run::blobs::Blob]| -> Vec<(String, String)> {
        made.iter().filter(|(name, _)| is_kept(name)).map(|(name, bytes)| (name.clone(), sha256(bytes))).collect()
    };
    let (first, second) = (run(&given), run(&given));
    assert_eq!(status(&first), Status::Matched, "{}", message(&first));
    let names: Vec<String> = digests(&first).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["tick-000007.png", "tick-000029.png", "tick-000059.png", "tick-000089.png", LOG_FILE, BUDGET_FILE, SOUND_FILE]);
    assert_eq!(digests(&first), digests(&second), "the kept files of two runs with one seed");
    // The log shows the run did what it was written to do.
    let lines = log(&first);
    assert!(lines.iter().any(|line| line.starts_with("000010 page log key Enter 10")), "{lines:#?}");
    assert!(lines.iter().any(|line| line.starts_with("000015 page log timer at 15 ")), "a 250 ms timer is due at tick 15: {lines:#?}");
    assert!(lines.iter().any(|line| line.ends_with("page log closing after 90")), "{lines:#?}");
    // The clock a script reads is the run's fixed moment, never the day's.
    assert!(lines[0].contains(" 1798761600000 0 "), "{}", lines[0]);

    let other = run(&Given { arguments: &["--ticks", "90", "--png-every", "30", "--png-at", "7", "--seed", "another"], ..given });
    assert_ne!(text(&first, LOG_FILE), text(&other, LOG_FILE), "another seed gives other random numbers");
}

/// Section 16, "Snapshots pictures and sound": the sound a script plays
/// is in the WAV file, as long as the run's ticks, and silent without
/// the viewer's press (section 3.5).
#[test]
fn the_wav_holds_what_the_mixer_rendered() {
    let page = busy();
    let pressed = run(&Given { arguments: &["--ticks", "60"], page: Some(&page), input: Some("10 key Enter\n"), ..Given::default() });
    let wav = find(&pressed, SOUND_FILE).unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(wav.len(), 44 + 60 * 800 * 2, "44 bytes of header, then 60 ticks of 800 samples of two bytes");
    assert!(wav[44..].iter().any(|byte| *byte != 0), "the note is heard");
    let report = text(&pressed, script_run::run::REPORT_TEXT);
    assert!(report.contains("sound: ") && !report.contains("peak 0\n"), "{report}");
    let untouched = run(&Given { arguments: &["--ticks", "60"], page: Some(&page), ..Given::default() });
    assert!(find(&untouched, SOUND_FILE).unwrap()[44..].iter().all(|byte| *byte == 0), "no key, no press: nothing sounds");
}

/// Sections 3.1 and 4.5: a channel script and a page script run in one
/// container, the channel context told the page is shown, each given the
/// events, the page context first.
#[test]
fn a_channel_script_and_a_page_script_run_together() {
    let channel = r#"/*! nt {"api": 1, "keys": ["a"]} */
        nt.addEventListener("pageshow", (event) => console.log("shown", event.page.number, event.page.renderer, nt.level));
        nt.addEventListener("keydown", (event) => console.log("channel hears", event.key));
        nt.addEventListener("sharedchange", (event) => console.log("shared", event.key, nt.shared.get(event.key)));"#;
    let page = r#"/*! nt {"api": 1, "renderer": "regis", "keys": ["a"]} */
        nt.addEventListener("keydown", (event) => { console.log("page hears", event.key); nt.shared.set("score", 7); });"#;
    let made = run(&Given {
        arguments: &["--ticks", "5", "--page", "1A0"],
        page: Some(page),
        channel: Some(channel),
        input: Some("2 keydown a\n"),
        ..Given::default()
    });
    assert_eq!(status(&made), Status::Matched, "{}", message(&made));
    let lines = log(&made);
    let wanted = [
        "000000 channel log shown 1A0 regis channel",
        "000002 page log page hears a",
        "000002 channel log shared score 7",
        "000002 channel log channel hears a",
    ];
    for line in wanted {
        assert!(lines.iter().any(|held| held == line), "no line `{line}` in {lines:#?}");
    }
    let budget = text(&made, BUDGET_FILE);
    assert!(budget.contains("\"channel\"") && budget.contains("\"page\""), "{budget}");
    assert!(!budget.contains("Percent"), "the shares of a time limit follow the wall clock and are not kept: {budget}");
}

/// Section 16's exit statuses 2 and 3, and 4 for a bad file: a script
/// stopped, a manifest refused, an input script that is not one.
#[test]
fn a_run_says_how_it_ended() {
    // 3: no manifest, and a member the manifest does not have.
    let made = run(&Given { page: Some("console.log(1);"), ..Given::default() });
    assert_eq!(status(&made), Status::Refused, "{}", message(&made));
    assert!(message(&made).contains("does not begin with its manifest"), "{}", message(&made));
    let made = run(&Given { page: Some(r#"/*! nt {"api": 1, "camera": true} */"#), ..Given::default() });
    assert_eq!((status(&made), message(&made).contains("`camera`")), (Status::Refused, true), "{}", message(&made));
    // 3: a page script whose renderer is not its page file's.
    let tti = "PN,10000\r\nSC,0000\r\nOL,1,HELLO\r\n";
    let made = run(&Given { page: Some(r#"/*! nt {"api": 1, "renderer": "regis"} */"#), page_file: Some(tti), ..Given::default() });
    assert_eq!(status(&made), Status::Refused, "{}", message(&made));
    // 2: a context that fails sixteen times in a row is stopped.
    let failing = format!("{CLAIMS}\nnt.addEventListener('tick', () => {{ throw new Error('no'); }});");
    let made = run(&Given { page: Some(&failing), ..Given::default() });
    assert_eq!(status(&made), Status::Stopped, "{}", message(&made));
    assert!(message(&made).contains("Failures"), "{}", message(&made));
    assert!(find(&made, LOG_FILE).is_some(), "a stopped run still gives its log");
    // 4: files that are not what they should be.
    let fine = format!("{CLAIMS}\n");
    let made = run(&Given { page: Some(&fine), input: Some("3 dance wildly\n"), ..Given::default() });
    assert_eq!((status(&made), message(&made).contains("line 1")), (Status::Usage, true), "{}", message(&made));
    let made = run(&Given { arguments: &["--network", "mock", "table"], page: Some(&fine), network: Some("+1 end\n"), ..Given::default() });
    assert_eq!(status(&made), Status::Usage, "{}", message(&made));
    // 0: and a run that simply runs.
    let made = run(&Given { page: Some(&fine), page_file: Some(tti), ..Given::default() });
    assert_eq!(status(&made), Status::Matched, "{}", message(&made));
}

/// Sections 3.2 and 16: a hidden tab's scripts are paused, with no tick
/// and no timer, and their tick count does not jump; `close` ends the
/// run with one handling.
#[test]
fn hide_pauses_and_close_ends_the_run() {
    let page = format!(
        r#"{CLAIMS}
        nt.addEventListener("visibilitychange", () => console.log("hidden", nt.hidden, "at", nt.now()));
        nt.addEventListener("tick", (event) => console.log("tick", event.tick));
        nt.addEventListener("close", () => console.log("close at", nt.now()));"#
    );
    let made = run(&Given { arguments: &["--ticks", "60"], page: Some(&page), input: Some("2 hide\n10 show\n12 close\n"), ..Given::default() });
    assert_eq!(status(&made), Status::Matched, "{}", message(&made));
    let said: Vec<String> = log(&made).into_iter().filter(|line| line.contains(" page log ")).map(|line| line.replacen(" page log", "", 1)).collect();
    assert_eq!(
        said,
        [
            "000000 tick 0",
            "000001 tick 1",
            "000002 tick 2",
            "000002 hidden true at 2",
            "000010 hidden false at 3",
            "000010 tick 3",
            "000011 tick 4",
            "000012 tick 5",
            "000013 close at 5"
        ]
    );
}
