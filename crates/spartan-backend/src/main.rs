//! Real stdio transport for `spartan-backend`'s protocol (`lib.rs`) --
//! one real newline-delimited JSON request per line in, one real
//! newline-delimited JSON response per line out. Deliberately minimal:
//! no framing headers (unlike LSP's `Content-Length:` convention this
//! workspace's own `lsp.rs` already speaks as a *client*) since JSON
//! itself already escapes any real newline inside a string value, so a
//! multi-line file's content never breaks the one-line-per-message
//! contract. Malformed input on a given line is reported as a real
//! error response keyed to id `0` rather than crashing the whole
//! process, since one bad line from the Electron side shouldn't kill
//! every other open document's session.

use std::io::{self, BufRead, Write};

use spartan_backend::{handle_request, BackendState, Request, Response};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut state = BackendState::new();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => handle_request(&mut state, req),
            Err(e) => Response {
                id: 0,
                result: None,
                error: Some(format!("malformed request: {e}")),
            },
        };
        let Ok(serialized) = serde_json::to_string(&response) else {
            continue;
        };
        if writeln!(stdout, "{serialized}").is_err() {
            break;
        }
        let _ = stdout.flush();
    }
}
