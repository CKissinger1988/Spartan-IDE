//! Real LiteLLM proxy process lifecycle: spawn, health-check, stop.
//!
//! LiteLLM's own proxy (`litellm --port <p> [--config <path>]`) is a real,
//! separate Python process this crate doesn't vendor or reimplement --
//! this module just launches it, streams its stdout/stderr as real progress
//! lines, and polls its own real HTTP health endpoint until it's actually
//! ready to serve requests, mirroring `spartan_devcontainer::docker`'s
//! "tokio contained in a thread" discipline even though this module needs
//! no tokio at all (a child process + a sync HTTP poll, nothing async).
//!
//! Restart-on-crash is a real, deliberately deferred follow-up, named here
//! rather than silently absorbed -- `try_wait`/`is_running` exist precisely
//! so a caller *can* detect a crash, but this module itself never restarts
//! anything automatically.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

/// LiteLLM's own documented liveness endpoint. Not live-verified against a
/// real `litellm` binary in this environment (none is installed here -- see
/// `tests/litellm_integration.rs`, which self-skips honestly rather than
/// fabricating a result); the *mechanics* of spawn/stream/poll/stop are
/// verified for real against a real subprocess via a stand-in program
/// (`python3 -m http.server`, always present) in this module's own tests.
pub const DEFAULT_HEALTH_PATH: &str = "/health/liveliness";

#[derive(Debug)]
pub struct LiteLlmProxyError(pub String);

impl std::fmt::Display for LiteLlmProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for LiteLlmProxyError {}

/// A real, running (or possibly just-exited) proxy child process.
pub struct ProxyProcess {
    child: Child,
    pub port: u16,
}

impl ProxyProcess {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Non-blocking: `Some(exit_code)` if the process has already exited on
    /// its own (e.g. a real startup crash), `None` if it's still running.
    pub fn try_wait(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            _ => None,
        }
    }

    pub fn is_running(&mut self) -> bool {
        self.try_wait().is_none()
    }

    /// Real stop: SIGKILL-equivalent (`Child::kill`) then wait for the real
    /// exit, tolerating "already exited" rather than treating it as an
    /// error -- a caller stopping a proxy that already crashed on its own
    /// should see a clean stop, not a confusing second failure.
    pub fn stop(mut self) -> Result<(), LiteLlmProxyError> {
        if self.try_wait().is_none() {
            let _ = self.child.kill();
        }
        self.child
            .wait()
            .map(|_| ())
            .map_err(|e| LiteLlmProxyError(format!("failed waiting for proxy to exit: {e}")))
    }
}

/// A real, cheap check -- `litellm --version`, discarding output -- whether
/// the `litellm` CLI is actually on `$PATH`. Matches
/// `spartan_devcontainer::docker::is_docker_available`'s own shape exactly.
pub fn is_litellm_available() -> bool {
    Command::new("litellm")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Spawns `program` with `args`, streaming its real stdout+stderr lines to
/// `progress_tx` on their own reader threads (a piped `Child`'s stdout must
/// be drained by someone or the child can block once its OS pipe buffer
/// fills -- two threads, since stdout and stderr are two independent
/// pipes). Generalized over `program`/`args` -- not just `litellm` -- purely
/// so this module's own tests can exercise the real spawn/stream mechanics
/// against an always-available stand-in process without needing a real
/// `litellm` install.
fn spawn_child(
    program: &str,
    args: &[String],
    port: u16,
    progress_tx: Sender<String>,
) -> Result<ProxyProcess, LiteLlmProxyError> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        LiteLlmProxyError(format!(
            "failed to spawn `{program}`: {e} (is it installed and on $PATH?)"
        ))
    })?;

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

    Ok(ProxyProcess { child, port })
}

/// Spawns a real `litellm --port <port> [--config <config_path>]` proxy
/// process. `config_path`, when given, must already exist -- this module
/// generates no config of its own, matching `devcontainer_up`'s own
/// "the caller supplies the real project state, this function only drives
/// it" discipline.
pub fn spawn(
    port: u16,
    config_path: Option<&str>,
    progress_tx: Sender<String>,
) -> Result<ProxyProcess, LiteLlmProxyError> {
    let mut args = vec!["--port".to_string(), port.to_string()];
    if let Some(cfg) = config_path {
        args.push("--config".to_string());
        args.push(cfg.to_string());
    }
    spawn_child("litellm", &args, port, progress_tx)
}

