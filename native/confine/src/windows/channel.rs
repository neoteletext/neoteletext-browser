//! The broker's channel on Windows: two pipes, the core's standard input
//! and output. The broker duplicates each helper's pipe handles into the
//! core's process and sends their values after the reply (decision 56).

use std::fs::File;
use std::io::{self, Read, Write};

use super::handles::{duplicate_into, file_from_handle_value};
use crate::broker::{HEADER_BYTES, KIND_SPAWNED, MAX_REPLY, frame, header};
use crate::{Input, Output, Process};

/// One end of the broker's channel.
pub(crate) struct Channel {
    reader: File,
    writer: File,
}

impl Channel {
    /// The broker's end: the core's output, read, and its input, written.
    pub(crate) fn new(reader: File, writer: File) -> Self {
        Self { reader, writer }
    }

    /// The core's end: its standard input and output.
    pub(crate) fn from_stdin() -> io::Result<Self> {
        use std::os::windows::io::AsHandle as _;
        let reader = File::from(io::stdin().as_handle().try_clone_to_owned()?);
        let writer = File::from(io::stdout().as_handle().try_clone_to_owned()?);
        Ok(Self { reader, writer })
    }

    /// One frame, its length within `max`.
    pub(crate) fn read_frame(&mut self, max: usize) -> io::Result<(u8, Vec<u8>)> {
        let mut bytes = [0_u8; HEADER_BYTES];
        let reader = &mut self.reader;
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
        let writer = &mut self.writer;
        writer.write_all(bytes)?;
        writer.flush()
    }

    /// A reply's kind and payload, and the pipes whose handle values
    /// followed its fields.
    pub(crate) fn receive_reply(&mut self) -> io::Result<(u8, Vec<u8>, Vec<File>)> {
        let (kind, mut payload) = self.read_frame(MAX_REPLY)?;
        let mut pipes = Vec::new();
        if kind == KIND_SPAWNED {
            // Two handle values the broker duplicated into this process
            // follow the reply's own fields.
            let tail = payload.len().saturating_sub(16);
            let values: Vec<u8> = payload.split_off(tail);
            for chunk in values.as_chunks::<8>().0 {
                pipes.push(file_from_handle_value(u64::from_be_bytes(*chunk))?);
            }
        }
        Ok((kind, payload, pipes))
    }

    /// Sends the reply `bytes` of a started helper, its two pipes
    /// duplicated into the core's process.
    pub(crate) fn send_spawned(&mut self, bytes: &[u8], stdin: &Input, stdout: &Output, core: &Process) -> io::Result<()> {
        // The handles are duplicated into the core and their values sent
        // after the reply's fields; the frame's length counts them.
        let into = core.raw_process();
        let values = [duplicate_into(stdin, into)?, duplicate_into(stdout, into)?];
        let mut payload = bytes.get(HEADER_BYTES..).unwrap_or_default().to_vec();
        for value in values {
            payload.extend_from_slice(&value.to_be_bytes());
        }
        self.write(&frame(KIND_SPAWNED, &payload)?)
    }
}
