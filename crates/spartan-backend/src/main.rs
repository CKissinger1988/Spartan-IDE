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
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use spartan_backend::{handle_request, BackendState, Request, Response};

fn main() {
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

    let state = Arc::new(Mutex::new(BackendState::new()));
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
