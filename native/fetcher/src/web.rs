//! The fetcher's general web client (neoteletext-common's DEC-7260 items 6
//! to 8, adopted by DEC-7262): the network half of the Fetch standard for
//! scripts' requests, beside the channel fetcher and sharing nothing with
//! it but the process's kind.
//!
//! - `client`: the process's loop, the trusted core's messages in and the
//!   events out, a tab's requests counted;
//! - `fetch`: the Fetch standard's main fetch, HTTP fetch, redirects,
//!   preflights and response filtering;
//! - `exchange`, `pool`, `connect`: one request and its answer over hyper,
//!   the connections kept, and their opening under the address rules;
//! - `cookies`: the jars, one for each partition;
//! - `policy`: the channel as the unit of isolation (DEC-7264);
//! - `cors`, `referrer`, `integrity`, `rules`: the standards' rules;
//! - `coding`, `sse`: content codings and event streams;
//! - `local`: this computer and the local network, as DEC-7315 allows
//!   them (DEC-7315);
//! - `limits`: every bound.

mod client;
mod coding;
mod connect;
mod cookies;
mod cors;
mod fetch;
pub mod fuzzing;
mod integrity;
mod limits;
mod local;
mod policy;
mod pool;
mod referrer;
mod rules;
mod sse;

pub use client::run;

#[cfg(test)]
mod tests;
