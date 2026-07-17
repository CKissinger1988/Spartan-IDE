//! Per-plan resource quotas and the allocation-admission check. These are
//! the concrete, enforceable numbers behind the tenant-separation promise:
//! CPU/memory/pids caps per container, a hard wall-clock lifetime (the
//! answer to §36.4.7's "uncapped consumption"), and a concurrency cap so one
//! tenant can't allocate unbounded containers.
//!
//! Pure policy only -- this module decides *whether* an allocation is
//! allowed and *what limits* it gets; the runtime crate (later) is what
//! actually applies those limits to a real container via bollard's
//! `HostConfig` (memory/nano_cpus/pids_limit) and an independent reaper task.

use spartan_cloud_protocol::PlanTier;

/// The real, enforceable limits for one allocation under a given plan. Every
/// field maps to a real control the runtime layer applies:
/// - `cpu_millis` -> Docker `nano_cpus` (1000 millis = 1 core),
/// - `memory_mb` -> `HostConfig.memory`,
/// - `pids_limit` -> `HostConfig.pids_limit` (fork-bomb defense),
/// - `max_lifetime_secs` -> the reaper's hard kill time,
/// - `max_concurrent` -> admission control here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanLimits {
    pub max_concurrent: u32,
    pub cpu_millis: u32,
    pub memory_mb: u32,
    pub pids_limit: u32,
    pub max_lifetime_secs: u64,
}

impl PlanLimits {
    /// The real, deliberately-modest limits per tier. Free is generous
    /// enough to evaluate the product but bounded hard; Pro is larger but
    /// still finite (no plan is ever "unlimited" -- that's the whole point
    /// of running untrusted tenant code under caps).
    pub fn for_tier(tier: PlanTier) -> Self {
        match tier {
            PlanTier::Free => PlanLimits {
                max_concurrent: 1,
                cpu_millis: 1000, // 1 core
                memory_mb: 1024,
                pids_limit: 256,
                max_lifetime_secs: 30 * 60, // 30 minutes
            },
            PlanTier::Pro => PlanLimits {
                max_concurrent: 5,
                cpu_millis: 4000, // 4 cores
                memory_mb: 8192,
                pids_limit: 2048,
                max_lifetime_secs: 4 * 60 * 60, // 4 hours
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaError {
    /// The tenant already has `limit` containers running -- the max for their
    /// plan. Carries the limit so the API can return an actionable message.
    ConcurrencyLimitReached { limit: u32 },
}

impl std::fmt::Display for QuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuotaError::ConcurrencyLimitReached { limit } => write!(
                f,
                "concurrency limit reached ({limit} running); stop one or upgrade your plan"
            ),
        }
    }
}

impl std::error::Error for QuotaError {}

/// Admission control: may a tenant currently running `current_active`
/// containers allocate one more under `limits`? Pure and total -- the only
/// input is the count and the limit, so it's trivially testable and can't
/// disagree with itself across call sites.
pub fn can_allocate(limits: &PlanLimits, current_active: u32) -> Result<(), QuotaError> {
    if current_active >= limits.max_concurrent {
        Err(QuotaError::ConcurrencyLimitReached {
            limit: limits.max_concurrent,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_is_more_constrained_than_pro() {
        let free = PlanLimits::for_tier(PlanTier::Free);
        let pro = PlanLimits::for_tier(PlanTier::Pro);
        assert!(pro.max_concurrent > free.max_concurrent);
        assert!(pro.cpu_millis > free.cpu_millis);
        assert!(pro.memory_mb > free.memory_mb);
        assert!(pro.max_lifetime_secs > free.max_lifetime_secs);
        // No plan is ever unlimited.
        assert!(free.max_concurrent > 0 && pro.max_concurrent < u32::MAX);
    }

    #[test]
    fn admission_allows_under_the_limit_and_denies_at_it() {
        let free = PlanLimits::for_tier(PlanTier::Free); // max_concurrent = 1
        assert!(can_allocate(&free, 0).is_ok(), "0 < 1 is allowed");
        assert_eq!(
            can_allocate(&free, 1),
            Err(QuotaError::ConcurrencyLimitReached { limit: 1 }),
            "at the limit is denied"
        );
        assert!(
            can_allocate(&free, 2).is_err(),
            "over the limit is denied too"
        );
    }

    #[test]
    fn pro_admission_tracks_its_higher_limit() {
        let pro = PlanLimits::for_tier(PlanTier::Pro); // max_concurrent = 5
        assert!(can_allocate(&pro, 4).is_ok());
        assert!(can_allocate(&pro, 5).is_err());
    }
}
