//! Passing open files between processes on macOS and Linux: a message on
//! a local-domain socket carries descriptors with it (`SCM_RIGHTS`,
//! `unix(7)`). The broker passes the trusted core the pipes of each helper
//! it starts this way. The standard library's ancillary-data interface is
//! not yet stable, so the two system calls are made here. And the local
//! time zone's offset, which the standard library does not offer, from
//! the C library's own `localtime_r`.

mod channel;
mod descriptors;
mod time;

pub(crate) use channel::Channel;
pub(crate) use descriptors::{receive, send};
pub(crate) use time::local_offset;
