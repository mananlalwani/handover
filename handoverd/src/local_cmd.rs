//! Bounded local command execution for desktop helpers.
//!
//! Presentation, volume, remote input, and clipboard actions shell out
//! to user-session tools (`xdotool`, `wpctl`, `wtype`, `wl-copy`,
//! `wl-paste`). A wedged tool must never stall daemon event
//! processing, and a spewing tool must never balloon memory: every
//! wait carries a timeout that kills the child, and every captured
//! output carries a byte cap.

use std::io::{self, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Upper bound for one desktop tool invocation. These tools answer in
/// milliseconds when healthy; anything slower is wedged.
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawned-command wait with a kill on expiry.
pub(crate) fn wait_timeout(mut child: Child) -> io::Result<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => return Ok(status),
            None => {
                if start.elapsed() >= COMMAND_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "desktop command timed out",
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// `command.status()` with a kill on expiry.
pub(crate) fn status_timeout(command: &mut Command) -> io::Result<ExitStatus> {
    wait_timeout(command.spawn()?)
}

/// Feed `input` to a piped-stdin child and wait with a kill on expiry.
/// Feeding runs on a side thread so a wedged sink cannot trap the
/// writer; killing the child releases it.
pub(crate) fn feed_and_wait(command: &mut Command, input: Vec<u8>) -> io::Result<ExitStatus> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("desktop command stdin unavailable"))?;
    let feeder = std::thread::spawn(move || stdin.write_all(&input));
    let status = wait_timeout(child);
    let _ = feeder.join();
    status
}

/// Feed an open file to a piped-stdin child and wait with a kill on
/// expiry. The copy runs on a side thread; killing a wedged child
/// releases it.
pub(crate) fn feed_file(command: &mut Command, mut input: std::fs::File) -> io::Result<ExitStatus> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("desktop command stdin unavailable"))?;
    let feeder = std::thread::spawn(move || {
        let _ = io::copy(&mut input, &mut stdin);
    });
    let status = wait_timeout(child);
    let _ = feeder.join();
    status
}

/// `command.output()` capped at `limit + 1` bytes with a kill on
/// expiry. Callers reject `stdout.len() > limit`. Allocation stays
/// bounded no matter how much a broken tool spews.
pub(crate) fn output_bounded(
    command: &mut Command,
    limit: usize,
) -> io::Result<std::process::Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("desktop command stdout unavailable"))?;
    let cap = (limit as u64).saturating_add(1);
    let reader = std::thread::spawn(move || {
        let mut buffered = stdout.take(cap);
        let mut output = Vec::new();
        buffered.read_to_end(&mut output).map(|_| output)
    });
    let start = Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => {
                return match reader.join() {
                    Ok(Ok(stdout)) => Ok(std::process::Output {
                        status,
                        stdout,
                        stderr: Vec::new(),
                    }),
                    _ => Err(io::Error::other("desktop command output unreadable")),
                };
            }
            None => {
                if start.elapsed() >= COMMAND_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "desktop command timed out",
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_command_returns_status() {
        let status = status_timeout(&mut Command::new("true")).expect("true runs");
        assert!(status.success());
    }

    #[test]
    fn wedged_command_is_killed() {
        let start = Instant::now();
        let mut sleeper = Command::new("sleep");
        sleeper.arg("30");
        let error = status_timeout(&mut sleeper).expect_err("sleep must time out");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(30));
    }

    #[test]
    fn output_is_capped_no_matter_the_spew() {
        let mut shower = Command::new("sh");
        shower.args(["-c", "head -c 100000 /dev/zero; exit 0"]);
        let output = output_bounded(&mut shower, 1024).expect("bounded read runs");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 1025);
    }
}
