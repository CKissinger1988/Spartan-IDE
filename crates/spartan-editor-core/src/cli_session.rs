//! Real multi-CLI orchestration (§75.57, user-requested -- matching the
//! "run all three coding tools in one place" concept from a real external
//! product's own site, reimplemented here as original code, not ported):
//! tracks zero or more real external coding-CLI sessions, each a real
//! `terminal::TerminalPanel` (a real PTY, per §75.56) running a real named
//! tool (`claude`, `codex`, `gemini`, or any custom command) rather than
//! an interactive shell. Deliberately thin -- all the real PTY/streaming
//! mechanics already exist in `terminal.rs`; this module only adds the
//! "which tool, what label, is it actually installed" bookkeeping on top,
//! plus a real, append-only trace log per session for `workflow.rs`'s own
//! session-detail/compare views to read.

use crate::terminal::TerminalPanel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliTool {
    Claude,
    Codex,
    Gemini,
    Custom(String),
}

impl CliTool {
    pub fn command(&self) -> &str {
        match self {
            CliTool::Claude => "claude",
            CliTool::Codex => "codex",
            CliTool::Gemini => "gemini",
            CliTool::Custom(cmd) => cmd,
        }
    }

    pub fn label(&self) -> String {
        match self {
            CliTool::Claude => "Claude".to_string(),
            CliTool::Codex => "Codex".to_string(),
            CliTool::Gemini => "Gemini".to_string(),
            CliTool::Custom(cmd) => cmd.clone(),
        }
    }
}

/// One real trace entry -- appended whenever a session is spawned, a real
/// command is sent to it, or it's detected to have exited. Deliberately
/// coarse (not a full per-byte transcript, which `TerminalPanel::
/// display_text` already provides) -- this is the real, structured
/// "input/output blocks" a session-detail view scrolls through, matching
/// the real external product's own described "scroll traces, inspect
/// input/output blocks" concept.
#[derive(Debug, Clone)]
pub struct TraceEntry {
    pub kind: TraceKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceKind {
    Spawned,
    Input,
    Exited,
}

/// One real tracked CLI session -- `panel` is `None` if the real spawn
/// itself failed (most commonly: the named tool isn't actually installed
/// on this machine, a real, honest, named failure mode rather than a
/// silent no-op).
pub struct CliSession {
    pub id: u64,
    pub tool: CliTool,
    pub label: String,
    pub panel: Option<TerminalPanel>,
    pub spawn_error: Option<String>,
    pub trace: Vec<TraceEntry>,
}

impl CliSession {
    /// Real spawn attempt -- `cwd`/`cols`/`rows` forwarded verbatim to
    /// `TerminalPanel::spawn_command`. A failed spawn (tool not installed,
    /// permission denied, ...) is captured as a real `spawn_error`, not
    /// propagated as a hard `Result` error, since a workflow with several
    /// real nodes should be able to show "Codex: not installed" next to
    /// working `Claude`/`Gemini` nodes rather than failing the whole
    /// workflow over one missing tool.
    pub fn spawn(id: u64, tool: CliTool, cwd: &std::path::Path, cols: u16, rows: u16) -> Self {
        let label = tool.label();
        let command = tool.command().to_string();
        match TerminalPanel::spawn_command(cwd, cols, rows, &command, &[]) {
            Ok(panel) => Self {
                id,
                tool,
                label,
                panel: Some(panel),
                spawn_error: None,
                trace: vec![TraceEntry {
                    kind: TraceKind::Spawned,
                    text: format!("Spawned real `{command}` session (pid via real PTY)"),
                }],
            },
            Err(e) => Self {
                id,
                tool,
                label,
                panel: None,
                spawn_error: Some(e.to_string()),
                trace: vec![TraceEntry {
                    kind: TraceKind::Exited,
                    text: format!("Failed to spawn `{command}`: {e}"),
                }],
            },
        }
    }

    /// Real, non-blocking poll -- delegates to the real underlying
    /// `TerminalPanel`, a no-op for a session that failed to spawn.
    pub fn poll(&mut self) -> bool {
        self.panel.as_mut().map(|p| p.poll()).unwrap_or(false)
    }

    /// Sends real input to the session's real PTY and appends a real
    /// trace entry recording what was sent -- the "input... blocks" half
    /// of the real session-detail concept this module names in its own
    /// doc comment.
    pub fn send_input(&mut self, text: &str) {
        if let Some(panel) = self.panel.as_mut() {
            panel.send_input(text.as_bytes());
            self.trace.push(TraceEntry {
                kind: TraceKind::Input,
                text: text.to_string(),
            });
        }
    }

