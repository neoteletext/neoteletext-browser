//! Tests of the browser's command line.
#![allow(clippy::unwrap_used, reason = "a test")]

use super::*;

fn parse(given: &[&str]) -> Result<CommandLine, String> {
    arguments(given.iter().map(|argument| String::from(*argument)))
}

/// Decision 113: `--no-keychain` anywhere on the command line is a flag,
/// never an address; without it the secret store is used as before.
#[test]
fn no_keychain_is_a_flag_anywhere_on_the_command_line() {
    let plain = parse(&["https://news.example/teletext/"]).unwrap();
    assert!(!plain.no_keychain);
    let first = parse(&[NO_KEYCHAIN, "--allow-local", "192.168.0.166", "http://192.168.0.166:8098/kitchensink/"]).unwrap();
    assert_eq!(
        first,
        CommandLine {
            addresses: vec![String::from("http://192.168.0.166:8098/kitchensink/")],
            allow_local: vec![String::from("192.168.0.166")],
            no_keychain: true,
        }
    );
    let last = parse(&["https://news.example/teletext/", NO_KEYCHAIN]).unwrap();
    assert!(last.no_keychain);
    assert_eq!(last.addresses, vec![String::from("https://news.example/teletext/")]);
}

/// `--allow-local` still needs its address.
#[test]
fn allow_local_needs_an_address() {
    assert!(parse(&["--allow-local"]).is_err());
}
