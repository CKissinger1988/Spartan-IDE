//! Spartan Devserver (Track A) -- a local, localhost-only service that
//! **wraps** `spartan-backend` rather than reimplementing it.
//!
//! Its whole design is one seam: a *wrapping dispatcher* that handles a
//! small, growing set of devserver-specific methods and **falls through to
//! `spartan_backend::handle_request` for everything else** (file ops, Leo,
//! pty, git, settings, dev containers -- every method that crate already
//! implements). It shares the exact same `Arc<Mutex<BackendState>>` a plain
//! `spartan-backend` process would, so nothing about the existing backend's
//! behavior changes; the devserver only adds.
//!
//! It drives that dispatcher over `spartan-backend`'s own real WebSocket
//! transport (`ws_transport`), which was made generic over `<S, D>` in the
//! preceding commit precisely so this crate could reuse its
//! security-critical, single-sourced handshake (per-process random token +
//! Origin allowlist) verbatim instead of forking it.
//!
//! **This module is the Phase 0 skeleton.** It establishes and verifies the
//! wrapping/fallthrough seam end-to-end with exactly one real
//! devserver-specific method (`devserver_ping`). The real model-management
//! methods (`model_status`, `hf_pull_model`, `litellm_proxy_*`) and the
//! static-file server (`/__spartan/session` token handoff for the `web/`
//! client) land on top of this same seam in later increments -- they are
//! deliberately not present yet, not stubbed with fake behavior.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use spartan_backend::ws_transport::{self, WsSecurity};
use spartan_backend::{handle_request, BackendState, Request, Response};

/// The devserver-specific liveness/identity method. A real, useful check
/// (it reports the running service, its version, and a real uptime) that is
/// genuinely *not* part of `spartan-backend`'s own method set -- so it
/// doubles as the Phase 0 proof that a devserver method is reached while
/// every other method still falls through to the backend.
pub const DEVSERVER_PING: &str = "devserver_ping";

/// Devserver-specific state, held *alongside* -- never inside -- the shared
/// `BackendState`. Later increments grow this with the real model registry,
/// LiteLLM proxy handle(s), and static-server config; for the Phase 0
/// skeleton it holds only a real construction instant so `devserver_ping`
/// can report a genuine uptime, proving captured devserver state actually
/// threads through the dispatcher rather than being a stateless placeholder.
pub struct DevServerState {
    started_at: Instant,
}

impl DevServerState {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }

    /// Real milliseconds since this state was constructed.
    pub fn uptime_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }
}

impl Default for DevServerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds the wrapping dispatcher: a real `Fn` of the exact shape
/// `ws_transport::serve`/`run_websocket_server` expect, capturing the
/// devserver's own state, handling devserver-specific methods, and falling
/// through to `spartan_backend::handle_request` (which owns the shared
/// `BackendState`) for everything else. Cloneable + `Send` + `'static`
/// because it captures only an `Arc`, so the transport can hand a clone to
/// each per-connection thread.
pub fn make_dispatcher(
    devserver: Arc<DevServerState>,
) -> impl Fn(&Arc<Mutex<BackendState>>, Request, Sender<String>) -> Response + Clone + Send + 'static
{
    move |state, req, out_tx| {
        // One devserver-specific method today; the rest fall through
        // unchanged. `==` (not `match ... as_str()`) so `req` can be moved
        // into `handle_request` in the fallthrough arm without a borrow
        // conflict -- and reads cleanly for a single special case.
        if req.method == DEVSERVER_PING {
            Response {
                id: req.id,
                result: Some(serde_json::json!({
                    "service": "spartan-devserver",
                    "version": env!("CARGO_PKG_VERSION"),
                    "uptime_ms": devserver.uptime_ms(),
                })),
                error: None,
            }
        } else {
            handle_request(state, req, out_tx)
        }
    }
}

