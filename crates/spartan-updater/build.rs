//! Real build-time capture of the git commit this binary was built from
//! (§75.49, user-requested auto-update feature) -- `git rev-parse HEAD`
//! run once at compile time, baked in via `cargo:rustc-env` so
//! `built_commit_hash()` needs no I/O at runtime. Falls back to the real,
//! honest string `"unknown"` (never a fabricated hash) if `git` isn't on
//! `PATH` or this isn't a real git checkout -- `check_for_updates` treats
//! that as a real, distinct, reported error rather than silently comparing
//! against a made-up baseline.

fn main() {
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=SPARTAN_BUILD_COMMIT={hash}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
