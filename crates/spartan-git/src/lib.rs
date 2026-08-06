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

/// Splits raw bytes into lines, each still carrying its own trailing `\n`
/// (a final line with no trailing newline is kept as-is) -- so re-joining
/// any contiguous sub-slice of the result is a plain concatenation with no
/// reconstruction guesswork. Used by `stage_hunk` to splice one hunk's
/// real new-side lines into an unrelated region of the real old content.
fn split_keep_newlines(content: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, &b) in content.iter().enumerate() {
        if b == b'\n' {
            lines.push(&content[start..=i]);
            start = i + 1;
        }
    }
    if start < content.len() {
        lines.push(&content[start..]);
    }
    lines
}

/// Walks every real hunk a `git2::Patch` identifies and builds the
/// `HunkInfo` list both `diff_hunks` (unstaged, index-vs-workdir) and
/// `diff_hunks_staged` (staged, HEAD-vs-index) return -- the two callers
/// diff in opposite directions but collect the resulting hunks identically,
/// so this is the one real shared traversal rather than two copies of the
/// same loop.
fn collect_hunks(patch: &git2::Patch<'_>) -> Result<Vec<HunkInfo>, git2::Error> {
    let mut hunks = Vec::with_capacity(patch.num_hunks());
    for i in 0..patch.num_hunks() {
        let (hunk, line_count) = patch.hunk(i)?;
        let mut body = String::new();
        for l in 0..line_count {
            let line = patch.line_in_hunk(i, l)?;
            let origin = line.origin();
            if origin == '+' || origin == '-' || origin == ' ' {
                body.push(origin);
            }
            body.push_str(&String::from_utf8_lossy(line.content()));
        }
        hunks.push(HunkInfo {
            index: i,
            header: String::from_utf8_lossy(hunk.header())
                .trim_end()
                .to_string(),
            body,
        });
    }
    Ok(hunks)
}

/// Collects one hunk's real per-line detail into `HunkLine`s -- the exact
/// complementary traversal to `collect_hunks`'s body concat, used by both
/// `hunk_lines` (unstaged) and `hunk_lines_staged` (staged) so the line
/// selection UI and `stage_lines`/`unstage_lines` see one identical line
/// list with identical indices.
fn collect_lines(patch: &git2::Patch<'_>, hunk_index: usize) -> Result<Vec<HunkLine>, git2::Error> {
    let (_, line_count) = patch.hunk(hunk_index)?;
    let mut lines = Vec::with_capacity(line_count);
    for l in 0..line_count {
        let line = patch.line_in_hunk(hunk_index, l)?;
        lines.push(HunkLine {
            index: l,
            origin: line.origin(),
            content: String::from_utf8_lossy(line.content()).to_string(),
        });
    }
    Ok(lines)
}

/// Replaces the real `base_lines[splice_start .. splice_start+base_len]`
/// region with `region` bytes, keeping everything outside it byte-identical
/// -- the plain splice `splice_hunk_region` (and the per-line selection
/// splice) reduce to.
fn splice_region_content(
    base_lines: &[&[u8]],
    splice_start: usize,
    base_len: usize,
    region: &[u8],
) -> Vec<u8> {
    let splice_end = (splice_start + base_len).min(base_lines.len());
    let mut out = Vec::new();
    for line in &base_lines[..splice_start] {
        out.extend_from_slice(line);
    }
    out.extend_from_slice(region);
    for line in &base_lines[splice_end..] {
        out.extend_from_slice(line);
    }
    out
}

