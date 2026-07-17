//! Spartan Devserver (Track A) -- a local, localhost-only service that
//! **wraps** `spartan-backend` rather than reimplementing it.
//!
//! Its whole design is one seam: a *wrapping dispatcher* that handles a
//! small, growing set of devserver-specific methods and **falls through to
//! `spartan_backend::handle_request` for everything else** (file ops, Leo,
//! pty, git, settings, dev containers, model management -- every method
//! that crate already implements). It shares the exact same
//! `Arc<Mutex<BackendState>>` a plain `spartan-backend` process would, so
//! nothing about the existing backend's behavior changes; the devserver
//! only adds.
//!
//! It drives that dispatcher over `spartan-backend`'s own real WebSocket
//! transport (`ws_transport`), which was made generic over `<S, D>` in the
//! preceding commit precisely so this crate could reuse its
//! security-critical, single-sourced handshake (per-process random token +
//! Origin allowlist) verbatim instead of forking it.
//!
//! **What's genuinely devserver-specific**: just `devserver_ping`
//! (liveness/identity). Every other real method this crate once answered
//! directly -- `model_status`, the LiteLLM proxy lifecycle
//! (`litellm_proxy_start`/`_stop`/`_status`), and both HF-backed model
//! downloaders (`hf_list_models`/`hf_pull_model`,
//! `lmstudio_list_models`/`lmstudio_pull_model`) -- moved down into
//! `spartan-backend` itself (task #145), so `desktop/`'s Electron shell
//! (which spawns a plain `spartan-backend`, not a `spartan-devserver`) gets
//! the identical real methods `web/` already had, with zero duplicated
//! logic. This crate's own dispatcher is now, by design, close to just the
//! wrapping/fallthrough seam it always aspired to be, plus the static-file
//! server (`/__spartan/session` token handoff for the `web/` client).

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use spartan_backend::ws_transport::{self, WsSecurity};
use spartan_backend::{handle_request, BackendState, Request, Response};

pub mod static_serve;

/// The devserver-specific liveness/identity method. A real, useful check
/// (it reports the running service, its version, and a real uptime) that is
/// genuinely *not* part of `spartan-backend`'s own method set -- so it
/// doubles as the Phase 0 proof that a devserver method is reached while
/// every other method still falls through to the backend.
pub const DEVSERVER_PING: &str = "devserver_ping";

/// Devserver-specific state, held *alongside* -- never inside -- the shared
/// `BackendState`. Holds a real construction instant (so `devserver_ping`
/// can report a genuine uptime, proving captured devserver state actually
/// threads through the dispatcher rather than being a stateless
/// placeholder). Real model-management state (the LiteLLM proxy handle)
/// moved into `BackendState` itself alongside task #145's dispatch move.
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
        // Devserver-specific methods; everything else falls through
        // unchanged. `==` (not `match ... as_str()`) so `req` can be moved
        // into `handle_request` in the fallthrough arm without a borrow
        // conflict.
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

