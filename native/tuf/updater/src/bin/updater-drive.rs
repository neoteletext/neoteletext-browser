//! A test driver, never shipped: `updater-drive <updater> [check | stage
//! <version>]` starts the updater installed at `<updater>` confined as
//! the broker would (`confine::spawn`, its staging folder and port by the
//! installation's layout and the release's identity), and plays the
//! trusted core's part of the protocol (`update::wire`): it asks for the
//! newest version, or has a version staged, as the user's Install would,
//! and prints the updater's answers. The end-to-end tests of
//! `scripts/updates-e2e.sh` stage with it, since a click on the window
//! cannot be posted to it.

use std::ffi::OsString;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::ExitCode;

use update::layout::Layout;
use update::platform::Platform;
use update::version::Version;
use update::wire::{FromUpdater, ToUpdater, read_from_updater, write_to_updater};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("updater-drive: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (program, rest) = arguments.split_first().ok_or("usage: updater-drive <updater> [check | stage <version>]")?;
    let program = PathBuf::from(program);
    let platform = Platform::this().ok_or("no releases for this platform")?;
    let layout = Layout::of_program(platform.family(), &program).ok_or("the updater is not in an installation")?;
    let staging = layout.staging();
    std::fs::create_dir_all(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let role = confine::Role::Updater { port: release_identity::PORT };
    let environment = confine::environment(role);
    let no_arguments: [OsString; 0] = [];
    let mut process = confine::spawn(&confine::Spawn {
        program: &program,
        arguments: &no_arguments,
        role,
        environment: &environment,
        local_network: false,
        writable: Some(&staging),
    })
    .map_err(|error| format!("cannot start the updater confined: {error}"))?;
    eprintln!("updater-drive: the updater started confined: {}", process.report);
    let mut input = process.stdin.take().ok_or("no input")?;
    let mut output = BufReader::new(process.stdout.take().ok_or("no output")?);
    let mut ask = |message: &ToUpdater| -> Result<FromUpdater, String> {
        write_to_updater(&mut input, message).map_err(|error| error.to_string())?;
        read_from_updater(&mut output).map_err(|error| format!("the updater: {error}"))
    };
    let answer = ask(&ToUpdater::Check)?;
    println!("{answer:?}");
    if let [command, version] = rest
        && command == "stage"
    {
        let version = Version::parse(version).ok_or("not a version")?;
        let answer = ask(&ToUpdater::Download(version))?;
        println!("{answer:?}");
        if !matches!(answer, FromUpdater::Staged(staged) if staged == version) {
            return Err(String::from("not staged"));
        }
    } else if !matches!(answer, FromUpdater::Latest { .. }) {
        return Err(String::from("the check failed"));
    }
    drop(input);
    let _ = process.wait();
    Ok(())
}
