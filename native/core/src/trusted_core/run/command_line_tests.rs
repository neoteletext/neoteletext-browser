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
    assert_eq!(plain.no_keychain, !KEYCHAIN_BY_DEFAULT);
    let first = parse(&[NO_KEYCHAIN, "--allow-local", "192.168.0.166", "http://192.168.0.166:8098/kitchensink/"]).unwrap();
    assert_eq!(
        first,
        CommandLine {
            addresses: vec![String::from("http://192.168.0.166:8098/kitchensink/")],
            allow_local: vec![String::from("192.168.0.166")],
            no_keychain: true,
            input_trace: false,
        }
    );
    let last = parse(&["https://news.example/teletext/", NO_KEYCHAIN]).unwrap();
    assert!(last.no_keychain);
    assert_eq!(last.addresses, vec![String::from("https://news.example/teletext/")]);
}

/// The owner, 28 October 2026: on macOS the Keychain is off unless
/// `--keychain` is given, until NeoTeletext is signed with its Developer
/// ID; `--keychain` turns it on anywhere on the command line, and
/// `--no-keychain` after it off again.
#[test]
fn the_keychain_is_off_by_default_on_macos_and_keychain_turns_it_on() {
    assert_eq!(KEYCHAIN_BY_DEFAULT, !cfg!(target_os = "macos"));
    assert!(!parse(&[KEYCHAIN, "https://news.example/teletext/"]).unwrap().no_keychain);
    assert!(parse(&[KEYCHAIN, NO_KEYCHAIN]).unwrap().no_keychain);
}

/// `--allow-local` still needs its address.
#[test]
fn allow_local_needs_an_address() {
    assert!(parse(&["--allow-local"]).is_err());
}

/// The owner, 27 October 2026: `--input-trace` is a flag anywhere on the
/// command line, never an address.
#[test]
fn input_trace_is_a_flag() {
    assert!(!parse(&["https://news.example/teletext/"]).unwrap().input_trace);
    let traced = parse(&["https://news.example/teletext/", INPUT_TRACE, NO_KEYCHAIN]).unwrap();
    assert!(traced.input_trace && traced.no_keychain);
    assert_eq!(traced.addresses, vec![String::from("https://news.example/teletext/")]);
}
