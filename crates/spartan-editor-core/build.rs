use std::env;
use std::fs;
use std::path::PathBuf;

/// Promoted verbatim from `spikes/ui-shell-spike/build.rs` (§47.11) for the
/// same real reason: `webview2-com-sys`'s own build script copies
/// `WebView2Loader.dll` into *its* `OUT_DIR` and wires up
/// `cargo:rustc-link-search` so linking succeeds, but Cargo has no built-in
/// mechanism for a dependency's build script to place a runtime DLL next to
/// the *final* executable, only next to its own build artifacts -- without
/// this, the real product binary would link cleanly on Windows but fail at
/// process start with `STATUS_DLL_NOT_FOUND`. A real, harmless no-op on
/// every other platform (confirmed repeatedly in this project's own history
/// running `ui-shell-spike` on Linux, §75.12 onward): the search simply
/// finds nothing and prints a warning instead of failing the build.
fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set by cargo"));
    let target_profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR had fewer ancestors than expected")
        .to_path_buf();

    let build_dir = target_profile_dir.join("build");
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        other => panic!("unexpected CARGO_CFG_TARGET_ARCH: {other:?}"),
    };

    let mut found = None;
    if let Ok(entries) = fs::read_dir(&build_dir) {
        for entry in entries.flatten() {
            let candidate = entry
                .path()
                .join("out")
                .join(arch)
                .join("WebView2Loader.dll");
            if candidate.is_file() {
                found = Some(candidate);
                break;
            }
        }
    }

    let Some(loader_src) = found else {
        // Don't fail the build over this -- a first run before
        // webview2-com-sys has built yet won't find it, and Cargo re-runs
        // build scripts on subsequent builds anyway.
        println!(
            "cargo:warning=WebView2Loader.dll not found yet under {build_dir:?} -- \
             re-run `cargo build` once more if the resulting exe fails to start"
        );
        return;
    };

    let dest = target_profile_dir.join("WebView2Loader.dll");
    if let Err(e) = fs::copy(&loader_src, &dest) {
        println!("cargo:warning=failed to copy WebView2Loader.dll to {dest:?}: {e}");
    } else {
        println!("cargo:warning=copied WebView2Loader.dll from {loader_src:?} to {dest:?}");
    }
}
