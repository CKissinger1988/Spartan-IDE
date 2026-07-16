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
//! **Beyond the Phase 0 skeleton.** The wrapping/fallthrough seam is verified
//! end-to-end; on top of it there are real devserver-specific methods --
//! `devserver_ping` (liveness/identity), `model_status` (the unified model
//! surface: the configured Leo provider's real capabilities + a live health
//! probe, via `spartan_backend::model_status_json`), a real LiteLLM proxy
//! lifecycle (`litellm_proxy_start`/`_stop`/`_status`, backed by
//! `litellm_proxy`), and a real Hugging Face -> Ollama model downloader
//! (`hf_list_models`/`hf_pull_model`, backed by `hf_downloader`) -- plus the
//! static-file server (`/__spartan/session` token handoff for the `web/`
//! client). Every method named in this crate's own original design is now
//! real; no devserver-specific method remains a stub.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use spartan_backend::ws_transport::{self, WsSecurity};
use spartan_backend::{handle_request, BackendState, Event, Request, Response};

pub mod hf_downloader;
pub mod litellm_proxy;
mod subprocess;

pub mod static_serve;

/// The devserver-specific liveness/identity method. A real, useful check
/// (it reports the running service, its version, and a real uptime) that is
/// genuinely *not* part of `spartan-backend`'s own method set -- so it
/// doubles as the Phase 0 proof that a devserver method is reached while
/// every other method still falls through to the backend.
pub const DEVSERVER_PING: &str = "devserver_ping";

/// Aggregated model-status method (Track A): reports the real, currently
/// configured Leo provider's identity + capabilities + a live health probe.
/// This is the "unified model-management surface" Track A exists to provide,
/// answered by `spartan_backend::model_status_json`, which uses the same
/// `build_leo_provider` every real Leo call already goes through, so the
/// status can never disagree with what a task would actually run.
pub const MODEL_STATUS: &str = "model_status";

/// Starts a real local LiteLLM proxy (`litellm --port <port> [--config
/// <config_path>]`). Async, ack-then-event, matching `devcontainer_up`'s own
/// shape: an immediate `{"status": "starting"}` while the real, possibly
/// slow (loading model routes, etc.) spawn+health-check runs on its own
/// thread, streaming real subprocess output as `litellm_progress` events
/// and finishing with either `litellm_ready` or `litellm_failed`.
pub const LITELLM_PROXY_START: &str = "litellm_proxy_start";
/// Stops the currently-running proxy (a real, honest `not_running` result,
/// never an error, if none is running -- matches `devcontainer_down`'s own
/// "stopping something already gone is fine" precedent).
pub const LITELLM_PROXY_STOP: &str = "litellm_proxy_stop";
/// Reports the real current proxy status (`running` with its real port/pid,
/// or `not_running`) -- also self-heals a stale handle whose process has
/// since exited on its own, so a crashed proxy doesn't linger as a false
/// "running" forever.
pub const LITELLM_PROXY_STATUS: &str = "litellm_proxy_status";

/// Lists the real, curated set of Hugging Face -> Ollama models this
/// devserver knows how to pull (id/display name/repo/tag/description) --
/// synchronous, no subprocess involved.
pub const HF_LIST_MODELS: &str = "hf_list_models";
/// Triggers a real `ollama pull hf.co/<repo>:<tag>`, either for a curated
/// model id (`{"model_id": "..."}`) or a real user-defined custom model
/// download link (`{"hf_repo": "<org>/<name-or-URL>", "tag": "Q4_K_M"}`) --
/// the latter is the real "user defined model download links" mechanism,
/// going through the identical validation, subprocess, and event plumbing
/// as a curated pull. Async, ack-then-event, the same shape
/// `litellm_proxy_start` already uses: an immediate `{"status":
/// "starting"}` while the real, possibly multi-minute download runs on its
/// own thread, streaming Ollama's own real pull-progress output as
/// `hf_pull_progress` events (each carrying a real, stable `model_id` --
/// the curated id, or `<repo>:<tag>` for a custom pull -- so multiple
/// concurrent pulls stay distinguishable) and finishing with
/// `hf_pull_ready`/`hf_pull_failed`.
pub const HF_PULL_MODEL: &str = "hf_pull_model";

