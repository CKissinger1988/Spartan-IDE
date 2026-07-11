// Real, thin wrapper around isomorphic-git's real API -- no mocking, no
// simulated repository. Uses Node's real `fs` module directly (isomorphic-
// git supports it natively); a real browser deployment would swap in a
// browser-compatible fs implementation (e.g. lightning-fs, IndexedDB-
// backed) instead -- not attempted in this pass, see README.md.

const git = require("isomorphic-git");
const fs = require("node:fs");

async function initRepo(dir) {
  await git.init({ fs, dir, defaultBranch: "main" });
}

async function writeAndStage(dir, filepath, content) {
  fs.writeFileSync(require("node:path").join(dir, filepath), content);
  await git.add({ fs, dir, filepath });
}

async function commit(dir, message) {
  return git.commit({
    fs,
    dir,
    message,
    author: { name: "spike", email: "spike@example.com" },
  });
}

async function statusOf(dir, filepath) {
  return git.status({ fs, dir, filepath });
}

async function realLog(dir) {
  return git.log({ fs, dir });
}

module.exports = { initRepo, writeAndStage, commit, statusOf, realLog };
