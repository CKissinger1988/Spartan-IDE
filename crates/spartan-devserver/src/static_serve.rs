//! Real static file server for the `web/` client, plus the same-origin
//! token handoff that closes the open question `ws_transport`'s own doc
//! comment named (§75.88): "how does a legitimate browser client obtain the
//! current WebSocket token and learn which origin to present?"
//!
//! The answer, implemented here: the devserver *serves the `web/` app's own
//! static files itself*, on `127.0.0.1:<static-port>` (never `0.0.0.0`). A
//! page loaded from that origin can then `fetch("/__spartan/session")` --
//! a **same-origin** request -- to receive `{ "wsPort", "wsToken" }`, and
//! open the WebSocket with that token from that (now allowlisted) origin.
//!
//! **Why this is safe, stated precisely, not overclaimed.** A cross-origin
//! page (a random site the user's browser visits) *can* issue the `fetch`,
//! but the browser's own Same-Origin Policy blocks it from *reading* the
//! response, because this endpoint deliberately emits **no**
//! `Access-Control-Allow-Origin` header -- so the token never reaches
//! foreign JavaScript. This is the exact property a raw WebSocket handshake
//! lacks (WS is not SOP-constrained), which is why the token had to be
//! delivered over an HTTP fetch rather than embedded in the WS URL from an
//! untrusted context. What this does **not** defend against -- named, not
//! hidden -- is another local process running as the same OS user reading
//! the served files or `~/.spartan/ws-token` directly; that is a
//! pre-existing property of any localhost service, not a new regression.
//!
//! File serving is **path-jailed** to the served root with the same
//! canonicalize-and-verify technique `spartan-leo::tool::Sandbox::resolve`
//! uses (directly mitigating the path-traversal CVE class §36.2 names): any
//! `..` escape or symlink pointing outside the root is refused.

use std::io;
use std::net::TcpListener;
use std::path::{Component, Path, PathBuf};

use tiny_http::{Header, Response, Server};

/// The one JSON endpoint that hands a same-origin page the live WebSocket
/// coordinates. Everything else is a static file request.
pub const SESSION_PATH: &str = "/__spartan/session";

/// Coordinates advertised by `SESSION_PATH`, plus the real directory of
/// static files to serve. `ws_token`/`ws_port` are the live values from the
/// WebSocket server this devserver started -- never a persisted default.
pub struct StaticServeConfig {
    pub web_root: PathBuf,
    pub ws_port: u16,
    pub ws_token: String,
}

/// Path-jail: resolve a request URL path (e.g. `/assets/index.js`, or `/`
/// -> `index.html`) to a real, existing file **inside** `root`, or `None`
/// if it escapes the jail or doesn't resolve to a regular file. Pure and
/// directly unit-testable, mirroring `spartan-leo::tool::Sandbox::resolve`.
pub fn resolve_web_path(root: &Path, url_path: &str) -> Option<PathBuf> {
    let canonical_root = root.canonicalize().ok()?;

    // Strip any query string, then all leading slashes so the remainder can
    // never be an absolute path that ignores `root`. An empty path is the
    // SPA entry point.
    let path_only = url_path.split('?').next().unwrap_or("");
    let trimmed = path_only.trim_start_matches('/');
    let rel = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };

    // Lexically normalize `.`/`..` without requiring existence yet; a `..`
    // that pops above the root is an unambiguous escape attempt.
    let joined = canonical_root.join(rel);
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }

    // Canonicalize the real resolved path (this both requires the file to
    // exist and resolves any symlink), then verify it's genuinely inside
    // the root -- catching a symlink that points outside.
    let canonical = normalized.canonicalize().ok()?;
    if !canonical.starts_with(&canonical_root) {
        return None;
    }
    if !canonical.is_file() {
        return None;
    }
    Some(canonical)
}

/// Minimal, honest content-type map -- exactly the types `web/dist`
/// actually produces (HTML/JS/CSS/WASM/JSON/assets). Unknown extensions
/// fall back to `application/octet-stream` rather than guessing.
fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .expect("a real, valid header must always build")
}