const LITELLM_HEALTH_TIMEOUT: Duration = Duration::from_secs(60);

/// Devserver-specific state, held *alongside* -- never inside -- the shared
/// `BackendState`. Holds a real construction instant (so `devserver_ping`
/// can report a genuine uptime, proving captured devserver state actually
/// threads through the dispatcher rather than being a stateless placeholder)
/// and the real, at-most-one LiteLLM proxy child process this devserver has
/// spawned, if any. Later increments grow this with the real model registry
/// and static-server config.
pub struct DevServerState {
    started_at: Instant,
    litellm: Mutex<Option<litellm_proxy::ProxyProcess>>,
}

impl DevServerState {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            litellm: Mutex::new(None),
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

/// Starts a real LiteLLM proxy in the background: an immediate
/// `{"status": "starting"}` ack, then a spawned thread runs the real,
/// possibly-slow spawn+health-check, forwarding real subprocess stdout/
/// stderr lines as `litellm_progress` events and finishing with
/// `litellm_ready`/`litellm_failed` -- the exact same "ack now, event
/// later" shape `spartan_backend::devcontainer_up` already established.
fn litellm_proxy_start(
    devserver: Arc<DevServerState>,
    out_tx: Sender<String>,
    port: u16,
    config_path: Option<String>,
) -> Result<serde_json::Value, String> {
    {
        let mut guard = devserver.litellm.lock().unwrap();
        if let Some(process) = guard.as_mut() {
            if process.is_running() {
                return Err(format!(
                    "a LiteLLM proxy is already running on port {} (pid {})",
                    process.port,
                    process.pid()
                ));
            }
            // A stale handle whose process already exited on its own --
            // clear it so this fresh spawn can take its place.
            *guard = None;
        }
    }

    if !litellm_proxy::is_litellm_available() {
        return Err(
            "`litellm` isn't on $PATH -- install it with `pip install 'litellm[proxy]'`"
                .to_string(),
        );
    }

    thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let forward_out_tx = out_tx.clone();
        thread::spawn(move || {
            for line in line_rx {
                let event = Event {
                    event: "litellm_progress".to_string(),
                    data: serde_json::json!({ "line": line }),
                };
                if let Ok(l) = serde_json::to_string(&event) {
                    let _ = forward_out_tx.send(l);
                }
            }
        });

        let event = match litellm_proxy::spawn(port, config_path.as_deref(), line_tx) {
            Ok(mut process) => match litellm_proxy::wait_for_health(
                &mut process,
                litellm_proxy::DEFAULT_HEALTH_PATH,
                LITELLM_HEALTH_TIMEOUT,
            ) {
                Ok(()) => {
                    let pid = process.pid();
                    *devserver.litellm.lock().unwrap() = Some(process);
                    Event {
                        event: "litellm_ready".to_string(),
                        data: serde_json::json!({ "port": port, "pid": pid }),
                    }
                }
                Err(e) => {
                    let _ = process.stop();
                    Event {
                        event: "litellm_failed".to_string(),
                        data: serde_json::json!({ "error": e.to_string() }),
                    }
                }
            },
            Err(e) => Event {
                event: "litellm_failed".to_string(),
                data: serde_json::json!({ "error": e.to_string() }),
            },
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "starting" }))
}

