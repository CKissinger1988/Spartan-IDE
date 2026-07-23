//! Real §4.5/§36.4.6 tool execution sandbox (task #5) -- the
//! non-negotiable security layer CLAUDE.md itself calls out: "File write
//! approval is enforced at the tool-execution layer in Rust, not just as a
//! UI suggestion" (§9), and "All tool-layer file path resolution is
//! canonicalized and hard-jailed to the project root... no `../`
//! traversal, symlink escape, or absolute-path override can resolve
//! outside it" (§36.4.6). This module is the actual enforcement point --
//! not a prompt instruction to the model, a real Rust check every tool
//! call passes through before touching the filesystem or spawning a
//! process.
//!
//! Deliberately does **not** implement §36.4.6's one documented Developer
//! Mode exception (a real, separate, later increment once Developer Mode
//! itself has a real settings surface to opt into it from) -- the jail
//! here is unconditional.

use std::fmt;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Real default wall-clock bound for a Leo-driven `run_terminal` call. Chosen
/// generous enough for a real build/test command (`cargo build`, `npm ci`) to
/// finish, but finite so a genuinely hung command can never freeze the agent's
/// execute loop forever (§75.66). Overridable per call via
/// `Sandbox::run_terminal_with_timeout`.
pub const DEFAULT_RUN_TERMINAL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub enum ToolCall {
    ReadFile {
        path: String,
    },
    EditFile {
        path: String,
        content: String,
    },
    RunTerminal {
        command: String,
    },
    /// Real §75.68 addition: a codebase-wide substring search, closing
    /// the single biggest real gap in Leo's original three-tool set --
    /// without this, Leo could only ever read a file it already knew the
    /// exact path of (from the plan's own `files` list), unlike every
    /// other current coding agent (Claude Code, Cursor, Aider all ship a
    /// real grep/search tool). Deliberately plain substring matching, not
    /// full regex -- a real, named v1 simplification avoiding a new
    /// `regex` dependency, not a limitation of the sandbox itself.
    SearchFiles {
        pattern: String,
        path: Option<String>,
    },
    /// Real §75.68 addition: lets Leo explore the project structure
    /// itself instead of relying entirely on the plan's own static file
    /// list -- `path: None` lists the real project root.
    ListDirectory {
        path: Option<String>,
    },
}

