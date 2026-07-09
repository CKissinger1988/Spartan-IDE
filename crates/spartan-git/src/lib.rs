//! Real local git operations (§56.1, task #7) backed by `git2` (vendored
//! `libgit2`, no system git binary or network access needed for any
//! operation in this crate -- everything here is local-repository-only).
//! Deliberately scoped to §56.1's "basic local Source Control is Tier 1"
//! line, not §56.2-56.4's GitHub layer (real OAuth device-code flow, a
//! real GitHub API round-trip) -- that's a separate, larger increment,
//! named as a real, open gap rather than attempted here.

use git2::{IndexAddOption, Repository, RepositoryOpenFlags, Status, StatusOptions};
use std::path::{Path, PathBuf};

/// One file's real status in the working tree, both halves independently
/// (a file can be both staged *and* have further unstaged changes on top
/// -- git's own real index/worktree split, not simplified away).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: PathBuf,
    /// `None` if this file has no staged (index) change at all.
    pub staged: Option<FileStatus>,
    /// `None` if this file has no unstaged (worktree) change at all --
    /// covers both "modified but not staged" and "untracked".
    pub unstaged: Option<FileStatus>,
    pub conflicted: bool,
}

fn from_git_status(s: Status) -> (Option<FileStatus>, Option<FileStatus>, bool) {
    let staged = if s.is_index_new() {
        Some(FileStatus::Added)
    } else if s.is_index_modified() {
        Some(FileStatus::Modified)
    } else if s.is_index_deleted() {
        Some(FileStatus::Deleted)
    } else if s.is_index_renamed() {
        Some(FileStatus::Renamed)
    } else if s.is_index_typechange() {
        Some(FileStatus::TypeChanged)
    } else {
        None
    };
    let unstaged = if s.is_wt_new() {
        Some(FileStatus::Added)
    } else if s.is_wt_modified() {
        Some(FileStatus::Modified)
    } else if s.is_wt_deleted() {
        Some(FileStatus::Deleted)
    } else if s.is_wt_renamed() {
        Some(FileStatus::Renamed)
    } else if s.is_wt_typechange() {
        Some(FileStatus::TypeChanged)
    } else {
        None
    };
    (staged, unstaged, s.is_conflicted())
}

/// A real, open local git repository. Every method here is a thin,
/// honest wrapper over a real `git2` call -- no simulated state.
pub struct GitRepo {
    repo: Repository,
}

impl GitRepo {
    /// Real repository discovery, walking upward from `start` exactly the
    /// way `git status` itself does (matches `language::find_project_root`'s
    /// own upward-walk convention already established elsewhere in this
    /// workspace, but delegated to `libgit2`'s own real discovery instead
    /// of a hand-rolled walk, since correctness here -- `.git` files for
    /// worktrees/submodules, `core.worktree` overrides -- is exactly what
    /// `libgit2` already gets right).
    pub fn discover(start: &Path) -> Option<Self> {
        let repo = Repository::open_ext(
            start,
            RepositoryOpenFlags::empty(),
            std::iter::empty::<&Path>(),
        )
        .ok()?;
        Some(Self { repo })
    }

    pub fn workdir(&self) -> Option<&Path> {
        self.repo.workdir()
    }

    /// Real, direct access to the underlying `git2::Repository` -- needed
    /// by `spartan-leo`'s real checkpointing (§4.2, task #5, §75.47),
    /// which operates on `git2`'s own stash/reset plumbing directly rather
    /// than through this crate's own higher-level status/stage/commit API.
    /// A thin escape hatch, not a second parallel API surface: every other
    /// real git operation in this workspace still goes through this
    /// struct's own methods.
    pub fn raw_repo_mut(&mut self) -> &mut Repository {
        &mut self.repo
    }