/// Stops the real currently-running proxy, if any. Stopping when nothing is
/// running is a real, honest `not_running` result, not an error -- matches
/// `spartan_backend::devcontainer_down`'s own precedent that "stop what's
/// already gone" is a harmless no-op, not a failure.
fn litellm_proxy_stop(devserver: &DevServerState) -> Result<serde_json::Value, String> {
    let process = devserver.litellm.lock().unwrap().take();
    match process {
        Some(process) => {
            let port = process.port;
            process.stop().map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "status": "stopped", "port": port }))
        }
        None => Ok(serde_json::json!({ "status": "not_running" })),
    }
}

/// Reports the real current proxy status, self-healing a stale handle
/// whose process has since exited on its own (a real crash) rather than
/// reporting a false "running" forever.
fn litellm_proxy_status(devserver: &DevServerState) -> serde_json::Value {
    let mut guard = devserver.litellm.lock().unwrap();
    // A match guard binds `process` immutably, but `is_running` needs
    // `&mut self` -- checked separately instead, so the mutable borrow is
    // real and the pattern match only branches on its already-computed
    // result.
    let running = guard.as_mut().map(|process| process.is_running());
    match running {
        Some(true) => {
            let process = guard.as_ref().expect("just confirmed Some above");
            serde_json::json!({ "status": "running", "port": process.port, "pid": process.pid() })
        }
        Some(false) => {
            *guard = None;
            serde_json::json!({ "status": "not_running" })
        }
        None => serde_json::json!({ "status": "not_running" }),
    }
}

/// Real, synchronous listing of the curated HF -> Ollama models.
fn hf_list_models_json() -> serde_json::Value {
    let models: Vec<serde_json::Value> = hf_downloader::CURATED_MODELS
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "hf_repo": m.hf_repo,
                "tag": m.tag,
                "description": m.description,
            })
        })
        .collect();
    serde_json::json!({ "models": models })
}

/// Resolves the real `(event_id, pull_target)` pair for either an
/// `hf_pull_model` call path -- a curated `model_id` lookup, or a
/// user-defined custom `hf_repo`+`tag` pair (the real "user defined model
/// download links" mechanism, validated via
/// `hf_downloader::custom_pull_target` before ever reaching a subprocess).
/// `model_id` wins if both are somehow present, matching this crate's own
/// "first matching real param wins" convention elsewhere (e.g.
/// `litellm_proxy_start`'s port/config_path handling).
fn resolve_hf_pull_target(
    model_id: Option<String>,
    hf_repo: Option<String>,
    tag: Option<String>,
) -> Result<(String, String), String> {
    match (model_id, hf_repo, tag) {
        (Some(model_id), _, _) => {
            let model = hf_downloader::find_model(&model_id)
                .ok_or_else(|| format!("unknown curated model id: {model_id:?}"))?;
            Ok((model.id.to_string(), hf_downloader::pull_target(model)))
        }
        (None, Some(hf_repo), Some(tag)) => {
            let normalized = hf_downloader::normalize_hf_repo_input(&hf_repo);
            let target = hf_downloader::custom_pull_target(&hf_repo, &tag)?;
            Ok((format!("{normalized}:{}", tag.trim()), target))
        }
        _ => Err(
            "hf_pull_model requires either a string `model_id`, or both a string `hf_repo` and \
             string `tag`"
                .to_string(),
        ),
    }
}

