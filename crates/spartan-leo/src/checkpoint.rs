//! Real §4.2 checkpointing (task #5): "Every `Executing` phase begins with
//! a git plumbing snapshot: `git stash create` equivalent (non-destructive,
//! doesn't touch working tree)... Restore to before this step is a real,
//! tested rollback." Built on `spartan-git`'s real `GitRepo` (§75.30) --
//! extended here with real `git2` stash plumbing `spartan-git` itself
//! doesn't expose (its own scope was the Source Control panel: status/
//! stage/commit, not snapshotting).
//!
//! **A real, deliberate v1 scope cut, named up front**: only git-backed
//! projects are supported. §4.2's own "for non-git projects, Spartan
//! maintains its own shadow version store (`.spartan/snapshots/`)" is
//! real, separate, unbuilt follow-up work -- every real verification this
//! module has actually run against has been a real git repository, so
//! claiming the non-git path works would be exactly the kind of
//! unverified claim this project's own discipline forbids.
//!
//! **A real, named technique choice**: git2's `stash_save`/`stash_apply`
//! pair (not a lower-level "create a commit object without touching the
//! index/working-tree" plumbing sequence, which git2-rs doesn't expose a
//! direct binding for) is used as save-then-immediately-reapply --
//! `create_checkpoint` calls `stash_save2` (which *does* clear the working
//! tree, a real, if momentary, mutation) followed immediately by
//! `stash_apply(0, ...)` to restore it. The net, observable effect matches
//! §4.2's "non-destructive" requirement (nothing is lost, the working tree
//! looks unchanged once `create_checkpoint` returns), even though the
//! literal mechanism transiently touches the working tree -- named
//! honestly here rather than glossed over as true zero-touch plumbing.

use git2::{Repository, ResetType, Signature, StashFlags, StatusOptions};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub label: String,
    /// `None` when the working tree was already clean at checkpoint time
    /// -- a real, common case (before Leo's very first edit in a task,
    /// nothing has changed yet), and a real bug found only by running
    /// these tests: `git2::Repository::stash_save2` itself errors with
    /// "cannot stash changes - there is nothing to stash" on a clean
    /// tree, so a checkpoint must not unconditionally assume one is
    /// created. Restoring only ever needs the base commit in that case.
    pub stash_oid: Option<git2::Oid>,
    pub base_commit_oid: git2::Oid,
    /// A real, live-found second gap: `git reset --hard` only ever
    /// touches *tracked* files -- a real untracked file created *after* a
    /// clean-tree checkpoint (`stash_oid: None`, nothing to reapply) is
    /// invisible to both the reset and any stash-apply, so it would
    /// silently survive a restore. This is a real snapshot of every
    /// untracked (non-ignored) path present at checkpoint time, used by
    /// `restore_checkpoint` to know which untracked files are safe to
    /// leave alone (they existed before the checkpoint) versus which are
    /// new since it (and must be removed to make "restore" actually mean
    /// restore).
    pub untracked_paths_at_checkpoint: HashSet<String>,
    pub created_at_unix: u64,
}

fn untracked_paths(repo: &Repository) -> Result<HashSet<String>, git2::Error> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    Ok(repo
        .statuses(Some(&mut opts))?
        .iter()
        .filter(|e| e.status().contains(git2::Status::WT_NEW))
        .filter_map(|e| e.path().map(str::to_string))
        .collect())
}

#[derive(Debug)]
pub enum CheckpointError {
    Git(git2::Error),
    /// Real, honest failure mode: the checkpoint's stash entry is no
    /// longer in the repo's stash list (dropped, applied and cleared by
    /// something else, or garbage-collected) -- restoring is refused
    /// rather than silently doing nothing or restoring the wrong state.
    StashNoLongerAvailable,
}

impl From<git2::Error> for CheckpointError {
    fn from(e: git2::Error) -> Self {
        CheckpointError::Git(e)
    }
}

/// Real git plumbing snapshot before an `Executing` phase begins. Requires
/// at least one real commit to already exist (a brand-new, commit-less
/// repo has no real `HEAD` to record as `base_commit_oid` -- a real,
/// narrow, named v1 limitation, not silently worked around).
pub fn create_checkpoint(
    repo: &mut Repository,
    label: &str,
) -> Result<Checkpoint, CheckpointError> {
    let base_commit_oid = repo
        .head()?
        .target()
        .ok_or_else(|| CheckpointError::Git(git2::Error::from_str("HEAD has no target commit")))?;

    let untracked_paths_at_checkpoint = untracked_paths(repo)?;

    // Real, live-found bug: `stash_save2` itself errors on a clean working
    // tree ("there is nothing to stash") -- checked for real here, not
    // assumed, since a checkpoint must succeed even before Leo's first
    // edit of a task, the most common real case of all.
    let is_dirty = repo
        .statuses(None)?
        .iter()
        .any(|e| !matches!(e.status(), git2::Status::IGNORED));

    let stash_oid = if is_dirty {
        let signature = repo
            .signature()
            .unwrap_or_else(|_| Signature::now("Spartan Leo", "leo@spartan.local").unwrap());
        let oid = repo.stash_save2(&signature, Some(label), Some(StashFlags::INCLUDE_UNTRACKED))?;
        // Immediately restore -- see this module's own doc comment for
        // why this is the real, chosen technique rather than lower-level
        // plumbing.
        repo.stash_apply(0, None)?;
        Some(oid)
    } else {
        None
    };

    let created_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(Checkpoint {
        label: label.to_string(),
        stash_oid,
        base_commit_oid,
        untracked_paths_at_checkpoint,
        created_at_unix,
    })
}

