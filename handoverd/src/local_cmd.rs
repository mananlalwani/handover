//! Bounded local command execution for desktop helpers.
//!
//! Presentation, volume, remote input, and clipboard actions shell out
//! to user-session tools (`xdotool`, `wpctl`, `wtype`, `wl-copy`,
//! `wl-paste`). A wedged tool must never stall daemon event
//! processing, and a spewing tool must never balloon memory: every
//! wait carries a timeout that kills the child, and every captured
//! output carries a byte cap.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use handover_core::{
    ClipboardFile, ClipboardText, PresentationCommand, RemoteInputAction, RemoteInputCommand,
    VolumeCommand,
};

/// Upper bound for one desktop tool invocation. These tools answer in
/// milliseconds when healthy; anything slower is wedged.
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// A desktop side effect queued off the event path. Backend callbacks
/// must never wait on subprocesses: even bounded waits stall event
/// processing when a helper misbehaves, so effects run here instead.
pub(crate) enum Effect {
    Presentation(PresentationCommand),
    Volume(VolumeCommand),
    RemoteInput(RemoteInputCommand),
    ClipboardText(ClipboardText),
    ClipboardFile(ClipboardFile),
}

/// Maximum queued effects. Overflow drops the newest with a warning;
/// pointer-move effects coalesce into the queued one instead.
const MAX_EFFECTS: usize = 64;

struct EffectQueue {
    queue: Mutex<VecDeque<Effect>>,
    ready: Condvar,
}

fn effects() -> &'static EffectQueue {
    static QUEUE: OnceLock<EffectQueue> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let queue = EffectQueue {
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
        };
        std::thread::Builder::new()
            .name("handover-effects".into())
            .spawn(worker)
            .expect("effect worker spawns");
        queue
    })
}

/// Queue a desktop effect without waiting. Never blocks the caller.
pub(crate) fn submit(effect: Effect) {
    let effects = effects();
    let mut queue = effects
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Effect::RemoteInput(command) = &effect {
        if command.action == RemoteInputAction::Move
            && let Some(Effect::RemoteInput(previous)) = queue.back_mut()
            && previous.action == RemoteInputAction::Move
        {
            // Coalesce only with the adjacent trailing move, and
            // sum the deltas: moves are relative, so replacing
            // would drop motion. Saturation keeps hostile input
            // inside xdotool's integer range.
            previous.delta_x = previous.delta_x.saturating_add(command.delta_x);
            previous.delta_y = previous.delta_y.saturating_add(command.delta_y);
            effects.ready.notify_one();
            return;
        }
    }
    if queue.len() >= MAX_EFFECTS {
        tracing::warn!("desktop effect queue full; dropping newest");
        // A dropped clipboard file never reaches the worker that
        // owns temp cleanup: delete it here instead of leaking it.
        if let Effect::ClipboardFile(file) = &effect {
            let _ = std::fs::remove_file(&file.path);
        }
        return;
    }
    queue.push_back(effect);
    effects.ready.notify_one();
}

fn worker() {
    let effects = effects();
    loop {
        let effect = {
            let mut queue = effects
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if let Some(effect) = queue.pop_front() {
                    break effect;
                }
                queue = effects
                    .ready
                    .wait(queue)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        match effect {
            Effect::Presentation(command) => crate::presentation::execute(&command),
            Effect::Volume(command) => crate::volume::execute(&command),
            Effect::RemoteInput(command) => crate::remote_input::execute(&command),
            Effect::ClipboardText(text) => crate::clipboard::apply(&text),
            Effect::ClipboardFile(file) => {
                crate::clipboard::apply_file(&file.path, &file.mime);
                let _ = std::fs::remove_file(&file.path);
            }
        }
    }
}

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
