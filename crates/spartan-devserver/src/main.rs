//! Real, runnable local devserver binary. Serves the `web/` client and the
//! same-origin WebSocket token handoff, driving the shared `spartan-backend`
//! over the reused WebSocket transport. Localhost-only by default; a paired
//! trusted-LAN mode exists for the companion mobile app.
//!
//! Usage:
//!   spartan-devserver [--web-root:<dir>] [--static-port:<port>] [--project-root:<dir>]
//!                     [--host:<LAN-IP> --mobile-pairing-token:<secret>] [--check-update]
//!
//! Defaults: `--web-root:web/dist`, `--static-port:4400`, `--project-root:.`
//! (the directory the devserver was launched from -- the intended workflow
//! is `cd my-project && spartan-devserver`, matching how `code .`/most CLI
//! IDE launchers already scope themselves to the invoking directory).

use std::path::PathBuf;

use qrcode::render::unicode;
use qrcode::QrCode;

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
    let host = parse_flag(&args, "--host:").unwrap_or("127.0.0.1");
    let mobile_pairing_token = parse_flag(&args, "--mobile-pairing-token:").map(str::to_owned);
    let print_mobile_qr = args.iter().any(|arg| arg == "--print-mobile-qr");

    if args.iter().any(|arg| arg == "--check-update") {
        let result = spartan_updater::check_latest_release(
            spartan_updater::SPARTAN_REPOSITORY,
            env!("CARGO_PKG_VERSION"),
        )
        .map_err(|error| std::io::Error::other(format!("GitHub release check failed: {error}")))?;
        if result.update_available {
            println!(
                "Update available: {} -> {}\nInstall from: {}",
                result.current_version, result.latest_version, result.release_url
            );
        } else {
            println!("spartan-devserver {} is up to date", result.current_version);
        }
        return Ok(());
    }

    if host == "0.0.0.0" || host == "::" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing wildcard bind; provide one explicit trusted-LAN IP with --host:<LAN-IP>",
        ));
    }
    if host != "127.0.0.1" && mobile_pairing_token.as_deref().is_none_or(str::is_empty) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "a non-loopback --host requires --mobile-pairing-token:<secret>",
        ));
    }
    if print_mobile_qr {
        let pairing_token = mobile_pairing_token.as_deref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--print-mobile-qr requires --mobile-pairing-token:<secret>",
            )
        })?;
        let payload = format!(
            "spartan://pair/v1?kind=private&endpoint=http%3A%2F%2F{host}%3A{static_port}&pairing={pairing_token}"
        );
        let qr = QrCode::new(payload.as_bytes())
            .map_err(|e| std::io::Error::other(format!("could not encode pairing QR: {e}")))?;
        eprintln!(
            "spartan-devserver: scan this QR only on a trusted device; it grants private-server access:\n{}",
            qr.render::<unicode::Dense1x2>().quiet_zone(true).build()
        );
    }

    if !web_root.exists() {
        eprintln!(
            "spartan-devserver: warning: web root {web_root:?} does not exist yet \
             (build it with `cd web && npm run build`); serving it anyway -- \
             static requests will 404 until it's present."
        );
    }
    eprintln!(
        "spartan-devserver: first-time setup: build web/ before serving it; run --check-update to check official GitHub Releases without changing this host"
    );

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

    spartan_devserver::run(
        web_root,
        host,
        static_port,
        project_root,
        mobile_pairing_token,
    )
}
