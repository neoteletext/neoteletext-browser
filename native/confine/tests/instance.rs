//! The single-instance channel, in a test program of its own.
//!
//! Keep this program to this one test. On macOS a new process holds a copy of
//! every open descriptor of its parent until its image is loaded, close-on-exec
//! or not, and `posix_spawn` makes that copy in the kernel too. A sibling test
//! starting any process while this one closes its listener therefore keeps the
//! listener answering for those milliseconds, and the test sees a socket that
//! was closed accept a connection. In the crate's unit test program that was
//! 139 of 200 runs on 1 November 2026.

#![cfg(unix)]
#![allow(clippy::unwrap_used, reason = "a test")]

use confine::broker::{Opening, read_opening, write_opening};
use confine::instance::{Instance, connect};

#[test]
fn a_second_start_reaches_the_first_and_nothing_else_answers_after() {
    let folder = std::env::temp_dir().join(format!("neoteletext-instance-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let place = folder.join("instance");
    assert!(connect(&place).is_err(), "none running");
    let first = Instance::bind(&place).unwrap();
    let opening = Opening::Outside { links: vec!["neottx://https://a.example/".into()], files: Vec::new() };
    write_opening(&mut connect(&place).unwrap(), &opening).unwrap();
    assert_eq!(read_opening(&mut first.next().unwrap()).unwrap(), opening);
    drop(first);
    assert!(connect(&place).is_err(), "a socket left by a browser that ended answers nothing");
    let _again = Instance::bind(&place).unwrap();
    assert!(connect(&place).is_ok(), "and its place can be taken");
    std::fs::remove_dir_all(&folder).unwrap();
}