impl ToolCall {
    pub fn name(&self) -> &'static str {
        match self {
            ToolCall::ReadFile { .. } => "read_file",
            ToolCall::EditFile { .. } => "edit_file",
            ToolCall::RunTerminal { .. } => "run_terminal",
            ToolCall::SearchFiles { .. } => "search_files",
            ToolCall::ListDirectory { .. } => "list_directory",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolResult {
    FileContent(String),
    FileWritten {
        path: String,
        bytes: usize,
    },
    TerminalOutput {
        stdout: String,
        stderr: String,
        exit_code: i32,
    },
    SearchMatches(Vec<SearchMatch>),
    DirectoryListing(Vec<DirEntry>),
}

#[derive(Debug)]
pub enum SandboxError {
    /// A real, hard path-jail violation (§36.4.6) -- the resolved path
    /// would land outside the project root, via `../` traversal, a
    /// symlink, or an absolute-path override. Always refused, never
    /// warned-and-allowed.
    PathEscapesJail {
        requested: String,
    },
    Io(std::io::Error),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::PathEscapesJail { requested } => {
                write!(
                    f,
                    "refused: '{requested}' resolves outside the project root (path-jail, §36.4.6)"
                )
            }
            SandboxError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// Owns the one real, hard-jailed root every tool call in a session is
/// confined to.
pub struct Sandbox {
    project_root: PathBuf,
}

impl Sandbox {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
        }
    }

    /// The real §36.4.6 enforcement point: resolves `requested` (which may
    /// be relative or absolute, and may not exist yet -- `edit_file`
    /// creating a new file is a real, legitimate case) against
    /// `project_root`, canonicalizing whatever prefix of it already
    /// exists on disk so a symlink partway down the path can't be used to
    /// escape, then checks the final resolved path is still a real
    /// descendant of the canonicalized root. Never trusts a `../`-free
    /// string alone -- a component-by-component join, not raw
    /// string-prefix matching, so `/project-evil/x` can't slip past a
    /// naive `starts_with("/project")` check.
    fn resolve(&self, requested: &str) -> Result<PathBuf, SandboxError> {
        let canonical_root = self.project_root.canonicalize().map_err(SandboxError::Io)?;

        let requested_path = Path::new(requested);
        let joined = if requested_path.is_absolute() {
            requested_path.to_path_buf()
        } else {
            canonical_root.join(requested_path)
        };

        // Lexically normalize (resolve `.`/`..` components) without
        // requiring the full path to exist yet -- `edit_file` on a new
        // file has no real canonicalizable target.
        let mut normalized = PathBuf::new();
        for component in joined.components() {
            match component {
                Component::ParentDir => {
                    if !normalized.pop() {
                        // Already at (or above) the root and asked to go
                        // up further -- a real, unambiguous escape
                        // attempt.
                        return Err(SandboxError::PathEscapesJail {
                            requested: requested.to_string(),
                        });
                    }
                }
                Component::CurDir => {}
                other => normalized.push(other.as_os_str()),
            }
        }

        // If the deepest existing ancestor of `normalized` is itself a
        // symlink pointing outside the root, canonicalizing that ancestor
        // (not the whole, possibly-nonexistent path) catches it.
        let mut check = normalized.clone();
        while !check.exists() {
            if !check.pop() {
                break;
            }
        }
        if check.exists() {
            let canonical_check = check.canonicalize().map_err(SandboxError::Io)?;
            if !canonical_check.starts_with(&canonical_root) {
                return Err(SandboxError::PathEscapesJail {
                    requested: requested.to_string(),
                });
            }
        }

        if !normalized.starts_with(&canonical_root) {
            return Err(SandboxError::PathEscapesJail {
                requested: requested.to_string(),
            });
        }

        Ok(normalized)
    }

    pub fn read_file(&self, path: &str) -> Result<ToolResult, SandboxError> {
        let resolved = self.resolve(path)?;
        let content = std::fs::read_to_string(resolved).map_err(SandboxError::Io)?;
        Ok(ToolResult::FileContent(content))
    }

    pub fn edit_file(&self, path: &str, content: &str) -> Result<ToolResult, SandboxError> {
        let resolved = self.resolve(path)?;
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(SandboxError::Io)?;
        }
        std::fs::write(&resolved, content).map_err(SandboxError::Io)?;
        Ok(ToolResult::FileWritten {
            path: resolved.to_string_lossy().to_string(),
            bytes: content.len(),
        })
    }

    /// Real terminal execution -- deliberately runs the command through a
    /// real shell (`sh -c`) with `current_dir` fixed to the jailed
    /// project root (a real command can't `cd` its way out of the jail
    /// for file operations it performs, since every *tool-layer* file
    /// operation still goes through `resolve()`; a shell command's own
    /// unrestricted filesystem access from *within* the process itself is
    /// a real, separate, named limitation -- see this crate's own
    /// top-level doc comment).
    pub fn run_terminal(&self, command: &str) -> Result<ToolResult, SandboxError> {
        self.run_terminal_with_timeout(command, DEFAULT_RUN_TERMINAL_TIMEOUT)
    }

