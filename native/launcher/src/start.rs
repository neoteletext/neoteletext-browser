//! Starting the broker of the version chosen, and hearing whether it is
//! healthy (`installation-and-updates.md` section 3.2). The broker writes
//! one line, `healthy`, on its standard output when the trusted core has
//! found the version healthy; a trial is committed then. A trial that
//! says nothing within [`HEALTH_DEADLINE`](crate::HEALTH_DEADLINE) is
//! ended with every process it started.

use std::ffi::OsString;
use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

use update::layout::{HEALTHY_LINE, LAUNCHED_VARIABLE};
use update::platform::MARKED_PROGRAM;

use crate::{Chosen, Ended, Launcher, launcher_release};

/// The longest line the broker's output is read in.
const MAX_LINE: u64 = 64;

/// Starts `chosen` with `arguments` and waits for it to end.
///
/// # Errors
/// When the broker cannot be started.
pub fn start(launcher: &Launcher, chosen: Chosen, arguments: &[OsString]) -> Result<Ended, String> {
    let version = chosen.version();
    let program = launcher.layout.programs(version).join(launcher.platform.program_file(MARKED_PROGRAM));
    let mut command = Command::new(&program);
    command.args(arguments).env(LAUNCHED_VARIABLE, "1").stdin(Stdio::null()).stdout(Stdio::piped());
    #[cfg(unix)]
    {
        // Its own process group, so a trial that hangs is ended whole.
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| format!("cannot start {}: {error}", program.display()))?;
    let output = child.stdout.take().ok_or("the broker has no output")?;
    let (said, heard) = mpsc::channel();
    std::thread::spawn(move || hear(output, &said));
    let healthy = match chosen {
        Chosen::Trial(version) => {
            match heard.recv_timeout(launcher.health_deadline) {
                Ok(()) => {
                    launcher.commit(version)?;
                    true
                }
                // Its output closed: it is ending of itself.
                Err(mpsc::RecvTimeoutError::Disconnected) => false,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    eprintln!("neoteletext: version {version} was not found healthy in time and is ended");
                    end_whole(&mut child);
                    false
                }
            }
        }
        Chosen::Run(_) => {
            // A committed version: its health ends a launcher release's
            // keeping of the old launcher (section 7).
            let healthy = heard.recv().is_ok();
            if healthy {
                launcher_release::forget_old_launcher(launcher);
            }
            healthy
        }
    };
    let status = child.wait().map_err(|error| format!("cannot wait for the broker: {error}"))?;
    Ok(Ended { healthy, success: status.success(), code: status.code() })
}

/// Reads the broker's output until it closes, and says each time it
/// writes the line that means it is healthy. Lines are read as bytes, at
/// most [`MAX_LINE`] at a time, and one counts only when the read before
/// it ended a line: a longer line, read in pieces, is never taken for the
/// line its last piece looks like (finding F-47), and a line that is not
/// text is passed over rather than ending the hearing.
fn hear(output: impl std::io::Read, said: &mpsc::Sender<()>) {
    let mut reader = BufReader::new(output);
    let mut line = Vec::new();
    let mut at_start = true;
    loop {
        line.clear();
        match std::io::Read::take(&mut reader, MAX_LINE).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let whole = at_start && line.ends_with(b"\n");
                at_start = line.ends_with(b"\n");
                if whole && line == HEALTHY_LINE.as_bytes() {
                    let _ = said.send(());
                }
            }
        }
    }
}

/// Ends the broker and every process it started: its process group on
/// macOS and Linux; on Windows each helper's job ends with the broker,
/// which holds it (`sandboxing.md` section 3).
fn end_whole(child: &mut Child) {
    #[cfg(unix)]
    {
        if let Some(group) = rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap_or(0)) {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `hear` says of `output`.
    fn heard(output: &[u8]) -> usize {
        let (said, heard) = mpsc::channel();
        hear(output, &said);
        drop(said);
        heard.iter().count()
    }

    /// Finding F-47: a line is heard only whole. `healthy` after other
    /// bytes on the same line, however many, is not the line `healthy`;
    /// nor does a line that is not text stop the hearing.
    #[test]
    fn a_line_is_heard_only_whole() {
        assert_eq!(heard(HEALTHY_LINE.as_bytes()), 1);
        let filler = usize::try_from(MAX_LINE).unwrap_or(0);
        for before in [1, filler.saturating_sub(1), filler, filler.saturating_mul(3)] {
            let mut output = vec![b'x'; before];
            output.extend_from_slice(HEALTHY_LINE.as_bytes());
            assert_eq!(heard(&output), 0, "{before} bytes, then healthy, on one line");
        }
        let mut output = b"\xff\xfe not text\n".to_vec();
        output.extend_from_slice(HEALTHY_LINE.as_bytes());
        assert_eq!(heard(&output), 1, "after a line that is not text");
    }
}