/// Starts a real HF -> Ollama pull in the background: an immediate
/// `{"status": "starting"}` ack, then a spawned thread runs the real,
/// possibly multi-minute `ollama pull`, forwarding real subprocess output
/// as `hf_pull_progress` events and finishing with `hf_pull_ready`/
/// `hf_pull_failed` -- the same "ack now, event later" shape
/// `litellm_proxy_start` already established. Accepts either a curated
/// `model_id` or a user-defined custom `hf_repo`+`tag` pair, resolved by
/// `resolve_hf_pull_target` above -- from this point on, both paths are
/// identical: same validation-already-done target string, same subprocess
/// spawn, same event shapes.
fn hf_pull_model(
    out_tx: Sender<String>,
    model_id: Option<String>,
    hf_repo: Option<String>,
    tag: Option<String>,
) -> Result<serde_json::Value, String> {
    let (event_id, target) = resolve_hf_pull_target(model_id, hf_repo, tag)?;

    if !hf_downloader::is_ollama_available() {
        return Err("`ollama` isn't on $PATH -- install it from https://ollama.com".to_string());
    }

    let ack_target = target.clone();
    thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let forward_out_tx = out_tx.clone();
        let forward_model_id = event_id.clone();
        thread::spawn(move || {
            for line in line_rx {
                let event = Event {
                    event: "hf_pull_progress".to_string(),
                    data: serde_json::json!({ "model_id": forward_model_id, "line": line }),
                };
                if let Ok(l) = serde_json::to_string(&event) {
                    let _ = forward_out_tx.send(l);
                }
            }
        });

        let event = match hf_downloader::spawn_pull_target(&target, line_tx) {
            Ok(mut child) => match child.wait() {
                Ok(status) if status.success() => Event {
                    event: "hf_pull_ready".to_string(),
                    data: serde_json::json!({ "model_id": event_id }),
                },
                Ok(status) => Event {
                    event: "hf_pull_failed".to_string(),
                    data: serde_json::json!({
                        "model_id": event_id,
                        "error": format!("ollama pull exited with {status}"),
                    }),
                },
                Err(e) => Event {
                    event: "hf_pull_failed".to_string(),
                    data: serde_json::json!({ "model_id": event_id, "error": e.to_string() }),
                },
            },
            Err(e) => Event {
                event: "hf_pull_failed".to_string(),
                data: serde_json::json!({ "model_id": event_id, "error": e.to_string() }),
            },
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = out_tx.send(line);
        }
    });

    Ok(serde_json::json!({ "status": "starting", "target": ack_target }))
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
        } else if req.method == MODEL_STATUS {
            Response {
                id: req.id,
                result: Some(spartan_backend::model_status_json()),
                error: None,
            }
        } else if req.method == LITELLM_PROXY_START {
            let port = req
                .params
                .get("port")
                .and_then(|v| v.as_u64())
                .map(|p| p as u16);
            let config_path = req
                .params
                .get("config_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let result = match port {
                Some(port) => {
                    litellm_proxy_start(Arc::clone(&devserver), out_tx, port, config_path)
                }
                None => Err("litellm_proxy_start requires a numeric `port` param".to_string()),
            };
            match result {
                Ok(value) => Response {
                    id: req.id,
                    result: Some(value),
                    error: None,
                },
                Err(message) => Response {
                    id: req.id,
                    result: None,
                    error: Some(message),
                },
            }
        } else if req.method == LITELLM_PROXY_STOP {
            match litellm_proxy_stop(&devserver) {
                Ok(value) => Response {
                    id: req.id,
                    result: Some(value),
                    error: None,
                },
                Err(message) => Response {
                    id: req.id,
                    result: None,
                    error: Some(message),
                },
            }
        } else if req.method == LITELLM_PROXY_STATUS {
            Response {
                id: req.id,
                result: Some(litellm_proxy_status(&devserver)),
                error: None,
            }
        } else if req.method == HF_LIST_MODELS {
            Response {
                id: req.id,
                result: Some(hf_list_models_json()),
                error: None,
            }
        } else if req.method == HF_PULL_MODEL {
            let model_id = req
                .params
                .get("model_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let hf_repo = req
                .params
                .get("hf_repo")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let tag = req
                .params
                .get("tag")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let result = hf_pull_model(out_tx, model_id, hf_repo, tag);
            match result {
                Ok(value) => Response {
                    id: req.id,
                    result: Some(value),
                    error: None,
                },
                Err(message) => Response {
                    id: req.id,
                    result: None,
                    error: Some(message),
                },
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

    /// `model_status` is a real devserver method (not a backend one) that
    /// reports the configured Leo provider's real capabilities + live health.
    #[test]
    fn model_status_reports_the_configured_provider() {
        let backend = Arc::new(Mutex::new(BackendState::new()));
        let dispatch = make_dispatcher(Arc::new(DevServerState::new()));
        let (tx, _rx) = mpsc::channel();

        let resp = dispatch(
            &backend,
            Request {
                id: 9,
                method: MODEL_STATUS.to_string(),
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

    #[test]
    fn resolve_hf_pull_target_looks_up_a_real_curated_model() {
        let model = hf_downloader::CURATED_MODELS[0];
        let (event_id, target) =
            resolve_hf_pull_target(Some(model.id.to_string()), None, None).unwrap();
        assert_eq!(event_id, model.id);
        assert_eq!(target, hf_downloader::pull_target(&model));
    }

    #[test]
    fn resolve_hf_pull_target_rejects_an_unknown_curated_id() {
        let err = resolve_hf_pull_target(Some("not-a-real-id".to_string()), None, None)
            .expect_err("unknown model_id must be a real error");
        assert!(err.contains("not-a-real-id"));
    }

    #[test]
    fn resolve_hf_pull_target_accepts_a_real_user_defined_custom_repo_and_tag() {
        let (event_id, target) = resolve_hf_pull_target(
            None,
            Some("https://huggingface.co/bartowski/Foo-GGUF".to_string()),
            Some("Q4_K_M".to_string()),
        )
        .unwrap();
        assert_eq!(event_id, "bartowski/Foo-GGUF:Q4_K_M");
        assert_eq!(target, "hf.co/bartowski/Foo-GGUF:Q4_K_M");
    }

    #[test]
    fn resolve_hf_pull_target_rejects_a_malformed_custom_repo() {
        let err = resolve_hf_pull_target(
            None,
            Some("not-a-real-repo-shape".to_string()),
            Some("Q4_K_M".to_string()),
        )
        .expect_err("malformed custom hf_repo must be a real error");
        assert!(err.contains("<org>/<name>"));
    }

    #[test]
    fn resolve_hf_pull_target_rejects_missing_params() {
        assert!(resolve_hf_pull_target(None, None, None).is_err());
        assert!(resolve_hf_pull_target(None, Some("org/repo".to_string()), None).is_err());
        assert!(resolve_hf_pull_target(None, None, Some("Q4_K_M".to_string())).is_err());
    }

    /// End-to-end through the dispatcher: a real user-defined custom link
    /// (no `model_id`) reaches `hf_pull_model` and either starts a real
    /// pull (if `ollama` happens to be installed here) or fails with the
    /// real, honest "`ollama` isn't on $PATH" message -- never a param
    /// error, proving the custom-link path is genuinely wired through
    /// `make_dispatcher`, not just unit-tested in isolation.
    #[test]
    fn hf_pull_model_dispatch_accepts_a_real_custom_link_end_to_end() {
        let backend = Arc::new(Mutex::new(BackendState::new()));
        let dispatch = make_dispatcher(Arc::new(DevServerState::new()));
        let (tx, _rx) = mpsc::channel();

        let resp = dispatch(
            &backend,
            Request {
                id: 11,
                method: HF_PULL_MODEL.to_string(),
                params: serde_json::json!({
                    "hf_repo": "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF",
                    "tag": "Q4_K_M",
                }),
            },
            tx,
        );

        assert_eq!(resp.id, 11);
        // Either a real "starting" ack (ollama present) or a real, specific
        // "ollama not on PATH" error -- never a params/validation error,
        // which would indicate the custom-link path isn't reaching
        // `resolve_hf_pull_target` correctly.
        match (&resp.result, &resp.error) {
            (Some(value), None) => assert_eq!(value["status"], "starting"),
            (None, Some(message)) => assert!(
                message.contains("ollama"),
                "expected an ollama-availability error, got: {message}"
            ),
            other => panic!("expected exactly one of result/error, got {other:?}"),
        }
    }
}
