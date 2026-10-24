//! The interface core of `docs/native-interface-design.md` section 4.1:
//! the browser window's state, layout, animation, hit-testing and input
//! handling,
//! written once for every target. It draws only through [`canvas::Canvas`],
//! so the same code runs over `vello_cpu` on a desktop or a phone and over
//! `embedded-graphics` on a microcontroller.
//!
//! It needs no standard library, only an allocator, and reads no clock: the
//! host passes the time in milliseconds.

#![no_std]
// No panicking constructs: the window draws what the trusted core sends
// (titles, addresses, views of the library, a phone's code), and the
// interface's own list does not reach this library (finding F-105). The
// same list is in each crate that reads input from outside its process;
// scripts/check.sh checks that they agree.
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

extern crate alloc;

pub mod browser;
pub mod canvas;
pub mod channel;
pub mod console;
pub mod controls;
pub mod cookies;
pub mod editor;
pub mod geometry;
#[cfg(feature = "interaction")]
pub mod input;
pub mod library;
#[cfg(feature = "links")]
pub mod links;
mod session;
pub mod settings;
pub mod size;
#[cfg(test)]
mod test_canvas;
pub mod text;
pub mod theme;
pub mod tween;
pub mod wall;