/// Polls `http://127.0.0.1:<port><health_path>` until it responds with a
/// non-server-error status, the process exits on its own (a real startup
/// crash, failed fast rather than waited out), or `timeout` elapses.
pub fn wait_for_health(
    process: &mut ProxyProcess,
    health_path: &str,
    timeout: Duration,
) -> Result<(), LiteLlmProxyError> {
    let deadline = Instant::now() + timeout;
    let url = format!("http://127.0.0.1:{}{}", process.port, health_path);
    loop {
        if let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(2)).call() {
            if resp.status() < 500 {
                return Ok(());
            }
        }
        if let Some(code) = process.try_wait() {
            return Err(LiteLlmProxyError(format!(
                "litellm exited with code {code} before becoming healthy"
            )));
        }
        if Instant::now() >= deadline {
            return Err(LiteLlmProxyError(format!(
                "proxy did not become healthy within {timeout:?}"
            )));
        }
        thread::sleep(Duration::from_millis(300));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A real, environment-dependent boolean -- no assertion on its value,
    /// only that the check itself runs without panicking, matching
    /// `spartan_devcontainer::docker`'s own precedent for this exact kind
    /// of external-tool-presence check.
    #[test]
    fn is_litellm_available_runs_without_panicking() {
        let _ = is_litellm_available();
    }

    /// A real, unused port to spawn the stand-in server on -- bind-then-
    /// drop-immediately to get one the OS considers free right now.
    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .expect("real bind")
            .local_addr()
            .unwrap()
            .port()
    }

    /// Real, always-on (no `litellm` install needed): exercises the actual
    /// spawn -> stream-progress -> poll-health -> stop mechanics against a
    /// real subprocess, using `python3 -m http.server` (always present in
    /// this project's CI, matching this repo's own established `cat`-as-
    /// stand-in precedent, §75.80) in place of a real `litellm` proxy.
    /// `http.server`'s `GET /` returns a real 200 directory listing, which
    /// `wait_for_health` treats identically to a real litellm health 200.
    #[test]
    fn spawn_stream_health_and_stop_work_against_a_real_subprocess() {
        let port = free_port();
        let (tx, rx) = mpsc::channel();

        let mut process = spawn_child(
            "python3",
            &[
                "-m".to_string(),
                "http.server".to_string(),
                port.to_string(),
            ],
            port,
            tx,
        )
        .expect("python3 must be on $PATH in this project's CI");

        wait_for_health(&mut process, "/", Duration::from_secs(10))
            .expect("a real http.server must become healthy");

        assert!(process.is_running(), "the real process must still be up");
        assert!(process.pid() > 0, "a real spawned process has a real pid");

        process
            .stop()
            .expect("a real running process must stop cleanly");

        // At least one real progress line (http.server logs each request,
        // including wait_for_health's own real polling GETs) must have
        // been streamed through the channel -- confirms the reader threads
        // genuinely forwarded real subprocess output, not just that the
        // process ran.
        let mut saw_a_line = false;
        while let Ok(_line) = rx.try_recv() {
            saw_a_line = true;
        }
        assert!(
            saw_a_line,
            "expected at least one real streamed progress line from the subprocess"
        );
    }

    /// A process that exits immediately (never opens the health port) must
    /// fail fast via the try_wait check, not hang until the full timeout.
    #[test]
    fn wait_for_health_fails_fast_when_the_process_exits_immediately() {
        let port = free_port();
        let (tx, _rx) = mpsc::channel();

        let mut process = spawn_child("true", &[], port, tx).expect("`true` must be on $PATH");

        let started = Instant::now();
        let result = wait_for_health(&mut process, "/", Duration::from_secs(30));
        let elapsed = started.elapsed();

        assert!(
            result.is_err(),
            "a process that never opens the port must fail health"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "must fail fast on real process exit, not wait out the full 30s timeout: took {elapsed:?}"
        );
    }
}
