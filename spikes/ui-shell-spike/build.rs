use std::env;
use std::fs;
use std::path::PathBuf;

/// A real, documented gotcha found by actually running this crate, not
/// predicted in advance: `webview2-com-sys`'s own `build.rs` already copies
/// `WebView2Loader.dll` into *its* `OUT_DIR` and wires up
/// `cargo:rustc-link-search` so linking succeeds -- but Cargo has no
/// built-in mechanism for a dependency's build script to place a runtime
/// DLL next to the final executable, only next to its own build artifacts.
/// The result: `ui-shell-spike.exe` links cleanly against
/// `WebView2Loader.dll.lib` but fails at process start with
/// `STATUS_DLL_NOT_FOUND` (0xC0000135) because the actual `.dll` is never
/// next to the `.exe`. Confirmed by comparing this binary's import table
/// against `render-spike.exe`'s (which doesn't use WebView2 and runs fine)
/// via `objdump -p` -- `WebView2Loader.dll` was the one import unique to
/// this binary that wasn't resolving. This is the same fix Tauri's own
/// build tooling (`tauri-build`) applies automatically for real Tauri apps;
/// this spike does it by hand since it isn't using that tooling.
fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set by cargo"));
    // OUT_DIR looks like target/<profile>/build/ui-shell-spike-<hash>/out --
    // three levels up is target/<profile>/, where the final .exe lands.
    let target_profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR had fewer ancestors than expected")
        .to_path_buf();

    // webview2-com-sys's own build.rs places its copy at
    // target/<profile>/build/webview2-com-sys-<hash>/out/x64/WebView2Loader.dll
    // (or x86/arm64 depending on target arch). Search for it rather than
    // hardcoding a hash, since that hash changes across Cargo invocations.
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
        // build scripts on subsequent builds anyway. Print a clear message
        // instead of silently doing nothing, since a missing DLL at runtime
        // otherwise fails with an opaque STATUS_DLL_NOT_FOUND.
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
