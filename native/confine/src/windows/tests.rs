//! The Windows sandbox's own tests: profiles made at once, and no helper
//! typing into the console it was started from (finding F-22).

#![expect(unsafe_code, reason = "the console calls a helper could make, tried as a helper (requirement 10.12, decision 100)")]

use std::ffi::OsString;
use std::os::windows::process::CommandExt as _;
use std::ptr;

use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, FlushConsoleInputBuffer, FreeConsole, INPUT_RECORD, KEY_EVENT, PeekConsoleInputW, WriteConsoleInputW,
};
use windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE;

use super::handles::wide;
use super::*;
use crate::environment;

/// A profile of this test's own, made and removed by it.
const TEST_PROFILE: &str = "NeoTeletext.ConfineTest";
/// How many times the profile is made afresh.
const ROUNDS: usize = 5;
/// How many helpers start at once in each new profile.
const AT_ONCE: usize = 8;

/// Helpers started at once by threads of one process, in a profile
/// none of them has made yet, all start: no thread starts a process in
/// a profile another is still making. Before, a decoder started while
/// another thread made the profile failed with "the system cannot find
/// the file specified".
#[test]
fn helpers_started_at_once_in_a_new_profile_all_start() {
    let program = std::env::current_exe().unwrap();
    for round in 0..ROUNDS {
        container::remove(TEST_PROFILE);
        let starts: Vec<_> = (0..AT_ONCE)
            .map(|_| {
                let program = program.clone();
                std::thread::spawn(move || -> io::Result<()> {
                    let sid = container::profile(TEST_PROFILE)?;
                    // A name no test has: the copy runs nothing and ends.
                    let arguments = [OsString::from("--exact"), OsString::from("no test has this name")];
                    let environment = environment(Role::Decoder);
                    let spawn = Spawn {
                        program: &program,
                        arguments: &arguments,
                        role: Role::Decoder,
                        environment: &environment,
                        local_network: false,
                        writable: None,
                    };
                    let mut process = spawn_in(&spawn, &sid)?;
                    process.wait().map(drop)
                })
            })
            .collect();
        let failed: Vec<String> = starts.into_iter().filter_map(|start| start.join().unwrap().err()).map(|error| error.to_string()).collect();
        container::remove(TEST_PROFILE);
        assert!(failed.is_empty(), "round {round}: {} of {AT_ONCE} did not start: {failed:?}", failed.len());
    }
}

/// What a copy of this test program is: the console's owner, or the
/// helper that tries to type into it.
const CONSOLE_OWNER: &str = "NEOTELETEXT_CONFINE_CONSOLE_OWNER";
const CONSOLE_TYPIST: &str = "NEOTELETEXT_CONFINE_CONSOLE_TYPIST";
/// What the typist types: a comment to `cmd.exe`, harmless if it ever
/// reached a shell. Each character is a key's press and its release.
const TYPED: &str = "rem";
/// Every role a helper is started in.
const ROLES: [Role; 6] = [Role::Decoder, Role::Network, Role::Interface, Role::Core, Role::Sound, Role::Updater { port: 443 }];

/// The console's input, as a helper or its owner opens it, if it can.
fn console_input() -> Option<Owned> {
    // SAFETY: the name is a NUL-terminated wide string alive for the call; the other
    // arguments are constants and nulls the call allows.
    let handle =
        unsafe { CreateFileW(wide("CONIN$").as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, ptr::null(), OPEN_EXISTING, 0, 0) };
    (handle != INVALID_HANDLE_VALUE).then(|| Owned::new(handle))
}

/// `TYPED` as key events, each character pressed and released.
fn key_events() -> Vec<INPUT_RECORD> {
    let mut records = Vec::new();
    for character in TYPED.encode_utf16() {
        for down in [1, 0] {
            // SAFETY: an all-zero INPUT_RECORD is a valid value of a plain C struct.
            let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
            record.EventType = u16::try_from(KEY_EVENT).unwrap();
            record.Event.KeyEvent.bKeyDown = down;
            record.Event.KeyEvent.wRepeatCount = 1;
            record.Event.KeyEvent.uChar.UnicodeChar = character;
            records.push(record);
        }
    }
    records
}

