//! Real §3.4 structured-output fallback parser (task #4), promoted
//! verbatim from `spikes/fallback-parser-spike` (Spike 0.3) into real
//! product code -- the same promotion pattern `spartan-editor-core`'s own
//! `lsp.rs`/`dap.rs` already established for their own origin spikes. This
//! module's own logic was already validated against synthetic adversarial
//! token streams (the tests below, unchanged from the spike) *and*, since
//! this session installed a real local Ollama instance, against a real
//! live model's real output too (`spikes/fallback-parser-spike/tests/
//! real_ollama_fidelity.rs`, now run against a real §39.3-target-class
//! `llama3.1:8b`, not just a synthetic fixture) -- see CLAUDE.md's Spike
//! 0.3 status note for that real result. This is the fallback path for
//! models `ModelProvider::supports_native_tool_calling()` reports `false`
//! for (§3.4); `OllamaProvider`'s own native tool-calling path (confirmed
//! real and working against `llama3.1:8b`, see `ollama.rs`) doesn't need
//! it, but a weaker local model without `"tools"` in its real `/api/tags`
//! capabilities array would.

use std::fmt;

#[derive(Debug, PartialEq, Clone)]
pub enum ParseEvent {
    TextChunk(String),
    ToolCallParsed {
        name: String,
        args: serde_json::Value,
    },
    ToolCallFailed {
        raw: String,
        reason: String,
    }, // MUST surface, never silently drop
}

#[derive(Debug, PartialEq)]
enum State {
    Scanning,
    InFence(String), // accumulating buffer inside a ```spartan-tool-call fence
}

pub struct FallbackParser {
    state: State,
    scan_buf: String,
    max_fence_bytes: usize,
}

impl fmt::Debug for FallbackParser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FallbackParser {{ state: {:?} }}", self.state)
    }
}

const FENCE_OPEN: &str = "```spartan-tool-call";
const FENCE_CLOSE: &str = "```";

impl Default for FallbackParser {
    fn default() -> Self {
        Self::new()
    }
}

impl FallbackParser {
    pub fn new() -> Self {
        Self {
            state: State::Scanning,
            scan_buf: String::new(),
            max_fence_bytes: 8192,
        }
    }

