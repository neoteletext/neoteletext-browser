//! The program `neoteletext-script-run` (script-api.md section 16): its
//! exit statuses, the kept run it is compared with in the gates, a
//! channel's folder run with one command, `--watch`, and the confined
//! worker.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const PROGRAM: &str = env!("CARGO_BIN_EXE_neoteletext-script-run");

/// The runner's own page and its kept run.
fn beacon() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pages/beacon")
}

/// A new, empty folder for one test.
fn scratch(name: &str) -> PathBuf {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("script-run-{name}"));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Runs the program in its own process, not confined: the tests of what
/// a run does, which must pass wherever they are built.
fn runner(arguments: &[&str]) -> Output {
    Command::new(PROGRAM).arg("run").args(arguments).arg("--unconfined").output().unwrap()
}

fn said(output: &Output) -> String {
    format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// The command line of the kept run: the folder, a channel's, with its
/// page script named.
fn beacon_arguments() -> Vec<String> {
    let folder = beacon();
    let path = |name: &str| folder.join(name).to_str().unwrap().to_owned();
    [path(""), "--script".into(), path("beacon.js"), "--ticks".into(), "40".into(), "--input".into(), path("input.txt"), "--png-at".into(), "0,39".into()]
        .into_iter()
        .chain(["--expect".to_owned(), path("expect")])
        .collect()
}

/// "Compares with kept results": the runner's own page run against the
/// results kept in the repository. This is the run the gates make: a
/// change to a picture, the sound, the log or the budget fails it, on
/// any machine.
#[test]
fn the_kept_run_of_the_runners_own_page_matches() {
    let arguments = beacon_arguments();
    let output = runner(&arguments.iter().map(String::as_str).collect::<Vec<&str>>());
    assert_eq!(output.status.code(), Some(0), "{}", said(&output));
    assert!(said(&output).contains("matched the 5 kept files"), "{}", said(&output));
}

/// "Tells a caller the result": exit status 0 ran and matched, 1
/// differed, 2 stopped, 3 refused, 4 bad arguments or files; and
/// `--bless` writes the set and raises its version only for a change.
#[test]
fn each_exit_status() {
    let folder = scratch("statuses");
    let page = folder.join("page.js");
    let kept = folder.join("kept");
    let write = |text: &str| std::fs::write(&page, text).unwrap();
    let manifest = r#"/*! nt {"api": 1, "tick": 1} */"#;
    let arguments = |extra: &[&'static str]| -> Vec<String> {
        let mut all = vec![page.to_str().unwrap().to_owned(), "--ticks".into(), "20".into(), "--expect".into(), kept.to_str().unwrap().to_owned()];
        all.extend(extra.iter().map(|one| (*one).to_owned()));
        all
    };
    let run = |extra: &[&'static str]| {
        let all = arguments(extra);
        runner(&all.iter().map(String::as_str).collect::<Vec<&str>>())
    };
    write(&format!("{manifest}\nconsole.log('one');"));
    // 4: nothing kept yet, and it says how to keep it.
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(4), "{}", said(&output));
    assert!(said(&output).contains("--bless"), "{}", said(&output));
    // 0: blessed, then matched; blessing again changes nothing.
    assert_eq!(run(&["--bless"]).status.code(), Some(0));
    let first = std::fs::read_to_string(kept.join("expect.json")).unwrap();
    assert!(first.contains("\"form\": 1") && first.contains("\"version\": 1") && first.contains("tick-000019.png"), "{first}");
    assert_eq!(run(&[]).status.code(), Some(0));
    assert_eq!(run(&["--bless"]).status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(kept.join("expect.json")).unwrap(), first, "nothing changed, so neither did the version");
    // 1: the page says something else.
    write(&format!("{manifest}\nconsole.log('two');"));
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(1), "{}", said(&output));
    assert!(said(&output).contains("differs: log.txt: differs"), "{}", said(&output));
    // Blessed again, the version is raised.
    assert_eq!(run(&["--bless"]).status.code(), Some(0));
    assert!(std::fs::read_to_string(kept.join("expect.json")).unwrap().contains("\"version\": 2"));
    // 2: stopped. Sixteen failures in a row; and under the browser's
    // time limits, a top level that never returns.
    write(&format!("{manifest}\nnt.addEventListener('tick', () => {{ throw new Error('no'); }});"));
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(2), "{}", said(&output));
    write(&format!("{manifest}\nfor (;;) {{}}"));
    let output = run(&["--strict-time"]);
    assert_eq!(output.status.code(), Some(2), "{}", said(&output));
    // 3: refused.
    write("console.log('no manifest');");
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(3), "{}", said(&output));
    // 4: bad arguments and files.
    for bad in
        [&["--ticks", "0"][..], &["--page", "9ZZ"], &["--bless"], &["--network", "carrier-pigeon"], &["--input", "/nonexistent/input"], &["--frobnicate"]]
    {
        let all: Vec<&str> = [page.to_str().unwrap()].into_iter().chain(bad.iter().copied()).collect();
        let output = runner(&all);
        assert_eq!(output.status.code(), Some(4), "{bad:?}: {}", said(&output));
    }
    let output = Command::new(PROGRAM).output().unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(said(&output).contains("usage:"));
}