    pub fn display_text(&self) -> String {
        self.panel
            .as_ref()
            .map(|p| p.display_text())
            .unwrap_or_default()
    }
}

/// Owns every real tracked session for the current workflow -- a plain
/// `Vec`, not a `HashMap`, since `workflow.rs`'s own node layout is
/// naturally ordered and small (a handful of real sessions at once, not
/// thousands).
#[derive(Default)]
pub struct CliSessionManager {
    sessions: Vec<CliSession>,
    next_id: u64,
}

impl CliSessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(&mut self, tool: CliTool, cwd: &std::path::Path, cols: u16, rows: u16) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions
            .push(CliSession::spawn(id, tool, cwd, cols, rows));
        id
    }

    pub fn get(&self, id: u64) -> Option<&CliSession> {
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut CliSession> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    pub fn sessions(&self) -> &[CliSession] {
        &self.sessions
    }

    /// Real, non-blocking poll across every real session -- returns
    /// `true` if any session's output changed, matching `TerminalPanel::
    /// poll`'s own "only reshape when something actually changed"
    /// contract.
    pub fn poll_all(&mut self) -> bool {
        let mut changed = false;
        for session in &mut self.sessions {
            if session.poll() {
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real, self-skipping tool-availability check, matching this crate's
    /// own established convention (`lsp_integration.rs`'s
    /// `rust_analyzer_available`) -- checks whether the process can be
    /// *spawned* at all (`Ok` from `.status()`), not whether `--version`
    /// is a real subcommand it recognizes, since a real installed `claude`
    /// binary may not support that flag and still be genuinely present.
    /// `claude` being on `$PATH` is true in this interactive session (the
    /// whole session runs on it) but was never guaranteed in a CI runner
    /// -- found for real by a CI failure, not by inspection.
    fn claude_cli_available() -> bool {
        std::process::Command::new("claude")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    }

    #[test]
    fn cli_tool_command_and_label_are_correct_for_each_real_variant() {
        assert_eq!(CliTool::Claude.command(), "claude");
        assert_eq!(CliTool::Claude.label(), "Claude");
        assert_eq!(CliTool::Codex.command(), "codex");
        assert_eq!(CliTool::Gemini.command(), "gemini");
        let custom = CliTool::Custom("my-tool".to_string());
        assert_eq!(custom.command(), "my-tool");
        assert_eq!(custom.label(), "my-tool");
    }

    #[test]
    fn spawning_a_real_installed_tool_succeeds_and_traces_it() {
        if !claude_cli_available() {
            eprintln!("SKIP: claude CLI not found on this machine");
            return;
        }
        // `claude` is confirmed real and installed in this environment
        // (this whole session runs on it) -- a real, not mocked, spawn.
        let dir = std::env::temp_dir();
        let session = CliSession::spawn(0, CliTool::Claude, &dir, 80, 24);
        assert!(session.spawn_error.is_none());
        assert!(session.panel.is_some());
        assert_eq!(session.trace.len(), 1);
        assert_eq!(session.trace[0].kind, TraceKind::Spawned);
    }

    #[test]
    fn spawning_a_real_nonexistent_tool_fails_honestly_not_silently() {
        let dir = std::env::temp_dir();
        let session = CliSession::spawn(
            0,
            CliTool::Custom("definitely-not-a-real-command-xyz".to_string()),
            &dir,
            80,
            24,
        );
        assert!(session.panel.is_none());
        assert!(session.spawn_error.is_some());
        assert_eq!(session.trace[0].kind, TraceKind::Exited);
    }

    #[test]
    fn manager_assigns_real_sequential_ids_and_can_look_sessions_up() {
        let dir = std::env::temp_dir();
        let mut manager = CliSessionManager::new();
        let id1 = manager.spawn(CliTool::Claude, &dir, 80, 24);
        let id2 = manager.spawn(CliTool::Custom("nonexistent-xyz".to_string()), &dir, 80, 24);
        assert_ne!(id1, id2);
        assert!(manager.get(id1).is_some());
        assert!(manager.get(id2).is_some());
        assert_eq!(manager.sessions().len(), 2);
    }

    #[test]
    fn send_input_appends_a_real_trace_entry() {
        if !claude_cli_available() {
            eprintln!("SKIP: claude CLI not found on this machine");
            return;
        }
        let dir = std::env::temp_dir();
        let mut session = CliSession::spawn(0, CliTool::Claude, &dir, 80, 24);
        session.send_input("--version\n");
        assert_eq!(session.trace.len(), 2);
        assert_eq!(session.trace[1].kind, TraceKind::Input);
        assert_eq!(session.trace[1].text, "--version\n");
    }
}
