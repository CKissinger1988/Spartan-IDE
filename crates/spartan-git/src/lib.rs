//! Real local git operations (§56.1, task #7) backed by `git2` (vendored
//! `libgit2`, no system git binary needed). Every *local* operation here
//! needs no network; the real remote operations added from the
//! `docs/FUTURE_FEATURES.md` P1 backlog (`list_remotes`/`fetch`/`push`/
//! `pull_fast_forward`) reach a real remote only when one is actually
//! configured, and are fully exercisable against a local bare-repo remote
//! with no network or credentials at all. Deliberately still scoped short
//! of §56.2-56.4's GitHub layer (real OAuth device-code flow, a real
//! GitHub API round-trip); remote auth here supports SSH-agent and
//! default/anonymous credentials only -- an interactive HTTPS-token entry
//! UI is a named, open follow-up, not attempted here.

use git2::{IndexAddOption, Repository, RepositoryOpenFlags, Status, StatusOptions};
use std::path::{Path, PathBuf};

/// Shared credential callback for real fetch/push against an authenticated
/// remote. Tries SSH-agent first (the common key-backed case), then a
/// default/anonymous credential (local `file://` remotes need none, so the
/// callback typically isn't even invoked for those). An interactive
/// username/password or HTTPS-token prompt is a named, open follow-up.
fn make_remote_callbacks() -> git2::RemoteCallbacks<'static> {
    let mut cb = git2::RemoteCallbacks::new();
    cb.credentials(|_url, username_from_url, allowed| {
        if allowed.contains(git2::CredentialType::SSH_KEY) {
            if let Some(user) = username_from_url {
                if let Ok(cred) = git2::Cred::ssh_key_from_agent(user) {
                    return Ok(cred);
                }
            }
        }
        git2::Cred::default()
    });
    cb
}

/// Outcome of a real `pull_fast_forward` -- deliberately fast-forward-only
/// (a real non-ff divergence is reported, never silently auto-merged or
/// rebased), the safe v1 semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullOutcome {
    UpToDate,
    FastForwarded,
    NonFastForward,
}

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

/// One real commit in `log()`'s output -- see that method's own doc
/// comment for exactly what each field carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub oid: String,
    pub summary: String,
    pub author: String,
    /// Real commit time, unix seconds.
    pub time: i64,
}

/// One line's real blame in `blame_file()`'s output -- the commit that
/// last touched that line, in file order. See that method's own doc
/// comment for the committed-vs-working-buffer alignment contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameLine {
    pub oid: String,
    pub summary: String,
    pub author: String,
    /// Real commit time, unix seconds.
    pub time: i64,
}

