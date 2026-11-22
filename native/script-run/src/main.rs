//! `neoteletext-script-run`: the standalone runner (script-api.md section
//! 16). See the library, `script_run`.
//!
//! Two processes. This one reads the command line and every file, writes
//! every file, and runs no script. It starts itself again as the worker,
//! confined as a decoder is (its pipes and nothing else), gives it the
//! files as bytes, and is given the run's files back. The worker runs the
//! scripts, the decoder's engine, the mixer and the PNG writer: everything
//! that reads untrusted bytes. With the live network the run cannot be
//! confined so, and says that it is not.

// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use script_run::args::{self, NetworkMode, Options};
use script_run::blobs::{self, Blob, find};
use script_run::expect::{self, EXPECT_FILE};
use script_run::files::Watched;
use script_run::limits::{MAX_BLOB_BYTES, MAX_EXPECT_BYTES, WATCH_INTERVAL};
use script_run::run::{LOG_FILE, MESSAGE, RECORDING, REPORT_JSON, REPORT_TEXT, SOUND_FILE, STATUS, is_kept};
use script_run::worker::run_given;
use script_run::{Failure, Status, files};

/// The argument that makes this program the worker.
const WORKER: &str = "--worker";

/// The worker: confined first, before any thread exists and before a
/// byte of anything untrusted is read.
fn worker() -> ExitCode {
    match confine::apply(confine::Role::Decoder, None) {
        Ok(report) => eprintln!("neoteletext-script-run: the run is confined as a decoder is: {report}"),
        Err(why) => {
            eprintln!(
                "neoteletext-script-run: the worker could not be confined, so nothing was run: {why}\n\
                 A script is run confined as the browser confines one. On Linux that needs a user namespace, which Ubuntu lets only\n\
                 programs with an AppArmor profile make (scripts/apparmor-development.sh covers the main checkout's build folder).\n\
                 To run a script you trust with no confinement, give --unconfined."
            );
            return ExitCode::from(Status::Usage.code());
        }
    }
    let given = match blobs::read(&mut std::io::stdin().lock()) {
        Ok(given) => given,
        Err(error) => {
            eprintln!("neoteletext-script-run: {error}");
            return ExitCode::from(Status::Usage.code());
        }
    };
    let made = run_given(&given);
    match blobs::write(&mut std::io::stdout().lock(), &made) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-script-run: {error}");
            ExitCode::from(Status::Usage.code())
        }
    }
}

/// Starts the worker, gives it `given` and takes what it made.
fn in_worker(given: &[Blob]) -> Result<Vec<Blob>, Failure> {
    let fail = |why: String| Failure::usage(format!("the worker process: {why}"));
    let program = std::env::current_exe().map_err(|error| fail(error.to_string()))?;
    let arguments = [std::ffi::OsString::from(WORKER)];
    let mut process = confine::spawn(&confine::Spawn {
        program: &program,
        arguments: &arguments,
        role: confine::Role::Decoder,
        environment: &[],
        local_network: false,
        udp_port: None,
        writable: None,
    })
    .map_err(|error| fail(error.to_string()))?;
    let (Some(mut to), Some(mut from)) = (process.stdin.take(), process.stdout.take()) else { return Err(fail("it has no pipes".into())) };
    // Written on a thread of its own: the worker reads everything before
    // it writes anything, but a pipe's room is not ours to count on.
    let sent = given.to_vec();
    let writer = std::thread::spawn(move || {
        let written = blobs::write(&mut to, &sent);
        drop(to);
        written
    });
    let made = blobs::read(&mut from);
    let _ = writer.join();
    let ended = process.wait().map_err(|error| fail(error.to_string()))?;
    // The watchdog of a run under --strict-time ends the worker with the
    // status of a stopped script, having written nothing.
    if made.is_err() && ended.code() == Some(i32::from(Status::Stopped.code())) {
        return Err(Failure {
            status: Status::Stopped,
            message: "the script was stopped: it passed a time limit and did not return (the watchdog ended the run)".into(),
        });
    }
    made.map_err(|error| fail(format!("{error} (it ended with {ended})")))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder).map_err(|error| Failure::usage(format!("{}: {error}", folder.display())))?;
    }
    std::fs::write(path, bytes).map_err(|error| Failure::usage(format!("{}: {error}", path.display())))
}

