//! The broker's channel on macOS and Linux: a local-domain socket, the
//! core's standard input, which carries each helper's pipes as
//! descriptors (decision 56).

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;

use super::{receive, send};
use crate::broker::{HEADER_BYTES, MAX_REPLY, header};
use crate::{Input, Output, Process};

/// One end of the broker's channel.
pub(crate) struct Channel {
    stream: UnixStream,
}

impl Channel {
    /// The broker's end, of a socket pair it made.
    pub(crate) fn new(stream: UnixStream) -> Self {
        Self { stream }
    }

    /// The core's end: its standard input, which a broker made a socket.
    pub(crate) fn from_stdin() -> io::Result<Self> {
        use std::os::fd::AsFd as _;
        let descriptor = io::stdin().as_fd().try_clone_to_owned()?;
        let stream = UnixStream::from(descriptor);
        // Standard input that is not a socket was not set up by a broker.
        stream.peer_addr().map_err(|_| io::Error::other("this program is started by the browser's broker, not directly"))?;
        Ok(Self { stream })
    }

    /// One frame, its length within `max`.
    pub(crate) fn read_frame(&mut self, max: usize) -> io::Result<(u8, Vec<u8>)> {
        let mut bytes = [0_u8; HEADER_BYTES];
        let reader = &mut self.stream;
        reader.read_exact(&mut bytes)?;
        let (kind, length) = header(bytes, max)?;
        let mut payload = Vec::with_capacity(length);
        reader.take(length as u64).read_to_end(&mut payload)?;
        if payload.len() != length {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok((kind, payload))
    }

    /// Writes `bytes` whole.
    pub(crate) fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let writer = &mut self.stream;
        writer.write_all(bytes)?;
        writer.flush()
    }

    /// A reply's kind and payload, and the pipes that came with it as
    /// descriptors.
    pub(crate) fn receive_reply(&mut self) -> io::Result<(u8, Vec<u8>, Vec<File>)> {
        let mut head = [0_u8; HEADER_BYTES];
        let descriptors = receive(&self.stream, &mut head)?;
        let (kind, length) = header(head, MAX_REPLY)?;
        let mut payload = Vec::with_capacity(length);
        (&self.stream).take(length as u64).read_to_end(&mut payload)?;
        if payload.len() != length {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok((kind, payload, descriptors.into_iter().map(File::from).collect()))
    }

    /// Sends the reply `bytes` of a started helper with its two pipes
    /// attached; the core's process is not needed here.
    pub(crate) fn send_spawned(&mut self, bytes: &[u8], stdin: &Input, stdout: &Output, _core: &Process) -> io::Result<()> {
        send(&self.stream, bytes, &[stdin.as_raw_fd(), stdout.as_raw_fd()])
        // Our copies close here; the core holds its own.
    }
}
