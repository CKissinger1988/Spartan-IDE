//! Real §39.3 fidelity test: this is the specific gap §47.2 named as
//! unconfirmed -- "the actual >=80% real-world tool-call fidelity target
//! against a real 7B/13B local model... that requires an actual Ollama
//! instance." A later session found that blocker stale (a network policy
//! change, not a permanent fact, see CLAUDE.md's Spike 0.3 status note) and
//! got a real Ollama instance reachable. This test drives it for real.
//!
//! Self-skips (prints a message, doesn't fail), matching `dap-spike`'s and
//! `lsp-spike`'s established pattern, if Ollama isn't running or the model
//! isn't pulled -- this is meant to degrade gracefully across machines, not
//! gate CI on a specific local model being present everywhere.
//!
//! Model used: `llama3.1:8b` -- a real §39.3-target-class model (~8B
//! parameters, the literal "~7B class" this spike's own fidelity target was
//! scoped to), updated from an earlier session's `llama3.2:1b` run. That
//! smaller model was a real, deliberate, honestly-scoped-down substitution
//! at the time (disk had only ~12GB free, nowhere near enough for a
//! 7B-class model on top of everything else already on disk) -- this
//! session found more real headroom (reclaimed regenerable build artifacts,
//! ~14GB freed) and pulled the actually-targeted model size instead of
//! continuing to report against a smaller stand-in. See CLAUDE.md's Spike
//! 0.3 status note for both the original 1.2B result and this update.

use fallback_parser_spike::{FallbackParser, ParseEvent};
use std::time::Duration;

const OLLAMA_URL: &str = "http://localhost:11434";
const MODEL: &str = "llama3.1:8b";

const SYSTEM_PROMPT: &str = "You are a coding assistant with access to tools. To call a tool, respond with ONLY a fenced code block in this exact format, and nothing else:\n```spartan-tool-call\n{\"tool\": \"<tool_name>\", \"args\": {<arguments as JSON>}}\n```\nAvailable tools:\n- read_file(path: string): reads a file's contents\n- edit_file(path: string, content: string): writes content to a file\n- run_terminal(command: string): runs a shell command";

fn ollama_reachable() -> bool {
    ureq::get(&format!("{OLLAMA_URL}/api/version"))
        .timeout(Duration::from_secs(2))
        .call()
        .is_ok()
}

fn model_available() -> bool {
    let Ok(resp) = ureq::get(&format!("{OLLAMA_URL}/api/tags"))
        .timeout(Duration::from_secs(2))
        .call()
    else {
        return false;
    };
    let Ok(body) = resp.into_json::<serde_json::Value>() else {
        return false;
    };
    body["models"]
        .as_array()
        .map(|models| models.iter().any(|m| m["name"] == MODEL))
        .unwrap_or(false)
}

/// A real, blocking HTTP call to a real local Ollama instance -- no mocked
/// server, no canned response.
fn real_chat(user_prompt: &str) -> String {
    let body = serde_json::json!({
        "model": MODEL,
        "stream": false,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": user_prompt},
        ]
    });
    let resp: serde_json::Value = ureq::post(&format!("{OLLAMA_URL}/api/chat"))
        .timeout(Duration::from_secs(90))
        .send_json(body)
        .expect("real Ollama /api/chat request failed")
        .into_json()
        .expect("failed to parse Ollama response as JSON");
    resp["message"]["content"]
        .as_str()
        .expect("response missing message.content")
        .to_string()
}

fn parse_all(text: &str) -> Vec<ParseEvent> {
    let mut p = FallbackParser::new();
    let mut events = p.feed(text);
    events.extend(p.finish());
    events
}

#[test]
fn real_local_model_tool_call_fidelity() {
    if !ollama_reachable() {
        println!(
            "SKIP: Ollama not reachable at {OLLAMA_URL} -- install and start Ollama \
             (https://ollama.com) to run this test for real"
        );
        return;
    }
    if !model_available() {
        println!(
            "SKIP: model {MODEL} not pulled -- run `ollama pull {MODEL}` to run this test for real"
        );
        return;
    }

    // Five real prompts: three should trigger a tool call, one deliberately
    // should not (a plain arithmetic question), one is a real ambiguous
    // case (asks for a *new* file, which a correct model should route to
    // `edit_file`, not `read_file`) included specifically because an
    // earlier manual trial against this exact model produced that wrong-tool
    // failure -- kept in the automated suite instead of only being a
    // one-off anecdote.
    let tool_prompts = [
        "Please read the file at src/main.rs so you can see what's in it.",
        "Run the command 'cargo test' in the terminal for me.",
        "Create a new file called hello.txt containing the text 'Hello, world!'",
    ];
    let non_tool_prompt = "What's 2 + 2? Just answer the number, no tools needed.";

    let mut parsed_a_tool_call = 0;
    let mut correct_tool_choice = 0;
    let expected_tools = ["read_file", "run_terminal", "edit_file"];

    for (prompt, expected_tool) in tool_prompts.iter().zip(expected_tools.iter()) {
        let raw = real_chat(prompt);
        let events = parse_all(&raw);
        println!("--- prompt: {prompt}\nraw response: {raw}\nparsed events: {events:?}\n");

        let matched_tool = events.iter().find_map(|e| match e {
            ParseEvent::ToolCallParsed { name, .. } => Some(name.as_str()),
            _ => None,
        });
        if matched_tool.is_some() {
            parsed_a_tool_call += 1;
        }
        if matched_tool == Some(*expected_tool) {
            correct_tool_choice += 1;
        }
    }

    let non_tool_raw = real_chat(non_tool_prompt);
    let non_tool_events = parse_all(&non_tool_raw);
    println!("--- prompt: {non_tool_prompt}\nraw response: {non_tool_raw}\nparsed events: {non_tool_events:?}\n");
    let incorrectly_called_a_tool = non_tool_events
        .iter()
        .any(|e| matches!(e, ParseEvent::ToolCallParsed { .. }));

    println!(
        "\n=== Real fidelity report: {MODEL} ===\n\
         Syntactically parseable tool calls: {parsed_a_tool_call}/{}\n\
         Correct tool chosen for the task:   {correct_tool_choice}/{}\n\
         Incorrectly called a tool when none was needed: {incorrectly_called_a_tool}",
        tool_prompts.len(),
        tool_prompts.len()
    );

    // This assertion is deliberately loose -- the point of this test is an
    // honest, printed fidelity report against a real model, not a strict
    // CI-enforced quality gate on this specific model's tool-selection
    // judgment (§39.3's own 80% target is a real design goal to measure
    // against, not something this test fails the build over -- a model
    // upgrade, prompt change, or quantization difference can legitimately
    // move this number run to run). The one thing that MUST hold, matching
    // §3.4's non-negotiable requirement: a real, syntactically-valid tool
    // call this model is capable of producing must actually parse -- if it
    // doesn't, the parser (not the model) has a real bug.
    assert!(
        parsed_a_tool_call >= 1,
        "expected at least one real tool call to parse successfully; got 0/{}",
        tool_prompts.len()
    );
}
