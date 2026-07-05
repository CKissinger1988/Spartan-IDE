//! Real, executed integration tests against a real `lldb-dap` subprocess and
//! real compiled Rust binaries — no mocked debug adapter, no mocked compiler.
//! Skips (rather than fails) if `lldb-dap-18`/`rustc` aren't on this machine,
//! since CI/dev environments vary; see docs/architecture-spec.md §47.5 for
//! what was actually run and observed the first time this was executed.

use dap_spike::*;
use ropey::Rope;
use std::path::PathBuf;
use std::time::Duration;

const ADAPTER_CANDIDATES: &[&str] = &["lldb-dap-18", "lldb-dap"];

fn find_adapter() -> Option<&'static str> {
    ADAPTER_CANDIDATES
        .iter()
        .find(|bin| {
            std::process::Command::new(bin)
                .arg("--help")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .copied()
}

fn work_dir() -> PathBuf {
    let dir = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const ORIGINAL_SOURCE: &str = r#"fn compute(x: i32) -> i32 {
    let y = x * 2; // BREAKPOINT_TARGET
    y + 1
}

fn main() {
    let result = compute(21);
    println!("result={}", result);
}
"#;

/// Extracts a `variables` response body's list and finds one by name.
fn find_variable<'a>(resp: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    resp.get("body")?
        .get("variables")?
        .as_array()?
        .iter()
        .find(|v| v.get("name").and_then(serde_json::Value::as_str) == Some(name))
}

#[test]
fn breakpoint_hits_and_reports_correct_local_variable() {
    let Some(adapter) = find_adapter() else {
        eprintln!("SKIP: no lldb-dap binary found on this machine");
        return;
    };
    let dir = work_dir();
    let src_path = dir.join("target_original.rs");
    let bin_path = dir.join("target_original_bin");
    compile_fixture(ORIGINAL_SOURCE, &src_path, &bin_path);

    let rope = Rope::from_str(ORIGINAL_SOURCE);
    let anchor = anchor_at_marker(&rope, "BREAKPOINT_TARGET");
    let line = line_1indexed_for_byte(&rope, anchor);
    assert_eq!(
        line, 2,
        "sanity: marker is on line 2 of the original source"
    );

    let mut client = DapClient::spawn(adapter).expect("spawn lldb-dap");
    let result = client.launch_and_break(
        bin_path.to_str().unwrap(),
        dir.to_str().unwrap(),
        src_path.to_str().unwrap(),
        &[line],
    );
    let Some(result) = result else {
        panic!("DAP launch/breakpoint sequence did not complete");
    };

    let set_bp = &result["setBreakpoints"]["body"]["breakpoints"][0];
    assert_eq!(
        set_bp["verified"].as_bool(),
        Some(true),
        "adapter should resolve the breakpoint against real debug info: {set_bp:?}"
    );
    assert_eq!(set_bp["line"].as_i64(), Some(line));

    let stopped = &result["stopped"];
    assert_eq!(stopped["body"]["reason"].as_str(), Some("breakpoint"));
    let thread_id = stopped["body"]["threadId"]
        .as_i64()
        .expect("threadId present");

    let frames = client.stack_trace(thread_id).expect("stackTrace response");
    let top_frame = &frames["body"]["stackFrames"][0];
    let frame_id = top_frame["id"].as_i64().expect("frame id");
    // lldb-dap reports the mangled symbol (name + hash suffix), not a fixed
    // "module`function" string as first assumed — checking containment
    // instead of an exact match, since the hash isn't stable across builds.
    let frame_name = top_frame["name"].as_str().unwrap_or_default();
    assert!(
        frame_name.contains("compute"),
        "top frame should be inside compute(), got: {top_frame:?}"
    );

    let scopes = client.scopes(frame_id).expect("scopes response");
    let locals_ref = scopes["body"]["scopes"][0]["variablesReference"]
        .as_i64()
        .expect("locals variablesReference");

    let vars = client.variables(locals_ref).expect("variables response");
    let x = find_variable(&vars, "x").expect("x is in scope at the breakpoint");
    assert_eq!(x["value"].as_str(), Some("21"));

    client.shutdown();
}

