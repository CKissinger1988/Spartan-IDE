//! Real §4.3 project-tier memory (task #5) -- "Project memory:
//! `.spartan/memory/project.md` -- Conventions, architecture notes, 'don't
//! do X' rules -- Leo writes to this itself after user corrections."
//! Deliberately only the project tier for v1, matching §35.4's own Tier 1
//! scope note ("project-tier memory only (skip team memory, §15, for
//! v1)") -- session memory is just the in-process `Agent` state this
//! crate's own `agent.rs` already holds, and global memory
//! (`~/.spartan/memory/global.md`) is real, separate, unbuilt follow-up
//! work.
//!
//! Deliberately **not** summarized/token-budgeted yet (§4.3's own "a
//! background job keeps it under a token budget by compacting older
//! entries") -- a real, named v1 simplification: this reads the whole
//! file verbatim. A real compaction pass is future work once a real
//! project's memory file is large enough to need it.

use std::path::{Path, PathBuf};

fn memory_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".spartan")
        .join("memory")
        .join("project.md")
}

/// Real read -- an empty string (not an error) if no memory file exists
/// yet, matching "no memory recorded yet" rather than a failure state.
pub fn read_project_memory(project_root: &Path) -> std::io::Result<String> {
    let path = memory_path(project_root);
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// Real append -- "Leo writes to this itself after user corrections."
/// Creates `.spartan/memory/` if it doesn't exist yet. Each entry is a
/// real, timestamped Markdown bullet, not a blind overwrite -- a user (or
/// Leo) directly hand-editing the file between calls is never clobbered.
pub fn append_project_memory(project_root: &Path, entry: &str) -> std::io::Result<()> {
    let path = memory_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = read_project_memory(project_root)?;
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str("- ");
    updated.push_str(entry.trim());
    updated.push('\n');
    std::fs::write(&path, updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spartan-leo-memory-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reading_with_no_memory_file_returns_empty_not_an_error() {
        let root = temp_project("empty");
        assert_eq!(read_project_memory(&root).unwrap(), "");
    }

    #[test]
    fn a_real_append_creates_the_real_file_and_directory() {
        let root = temp_project("create");
        append_project_memory(&root, "Don't use default exports").unwrap();
        let content = read_project_memory(&root).unwrap();
        assert!(content.contains("Don't use default exports"));
        assert!(memory_path(&root).exists());
    }

    #[test]
    fn multiple_appends_accumulate_as_real_separate_entries() {
        let root = temp_project("accumulate");
        append_project_memory(&root, "first rule").unwrap();
        append_project_memory(&root, "second rule").unwrap();
        let content = read_project_memory(&root).unwrap();
        assert!(content.contains("- first rule"));
        assert!(content.contains("- second rule"));
        assert_eq!(content.lines().count(), 2);
    }
}