/// Real, blocking entry point: run the devserver's WebSocket transport on
/// `addr`, driving the wrapping dispatcher over the shared `BackendState`.
/// A thin, honest composition of `make_dispatcher` + the reused, generic
/// `ws_transport::run_websocket_server` -- no new transport code.
pub fn run_websocket_server(
    backend: Arc<Mutex<BackendState>>,
    addr: &str,
    security: WsSecurity,
    devserver: Arc<DevServerState>,
) -> std::io::Result<()> {
    let dispatch = make_dispatcher(devserver);
    ws_transport::run_websocket_server(backend, addr, security, dispatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use tungstenite::client::connect;
    use tungstenite::http::Uri;
    use tungstenite::Message;

    /// The dispatcher, called directly (no network), returns the devserver's
    /// own identity for `devserver_ping`.
    #[test]
    fn devserver_ping_returns_real_service_identity() {
        let backend = Arc::new(Mutex::new(BackendState::new()));
        let dispatch = make_dispatcher(Arc::new(DevServerState::new()));
        let (tx, _rx) = mpsc::channel();

        let resp = dispatch(
            &backend,
            Request {
                id: 7,
                method: DEVSERVER_PING.to_string(),
                params: serde_json::json!({}),
            },
            tx,
        );

        assert_eq!(resp.id, 7);
        assert!(resp.error.is_none(), "ping must succeed: {resp:?}");
        let result = resp.result.expect("ping returns a real result");
        assert_eq!(result["service"], "spartan-devserver");
        assert!(
            result["uptime_ms"].is_u64(),
            "ping reports a real numeric uptime: {result:?}"
        );
    }

    /// An unknown method (a real `spartan-backend` method) falls through to
    /// `handle_request` and is served by the shared `BackendState` -- the
    /// core "wrap, don't reimplement" guarantee.
    #[test]
    fn an_existing_backend_method_falls_through_to_the_real_backend() {
        let backend = Arc::new(Mutex::new(BackendState::new()));
        let dispatch = make_dispatcher(Arc::new(DevServerState::new()));
        let (tx, _rx) = mpsc::channel();

        let tmp = std::env::temp_dir();
        let resp = dispatch(
            &backend,
            Request {
                id: 8,
                method: "list_dir".to_string(),
                params: serde_json::json!({ "path": tmp.to_string_lossy() }),
            },
            tx,
        );

        assert_eq!(resp.id, 8);
        assert!(
            resp.error.is_none(),
            "list_dir must fall through to the real backend and succeed: {resp:?}"
        );
    }

    /// End-to-end over the real WebSocket transport: proves the generic
    /// `ws_transport::serve` genuinely drives a *foreign* dispatcher (this
    /// crate's, not `handle_request`), with both a devserver-specific method
    /// and a fell-through backend method answered over one real connection.
    #[test]
    fn the_real_transport_drives_the_devserver_dispatcher_end_to_end() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("real bind");
        let port = listener.local_addr().unwrap().port();
        let backend = Arc::new(Mutex::new(BackendState::new()));
        let security = WsSecurity {
            allowed_origins: vec![],
            token: "devserver-e2e-token".to_string(),
        };
        let dispatch = make_dispatcher(Arc::new(DevServerState::new()));
        thread::spawn(move || {
            let _ = ws_transport::serve(backend, listener, security, dispatch);
        });
        thread::sleep(Duration::from_millis(20));

        let uri: Uri = format!("ws://127.0.0.1:{port}/?token=devserver-e2e-token")
            .parse()
            .unwrap();
        let (mut ws, _resp) = connect(uri).expect("a correctly-tokened connection must succeed");

        // 1. A devserver-specific method.
        ws.send(Message::Text(
            serde_json::to_string(&Request {
                id: 1,
                method: DEVSERVER_PING.to_string(),
                params: serde_json::json!({}),
            })
            .unwrap()
            .into(),
        ))
        .unwrap();
        let Message::Text(text) = ws.read().unwrap() else {
            panic!("expected a real text frame");
        };
        let resp: Response = serde_json::from_str(&text).unwrap();
        assert_eq!(resp.id, 1);
        assert_eq!(resp.result.unwrap()["service"], "spartan-devserver");

        // 2. A fell-through backend method, over the same real connection.
        ws.send(Message::Text(
            serde_json::to_string(&Request {
                id: 2,
                method: "list_dir".to_string(),
                params: serde_json::json!({ "path": std::env::temp_dir().to_string_lossy() }),
            })
            .unwrap()
            .into(),
        ))
        .unwrap();
        let Message::Text(text) = ws.read().unwrap() else {
            panic!("expected a real text frame");
        };
        let resp: Response = serde_json::from_str(&text).unwrap();
        assert_eq!(resp.id, 2);
        assert!(
            resp.error.is_none(),
            "a fell-through list_dir must succeed over the real transport: {resp:?}"
        );
    }
}
