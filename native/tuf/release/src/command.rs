//! The command line: each command's arguments read, and the step it
//! names run.

use std::io::{Read as _, Write as _};
#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;

use update::platform::Platform;
use update::version::Version;

use crate::keys::{self, Source};
use crate::{package, repo, test_identity};

/// The longest message `sign-stdin` signs: the largest metadata file.
const MAX_MESSAGE: u64 = 16 * 1024 * 1024;

fn platform(text: Option<&String>) -> Result<Platform, String> {
    let text = text.ok_or("a platform is needed")?;
    Platform::from_name(text).ok_or_else(|| format!("{text}: not a platform"))
}

fn version(text: Option<&String>) -> Result<Version, String> {
    let text = text.ok_or("a version is needed")?;
    Version::parse(text).ok_or_else(|| format!("{text}: not a version"))
}

fn path(text: Option<&String>) -> Result<PathBuf, String> {
    text.map(PathBuf::from).ok_or_else(|| String::from("a path is needed"))
}

/// The values of every `--name value` in `arguments`.
fn options(arguments: &[String], name: &str) -> Vec<String> {
    arguments.windows(2).filter(|pair| pair.first().is_some_and(|first| first == name)).filter_map(|pair| pair.get(1).cloned()).collect()
}

fn one(arguments: &[String], name: &str) -> Result<Source, String> {
    match options(arguments, name).as_slice() {
        [only] => Source::parse(only),
        _ => Err(format!("{name} must be given once")),
    }
}

/// `--timestamp-days N`, the timestamp's life; 7 days unless a test asks
/// for another, an expired one among them.
fn timestamp_days(arguments: &[String]) -> Result<i64, String> {
    options(arguments, "--timestamp-days")
        .first()
        .map_or(Ok(repo::TIMESTAMP_DAYS), |days| days.parse().map_err(|_| String::from("--timestamp-days must be a number")))
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string())
}

pub(crate) fn run(arguments: &[String]) -> Result<(), String> {
    let (command, rest) = arguments.split_first().ok_or("a command is needed; see the program's documentation")?;
    match command.as_str() {
        "layout" => package::layout(platform(rest.first())?, version(rest.get(1))?, &path(rest.get(2))?, &path(rest.get(3))?),
        "manifest" => package::manifest(&path(rest.first())?, platform(rest.get(1))?, version(rest.get(2))?).map(|_| ()),
        "sign-linux" => package::sign_linux(&path(rest.first())?, version(rest.get(1))?, &Source::parse(rest.get(2).ok_or("a key is needed")?)?),
        "sign-windows" => package::sign_windows(&path(rest.first())?, version(rest.get(1))?, &Source::parse(rest.get(2).ok_or("a key is needed")?)?),
        "sign-file" => package::sign_file(&path(rest.first())?, &Source::parse(rest.get(1).ok_or("a key is needed")?)?),
        "check" => {
            let last = options(rest, "--last-launcher").first().map(PathBuf::from);
            let version =
                package::check(&path(rest.first())?, platform(rest.get(1))?, last.as_deref(), rest.iter().any(|argument| argument == "--launcher-release"))?;
            println!("{version}: the launcher accepts it (identity: {})", release_identity::NAME);
            Ok(())
        }
        "targets" => package::targets(&path(rest.first())?, platform(rest.get(1))?, &path(rest.get(2))?),
        "root" => {
            let roots = options(rest, "--root").iter().map(|text| Source::parse(text)).collect::<Result<Vec<_>, _>>()?;
            let old = options(rest, "--old-root").iter().map(|text| Source::parse(text)).collect::<Result<Vec<_>, _>>()?;
            let keys =
                repo::RootKeys { root: roots, targets: one(rest, "--targets")?, snapshot: one(rest, "--snapshot")?, timestamp: one(rest, "--timestamp")? };
            let number: u64 = rest.get(1).and_then(|text| text.parse().ok()).ok_or("a root version is needed")?;
            runtime()?.block_on(repo::root(&path(rest.first())?, &keys, number, &old))
        }
        "publish" => {
            let keys =
                repo::RootKeys { root: Vec::new(), targets: one(rest, "--targets")?, snapshot: one(rest, "--snapshot")?, timestamp: one(rest, "--timestamp")? };
            runtime()?.block_on(repo::publish(&path(rest.first())?, &path(rest.get(1))?, &path(rest.get(2))?, &keys, timestamp_days(rest)?))
        }
        "timestamp" => runtime()?.block_on(repo::timestamp(&path(rest.first())?, &path(rest.get(1))?, &one(rest, "--timestamp")?, timestamp_days(rest)?)),
        "test-identity" => {
            let port =
                options(rest, "--port").first().map_or(Ok(release_identity::PORT), |port| port.parse().map_err(|_| String::from("--port must be a number")))?;
            let host = options(rest, "--host").first().cloned().unwrap_or_else(|| String::from("localhost"));
            let requirement = options(rest, "--macos-requirement").first().cloned();
            let signer = options(rest, "--windows-signer").first().cloned();
            runtime()?.block_on(test_identity::make(&path(rest.first())?, &host, port, requirement.as_deref(), signer.as_deref()))
        }
        "key" => {
            let out = path(rest.first())?;
            if out.exists() {
                return Err(format!("{} exists; a key is never overwritten", out.display()));
            }
            update::files::write_synced(&out, &keys::generate()?, false).map_err(|error| error.to_string())
        }
        "public" => {
            println!("{}", update::hex(&Source::parse(rest.first().ok_or("a key is needed")?)?.public()?));
            Ok(())
        }
        "pem" => {
            print!("{}", update::signature::public_key_pem(&Source::parse(rest.first().ok_or("a key is needed")?)?.public()?));
            Ok(())
        }
        "sign-stdin" => {
            let source = Source::File(path(rest.first())?);
            let mut message = Vec::new();
            std::io::stdin().take(MAX_MESSAGE).read_to_end(&mut message).map_err(|error| error.to_string())?;
            std::io::stdout().write_all(&source.sign(&message)?).map_err(|error| error.to_string())
        }
        #[cfg(windows)]
        "signer-key" => {
            println!("{}", update::hex(&os_install::signer_key(Path::new(rest.first().ok_or("a program is needed")?))?));
            Ok(())
        }
        other => Err(format!("{other}: no such command")),
    }
}