/// Compares the run with the kept set, or writes the set.
fn kept(options: &Options, folder: &Path, made: &[Blob]) -> Result<Status, Failure> {
    let expect_file = folder.join(EXPECT_FILE);
    let old = if expect_file.is_file() { Some(expect::parse(&files::read(&expect_file, MAX_EXPECT_BYTES)?).map_err(Failure::usage)?) } else { None };
    if options.bless {
        let new = expect::blessed(old.as_ref(), made);
        for (name, bytes) in expect::kept_files(made) {
            write_file(&folder.join(name), bytes)?;
        }
        // A file the old set kept and this run did not make is no longer
        // part of the set. Only names `expect.json` itself listed are
        // removed, and the set is a committed one: its history is the way
        // back.
        for (name, _) in old.iter().flat_map(|old| &old.files) {
            if !new.files.iter().any(|(kept, _)| kept == name) {
                let _ = std::fs::remove_file(folder.join(name));
            }
        }
        write_file(&expect_file, &expect::write(&new))?;
        println!("kept {} files in {} (version {} of the set)", new.files.len(), folder.display(), new.version);
        return Ok(Status::Matched);
    }
    let Some(old) = old else {
        return Err(Failure::usage(format!("{}: no kept results there; run with --bless to write them", expect_file.display())));
    };
    let found = expect::differences(&old, made, &mut |name| files::read(&folder.join(name), MAX_BLOB_BYTES).ok());
    for difference in &found {
        eprintln!("neoteletext-script-run: differs: {difference}");
    }
    if found.is_empty() {
        println!("matched the {} kept files in {} (version {} of the set)", old.files.len(), folder.display(), old.version);
        Ok(Status::Matched)
    } else {
        Ok(Status::Differed)
    }
}

/// One run, from the command line to its exit status; gives the paths
/// it read, for `--watch`.
fn once(options: &Options, arguments: &[String]) -> (Result<Status, Failure>, Vec<Watched>) {
    let before = files::changed(&options.target);
    let (given, paths) = match files::gather(options, arguments) {
        Ok(gathered) => gathered,
        Err(failure) => return (Err(failure), vec![(options.target.clone(), before)]),
    };
    let result = (|| {
        let made = if options.network.is_live() || options.unconfined {
            eprintln!(
                "neoteletext-script-run: this run is NOT confined ({})",
                if options.unconfined { "--unconfined" } else { "the live network is asked for" }
            );
            run_given(&given)
        } else {
            in_worker(&given)?
        };
        // What the worker wrote is untrusted: only the names a run makes
        // are taken, each to the place the command line gave it.
        let status = find(&made, STATUS)
            .and_then(|bytes| bytes.first())
            .and_then(|digit| Status::of(digit.wrapping_sub(b'0')))
            .ok_or_else(|| Failure::usage("the worker gave no status"))?;
        let message = find(&made, MESSAGE).map(|bytes| String::from_utf8_lossy(bytes).into_owned());
        if let Some(log) = find(&made, LOG_FILE) {
            std::io::stdout().write_all(log).map_err(|error| Failure::usage(error.to_string()))?;
        }
        if let Some(report) = find(&made, REPORT_TEXT) {
            std::io::stdout().write_all(report).map_err(|error| Failure::usage(error.to_string()))?;
        }
        if matches!(status, Status::Refused | Status::Usage) {
            return Err(Failure { status, message: message.unwrap_or_default() });
        }
        if let Some(folder) = &options.out {
            for (name, bytes) in made.iter().filter(|(name, _)| is_kept(name)) {
                write_file(&folder.join(name), bytes)?;
            }
        }
        if let (Some(path), Some(bytes)) = (&options.wav, find(&made, SOUND_FILE)) {
            write_file(path, bytes)?;
        }
        if let (Some(path), Some(bytes)) = (&options.report, find(&made, REPORT_JSON)) {
            write_file(path, bytes)?;
        }
        if let (NetworkMode::Record(path), Some(bytes)) = (&options.network, find(&made, RECORDING)) {
            write_file(path, bytes)?;
        }
        if status == Status::Stopped {
            return Err(Failure { status, message: message.unwrap_or_else(|| "the script was stopped".into()) });
        }
        match &options.expect {
            Some(folder) => kept(options, folder, &made),
            None => Ok(status),
        }
    })();
    (result, paths)
}

fn report(result: &Result<Status, Failure>) -> Status {
    match result {
        Ok(status) => *status,
        Err(failure) => {
            eprintln!("neoteletext-script-run: {failure}");
            failure.status
        }
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|first| first == WORKER) {
        return worker();
    }
    let options = match args::parse(&arguments) {
        Ok(options) => options,
        Err(failure) => {
            eprintln!("neoteletext-script-run: {failure}");
            return ExitCode::from(failure.status.code());
        }
    };
    loop {
        let (result, paths) = once(&options, &arguments);
        let status = report(&result);
        if !options.watch {
            return ExitCode::from(status.code());
        }
        println!("-- exit status {}; watching {} files for a change --", status.code(), paths.len());
        // The standard library has no notice of a file's change, and the
        // runner takes no library for one: the files' times are looked at
        // every WATCH_INTERVAL (the runner alone, for a person at a
        // keyboard; recorded as a departure from "wait on what you need").
        // Each file's time was taken as its reading began, so a change
        // made while the run was going starts the next at once.
        while paths.iter().all(|(path, seen)| files::changed(path) == *seen) {
            std::thread::sleep(WATCH_INTERVAL);
        }
    }
}
