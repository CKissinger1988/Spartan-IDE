//! Real stdio transport for `spartan-backend`'s protocol (`lib.rs`) --
//! one real newline-delimited JSON request per line in; one real
//! newline-delimited JSON line per real `Response` *or* real `Event`
//! out. A single, dedicated writer thread owns real stdout exclusively
//! (fed by an `mpsc::Sender<String>` every other thread holds a clone
//! of), so a real background Leo thread pushing an unprompted `Event`
//! can never interleave mid-line with the main request loop writing a
//! `Response` -- the real reason a shared channel replaces a direct
//! `writeln!` from multiple call sites. Malformed input on a given line
//! is reported as a real error response keyed to id `0` rather than
//! crashing the whole process, since one bad line from the Electron
//! side shouldn't kill every other open document's session.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use spartan_backend::{
    crash_dir, handle_request,
    ws_transport::{self, WsSecurity},
    BackendState, Request, Response,
};

/// Real, optional opt-in for the WebSocket transport (task #80, §75.88) --
/// `--ws-port:<port>` starts a real WebSocket listener on `127.0.0.1:<port>`
/// *alongside* stdio, not instead of it. Absent by default: every existing
/// Electron launch (no such arg) behaves exactly as before.
fn parse_ws_port(args: &[String]) -> Option<u16> {
    args.iter()
        .find_map(|a| a.strip_prefix("--ws-port:"))
        .and_then(|p| p.parse().ok())
}

/// Real, repeatable `--ws-allowed-origin:<origin>` args -- the real
/// browser-context half of `ws_transport`'s own defense-in-depth posture
/// (see that module's top-level doc comment). No default allowlist exists
/// -- a launch with the WebSocket transport enabled but no allowed
/// origins configured still works for non-browser (no-`Origin`-header)
/// clients via the token check alone, but rejects every real browser
/// connection until at least one real origin is explicitly named.
fn parse_ws_allowed_origins(args: &[String]) -> Vec<String> {
    args.iter()
        .filter_map(|a| a.strip_prefix("--ws-allowed-origin:"))
        .map(str::to_string)
        .collect()
}

/// Real, per-process random token written to `~/.spartan/ws-token`
/// (`0600` on Unix) so a legitimate local process can read it. *How* a
/// real browser-based web client actually obtains this value is a real,
/// separate, not-yet-decided product question -- see `ws_transport`'s own
/// doc comment for why that's deliberately not guessed at here.
fn write_ws_token_file(token: &str) -> io::Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(home).join(".spartan");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("ws-token");
    std::fs::write(&path, token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

fn main() {
    // Real §75.82 fix for a real, previously-unnoticed gap: `spartan-
    // editor-core`'s own `main.rs` has installed a real crash hook since
    // §75.32, but this crate -- the actual IPC service process driving
    // the primary Electron shell -- never did, so a real panic in this
    // process (which would kill the whole backend Electron depends on)
    // left nothing on disk to diagnose it with. Installed first, before
    // anything else in `main` can possibly panic, matching the original
    // hook's own ordering rationale exactly. Shares the same real
    // `~/.spartan/crashes` directory both shells write to on one machine.
    spartan_crash::install_hook(crash_dir());

    let state = Arc::new(Mutex::new(BackendState::new()));

    // Real, opt-in WebSocket transport (task #80, §75.88) -- shares the
    // exact same `state` the stdio loop below uses, so a browser tab
    // connecting here sees the same open files/Leo state a simultaneously
    // running Electron client would. Spawned on its own thread; a real
    // bind failure (e.g. the port already in use) is reported to stderr
    // rather than aborting the whole process, since stdio still needs to
    // work even if the WebSocket half can't start.
    let args: Vec<String> = std::env::args().collect();
    if let Some(port) = parse_ws_port(&args) {
        let token = ws_transport::generate_token();
        match write_ws_token_file(&token) {
            Ok(path) => eprintln!("spartan-backend: WebSocket transport token written to {path:?}"),
            Err(e) => eprintln!("spartan-backend: failed to write WebSocket token file: {e}"),
        }
        let security = WsSecurity {
            allowed_origins: parse_ws_allowed_origins(&args),
            token,
        };
        let ws_state = Arc::clone(&state);
        thread::spawn(move || {
            let addr = format!("127.0.0.1:{port}");
            if let Err(e) = ws_transport::run_websocket_server(ws_state, &addr, security) {
                eprintln!("spartan-backend: WebSocket transport failed to start on {addr}: {e}");
            }
        });
    }

    let (out_tx, out_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut stdout = io::stdout();
        for line in out_rx {
            if writeln!(stdout, "{line}").is_err() {
                break;
            }
            let _ = stdout.flush();
        }
    });

    let stdin = io::stdin();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => handle_request(&state, req, out_tx.clone()),
            Err(e) => Response {
                id: 0,
                result: None,
                error: Some(format!("malformed request: {e}")),
            },
        };
        let Ok(serialized) = serde_json::to_string(&response) else {
            continue;
        };
        if out_tx.send(serialized).is_err() {
            break;
        }
    }
}
