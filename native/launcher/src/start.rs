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
//!
//! Whether the broker has ended is looked at apart from its output, which a
//! thread reads; a broker that has ended may have written lines that thread
//! has not yet read. So the launcher keeps an end of the broker's output of
//! its own, and once the broker has ended writes [`END_MARK_LINE`] into it:
//! the broker wrote everything before it, so the broker's end is taken only
//! once the mark is heard, never while what it said is still in the pipe.

use std::ffi::OsString;
use std::io::{BufRead as _, BufReader, PipeWriter, Write as _};
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
/// The line the launcher writes into the broker's output once the broker
/// has ended, after a newline that ends any line the broker left
/// unfinished.
const END_MARK_LINE: &str = "neoteletext-launcher: the broker ended\n";
/// The most bytes POSIX lets one write to a pipe and promises it whole,
/// never mixed with another process's writes (POSIX.1-2017, `write`:
/// `{PIPE_BUF}`, at least `_POSIX_PIPE_BUF`, 512): a helper still running
/// as the mark is written cannot split it.
const ATOMIC_PIPE_WRITE: usize = 512;
const _: () = assert!(END_MARK_LINE.len() < ATOMIC_PIPE_WRITE, "the newline and the mark are written whole");
const _: () = assert!((END_MARK_LINE.len() as u64) <= MAX_LINE, "the mark is read as one line");

/// A line of the broker's output that means something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heard {
    /// It is about to start the trusted core.
    Started,
    /// The trusted core found the version healthy.
    Healthy,
    /// The launcher's mark: every line before it has been heard.
    EndMark,
}

/// The broker's standard output, as the launcher hears it.
struct Output {
    /// Each line heard that means something, in order.
    heard: mpsc::Receiver<Heard>,
    /// The launcher's own end of it, until the broker has ended and the
    /// mark is written.
    own_end: Option<PipeWriter>,
    /// The hearing has ended: the output closed or could not be read, or
    /// the mark could not be written.
    closed: bool,
}

impl Output {
    /// Hears `output` on a thread of its own; `own_end` writes into it.
    fn hear(output: impl std::io::Read + Send + 'static, own_end: PipeWriter) -> Self {
        let (said, heard) = mpsc::channel();
        std::thread::spawn(move || hear(output, &said));
        Self { heard, own_end: Some(own_end), closed: false }
    }

