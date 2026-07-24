//! Shared subprocess-spawn-and-stream-output helper. Spawning a real child
//! process and forwarding its real stdout+stderr lines to a channel is
//! identical logic between `litellm_proxy` and `hf_downloader` -- it lives
//! once here instead of copied twice.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

/// Spawns `program` with `args`, streaming its real stdout+stderr lines to
/// `progress_tx` on their own reader threads (a piped `Child`'s stdout must
/// be drained by someone or the child can block once its OS pipe buffer
/// fills -- two threads, since stdout and stderr are two independent
/// pipes). Stdin is inherited from this process, matching every existing
/// caller's own real needs (`litellm`/`ollama pull` never read from stdin).
pub(crate) fn spawn_streaming(
    program: &str,
    args: &[String],
    progress_tx: Sender<String>,
) -> std::io::Result<Child> {
    spawn_streaming_with_stdin(program, args, Stdio::inherit(), progress_tx)
}

/// The same real spawn-and-stream mechanics as `spawn_streaming`, with an
/// explicit, caller-chosen stdin -- added for `lmstudio_downloader`'s own
/// real need: `lms get` can fall back to an interactive multi-result picker
/// on an ambiguous query, and this process has no real interactive terminal
/// to answer it, so that caller passes `Stdio::null()` to turn a would-be
/// indefinite hang into an immediate real EOF `lms` itself must handle.
pub(crate) fn spawn_streaming_with_stdin(
    program: &str,
    args: &[String],
    stdin: Stdio,
    progress_tx: Sender<String>,
) -> std::io::Result<Child> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.stdin(stdin);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn()?;

    if let Some(stdout) = child.stdout.take() {
        let tx = progress_tx.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = progress_tx.send(line);
            }
        });
    }

    Ok(child)
}

/// Blocks until `child` exits on its own (`Ok(Some(status))`) or `cancel`
/// is set from another thread (real, live-checked every `poll_interval`,
/// not a one-shot check at the start) -- in which case the child is really
/// killed and reaped before returning `Ok(None)`, so a cancelled download
/// never leaves an orphaned process still holding real network/disk I/O
/// open behind it. The one real building block task #268's own model-
/// download cancellation needs: everywhere this crate previously called
/// a plain, uninterruptible `child.wait()` on a download's own subprocess
/// (`hf_pull_model`, `lmstudio_pull_model`) now calls this instead.
pub(crate) fn wait_with_cancellation(
    child: &mut Child,
    cancel: &AtomicBool,
    poll_interval: Duration,
) -> std::io::Result<Option<ExitStatus>> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        thread::sleep(poll_interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    /// Real, always-on: exercises the actual spawn+stream mechanics against
    /// a real subprocess (`python3`, always present in this project's CI,
    /// matching the established `cat`/`http.server`-as-stand-in precedent).
    #[test]
    fn spawn_streaming_forwards_real_stdout_lines() {
        let (tx, rx) = mpsc::channel();
        let mut child = spawn_streaming(
            "python3",
            &[
                "-c".to_string(),
                "print('hello-from-subprocess')".to_string(),
            ],
            tx,
        )
        .expect("python3 must be on $PATH in this project's CI");

        let status = child.wait().expect("real process must exit");
        assert!(status.success());

        let mut lines = Vec::new();
        while let Ok(line) = rx.recv_timeout(std::time::Duration::from_secs(2)) {
            lines.push(line);
        }
        assert!(
            lines.iter().any(|l| l.contains("hello-from-subprocess")),
            "expected the real stdout line to be forwarded, got: {lines:?}"
        );
    }

    /// An unspawnable program (doesn't exist) must return a real `Err`, not
    /// panic or silently succeed.
    #[test]
    fn spawn_streaming_reports_a_real_error_for_a_missing_program() {
        let (tx, _rx) = mpsc::channel();
        let result = spawn_streaming("definitely-not-a-real-binary-xyz", &[], tx);
        assert!(result.is_err());
    }

    /// A real process that exits normally (`true`) must be reported via
    /// `Some(status)`, exactly like a plain `child.wait()` would -- the
    /// cancellation flag being present and unset must change nothing about
    /// the ordinary, non-cancelled path.
    #[test]
    fn wait_with_cancellation_reports_a_real_normal_exit() {
        let (tx, _rx) = mpsc::channel();
        let mut child = spawn_streaming("true", &[], tx).expect("`true` must be on $PATH");
        let cancel = AtomicBool::new(false);
        let status = wait_with_cancellation(&mut child, &cancel, Duration::from_millis(20))
            .expect("real wait must succeed");
        assert!(status.is_some(), "a normal exit must report Some(status)");
        assert!(status.unwrap().success());
    }

    /// A real, genuinely long-running process (`sleep 30`), cancelled from a
    /// second thread shortly after it starts, must be killed and reaped
    /// promptly -- confirming both that `Ok(None)` is returned (not left
    /// hanging until the real 30s sleep would finish on its own) and that
    /// the underlying OS process is truly gone afterward, not just that
    /// this function returned.
    #[test]
    fn wait_with_cancellation_kills_a_real_long_running_process() {
        let (tx, _rx) = mpsc::channel();
        let mut child =
            spawn_streaming("sleep", &["30".to_string()], tx).expect("`sleep` must be on $PATH");
        let pid = child.id();
        let cancel = Arc::new(AtomicBool::new(false));

        let cancel_clone = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            cancel_clone.store(true, Ordering::SeqCst);
        });

        let started = std::time::Instant::now();
        let status = wait_with_cancellation(&mut child, &cancel, Duration::from_millis(30))
            .expect("real wait must succeed even when cancelled");
        let elapsed = started.elapsed();

        assert!(
            status.is_none(),
            "a cancelled wait must report None, not a real exit status"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "must return promptly once cancelled, not wait out the real 30s sleep: took {elapsed:?}"
        );

        // Real confirmation the OS process is actually gone, not just that
        // this function returned -- sending signal 0 to a real-but-dead pid
        // fails with ESRCH; `kill -0` is the standard portable liveness
        // check with no side effect on a process that's still alive.
        let still_alive = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(
            !still_alive,
            "the real killed process must no longer exist (pid {pid})"
        );
    }
}