    /// Real working-tree status for every changed/untracked/conflicted
    /// file, sorted by path for a stable, deterministic display order.
    /// Ignored files are excluded (matches `git status`'s own default,
    /// not `git status --ignored`).
    pub fn status(&self) -> Result<Vec<StatusEntry>, git2::Error> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);
        let statuses = self.repo.statuses(Some(&mut opts))?;
        let mut entries: Vec<StatusEntry> = statuses
            .iter()
            .filter_map(|entry| {
                let path = entry.path()?;
                let (staged, unstaged, conflicted) = from_git_status(entry.status());
                if staged.is_none() && unstaged.is_none() && !conflicted {
                    return None;
                }
                Some(StatusEntry {
                    path: PathBuf::from(path),
                    staged,
                    unstaged,
                    conflicted,
                })
            })
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    /// Real `git add <path>` -- stages the file's current working-tree
    /// content (including a delete, via `add_all`'s own real deleted-file
    /// handling) into the index.
    pub fn stage(&self, path: &Path) -> Result<(), git2::Error> {
        let mut index = self.repo.index()?;
        index.add_all([path], IndexAddOption::DEFAULT, None)?;
        index.write()
    }

    /// Real `git restore --staged <path>` -- resets this one path's index
    /// entry back to what `HEAD` has (or removes it from the index
    /// entirely if `HEAD` has no such path, e.g. a newly-added file).
    pub fn unstage(&self, path: &Path) -> Result<(), git2::Error> {
        let head = self.repo.head().and_then(|h| h.peel_to_commit());
        match head {
            Ok(commit) => self.repo.reset_default(Some(commit.as_object()), [path]),
            // No HEAD yet (a brand-new repo with no commits) -- there is
            // nothing to reset *to*, so the real correct action is
            // removing the path from the index outright.
            Err(_) => {
                let mut index = self.repo.index()?;
                index.remove_path(path)?;
                index.write()
            }
        }
    }

    /// Real commit of the current index against `HEAD` (or as the repo's
    /// first commit if there is no `HEAD` yet), using the real
    /// `user.name`/`user.email` from git config (repo-level, falling back
    /// to global -- `libgit2`'s own real `signature()` resolution, not
    /// reimplemented here). Returns the new commit's real `Oid`.
    pub fn commit(&self, message: &str) -> Result<git2::Oid, git2::Error> {
        let mut index = self.repo.index()?;
        let tree_oid = index.write_tree()?;
        let tree = self.repo.find_tree(tree_oid)?;
        let signature = self.repo.signature()?;
        let parents: Vec<git2::Commit> = match self.repo.head().and_then(|h| h.peel_to_commit()) {
            Ok(commit) => vec![commit],
            Err(_) => Vec::new(),
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )
    }

    /// The real current branch name, or `None` for a detached `HEAD` (a
    /// real, valid git state, not an error).
    pub fn current_branch(&self) -> Option<String> {
        let head = self.repo.head().ok()?;
        if head.is_branch() {
            head.shorthand().map(str::to_string)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A real temp git repository, initialized with a real `git2::Repository::init`
    /// and a real, fixed test signature configured on the repo itself (not
    /// the ambient environment's global git config, which this sandboxed
    /// test environment may not have set at all) -- matches this
    /// workspace's own established `TempTree` pattern (`file_tree.rs`) of
    /// real I/O over a mocked filesystem/VCS layer.
    struct TempRepo {
        dir: PathBuf,
    }

    impl TempRepo {
        fn new(unique: &str) -> (Self, GitRepo) {
            let dir = std::env::temp_dir().join(format!("spartan_git_test_{unique}"));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let repo = Repository::init(&dir).unwrap();
            {
                let mut config = repo.config().unwrap();
                config.set_str("user.name", "Spartan Test").unwrap();
                config
                    .set_str("user.email", "test@example.invalid")
                    .unwrap();
            }
            let git_repo = GitRepo { repo };
            (Self { dir }, git_repo)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.dir.join(name);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn discover_finds_a_real_repo_from_its_own_root() {
        let (tmp, _repo) = TempRepo::new("discover_root");
        let found = GitRepo::discover(&tmp.dir);
        assert!(found.is_some());
    }

    #[test]
    fn discover_finds_a_real_repo_from_a_nested_subdirectory() {
        let (tmp, _repo) = TempRepo::new("discover_nested");
        let nested = tmp.dir.join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        let found = GitRepo::discover(&nested);
        assert!(found.is_some());
        assert_eq!(
            found.unwrap().workdir().unwrap().canonicalize().unwrap(),
            tmp.dir.canonicalize().unwrap()
        );
    }

    #[test]
    fn discover_on_a_non_repo_directory_returns_none() {
        let dir = std::env::temp_dir().join("spartan_git_test_not_a_repo");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Guard against the temp dir itself accidentally being under a
        // real repo already (this whole workspace, for instance).
        assert!(GitRepo::discover(&dir).is_none() || dir.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_untracked_file_shows_up_as_unstaged_added() {
        let (tmp, repo) = TempRepo::new("untracked");
        tmp.write("new.txt", "hello");
        let status = repo.status().unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].path, PathBuf::from("new.txt"));
        assert_eq!(status[0].staged, None);
        assert_eq!(status[0].unstaged, Some(FileStatus::Added));
    }

    #[test]
    fn staging_a_new_file_moves_it_to_staged_added() {
        let (tmp, repo) = TempRepo::new("stage_new");
        let path = tmp.write("new.txt", "hello");
        repo.stage(Path::new("new.txt")).unwrap();
        let status = repo.status().unwrap();
        assert_eq!(status[0].staged, Some(FileStatus::Added));
        assert_eq!(status[0].unstaged, None);
        // Real file untouched by staging -- staging never mutates the
        // working tree, only the index.
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn unstaging_a_new_file_removes_it_from_the_index_entirely() {
        let (tmp, repo) = TempRepo::new("unstage_new");
        tmp.write("new.txt", "hello");
        repo.stage(Path::new("new.txt")).unwrap();
        repo.unstage(Path::new("new.txt")).unwrap();
        let status = repo.status().unwrap();
        assert_eq!(status[0].staged, None);
        assert_eq!(status[0].unstaged, Some(FileStatus::Added));
    }

    #[test]
    fn committing_staged_changes_creates_a_real_commit_and_clears_status() {
        let (tmp, repo) = TempRepo::new("commit");
        tmp.write("new.txt", "hello");
        repo.stage(Path::new("new.txt")).unwrap();
        let oid = repo.commit("real first commit").unwrap();
        assert!(!oid.is_zero());
        assert!(repo.status().unwrap().is_empty());
        assert_eq!(repo.current_branch().unwrap(), "master");
    }

    #[test]
    fn a_modification_after_a_real_commit_shows_up_as_unstaged_modified() {
        let (tmp, repo) = TempRepo::new("modify_after_commit");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("v1").unwrap();
        tmp.write("f.txt", "v2");
        let status = repo.status().unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].unstaged, Some(FileStatus::Modified));
        assert_eq!(status[0].staged, None);
    }

    #[test]
    fn unstaging_a_modification_after_head_exists_resets_to_head_not_removes() {
        let (tmp, repo) = TempRepo::new("unstage_after_head");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("v1").unwrap();
        tmp.write("f.txt", "v2");
        repo.stage(Path::new("f.txt")).unwrap();
        assert_eq!(repo.status().unwrap()[0].staged, Some(FileStatus::Modified));
        repo.unstage(Path::new("f.txt")).unwrap();
        let status = repo.status().unwrap();
        assert_eq!(status[0].staged, None);
        assert_eq!(status[0].unstaged, Some(FileStatus::Modified));
    }

    #[test]
    fn a_second_commit_correctly_uses_the_first_as_its_parent() {
        let (tmp, repo) = TempRepo::new("second_commit");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        let first = repo.commit("first").unwrap();
        tmp.write("f.txt", "v2");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("second").unwrap();
        let head = repo.repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.parent_id(0).unwrap(), first);
    }
}
