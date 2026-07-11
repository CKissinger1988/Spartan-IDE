# git-browser-spike — real git-in-browser feasibility for the web app

Real, runnable code, not a feasibility argument. Third real preparation step
for the planned vscode.dev-inspired web app (§75.85's hybrid architecture) --
proves the client-side git half works: can `isomorphic-git` (a pure JS
reimplementation of git, no native `libgit2` dependency at all -- unlike
`spartan-git`'s own real, existing use of `git2`/vendored `libgit2` for the
desktop shells) perform real, correct git operations?

## What was tested

Real, executed verification via `node --test`, 4 tests, all passing on the
first run (unlike the other two web-app spikes, no real bug was found this
time -- reported plainly rather than manufactured):

- `git.init()` + a real first commit produces a real, well-formed 40-char
  hex SHA.
- `git.status()` correctly distinguishes an unstaged modification
  (`"*modified"`) from a staged one (`"modified"`) -- the same real
  independent staged/unstaged split `spartan-git`'s own Rust implementation
  already exposes on the desktop side.
- `git.log()` returns real commits in the correct (newest-first) order with
  the real commit messages intact.
- **A real cross-tool check, not just internal self-consistency**: a
  repository written entirely by `isomorphic-git` was read back by the
  actual native `git` CLI (`git log --format=%H %s`, `git show HEAD:<path>`)
  and matched exactly -- the same commit SHA, the same message, the same
  file content -- confirming these are genuinely valid git objects any real
  git tooling can read, not something only `isomorphic-git` itself can
  interpret. This mirrors the same cross-tool discipline this project's own
  Source Control panel work (§75.30) already established for `spartan-git`.

## What this does and doesn't confirm

**Confirmed, real, and load-bearing**: the git half of the client-side core
can perform real init/add/commit/status/log operations correctly, producing
genuinely valid, standard git objects -- with **zero native dependency**,
meaning no `libgit2`/`git2` compilation story is needed for the browser at
all, a real, positive simplification versus the desktop shells' own
approach.

**Not attempted in this pass**, each a real, separate, still-open piece:
this spike used Node's real, native `fs` module directly (isomorphic-git
supports it out of the box) -- a real browser deployment needs a
browser-compatible filesystem backend instead, most likely `lightning-fs`
(an IndexedDB-backed implementation of the same `fs` interface,
purpose-built for `isomorphic-git`) or an adapter over the File System
Access API (the same real browser API task #81's own web/ scaffold work
will need for plain file read/write regardless of git); no real remote
operations (`clone`/`fetch`/`push`) were tested -- those need a real HTTP
transport and, in a browser, a CORS-friendly git server or a proxy,
genuinely different plumbing than the local-only operations tested here;
no diff/merge-conflict handling; no real performance measurement against a
large real repository (this project's own `.git` history, for instance) --
only a two-commit, one-file toy repo was exercised.