/// Real, blocking entry point for the **full** local devserver: serves the
/// `web/` client's static files and the same-origin `/__spartan/session`
/// token handoff on `<host>:<static_port>`, while running the WebSocket
/// transport on a separate ephemeral port whose live token is advertised
/// only to same-origin pages.
///
/// The wiring order is load-bearing: both real ports are learned *before*
/// the WebSocket `allowed_origins` is set to the static server's own origin,
/// so a browser tab loaded from the served app -- and only such a tab, by
/// SOP + the Origin allowlist together -- can open the WebSocket. The token
/// is freshly generated per run (never a persisted default), matching
/// `ws_transport`'s own security posture exactly.
///
/// `project_root`, when given, is advertised verbatim (already
/// caller-canonicalized -- this function does no path resolution of its
/// own) via the session endpoint, closing the gap `static_serve`'s own doc
/// comment names: a browser's `FileSystemDirectoryHandle` has no real OS
/// path to give `git_status`/`open_file`/Leo, but the devserver's own
/// launch directory does.
pub fn run(
    web_root: PathBuf,
    host: &str,
    static_port: u16,
    project_root: Option<PathBuf>,
) -> std::io::Result<()> {
    // Bind the static server (its port is user-facing) and the WS listener
    // (ephemeral) up front so both real ports are known before wiring.
    let static_server = static_serve::bind(&format!("{host}:{static_port}"))?;
    let actual_static_port = static_server
        .server_addr()
        .to_ip()
        .ok_or_else(|| std::io::Error::other("static server bound to a non-IP address"))?
        .port();
    let (ws_listener, ws_port) = static_serve::bind_ws_listener(host)?;

    let ws_token = ws_transport::generate_token();
    // Only the served app's own origin(s) may open the WebSocket. Both the
    // `127.0.0.1` and `localhost` spellings of the static port are included
    // because a browser presents whichever the user actually navigated to.
    let allowed_origins = vec![
        format!("http://{host}:{actual_static_port}"),
        format!("http://localhost:{actual_static_port}"),
        format!("http://127.0.0.1:{actual_static_port}"),
    ];
    let security = WsSecurity {
        allowed_origins,
        token: ws_token.clone(),
    };

    let backend = Arc::new(Mutex::new(BackendState::new()));
    let dispatch = make_dispatcher(Arc::new(DevServerState::new()));
    thread::spawn(move || {
        let _ = ws_transport::serve(backend, ws_listener, security, dispatch);
    });

    let project_root_display = project_root
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(none)".to_string());
    eprintln!(
        "spartan-devserver: serving {web_root:?} on http://{host}:{actual_static_port} \
         (WebSocket on 127.0.0.1:{ws_port}, token handed off via {}, project root: {project_root_display})",
        static_serve::SESSION_PATH
    );
    static_serve::serve(
        static_server,
        static_serve::StaticServeConfig {
            web_root,
            ws_port,
            ws_token,
            project_root: project_root.map(|p| p.to_string_lossy().to_string()),
        },
    )
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

    /// `model_status` is a real `spartan-backend` method (moved down from
    /// this crate in task #145) reachable here purely via fallthrough --
    /// reports the configured Leo provider's real capabilities + live
    /// health.
    #[test]
    fn model_status_reports_the_configured_provider() {
        let backend = Arc::new(Mutex::new(BackendState::new()));
        let dispatch = make_dispatcher(Arc::new(DevServerState::new()));
        let (tx, _rx) = mpsc::channel();

        let resp = dispatch(
            &backend,
            Request {
                id: 9,
                method: "model_status".to_string(),
                params: serde_json::json!({}),
            },
            tx,
        );

        assert_eq!(resp.id, 9);
        assert!(resp.error.is_none(), "model_status must succeed: {resp:?}");
        let result = resp.result.expect("model_status returns a real result");
        // Either a configured provider or an honest construction error -- both
        // are real, neither is fabricated.
        assert!(
            result["configured"].is_boolean(),
            "reports a real configured flag: {result:?}"
        );
        assert!(result["kind"].is_string(), "reports the provider kind");
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

    /// Real, direct confirmation (not just an inference from
    /// `an_existing_backend_method_falls_through_to_the_real_backend`) that
    /// the real HF/LM Studio/LiteLLM model-management methods moved down
    /// into `spartan-backend` (task #145) are genuinely reachable through
    /// this crate's own dispatcher via fallthrough -- their own full
    /// behavioral test coverage now lives in `spartan-backend`'s own test
    /// suite, this only proves the wiring here didn't regress.
    #[test]
    fn model_management_methods_fall_through_to_the_real_backend() {
        let backend = Arc::new(Mutex::new(BackendState::new()));
        let dispatch = make_dispatcher(Arc::new(DevServerState::new()));

        for method in [
            "litellm_proxy_status",
            "hf_list_models",
            "lmstudio_list_models",
        ] {
            let (tx, _rx) = mpsc::channel();
            let resp = dispatch(
                &backend,
                Request {
                    id: 20,
                    method: method.to_string(),
                    params: serde_json::json!({}),
                },
                tx,
            );
            assert!(
                resp.error.is_none(),
                "{method} must fall through and succeed: {resp:?}"
            );
        }
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
