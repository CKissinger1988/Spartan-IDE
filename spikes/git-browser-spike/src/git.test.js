const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { initRepo, writeAndStage, commit, statusOf, realLog } = require("./git");

/** Real, isolated temp directory per test -- created and removed for real,
 * matching spartan-git's own established Rust test convention of a real
 * temp git repository per test rather than a shared fixture. */
function realTempDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "git-browser-spike-"));
}

test("init + a real first commit produces a real, non-empty SHA", async () => {
  const dir = realTempDir();
  try {
    await initRepo(dir);
    await writeAndStage(dir, "hello.txt", "hello world\n");
    const sha = await commit(dir, "real first commit");
    assert.match(sha, /^[0-9a-f]{40}$/, `expected a real 40-char hex SHA, got ${sha}`);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("status correctly distinguishes unstaged from staged modifications", async () => {
  const dir = realTempDir();
  try {
    await initRepo(dir);
    await writeAndStage(dir, "hello.txt", "hello world\n");
    await commit(dir, "real first commit");

    fs.writeFileSync(path.join(dir, "hello.txt"), "hello world\nmodified\n");
    assert.equal(await statusOf(dir, "hello.txt"), "*modified");

    await writeAndStage(dir, "hello.txt", "hello world\nmodified\n");
    assert.equal(await statusOf(dir, "hello.txt"), "modified");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("log returns real commits, newest first, with the real messages", async () => {
  const dir = realTempDir();
  try {
    await initRepo(dir);
    await writeAndStage(dir, "hello.txt", "hello world\n");
    await commit(dir, "real first commit");
    await writeAndStage(dir, "hello.txt", "hello world\nmodified\n");
    await commit(dir, "real second commit");

    const log = await realLog(dir);
    assert.equal(log.length, 2);
    assert.equal(log[0].commit.message.trim(), "real second commit");
    assert.equal(log[1].commit.message.trim(), "real first commit");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a repository written by isomorphic-git is a genuinely valid git repo, confirmed by the real native git CLI", async () => {
  let gitAvailable = true;
  try {
    execFileSync("git", ["--version"], { stdio: "ignore" });
  } catch {
    gitAvailable = false;
  }
  if (!gitAvailable) {
    console.log("SKIP: real `git` binary not on PATH, skipping cross-tool verification");
    return;
  }

  const dir = realTempDir();
  try {
    await initRepo(dir);
    await writeAndStage(dir, "hello.txt", "hello world\n");
    const sha = await commit(dir, "real cross-checked commit");

    const nativeLog = execFileSync("git", ["log", "--format=%H %s"], {
      cwd: dir,
      encoding: "utf8",
    }).trim();
    assert.equal(nativeLog, `${sha} real cross-checked commit`);

    const nativeShow = execFileSync("git", ["show", "HEAD:hello.txt"], {
      cwd: dir,
      encoding: "utf8",
    });
    assert.equal(nativeShow, "hello world\n");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
