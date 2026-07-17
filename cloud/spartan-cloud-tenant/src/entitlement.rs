//! The entitlement seam -- the single trait behind which real billing is
//! swapped in later, exactly mirroring the `ModelProvider` "one trait, swap
//! the real implementation" pattern this codebase already established.
//!
//! Billing is **deferred by explicit decision**, not forgotten: the MVP
//! ships `StubEntitlementProvider` (a manual/admin toggle), and a real
//! `StripeEntitlementProvider` -- same trait, webhook-driven state -- is an
//! additive swap with no change to any caller.

use std::collections::HashSet;
use std::sync::RwLock;

use spartan_cloud_protocol::{PlanTier, UserId};

/// What a user is currently entitled to. `active` distinguishes "has a Pro
/// plan that is currently paid/valid" from "had one that lapsed" -- a lapsed
/// Pro user falls back to `Free` quotas without losing their account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entitlement {
    pub tier: PlanTier,
    pub active: bool,
}

impl Entitlement {
    /// The tier actually used for quota decisions: a non-active entitlement
    /// (e.g. a lapsed subscription) is treated as `Free`, never denied
    /// outright -- the account still works, just at Free limits.
    pub fn effective_tier(&self) -> PlanTier {
        if self.active {
            self.tier
        } else {
            PlanTier::Free
        }
    }
}

/// The seam. A real implementation answers "what is this user entitled to
/// right now?" from whatever authority it trusts (a manual toggle today, a
/// billing provider's webhook-driven state later).
pub trait EntitlementProvider: Send + Sync {
    fn check(&self, user_id: &UserId) -> Entitlement;
}

/// The MVP entitlement source: everyone is `Free` unless explicitly granted
/// `Pro` via an admin action (`grant_pro`). Thread-safe so the API can share
/// one instance across request handlers. This is the honest stand-in until a
/// real billing integration exists -- it does not fabricate subscriptions.
#[derive(Debug, Default)]
pub struct StubEntitlementProvider {
    pro_users: RwLock<HashSet<UserId>>,
}

impl StubEntitlementProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Admin action: grant a user Pro entitlement (the manual toggle the
    /// real billing webhook eventually replaces).
    pub fn grant_pro(&self, user_id: UserId) {
        self.pro_users
            .write()
            .expect("entitlement lock poisoned")
            .insert(user_id);
    }

    /// Admin action: revoke Pro (drop back to Free).
    pub fn revoke_pro(&self, user_id: &UserId) {
        self.pro_users
            .write()
            .expect("entitlement lock poisoned")
            .remove(user_id);
    }
}

impl EntitlementProvider for StubEntitlementProvider {
    fn check(&self, user_id: &UserId) -> Entitlement {
        let is_pro = self
            .pro_users
            .read()
            .expect("entitlement lock poisoned")
            .contains(user_id);
        if is_pro {
            Entitlement {
                tier: PlanTier::Pro,
                active: true,
            }
        } else {
            Entitlement {
                tier: PlanTier::Free,
                active: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_defaults_everyone_to_free() {
        let provider = StubEntitlementProvider::new();
        let e = provider.check(&UserId("alice".into()));
        assert_eq!(e.tier, PlanTier::Free);
        assert!(e.active);
    }

    #[test]
    fn granting_and_revoking_pro_moves_a_user_between_tiers() {
        let provider = StubEntitlementProvider::new();
        let alice = UserId("alice".into());

        provider.grant_pro(alice.clone());
        assert_eq!(provider.check(&alice).tier, PlanTier::Pro);

        provider.revoke_pro(&alice);
        assert_eq!(provider.check(&alice).tier, PlanTier::Free);
    }

    #[test]
    fn a_lapsed_entitlement_falls_back_to_free_without_denial() {
        let lapsed = Entitlement {
            tier: PlanTier::Pro,
            active: false,
        };
        assert_eq!(lapsed.effective_tier(), PlanTier::Free);

        let active = Entitlement {
            tier: PlanTier::Pro,
            active: true,
        };
        assert_eq!(active.effective_tier(), PlanTier::Pro);
    }
}