/// Bind a `tiny_http` server, returning it so the caller can read the real
/// bound port (`server.server_addr()`) before serving -- exactly what the
/// devserver's own `run` needs to set the WebSocket `allowed_origins` to
/// this server's own origin.
pub fn bind(addr: &str) -> io::Result<Server> {
    Server::http(addr).map_err(io::Error::other)
}

/// Real, blocking serve loop over an already-bound `tiny_http` server.
/// Handles `SESSION_PATH` (same-origin token handoff, deliberately with no
/// permissive CORS header) and otherwise serves path-jailed static files.
pub fn serve(server: Server, config: StaticServeConfig) -> io::Result<()> {
    for request in server.incoming_requests() {
        // `url()` includes any query string; split the path off for routing.
        let url = request.url().to_string();
        let path = url.split('?').next().unwrap_or("");

        if path == SESSION_PATH {
            let body = serde_json::json!({
                "wsPort": config.ws_port,
                "wsToken": config.ws_token,
            })
            .to_string();
            // No `Access-Control-Allow-Origin` header on purpose -- the
            // browser's SOP must block a cross-origin read of this token.
            let response = Response::from_string(body)
                .with_header(header("Content-Type", "application/json; charset=utf-8"));
            let _ = request.respond(response);
            continue;
        }

        match resolve_web_path(&config.web_root, path) {
            Some(file_path) => match std::fs::File::open(&file_path) {
                Ok(file) => {
                    let response = Response::from_file(file)
                        .with_header(header("Content-Type", content_type_for(&file_path)));
                    let _ = request.respond(response);
                }
                Err(_) => {
                    let _ =
                        request.respond(Response::from_string("not found").with_status_code(404));
                }
            },
            None => {
                let _ = request.respond(Response::from_string("not found").with_status_code(404));
            }
        }
    }
    Ok(())
}