    /// Real, timeout-bounded terminal execution -- closes the "the sandbox
    /// method has no timeout" gap named on §75.66. A model-driven agent will
    /// occasionally emit a command that never returns (an infinite loop, a
    /// process blocking on stdin, a hung network fetch); without a bound that
    /// freezes the entire execute loop forever. Three real, deliberate
    /// hardening choices here, none of which the old `.output()` had:
    ///   1. **stdin is `/dev/null`** (`Stdio::null()`), so a command that reads
    ///      stdin gets an immediate EOF and exits rather than blocking on input
    ///      no agent is there to type -- the single most common real "hang."
    ///   2. **stdout/stderr are drained on their own threads**, so a command
    ///      that produces a lot of output can't deadlock by filling a pipe
    ///      buffer while the main thread is busy polling -- the classic
    ///      spawn-then-wait pipe-buffer deadlock, avoided the same way this
    ///      project's PTY/subprocess code already does elsewhere.
    ///   3. **a wall-clock deadline** -- past it the child is `kill()`ed and a
    ///      real, honest `exit_code: -1` plus a clear "[timed out after Ns...]"
    ///      note appended to stderr is returned, so the model *sees* that its
    ///      command was killed and can react, rather than the whole call
    ///      erroring or (worse) hanging.
    pub fn run_terminal_with_timeout(
        &self,
        command: &str,
        timeout: Duration,
    ) -> Result<ToolResult, SandboxError> {
        let canonical_root = self.project_root.canonicalize().map_err(SandboxError::Io)?;
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(&canonical_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Run the child in its own process group so a timeout can kill the
        // *whole tree* (`sh` plus any grandchildren it spawned), not just `sh`.
        // Critical for correctness here: an orphaned grandchild would otherwise
        // keep the stdout/stderr pipe write-ends open, so the reader threads'
        // `read_to_end` never hits EOF and their `join()` below would block
        // until the grandchild finally exits -- defeating the timeout entirely.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().map_err(SandboxError::Io)?;

        // Drain stdout/stderr concurrently (so a chatty command can't deadlock
        // against a full pipe buffer while we poll for exit), delivering each
        // buffer over a channel. Using a channel + `recv_timeout` rather than a
        // blocking `join` is what makes the timeout *actually* bounded: if a
        // killed process leaves an orphaned grandchild still holding a pipe
        // write-end, the reader thread's `read_to_end` won't hit EOF, but we
        // simply stop waiting for it after a short grace period instead of
        // hanging on its join.
        let (tx, rx) = std::sync::mpsc::channel::<(bool, Vec<u8>)>();
        {
            let tx_out = tx.clone();
            let mut s = child.stdout.take();
            thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(ref mut r) = s {
                    let _ = r.read_to_end(&mut buf);
                }
                let _ = tx_out.send((true, buf));
            });
            let mut s = child.stderr.take();
            thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(ref mut r) = s {
                    let _ = r.read_to_end(&mut buf);
                }
                let _ = tx.send((false, buf));
            });
        }

        let deadline = Instant::now() + timeout;
        let mut timed_out = false;
        let exit_code = loop {
            match child.try_wait().map_err(SandboxError::Io)? {
                Some(status) => break status.code().unwrap_or(-1),
                None => {
                    if Instant::now() >= deadline {
                        // Best-effort kill of the whole process group so
                        // grandchildren die too (see the `process_group(0)`
                        // note above). A negative pid targets the group whose
                        // pgid == the child's pid.
                        #[cfg(unix)]
                        {
                            let _ = Command::new("kill")
                                .arg("-KILL")
                                .arg(format!("-{}", child.id()))
                                .output();
                        }
                        let _ = child.kill();
                        let _ = child.wait();
                        timed_out = true;
                        break -1;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
        };

        // Collect the two reader buffers. On a clean exit the child is gone so
        // both arrive at once; on a timeout we allow only a short grace period
        // so a still-lingering pipe writer can never make this call hang.
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let recv_deadline = Instant::now()
            + if timed_out {
                Duration::from_millis(500)
            } else {
                Duration::from_secs(10)
            };
        for _ in 0..2 {
            let remaining = recv_deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining) {
                Ok((true, b)) => stdout_buf = b,
                Ok((false, b)) => stderr_buf = b,
                Err(_) => break,
            }
        }
        let stdout = String::from_utf8_lossy(&stdout_buf).into_owned();
        let mut stderr = String::from_utf8_lossy(&stderr_buf).into_owned();
        if timed_out {
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "[command timed out after {}s and was killed]",
                timeout.as_secs()
            ));
        }
        Ok(ToolResult::TerminalOutput {
            stdout,
            stderr,
            exit_code,
        })
    }

    /// Real, bounded recursive substring search under `path` (or the
    /// whole real project root when `None`) -- directly closes the gap
    /// named on `ToolCall::SearchFiles` itself. Two real, named bounds
    /// prevent a pathological repo (a huge `node_modules`/build output
    /// that somehow isn't excluded) from hanging the agent loop: at most
    /// `SEARCH_MAX_MATCHES` results and at most `SEARCH_MAX_FILES_VISITED`
    /// real directory entries visited. A handful of common, real,
    /// noise-only directories are skipped outright rather than searched
    /// and immediately discarded. Files that fail real UTF-8 decoding
    /// (binaries) are silently skipped, not an error -- a real, expected,
    /// common case, not a failure.
    pub fn search_files(
        &self,
        pattern: &str,
        path: Option<&str>,
    ) -> Result<ToolResult, SandboxError> {
        const SKIP_DIRS: &[&str] = &[
            ".git",
            "node_modules",
            "target",
            "dist",
            "build",
            ".next",
            ".venv",
            "venv",
            "__pycache__",
        ];
        const MAX_MATCHES: usize = 200;
        const MAX_FILES_VISITED: usize = 20_000;

        let canonical_root = self.project_root.canonicalize().map_err(SandboxError::Io)?;
        let start = match path {
            Some(p) => self.resolve(p)?,
            None => canonical_root.clone(),
        };

        let mut matches = Vec::new();
        let mut visited = 0usize;
        let mut stack = vec![start];
        'walk: while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                visited += 1;
                if visited >= MAX_FILES_VISITED || matches.len() >= MAX_MATCHES {
                    break 'walk;
                }
                let entry_path = entry.path();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    let name = entry.file_name();
                    if SKIP_DIRS.contains(&name.to_string_lossy().as_ref()) {
                        continue;
                    }
                    stack.push(entry_path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&entry_path) else {
                    continue;
                };
                let rel = entry_path
                    .strip_prefix(&canonical_root)
                    .unwrap_or(&entry_path)
                    .to_string_lossy()
                    .to_string();
                for (i, line) in content.lines().enumerate() {
                    if line.contains(pattern) {
                        matches.push(SearchMatch {
                            path: rel.clone(),
                            line: i + 1,
                            text: line.trim().chars().take(200).collect(),
                        });
                        if matches.len() >= MAX_MATCHES {
                            break;
                        }
                    }
                }
            }
        }
        Ok(ToolResult::SearchMatches(matches))
    }

    /// Real, non-recursive directory listing, mirroring `spartan-backend`
    /// own `list_dir` IPC method's real sort convention (dirs first, then
    /// alphabetical) -- kept as a separate, sandboxed implementation
    /// rather than sharing code with that method, since this one must go
    /// through `resolve()`'s real path-jail enforcement and that one is
    /// already trusted (it only ever serves the real, already-open
    /// project's own file tree).
    pub fn list_directory(&self, path: Option<&str>) -> Result<ToolResult, SandboxError> {
        let resolved = match path {
            Some(p) => self.resolve(p)?,
            None => self.project_root.canonicalize().map_err(SandboxError::Io)?,
        };
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&resolved).map_err(SandboxError::Io)? {
            let entry = entry.map_err(SandboxError::Io)?;
            let file_type = entry.file_type().map_err(SandboxError::Io)?;
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: file_type.is_dir(),
            });
        }
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(ToolResult::DirectoryListing(entries))
    }

    pub fn execute(&self, call: &ToolCall) -> Result<ToolResult, SandboxError> {
        match call {
            ToolCall::ReadFile { path } => self.read_file(path),
            ToolCall::EditFile { path, content } => self.edit_file(path, content),
            ToolCall::RunTerminal { command } => self.run_terminal(command),
            ToolCall::SearchFiles { pattern, path } => self.search_files(pattern, path.as_deref()),
            ToolCall::ListDirectory { path } => self.list_directory(path.as_deref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real, live bug found by running these tests, not by inspection:
    /// an earlier version keyed this directory only by `std::process::id()`
    /// -- but every test in this file runs as a thread *within the same
    /// process*, and `cargo test` runs them concurrently by default, so
    /// every test was silently sharing (and racing on) the exact same
    /// directory. Keyed by `name` instead, one unique value per call site.
    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spartan-leo-sandbox-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_a_real_file_inside_the_jail() {
        let root = temp_project("read");
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let sandbox = Sandbox::new(&root);
        let result = sandbox.read_file("a.txt").unwrap();
        assert_eq!(result, ToolResult::FileContent("hello".to_string()));
    }

    #[test]
    fn writes_a_real_new_file_inside_the_jail() {
        let root = temp_project("write");
        let sandbox = Sandbox::new(&root);
        sandbox.edit_file("sub/new.txt", "content").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("sub/new.txt")).unwrap(),
            "content"
        );
    }

    #[test]
    fn refuses_a_real_parent_traversal_escape() {
        let root = temp_project("traversal");
        let sandbox = Sandbox::new(&root);
        let result = sandbox.read_file("../../../etc/passwd");
        assert!(matches!(result, Err(SandboxError::PathEscapesJail { .. })));
    }

    #[test]
    fn refuses_a_real_absolute_path_override_outside_the_root() {
        let root = temp_project("absolute");
        let sandbox = Sandbox::new(&root);
        let result = sandbox.read_file("/etc/passwd");
        assert!(matches!(result, Err(SandboxError::PathEscapesJail { .. })));
    }

    #[test]
    fn refuses_a_real_symlink_escape() {
        let root = temp_project("symlink-root");
        let outside = temp_project("symlink-outside");
        std::fs::write(outside.join("secret.txt"), "top secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(&outside, root.join("escape_link")).unwrap();
        }
        #[cfg(unix)]
        {
            let sandbox = Sandbox::new(&root);
            let result = sandbox.read_file("escape_link/secret.txt");
            assert!(matches!(result, Err(SandboxError::PathEscapesJail { .. })));
        }
    }

    #[test]
    fn allows_a_real_absolute_path_that_is_actually_inside_the_root() {
        let root = temp_project("abs-inside");
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let sandbox = Sandbox::new(&root);
        let canonical_root = root.canonicalize().unwrap();
        let abs_path_inside = canonical_root.join("a.txt");
        let result = sandbox.read_file(abs_path_inside.to_str().unwrap());
        assert_eq!(
            result.unwrap(),
            ToolResult::FileContent("hello".to_string())
        );
    }

    #[test]
    fn runs_a_real_terminal_command_inside_the_jail() {
        let root = temp_project("terminal");
        std::fs::write(root.join("marker.txt"), "x").unwrap();
        let sandbox = Sandbox::new(&root);
        let result = sandbox.run_terminal("ls").unwrap();
        let ToolResult::TerminalOutput {
            stdout, exit_code, ..
        } = result
        else {
            panic!("expected TerminalOutput");
        };
        assert_eq!(exit_code, 0);
        assert!(stdout.contains("marker.txt"));
    }

    #[test]
    fn run_terminal_kills_a_command_that_exceeds_the_timeout() {
        let root = temp_project("terminal-timeout");
        let sandbox = Sandbox::new(&root);
        let start = Instant::now();
        // `sleep 30` would run far past the bound; a short timeout must kill it.
        let result = sandbox
            .run_terminal_with_timeout("sleep 30", Duration::from_millis(300))
            .unwrap();
        let elapsed = start.elapsed();
        // Must return quickly (killed near the deadline), not wait 30s.
        assert!(
            elapsed < Duration::from_secs(5),
            "timed-out command should return promptly, took {elapsed:?}"
        );
        let ToolResult::TerminalOutput {
            stderr, exit_code, ..
        } = result
        else {
            panic!("expected TerminalOutput");
        };
        assert_eq!(exit_code, -1);
        assert!(
            stderr.contains("timed out"),
            "stderr should note the timeout: {stderr:?}"
        );
    }

    #[test]
    fn run_terminal_gives_stdin_eof_so_a_reader_does_not_hang() {
        let root = temp_project("terminal-stdin");
        let sandbox = Sandbox::new(&root);
        let start = Instant::now();
        // `cat` with no args reads stdin forever on a TTY; with stdin=null it
        // hits immediate EOF and exits 0, well within the timeout.
        let result = sandbox
            .run_terminal_with_timeout("cat", Duration::from_secs(5))
            .unwrap();
        assert!(start.elapsed() < Duration::from_secs(3));
        let ToolResult::TerminalOutput { exit_code, .. } = result else {
            panic!("expected TerminalOutput");
        };
        assert_eq!(exit_code, 0, "cat with stdin=null should exit cleanly");
    }

    #[test]
    fn run_terminal_captures_large_output_without_deadlock() {
        let root = temp_project("terminal-bigout");
        let sandbox = Sandbox::new(&root);
        // ~200 KB of output would overflow a pipe buffer if not drained
        // concurrently; the reader threads must prevent a deadlock.
        let result = sandbox
            .run_terminal_with_timeout(
                "for i in $(seq 1 5000); do echo 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'; done",
                Duration::from_secs(10),
            )
            .unwrap();
        let ToolResult::TerminalOutput {
            stdout, exit_code, ..
        } = result
        else {
            panic!("expected TerminalOutput");
        };
        assert_eq!(exit_code, 0);
        assert_eq!(stdout.lines().count(), 5000);
    }

    #[test]
    fn search_files_finds_a_real_substring_across_multiple_real_files() {
        let root = temp_project("search-basic");
        std::fs::write(root.join("a.rs"), "fn foo() {}\nfn bar() {}\n").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/b.rs"), "// calls foo() here\n").unwrap();
        let sandbox = Sandbox::new(&root);
        let result = sandbox.search_files("foo", None).unwrap();
        let ToolResult::SearchMatches(matches) = result else {
            panic!("expected SearchMatches");
        };
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().any(|m| m.path == "a.rs" && m.line == 1));
        assert!(matches
            .iter()
            .any(|m| m.path.ends_with("sub/b.rs") && m.line == 1));
    }

    #[test]
    fn search_files_skips_real_noise_directories() {
        let root = temp_project("search-skip-dirs");
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(root.join("node_modules/dep.js"), "needle").unwrap();
        std::fs::write(root.join("real.js"), "needle").unwrap();
        let sandbox = Sandbox::new(&root);
        let result = sandbox.search_files("needle", None).unwrap();
        let ToolResult::SearchMatches(matches) = result else {
            panic!("expected SearchMatches");
        };
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "real.js");
    }

    #[test]
    fn search_files_scoped_to_a_real_subdirectory_only_searches_it() {
        let root = temp_project("search-scoped");
        std::fs::write(root.join("outside.txt"), "needle").unwrap();
        std::fs::create_dir_all(root.join("inside")).unwrap();
        std::fs::write(root.join("inside/hit.txt"), "needle").unwrap();
        let sandbox = Sandbox::new(&root);
        let result = sandbox.search_files("needle", Some("inside")).unwrap();
        let ToolResult::SearchMatches(matches) = result else {
            panic!("expected SearchMatches");
        };
        assert_eq!(matches.len(), 1);
        assert!(matches[0].path.ends_with("inside/hit.txt"));
    }

    #[test]
    fn search_files_refuses_a_real_path_jail_escape() {
        let root = temp_project("search-jail");
        let sandbox = Sandbox::new(&root);
        let result = sandbox.search_files("x", Some("../../../etc"));
        assert!(matches!(result, Err(SandboxError::PathEscapesJail { .. })));
    }

    #[test]
    fn list_directory_with_no_path_lists_the_real_project_root() {
        let root = temp_project("list-root");
        std::fs::write(root.join("a.txt"), "x").unwrap();
        std::fs::create_dir_all(root.join("zsub")).unwrap();
        let sandbox = Sandbox::new(&root);
        let result = sandbox.list_directory(None).unwrap();
        let ToolResult::DirectoryListing(entries) = result else {
            panic!("expected DirectoryListing");
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "zsub");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "a.txt");
        assert!(!entries[1].is_dir);
    }

    #[test]
    fn list_directory_refuses_a_real_path_jail_escape() {
        let root = temp_project("list-jail");
        let sandbox = Sandbox::new(&root);
        let result = sandbox.list_directory(Some("../../../etc"));
        assert!(matches!(result, Err(SandboxError::PathEscapesJail { .. })));
    }
}
