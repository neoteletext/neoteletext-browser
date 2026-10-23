//! Starting the broker of the version chosen, and hearing whether it has
//! started and whether it is healthy (`installation-and-updates.md`
//! section 3.2, decision 93). The broker writes `started` on its standard
//! output before it starts the trusted core, and `healthy` once the core
//! has found the version healthy; a trial is committed then. A trial gets
//! [`START_DEADLINE`](update::layout::START_DEADLINE) to say it started,
//! for a first run's checks by the system, and then
//! [`HEALTH_DEADLINE`](update::layout::HEALTH_DEADLINE) from that moment
//! to be healthy; one that misses either is ended with every process it
//! started. `healthy` says `started` too, for a broker that says only that.

use std::ffi::OsString;
use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use update::layout::{HEALTHY_LINE, LAUNCHED_VARIABLE, STARTED_LINE};
use update::platform::MARKED_PROGRAM;

use crate::{Chosen, Deadline, Ended, Launcher, launcher_release};

/// The longest line the broker's output is read in.
const MAX_LINE: u64 = 64;
/// How often a trial's broker is looked at to see whether it has ended,
/// while its deadline runs.
const EXIT_POLL: Duration = Duration::from_millis(50);

/// A line of the broker's that means something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heard {
    /// It is about to start the trusted core.
    Started,
    /// The trusted core found the version healthy.
    Healthy,
}

/// Starts `chosen` with `arguments` and waits for it to end. A trial's
/// start is recorded in the state as it is heard, and its end after.
///
/// # Errors
/// When the broker cannot be started, or a trial's state cannot be
/// written.
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
    let mut ended = Ended { started: false, healthy: false, deadline: None, success: false, code: None };
    match chosen {
        Chosen::Trial(version) => {
            let start_by = Instant::now().checked_add(launcher.start_deadline);
            match wait_for(&heard, &mut child, start_by, |_| true) {
                Waited::Heard(line) => {
                    ended.started = true;
                    ended.healthy = line == Heard::Healthy;
                    launcher.trial_started(version)?;
                }
                Waited::Ended => {}
                Waited::Deadline => {
                    eprintln!("neoteletext: version {version} did not start in time and is ended");
                    end_whole(&mut child);
                    ended.deadline = Some(Deadline::Start);
                }
            }
            if ended.started && !ended.healthy {
                let healthy_by = Instant::now().checked_add(launcher.health_deadline);
                match wait_for(&heard, &mut child, healthy_by, |line| line == Heard::Healthy) {
                    Waited::Heard(_) => ended.healthy = true,
                    Waited::Ended => {}
                    Waited::Deadline => {
                        eprintln!("neoteletext: version {version} was not found healthy in time and is ended");
                        end_whole(&mut child);
                        ended.deadline = Some(Deadline::Health);
                    }
                }
            }
            if ended.healthy {
                launcher.commit(version)?;
            }
        }
        Chosen::Run(_) => {
            // A committed version: its health ends a launcher release's
            // keeping of the old launcher (section 7).
            ended.healthy = matches!(wait_for(&heard, &mut child, None, |line| line == Heard::Healthy), Waited::Heard(_));
            if ended.healthy {
                launcher_release::forget_old_launcher(launcher);
            }
        }
    }
    let status = wait_whole(&mut child).map_err(|error| format!("cannot wait for the broker: {error}"))?;
    ended.success = status.success();
    ended.code = status.code();
    if let Chosen::Trial(version) = chosen
        && ended.started
        && !ended.healthy
    {
        launcher.trial_ended(version, &ended)?;
    }
    Ok(ended)
}

/// How a wait ended.
enum Waited {
    /// The line wanted was heard.
    Heard(Heard),
    /// The broker ended, or closed its output and then ended, first.
    Ended,
    /// The deadline passed first.
    Deadline,
}

/// Waits until `by` for a line `wanted` accepts, or for the broker to
/// end. Its end is looked at every [`EXIT_POLL`], not only when its output
/// closes: a helper it started holds the output open after it ends, and a
/// broker may close it and go on running (finding F-34).
fn wait_for(heard: &mpsc::Receiver<Heard>, child: &mut Child, by: Option<Instant>, wanted: impl Fn(Heard) -> bool) -> Waited {
    let mut closed = false;
    loop {
        let left = by.map_or(Duration::MAX, |by| by.saturating_duration_since(Instant::now()));
        let line = if closed {
            std::thread::sleep(left.min(EXIT_POLL));
            None
        } else {
            match heard.recv_timeout(left.min(EXIT_POLL)) {
                Ok(line) => Some(line),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    closed = true;
                    None
                }
            }
        };
        match line {
            Some(line) if wanted(line) => return Waited::Heard(line),
            None if child.try_wait().ok().flatten().is_some() => return Waited::Ended,
            None if by.is_some_and(|by| Instant::now() >= by) => return Waited::Deadline,
            // A line not wanted, or nothing yet.
            _ => {}
        }
    }
}

/// Reads the broker's output until it closes, and says each line that
/// means something. Lines are read as bytes, at most [`MAX_LINE`] at a
/// time, and one counts only when the read before it ended a line: a
/// longer line, read in pieces, is never taken for the line its last piece
/// looks like (finding F-47), and a line that is not text is passed over
/// rather than ending the hearing.
fn hear(output: impl std::io::Read, said: &mpsc::Sender<Heard>) {
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
                let meant = if !whole {
                    None
                } else if line == STARTED_LINE.as_bytes() {
                    Some(Heard::Started)
                } else if line == HEALTHY_LINE.as_bytes() {
                    Some(Heard::Healthy)
                } else {
                    None
                };
                if let Some(meant) = meant {
                    let _ = said.send(meant);
                }
            }
        }
    }
}

/// Waits for the broker to end, then ends whatever is left in its process
/// group (finding F-34): a broker that crashed leaves its helpers running
/// with nothing to end them. The group is ended while the broker is ended
/// but not yet reaped, so its number, the group's, cannot meanwhile have
/// been given to another process. On Windows each helper's job ends with
/// the broker, which holds it (`sandboxing.md` section 3).
fn wait_whole(child: &mut Child) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(unix)]
    if let Some(group) = rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap_or(0)) {
        use rustix::process::{WaitId, WaitIdOptions, waitid};
        while let Err(error) = waitid(WaitId::Pid(group), WaitIdOptions::EXITED | WaitIdOptions::NOWAIT) {
            if error != rustix::io::Errno::INTR {
                break;
            }
        }
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
    child.wait()
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

    /// How many times `hear` says `healthy` of `output`.
    fn heard(output: &[u8]) -> usize {
        let (said, heard) = mpsc::channel();
        hear(output, &said);
        drop(said);
        heard.iter().filter(|line| *line == Heard::Healthy).count()
    }

    /// Decision 93: `started` is heard as itself, before `healthy`.
    #[test]
    fn started_is_heard_before_healthy() {
        let (said, heard) = mpsc::channel();
        hear([STARTED_LINE, HEALTHY_LINE].concat().as_bytes(), &said);
        drop(said);
        assert_eq!(heard.iter().collect::<Vec<_>>(), [Heard::Started, Heard::Healthy]);
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