#[test]
fn breakpoint_survives_an_edit_that_shifts_line_numbers() {
    // This is the exact success criterion from §39.2: "confirm the
    // breakpoint stays attached to the correct logical line, not the
    // original line number" after an edit inserts lines above it.
    let Some(adapter) = find_adapter() else {
        eprintln!("SKIP: no lldb-dap binary found on this machine");
        return;
    };
    let dir = work_dir();

    let original_rope = Rope::from_str(ORIGINAL_SOURCE);
    let original_anchor = anchor_at_marker(&original_rope, "BREAKPOINT_TARGET");
    assert_eq!(line_1indexed_for_byte(&original_rope, original_anchor), 2);

    // Simulate a real edit: insert three new lines (a doc comment block)
    // immediately above `fn compute`, i.e. at byte 0 — the exact scenario
    // that breaks naive line-number-based breakpoints.
    let inserted_text = "// A newly added\n// doc comment\n// block.\n";
    let insert_at_byte = 0usize;

    let mut edited_rope = original_rope.clone();
    let insert_char_idx = edited_rope.byte_to_char(insert_at_byte);
    edited_rope.insert(insert_char_idx, inserted_text);

    let shifted_anchor =
        shift_anchor_for_insertion(original_anchor, insert_at_byte, inserted_text.len());
    let computed_new_line = line_1indexed_for_byte(&edited_rope, shifted_anchor);

    // Ground truth, independent of the rope math above: search the edited
    // text itself for where the marker actually landed.
    let edited_source = edited_rope.to_string();
    let ground_truth_line = edited_source
        .lines()
        .position(|l| l.contains("BREAKPOINT_TARGET"))
        .map(|idx| (idx + 1) as i64)
        .expect("marker still present after edit");
    assert_eq!(
        computed_new_line, ground_truth_line,
        "rope-anchored breakpoint math must agree with where the marker actually ended up"
    );
    assert_eq!(
        computed_new_line, 5,
        "3 inserted lines should push line 2 to line 5"
    );

    let src_path = dir.join("target_edited.rs");
    let bin_path = dir.join("target_edited_bin");
    compile_fixture(&edited_source, &src_path, &bin_path);

    let mut client = DapClient::spawn(adapter).expect("spawn lldb-dap");
    let result = client
        .launch_and_break(
            bin_path.to_str().unwrap(),
            dir.to_str().unwrap(),
            src_path.to_str().unwrap(),
            &[computed_new_line],
        )
        .expect("DAP launch/breakpoint sequence did not complete");

    let set_bp = &result["setBreakpoints"]["body"]["breakpoints"][0];
    assert_eq!(set_bp["verified"].as_bool(), Some(true));
    assert_eq!(
        set_bp["line"].as_i64(),
        Some(computed_new_line),
        "adapter must resolve the breakpoint at the rope-computed line, not the stale original line 2"
    );

    let stopped = &result["stopped"];
    assert_eq!(stopped["body"]["reason"].as_str(), Some("breakpoint"));
    let thread_id = stopped["body"]["threadId"].as_i64().unwrap();

    // Confirm it's the *same logical statement*, not just some breakpoint at
    // the right line number by coincidence: x should still be 21 (the call
    // site is unchanged) at the moment of the hit.
    let frames = client.stack_trace(thread_id).unwrap();
    let frame_id = frames["body"]["stackFrames"][0]["id"].as_i64().unwrap();
    let scopes = client.scopes(frame_id).unwrap();
    let locals_ref = scopes["body"]["scopes"][0]["variablesReference"]
        .as_i64()
        .unwrap();
    let vars = client.variables(locals_ref).unwrap();
    let x = find_variable(&vars, "x").expect("x is in scope at the shifted breakpoint");
    assert_eq!(x["value"].as_str(), Some("21"));

    client.shutdown();
}

#[test]
fn client_survives_adapter_crash_on_shutdown() {
    // Regression test for the real finding in §47.5: this lldb-dap build
    // SIGABRTs somewhere in its own exit path right after replying to
    // `disconnect`. `shutdown()` must not hang or panic when that happens.
    let Some(adapter) = find_adapter() else {
        eprintln!("SKIP: no lldb-dap binary found on this machine");
        return;
    };
    let mut client = DapClient::spawn(adapter).expect("spawn lldb-dap");
    let resp = client.request(
        "initialize",
        serde_json::json!({
            "clientID": "spartan-dap-spike",
            "adapterID": "lldb-dap",
            "pathFormat": "path",
            "linesStartAt1": true,
            "columnsStartAt1": true,
        }),
        Duration::from_secs(5),
    );
    assert!(
        resp.is_some(),
        "initialize should succeed even with no launch"
    );
    client.shutdown(); // must return promptly, not hang
}
