//! The updater process (`installation-and-updates.md` section 3):
//! `neoteletext-updater [--proxy URL]`, started by the trusted core
//! through the broker, confined to the update host's port (or the
//! proxy's) and its staging folder (requirement 13.18). It reads the
//! core's messages on its standard input and answers on its standard
//! output (`update::wire`).

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::io::{BufReader, BufWriter};
use std::process::ExitCode;

use neoteletext_web::proxy::Proxy;
use update::layout::Layout;
use update::platform::Platform;
use update::wire::{FromUpdater, ToUpdater, read_to_updater, write_from_updater};
use updater::{Config, Https, now};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-updater: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The proxy the core passed, if any.
fn proxy() -> Result<Option<Proxy>, String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [] => Ok(None),
        [flag, url] if flag == "--proxy" => Proxy::parse(url).map(Some),
        _ => Err(String::from("usage: neoteletext-updater [--proxy URL]")),
    }
}

fn run() -> Result<(), String> {
    let proxy = proxy()?;
    let platform = Platform::this().ok_or("there are no releases for this platform")?;
    let program = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
    let layout = Layout::of_program(platform.family(), &program).ok_or("this program is not in an installation")?;
    let staging = layout.staging();
    let port = proxy.as_ref().map_or(release_identity::PORT, |proxy| proxy.port);
    // First, before any thread: the sandbox (decision 58).
    let report = confine::apply(confine::Role::Updater { port }, Some(&staging))?;
    eprintln!("neoteletext-updater: confined: {report}");
    let https = Https::new(release_identity::HOST, release_identity::PORT, release_identity::TLS_ROOT, proxy)?;
    let config = release_identity::TUF_ROOT.map(|root| Config {
        https,
        host: release_identity::HOST.to_owned(),
        port: release_identity::PORT,
        root: root.to_vec(),
        staging: staging.clone(),
        platform,
    });
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string())?;
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = BufWriter::new(std::io::stdout().lock());
    loop {
        let message = match read_to_updater(&mut input) {
            Ok(message) => message,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(format!("the trusted core broke the protocol: {error}")),
        };
        let answer = match &config {
            None => FromUpdater::Failed { reason: String::from("this build carries no update keys"), last_success: None },
            Some(config) => match message {
                ToUpdater::Check => match runtime.block_on(config.check()) {
                    Ok(version) => FromUpdater::Latest { version, checked: now() },
                    Err(reason) => FromUpdater::Failed { reason, last_success: config.last_success() },
                },
                ToUpdater::Download(version) => match runtime.block_on(config.download(version)) {
                    Ok(()) => FromUpdater::Staged(version),
                    Err(reason) => FromUpdater::Failed { reason, last_success: config.last_success() },
                },
            },
        };
        if let FromUpdater::Failed { reason, .. } = &answer {
            eprintln!("neoteletext-updater: {reason}");
        }
        write_from_updater(&mut output, &answer).map_err(|error| format!("cannot answer the trusted core: {error}"))?;
    }
}