/// Real restore: hard-resets the working tree to the checkpoint's real
/// recorded base commit, then -- only if this checkpoint actually created
/// one (see `Checkpoint::stash_oid`'s own doc comment) -- re-applies the
/// real stash entry matching it, refusing (rather than silently no-oping
/// or guessing) if that stash entry can no longer be found by real, live
/// enumeration of the repo's current stash list.
pub fn restore_checkpoint(
    repo: &mut Repository,
    checkpoint: &Checkpoint,
) -> Result<(), CheckpointError> {
    let stash_index = if let Some(stash_oid) = checkpoint.stash_oid {
        let mut found_index = None;
        repo.stash_foreach(|index, _message, oid| {
            if *oid == stash_oid {
                found_index = Some(index);
                false // stop iterating
            } else {
                true
            }
        })?;
        match found_index {
            Some(index) => Some(index),
            None => return Err(CheckpointError::StashNoLongerAvailable),
        }
    } else {
        None
    };

    {
        let base_commit = repo.find_commit(checkpoint.base_commit_oid)?;
        repo.reset(base_commit.as_object(), ResetType::Hard, None)?;
    }
    if let Some(index) = stash_index {
        repo.stash_apply(index, None)?;
    }

    // Real cleanup pass (see `Checkpoint::untracked_paths_at_checkpoint`'s
    // own doc comment): a real untracked file created after the
    // checkpoint is invisible to both the hard reset and any stash-apply
    // above, so it's removed here explicitly if it wasn't part of the
    // checkpoint's own real snapshot.
    let workdir = repo
        .workdir()
        .ok_or_else(|| {
            CheckpointError::Git(git2::Error::from_str("repo has no working directory"))
        })?
        .to_path_buf();
    for path in untracked_paths(repo)? {
        if !checkpoint.untracked_paths_at_checkpoint.contains(&path) {
            let _ = std::fs::remove_file(workdir.join(&path));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn real_repo_with_one_commit(name: &str) -> (PathBuf, Repository) {
        let dir = std::env::temp_dir().join(format!("spartan-leo-checkpoint-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = Repository::init(&dir).unwrap();
        std::fs::write(dir.join("file.txt"), "original content\n").unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new("file.txt")).unwrap();
            index.write().unwrap();
            let tree_oid = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }
        (dir, repo)
    }

    #[test]
    fn real_checkpoint_preserves_working_tree_content_after_creation() {
        let (dir, mut repo) = real_repo_with_one_commit("preserve");
        std::fs::write(dir.join("file.txt"), "modified content\n").unwrap();

        create_checkpoint(&mut repo, "before edit").unwrap();

        // The whole point of §4.2's "non-destructive" requirement: the
        // real modification is still visible on disk right after the
        // checkpoint is created.
        assert_eq!(
            std::fs::read_to_string(dir.join("file.txt")).unwrap(),
            "modified content\n"
        );
    }

    #[test]
    fn real_restore_reverts_a_real_modification() {
        let (dir, mut repo) = real_repo_with_one_commit("restore");
        let checkpoint = create_checkpoint(&mut repo, "before edit").unwrap();

        std::fs::write(dir.join("file.txt"), "a bad edit\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("file.txt")).unwrap(),
            "a bad edit\n"
        );

        restore_checkpoint(&mut repo, &checkpoint).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("file.txt")).unwrap(),
            "original content\n"
        );
    }

    #[test]
    fn real_restore_reverts_a_real_new_untracked_file() {
        let (dir, mut repo) = real_repo_with_one_commit("untracked");
        let checkpoint = create_checkpoint(&mut repo, "before creating new file").unwrap();

        std::fs::write(dir.join("new_file.txt"), "should be gone").unwrap();
        assert!(dir.join("new_file.txt").exists());

        restore_checkpoint(&mut repo, &checkpoint).unwrap();

        assert!(
            !dir.join("new_file.txt").exists(),
            "a real untracked file created after the checkpoint should be gone after restore"
        );
    }

    #[test]
    fn restoring_a_dropped_stash_returns_a_real_error_not_a_silent_no_op() {
        let (dir, mut repo) = real_repo_with_one_commit("dropped");
        // A real dirty tree at checkpoint time, so a real stash actually
        // gets created (a clean-tree checkpoint has no stash at all to
        // drop -- see `Checkpoint::stash_oid`'s own doc comment).
        std::fs::write(dir.join("file.txt"), "modified before drop\n").unwrap();
        let checkpoint = create_checkpoint(&mut repo, "will be dropped").unwrap();
        assert!(checkpoint.stash_oid.is_some());
        repo.stash_drop(0).unwrap();

        let result = restore_checkpoint(&mut repo, &checkpoint);
        assert!(matches!(
            result,
            Err(CheckpointError::StashNoLongerAvailable)
        ));
    }
}