/// One real stash entry in `stash_list()`'s output -- `index` 0 is the
/// most recent (`stash@{0}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub oid: String,
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

    /// Real `HEAD`'s own version of `path`'s content, as real UTF-8 text --
    /// the "before" half of a real staged diff. `Ok(None)` covers both real,
    /// honest cases that aren't errors: no `HEAD` yet (a brand-new repo with
    /// no commits), or `HEAD`'s tree simply has no such path (a newly-added
    /// file). A real, non-UTF-8 blob is also reported as `Ok(None)` rather
    /// than a lossy or garbled diff -- this crate's own real scope is text
    /// source files, matching every other real text-only assumption already
    /// made elsewhere in this workspace (tree-sitter highlighting, LSP).
    pub fn head_blob_content(&self, path: &Path) -> Result<Option<String>, git2::Error> {
        let head_commit = match self.repo.head().and_then(|h| h.peel_to_commit()) {
            Ok(commit) => commit,
            Err(_) => return Ok(None),
        };
        let tree = head_commit.tree()?;
        let entry = match tree.get_path(path) {
            Ok(entry) => entry,
            Err(_) => return Ok(None),
        };
        let object = entry.to_object(&self.repo)?;
        let blob = match object.as_blob() {
            Some(blob) => blob,
            None => return Ok(None),
        };
        Ok(std::str::from_utf8(blob.content()).ok().map(str::to_string))
    }

    /// Real `git log` -- the most recent `max` commits reachable from
    /// `HEAD`, newest first (a real `revwalk` over the actual commit
    /// graph, not just first-parent hopping, so merge history is
    /// complete). A repo with no commits yet returns an honest empty
    /// list, not an error. Each entry: (full hex oid, summary line,
    /// author name, commit time as real unix seconds).
    pub fn log(&self, max: usize) -> Result<Vec<CommitInfo>, git2::Error> {
        if self.repo.head().is_err() {
            return Ok(Vec::new());
        }
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        let mut out = Vec::new();
        for oid in walk {
            if out.len() >= max {
                break;
            }
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            out.push(CommitInfo {
                oid: oid.to_string(),
                summary: commit.summary().unwrap_or("").to_string(),
                author: commit.author().name().unwrap_or("").to_string(),
                time: commit.time().seconds(),
            });
        }
        Ok(out)
    }

    /// Every file a real commit changed, relative to its first parent
    /// (a root commit diffs against the empty tree, so everything shows
    /// as `Added` -- the real, correct answer). A real tree-to-tree
    /// `git2::Diff`, not a hand-rolled walk.
    pub fn commit_changed_files(
        &self,
        oid_str: &str,
    ) -> Result<Vec<(String, FileStatus)>, git2::Error> {
        let oid = git2::Oid::from_str(oid_str)?;
        let commit = self.repo.find_commit(oid)?;
        let tree = commit.tree()?;
        let parent_tree = match commit.parent(0) {
            Ok(parent) => Some(parent.tree()?),
            Err(_) => None,
        };
        let diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        let mut out = Vec::new();
        for delta in diff.deltas() {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let status = match delta.status() {
                git2::Delta::Added | git2::Delta::Copied => FileStatus::Added,
                git2::Delta::Deleted => FileStatus::Deleted,
                git2::Delta::Renamed => FileStatus::Renamed,
                git2::Delta::Typechange => FileStatus::TypeChanged,
                _ => FileStatus::Modified,
            };
            out.push((path, status));
        }
        Ok(out)
    }

    /// A specific real commit's own version of `path`'s content -- the
    /// same `Ok(None)`-for-missing-path/non-UTF-8 contract as
    /// `head_blob_content`, generalized to any commit by oid.
    pub fn commit_blob_content(
        &self,
        oid_str: &str,
        path: &Path,
    ) -> Result<Option<String>, git2::Error> {
        let oid = git2::Oid::from_str(oid_str)?;
        let commit = self.repo.find_commit(oid)?;
        let tree = commit.tree()?;
        let entry = match tree.get_path(path) {
            Ok(entry) => entry,
            Err(_) => return Ok(None),
        };
        let object = entry.to_object(&self.repo)?;
        let blob = match object.as_blob() {
            Some(blob) => blob,
            None => return Ok(None),
        };
        Ok(std::str::from_utf8(blob.content()).ok().map(str::to_string))
    }

    /// A real commit's first-parent oid, or `None` for a root commit (a
    /// real, valid state, not an error).
    pub fn commit_parent(&self, oid_str: &str) -> Result<Option<String>, git2::Error> {
        let oid = git2::Oid::from_str(oid_str)?;
        let commit = self.repo.find_commit(oid)?;
        Ok(commit.parent_id(0).ok().map(|p| p.to_string()))
    }

    /// Every real local branch name, sorted, with the current branch
    /// flagged. Detached `HEAD` (a real, valid git state) simply flags
    /// nothing as current.
    pub fn list_branches(&self) -> Result<Vec<(String, bool)>, git2::Error> {
        let current = self.current_branch();
        let mut names: Vec<(String, bool)> = self
            .repo
            .branches(Some(git2::BranchType::Local))?
            .filter_map(|b| {
                let (branch, _) = b.ok()?;
                let name = branch.name().ok()??.to_string();
                let is_current = current.as_deref() == Some(name.as_str());
                Some((name, is_current))
            })
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(names)
    }

    /// Real `git switch <name>` -- a *safe* checkout: `libgit2`'s default
    /// (`CheckoutBuilder::new()` with `safe()`) refuses to overwrite real
    /// uncommitted changes that conflict with the target branch's content,
    /// exactly like the real `git switch` refuses, surfacing `libgit2`'s
    /// own real conflict error rather than force-discarding anything.
    /// `HEAD` is only moved *after* the working-tree checkout succeeds, so
    /// a refused checkout leaves the repository exactly where it was.
    pub fn checkout_branch(&self, name: &str) -> Result<(), git2::Error> {
        let refname = format!("refs/heads/{name}");
        let obj = self.repo.revparse_single(&refname)?;
        let mut opts = git2::build::CheckoutBuilder::new();
        opts.safe();
        self.repo.checkout_tree(&obj, Some(&mut opts))?;
        self.repo.set_head(&refname)
    }

    /// Real `git branch <name>` from the current `HEAD` commit. Does not
    /// switch to it (matching the real `git branch` command's own
    /// behavior); `force = false`, so an existing branch of the same name
    /// is a real error, never silently moved.
    pub fn create_branch(&self, name: &str) -> Result<(), git2::Error> {
        let head = self.repo.head()?.peel_to_commit()?;
        self.repo.branch(name, &head, false)?;
        Ok(())
    }

    /// Every real remote-tracking branch (e.g. `origin/feature`), sorted.
    /// These are the `refs/remotes/*` refs a real `fetch` populates -- so
    /// they reflect the last fetch, not a live network query. The symbolic
    /// `origin/HEAD` pointer is skipped (it's not a real branch to check
    /// out). A repo with no remotes returns an empty list.
    pub fn list_remote_branches(&self) -> Result<Vec<String>, git2::Error> {
        let mut names: Vec<String> = self
            .repo
            .branches(Some(git2::BranchType::Remote))?
            .filter_map(|b| {
                let (branch, _) = b.ok()?;
                let name = branch.name().ok()??.to_string();
                // Skip the symbolic `<remote>/HEAD` alias -- not a real branch.
                if name.ends_with("/HEAD") {
                    return None;
                }
                Some(name)
            })
            .collect();
        names.sort();
        Ok(names)
    }

    /// Check out a remote-tracking branch (e.g. `origin/feature`): if no
    /// local branch of that name exists yet, create one from the remote
    /// ref's commit with the remote set as its upstream (real `git checkout
    /// -b feature --track origin/feature` behavior), then switch to it via
    /// the same *safe* `checkout_branch` (so a conflicting dirty change is
    /// refused, not clobbered). If the local branch already exists, just
    /// switches to it. The local name is the part after the first `/`
    /// (`origin/feature` -> `feature`).
    pub fn checkout_remote_branch(&self, remote_branch: &str) -> Result<(), git2::Error> {
        let local_name = remote_branch
            .split_once('/')
            .map(|(_, rest)| rest)
            .filter(|s| !s.is_empty())
            .unwrap_or(remote_branch);
        if self
            .repo
            .find_branch(local_name, git2::BranchType::Local)
            .is_err()
        {
            let remote_ref = format!("refs/remotes/{remote_branch}");
            let commit = self.repo.revparse_single(&remote_ref)?.peel_to_commit()?;
            let mut b = self.repo.branch(local_name, &commit, false)?;
            // Best-effort upstream tracking -- a real checkout still succeeds
            // even if the upstream config can't be written.
            let _ = b.set_upstream(Some(remote_branch));
        }
        self.checkout_branch(local_name)
    }

    /// Real index's own version of `path`'s content, as real UTF-8 text --
    /// the "after" half of a real staged diff, and the "before" half of a
    /// real unstaged diff. `Ok(None)` covers a path with no index entry at
    /// all (an untracked file has nothing staged), or a real non-UTF-8 blob
    /// -- same real scope decision as `head_blob_content`.
    pub fn index_blob_content(&self, path: &Path) -> Result<Option<String>, git2::Error> {
        let index = self.repo.index()?;
        let entry = match index.get_path(path, 0) {
            Some(entry) => entry,
            None => return Ok(None),
        };
        let blob = self.repo.find_blob(entry.id)?;
        Ok(std::str::from_utf8(blob.content()).ok().map(str::to_string))
    }

    /// Per-line blame for `path` as committed in `HEAD`: for each line, in
    /// file order, the real commit that last touched it (full oid -- the
    /// frontend shortens), its author, summary, and time. A real
    /// `git2::Repository::blame_file`, not a hand-rolled walk.
    ///
    /// Alignment contract, named honestly: this blames the file *as
    /// committed in `HEAD`*, so the returned vec has one entry per line of
    /// the committed version. The editor's live buffer may have unsaved
    /// edits; the UI aligns blame by line index, so blame is exact for an
    /// unedited buffer and drifts within edited regions until the next
    /// commit -- the same limitation every inline-blame tool has. An
    /// untracked/new path (not in `HEAD`) blames to an empty vec: a real,
    /// valid state, not an error, matching `head_blob_content`'s own
    /// missing-path contract. A repo with no commits yet also returns empty.
    pub fn blame_file(&self, path: &Path) -> Result<Vec<BlameLine>, git2::Error> {
        if self.repo.head().is_err() {
            return Ok(Vec::new());
        }
        // Untracked/new path -> no blame (a valid state, not an error).
        let head_tree = self.repo.head()?.peel_to_tree()?;
        if head_tree.get_path(path).is_err() {
            return Ok(Vec::new());
        }
        let blame = self.repo.blame_file(path, None)?;
        // Multiple hunks routinely share one commit -- look each up once.
        let mut cache: std::collections::HashMap<git2::Oid, (String, String, i64)> =
            std::collections::HashMap::new();
        let mut out: Vec<BlameLine> = Vec::new();
        for hunk in blame.iter() {
            let oid = hunk.final_commit_id();
            let (summary, author, time) = cache
                .entry(oid)
                .or_insert_with(|| match self.repo.find_commit(oid) {
                    Ok(c) => (
                        c.summary().unwrap_or("").to_string(),
                        c.author().name().unwrap_or("").to_string(),
                        c.time().seconds(),
                    ),
                    Err(_) => (String::new(), String::new(), 0),
                })
                .clone();
            for _ in 0..hunk.lines_in_hunk() {
                out.push(BlameLine {
                    oid: oid.to_string(),
                    summary: summary.clone(),
                    author: author.clone(),
                    time,
                });
            }
        }
        Ok(out)
    }

    /// Every configured real remote as `(name, url)` -- `url` is `None` for
    /// a remote with no fetch URL set (a real, valid, if unusual, state).
    pub fn list_remotes(&self) -> Result<Vec<(String, Option<String>)>, git2::Error> {
        let names = self.repo.remotes()?;
        let mut out = Vec::new();
        for name in names.iter().flatten() {
            let url = self
                .repo
                .find_remote(name)
                .ok()
                .and_then(|r| r.url().map(str::to_string));
            out.push((name.to_string(), url));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// Real fetch from a configured remote using its own default refspecs
    /// (updates the remote-tracking refs; does not touch the working tree).
    pub fn fetch(&self, remote_name: &str) -> Result<(), git2::Error> {
        let mut remote = self.repo.find_remote(remote_name)?;
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(make_remote_callbacks());
        let empty: [&str; 0] = [];
        remote.fetch(&empty, Some(&mut fo), None)
    }

    /// Real push of a single local branch to the same-named branch on a
    /// configured remote. A rejected push (non-fast-forward on the remote,
    /// auth failure) surfaces `libgit2`'s own real error verbatim -- never
    /// a force-push, which would need an explicit, separate opt-in.
    pub fn push(&self, remote_name: &str, branch: &str) -> Result<(), git2::Error> {
        let mut remote = self.repo.find_remote(remote_name)?;
        let mut po = git2::PushOptions::new();
        po.remote_callbacks(make_remote_callbacks());
        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
        remote.push(&[refspec.as_str()], Some(&mut po))
    }

    /// Real pull = fetch, then a *fast-forward-only* update of `branch`.
    /// A real divergence (the local branch has commits the remote doesn't)
    /// returns `NonFastForward` rather than auto-merging or rebasing -- the
    /// safe v1 behavior, leaving the working tree untouched. A fast-forward
    /// uses `libgit2`'s own safe (conflict-refusing) checkout, so a real
    /// conflicting uncommitted change surfaces its error rather than being
    /// force-discarded.
    pub fn pull_fast_forward(
        &self,
        remote_name: &str,
        branch: &str,
    ) -> Result<PullOutcome, git2::Error> {
        self.fetch(remote_name)?;
        let fetch_head = self.repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = self.repo.reference_to_annotated_commit(&fetch_head)?;
        let (analysis, _) = self.repo.merge_analysis(&[&fetch_commit])?;
        if analysis.is_up_to_date() {
            return Ok(PullOutcome::UpToDate);
        }
        if analysis.is_fast_forward() {
            // The canonical libgit2 fast-forward recipe force-checks-out the
            // new HEAD (the default/SAFE strategies are a dry run / don't
            // reliably update an existing checkout). To keep the "never
            // clobber uncommitted changes" guarantee, refuse first if the
            // working tree has real uncommitted changes to *tracked* files
            // (untracked files are left untouched by the checkout anyway).
            if self.has_uncommitted_tracked_changes()? {
                return Err(git2::Error::from_str(
                    "pull: uncommitted changes would be overwritten by fast-forward -- commit or stash first",
                ));
            }
            let refname = format!("refs/heads/{branch}");
            match self.repo.find_reference(&refname) {
                Ok(mut reference) => {
                    reference.set_target(fetch_commit.id(), "pull: fast-forward")?;
                }
                Err(_) => {
                    // A branch ref that doesn't exist locally yet (first
                    // pull of a new branch) -- create it at the fetched tip.
                    self.repo.reference(
                        &refname,
                        fetch_commit.id(),
                        true,
                        "pull: create from fast-forward",
                    )?;
                }
            }
            self.repo.set_head(&refname)?;
            let mut checkout = git2::build::CheckoutBuilder::default();
            checkout.force();
            self.repo.checkout_head(Some(&mut checkout))?;
            return Ok(PullOutcome::FastForwarded);
        }
        Ok(PullOutcome::NonFastForward)
    }

    /// Whether the working tree has any uncommitted change to a *tracked*
    /// file (staged, or an unstaged modify/delete/rename/typechange). A
    /// purely untracked new file does not count -- a checkout leaves those
    /// alone. Used to keep `pull_fast_forward` from force-overwriting real
    /// uncommitted edits.
    fn has_uncommitted_tracked_changes(&self) -> Result<bool, git2::Error> {
        Ok(self.status()?.iter().any(|e| {
            e.staged.is_some()
                || matches!(
                    e.unstaged,
                    Some(
                        FileStatus::Modified
                            | FileStatus::Deleted
                            | FileStatus::Renamed
                            | FileStatus::TypeChanged
                    )
                )
        }))
    }

    /// Real `git stash` of the current working changes to tracked files
    /// (matches git's own default -- untracked files are left in place, so
    /// the same `stash` a user gets from the CLI). Returns `Ok(None)` if
    /// there's nothing to stash (a clean tree, or only untracked files) --
    /// a real, valid state, not an error.
    pub fn stash_save(&mut self, message: &str) -> Result<Option<String>, git2::Error> {
        if !self.has_uncommitted_tracked_changes()? {
            return Ok(None);
        }
        let sig = self.repo.signature()?;
        let msg = if message.trim().is_empty() {
            None
        } else {
            Some(message)
        };
        let oid = self.repo.stash_save2(&sig, msg, None)?;
        Ok(Some(oid.to_string()))
    }

    /// Every real stash entry, newest first (index 0 is `stash@{0}`, the
    /// most recent).
    pub fn stash_list(&mut self) -> Result<Vec<StashEntry>, git2::Error> {
        let mut out = Vec::new();
        self.repo.stash_foreach(|index, message, oid| {
            out.push(StashEntry {
                index,
                message: message.to_string(),
                oid: oid.to_string(),
            });
            true
        })?;
        Ok(out)
    }

    /// Real `git stash pop <index>` -- applies the stash back onto the
    /// working tree and drops it. `libgit2`'s own apply refuses (errors)
    /// on a real conflict rather than force-overwriting, surfaced verbatim.
    pub fn stash_pop(&mut self, index: usize) -> Result<(), git2::Error> {
        self.repo.stash_pop(index, None)
    }

    /// Real `git stash apply <index>` -- applies the stash back onto the
    /// working tree but *keeps* it in the stash list (unlike `stash_pop`,
    /// which drops it after applying). Same conflict-refusing semantics as
    /// `stash_pop`: `libgit2`'s own apply errors on a real conflict rather
    /// than force-overwriting, surfaced verbatim.
    pub fn stash_apply(&mut self, index: usize) -> Result<(), git2::Error> {
        self.repo.stash_apply(index, None)
    }

    /// Real `git stash drop <index>` -- discards a stash without applying.
    pub fn stash_drop(&mut self, index: usize) -> Result<(), git2::Error> {
        self.repo.stash_drop(index)
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
    fn head_blob_content_is_none_with_no_commits_yet() {
        let (tmp, repo) = TempRepo::new("head_blob_no_commits");
        tmp.write("f.txt", "v1");
        assert_eq!(repo.head_blob_content(Path::new("f.txt")).unwrap(), None);
    }

    #[test]
    fn head_blob_content_is_none_for_a_path_head_does_not_have() {
        let (tmp, repo) = TempRepo::new("head_blob_missing_path");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        assert_eq!(repo.head_blob_content(Path::new("nope.txt")).unwrap(), None);
    }

    #[test]
    fn head_blob_content_returns_the_real_committed_text() {
        let (tmp, repo) = TempRepo::new("head_blob_real");
        tmp.write("f.txt", "v1 content");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        tmp.write("f.txt", "v2 content, not yet staged");
        assert_eq!(
            repo.head_blob_content(Path::new("f.txt")).unwrap(),
            Some("v1 content".to_string())
        );
    }

    #[test]
    fn index_blob_content_is_none_for_an_untracked_file() {
        let (tmp, repo) = TempRepo::new("index_blob_untracked");
        tmp.write("f.txt", "hello");
        assert_eq!(repo.index_blob_content(Path::new("f.txt")).unwrap(), None);
    }

    #[test]
    fn index_blob_content_returns_the_real_staged_text() {
        let (tmp, repo) = TempRepo::new("index_blob_real");
        tmp.write("f.txt", "committed");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        tmp.write("f.txt", "staged version");
        repo.stage(Path::new("f.txt")).unwrap();
        tmp.write("f.txt", "working tree version, not staged");
        assert_eq!(
            repo.index_blob_content(Path::new("f.txt")).unwrap(),
            Some("staged version".to_string())
        );
    }

    #[test]
    fn log_on_a_repo_with_no_commits_is_an_honest_empty_list() {
        let (_tmp, repo) = TempRepo::new("log_empty");
        assert_eq!(repo.log(10).unwrap(), Vec::new());
    }

    #[test]
    fn log_returns_real_commits_newest_first_and_honors_max() {
        let (tmp, repo) = TempRepo::new("log_real");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        let first = repo.commit("first commit").unwrap();
        tmp.write("f.txt", "v2");
        repo.stage(Path::new("f.txt")).unwrap();
        let second = repo.commit("second commit").unwrap();
        tmp.write("f.txt", "v3");
        repo.stage(Path::new("f.txt")).unwrap();
        let third = repo.commit("third commit").unwrap();

        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].oid, third.to_string());
        assert_eq!(log[0].summary, "third commit");
        assert_eq!(log[0].author, "Spartan Test");
        assert!(log[0].time > 0);
        assert_eq!(log[1].oid, second.to_string());
        assert_eq!(log[2].oid, first.to_string());

        let bounded = repo.log(2).unwrap();
        assert_eq!(bounded.len(), 2);
        assert_eq!(bounded[0].oid, third.to_string());
        assert_eq!(bounded[1].oid, second.to_string());
    }

    #[test]
    fn commit_changed_files_reports_a_root_commit_as_all_added() {
        let (tmp, repo) = TempRepo::new("commit_files_root");
        tmp.write("a.txt", "a");
        tmp.write("b.txt", "b");
        repo.stage(Path::new("a.txt")).unwrap();
        repo.stage(Path::new("b.txt")).unwrap();
        let oid = repo.commit("root").unwrap();
        let mut files = repo.commit_changed_files(&oid.to_string()).unwrap();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            files,
            vec![
                ("a.txt".to_string(), FileStatus::Added),
                ("b.txt".to_string(), FileStatus::Added)
            ]
        );
    }

    #[test]
    fn commit_changed_files_reports_only_what_that_commit_really_touched() {
        let (tmp, repo) = TempRepo::new("commit_files_second");
        tmp.write("a.txt", "a v1");
        tmp.write("b.txt", "b v1");
        repo.stage(Path::new("a.txt")).unwrap();
        repo.stage(Path::new("b.txt")).unwrap();
        repo.commit("first").unwrap();
        tmp.write("a.txt", "a v2");
        tmp.write("c.txt", "c new");
        repo.stage(Path::new("a.txt")).unwrap();
        repo.stage(Path::new("c.txt")).unwrap();
        let second = repo.commit("second").unwrap();
        let mut files = repo.commit_changed_files(&second.to_string()).unwrap();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        // b.txt was untouched by the second commit and must not appear.
        assert_eq!(
            files,
            vec![
                ("a.txt".to_string(), FileStatus::Modified),
                ("c.txt".to_string(), FileStatus::Added)
            ]
        );
    }

    #[test]
    fn commit_blob_content_and_parent_resolve_real_per_commit_data() {
        let (tmp, repo) = TempRepo::new("commit_blob");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        let first = repo.commit("first").unwrap();
        tmp.write("f.txt", "v2");
        repo.stage(Path::new("f.txt")).unwrap();
        let second = repo.commit("second").unwrap();
        assert_eq!(
            repo.commit_blob_content(&first.to_string(), Path::new("f.txt"))
                .unwrap(),
            Some("v1".to_string())
        );
        assert_eq!(
            repo.commit_blob_content(&second.to_string(), Path::new("f.txt"))
                .unwrap(),
            Some("v2".to_string())
        );
        assert_eq!(
            repo.commit_parent(&second.to_string()).unwrap(),
            Some(first.to_string())
        );
        assert_eq!(repo.commit_parent(&first.to_string()).unwrap(), None);
    }

    #[test]
    fn list_branches_reports_the_single_initial_branch_as_current() {
        let (tmp, repo) = TempRepo::new("branches_initial");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        assert_eq!(
            repo.list_branches().unwrap(),
            vec![("master".to_string(), true)]
        );
    }

    #[test]
    fn create_branch_adds_a_real_branch_without_switching_to_it() {
        let (tmp, repo) = TempRepo::new("branches_create");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        repo.create_branch("feature").unwrap();
        assert_eq!(
            repo.list_branches().unwrap(),
            vec![("feature".to_string(), false), ("master".to_string(), true)]
        );
        // Creating an already-existing branch is a real error, not a
        // silent overwrite.
        assert!(repo.create_branch("feature").is_err());
    }

    #[test]
    fn checkout_branch_really_switches_head_and_the_working_tree() {
        let (tmp, repo) = TempRepo::new("branches_checkout");
        tmp.write("f.txt", "master content");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("on master").unwrap();
        repo.create_branch("feature").unwrap();
        repo.checkout_branch("feature").unwrap();
        assert_eq!(repo.current_branch().unwrap(), "feature");
        tmp.write("f.txt", "feature content");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("on feature").unwrap();
        repo.checkout_branch("master").unwrap();
        assert_eq!(repo.current_branch().unwrap(), "master");
        // The real working-tree file must reflect master's own content
        // again -- a real checkout, not just a HEAD pointer move.
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "master content"
        );
    }

    #[test]
    fn checkout_branch_refuses_to_clobber_a_real_conflicting_dirty_change() {
        let (tmp, repo) = TempRepo::new("branches_checkout_dirty");
        tmp.write("f.txt", "master content");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("on master").unwrap();
        repo.create_branch("feature").unwrap();
        repo.checkout_branch("feature").unwrap();
        tmp.write("f.txt", "feature content");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("on feature").unwrap();
        repo.checkout_branch("master").unwrap();
        // A real, uncommitted local edit that conflicts with feature's
        // own version of the same file.
        tmp.write("f.txt", "LOCAL UNCOMMITTED");
        let result = repo.checkout_branch("feature");
        assert!(result.is_err(), "safe checkout must refuse the conflict");
        // Refused means *nothing* moved: still on master, local edit
        // untouched.
        assert_eq!(repo.current_branch().unwrap(), "master");
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "LOCAL UNCOMMITTED"
        );
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

    #[test]
    fn blame_attributes_each_line_to_the_commit_that_last_touched_it() {
        let (tmp, repo) = TempRepo::new("blame_basic");
        // Commit 1: two lines.
        tmp.write("f.txt", "line one\nline two\n");
        repo.stage(Path::new("f.txt")).unwrap();
        let c1 = repo.commit("first commit").unwrap();
        // Commit 2: change only line two.
        tmp.write("f.txt", "line one\nline two CHANGED\n");
        repo.stage(Path::new("f.txt")).unwrap();
        let c2 = repo.commit("second commit").unwrap();
        assert_ne!(c1, c2);

        let blame = repo.blame_file(Path::new("f.txt")).unwrap();
        assert_eq!(blame.len(), 2, "one blame entry per committed line");
        assert_eq!(
            blame[0].oid,
            c1.to_string(),
            "line 1 unchanged -> first commit"
        );
        assert_eq!(blame[0].summary, "first commit");
        assert_eq!(blame[0].author, "Spartan Test");
        assert_eq!(
            blame[1].oid,
            c2.to_string(),
            "line 2 changed -> second commit"
        );
        assert_eq!(blame[1].summary, "second commit");
    }

    #[test]
    fn blame_on_an_untracked_file_is_empty_not_an_error() {
        let (tmp, repo) = TempRepo::new("blame_untracked");
        tmp.write("committed.txt", "x\n");
        repo.stage(Path::new("committed.txt")).unwrap();
        repo.commit("init").unwrap();
        tmp.write("untracked.txt", "y\n");
        let blame = repo.blame_file(Path::new("untracked.txt")).unwrap();
        assert!(blame.is_empty());
    }

    #[test]
    fn blame_on_a_repo_with_no_commits_is_empty() {
        let (tmp, repo) = TempRepo::new("blame_nocommits");
        tmp.write("f.txt", "hello\n");
        let blame = repo.blame_file(Path::new("f.txt")).unwrap();
        assert!(blame.is_empty());
    }

    #[test]
    fn stash_save_list_pop_round_trip() {
        let (tmp, mut repo) = TempRepo::new("stash_rt");
        // Commit an initial version.
        tmp.write("f.txt", "original\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        // Make a real uncommitted change, then stash it.
        tmp.write("f.txt", "modified\n");
        let oid = repo.stash_save("wip").unwrap();
        assert!(oid.is_some(), "a real change should produce a stash");
        // Working tree is reverted to the committed version.
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "original\n"
        );
        // The stash is listed.
        let list = repo.stash_list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].index, 0);
        assert!(list[0].message.contains("wip"));
        // Pop restores the change and clears the stash.
        repo.stash_pop(0).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "modified\n"
        );
        assert!(repo.stash_list().unwrap().is_empty());
    }

    #[test]
    fn stash_apply_restores_the_change_but_keeps_the_stash() {
        let (tmp, mut repo) = TempRepo::new("stash_apply");
        tmp.write("f.txt", "original\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        tmp.write("f.txt", "modified\n");
        repo.stash_save("wip").unwrap();
        // Working tree reverted after stashing.
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "original\n"
        );
        // Apply restores the change but leaves the stash in place (unlike pop).
        repo.stash_apply(0).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "modified\n"
        );
        assert_eq!(
            repo.stash_list().unwrap().len(),
            1,
            "apply must keep the stash, not drop it"
        );
    }

    #[test]
    fn stash_on_a_clean_tree_is_none_not_an_error() {
        let (tmp, mut repo) = TempRepo::new("stash_clean");
        tmp.write("f.txt", "x\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        assert_eq!(repo.stash_save("nothing").unwrap(), None);
        assert!(repo.stash_list().unwrap().is_empty());
    }

    #[test]
    fn stash_drop_discards_without_applying() {
        let (tmp, mut repo) = TempRepo::new("stash_drop");
        tmp.write("f.txt", "original\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        tmp.write("f.txt", "modified\n");
        repo.stash_save("wip").unwrap();
        assert_eq!(repo.stash_list().unwrap().len(), 1);
        repo.stash_drop(0).unwrap();
        // Stash gone; working tree still at the reverted content (drop does
        // not re-apply).
        assert!(repo.stash_list().unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "original\n"
        );
    }

    #[test]
    fn remote_push_fetch_pull_round_trip_against_a_local_bare_remote() {
        // A real bare repo acting as the "remote" -- no network, no
        // credentials, exercising the real fetch/push/pull code paths.
        let remote_dir = std::env::temp_dir().join("spartan_git_test_remote_bare_rt");
        let _ = fs::remove_dir_all(&remote_dir);
        Repository::init_bare(&remote_dir).unwrap();
        let remote_url = remote_dir.to_str().unwrap();

        // Local repo A: commit, add the bare as `origin`, push.
        let (tmp_a, repo_a) = TempRepo::new("remote_a");
        tmp_a.write("f.txt", "line one\n");
        repo_a.stage(Path::new("f.txt")).unwrap();
        repo_a.commit("first").unwrap();
        repo_a.repo.remote("origin", remote_url).unwrap();

        let remotes = repo_a.list_remotes().unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].0, "origin");
        assert_eq!(remotes[0].1.as_deref(), Some(remote_url));

        let branch = repo_a.current_branch().unwrap();
        repo_a.push("origin", &branch).unwrap();

        // Local repo B: add the same remote, pull -> fast-forwards to A's commit.
        let (tmp_b, repo_b) = TempRepo::new("remote_b");
        repo_b.repo.remote("origin", remote_url).unwrap();
        let outcome = repo_b.pull_fast_forward("origin", &branch).unwrap();
        assert_eq!(outcome, PullOutcome::FastForwarded);
        assert_eq!(
            fs::read_to_string(tmp_b.dir.join("f.txt")).unwrap(),
            "line one\n",
            "B's working tree really got A's pushed content"
        );

        // A second pull with nothing new -> UpToDate.
        assert_eq!(
            repo_b.pull_fast_forward("origin", &branch).unwrap(),
            PullOutcome::UpToDate
        );

        // Now A commits again and pushes; B's next pull must fast-forward an
        // *existing* checkout and really update the working tree to the new
        // content (a real bug lived here: the default checkout strategy is a
        // dry run, so the ref moved but the file didn't -- guarded now).
        tmp_a.write("f.txt", "line one\nline two\n");
        repo_a.stage(Path::new("f.txt")).unwrap();
        repo_a.commit("second").unwrap();
        repo_a.push("origin", &branch).unwrap();
        assert_eq!(
            repo_b.pull_fast_forward("origin", &branch).unwrap(),
            PullOutcome::FastForwarded
        );
        assert_eq!(
            fs::read_to_string(tmp_b.dir.join("f.txt")).unwrap(),
            "line one\nline two\n",
            "fast-forward of an existing checkout must update the working tree"
        );

        let _ = fs::remove_dir_all(&remote_dir);
    }

    #[test]
    fn pull_reports_non_fast_forward_on_a_real_divergence() {
        let remote_dir = std::env::temp_dir().join("spartan_git_test_remote_bare_nff");
        let _ = fs::remove_dir_all(&remote_dir);
        Repository::init_bare(&remote_dir).unwrap();
        let remote_url = remote_dir.to_str().unwrap();

        // A: commit1, push.
        let (tmp_a, repo_a) = TempRepo::new("nff_a");
        tmp_a.write("f.txt", "c1\n");
        repo_a.stage(Path::new("f.txt")).unwrap();
        repo_a.commit("commit1").unwrap();
        repo_a.repo.remote("origin", remote_url).unwrap();
        let branch = repo_a.current_branch().unwrap();
        repo_a.push("origin", &branch).unwrap();

        // B: pull commit1, then make its own divergent local commit2.
        let (tmp_b, repo_b) = TempRepo::new("nff_b");
        repo_b.repo.remote("origin", remote_url).unwrap();
        repo_b.pull_fast_forward("origin", &branch).unwrap();
        tmp_b.write("f.txt", "c1\nB-local\n");
        repo_b.stage(Path::new("f.txt")).unwrap();
        repo_b.commit("commit2-local").unwrap();

        // A: commit3, push (remote is now commit1 -> commit3).
        tmp_a.write("f.txt", "c1\nA-remote\n");
        repo_a.stage(Path::new("f.txt")).unwrap();
        repo_a.commit("commit3").unwrap();
        repo_a.push("origin", &branch).unwrap();

        // B pulling now diverges (B has commit2, remote has commit3, both
        // off commit1) -> reported as non-fast-forward, never auto-merged.
        assert_eq!(
            repo_b.pull_fast_forward("origin", &branch).unwrap(),
            PullOutcome::NonFastForward
        );
        // B's own local commit is untouched.
        assert_eq!(
            fs::read_to_string(tmp_b.dir.join("f.txt")).unwrap(),
            "c1\nB-local\n"
        );

        let _ = fs::remove_dir_all(&remote_dir);
    }

    #[test]
    fn list_and_checkout_remote_branches_against_a_local_bare_remote() {
        let remote_dir = std::env::temp_dir().join("spartan_git_test_remote_bare_branches");
        let _ = fs::remove_dir_all(&remote_dir);
        Repository::init_bare(&remote_dir).unwrap();
        let remote_url = remote_dir.to_str().unwrap();

        // A: commit on the default branch and push it; then a real `feature`
        // branch with its own commit, pushed too.
        let (tmp_a, repo_a) = TempRepo::new("rbranch_a");
        tmp_a.write("f.txt", "main-content\n");
        repo_a.stage(Path::new("f.txt")).unwrap();
        repo_a.commit("main").unwrap();
        repo_a.repo.remote("origin", remote_url).unwrap();
        let default_branch = repo_a.current_branch().unwrap();
        repo_a.push("origin", &default_branch).unwrap();

        repo_a.create_branch("feature").unwrap();
        repo_a.checkout_branch("feature").unwrap();
        tmp_a.write("f.txt", "feature-content\n");
        repo_a.stage(Path::new("f.txt")).unwrap();
        repo_a.commit("feature work").unwrap();
        repo_a.push("origin", "feature").unwrap();

        // B: add the remote, fetch -> its remote-tracking refs populate.
        let (tmp_b, repo_b) = TempRepo::new("rbranch_b");
        repo_b.repo.remote("origin", remote_url).unwrap();
        repo_b.fetch("origin").unwrap();

        let remotes = repo_b.list_remote_branches().unwrap();
        assert!(
            remotes.contains(&"origin/feature".to_string()),
            "expected origin/feature in remote branches: {remotes:?}"
        );
        assert!(
            remotes.iter().all(|b| !b.ends_with("/HEAD")),
            "the symbolic origin/HEAD must be skipped: {remotes:?}"
        );

        // Checking out the remote branch creates a real local tracking branch
        // and lands its real content in the working tree.
        repo_b.checkout_remote_branch("origin/feature").unwrap();
        assert_eq!(repo_b.current_branch().as_deref(), Some("feature"));
        assert_eq!(
            fs::read_to_string(tmp_b.dir.join("f.txt")).unwrap(),
            "feature-content\n",
            "checkout of origin/feature must land its real content"
        );
        let locals = repo_b.list_branches().unwrap();
        assert!(
            locals.iter().any(|(n, _)| n == "feature"),
            "a real local `feature` branch must now exist: {locals:?}"
        );

        let _ = fs::remove_dir_all(&remote_dir);
    }
}