    /// Feed one streamed token/chunk at a time -- mirrors how a real
    /// streaming completion arrives token-by-token, not as one blob.
    pub fn feed(&mut self, chunk: &str) -> Vec<ParseEvent> {
        let mut events = Vec::new();
        self.scan_buf.push_str(chunk);

        loop {
            match &self.state {
                State::Scanning => {
                    if let Some(idx) = self.scan_buf.find(FENCE_OPEN) {
                        if idx > 0 {
                            events.push(ParseEvent::TextChunk(self.scan_buf[..idx].to_string()));
                        }
                        let rest = self.scan_buf[idx + FENCE_OPEN.len()..].to_string();
                        self.scan_buf = rest;
                        self.state = State::InFence(String::new());
                    } else {
                        // No fence found yet -- emit as plain text but keep a small
                        // tail in case a fence marker is split across chunks.
                        //
                        // BUG FOUND & FIXED during debugging: the raw byte offset
                        // `len - keep` can land inside a multi-byte UTF-8 character
                        // (Leo's own prose routinely contains non-ASCII text --
                        // em-dashes, emoji, non-English content). Slicing a &str at
                        // a non-char-boundary byte index panics in Rust. Snap the
                        // split point down to the nearest valid char boundary
                        // instead; this only ever keeps a few extra bytes in the
                        // buffer, never loses or misreads data.
                        let keep = FENCE_OPEN.len().saturating_sub(1);
                        if self.scan_buf.len() > keep {
                            let mut split = self.scan_buf.len() - keep;
                            while split > 0 && !self.scan_buf.is_char_boundary(split) {
                                split -= 1;
                            }
                            if split > 0 {
                                events.push(ParseEvent::TextChunk(
                                    self.scan_buf[..split].to_string(),
                                ));
                                self.scan_buf = self.scan_buf[split..].to_string();
                            }
                        }
                        break;
                    }
                }
                State::InFence(_) => {
                    if let Some(idx) = self.scan_buf.find(FENCE_CLOSE) {
                        let body = self.scan_buf[..idx].to_string();
                        let after = self.scan_buf[idx + FENCE_CLOSE.len()..].to_string();
                        self.scan_buf = after;
                        self.state = State::Scanning;

                        match serde_json::from_str::<serde_json::Value>(body.trim()) {
                            Ok(v) => {
                                let name = v
                                    .get("tool")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("<missing 'tool' field>")
                                    .to_string();
                                let args =
                                    v.get("args").cloned().unwrap_or(serde_json::Value::Null);
                                if v.get("tool").is_some() {
                                    events.push(ParseEvent::ToolCallParsed { name, args });
                                } else {
                                    events.push(ParseEvent::ToolCallFailed {
                                        raw: body.clone(),
                                        reason: "valid JSON but missing required 'tool' field"
                                            .into(),
                                    });
                                }
                            }
                            Err(e) => {
                                events.push(ParseEvent::ToolCallFailed {
                                    raw: body.clone(),
                                    reason: format!("invalid JSON: {e}"),
                                });
                            }
                        }
                    } else if self.scan_buf.len() > self.max_fence_bytes {
                        // Guard: a fence that never closes must not buffer forever
                        // or hang silently -- surface it as a failure and reset.
                        events.push(ParseEvent::ToolCallFailed {
                            raw: self.scan_buf.clone(),
                            reason: "fence exceeded max buffer size without closing".into(),
                        });
                        self.scan_buf.clear();
                        self.state = State::Scanning;
                    } else {
                        break; // wait for more chunks
                    }
                }
            }
        }
        events
    }