/// Writes `TYPED` into a console's input, as if typed.
fn type_into(input: &Owned) {
    let records = key_events();
    let mut written = 0_u32;
    // SAFETY: the handle is open; the records are a live local of the length given.
    unsafe { WriteConsoleInputW(input.raw(), records.as_ptr(), u32::try_from(records.len()).unwrap(), &raw mut written) };
}

/// The most input events looked at in one count: far more than one typist
/// types.
const MAX_EVENTS: usize = 256;

/// How many key events wait in a console's input; it is emptied after.
/// Only key events count: the console's own window may add a focus or
/// menu event of its own as it opens.
fn keys_waiting(input: &Owned) -> usize {
    // SAFETY: an all-zero INPUT_RECORD is a valid value of a plain C struct.
    let mut records: Vec<INPUT_RECORD> = vec![unsafe { std::mem::zeroed() }; MAX_EVENTS];
    let mut read = 0_u32;
    // SAFETY: the handle is the owner's open console input; the records are a live local of
    // the length given, the count a live local.
    unsafe {
        PeekConsoleInputW(input.raw(), records.as_mut_ptr(), u32::try_from(records.len()).unwrap(), &raw mut read);
        FlushConsoleInputBuffer(input.raw());
    }
    records.iter().take(usize::try_from(read).unwrap()).filter(|record| u32::from(record.EventType) == KEY_EVENT).count()
}

/// The typist: started confined by `console_owner`, it tries every way a
/// helper has into a console: the one it was started with, and its
/// parent's, attached to. Run by that test only.
#[test]
fn console_typist() {
    if std::env::var_os(CONSOLE_TYPIST).is_none() {
        return;
    }
    if let Some(input) = console_input() {
        type_into(&input);
    }
    // SAFETY: FreeConsole and AttachConsole take nothing but a constant.
    if unsafe {
        FreeConsole();
        AttachConsole(ATTACH_PARENT_PROCESS)
    } != 0
        && let Some(input) = console_input()
    {
        type_into(&input);
    }
}

/// The console's owner, standing for a broker started from a console: run
/// by `a_confined_helper_cannot_type_into_its_console` in a console of its
/// own, it starts the typist confined in every role and says how many key
/// events reached its console's input. Run by that test only.
#[test]
fn console_owner() {
    if std::env::var_os(CONSOLE_OWNER).is_none() {
        return;
    }
    let input = console_input().expect("the owner's own console");
    keys_waiting(&input);
    // The control: the console takes typed key events at all.
    type_into(&input);
    println!("CONTROL {}", keys_waiting(&input));
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("windows::tests::console_typist")];
    for role in ROLES {
        let mut environment = environment(role);
        environment.push((OsString::from(CONSOLE_TYPIST), OsString::from("1")));
        let spawn = Spawn { program: &program, arguments: &arguments, role, environment: &environment, local_network: false, writable: None };
        match self::spawn(&spawn) {
            Ok(mut process) => {
                let _ = process.wait();
                println!("TYPED {role:?} {}", keys_waiting(&input));
            }
            Err(error) => println!("NOT STARTED {role:?}: {error}"),
        }
    }
}

/// Finding F-22 (decision 100 item 1): a confined helper of a broker
/// started from a console cannot type into that console, whose shell would
/// read it once the browser ends. The owner runs in a console of its own,
/// so the test does not depend on how it was started. Before, every helper
/// shared the broker's console, and in the desktop session (a console of
/// `conhost`) each role's helper wrote its key events into it.
#[test]
fn a_confined_helper_cannot_type_into_its_console() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "windows::tests::console_owner", "--nocapture", "--test-threads", "1"])
        .env(CONSOLE_OWNER, "1")
        .creation_flags(CREATE_NEW_CONSOLE)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.contains(&format!("CONTROL {}", key_events().len())), "the owner's console takes typed key events: {said}");
    for role in ROLES {
        assert!(said.contains(&format!("TYPED {role:?} 0\n")) || said.contains(&format!("TYPED {role:?} 0\r\n")), "{role:?} typed into its console: {said}");
    }
}
