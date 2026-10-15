//! A stand-in for the neoteletext engine, used to build and test T42
//! Browser while the real engine's changes (`architecture.md` section 5.1)
//! are made. It reproduces only the shapes T42 Browser depends on:
//!
//! - the framed pipe protocol of `neoteletext-app/src/pipe.rs`: a kind
//!   byte, a 32-bit big-endian length and the payload, bounded and parsed
//!   strictly ([`pipe`]);
//! - a decoder's frame: 8-bit palette indices at the native raster of
//!   40 columns by 25 rows of 12 by 20 dots, with at most 32 colours
//!   ([`page`]);
//! - a decoder process that renders the pages it is shown ([`decoder`]);
//! - an indexed Portable Network Graphics writer in place of
//!   `neoteletext-image` ([`png`]).
//!
//! The pages it draws are synthetic. They carry no teletext glyphs.

pub mod decoder;
pub mod page;
pub mod pipe;
pub mod png;
