//! Shared subprocess-spawn-and-stream-output helper. Spawning a real child
//! process and forwarding its real stdout+stderr lines to a channel is
//! identical logic between `litellm_proxy` and `hf_downloader` -- it lives
//! once here instead of copied twice.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

/// Spawns `program` with `args`, streaming its real stdout+stderr lines to
/// `progress_tx` on their own reader threads (a piped `Child`'s stdout must
/// be drained by someone or the child can block once its OS pipe buffer
/// fills -- two threads, since stdout and stderr are two independent
/// pipes).
pub(crate) fn spawn_streaming(
    program: &str,
    args: &[String],
    progress_tx: Sender<String>,
) -> std::io::Result<Child> {
    let mut cmd = Command::new(program);
    cmd.args(args);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

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
}
