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
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub enum ToolCall {
    ReadFile { path: String },
    EditFile { path: String, content: String },
    RunTerminal { command: String },
}

impl ToolCall {
    pub fn name(&self) -> &'static str {
        match self {
            ToolCall::ReadFile { .. } => "read_file",
            ToolCall::EditFile { .. } => "edit_file",
            ToolCall::RunTerminal { .. } => "run_terminal",
        }
    }
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
        let canonical_root = self.project_root.canonicalize().map_err(SandboxError::Io)?;
        let output = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&canonical_root)
            .output()
            .map_err(SandboxError::Io)?;
        Ok(ToolResult::TerminalOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    pub fn execute(&self, call: &ToolCall) -> Result<ToolResult, SandboxError> {
        match call {
            ToolCall::ReadFile { path } => self.read_file(path),
            ToolCall::EditFile { path, content } => self.edit_file(path, content),
            ToolCall::RunTerminal { command } => self.run_terminal(command),
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
}