    /// Call at end-of-stream. If we're still mid-fence, the model's output was
    /// cut off -- this MUST be surfaced, never silently discarded per section 3.4.
    pub fn finish(&mut self) -> Vec<ParseEvent> {
        let mut events = Vec::new();
        if let State::InFence(_) = self.state {
            events.push(ParseEvent::ToolCallFailed {
                raw: self.scan_buf.clone(),
                reason: "stream ended mid-fence (truncated tool call)".into(),
            });
        } else if !self.scan_buf.is_empty() {
            events.push(ParseEvent::TextChunk(self.scan_buf.clone()));
        }
        self.scan_buf.clear();
        self.state = State::Scanning;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_streamed(chunks: &[&str]) -> Vec<ParseEvent> {
        let mut p = FallbackParser::new();
        let mut out = Vec::new();
        for c in chunks {
            out.extend(p.feed(c));
        }
        out.extend(p.finish());
        out
    }

    #[test]
    fn well_formed_tool_call_streamed_token_by_token() {
        // Simulates a real model emitting the fence one token at a time.
        let full = r#"Sure, I'll check that file. ```spartan-tool-call
{"tool": "read_file", "args": {"path": "src/main.rs"}}
``` done."#;
        let chunks: Vec<&str> = full.split_inclusive(' ').collect();
        let events = run_streamed(&chunks);
        let ok = events.iter().any(|e| {
            matches!(e,
            ParseEvent::ToolCallParsed { name, .. } if name == "read_file")
        });
        assert!(ok, "expected a parsed read_file call, got: {events:?}");
        assert!(!events
            .iter()
            .any(|e| matches!(e, ParseEvent::ToolCallFailed { .. })));
    }

    #[test]
    fn plain_text_no_tool_call_passes_through_untouched() {
        let events = run_streamed(&["Just explaining ", "something, no tool needed."]);
        assert!(events.iter().all(|e| matches!(e, ParseEvent::TextChunk(_))));
    }

    #[test]
    fn malformed_json_is_surfaced_not_dropped() {
        // A weak local model producing near-JSON, e.g. trailing comma / unquoted key.
        let full = "```spartan-tool-call\n{tool: \"edit_file\", args: {}}\n```";
        let events = run_streamed(&[full]);
        let failed = events
            .iter()
            .find(|e| matches!(e, ParseEvent::ToolCallFailed { .. }));
        assert!(
            failed.is_some(),
            "malformed JSON must be surfaced as a failure, not silently dropped"
        );
    }

    #[test]
    fn missing_tool_field_is_surfaced_not_silently_guessed() {
        let full = r#"```spartan-tool-call
{"args": {"path": "x"}}
```"#;
        let events = run_streamed(&[full]);
        assert!(events.iter().any(|e| matches!(e,
            ParseEvent::ToolCallFailed { reason, .. } if reason.contains("missing required 'tool'"))));
    }

    #[test]
    fn truncated_stream_mid_fence_is_surfaced_not_hung() {
        // Simulates the connection dying / model cut off before closing the fence.
        let events =
            run_streamed(&["```spartan-tool-call\n{\"tool\": \"run_terminal\", \"args\": {"]);
        assert!(events.iter().any(|e| matches!(e,
            ParseEvent::ToolCallFailed { reason, .. } if reason.contains("truncated"))));
    }

    #[test]
    fn fence_marker_split_across_chunk_boundary_still_detected() {
        // The most common real streaming failure mode: the fence marker itself
        // gets split across two network chunks.
        let events = run_streamed(&[
            "Let me help. ``",
            "`spartan-tool-call\n{\"tool\": \"lint_check\", \"args\": {}}\n```",
        ]);
        assert!(events.iter().any(|e| matches!(e,
            ParseEvent::ToolCallParsed { name, .. } if name == "lint_check")));
    }

    #[test]
    fn runaway_unclosed_fence_does_not_hang_or_oom() {
        let mut p = FallbackParser::new();
        let junk = "x".repeat(20_000); // exceeds max_fence_bytes
        let events = p.feed(&format!("```spartan-tool-call\n{junk}"));
        assert!(events.iter().any(|e| matches!(e,
            ParseEvent::ToolCallFailed { reason, .. } if reason.contains("exceeded max buffer"))));
    }

    #[test]
    fn multiple_tool_calls_in_one_response_both_captured() {
        let full = "```spartan-tool-call\n{\"tool\":\"a\",\"args\":{}}\n``` then ```spartan-tool-call\n{\"tool\":\"b\",\"args\":{}}\n```";
        let events = run_streamed(&[full]);
        let names: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ParseEvent::ToolCallParsed { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;

    /// Regression test for a real bug found during debugging: a multi-byte
    /// UTF-8 character (e.g. an emoji in Leo's own prose) landing exactly at
    /// the tail-keep cut point used to panic with "byte index N is not a char
    /// boundary". Sweeps the exact suffix lengths that reproduce it.
    #[test]
    fn no_char_boundary_panic_when_multibyte_char_hits_tail_cut_point() {
        for suffix_len in 0..=30 {
            let text = format!("x🐛{}", "b".repeat(suffix_len));
            let mut p = FallbackParser::new();
            p.feed(&text); // must not panic for any suffix length
        }
    }

    #[test]
    fn multibyte_content_is_never_dropped_only_deferred() {
        // Correctness, not just crash-safety: confirm all bytes eventually
        // surface as text when the buffer is finally flushed.
        let mut p = FallbackParser::new();
        let text = "prefix 🐛🦀 suffix, no fence in this message at all.";
        let mut collected = String::new();
        for e in p.feed(text) {
            if let ParseEvent::TextChunk(s) = e {
                collected.push_str(&s);
            }
        }
        for e in p.finish() {
            if let ParseEvent::TextChunk(s) = e {
                collected.push_str(&s);
            }
        }
        assert_eq!(
            collected, text,
            "no bytes should be lost across the tail-keep boundary"
        );
    }
}
