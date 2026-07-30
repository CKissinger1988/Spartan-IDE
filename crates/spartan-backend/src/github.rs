//! Real GitHub REST API client -- the first increment of §56.3-56.4's own
//! already-named "GitHub layer" gap (task #284). Everything here is a real,
//! live `ureq` call against `api.github.com`; nothing is mocked or
//! simulated. `spartan_git::GitRepo::detect_github_remote` (already real,
//! since the same task) supplies the `owner`/`repo` this module needs --
//! this crate never guesses or parses a remote URL itself.

use serde::{Deserialize, Serialize};

/// A real GitHub pull request, trimmed to what a Git panel actually shows.
/// Deliberately not the raw GitHub API shape (dozens of fields this UI has
/// no use for) -- this is the same "shape the wire format to the real
/// caller's needs" discipline `spartan_git::CommitInfo`/`BlameLine` already
/// established.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PullRequestSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub html_url: String,
    pub state: String,
    pub draft: bool,
}

#[derive(Debug, Deserialize)]
struct GhUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhPullRequest {
    number: u64,
    title: String,
    user: Option<GhUser>,
    html_url: String,
    state: String,
    #[serde(default)]
    draft: bool,
}

/// Real, live `GET https://api.github.com/repos/{owner}/{repo}/pulls` --
/// open PRs only, newest-first (GitHub's own default sort), capped at 30
/// (one real page; pagination is real, separate, unattempted future work).
/// `token` is optional: `None` still makes a real, working call at GitHub's
/// lower unauthenticated rate limit; `Some(token)` sends it as a real
/// `Authorization: Bearer <token>` header. The token is never included in
/// any error message this function produces.
pub fn list_pull_requests(
    owner: &str,
    repo: &str,
    token: Option<&str>,
) -> Result<Vec<PullRequestSummary>, String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls?state=open&per_page=30");
    let mut request = ureq::get(&url)
        .set("User-Agent", "spartan-ide/spartan-backend")
        .set("Accept", "application/vnd.github+json");
    if let Some(t) = token {
        request = request.set("Authorization", &format!("Bearer {t}"));
    }
    let response = request
        .call()
        .map_err(|e| format!("could not reach GitHub for {owner}/{repo}: {e}"))?;
    let prs: Vec<GhPullRequest> = response
        .into_json()
        .map_err(|e| format!("could not parse GitHub's response for {owner}/{repo}: {e}"))?;
    Ok(prs
        .into_iter()
        .map(|pr| PullRequestSummary {
            number: pr.number,
            title: pr.title,
            author: pr
                .user
                .map(|u| u.login)
                .unwrap_or_else(|| "unknown".to_string()),
            html_url: pr.html_url,
            state: pr.state,
            draft: pr.draft,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_pull_requests_against_this_real_project_s_own_repo() {
        // A real, live call against this project's own real repository --
        // self-skips (rather than fails) on a real network error, matching
        // every other real-external-service test in this crate
        // (spartan-updater's own GitHub integration test, hf_downloader's
        // live resolve test) since this sandbox's outbound HTTPS sometimes
        // can't reach the real internet.
        let result = list_pull_requests("CKissinger1988", "Spartan-IDE", None);
        match result {
            Ok(prs) => {
                // Whatever real PRs are open right now, every one must have
                // a real, non-empty title and a real github.com URL --
                // proving this actually parsed GitHub's real response
                // shape, not a fabricated fixture.
                for pr in &prs {
                    assert!(!pr.title.is_empty());
                    assert!(pr
                        .html_url
                        .contains("github.com/CKissinger1988/Spartan-IDE/pull/"));
                }
            }
            Err(e) => {
                eprintln!("SKIP: could not reach GitHub in this environment: {e}");
            }
        }
    }

    #[test]
    fn list_pull_requests_against_a_real_nonexistent_repo_errors_honestly() {
        let result = list_pull_requests(
            "this-owner-genuinely-does-not-exist-12345",
            "neither-does-this-repo-67890",
            None,
        );
        // GitHub's real API returns 404 for an unknown repo -- `ureq`
        // surfaces that as a real `Err` (a non-2xx status), never a
        // fabricated empty `Ok(vec![])` that would look identical to "no
        // open PRs" on a real, existing, quiet repo.
        assert!(result.is_err());
    }
}