    /// The broker has ended: the mark is written after all it wrote, and
    /// the launcher's own end closed. Written once, as one write.
    fn broker_ended(&mut self) {
        if let Some(mut own_end) = self.own_end.take() {
            let mark = [b"\n", END_MARK_LINE.as_bytes()].concat();
            if own_end.write_all(&mark).is_err() {
                // Nothing reads the output any more: what was heard is all.
                self.closed = true;
            }
        }
    }
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
    let (output, broker_end) = std::io::pipe().map_err(|error| format!("cannot make the broker's output: {error}"))?;
    let own_end = broker_end.try_clone().map_err(|error| format!("cannot make the broker's output: {error}"))?;
    let mut command = Command::new(&program);
    command.args(arguments).env(LAUNCHED_VARIABLE, "1").stdin(Stdio::null()).stdout(broker_end);
    #[cfg(unix)]
    {
        // Its own process group, so a trial that hangs is ended whole.
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let spawned = command.spawn();
    // The command holds the broker's end of the output; only the broker
    // and what it starts are to.
    drop(command);
    let mut child = spawned.map_err(|error| format!("cannot start {}: {error}", program.display()))?;
    let mut output = Output::hear(output, own_end);
    let mut ended = Ended { started: false, healthy: false, deadline: None, success: false, code: None };
    match chosen {
        Chosen::Trial(version) => {
            let start_by = Instant::now().checked_add(launcher.start_deadline);
            match wait_for(&mut output, &mut child, start_by, |_| true) {
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
                match wait_for(&mut output, &mut child, healthy_by, |line| line == Heard::Healthy) {
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
            ended.healthy = matches!(wait_for(&mut output, &mut child, None, |line| line == Heard::Healthy), Waited::Heard(_));
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
#[derive(Debug, PartialEq, Eq)]
enum Waited {
    /// The line wanted was heard.
    Heard(Heard),
    /// The broker ended first, and every line it wrote was heard.
    Ended,
    /// The deadline passed first.
    Deadline,
}

/// Waits until `by` for a line `wanted` accepts, or for the broker to
/// end. Its end is looked at every [`EXIT_POLL`], not only when its output
/// closes: a helper it started holds the output open after it ends, and a
/// broker may close it and go on running (finding F-34). Once it has ended
/// no deadline applies, and the wait goes on until the mark is heard, so
/// every line it wrote is heard first. A mark the broker wrote itself is
/// passed over while it runs; one it wrote just before it ended may be
/// taken for the launcher's, which only loses what it wrote after.
fn wait_for(output: &mut Output, child: &mut Child, by: Option<Instant>, wanted: impl Fn(Heard) -> bool) -> Waited {
    loop {
        if output.own_end.is_some() && child.try_wait().ok().flatten().is_some() {
            output.broker_ended();
        }
        let ended = output.own_end.is_none();
        if output.closed && ended {
            return Waited::Ended;
        }
        if !ended && by.is_some_and(|by| Instant::now() >= by) {
            return Waited::Deadline;
        }
        let left = by.map_or(EXIT_POLL, |by| by.saturating_duration_since(Instant::now()).min(EXIT_POLL));
        if output.closed {
            std::thread::sleep(left);
            continue;
        }
        let line = if ended { output.heard.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected) } else { output.heard.recv_timeout(left) };
        match line {
            Ok(Heard::EndMark) if ended => return Waited::Ended,
            Ok(Heard::EndMark) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(line) if wanted(line) => return Waited::Heard(line),
            // A line not wanted.
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => output.closed = true,
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
                } else if line == END_MARK_LINE.as_bytes() {
                    Some(Heard::EndMark)
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

    /// The variable that makes this test program a child that lives,
    /// for [`a_child_that_lives`].
    const CHILD: &str = "LAUNCHER_START_TEST_CHILD";
    /// How long a child that lives lives: longer than any test here waits.
    const CHILD_LIFE: Duration = Duration::from_secs(60);
    /// How late the late reader first reads: several looks at whether the
    /// broker has ended.
    const LATE: Duration = EXIT_POLL.saturating_mul(4);
    /// The longest a wait here may take before its test fails, rather than
    /// hanging when the wait never ends.
    const CEILING: Duration = Duration::from_secs(30);

    /// A reader that reads nothing before a moment, as a thread the system
    /// has not yet run does not.
    struct Late<R> {
        reader: R,
        from: Instant,
    }

    impl<R: std::io::Read> std::io::Read for Late<R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(self.from.saturating_duration_since(Instant::now()));
            self.reader.read(buffer)
        }
    }

    /// The broker's output, `written` in it, heard late, and an end of it
    /// that stays open as a helper of the broker's holds it, so that it
    /// never closes.
    fn heard_late(written: &str) -> (Output, PipeWriter) {
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(written.as_bytes()).unwrap();
        let helper = writer.try_clone().unwrap();
        let from = Instant::now().checked_add(LATE).unwrap();
        (Output::hear(Late { reader, from }, writer), helper)
    }

    /// This test program, as a child that has ended or one that lives.
    fn child(lives: bool) -> Child {
        let mut command = Command::new(std::env::current_exe().unwrap());
        if lives {
            command.args(["--exact", "start::tests::a_child_that_lives"]).env(CHILD, "1");
        } else {
            command.arg("--list");
        }
        let mut child = command.stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        if !lives {
            child.wait().unwrap();
        }
        child
    }

    /// Not a test of its own: the child that lives.
    #[test]
    fn a_child_that_lives() {
        if std::env::var_os(CHILD).is_some() {
            std::thread::sleep(CHILD_LIFE);
        }
    }

    /// The waits of `wanted` in turn, each until `by`, and how each ended;
    /// a wait that has not ended by [`CEILING`] fails the test.
    fn waits(mut output: Output, mut child: Child, by: Option<Instant>, wanted: &'static [fn(Heard) -> bool]) -> Vec<Waited> {
        let (ended, endings) = mpsc::channel();
        std::thread::spawn(move || {
            for wanted in wanted {
                let _ = ended.send(wait_for(&mut output, &mut child, by, wanted));
            }
            let _ = child.kill();
            let _ = child.wait();
        });
        (0..wanted.len()).map(|_| endings.recv_timeout(CEILING).expect("a wait ended")).collect()
    }

    /// A broker that said `started` and `healthy` and ended at once is
    /// heard to have said them, however late its output is read: its end
    /// is taken only once all it wrote is heard (two of twenty workspace
    /// runs found a healthy trial's end before its `started`, and left it
    /// counted as not started).
    #[test]
    fn every_line_written_before_the_end_is_heard() {
        let (output, _helper) = heard_late(&[STARTED_LINE, HEALTHY_LINE].concat());
        let waited = waits(output, child(false), None, &[|_| true, |line| line == Heard::Healthy, |_| true]);
        assert_eq!(waited, [Waited::Heard(Heard::Started), Waited::Heard(Heard::Healthy), Waited::Ended]);
    }

    /// The mark, written by a broker still running, neither ends the wait
    /// nor stops its deadline.
    #[test]
    fn a_mark_written_by_the_broker_is_passed_over() {
        let (output, _helper) = heard_late(&[END_MARK_LINE, STARTED_LINE].concat());
        let by = Instant::now().checked_add(LATE.saturating_mul(2)).unwrap();
        assert_eq!(waits(output, child(true), Some(by), &[|line| line == Heard::Healthy]), [Waited::Deadline]);
    }

    /// A line the broker left unfinished when it ended does not hide the
    /// mark.
    #[test]
    fn an_unfinished_line_does_not_hide_the_mark() {
        let (output, _helper) = heard_late("start");
        assert_eq!(waits(output, child(false), None, &[|_| true]), [Waited::Ended]);
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
