//! The web client's tests, against servers in this process: a web client
//! runs on a thread as it runs in its process, given messages on a pipe
//! and read from one, as the trusted core would.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::arithmetic_side_effects, reason = "a test")]

mod support;

mod cookies;
mod fetching;
mod protections;
mod standards;
mod streams;