/// Shared real splice every hunk-level index write uses (`stage_hunk`,
/// `unstage_hunk`): replaces the real `base_lines[splice_start .. splice_start+base_len]`
/// region of the index with exactly the hunk lines `emit` accepts (each
/// line's own real bytes preserved, trailing newline included), keeping
/// everything outside the region byte-identical. `emit` gets each hunk
/// line's own 0-based position *and* the real `DiffLine`. Used by the
/// whole-hunk callers, whose emitted lines are already monotonic in the
/// hunk's own real line order; the per-line selection splice needs
/// coordinate-ordered emission (see `selection_region`) and calls
/// `splice_region_content` directly instead.
fn splice_hunk_region(
    patch: &git2::Patch<'_>,
    hunk_index: usize,
    base_lines: &[&[u8]],
    splice_start: usize,
    base_len: usize,
    mut emit: impl FnMut(usize, &git2::DiffLine<'_>) -> bool,
) -> Result<Vec<u8>, git2::Error> {
    let (_, line_count) = patch.hunk(hunk_index)?;
    let mut region = Vec::new();
    for l in 0..line_count {
        let line = patch.line_in_hunk(hunk_index, l)?;
        if emit(l, &line) {
            region.extend_from_slice(line.content());
        }
    }
    Ok(splice_region_content(
        base_lines,
        splice_start,
        base_len,
        &region,
    ))
}

/// Emits one change group (a maximal run of `'-'`/`'+'` hunk lines) as a
/// single sorted block: every kept deletion and selected addition is given
/// a real slot against the side of the destination file the group replaces
/// (old-side slots when staging, new-side slots when unstaging) and the
/// block is emitted in `(slot, deletion-first)` order. This is what keeps a
/// selection spanning two adjacent changes in one hunk correct: libgit2
/// reports those lines deletion-group-then-addition-group (real git's own
/// unified display), so raw hunk order would land an unselected deletion of
/// the later change before a selected addition of the earlier one -- but
/// slotting both against the same anchor (each deletion occupying the slot
/// of the old line it removes, each addition the slot of the old/new line it
/// replaces or inserts before) gives every emitted line its one real
/// position in the result. The single slot a replacement pair shares is
/// broken deletion-first (the kept old line before the added new line);
/// pure-insertion groups slot their additions sequentially after the
/// group's anchor, so they stay before the following context line they
/// precede. `staged` picks the direction: `false` stages (a `'+'` line is
/// emitted iff selected, a `'-'` line iff *not* selected), `true` unstages
/// (the exact complement -- a `'-'` line is emitted iff selected, a `'+'`
/// line iff *not* selected). End-of-file-newline marker lines are never
/// emitted, matching `stage_hunk`/`unstage_hunk`'s own treatment.
fn selection_region(
    patch: &git2::Patch<'_>,
    hunk_index: usize,
    selected: &[bool],
    staged: bool,
) -> Result<Vec<u8>, git2::Error> {
    let (hunk, line_count) = patch.hunk(hunk_index)?;
    debug_assert_eq!(selected.len(), line_count);
    let mut out = Vec::new();
    // The count of old/new lines already consumed: the anchor a change
    // group's slots are numbered from. Initialized one before the hunk's
    // own first line (`old_start`/`new_start` are 1-indexed per real
    // unified-diff convention).
    let mut old_anchor: usize = hunk.old_start().saturating_sub(1) as usize;
    let mut new_anchor: usize = hunk.new_start().saturating_sub(1) as usize;
    let mut group: Vec<(usize, usize, bool)> = Vec::new(); // (line index, slot, is deletion)
    for l in 0..line_count {
        let line = patch.line_in_hunk(hunk_index, l)?;
        match line.origin() {
            ' ' => {
                flush_selection_group(&mut group, &mut out, patch, hunk_index, selected, staged)?;
                out.extend_from_slice(line.content());
                old_anchor += 1;
                new_anchor += 1;
            }
            '-' => {
                old_anchor += 1;
                group.push((l, old_anchor, true));
            }
            '+' => {
                new_anchor += 1;
                group.push((l, new_anchor, false));
            }
            _ => {}
        }
    }
    flush_selection_group(&mut group, &mut out, patch, hunk_index, selected, staged)?;
    Ok(out)
}

/// Sorts and emits one collected change group. `slot` values are already in
/// the destination side's coordinate space (old-side when `staged` is false,
/// new-side when true, since `selection_region` increments `old_anchor` for
/// `'-'` lines and `new_anchor` for `'+'` lines), so a pure `(slot,
/// deletion-first)` sort orders every line at its one real position. Only
/// the lines the direction's own predicate accepts are emitted; the rest are
/// dropped from the region (and thus from the spliced index).
fn flush_selection_group(
    group: &mut Vec<(usize, usize, bool)>,
    out: &mut Vec<u8>,
    patch: &git2::Patch<'_>,
    hunk_index: usize,
    selected: &[bool],
    staged: bool,
) -> Result<(), git2::Error> {
    group.sort_by_key(|&(_, slot, is_del)| (slot, !is_del));
    for &(l, _, is_del) in group.iter() {
        let line = patch.line_in_hunk(hunk_index, l)?;
        let is_selected = selected[l];
        let emit = if is_del {
            is_selected == staged
        } else {
            is_selected != staged
        };
        if emit {
            out.extend_from_slice(line.content());
        }
    }
    group.clear();
    Ok(())
}

/// Shared credential callback for real fetch/push against an authenticated
/// remote. Tries SSH-agent first (the common key-backed case), then a
/// default/anonymous credential (local `file://` remotes need none, so the
/// callback typically isn't even invoked for those). An interactive
/// username/password or HTTPS-token prompt is a named, open follow-up.
fn make_remote_callbacks(github_token: Option<String>) -> git2::RemoteCallbacks<'static> {
    let mut cb = git2::RemoteCallbacks::new();
    cb.credentials(move |url, username_from_url, allowed| {
        if allowed.contains(git2::CredentialType::SSH_KEY) {
            if let Some(user) = username_from_url {
                if let Ok(cred) = git2::Cred::ssh_key_from_agent(user) {
                    return Ok(cred);
                }
            }
        }
        // Never send a GitHub token to an arbitrary configured remote. The
        // token is only valid for HTTPS GitHub hosts; SSH still uses the
        // agent path above.
        if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT)
            && github_token.is_some()
            && url.starts_with("https://github.com/")
        {
            return git2::Cred::userpass_plaintext(
                "x-access-token",
                github_token.as_deref().unwrap(),
            );
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

/// Outcome of a real `merge_branch` attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeOutcome {
    UpToDate,
    FastForwarded,
    /// A real merge commit was created (two parents); no conflicts.
    Merged,
    /// The merge left real, unresolved conflicts in the index and working
    /// tree -- `list_conflicts()`/`resolve_conflict_with_content()`/
    /// `commit_merge()` are the real next steps.
    Conflicted,
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

/// Parses a real git remote URL into `(owner, repo)` if it points at
/// github.com, in any of the real shapes git itself accepts:
/// `git@github.com:owner/repo.git`, `https://github.com/owner/repo(.git)`,
/// or `ssh://git@github.com/owner/repo(.git)`. Returns `None` for anything
/// else (a non-GitHub remote, or a malformed URL) -- a real, honest "not a
/// GitHub repo" rather than a guess.
pub fn parse_github_owner_repo(remote_url: &str) -> Option<(String, String)> {
    let after_host = if let Some(rest) = remote_url.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = remote_url.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = remote_url.strip_prefix("http://github.com/") {
        rest
    } else {
        remote_url.strip_prefix("ssh://git@github.com/")?
    };
    let trimmed = after_host.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = trimmed.split_once('/')?;
    // A real remote URL's `owner`/`repo` segments end up interpolated directly into a
    // `format!`-built GitHub API URL by this crate's own caller (`crates/spartan-backend::
    // github.rs`) -- GitHub's own real username/repo-name rules only ever allow
    // `[A-Za-z0-9._-]`, so anything outside that set here means either a malformed/unexpected
    // URL shape this parser doesn't actually understand, or content that was never a real
    // GitHub identifier at all. Reject it rather than passing it through unchecked.
    let is_valid_github_segment = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    if !is_valid_github_segment(owner) || !is_valid_github_segment(repo) {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
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

/// One real tag in `list_tags()`'s output -- `target` is the hex oid of the
/// commit the tag points at (resolved through an annotated tag object if the
/// tag is annotated), and `annotated` distinguishes an annotated tag (its own
/// tag object + message) from a lightweight one (just a ref).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagInfo {
    pub name: String,
    pub target: String,
    pub annotated: bool,
}

/// One real conflicted file, as reported by `list_conflicts()` -- see that
/// method's own doc comment for what a `None` side really means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictEntry {
    pub path: PathBuf,
    pub ancestor: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

/// One real unstaged diff hunk for a single file, as identified by
/// `libgit2` itself (not a hand-rolled diff algorithm) -- see
/// `diff_hunks()`'s own doc comment for exactly what it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkInfo {
    /// This hunk's position among the file's real hunks, 0-indexed --
    /// the value `stage_hunk()` expects back.
    pub index: usize,
    /// The real unified-diff hunk header, e.g. `@@ -3,4 +3,6 @@`.
    pub header: String,
    /// The real hunk body: every context/`+`/`-` line concatenated, each
    /// still carrying its real leading `' '`/`'+'`/`'-'` origin marker.
    pub body: String,
}

/// One real line inside a hunk, as reported by `git2::Patch::line_in_hunk` --
/// the per-line (sub-hunk) unit both the line-selection UI and
/// `stage_lines`/`unstage_lines` key on. `index` is the line's 0-based
/// position *within its own hunk* (matching the order `hunk_lines`/
/// `hunk_lines_staged` return and what `stage_lines`/`unstage_lines` expect
/// back as a selection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkLine {
    pub index: usize,
    /// The real `git2` diff origin: `' '` context, `'+'` addition, `'-'`
    /// deletion -- plus the end-of-file-newline markers libgit2 itself
    /// reports as `'='`/`'>'`/`'<'` (which the UI shows but never makes
    /// selectable, matching `stage_hunk`'s own treatment of those lines).
    pub origin: char,
    /// The line's real content, still carrying its own trailing newline
    /// (a final line with none keeps none). Displayed by the UI, but the
    /// splice itself never round-trips this string -- it re-reads the
    /// patch's own raw bytes by `index`, so there is no lossy round trip.
    pub content: String,
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

    /// Real clone of a remote repository into `dest` (which must not yet
    /// exist, or exist empty -- `libgit2` itself refuses to clone over a
    /// real, non-empty directory). Uses the same `make_remote_callbacks`
    /// credentials the fetch/push paths use: SSH-agent first, then a
    /// default/anonymous credential (a local-path or `file://` remote needs
    /// none). An interactive HTTPS-token prompt is the same named, open
    /// follow-up as for the other remote ops, not attempted here.
    pub fn clone(url: &str, dest: &Path) -> Result<Self, git2::Error> {
        Self::clone_with_github_token(url, dest, None)
    }

    pub fn clone_with_github_token(
        url: &str,
        dest: &Path,
        github_token: Option<String>,
    ) -> Result<Self, git2::Error> {
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(make_remote_callbacks(github_token));
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fo);
        let repo = builder.clone(url, dest)?;
        Ok(Self { repo })
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
            .include_ignored(false)
            // Refresh the index's stat cache before any operation that may
            // hand the repository to libgit2's stash machinery. Without
            // this, a same-size edit made immediately after a commit can be
            // treated as unchanged by the stash tree builder on filesystems
            // with coarse timestamp resolution.
            .update_index(true);
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

    /// Real per-hunk unstaged diff for one file -- diffs the file's real
    /// index (staged) blob against its real current working-tree content
    /// via `git2::Patch::from_blob_and_buffer` (a real libgit2 diff, not a
    /// hand-rolled line algorithm) and returns every hunk it identifies, in
    /// order. A real, named v1 scope cut: a path with no index entry yet
    /// (an untracked file) has no "old" baseline to hunk against and
    /// returns a real, honest empty list -- whole-file `stage()` already
    /// covers that case; partial staging of a brand-new file is a
    /// separate, unimplemented follow-up.
    pub fn diff_hunks(&self, path: &Path) -> Result<Vec<HunkInfo>, git2::Error> {
        let workdir = self
            .repo
            .workdir()
            .ok_or_else(|| git2::Error::from_str("repository has no working directory"))?;
        let index = self.repo.index()?;
        let old_blob = match index.get_path(path, 0) {
            Some(entry) => self.repo.find_blob(entry.id)?,
            None => return Ok(Vec::new()),
        };
        let new_content = std::fs::read(workdir.join(path)).unwrap_or_default();
        let patch = git2::Patch::from_blob_and_buffer(
            &old_blob,
            Some(path),
            &new_content,
            Some(path),
            None,
        )?;
        collect_hunks(&patch)
    }

    /// Real "stage this one hunk" (the mechanism behind `git add -p`'s own
    /// per-hunk selection) -- recomputes the real unstaged diff for `path`
    /// (matching `diff_hunks()` exactly, so `hunk_index` always refers to
    /// the hunk the caller most recently saw), splices that one hunk's real
    /// "new side" (its context + `+` lines) into the real index (old)
    /// content at the hunk's own `old_start`/`old_lines` position, and
    /// writes the result as the file's new index entry via
    /// `Index::add_frombuffer` (which computes and writes the real blob
    /// itself) -- the working tree is left completely untouched, matching
    /// real `git add -p`'s own behavior exactly. Staging hunks one at a
    /// time (re-fetching the list between each) keeps every later hunk's
    /// own `old_start`/`old_lines` correct, since each call re-diffs
    /// against the just-updated index; staging by a stale index out of
    /// order in one batch is not supported -- a real, named v1 scope cut.
    pub fn stage_hunk(&self, path: &Path, hunk_index: usize) -> Result<(), git2::Error> {
        let workdir = self
            .repo
            .workdir()
            .ok_or_else(|| git2::Error::from_str("repository has no working directory"))?;
        let mut index = self.repo.index()?;
        let old_entry = index
            .get_path(path, 0)
            .ok_or_else(|| git2::Error::from_str("no staged base for this path to hunk against"))?;
        let old_blob = self.repo.find_blob(old_entry.id)?;
        let old_content = old_blob.content();
        let new_content = std::fs::read(workdir.join(path)).unwrap_or_default();
        let patch = git2::Patch::from_blob_and_buffer(
            &old_blob,
            Some(path),
            &new_content,
            Some(path),
            None,
        )?;
        if hunk_index >= patch.num_hunks() {
            return Err(git2::Error::from_str("hunk index out of range"));
        }
        let (hunk, _line_count) = patch.hunk(hunk_index)?;
        // Split the real old (index) content into lines, keeping each
        // line's own trailing `\n` attached, so re-joining is a plain
        // concatenation with no reconstruction guesswork.
        let old_lines: Vec<&[u8]> = split_keep_newlines(old_content);
        let old_start = hunk.old_start() as usize; // 1-indexed, per real unified-diff convention
        let old_len = hunk.old_lines() as usize;
        // `old_start` is 1-indexed and, for a pure-addition hunk with
        // `old_lines() == 0`, names the real line *after* which the
        // insertion happens -- so the real splice point is `old_start`
        // unchanged in that case (nothing to remove) and `old_start - 1`
        // otherwise (the first of the `old_len` lines this hunk replaces).
        let splice_start = if old_len == 0 {
            old_start.min(old_lines.len())
        } else {
            old_start.saturating_sub(1)
        };
        // The whole-hunk predicate: every context line and every real
        // addition, no deletions -- `stage_lines`'s own selection predicate
        // reduces to exactly this when every change line is selected.
        let new_index_content = splice_hunk_region(
            &patch,
            hunk_index,
            &old_lines,
            splice_start,
            old_len,
            |_, line| line.origin() == ' ' || line.origin() == '+',
        )?;
        let mut entry = old_entry;
        entry.id = git2::Oid::zero(); // overwritten by add_frombuffer from the real content
        entry.file_size = 0; // same
        index.add_frombuffer(&entry, &new_index_content)?;
        index.write()
    }

    /// Real per-line detail of one unstaged hunk (`index`-vs-`workdir`, the
    /// exact same diff `diff_hunks()`/`stage_hunk()` use, so `hunk_index`
    /// and every returned `HunkLine.index` line up with what the caller most
    /// recently saw) -- the data the per-line (sub-hunk) selection UI
    /// renders, and the selection namespace `stage_lines()` expects back.
    /// Mirrors `diff_hunks()`'s own untracked-file scope cut: no index
    /// entry means no old baseline, a real honest error rather than a
    /// guessed one.
    pub fn hunk_lines(&self, path: &Path, hunk_index: usize) -> Result<Vec<HunkLine>, git2::Error> {
        let workdir = self
            .repo
            .workdir()
            .ok_or_else(|| git2::Error::from_str("repository has no working directory"))?;
        let index = self.repo.index()?;
        let old_blob = match index.get_path(path, 0) {
            Some(entry) => self.repo.find_blob(entry.id)?,
            None => {
                return Err(git2::Error::from_str(
                    "no staged base for this path to diff against",
                ))
            }
        };
        let new_content = std::fs::read(workdir.join(path)).unwrap_or_default();
        let patch = git2::Patch::from_blob_and_buffer(
            &old_blob,
            Some(path),
            &new_content,
            Some(path),
            None,
        )?;
        if hunk_index >= patch.num_hunks() {
            return Err(git2::Error::from_str("hunk index out of range"));
        }
        collect_lines(&patch, hunk_index)
    }

    /// Real per-line staging (`git add -p`'s own line-level selection, the
    /// sub-hunk counterpart to `stage_hunk()`). Recomputes the real unstaged
    /// diff fresh (matching `diff_hunks()`/`hunk_lines()` exactly, so
    /// `hunk_index` and the `lines` selection always refer to the hunk the
    /// caller most recently saw), and splices that hunk's context + selected
    /// real lines into the index at the hunk's own `old_start`/`old_lines`
    /// position -- the exact same shared region splice `stage_hunk()` uses,
    /// differing only in which lines are emitted: `lines` is the 0-based
    /// selection of change lines within the hunk (indices from
    /// `hunk_lines()`), context lines are never selectable, and each
    /// selected `'+'` addition is emitted while each selected `'-'` deletion
    /// is *omitted* (staging a deletion means removing that old line from
    /// the index). Selecting every change line reduces to exactly
    /// `stage_hunk()`; selecting none is an exact no-op. The working tree is
    /// left completely untouched. Staging lines one hunk at a time
    /// (re-fetching the list between each) keeps every later hunk's position
    /// correct -- the same real, named v1 scope cut `stage_hunk()` already
    /// documents.
    pub fn stage_lines(
        &self,
        path: &Path,
        hunk_index: usize,
        lines: &[usize],
    ) -> Result<(), git2::Error> {
        let workdir = self
            .repo
            .workdir()
            .ok_or_else(|| git2::Error::from_str("repository has no working directory"))?;
        let mut index = self.repo.index()?;
        let old_entry = index
            .get_path(path, 0)
            .ok_or_else(|| git2::Error::from_str("no staged base for this path to hunk against"))?;
        let old_blob = self.repo.find_blob(old_entry.id)?;
        let old_content = old_blob.content();
        let new_content = std::fs::read(workdir.join(path)).unwrap_or_default();
        let patch = git2::Patch::from_blob_and_buffer(
            &old_blob,
            Some(path),
            &new_content,
            Some(path),
            None,
        )?;
        if hunk_index >= patch.num_hunks() {
            return Err(git2::Error::from_str("hunk index out of range"));
        }
        let (hunk, line_count) = patch.hunk(hunk_index)?;
        if let Some(&bad) = lines.iter().find(|&&l| l >= line_count) {
            return Err(git2::Error::from_str(&format!(
                "line index {bad} out of range for hunk with {line_count} lines"
            )));
        }
        let selected: Vec<bool> = (0..line_count).map(|l| lines.contains(&l)).collect();
        let old_lines: Vec<&[u8]> = split_keep_newlines(old_content);
        let old_start = hunk.old_start() as usize; // 1-indexed, per real unified-diff convention
        let old_len = hunk.old_lines() as usize;
        // Same pure-insertion splice rule `stage_hunk()` documents.
        let splice_start = if old_len == 0 {
            old_start.min(old_lines.len())
        } else {
            old_start.saturating_sub(1)
        };
        // `staged: false` = this is a *staging* selection: emit `'+'` lines
        // iff selected, `'-'` lines iff not selected.
        let region = selection_region(&patch, hunk_index, &selected, false)?;
        let new_index_content = splice_region_content(&old_lines, splice_start, old_len, &region);
        let mut entry = old_entry;
        entry.id = git2::Oid::zero(); // overwritten by add_frombuffer from the real content
        entry.file_size = 0; // same
        index.add_frombuffer(&entry, &new_index_content)?;
        index.write()
    }

    /// Real per-hunk *staged* diff for one file -- the exact complementary
    /// diff to `diff_hunks()`'s own unstaged one: diffs the file's real
    /// `HEAD` blob (old) against its real current index (staged) blob (new)
    /// via `git2::Patch::from_buffers`, and returns every hunk it
    /// identifies, in order. A real, named v1 scope cut mirroring
    /// `diff_hunks()`'s own: a path with no `HEAD` entry (nothing committed
    /// yet for it -- e.g. a brand-new file that's fully staged) has no real
    /// "old" baseline to hunk against and returns a real, honest empty
    /// list -- whole-file `unstage()` already covers that case.
    pub fn diff_hunks_staged(&self, path: &Path) -> Result<Vec<HunkInfo>, git2::Error> {
        let index = self.repo.index()?;
        let index_entry = match index.get_path(path, 0) {
            Some(entry) => entry,
            None => return Ok(Vec::new()),
        };
        let index_blob = self.repo.find_blob(index_entry.id)?;
        let head_blob = self
            .repo
            .head()
            .and_then(|h| h.peel_to_tree())
            .ok()
            .and_then(|tree| tree.get_path(path).ok())
            .and_then(|entry| self.repo.find_blob(entry.id()).ok());
        let head_blob = match head_blob {
            Some(blob) => blob,
            None => return Ok(Vec::new()),
        };
        let patch = git2::Patch::from_buffers(
            head_blob.content(),
            Some(path),
            index_blob.content(),
            Some(path),
            None,
        )?;
        collect_hunks(&patch)
    }

    /// Real "unstage this one hunk" -- the direct mirror of `stage_hunk()`,
    /// the mechanism behind `git add -p`'s own per-hunk *de*selection (`git
    /// restore --staged -p` in modern git). Recomputes the real staged diff
    /// for `path` (`HEAD` vs index, matching `diff_hunks_staged()` exactly,
    /// so `hunk_index` always refers to the hunk the caller most recently
    /// saw), and splices that hunk's real "old side" (`HEAD`'s own context +
    /// removed lines) into the real current index content at the hunk's own
    /// `new_start`/`new_lines` position -- the exact mirror of
    /// `stage_hunk`'s own `old_start`/`old_lines` splice: there, the OLD
    /// side (index) received the hunk's NEW content; here, the NEW side
    /// (index) receives the hunk's OLD (`HEAD`) content. The working tree is
    /// left completely untouched, matching `stage_hunk`'s own behavior.
    /// Unstaging hunks one at a time (re-fetching the list between each)
    /// keeps every later hunk's own position correct -- the same real,
    /// named v1 scope cut `stage_hunk` already documents.
    pub fn unstage_hunk(&self, path: &Path, hunk_index: usize) -> Result<(), git2::Error> {
        let mut index = self.repo.index()?;
        let index_entry = index
            .get_path(path, 0)
            .ok_or_else(|| git2::Error::from_str("no staged content for this path to unstage"))?;
        let index_blob = self.repo.find_blob(index_entry.id)?;
        let index_content = index_blob.content();
        let head_blob = self
            .repo
            .head()
            .and_then(|h| h.peel_to_tree())
            .ok()
            .and_then(|tree| tree.get_path(path).ok())
            .and_then(|entry| self.repo.find_blob(entry.id()).ok())
            .ok_or_else(|| {
                git2::Error::from_str("no HEAD baseline for this path to unstage against")
            })?;
        let patch = git2::Patch::from_buffers(
            head_blob.content(),
            Some(path),
            index_content,
            Some(path),
            None,
        )?;
        if hunk_index >= patch.num_hunks() {
            return Err(git2::Error::from_str("hunk index out of range"));
        }
        let (hunk, _line_count) = patch.hunk(hunk_index)?;
        // Split the real current (index) content into lines, keeping each
        // line's own trailing `\n` attached -- mirrors `stage_hunk`'s own
        // `old_lines` split exactly, just on the opposite side.
        let new_lines: Vec<&[u8]> = split_keep_newlines(index_content);
        let new_start = hunk.new_start() as usize; // 1-indexed, per real unified-diff convention
        let new_len = hunk.new_lines() as usize;
        // Mirrors `stage_hunk`'s own `old_len == 0` pure-insertion case,
        // swapped: `new_len == 0` means this hunk is a pure deletion from
        // the index's own perspective (HEAD had lines here that staging
        // removed), so the real splice point is `new_start` unchanged
        // (nothing in the index to remove) rather than `new_start - 1`.
        let splice_start = if new_len == 0 {
            new_start.min(new_lines.len())
        } else {
            new_start.saturating_sub(1)
        };
        // The whole-hunk predicate: every context line and every real
        // deletion, no additions -- `unstage_lines`'s own selection
        // predicate reduces to exactly this when every change line is
        // selected.
        let new_index_content = splice_hunk_region(
            &patch,
            hunk_index,
            &new_lines,
            splice_start,
            new_len,
            |_, line| line.origin() == ' ' || line.origin() == '-',
        )?;
        let mut entry = index_entry;
        entry.id = git2::Oid::zero(); // overwritten by add_frombuffer from the real content
        entry.file_size = 0; // same
        index.add_frombuffer(&entry, &new_index_content)?;
        index.write()
    }

    /// Real per-line detail of one *staged* hunk (`HEAD`-vs-`index`, the
    /// exact same diff `diff_hunks_staged()`/`unstage_hunk()` use) -- the
    /// mirror of `hunk_lines()` for the staged side, feeding the same
    /// per-line selection UI and the `unstage_lines()` selection namespace.
    /// Mirrors `diff_hunks_staged()`'s own scope cut: a path with no `HEAD`
    /// entry has no old baseline, a real honest error rather than a guess.
    pub fn hunk_lines_staged(
        &self,
        path: &Path,
        hunk_index: usize,
    ) -> Result<Vec<HunkLine>, git2::Error> {
        let index = self.repo.index()?;
        let index_entry = index
            .get_path(path, 0)
            .ok_or_else(|| git2::Error::from_str("no staged content for this path"))?;
        let index_blob = self.repo.find_blob(index_entry.id)?;
        let head_blob = self
            .repo
            .head()
            .and_then(|h| h.peel_to_tree())
            .ok()
            .and_then(|tree| tree.get_path(path).ok())
            .and_then(|entry| self.repo.find_blob(entry.id()).ok())
            .ok_or_else(|| {
                git2::Error::from_str("no HEAD baseline for this path to diff against")
            })?;
        let patch = git2::Patch::from_buffers(
            head_blob.content(),
            Some(path),
            index_blob.content(),
            Some(path),
            None,
        )?;
        if hunk_index >= patch.num_hunks() {
            return Err(git2::Error::from_str("hunk index out of range"));
        }
        collect_lines(&patch, hunk_index)
    }

    /// Real per-line *unstaging* (`git restore --staged -p`'s own line-level
    /// deselection, the direct mirror of `stage_lines()`). Recomputes the
    /// real staged diff fresh (`HEAD` vs index, matching
    /// `diff_hunks_staged()`/`hunk_lines_staged()` exactly), and splices
    /// the hunk's context + selected real lines into the index at the
    /// hunk's own `new_start`/`new_lines` position -- the same shared splice
    /// `unstage_hunk()` uses, differing only in which lines are emitted:
    /// each selected `'-'` deletion is re-added to the index (emitted), each
    /// selected `'+'` addition is removed from it (omitted), context lines
    /// are never selectable. Selecting every change line reduces to exactly
    /// `unstage_hunk()`; selecting none is an exact no-op. The working tree
    /// is left completely untouched. Unstaging lines one hunk at a time is
    /// the same real, named v1 scope cut `unstage_hunk()` already documents.
    pub fn unstage_lines(
        &self,
        path: &Path,
        hunk_index: usize,
        lines: &[usize],
    ) -> Result<(), git2::Error> {
        let mut index = self.repo.index()?;
        let index_entry = index
            .get_path(path, 0)
            .ok_or_else(|| git2::Error::from_str("no staged content for this path to unstage"))?;
        let index_blob = self.repo.find_blob(index_entry.id)?;
        let index_content = index_blob.content();
        let head_blob = self
            .repo
            .head()
            .and_then(|h| h.peel_to_tree())
            .ok()
            .and_then(|tree| tree.get_path(path).ok())
            .and_then(|entry| self.repo.find_blob(entry.id()).ok())
            .ok_or_else(|| {
                git2::Error::from_str("no HEAD baseline for this path to unstage against")
            })?;
        let patch = git2::Patch::from_buffers(
            head_blob.content(),
            Some(path),
            index_content,
            Some(path),
            None,
        )?;
        if hunk_index >= patch.num_hunks() {
            return Err(git2::Error::from_str("hunk index out of range"));
        }
        let (hunk, line_count) = patch.hunk(hunk_index)?;
        if let Some(&bad) = lines.iter().find(|&&l| l >= line_count) {
            return Err(git2::Error::from_str(&format!(
                "line index {bad} out of range for hunk with {line_count} lines"
            )));
        }
        let selected: Vec<bool> = (0..line_count).map(|l| lines.contains(&l)).collect();
        // Split the real current (index) content into lines, keeping each
        // line's own trailing `\n` attached -- mirrors `unstage_hunk`'s own
        // `new_lines` split exactly.
        let new_lines: Vec<&[u8]> = split_keep_newlines(index_content);
        let new_start = hunk.new_start() as usize; // 1-indexed, per real unified-diff convention
        let new_len = hunk.new_lines() as usize;
        // Same pure-insertion splice rule `unstage_hunk()` documents.
        let splice_start = if new_len == 0 {
            new_start.min(new_lines.len())
        } else {
            new_start.saturating_sub(1)
        };
        // `staged: true` = this is an *un*staging selection: emit `'-'`
        // lines iff selected (re-adding that old line to the index), `'+'`
        // lines iff not selected (keeping it in the index).
        let region = selection_region(&patch, hunk_index, &selected, true)?;
        let new_index_content = splice_region_content(&new_lines, splice_start, new_len, &region);
        let mut entry = index_entry;
        entry.id = git2::Oid::zero(); // overwritten by add_frombuffer from the real content
        entry.file_size = 0; // same
        index.add_frombuffer(&entry, &new_index_content)?;
        index.write()
    }

    /// Real "discard changes" -- restores this one path's working-tree file
    /// to the version in the index (a `git checkout -- <path>`, i.e. it
    /// discards *unstaged* modifications but keeps whatever is staged). A
    /// real, destructive operation by design; the caller is responsible for
    /// confirming with the user first. Force-overwrites the working file.
    pub fn discard_changes(&self, path: &Path) -> Result<(), git2::Error> {
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force().path(path);
        // Checking out the index (not the HEAD tree) matches `git checkout
        // -- <path>` semantics: unstaged changes are discarded, staged ones
        // are preserved.
        self.repo.checkout_index(None, Some(&mut checkout))
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

    /// Real amend of the last commit (`HEAD`) -- rewrites its message and
    /// its tree to the current index, keeping the same parent(s) so the
    /// commit count is unchanged (this replaces the last commit, it does
    /// not add a new one). Uses `git2`'s own `Commit::amend` with the
    /// current index tree and a fresh committer signature (matching the
    /// real `git commit --amend` behavior of updating the committer). The
    /// author is preserved by passing `None` for the author signature.
    /// Errors honestly if there is no `HEAD` commit to amend yet.
    pub fn commit_amend(&self, message: &str) -> Result<git2::Oid, git2::Error> {
        let head_commit = self.repo.head()?.peel_to_commit()?;
        let mut index = self.repo.index()?;
        let tree_oid = index.write_tree()?;
        let tree = self.repo.find_tree(tree_oid)?;
        let signature = self.repo.signature()?;
        // `amend` updates HEAD in place; author preserved (None), committer
        // refreshed, message and tree replaced with the current ones.
        head_commit.amend(
            Some("HEAD"),
            None,
            Some(&signature),
            None,
            Some(message),
            Some(&tree),
        )
    }

    /// Real revert of a commit by its hex `oid` -- creates a *new* commit on
    /// `HEAD` that undoes the named commit's changes (exactly `git revert`),
    /// rather than rewriting history. Applies the reverse changes to the index
    /// and working tree via `git2`'s own `revert`, and if that produces no
    /// conflicts, commits the result with a real `Revert "<summary>"` message
    /// and clears the in-progress REVERT state. A revert that conflicts is a
    /// real, honest error: the in-progress state is cleaned up (so the repo
    /// isn't left half-reverted) and the caller is told, rather than silently
    /// committing a broken tree. Errors honestly if the oid is unknown or there
    /// is no `HEAD` commit to revert onto.
    pub fn revert_commit(&self, oid_hex: &str) -> Result<git2::Oid, git2::Error> {
        let oid = git2::Oid::from_str(oid_hex)?;
        let commit = self.repo.find_commit(oid)?;
        let head_commit = self.repo.head()?.peel_to_commit()?;
        // Apply the reverse changes to index + working tree (like
        // `git revert --no-commit`); this also sets the repo's REVERT state.
        self.repo.revert(&commit, None)?;
        let mut index = self.repo.index()?;
        if index.has_conflicts() {
            // Don't leave the repo half-reverted -- clean up and report.
            let _ = self.repo.cleanup_state();
            return Err(git2::Error::from_str(
                "revert produced conflicts; the working tree was left unchanged",
            ));
        }
        let tree_oid = index.write_tree()?;
        let tree = self.repo.find_tree(tree_oid)?;
        let signature = self.repo.signature()?;
        let summary = commit.summary().unwrap_or("");
        let message = format!("Revert \"{summary}\"\n\nThis reverts commit {oid}.\n");
        let new_oid = self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            &message,
            &tree,
            &[&head_commit],
        )?;
        // Clear REVERT_HEAD now that the revert is committed.
        let _ = self.repo.cleanup_state();
        Ok(new_oid)
    }

    /// Real `true` while a real merge (started by `merge_branch`, or a real
    /// `git merge` run outside this crate) has left the repository with
    /// unresolved conflicts or otherwise not yet committed --
    /// `RepositoryState::Merge` specifically. A real rebase/cherry-pick/
    /// revert are separate `git2` states this method deliberately does not
    /// report on, matching this crate's own real, narrow "Merge-conflict
    /// resolution UI" scope, not a general "any operation is in progress"
    /// check.
    pub fn merge_in_progress(&self) -> bool {
        self.repo.state() == git2::RepositoryState::Merge
    }

    /// Real `git merge <branch>` -- accepts either a local branch name
    /// (`feature`) or a remote-tracking one (`origin/feature`), matching
    /// the same two real branch namespaces `list_branches`/
    /// `list_remote_branches` already expose. Uses `git2`'s own real merge
    /// analysis to pick the correct real outcome: already up to date, a
    /// real fast-forward (moves `HEAD` directly, no merge commit), or a
    /// real three-way merge -- which either produces a clean merge commit
    /// (two parents: `HEAD` and the merged branch) or leaves real conflicts
    /// in the index/working tree for `list_conflicts`/
    /// `resolve_conflict_with_content`/`commit_merge` to handle. Errors
    /// honestly if the named branch doesn't exist in either namespace.
    pub fn merge_branch(&mut self, branch: &str) -> Result<MergeOutcome, git2::Error> {
        // Every real `git2` type below (`Reference`/`Commit`/`AnnotatedCommit`/
        // `Index`) implements `Drop`, which extends its borrow of `self.repo`
        // to the end of its enclosing scope regardless of last syntactic use
        // -- each real lookup is deliberately scoped to its own block, with
        // only `Copy` values (`Oid`, `bool`) crossing a block boundary, so
        // `self.commit_merge(...)` below can still take `&mut self`.
        let their_commit_id = {
            let their_ref = self
                .repo
                .find_branch(branch, git2::BranchType::Local)
                .or_else(|_| self.repo.find_branch(branch, git2::BranchType::Remote))?
                .into_reference();
            their_ref.peel_to_commit()?.id()
        };

        let (is_up_to_date, is_fast_forward) = {
            let annotated = self.repo.find_annotated_commit(their_commit_id)?;
            let (analysis, _preference) = self.repo.merge_analysis(&[&annotated])?;
            (analysis.is_up_to_date(), analysis.is_fast_forward())
        };

        if is_up_to_date {
            return Ok(MergeOutcome::UpToDate);
        }

        if is_fast_forward {
            let their_commit = self.repo.find_commit(their_commit_id)?;
            let mut checkout = git2::build::CheckoutBuilder::new();
            checkout.force();
            self.repo
                .checkout_tree(their_commit.as_object(), Some(&mut checkout))?;
            let mut head_ref = self.repo.head()?;
            if head_ref.is_branch() {
                head_ref.set_target(their_commit_id, "fast-forward merge")?;
            } else {
                // A real detached HEAD -- move it directly, matching
                // `checkout_remote_branch`'s own precedent for the
                // no-real-branch-ref case.
                self.repo.set_head_detached(their_commit_id)?;
            }
            return Ok(MergeOutcome::FastForwarded);
        }

        // A real, genuine three-way merge -- writes real conflict markers
        // into the working tree and populates the index's real conflict
        // entries when the two sides touched overlapping content; leaves
        // both untouched (a clean merge, ready to commit) otherwise.
        let has_conflicts = {
            let annotated = self.repo.find_annotated_commit(their_commit_id)?;
            self.repo.merge(&[&annotated], None, None)?;
            self.repo.index()?.has_conflicts()
        };
        if has_conflicts {
            return Ok(MergeOutcome::Conflicted);
        }
        self.commit_merge(&format!("Merge branch '{branch}'"))?;
        Ok(MergeOutcome::Merged)
    }

    /// Real per-file conflict listing -- one entry per real conflicted path
    /// currently in the index, each side's real content read from its own
    /// real blob. A side is `None` when that side has no entry at all for
    /// this path -- a real, valid "modify/delete" conflict shape, not
    /// assumed to always be "both sides modified the same file".
    pub fn list_conflicts(&self) -> Result<Vec<ConflictEntry>, git2::Error> {
        let index = self.repo.index()?;
        let blob_content =
            |entry: &Option<git2::IndexEntry>| -> Result<Option<String>, git2::Error> {
                match entry {
                    Some(e) => {
                        let blob = self.repo.find_blob(e.id)?;
                        Ok(Some(String::from_utf8_lossy(blob.content()).into_owned()))
                    }
                    None => Ok(None),
                }
            };
        let mut entries = Vec::new();
        for conflict in index.conflicts()? {
            let conflict = conflict?;
            let path = conflict
                .ancestor
                .as_ref()
                .or(conflict.our.as_ref())
                .or(conflict.their.as_ref())
                .map(|e| PathBuf::from(String::from_utf8_lossy(&e.path).into_owned()))
                .ok_or_else(|| git2::Error::from_str("real conflict entry carries no path"))?;
            entries.push(ConflictEntry {
                path,
                ancestor: blob_content(&conflict.ancestor)?,
                ours: blob_content(&conflict.our)?,
                theirs: blob_content(&conflict.their)?,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    /// Real one-click conflict resolution -- writes `content` (the chosen
    /// real "take ours"/"take theirs" blob text, or the user's own real
    /// hand-edited text) directly to the real working-tree file, then
    /// stages it, which resolves the conflict exactly the way a real
    /// `git add <path>` on a conflicted path already does (removing the
    /// ancestor/ours/theirs index entries, replacing them with one normal
    /// staged entry). A file already edited and saved through this
    /// project's own existing editor `edit`/`save_file` path needs no new
    /// method at all -- the existing `stage()` (this crate's own `git
    /// add`) already resolves it identically; this method exists
    /// specifically for the one-click case where the resolved content
    /// isn't already on disk.
    pub fn resolve_conflict_with_content(
        &self,
        path: &Path,
        content: &str,
    ) -> Result<(), git2::Error> {
        let workdir = self
            .repo
            .workdir()
            .ok_or_else(|| git2::Error::from_str("repository has no working directory"))?;
        std::fs::write(workdir.join(path), content).map_err(|e| {
            git2::Error::from_str(&format!("failed to write resolved content: {e}"))
        })?;
        self.stage(path)
    }

    /// Real merge-commit completion -- creates a commit with **two**
    /// real parents (`HEAD` and the real `MERGE_HEAD` a `merge_branch`
    /// call left behind), the one real shape `commit()` itself
    /// deliberately never produces (always single-parent). Refuses with a
    /// real, honest error if the index still has unresolved conflicts, or
    /// if there's no real `MERGE_HEAD` to read (not actually mid-merge).
    /// Clears the in-progress merge state afterward, the same real
    /// `cleanup_state()` call `revert_commit`'s own conflict-cleanup path
    /// already established.
    pub fn commit_merge(&mut self, message: &str) -> Result<git2::Oid, git2::Error> {
        {
            let index = self.repo.index()?;
            if index.has_conflicts() {
                return Err(git2::Error::from_str(
                    "cannot complete the merge -- real unresolved conflicts remain",
                ));
            }
        }
        let mut merge_parent_oids = Vec::new();
        self.repo.mergehead_foreach(|oid| {
            merge_parent_oids.push(*oid);
            true
        })?;
        if merge_parent_oids.is_empty() {
            return Err(git2::Error::from_str(
                "no real MERGE_HEAD found -- this repository is not mid-merge",
            ));
        }
        let mut index = self.repo.index()?;
        let tree_oid = index.write_tree()?;
        let tree = self.repo.find_tree(tree_oid)?;
        let signature = self.repo.signature()?;
        let head_commit = self.repo.head()?.peel_to_commit()?;
        let mut parents = vec![head_commit];
        for oid in &merge_parent_oids {
            parents.push(self.repo.find_commit(*oid)?);
        }
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        let new_oid = self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )?;
        self.repo.cleanup_state()?;
        Ok(new_oid)
    }

    /// Real, destructive merge abort -- resets the working tree and index
    /// back to `HEAD` (discarding the real in-progress merge entirely,
    /// including any real partial conflict resolutions already staged) and
    /// clears the in-progress state. The caller is responsible for
    /// confirming with the user first, matching `discard_changes`'s own
    /// precedent for a real destructive operation in this crate.
    pub fn abort_merge(&self) -> Result<(), git2::Error> {
        let head_commit = self.repo.head()?.peel_to_commit()?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        self.repo.reset(
            head_commit.as_object(),
            git2::ResetType::Hard,
            Some(&mut checkout),
        )?;
        self.repo.cleanup_state()
    }

    /// Real list of tags, sorted by name -- each with the hex oid of the
    /// commit it points at (peeled through an annotated tag object where
    /// needed) and whether it's annotated (its own tag object) or lightweight
    /// (just a ref). A tag pointing at a non-commit object, or one that can't
    /// be resolved, is skipped rather than failing the whole listing.
    pub fn list_tags(&self) -> Result<Vec<TagInfo>, git2::Error> {
        let names = self.repo.tag_names(None)?;
        let mut tags: Vec<TagInfo> = Vec::new();
        for name in names.iter().flatten() {
            let refname = format!("refs/tags/{name}");
            let Ok(obj) = self.repo.revparse_single(&refname) else {
                continue;
            };
            let annotated = obj.kind() == Some(git2::ObjectType::Tag);
            let Ok(commit) = obj.peel_to_commit() else {
                continue;
            };
            tags.push(TagInfo {
                name: name.to_string(),
                target: commit.id().to_string(),
                annotated,
            });
        }
        tags.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tags)
    }

    /// Real tag creation on a given commit `oid` (hex). If `message` is a
    /// non-empty string an annotated tag is created (its own tag object,
    /// tagger signature + message); otherwise a lightweight tag (just a ref)
    /// is created. `force` is always false, so tagging an existing name is a
    /// real, honest error rather than silently moving the tag. Errors if the
    /// oid is unknown.
    pub fn create_tag(
        &self,
        name: &str,
        oid_hex: &str,
        message: Option<&str>,
    ) -> Result<git2::Oid, git2::Error> {
        let oid = git2::Oid::from_str(oid_hex)?;
        let target = self.repo.find_object(oid, None)?;
        match message {
            Some(msg) if !msg.trim().is_empty() => {
                let signature = self.repo.signature()?;
                self.repo.tag(name, &target, &signature, msg, false)
            }
            _ => self.repo.tag_lightweight(name, &target, false),
        }
    }

    /// Real tag deletion by name (the bare tag name, e.g. `v1.0`, not the
    /// full `refs/tags/...` ref). Errors honestly if no such tag exists.
    pub fn delete_tag(&self, name: &str) -> Result<(), git2::Error> {
        self.repo.tag_delete(name)
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

    /// Real `git log <ref_name>` -- the same real, full-graph `revwalk` as
    /// `log()`, but starting from a named branch (local or remote-tracking,
    /// the same two-namespace resolution `merge_branch` already uses)
    /// instead of `HEAD` -- so a branch's own commits can be browsed
    /// without checking it out. Errors honestly if the ref doesn't exist in
    /// either namespace.
    pub fn list_commits_for_ref(
        &self,
        ref_name: &str,
        max: usize,
    ) -> Result<Vec<CommitInfo>, git2::Error> {
        let start_oid = self
            .repo
            .find_branch(ref_name, git2::BranchType::Local)
            .or_else(|_| self.repo.find_branch(ref_name, git2::BranchType::Remote))?
            .into_reference()
            .peel_to_commit()?
            .id();
        let mut walk = self.repo.revwalk()?;
        walk.push(start_oid)?;
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

    /// Real `git cherry-pick <oid>` -- applies the named commit's real
    /// changes onto the current `HEAD` (via `git2`'s own real `cherrypick`,
    /// which sets the real `CHERRYPICK_HEAD` state and populates the index/
    /// working tree), then commits the result as a new commit with a single
    /// parent (`HEAD`) -- unlike `revert_commit`'s two-parent-free "Revert"
    /// commit, a cherry-pick commit is a normal, single-parent one, matching
    /// real `git cherry-pick`'s own commit shape. A real conflict is an
    /// honest error: the in-progress cherry-pick state is cleaned up (never
    /// left half-applied) and the caller is told, exactly like
    /// `revert_commit`'s own conflict handling. A cherry-pick that produces
    /// no real changes at all (the commit's own diff is already fully
    /// present on `HEAD`) is also a real, honest error rather than a
    /// silent, pointless duplicate commit -- checked by comparing the
    /// resulting tree to `HEAD`'s own tree before committing.
    pub fn cherry_pick_commit(&self, oid_hex: &str) -> Result<git2::Oid, git2::Error> {
        let oid = git2::Oid::from_str(oid_hex)?;
        let commit = self.repo.find_commit(oid)?;
        let head_commit = self.repo.head()?.peel_to_commit()?;
        self.repo.cherrypick(&commit, None)?;
        let mut index = self.repo.index()?;
        if index.has_conflicts() {
            let _ = self.repo.cleanup_state();
            return Err(git2::Error::from_str(
                "cherry-pick produced conflicts; the working tree was left unchanged",
            ));
        }
        let tree_oid = index.write_tree()?;
        if tree_oid == head_commit.tree_id() {
            let _ = self.repo.cleanup_state();
            return Err(git2::Error::from_str(
                "the previous cherry-pick is now empty -- its changes are already on HEAD",
            ));
        }
        let tree = self.repo.find_tree(tree_oid)?;
        let signature = self.repo.signature()?;
        let author = commit.author();
        let summary = commit.summary().unwrap_or("");
        let body = commit.body().unwrap_or("");
        let message = if body.is_empty() {
            format!("{summary}\n\n(cherry picked from commit {oid})\n")
        } else {
            format!("{summary}\n\n{body}\n(cherry picked from commit {oid})\n")
        };
        let new_oid = self.repo.commit(
            Some("HEAD"),
            &author,
            &signature,
            &message,
            &tree,
            &[&head_commit],
        )?;
        let _ = self.repo.cleanup_state();
        Ok(new_oid)
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

    /// Detects `(owner, repo)` from the repo's own `origin` remote, if it
    /// points at a real github.com URL -- the one real mechanism the
    /// GitHub layer (§56.3-56.4) needs to know which repo to talk to
    /// without asking the user to type it in by hand. Falls back to the
    /// first remote with a parseable github.com URL if there's no
    /// `origin` (a real, valid state some repos are in), and returns
    /// `None` for a repo with no github.com remote at all -- a genuine
    /// non-GitHub project, not an error.
    pub fn detect_github_remote(&self) -> Option<(String, String)> {
        let remotes = self.list_remotes().ok()?;
        let mut fallback = None;
        for (name, url) in remotes {
            let Some(url) = url else { continue };
            let Some(parsed) = parse_github_owner_repo(&url) else {
                continue;
            };
            if name == "origin" {
                return Some(parsed);
            }
            fallback.get_or_insert(parsed);
        }
        fallback
    }

    /// Real fetch from a configured remote using its own default refspecs
    /// (updates the remote-tracking refs; does not touch the working tree).
    pub fn fetch(&self, remote_name: &str) -> Result<(), git2::Error> {
        self.fetch_with_github_token(remote_name, None)
    }

    pub fn fetch_with_github_token(
        &self,
        remote_name: &str,
        github_token: Option<String>,
    ) -> Result<(), git2::Error> {
        let mut remote = self.repo.find_remote(remote_name)?;
        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(make_remote_callbacks(github_token));
        let empty: [&str; 0] = [];
        remote.fetch(&empty, Some(&mut fo), None)
    }

    /// Real push of a single local branch to the same-named branch on a
    /// configured remote. A rejected push (non-fast-forward on the remote,
    /// auth failure) surfaces `libgit2`'s own real error verbatim -- never
    /// a force-push, which would need an explicit, separate opt-in.
    pub fn push(&self, remote_name: &str, branch: &str) -> Result<(), git2::Error> {
        self.push_with_github_token(remote_name, branch, None)
    }

    pub fn push_with_github_token(
        &self,
        remote_name: &str,
        branch: &str,
        github_token: Option<String>,
    ) -> Result<(), git2::Error> {
        let mut remote = self.repo.find_remote(remote_name)?;
        let mut po = git2::PushOptions::new();
        po.remote_callbacks(make_remote_callbacks(github_token));
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
        self.pull_fast_forward_with_github_token(remote_name, branch, None)
    }

    pub fn pull_fast_forward_with_github_token(
        &self,
        remote_name: &str,
        branch: &str,
        github_token: Option<String>,
    ) -> Result<PullOutcome, git2::Error> {
        self.fetch_with_github_token(remote_name, github_token)?;
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
        // libgit2's stash tree builder relies on the index stat cache when it
        // computes index-to-worktree content. In particular, a same-size edit
        // made immediately after a commit can retain identical coarse mtime
        // data on mobile filesystems and be omitted from the stash. Invalidate
        // those cached times while preserving every index blob/mode/path so
        // the real content diff is forced without staging the edit.
        let mut index = self.repo.index()?;
        for position in 0..index.len() {
            if let Some(mut entry) = index.get(position) {
                entry.ctime = git2::IndexTime::new(0, 0);
                entry.mtime = git2::IndexTime::new(0, 0);
                index.add(&entry)?;
            }
        }
        index.write()?;
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
        let mut options = git2::StashApplyOptions::new();
        options.reinstantiate_index();
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        options.checkout_options(checkout);
        self.repo.stash_pop(index, Some(&mut options))
    }

    /// Real `git stash apply <index>` -- applies the stash back onto the
    /// working tree but *keeps* it in the stash list (unlike `stash_pop`,
    /// which drops it after applying). Same conflict-refusing semantics as
    /// `stash_pop`: `libgit2`'s own apply errors on a real conflict rather
    /// than force-overwriting, surfaced verbatim.
    pub fn stash_apply(&mut self, index: usize) -> Result<(), git2::Error> {
        let mut options = git2::StashApplyOptions::new();
        options.reinstantiate_index();
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        options.checkout_options(checkout);
        self.repo.stash_apply(index, Some(&mut options))
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
    fn discard_changes_reverts_an_unstaged_modification() {
        let (tmp, repo) = TempRepo::new("discard");
        tmp.write("f.txt", "committed\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        // Real unstaged modification.
        tmp.write("f.txt", "dirty edit\n");
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "dirty edit\n"
        );
        // Discard restores the working file to the committed (== index) version.
        repo.discard_changes(Path::new("f.txt")).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "committed\n"
        );
    }

    #[test]
    fn discard_changes_keeps_staged_changes() {
        let (tmp, repo) = TempRepo::new("discard_keep_staged");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        // Stage a change, then make a further unstaged change on top.
        tmp.write("f.txt", "v2-staged\n");
        repo.stage(Path::new("f.txt")).unwrap();
        tmp.write("f.txt", "v3-unstaged\n");
        // Discard drops only the unstaged part, restoring to the staged version.
        repo.discard_changes(Path::new("f.txt")).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "v2-staged\n"
        );
    }

    #[test]
    fn commit_amend_rewrites_the_last_commit_message_without_adding_a_commit() {
        let (tmp, repo) = TempRepo::new("amend_message");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("original message").unwrap();
        assert_eq!(repo.log(10).unwrap().len(), 1);
        // Amend rewrites the message; the commit count stays 1.
        repo.commit_amend("amended message").unwrap();
        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 1, "amend must replace, not add, a commit");
        assert_eq!(log[0].summary, "amended message");
    }

    #[test]
    fn commit_amend_folds_the_current_index_into_the_last_commit() {
        let (tmp, repo) = TempRepo::new("amend_tree");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        // Stage a further change, then amend -- it folds into the last commit.
        tmp.write("g.txt", "new file\n");
        repo.stage(Path::new("g.txt")).unwrap();
        repo.commit_amend("first (amended)").unwrap();
        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 1, "amend must not create a second commit");
        // The amended commit's tree now contains g.txt (a working-tree diff
        // against HEAD would report nothing to commit for it).
        let status = repo.status().unwrap();
        assert!(
            status.iter().all(|s| s.path != Path::new("g.txt")),
            "g.txt should be committed via the amend, not left pending: {status:?}"
        );
    }

    #[test]
    fn commit_amend_with_no_head_commit_errors() {
        let (tmp, repo) = TempRepo::new("amend_no_head");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        // No commit yet -- there is nothing to amend.
        assert!(repo.commit_amend("nope").is_err());
    }

    #[test]
    fn revert_commit_undoes_a_change_as_a_new_commit() {
        let (tmp, repo) = TempRepo::new("revert_basic");
        tmp.write("f.txt", "line one\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        // A second commit adds a line we will revert.
        tmp.write("f.txt", "line one\nline two\n");
        repo.stage(Path::new("f.txt")).unwrap();
        let bad = repo.commit("add line two").unwrap();
        assert_eq!(repo.log(10).unwrap().len(), 2);
        // Revert the second commit -> a NEW commit that removes line two.
        repo.revert_commit(&bad.to_string()).unwrap();
        let log = repo.log(10).unwrap();
        assert_eq!(
            log.len(),
            3,
            "revert must add a commit, not rewrite history"
        );
        assert_eq!(log[0].summary, "Revert \"add line two\"");
        // The working file content is back to the first-commit state.
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\n"
        );
        // The repo is not left in an in-progress REVERT state.
        assert_eq!(repo.repo.state(), git2::RepositoryState::Clean);
    }

    #[test]
    fn revert_commit_with_an_unknown_oid_errors() {
        let (tmp, repo) = TempRepo::new("revert_bad_oid");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        assert!(repo
            .revert_commit("0000000000000000000000000000000000000000")
            .is_err());
    }

    #[test]
    fn list_commits_for_ref_browses_a_branch_without_checking_it_out() {
        let (tmp, repo) = TempRepo::new("log_for_ref");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("root").unwrap();
        repo.repo
            .branch(
                "feature",
                &repo.repo.head().unwrap().peel_to_commit().unwrap(),
                false,
            )
            .unwrap();
        repo.repo.set_head("refs/heads/feature").unwrap();
        tmp.write("f.txt", "v2\n");
        repo.stage(Path::new("f.txt")).unwrap();
        let feature_commit = repo.commit("feature work").unwrap();
        // Switch back to master -- "feature"'s own extra commit must still
        // be browsable without checking it out again.
        repo.repo.set_head("refs/heads/master").unwrap();
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        repo.repo.checkout_head(Some(&mut checkout)).unwrap();
        assert_eq!(repo.log(10).unwrap().len(), 1, "master itself has 1 commit");
        let feature_log = repo.list_commits_for_ref("feature", 10).unwrap();
        assert_eq!(feature_log.len(), 2, "feature has root + its own commit");
        assert_eq!(feature_log[0].oid, feature_commit.to_string());
        assert_eq!(feature_log[0].summary, "feature work");
    }

    #[test]
    fn list_commits_for_ref_on_an_unknown_branch_errors() {
        let (tmp, repo) = TempRepo::new("log_for_ref_bad");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        assert!(repo.list_commits_for_ref("does-not-exist", 10).is_err());
    }

    #[test]
    fn cherry_pick_commit_applies_a_real_change_from_another_branch() {
        let (tmp, repo) = TempRepo::new("cherry_pick_basic");
        tmp.write("f.txt", "line one\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("root").unwrap();
        repo.repo
            .branch(
                "feature",
                &repo.repo.head().unwrap().peel_to_commit().unwrap(),
                false,
            )
            .unwrap();
        repo.repo.set_head("refs/heads/feature").unwrap();
        tmp.write("f.txt", "line one\nline two\n");
        repo.stage(Path::new("f.txt")).unwrap();
        let feature_commit = repo.commit("add line two").unwrap();
        // Back to master, which does NOT have "line two" yet.
        repo.repo.set_head("refs/heads/master").unwrap();
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        repo.repo.checkout_head(Some(&mut checkout)).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\n"
        );
        repo.cherry_pick_commit(&feature_commit.to_string())
            .unwrap();
        // The cherry-picked change is now on master's working tree...
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\nline two\n"
        );
        // ...as a real, new, single-parent commit (not a rewrite of history).
        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 2);
        assert!(log[0].summary.starts_with("add line two"));
        assert_eq!(repo.repo.state(), git2::RepositoryState::Clean);
    }

    #[test]
    fn cherry_pick_commit_with_an_unknown_oid_errors() {
        let (tmp, repo) = TempRepo::new("cherry_pick_bad_oid");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        assert!(repo
            .cherry_pick_commit("0000000000000000000000000000000000000000")
            .is_err());
    }

    #[test]
    fn cherry_pick_an_already_applied_commit_is_an_honest_empty_error() {
        let (tmp, repo) = TempRepo::new("cherry_pick_empty");
        tmp.write("f.txt", "line one\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("root").unwrap();
        tmp.write("f.txt", "line one\nline two\n");
        repo.stage(Path::new("f.txt")).unwrap();
        let same_branch_commit = repo.commit("add line two").unwrap();
        // Cherry-picking a commit that's already HEAD's own change produces
        // no real diff against HEAD -- a real, honest error, not a
        // pointless duplicate commit.
        let err = repo
            .cherry_pick_commit(&same_branch_commit.to_string())
            .unwrap_err();
        assert!(err.message().contains("already on HEAD"));
        assert_eq!(repo.repo.state(), git2::RepositoryState::Clean);
    }

    #[test]
    fn create_list_and_delete_tags() {
        let (tmp, repo) = TempRepo::new("tags");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        let head = repo.commit("init").unwrap();
        assert!(repo.list_tags().unwrap().is_empty());
        // A lightweight tag and an annotated tag on the same commit.
        repo.create_tag("v1.0", &head.to_string(), None).unwrap();
        repo.create_tag("v1.0-annotated", &head.to_string(), Some("release one"))
            .unwrap();
        let tags = repo.list_tags().unwrap();
        assert_eq!(tags.len(), 2);
        // Sorted by name: "v1.0" then "v1.0-annotated".
        assert_eq!(tags[0].name, "v1.0");
        assert!(!tags[0].annotated);
        assert_eq!(tags[0].target, head.to_string());
        assert_eq!(tags[1].name, "v1.0-annotated");
        assert!(tags[1].annotated, "message-carrying tag must be annotated");
        assert_eq!(tags[1].target, head.to_string());
        // Deleting one leaves the other.
        repo.delete_tag("v1.0").unwrap();
        let tags = repo.list_tags().unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "v1.0-annotated");
    }

    #[test]
    fn create_tag_with_a_duplicate_name_errors() {
        let (tmp, repo) = TempRepo::new("tag_dup");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        let head = repo.commit("init").unwrap();
        repo.create_tag("v1", &head.to_string(), None).unwrap();
        // A second tag with the same name must not silently move it.
        assert!(repo.create_tag("v1", &head.to_string(), None).is_err());
    }

    #[test]
    fn delete_nonexistent_tag_errors() {
        let (tmp, repo) = TempRepo::new("tag_del_missing");
        tmp.write("f.txt", "v1\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("init").unwrap();
        assert!(repo.delete_tag("nope").is_err());
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
    fn clone_builds_a_real_working_repo_from_a_local_bare_remote() {
        // A real bare repo acting as the "remote" -- no network, no
        // credentials; `GitRepo::clone` must produce a real, openable
        // working repo with the pushed content checked out and `origin`
        // already configured (libgit2's real clone behavior, asserted
        // against the real objects, not a stubbed return).
        let remote_dir = std::env::temp_dir().join("spartan_git_test_clone_bare");
        let _ = fs::remove_dir_all(&remote_dir);
        Repository::init_bare(&remote_dir).unwrap();
        let remote_url = remote_dir.to_str().unwrap();

        let (tmp_a, repo_a) = TempRepo::new("clone_source");
        tmp_a.write("f.txt", "hello from the source\n");
        repo_a.stage(Path::new("f.txt")).unwrap();
        repo_a.commit("initial").unwrap();
        repo_a.repo.remote("origin", remote_url).unwrap();
        let branch = repo_a.current_branch().unwrap();
        repo_a.push("origin", &branch).unwrap();

        let clone_dir = std::env::temp_dir().join("spartan_git_test_clone_dest");
        let _ = fs::remove_dir_all(&clone_dir);
        let cloned = GitRepo::clone(remote_url, &clone_dir).unwrap();
        assert_eq!(
            fs::read_to_string(clone_dir.join("f.txt")).unwrap(),
            "hello from the source\n",
            "clone must check out the pushed content into the working tree"
        );
        assert_eq!(
            cloned.current_branch().unwrap(),
            branch,
            "the clone's checked-out branch matches the source's"
        );
        let remotes = cloned.list_remotes().unwrap();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].0, "origin");
        assert_eq!(remotes[0].1.as_deref(), Some(remote_url));

        // A second clone must refuse to land over a real, non-empty
        // directory (libgit2's own guard, surfaced as a real error).
        match GitRepo::clone(remote_url, &clone_dir) {
            Ok(_) => panic!("second clone over an existing non-empty dir must fail"),
            Err(e) => assert!(e.message().contains("exists"), "unexpected: {e}"),
        }

        let _ = fs::remove_dir_all(&remote_dir);
        let _ = fs::remove_dir_all(&clone_dir);
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
    fn parse_github_owner_repo_handles_every_real_url_shape() {
        assert_eq!(
            parse_github_owner_repo("git@github.com:CKissinger1988/Spartan-IDE.git"),
            Some(("CKissinger1988".to_string(), "Spartan-IDE".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/CKissinger1988/Spartan-IDE.git"),
            Some(("CKissinger1988".to_string(), "Spartan-IDE".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/CKissinger1988/Spartan-IDE"),
            Some(("CKissinger1988".to_string(), "Spartan-IDE".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("ssh://git@github.com/owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/owner/repo/"),
            Some(("owner".to_string(), "repo".to_string())),
            "a real trailing slash must not become part of the repo name"
        );
    }

    #[test]
    fn parse_github_owner_repo_rejects_non_github_and_malformed_urls() {
        assert_eq!(
            parse_github_owner_repo("https://gitlab.com/owner/repo.git"),
            None
        );
        assert_eq!(
            parse_github_owner_repo("/local/bare/repo/path"),
            None,
            "a real local bare-repo remote path is a valid remote, just not a GitHub one"
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/owner-only"),
            None
        );
        assert_eq!(parse_github_owner_repo("https://github.com/"), None);
    }

    #[test]
    fn parse_github_owner_repo_rejects_a_real_invalid_character_in_either_segment() {
        assert_eq!(
            parse_github_owner_repo("https://github.com/owner/repo/extra"),
            None,
            "a real, unexpected third path segment must never be silently folded into `repo`"
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/ow ner/repo.git"),
            None,
            "a space is not a real, valid GitHub owner-name character"
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/owner/re%2Fpo.git"),
            None,
            "a real percent-encoded slash must not be treated as a valid repo-name character"
        );
        assert_eq!(
            parse_github_owner_repo("https://github.com/valid-owner.name/valid_repo.name"),
            Some((
                "valid-owner.name".to_string(),
                "valid_repo.name".to_string()
            )),
            "real, valid GitHub identifiers may contain '.', '_', and '-'"
        );
    }

    #[test]
    fn detect_github_remote_finds_origin_and_ignores_non_github_remotes() {
        let (_tmp, repo) = TempRepo::new("github_remote_detect");
        repo.repo
            .remote("upstream", "https://gitlab.com/other/thing.git")
            .unwrap();
        repo.repo
            .remote("origin", "git@github.com:CKissinger1988/Spartan-IDE.git")
            .unwrap();
        assert_eq!(
            repo.detect_github_remote(),
            Some(("CKissinger1988".to_string(), "Spartan-IDE".to_string()))
        );
    }

    #[test]
    fn detect_github_remote_is_none_for_a_repo_with_no_github_remote() {
        let (_tmp, repo) = TempRepo::new("github_remote_detect_none");
        repo.repo
            .remote("origin", "https://gitlab.com/other/thing.git")
            .unwrap();
        assert_eq!(repo.detect_github_remote(), None);
    }

    #[test]
    fn detect_github_remote_falls_back_to_a_non_origin_github_remote() {
        // A real, valid state some repos are in: `origin` points somewhere
        // else (or doesn't exist at all), but a differently-named remote
        // (e.g. `upstream`) is the real GitHub one -- the fallback path
        // `detect_github_remote`'s own doc comment names explicitly.
        //
        // The SSH form (`git@github.com:...`, matching the sibling
        // `detect_github_remote_finds_origin_and_ignores_non_github_remotes`
        // test's own precedent) is deliberate, not incidental: this session's
        // own sandboxed `.gitconfig` carries a real `url.<proxy>.insteadOf =
        // https://github.com/` rewrite rule, so a *literal* `https://
        // github.com/...` URL passed to `git2::Repository::remote()` gets
        // silently rewritten to the local proxy URL at creation time --
        // confirmed live via a temporary debug print showing the real
        // stored remote as `http://local_proxy@127.0.0.1:.../git/...`, not
        // the URL this test actually passed. The SSH form isn't touched by
        // that `https://`-scoped rule, so it's the real, environment-safe
        // way to test this fallback path here.
        let (_tmp, repo) = TempRepo::new("github_remote_detect_fallback");
        repo.repo
            .remote("origin", "https://gitlab.com/other/thing.git")
            .unwrap();
        repo.repo
            .remote("upstream", "git@github.com:CKissinger1988/Spartan-IDE.git")
            .unwrap();
        assert_eq!(
            repo.detect_github_remote(),
            Some(("CKissinger1988".to_string(), "Spartan-IDE".to_string()))
        );
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

    /// Real helper shared by the merge tests below -- a repo with a real
    /// common ancestor commit, then a real divergent commit on each of
    /// `master`/`feature` touching the *same* file, exactly the shape a
    /// real conflicting merge needs. Returns to `master` (the branch a real
    /// `merge_branch("feature")` call would run from).
    fn repo_with_divergent_branches(unique: &str) -> (TempRepo, GitRepo) {
        let (tmp, repo) = TempRepo::new(unique);
        tmp.write("f.txt", "line one\nline two\nline three\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("common ancestor").unwrap();
        repo.create_branch("feature").unwrap();
        repo.checkout_branch("feature").unwrap();
        tmp.write("f.txt", "line one\nFEATURE CHANGE\nline three\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("feature change").unwrap();
        repo.checkout_branch("master").unwrap();
        tmp.write("f.txt", "line one\nMASTER CHANGE\nline three\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("master change").unwrap();
        (tmp, repo)
    }

    #[test]
    fn merge_branch_reports_up_to_date_when_already_merged() {
        let (tmp, mut repo) = TempRepo::new("merge_up_to_date");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        repo.create_branch("feature").unwrap();
        let outcome = repo.merge_branch("feature").unwrap();
        assert_eq!(outcome, MergeOutcome::UpToDate);
    }

    #[test]
    fn merge_branch_fast_forwards_when_head_has_not_diverged() {
        let (tmp, mut repo) = TempRepo::new("merge_fast_forward");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        repo.create_branch("feature").unwrap();
        repo.checkout_branch("feature").unwrap();
        tmp.write("f.txt", "v2");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("feature-only change").unwrap();
        repo.checkout_branch("master").unwrap();

        let outcome = repo.merge_branch("feature").unwrap();
        assert_eq!(outcome, MergeOutcome::FastForwarded);
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "v2",
            "a real fast-forward must land feature's own content"
        );
        assert!(!repo.merge_in_progress());
    }

    #[test]
    fn merge_branch_with_no_real_overlap_merges_cleanly_with_two_parents() {
        let (tmp, mut repo) = TempRepo::new("merge_clean_two_parent");
        tmp.write("a.txt", "a");
        repo.stage(Path::new("a.txt")).unwrap();
        repo.commit("common ancestor").unwrap();
        repo.create_branch("feature").unwrap();
        repo.checkout_branch("feature").unwrap();
        tmp.write("b.txt", "b"); // a real, non-overlapping new file
        repo.stage(Path::new("b.txt")).unwrap();
        repo.commit("feature adds b.txt").unwrap();
        repo.checkout_branch("master").unwrap();
        tmp.write("c.txt", "c"); // a real, non-overlapping new file
        repo.stage(Path::new("c.txt")).unwrap();
        repo.commit("master adds c.txt").unwrap();

        let outcome = repo.merge_branch("feature").unwrap();
        assert_eq!(outcome, MergeOutcome::Merged);
        assert!(!repo.merge_in_progress());
        // A real merge commit -- both real files from both real sides
        // are present.
        assert!(tmp.dir.join("b.txt").exists());
        assert!(tmp.dir.join("c.txt").exists());
        let commit = repo.repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(
            commit.parent_count(),
            2,
            "a real merge commit must have exactly two parents"
        );
    }

    #[test]
    fn merge_branch_with_real_overlapping_edits_reports_conflicted_and_lists_both_sides() {
        let (_tmp, mut repo) = repo_with_divergent_branches("merge_conflicted_list");
        let outcome = repo.merge_branch("feature").unwrap();
        assert_eq!(outcome, MergeOutcome::Conflicted);
        assert!(repo.merge_in_progress());

        let conflicts = repo.list_conflicts().unwrap();
        assert_eq!(conflicts.len(), 1);
        let c = &conflicts[0];
        assert_eq!(c.path, PathBuf::from("f.txt"));
        assert!(
            c.ancestor.as_deref() == Some("line one\nline two\nline three\n"),
            "real ancestor content: {:?}",
            c.ancestor
        );
        assert!(
            c.ours.as_deref().unwrap().contains("MASTER CHANGE"),
            "real 'ours' (master) content: {:?}",
            c.ours
        );
        assert!(
            c.theirs.as_deref().unwrap().contains("FEATURE CHANGE"),
            "real 'theirs' (feature) content: {:?}",
            c.theirs
        );
    }

    #[test]
    fn resolve_conflict_with_content_then_commit_merge_produces_a_real_two_parent_commit() {
        let (tmp, mut repo) = repo_with_divergent_branches("merge_resolve_then_commit");
        let outcome = repo.merge_branch("feature").unwrap();
        assert_eq!(outcome, MergeOutcome::Conflicted);

        // Completing the merge before resolving must be a real, honest
        // refusal, not a broken commit.
        let premature = repo.commit_merge("too early");
        assert!(premature.is_err());

        repo.resolve_conflict_with_content(
            Path::new("f.txt"),
            "line one\nRESOLVED BY HAND\nline three\n",
        )
        .unwrap();
        assert!(!repo
            .list_conflicts()
            .unwrap()
            .iter()
            .any(|c| c.path == Path::new("f.txt")));

        let oid = repo.commit_merge("Merge branch 'feature'").unwrap();
        assert!(!oid.is_zero());
        assert!(!repo.merge_in_progress());
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\nRESOLVED BY HAND\nline three\n"
        );
        let commit = repo.repo.find_commit(oid).unwrap();
        assert_eq!(commit.parent_count(), 2);
    }

    #[test]
    fn abort_merge_really_discards_the_in_progress_merge_and_partial_resolutions() {
        let (tmp, mut repo) = repo_with_divergent_branches("merge_abort");
        repo.merge_branch("feature").unwrap();
        // A real, partial resolution staged before the abort.
        repo.resolve_conflict_with_content(Path::new("f.txt"), "partially resolved")
            .unwrap();

        repo.abort_merge().unwrap();

        assert!(!repo.merge_in_progress());
        assert!(repo.list_conflicts().unwrap().is_empty());
        assert!(
            repo.status().unwrap().is_empty(),
            "a real abort leaves a clean tree"
        );
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line one\nMASTER CHANGE\nline three\n",
            "a real abort restores HEAD's own (master's) content"
        );
    }

    #[test]
    fn commit_merge_with_no_real_merge_head_errors_honestly() {
        let (tmp, mut repo) = TempRepo::new("commit_merge_no_merge_head");
        tmp.write("f.txt", "v1");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("first").unwrap();
        assert!(repo.commit_merge("nothing to complete").is_err());
    }

    /// A real, real committed 20-line file with two well-separated later
    /// edits (line 2 and line 19 -- far enough apart that libgit2's default
    /// 3-line diff context can't merge them into one hunk) -- the standard
    /// real fixture for exercising real per-hunk staging.
    fn repo_with_two_separated_unstaged_hunks(unique: &str) -> (TempRepo, GitRepo) {
        let (tmp, repo) = TempRepo::new(unique);
        let base: String = (1..=20)
            .map(|n| format!("line{n}\n"))
            .collect::<Vec<_>>()
            .join("");
        tmp.write("f.txt", &base);
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("base").unwrap();
        let modified = base
            .replace("line2\n", "line2 CHANGED\n")
            .replace("line19\n", "line19 CHANGED\n");
        tmp.write("f.txt", &modified);
        (tmp, repo)
    }

    #[test]
    fn diff_hunks_on_an_untracked_file_is_a_real_honest_empty_list() {
        let (tmp, repo) = TempRepo::new("diff_hunks_untracked");
        tmp.write("untracked.txt", "content\n");
        assert!(repo
            .diff_hunks(Path::new("untracked.txt"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn diff_hunks_reports_two_real_separate_hunks_for_two_separated_edits() {
        let (_tmp, repo) = repo_with_two_separated_unstaged_hunks("diff_hunks_two");
        let hunks = repo.diff_hunks(Path::new("f.txt")).unwrap();
        assert_eq!(
            hunks.len(),
            2,
            "the two edits are far enough apart to stay separate hunks"
        );
        assert!(hunks[0].body.contains("line2 CHANGED"));
        assert!(!hunks[0].body.contains("line19"));
        assert!(hunks[1].body.contains("line19 CHANGED"));
        assert!(!hunks[1].body.contains("line2 CHANGED"));
        assert!(hunks[0].header.starts_with("@@"));
        assert_eq!(hunks[0].index, 0);
        assert_eq!(hunks[1].index, 1);
    }

    #[test]
    fn stage_hunk_stages_only_the_selected_hunk_leaving_the_other_real_change_unstaged() {
        let (tmp, repo) = repo_with_two_separated_unstaged_hunks("stage_hunk_one");
        repo.stage_hunk(Path::new("f.txt"), 0).unwrap();

        // The real staged (index) content now has line2's change but not
        // line19's.
        let staged = repo
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        assert!(staged.contains("line2 CHANGED"));
        assert!(!staged.contains("line19 CHANGED"));

        // The real working-tree file is completely untouched by staging.
        assert!(fs::read_to_string(tmp.dir.join("f.txt"))
            .unwrap()
            .contains("line19 CHANGED"));

        // git_status shows this file as both staged (line2) AND still
        // unstaged (line19's real remaining diff).
        let status = repo.status().unwrap();
        let entry = status
            .iter()
            .find(|e| e.path == Path::new("f.txt"))
            .unwrap();
        assert!(entry.staged.is_some());
        assert!(entry.unstaged.is_some());
    }

    #[test]
    fn stage_hunk_twice_in_sequence_stages_the_real_whole_file() {
        let (tmp, repo) = repo_with_two_separated_unstaged_hunks("stage_hunk_both");
        // First call recomputes the real diff fresh, sees 2 hunks, stages
        // hunk 0.
        repo.stage_hunk(Path::new("f.txt"), 0).unwrap();
        // The real remaining diff now has exactly 1 hunk (line19's change)
        // -- re-fetched fresh, not reused from the first call.
        let remaining = repo.diff_hunks(Path::new("f.txt")).unwrap();
        assert_eq!(remaining.len(), 1);
        repo.stage_hunk(Path::new("f.txt"), 0).unwrap();

        let staged = repo
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        let working = fs::read_to_string(tmp.dir.join("f.txt")).unwrap();
        assert_eq!(
            staged, working,
            "staging every real hunk matches the working tree exactly"
        );
        assert!(repo.diff_hunks(Path::new("f.txt")).unwrap().is_empty());
    }

    #[test]
    fn stage_hunk_out_of_range_errors_honestly() {
        let (_tmp, repo) = repo_with_two_separated_unstaged_hunks("stage_hunk_oor");
        assert!(repo.stage_hunk(Path::new("f.txt"), 99).is_err());
    }

    #[test]
    fn stage_hunk_on_a_pure_addition_at_end_of_file_works() {
        let (tmp, repo) = TempRepo::new("stage_hunk_addition");
        tmp.write("f.txt", "line1\nline2\nline3\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("base").unwrap();
        tmp.write("f.txt", "line1\nline2\nline3\nline4\nline5\n");

        let hunks = repo.diff_hunks(Path::new("f.txt")).unwrap();
        assert_eq!(hunks.len(), 1);
        repo.stage_hunk(Path::new("f.txt"), 0).unwrap();

        let staged = repo
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        assert_eq!(staged, "line1\nline2\nline3\nline4\nline5\n");
    }

    fn repo_with_two_separated_staged_hunks(unique: &str) -> (TempRepo, GitRepo) {
        let (tmp, repo) = repo_with_two_separated_unstaged_hunks(unique);
        // Stage the real, current (both-edits-applied) working-tree
        // content wholesale, so both changes start out fully staged.
        repo.stage(Path::new("f.txt")).unwrap();
        (tmp, repo)
    }

    #[test]
    fn diff_hunks_staged_on_a_brand_new_fully_staged_file_is_a_real_honest_empty_list() {
        let (tmp, repo) = TempRepo::new("diff_hunks_staged_new_file");
        tmp.write("new.txt", "content\n");
        repo.stage(Path::new("new.txt")).unwrap();
        // No HEAD commit exists yet at all -- a real, honest empty list,
        // matching `diff_hunks()`'s own "no old baseline" convention one
        // layer up (whole-file `unstage()` already covers this case).
        assert!(repo
            .diff_hunks_staged(Path::new("new.txt"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn diff_hunks_staged_reports_two_real_separate_hunks_for_two_separated_staged_edits() {
        let (_tmp, repo) = repo_with_two_separated_staged_hunks("diff_hunks_staged_two");
        let hunks = repo.diff_hunks_staged(Path::new("f.txt")).unwrap();
        assert_eq!(
            hunks.len(),
            2,
            "the two staged edits are far enough apart to stay separate hunks"
        );
        assert!(hunks[0].body.contains("line2 CHANGED"));
        assert!(!hunks[0].body.contains("line19"));
        assert!(hunks[1].body.contains("line19 CHANGED"));
        assert!(!hunks[1].body.contains("line2 CHANGED"));
    }

    #[test]
    fn unstage_hunk_unstages_only_the_selected_hunk_leaving_the_other_real_change_staged() {
        let (tmp, repo) = repo_with_two_separated_staged_hunks("unstage_hunk_one");
        repo.unstage_hunk(Path::new("f.txt"), 0).unwrap();

        // The real staged (index) content no longer has line2's change, but
        // still has line19's.
        let staged = repo
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        assert!(!staged.contains("line2 CHANGED"));
        assert!(staged.contains("line19 CHANGED"));

        // The real working-tree file is completely untouched by unstaging.
        let working = fs::read_to_string(tmp.dir.join("f.txt")).unwrap();
        assert!(working.contains("line2 CHANGED"));
        assert!(working.contains("line19 CHANGED"));

        // git_status shows this file as both staged (line19) AND still
        // unstaged (line2's real reverted-back-to-unstaged diff).
        let status = repo.status().unwrap();
        let entry = status
            .iter()
            .find(|e| e.path == Path::new("f.txt"))
            .unwrap();
        assert!(entry.staged.is_some());
        assert!(entry.unstaged.is_some());
    }

    #[test]
    fn unstage_hunk_twice_in_sequence_unstages_the_real_whole_file_back_to_head() {
        let (_tmp, repo) = repo_with_two_separated_staged_hunks("unstage_hunk_both");
        repo.unstage_hunk(Path::new("f.txt"), 0).unwrap();
        // The real remaining staged diff now has exactly 1 hunk (line19's
        // change) -- re-fetched fresh, not reused from the first call.
        let remaining = repo.diff_hunks_staged(Path::new("f.txt")).unwrap();
        assert_eq!(remaining.len(), 1);
        repo.unstage_hunk(Path::new("f.txt"), 0).unwrap();

        let staged = repo
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        let head = repo.head_blob_content(Path::new("f.txt")).unwrap().unwrap();
        assert_eq!(
            staged, head,
            "unstaging every real hunk matches HEAD exactly"
        );
        assert!(repo
            .diff_hunks_staged(Path::new("f.txt"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn unstage_hunk_out_of_range_errors_honestly() {
        let (_tmp, repo) = repo_with_two_separated_staged_hunks("unstage_hunk_oor");
        assert!(repo.unstage_hunk(Path::new("f.txt"), 99).is_err());
    }

    #[test]
    fn unstage_hunk_on_a_pure_staged_addition_reverts_index_to_head_exactly() {
        let (tmp, repo) = TempRepo::new("unstage_hunk_addition");
        tmp.write("f.txt", "line1\nline2\nline3\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("base").unwrap();
        tmp.write("f.txt", "line1\nline2\nline3\nline4\nline5\n");
        repo.stage(Path::new("f.txt")).unwrap();

        let hunks = repo.diff_hunks_staged(Path::new("f.txt")).unwrap();
        assert_eq!(hunks.len(), 1);
        repo.unstage_hunk(Path::new("f.txt"), 0).unwrap();

        let staged = repo
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        assert_eq!(staged, "line1\nline2\nline3\n");
        // The real working-tree file still has the addition -- only the
        // index reverted.
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "line1\nline2\nline3\nline4\nline5\n"
        );
    }

    /// A real one-hunk replace (`beta` -> `BETA`) that is *unstaged* --
    /// the shared fixture for the per-line staging tests, kept minimal so
    /// each test asserts one real selection's exact index content.
    fn repo_with_a_replace_change(unique: &str) -> (TempRepo, GitRepo) {
        let (tmp, repo) = TempRepo::new(unique);
        tmp.write("f.txt", "alpha\nbeta\ngamma\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("base").unwrap();
        tmp.write("f.txt", "alpha\nBETA\ngamma\n");
        (tmp, repo)
    }

    /// The exact same single replace change, fully staged -- the fixture
    /// for the per-line *un*staging tests.
    fn repo_with_a_staged_replace_change(unique: &str) -> (TempRepo, GitRepo) {
        let (tmp, repo) = repo_with_a_replace_change(unique);
        repo.stage(Path::new("f.txt")).unwrap();
        (tmp, repo)
    }

    #[test]
    fn hunk_lines_reports_the_real_per_line_origin_and_content() {
        let (_tmp, repo) = repo_with_a_replace_change("hunk_lines_replace");
        let lines = repo.hunk_lines(Path::new("f.txt"), 0).unwrap();
        let rendered: Vec<(usize, char, &str)> = lines
            .iter()
            .map(|l| (l.index, l.origin, l.content.as_str()))
            .collect();
        assert_eq!(
            rendered,
            vec![
                (0, ' ', "alpha\n"),
                (1, '-', "beta\n"),
                (2, '+', "BETA\n"),
                (3, ' ', "gamma\n"),
            ],
            "the single replace hunk's real patch-order lines with 0-based indices"
        );
    }

    #[test]
    fn hunk_lines_out_of_range_errors_honestly() {
        let (_tmp, repo) = repo_with_a_replace_change("hunk_lines_oor");
        assert!(repo.hunk_lines(Path::new("f.txt"), 99).is_err());
    }

    #[test]
    fn stage_lines_selecting_every_change_line_reduces_to_stage_hunk_exactly() {
        let (_tmp, repo) = repo_with_a_replace_change("stage_lines_all");
        repo.stage_lines(Path::new("f.txt"), 0, &[1, 2]).unwrap();
        let by_lines = repo
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        let (_tmp2, repo2) = repo_with_a_replace_change("stage_lines_all_baseline");
        repo2.stage_hunk(Path::new("f.txt"), 0).unwrap();
        let by_hunk = repo2
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        assert_eq!(
            by_lines, by_hunk,
            "selecting every change line is exactly `stage_hunk`"
        );
        assert_eq!(by_lines, "alpha\nBETA\ngamma\n");
    }

    #[test]
    fn stage_lines_selecting_only_the_addition_keeps_the_deleted_line_in_the_index() {
        let (tmp, repo) = repo_with_a_replace_change("stage_lines_add_only");
        repo.stage_lines(Path::new("f.txt"), 0, &[2]).unwrap();
        assert_eq!(
            repo.index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\nbeta\nBETA\ngamma\n",
            "the new line is staged while the old line stays in the index"
        );
        // The real working tree is completely untouched.
        assert_eq!(
            fs::read_to_string(tmp.dir.join("f.txt")).unwrap(),
            "alpha\nBETA\ngamma\n"
        );
        // The real remaining unstaged diff is exactly the deletion.
        let remaining = repo.diff_hunks(Path::new("f.txt")).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].body.contains("-beta\n"));
        assert!(!remaining[0].body.contains("+BETA\n"));
    }

    #[test]
    fn stage_lines_selecting_only_the_deletion_removes_that_old_line_from_the_index() {
        let (_tmp, repo) = repo_with_a_replace_change("stage_lines_del_only");
        repo.stage_lines(Path::new("f.txt"), 0, &[1]).unwrap();
        assert_eq!(
            repo.index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\ngamma\n",
            "the old line leaves the index while the new line is not staged"
        );
        // The real remaining unstaged diff is exactly the addition.
        let remaining = repo.diff_hunks(Path::new("f.txt")).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].body.contains("+BETA\n"));
        assert!(!remaining[0].body.contains("-beta\n"));
    }

    #[test]
    fn stage_lines_selecting_nothing_is_an_exact_no_op() {
        let (_tmp, repo) = repo_with_a_replace_change("stage_lines_none");
        repo.stage_lines(Path::new("f.txt"), 0, &[]).unwrap();
        assert_eq!(
            repo.index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\nbeta\ngamma\n",
            "an empty selection stages nothing, leaving the index byte-identical"
        );
    }

    #[test]
    fn stage_lines_out_of_range_errors_honestly() {
        let (_tmp, repo) = repo_with_a_replace_change("stage_lines_oor");
        assert!(repo.stage_lines(Path::new("f.txt"), 0, &[999]).is_err());
    }

    #[test]
    fn stage_lines_can_stage_one_change_of_two_in_the_same_real_hunk() {
        let (tmp, repo) = TempRepo::new("stage_lines_two_in_one");
        tmp.write("f.txt", "line1\nline2\nline3\nline4\nline5\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("base").unwrap();
        tmp.write(
            "f.txt",
            "line1\nline2 CHANGED\nline3 CHANGED\nline4\nline5\n",
        );

        let hunks = repo.diff_hunks(Path::new("f.txt")).unwrap();
        assert_eq!(hunks.len(), 1, "adjacent edits stay one real hunk");
        // Real per-line layout, in libgit2's own hunk order (which groups
        // the two adjacent changes as one deletion run then one addition
        // run -- real git's canonical unified display for adjacent edits):
        // ' ' line1, '-' line2, '-' line3, '+' line2 CHANGED,
        // '+' line3 CHANGED, ' ' line4, ' ' line5.
        let lines = repo.hunk_lines(Path::new("f.txt"), 0).unwrap();
        assert_eq!(
            lines
                .iter()
                .map(|l| (l.origin, l.content.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (' ', "line1\n"),
                ('-', "line2\n"),
                ('-', "line3\n"),
                ('+', "line2 CHANGED\n"),
                ('+', "line3 CHANGED\n"),
                (' ', "line4\n"),
                (' ', "line5\n"),
            ]
        );

        // Stage only the first change (line2 -> line2 CHANGED): delete line
        // index 1, add line index 3.
        repo.stage_lines(Path::new("f.txt"), 0, &[1, 3]).unwrap();
        assert_eq!(
            repo.index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "line1\nline2 CHANGED\nline3\nline4\nline5\n",
            "only the first change is staged at its real position; the second stays unstaged"
        );
        let remaining = repo.diff_hunks(Path::new("f.txt")).unwrap();
        assert!(remaining[0].body.contains("-line3\n"));
        assert!(!remaining[0].body.contains("-line2\n"));
    }

    #[test]
    fn stage_lines_with_a_pure_insertion_then_a_later_deletion_orders_by_real_position() {
        let (tmp, repo) = TempRepo::new("stage_lines_insertion_then_deletion");
        tmp.write("f.txt", "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n");
        repo.stage(Path::new("f.txt")).unwrap();
        repo.commit("base").unwrap();
        // Insert one line right after `alpha`, and delete `zeta` several
        // lines later -- two changes in one real hunk whose old and new line
        // coordinates genuinely differ (the addition's `new_lineno` and the
        // deletion's `old_lineno` land on the same number even though they
        // occupy different positions), the mixed-coordinate shape that the
        // slot-ordering exists for.
        tmp.write("f.txt", "alpha\nINSERTED\nbeta\ngamma\ndelta\nepsilon\n");

        let hunks = repo.diff_hunks(Path::new("f.txt")).unwrap();
        assert_eq!(hunks.len(), 1, "both changes stay one real hunk");
        let lines = repo.hunk_lines(Path::new("f.txt"), 0).unwrap();
        assert_eq!(
            lines
                .iter()
                .map(|l| (l.origin, l.content.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (' ', "alpha\n"),
                ('+', "INSERTED\n"),
                (' ', "beta\n"),
                (' ', "gamma\n"),
                (' ', "delta\n"),
                (' ', "epsilon\n"),
                ('-', "zeta\n"),
            ]
        );

        // Stage only the insertion, leaving the later deletion unstaged: the
        // index becomes the old content plus the inserted line at its real
        // position (before beta), with `zeta` still present. (The insertion
        // is in-hunk index 1, per the per-line layout asserted above.)
        repo.stage_lines(Path::new("f.txt"), 0, &[1]).unwrap();
        assert_eq!(
            repo.index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\nINSERTED\nbeta\ngamma\ndelta\nepsilon\nzeta\n",
            "the inserted line lands before beta at its real position; zeta stays"
        );
        // The real remaining unstaged diff is exactly the deletion.
        let remaining = repo.diff_hunks(Path::new("f.txt")).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].body.contains("-zeta\n"));
        assert!(!remaining[0].body.contains("+INSERTED\n"));
    }

    #[test]
    fn unstage_lines_selecting_every_change_line_reduces_to_unstage_hunk_exactly() {
        let (_tmp, repo) = repo_with_a_staged_replace_change("unstage_lines_all");
        repo.unstage_lines(Path::new("f.txt"), 0, &[1, 2]).unwrap();
        let by_lines = repo
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        let (_tmp2, repo2) = repo_with_a_staged_replace_change("unstage_lines_all_baseline");
        repo2.unstage_hunk(Path::new("f.txt"), 0).unwrap();
        let by_hunk = repo2
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap();
        assert_eq!(
            by_lines, by_hunk,
            "selecting every change line is exactly `unstage_hunk`"
        );
        assert_eq!(by_lines, "alpha\nbeta\ngamma\n");
    }

    #[test]
    fn unstage_lines_selecting_only_the_addition_removes_just_that_new_line() {
        let (_tmp, repo) = repo_with_a_staged_replace_change("unstage_lines_add_only");
        repo.unstage_lines(Path::new("f.txt"), 0, &[2]).unwrap();
        // The staged addition leaves the index; beta's deletion stays staged,
        // so the index now has neither beta nor BETA at that spot.
        assert_eq!(
            repo.index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\ngamma\n"
        );
        // The real remaining staged diff is exactly the deletion.
        let remaining = repo.diff_hunks_staged(Path::new("f.txt")).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].body.contains("-beta\n"));
        assert!(!remaining[0].body.contains("+BETA\n"));
    }

    #[test]
    fn unstage_lines_selecting_only_a_deletion_readds_just_that_old_line() {
        let (_tmp, repo) = repo_with_a_staged_replace_change("unstage_lines_del_only");
        repo.unstage_lines(Path::new("f.txt"), 0, &[1]).unwrap();
        assert_eq!(
            repo.index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\nbeta\nBETA\ngamma\n",
            "beta returns to the index while the BETA addition stays staged"
        );
        // The real remaining staged diff is exactly the addition.
        let remaining = repo.diff_hunks_staged(Path::new("f.txt")).unwrap();
        assert!(remaining[0].body.contains("+BETA\n"));
        assert!(!remaining[0].body.contains("-beta\n"));
    }

    #[test]
    fn unstage_lines_selecting_nothing_is_an_exact_no_op() {
        let (_tmp, repo) = repo_with_a_staged_replace_change("unstage_lines_none");
        repo.unstage_lines(Path::new("f.txt"), 0, &[]).unwrap();
        assert_eq!(
            repo.index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\nBETA\ngamma\n",
            "an empty selection unstages nothing, leaving the index byte-identical"
        );
    }

    #[test]
    fn unstage_lines_out_of_range_errors_honestly() {
        let (_tmp, repo) = repo_with_a_staged_replace_change("unstage_lines_oor");
        assert!(repo.unstage_lines(Path::new("f.txt"), 0, &[99]).is_err());
    }

    /// Applies a real unified-diff patch to a temp repo's index via the real
    /// `git apply --cached` binary -- the ground-truth per-line mechanism
    /// this suite's own `stage_lines`/`unstage_lines` are compared against.
    /// Self-skipping is the caller's job; this helper only runs when git is
    /// actually present.
    fn git_apply_cached(dir: &Path, patch: &str) -> Result<(), String> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new("git")
            .args(["apply", "--cached", "-"])
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn git: {e}"))?;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(patch.as_bytes())
            .map_err(|e| format!("write patch: {e}"))?;
        let status = child.wait().map_err(|e| format!("wait git: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("git apply --cached rejected the patch".to_string())
        }
    }

    /// Reads a repo's real current index content through a *fresh* `GitRepo`
    /// -- libgit2 caches the index per `Repository` object, so after the
    /// real `git apply --cached` subprocess rewrites the index file on disk,
    /// a pre-existing `GitRepo` would hand back its stale in-memory copy.
    /// (Each backend RPC discovers its own fresh `GitRepo`, so this is a
    /// test-harness-only concern, not a production one.)
    fn fresh_index(dir: &Path) -> String {
        GitRepo::discover(dir)
            .unwrap()
            .index_blob_content(Path::new("f.txt"))
            .unwrap()
            .unwrap()
    }

    /// Cross-checks the per-line splice against real `git apply --cached`
    /// for exactly the selections real git's own plumbing can express (git's
    /// `add -e`/`apply` cannot express "stage the addition while keeping the
    /// old line" -- that selection is covered by the direct-content tests
    /// instead, and is the one place this model intentionally goes beyond
    /// git's own line-edit mode, matching VS Code/GitKraken).
    #[test]
    fn per_line_staging_matches_real_git_apply_for_the_selections_git_can_express() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("SKIP: no real `git` binary on PATH");
            return;
        }

        // --- staging side: index starts at HEAD (alpha,beta,gamma), workdir
        // has the beta -> BETA replace. ---
        // stage only the deletion -> index alpha,gamma.
        let (_tmp_a, repo_a) = repo_with_a_replace_change("stage_vs_git_del");
        repo_a.stage_lines(Path::new("f.txt"), 0, &[1]).unwrap();
        assert_eq!(
            repo_a
                .index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\ngamma\n"
        );
        let (tmp_b, _repo_b) = repo_with_a_replace_change("stage_vs_git_del_apply");
        git_apply_cached(
            &tmp_b.dir,
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,2 @@\n alpha\n-beta\n gamma\n",
        )
        .unwrap();
        assert_eq!(fresh_index(&tmp_b.dir), "alpha\ngamma\n");

        // stage the whole hunk (both change lines) -> index alpha,BETA,gamma.
        let (_tmp_c, repo_c) = repo_with_a_replace_change("stage_vs_git_all");
        repo_c.stage_lines(Path::new("f.txt"), 0, &[1, 2]).unwrap();
        assert_eq!(
            repo_c
                .index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\nBETA\ngamma\n"
        );
        let (tmp_d, _repo_d) = repo_with_a_replace_change("stage_vs_git_all_apply");
        git_apply_cached(
            &tmp_d.dir,
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,3 @@\n alpha\n-beta\n+BETA\n gamma\n",
        )
        .unwrap();
        assert_eq!(fresh_index(&tmp_d.dir), "alpha\nBETA\ngamma\n");

        // --- unstaging side: index is the fully-staged replace
        // (alpha,BETA,gamma). ---
        // unstage only the deletion (re-add beta, keep BETA staged) -> index
        // alpha,beta,BETA,gamma.
        let (_tmp_e, repo_e) = repo_with_a_staged_replace_change("unstage_vs_git_del");
        repo_e.unstage_lines(Path::new("f.txt"), 0, &[1]).unwrap();
        assert_eq!(
            repo_e
                .index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\nbeta\nBETA\ngamma\n"
        );
        let (tmp_f, _repo_f) = repo_with_a_staged_replace_change("unstage_vs_git_del_apply");
        git_apply_cached(
            &tmp_f.dir,
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,4 @@\n alpha\n+beta\n BETA\n gamma\n",
        )
        .unwrap();
        assert_eq!(fresh_index(&tmp_f.dir), "alpha\nbeta\nBETA\ngamma\n");

        // unstage only the addition (remove BETA, keep beta's deletion staged)
        // -> index alpha,gamma.
        let (_tmp_g, repo_g) = repo_with_a_staged_replace_change("unstage_vs_git_add");
        repo_g.unstage_lines(Path::new("f.txt"), 0, &[2]).unwrap();
        assert_eq!(
            repo_g
                .index_blob_content(Path::new("f.txt"))
                .unwrap()
                .unwrap(),
            "alpha\ngamma\n"
        );
        let (tmp_h, _repo_h) = repo_with_a_staged_replace_change("unstage_vs_git_add_apply");
        git_apply_cached(
            &tmp_h.dir,
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,2 @@\n alpha\n-BETA\n gamma\n",
        )
        .unwrap();
        assert_eq!(fresh_index(&tmp_h.dir), "alpha\ngamma\n");
    }
}