/// "Runs a channel folder, one page of it, or one script": a folder with
/// a channel file and a page file, the channel file and the page file
/// read by the engine's readers, with one command; and the files a run
/// writes where the command line says.
#[test]
fn a_channel_folder_runs_with_one_command() {
    let folder = scratch("channel");
    std::fs::write(folder.join("channel.ttx"), r#"{"name": "Harbour", "initial_page": "101"}"#).unwrap();
    std::fs::write(folder.join("101.tti"), "PN,10100\r\nSC,0000\r\nOL,4,TIDES\r\n").unwrap();
    std::fs::write(
        folder.join("101.js"),
        r#"/*! nt {"api": 1, "renderer": "teletext"} */ console.log("on", nt.page.number, "of", nt.channel.title, nt.channel.url, nt.page.url);"#,
    )
    .unwrap();
    std::fs::write(folder.join("channel.js"), r#"/*! nt {"api": 1} */ nt.addEventListener("pageshow", (event) => console.log("shows", event.page.url));"#)
        .unwrap();
    let out = folder.join("out");
    let report = folder.join("report.json");
    let wav = folder.join("heard.wav");
    let output = runner(&[
        folder.to_str().unwrap(),
        "--script",
        folder.join("101.js").to_str().unwrap(),
        "--channel-script",
        folder.join("channel.js").to_str().unwrap(),
        "--ticks",
        "3",
        "--out",
        out.to_str().unwrap(),
        "--report",
        report.to_str().unwrap(),
        "--wav",
        wav.to_str().unwrap(),
        "--channel-url",
        "https://harbour.test/boats/",
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", said(&output));
    let all = said(&output);
    assert!(all.contains("page log on 101 of Harbour https://harbour.test/boats/ https://harbour.test/boats/101.tti"), "{all}");
    assert!(all.contains("channel log shows https://harbour.test/boats/101"), "{all}");
    assert!(out.join("tick-000002.png").is_file() && out.join("log.txt").is_file() && out.join("budget.json").is_file());
    assert_eq!(std::fs::read(&wav).unwrap().len(), 44 + 3 * 800 * 2);
    let report = std::fs::read_to_string(&report).unwrap();
    for member in ["\"stepMicroseconds\"", "\"ninthDecile\"", "\"outputBytes\"", "\"network\"", "\"events\"", "\"budget\"", "\"heapBytes\"", "\"most\""] {
        assert!(report.contains(member), "no {member} in {report}");
    }
    // The page-level script of another page than the channel's first.
    std::fs::write(folder.join("1A0.js"), r#"/*! nt {"api": 1, "renderer": "regis"} */ console.log("a JavaScript page", nt.page.number, nt.page.renderer);"#)
        .unwrap();
    let output = runner(&[folder.to_str().unwrap(), "--page", "1A0", "--script", folder.join("1A0.js").to_str().unwrap(), "--ticks", "1"]);
    assert_eq!(output.status.code(), Some(0), "{}", said(&output));
    assert!(said(&output).contains("a JavaScript page 1A0 regis"), "{}", said(&output));
}

/// "Reloads": `--watch` runs again when a file it read changes.
#[test]
fn watch_runs_again_when_the_script_changes() {
    let folder = scratch("watch");
    let page = folder.join("page.js");
    std::fs::write(&page, "/*! nt {\"api\": 1} */ console.log('first');").unwrap();
    let mut child = Command::new(PROGRAM)
        .args(["run", page.to_str().unwrap(), "--ticks", "1", "--watch", "--unconfined"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines().map(Result::unwrap);
    let mut until_watching = |wanted: &str| {
        let mut seen = false;
        for line in lines.by_ref() {
            seen |= line.contains(wanted);
            if line.contains("watching") {
                break;
            }
        }
        seen
    };
    assert!(until_watching("page log first"));
    std::fs::write(&page, "/*! nt {\"api\": 1} */ console.log('second, and longer');").unwrap();
    assert!(until_watching("page log second, and longer"));
    child.kill().unwrap();
    child.wait().unwrap();
}

/// "It reads untrusted scripts like the browser does and so confines
/// itself the same way": with no `--unconfined` the run is made in a
/// worker confined as the script host is, and gives the same kept files. On a
/// Linux that restricts user namespaces this needs the build folder to
/// be one the system lets make them (`scripts/apparmor-development.sh`
/// covers the main checkout's `native/target`).
#[test]
fn the_confined_worker_gives_the_same_run() {
    let output = Command::new(PROGRAM).arg("run").args(beacon_arguments()).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", said(&output));
    assert!(said(&output).contains("confined as the script host is"), "{}", said(&output));
    assert!(said(&output).contains("matched the 5 kept files"), "{}", said(&output));
}

/// The security review's hostile scripts (TODO-7267, TODO-7336), each
/// run by the program under the browser's time limits (`--strict-time`):
/// the runner's own part of them is that every one ends as a run or as a
/// stop, exit status 0 or 2, with its log, and never hangs the runner,
/// crashes it or passes as bad arguments; and that the two that never
/// return are told as stopped.
#[test]
fn the_hostile_scripts_each_end_as_a_run_or_a_stop() {
    let hostile = Path::new(env!("CARGO_MANIFEST_DIR")).join("../script/tests/hostile");
    let mut scripts: Vec<PathBuf> =
        std::fs::read_dir(&hostile).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.extension().is_some_and(|found| found == "js")).collect();
    scripts.sort();
    assert!(scripts.len() >= 24, "the review's scripts are in {}", hostile.display());
    let mut stopped = Vec::new();
    for script in &scripts {
        let output = runner(&[script.to_str().unwrap(), "--ticks", "90", "--strict-time", "--sound-allowed"]);
        let name = script.file_name().unwrap().to_str().unwrap().to_owned();
        let code = output.status.code();
        assert!(matches!(code, Some(0 | 2)), "{name} ended with {code:?}: {}", said(&output));
        if code == Some(2) {
            stopped.push(name);
        }
    }
    for never_returns in ["loop.js", "toploop.js"] {
        assert!(stopped.iter().any(|name| name == never_returns), "{never_returns} was not told as stopped; stopped: {stopped:?}");
    }
    println!("stopped under the browser's limits: {stopped:?}");
}

/// The teletext API's worked example (script-api.md section 5.8; work
/// package T1's `native/script/tests/teletext/101.js`) run against its
/// kept results: the page plane it writes is drawn by the decoder's
/// engine. Until the decoder's side of a step exists (work package F4)
/// the runner's stand-in does not draw the example's overlay band or its
/// defined colour, and the report must say so, never pass silently.
#[test]
fn the_teletext_worked_example_matches_its_kept_run() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let kept = manifest.join("tests/pages/harbour");
    let output = runner(&[
        manifest.join("../script/tests/teletext/101.js").to_str().unwrap(),
        "--page",
        "101",
        "--ticks",
        "61",
        "--input",
        kept.join("input.txt").to_str().unwrap(),
        "--png-at",
        "0,60",
        "--expect",
        kept.join("expect").to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(0), "{}", said(&output));
    let all = said(&output);
    assert!(all.contains("matched the 5 kept files"), "{all}");
    assert!(all.contains("runner note navigate 103") && all.contains("runner note navigate 102"), "the keys 2 and 1 navigate: {all}");
    assert!(all.contains("NOT DRAWN") && all.contains("teletext enhancement, teletext settings"), "{all}");
}
