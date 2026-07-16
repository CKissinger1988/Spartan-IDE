//! Real, runnable local devserver binary. Serves the `web/` client and the
//! same-origin WebSocket token handoff, driving the shared `spartan-backend`
//! over the reused WebSocket transport. Localhost-only by construction.
//!
//! Usage:
//!   spartan-devserver [--web-root:<dir>] [--static-port:<port>] [--project-root:<dir>]
//!
//! Defaults: `--web-root:web/dist`, `--static-port:4400`, `--project-root:.`
//! (the directory the devserver was launched from -- the intended workflow
//! is `cd my-project && spartan-devserver`, matching how `code .`/most CLI
//! IDE launchers already scope themselves to the invoking directory).

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
    let project_root_arg = parse_flag(&args, "--project-root:")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    if !web_root.exists() {
        eprintln!(
            "spartan-devserver: warning: web root {web_root:?} does not exist yet \
             (build it with `cd web && npm run build`); serving it anyway -- \
             static requests will 404 until it's present."
        );
    }

    // Canonicalized so the advertised `projectRoot` is a real, absolute
    // path a browser client can pass straight back into `git_status`/
    // `open_file`/Leo calls with no further resolution. A directory that
    // doesn't exist (or isn't readable) degrades to `None` -- an honest
    // "no known project root" rather than advertising a path that can't
    // actually be opened.
    let project_root = match project_root_arg.canonicalize() {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!(
                "spartan-devserver: warning: project root {project_root_arg:?} could not be \
                 resolved ({e}); git/file/Leo methods that need a project root will be \
                 unavailable to connected web clients."
            );
            None
        }
    };

    spartan_devserver::run(web_root, "127.0.0.1", static_port, project_root)
}