/// Convenience for the common case: bind an ephemeral static port and read
/// it back before the caller wires origins/serves. Returns the bound server
/// and its real port.
pub fn bind_ephemeral(host: &str) -> io::Result<(Server, u16)> {
    let server = bind(&format!("{host}:0"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| io::Error::other("static server bound to a non-IP address"))?
        .port();
    Ok((server, port))
}

/// Bind an ephemeral WebSocket listener on `host` and read its real port --
/// the companion of `bind_ephemeral` for the WS half, so `run` can learn
/// both ports before wiring the Origin allowlist.
pub fn bind_ws_listener(host: &str) -> io::Result<(TcpListener, u16)> {
    let listener = TcpListener::bind(format!("{host}:0"))?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    /// Builds a real temp web root with an index.html and a nested asset.
    fn make_web_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spartan-devserver-static-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "<!doctype html><title>web</title>").unwrap();
        std::fs::write(dir.join("assets").join("app.js"), "console.log('hi')").unwrap();
        dir
    }

    #[test]
    fn resolve_web_path_maps_root_to_index_and_serves_a_real_asset() {
        let root = make_web_root();
        assert_eq!(
            resolve_web_path(&root, "/").unwrap().file_name().unwrap(),
            "index.html"
        );
        assert_eq!(
            resolve_web_path(&root, "/assets/app.js")
                .unwrap()
                .file_name()
                .unwrap(),
            "app.js"
        );
        // Query strings are stripped before resolution.
        assert!(resolve_web_path(&root, "/assets/app.js?v=123").is_some());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_web_path_refuses_traversal_and_missing_files() {
        let root = make_web_root();
        assert!(
            resolve_web_path(&root, "/../../../etc/passwd").is_none(),
            "a real `..` traversal must be refused"
        );
        assert!(
            resolve_web_path(&root, "/../..").is_none(),
            "popping above the root must be refused"
        );
        assert!(
            resolve_web_path(&root, "/does-not-exist.js").is_none(),
            "a missing file resolves to None, not an escape"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// Tiny raw-HTTP GET helper -- returns (status_line, headers_blob, body)
    /// so a test can assert on both headers and body with no HTTP-client
    /// dependency.
    fn http_get(port: u16, path: &str) -> (String, String, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let (status, headers) = head.split_once("\r\n").unwrap_or((head, ""));
        (status.to_string(), headers.to_string(), body.to_string())
    }

    #[test]
    fn the_session_endpoint_returns_the_real_ws_coordinates_with_no_permissive_cors() {
        let root = make_web_root();
        let (server, port) = bind_ephemeral("127.0.0.1").unwrap();
        let config = StaticServeConfig {
            web_root: root.clone(),
            ws_port: 54321,
            ws_token: "a-real-live-token".to_string(),
        };
        thread::spawn(move || {
            let _ = serve(server, config);
        });
        thread::sleep(Duration::from_millis(20));

        let (status, headers, body) = http_get(port, SESSION_PATH);
        assert!(
            status.contains("200"),
            "session endpoint must 200: {status}"
        );
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["wsPort"], 54321);
        assert_eq!(json["wsToken"], "a-real-live-token");
        assert!(
            !headers.to_lowercase().contains("access-control-allow-origin"),
            "the token endpoint must NOT emit a permissive CORS header -- SOP is the guard: {headers}"
        );

        // A real static file is served; a traversal attempt 404s.
        let (index_status, _, index_body) = http_get(port, "/");
        assert!(index_status.contains("200"));
        assert!(index_body.contains("<title>web</title>"));
        let (bad_status, _, _) = http_get(port, "/../../../etc/passwd");
        assert!(
            bad_status.contains("404"),
            "a traversal attempt over the wire must 404: {bad_status}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// Full orchestration smoke test: `run`-style wiring -- static server +
    /// WS server both live, the session endpoint's advertised token actually
    /// authenticates a real WebSocket connection from the allowlisted origin.
    #[test]
    fn the_advertised_token_authenticates_a_real_ws_connection() {
        use spartan_backend::ws_transport::{self, WsSecurity};
        use spartan_backend::BackendState;
        use tungstenite::client::ClientRequestBuilder;
        use tungstenite::http::Uri;
        use tungstenite::Message;

        let root = make_web_root();
        let (static_server, static_port) = bind_ephemeral("127.0.0.1").unwrap();
        let (ws_listener, ws_port) = bind_ws_listener("127.0.0.1").unwrap();
        let token = "orchestrated-token".to_string();
        let static_origin = format!("http://127.0.0.1:{static_port}");

        // WS server: only the static server's own origin is allowlisted.
        let security = WsSecurity {
            allowed_origins: vec![static_origin.clone()],
            token: token.clone(),
        };
        let backend = Arc::new(Mutex::new(BackendState::new()));
        let dispatch = crate::make_dispatcher(Arc::new(crate::DevServerState::new()));
        thread::spawn(move || {
            let _ = ws_transport::serve(backend, ws_listener, security, dispatch);
        });

        // Static server advertises those exact WS coordinates.
        thread::spawn(move || {
            let _ = serve(
                static_server,
                StaticServeConfig {
                    web_root: root,
                    ws_port,
                    ws_token: token,
                },
            );
        });
        thread::sleep(Duration::from_millis(30));

        // Fetch the session coordinates over HTTP...
        let (_, _, body) = http_get(static_port, SESSION_PATH);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let advertised_port = json["wsPort"].as_u64().unwrap() as u16;
        let advertised_token = json["wsToken"].as_str().unwrap();

        // ...and use them to open a real WS connection from the allowlisted
        // origin. It must succeed and answer a real devserver_ping.
        let uri: Uri = format!("ws://127.0.0.1:{advertised_port}/?token={advertised_token}")
            .parse()
            .unwrap();
        let request = ClientRequestBuilder::new(uri).with_header("Origin", static_origin);
        let (mut ws, _) = tungstenite::connect(request)
            .expect("the advertised token+origin must authenticate a real WS connection");
        ws.send(Message::Text(
            serde_json::to_string(&spartan_backend::Request {
                id: 1,
                method: crate::DEVSERVER_PING.to_string(),
                params: serde_json::json!({}),
            })
            .unwrap()
            .into(),
        ))
        .unwrap();
        let tungstenite::Message::Text(text) = ws.read().unwrap() else {
            panic!("expected a real text frame");
        };
        let resp: spartan_backend::Response = serde_json::from_str(&text).unwrap();
        assert_eq!(resp.result.unwrap()["service"], "spartan-devserver");
    }
}
