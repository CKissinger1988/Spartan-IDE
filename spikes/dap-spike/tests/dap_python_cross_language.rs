//! Cross-language reuse check for the *same* `DapClient` from `dap_integration.rs`
//! — no Python-specific code added to the client, no new abstraction. This
//! directly tests the open risk named in docs/architecture-spec.md §47.6:
//! "any language other than Rust... the registry-replication risk §39.2's
//! exit artifact was meant to inform is still open." Probed manually against
//! `debugpy.adapter` before writing this file, same discipline as the Rust
//! DAP/LSP spikes. Skips if `debugpy` isn't installed.

use dap_spike::DapClient;
use std::path::PathBuf;

/// Windows' official python.org installer creates only `python.exe` — no
/// `python3` alias, unlike most Unix distros where `python3` is the name
/// guaranteed to exist instead. Try the more portable name first, then fall
/// back to the Windows-only one, rather than assuming either unconditionally
/// — found by running this for real on Windows, where a hardcoded `python3`
/// made every check here silently report "not available" even with a
/// working Python + debugpy install on `PATH`.
fn python_bin() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|candidate| {
        std::process::Command::new(candidate)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

fn debugpy_available(python: &str) -> bool {
    std::process::Command::new(python)
        .args(["-c", "import debugpy"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const TARGET_PY: &str = r#"def compute(x):
    y = x * 2  # BREAKPOINT_TARGET
    return y + 1

def main():
    result = compute(21)
    print(f"result={result}")

if __name__ == "__main__":
    main()
"#;

fn work_dir() -> PathBuf {
    let dir = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn same_dap_client_hits_a_real_breakpoint_in_a_real_python_program() {
    let Some(python) = python_bin() else {
        eprintln!("SKIP: no python3/python on this machine");
        return;
    };
    if !debugpy_available(python) {
        eprintln!("SKIP: python3/python or the debugpy package isn't available on this machine");
        return;
    }

    let dir = work_dir();
    let src_path = dir.join("target_cross_lang.py");
    std::fs::write(&src_path, TARGET_PY).expect("write python fixture");

    // The *exact same* client code path used for lldb-dap in dap_integration.rs
    // — spawned against a different adapter binary and a different target
    // language, nothing else changed.
    let mut client = spawn_debugpy_adapter(python).expect("spawn debugpy adapter");

    let result = client.launch_and_break(
        src_path.to_str().unwrap(),
        dir.to_str().unwrap(),
        src_path.to_str().unwrap(),
        &[2], // the BREAKPOINT_TARGET line, 1-indexed
    );
    let Some(result) = result else {
        panic!("DAP launch/breakpoint sequence did not complete against debugpy");
    };

    let set_bp = &result["setBreakpoints"]["body"]["breakpoints"][0];
    assert_eq!(set_bp["verified"].as_bool(), Some(true));
    assert_eq!(set_bp["line"].as_i64(), Some(2));

    let stopped = &result["stopped"];
    assert_eq!(stopped["body"]["reason"].as_str(), Some("breakpoint"));
    let thread_id = stopped["body"]["threadId"]
        .as_i64()
        .expect("threadId present");

    let frames = client.stack_trace(thread_id).expect("stackTrace response");
    let top_frame = &frames["body"]["stackFrames"][0];
    let frame_name = top_frame["name"].as_str().unwrap_or_default();
    assert!(
        frame_name.contains("compute"),
        "top frame should be inside compute(), got: {top_frame:?}"
    );
    let frame_id = top_frame["id"].as_i64().unwrap();

    let scopes = client.scopes(frame_id).expect("scopes response");
    let locals_ref = scopes["body"]["scopes"][0]["variablesReference"]
        .as_i64()
        .unwrap();
    let vars = client.variables(locals_ref).expect("variables response");
    let x = vars["body"]["variables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"].as_str() == Some("x"))
        .expect("x is in scope at the breakpoint");
    assert_eq!(x["value"].as_str(), Some("21"));

    client.shutdown();
}

/// `debugpy`'s adapter binary is `python3 -m debugpy.adapter`, not a single
/// executable, so it needs its own spawn helper rather than reusing
/// `DapClient::spawn`'s single-binary-path signature as-is. This is the one
/// piece of glue code this cross-language check actually needed — everything
/// downstream (`launch_and_break`, `stack_trace`, `scopes`, `variables`,
/// `shutdown`) is the identical, unmodified client used against `lldb-dap`.
fn spawn_debugpy_adapter(python: &str) -> std::io::Result<DapClient> {
    // `DapClient::spawn` takes a single command string and executes it
    // directly (no shell), so `<python> -m debugpy.adapter` needs a tiny
    // wrapper script on $PATH-independent grounds: write one to the tmp dir.
    // A `#!/bin/sh` script has no interpreter on Windows (no shebang
    // support), so the wrapper format is platform-specific — found by
    // running this for real on Windows, where the original Unix-only
    // wrapper could never actually launch.
    let dir = work_dir();
    #[cfg(windows)]
    {
        let wrapper = dir.join("run-debugpy-adapter.cmd");
        std::fs::write(
            &wrapper,
            format!("@echo off\r\n{python} -m debugpy.adapter %*\r\n"),
        )
        .unwrap();
        DapClient::spawn(wrapper.to_str().unwrap())
    }
    #[cfg(not(windows))]
    {
        let wrapper = dir.join("run-debugpy-adapter.sh");
        std::fs::write(
            &wrapper,
            format!("#!/bin/sh\nexec {python} -m debugpy.adapter \"$@\"\n"),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&wrapper).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&wrapper, perms).unwrap();
        DapClient::spawn(wrapper.to_str().unwrap())
    }
}
