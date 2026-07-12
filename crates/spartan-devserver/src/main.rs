//! Real, runnable local devserver binary. Serves the `web/` client and the
//! same-origin WebSocket token handoff, driving the shared `spartan-backend`
//! over the reused WebSocket transport. Localhost-only by construction.
//!
//! Usage:
//!   spartan-devserver [--web-root:<dir>] [--static-port:<port>]
//!
//! Defaults: `--web-root:web/dist`, `--static-port:4400`.

use std::path::PathBuf;

fn parse_flag<'a>(args: &'a [String], prefix: &str) -> Option<&'a str> {
    args.iter().find_map(|a| a.strip_prefix(prefix))
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let web_root = parse_flag(&args, "--web-root:")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("web/dist"));
    let static_port = parse_flag(&args, "--static-port:")
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(4400);

    if !web_root.exists() {
        eprintln!(
            "spartan-devserver: warning: web root {web_root:?} does not exist yet \
             (build it with `cd web && npm run build`); serving it anyway -- \
             static requests will 404 until it's present."
        );
    }

    spartan_devserver::run(web_root, "127.0.0.1", static_port)
}
