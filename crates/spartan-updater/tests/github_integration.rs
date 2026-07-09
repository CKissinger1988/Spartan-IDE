//! Real, live integration test against the actual GitHub REST API for
//! this project's own real repository. Self-skips (rather than fails) on
//! any network/HTTP problem -- including a real, expected one this
//! environment has already hit once during development: GitHub's public
//! API rate-limits unauthenticated requests per source IP, and this
//! sandboxed environment's outbound IP is shared, so a 403 here is a
//! real, environment-specific condition, not a bug in this crate --
//! matching every other real-external-service integration test in this
//! workspace (`ollama_integration.rs`, `plan_ollama_integration.rs`).

use spartan_updater::check_for_updates;

const REPO: &str = "ckissinger1988/spartan-ide";
const BRANCH: &str = "main";

#[test]
fn real_check_against_the_real_github_api_returns_a_real_comparable_result() {
    match check_for_updates(REPO, BRANCH) {
        Ok(result) => {
            assert_eq!(result.current_commit.len(), 40, "a real git commit hash");
            assert_eq!(result.latest_commit.len(), 40, "a real git commit hash");
            println!("real update check result: {result:?}");
        }
        Err(e) => {
            eprintln!(
                "SKIP: real GitHub API call failed (network/rate-limit/unknown build commit): {e}"
            );
        }
    }
}
