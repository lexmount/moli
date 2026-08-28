use super::JsContextHost;
use crate::{
    document_runtime::{
        AuxiliaryBrowsingContextCreationAdmission, AuxiliaryBrowsingContextCreationDenial,
        DocumentPolicyContainer,
    },
    runtime::{RendererPopupBlockerPolicy, RendererPopupCreationUserActivation},
};
use std::time::{Duration, Instant};

const TRANSIENT_USER_ACTIVATION_LIFESPAN: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug)]
struct TransientUserActivationGrant {
    generation: u64,
    expires_at: Instant,
}

/// Page/frame-tree activation state. Moli currently runs every local
/// frame realm under one Page owner, so this ledger is the stable aggregate
/// consumed by top-level auxiliary creation.
#[derive(Debug, Default)]
pub(super) struct TransientUserActivationLedger {
    has_been_active: bool,
    next_generation: u64,
    active_grant: Option<TransientUserActivationGrant>,
}

impl TransientUserActivationLedger {
    fn notify_at(&mut self, now: Instant) -> u64 {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("transient user-activation generation exhausted");
        let generation = self.next_generation;
        self.has_been_active = true;
        self.active_grant = Some(TransientUserActivationGrant {
            generation,
            expires_at: now + TRANSIENT_USER_ACTIVATION_LIFESPAN,
        });
        generation
    }

    fn active_generation_at(&self, now: Instant) -> Option<u64> {
        self.active_grant
            .filter(|grant| now <= grant.expires_at)
            .map(|grant| grant.generation)
    }

    fn consume_at(&mut self, now: Instant) -> Option<u64> {
        let generation = self.active_generation_at(now);
        self.active_grant = None;
        generation
    }

    const fn has_been_active(&self) -> bool {
        self.has_been_active
    }
}

impl JsContextHost {
    /// Mirrors Blink's `NotifyUserActivation`: a protocol gesture and trusted
    /// activation-triggering input both create persistent transient + sticky
    /// state instead of a stack-scoped command flag.
    pub(crate) fn notify_user_activation(&mut self) {
        self.transient_user_activation.notify_at(Instant::now());
    }

    pub(crate) fn transient_user_activation(&self) -> bool {
        self.transient_user_activation
            .active_generation_at(Instant::now())
            .is_some()
    }

    pub(crate) fn sticky_user_activation(&self) -> bool {
        self.transient_user_activation.has_been_active()
    }

    /// Blink's `DOMWindow::focus()` consumes the incumbent window interaction
    /// before consulting the target's opener exception.
    pub(crate) fn consume_transient_user_activation_for_window_focus(&mut self) -> bool {
        self.transient_user_activation
            .consume_at(Instant::now())
            .is_some()
    }

    /// Admits and freezes one *new* auxiliary context transaction. Callers
    /// must resolve existing targets first because navigation to an existing
    /// context neither consults the popup blocker nor consumes activation.
    pub(crate) fn admit_new_auxiliary_browsing_context(
        &mut self,
        creator_policy_container: DocumentPolicyContainer,
    ) -> Result<AuxiliaryBrowsingContextCreationAdmission, AuxiliaryBrowsingContextCreationDenial>
    {
        let creation_policy =
            creator_policy_container.into_auxiliary_browsing_context_creation_policy()?;
        let now = Instant::now();
        let observed_generation = self.transient_user_activation.active_generation_at(now);
        if observed_generation.is_none()
            && self.browser_context_runtime.popup_blocker_policy()
                == RendererPopupBlockerPolicy::RequireTransientActivation
        {
            return Err(
                AuxiliaryBrowsingContextCreationDenial::BlockedWithoutTransientUserActivation,
            );
        }
        let consumed_generation = self.transient_user_activation.consume_at(now);
        Ok(AuxiliaryBrowsingContextCreationAdmission::new(
            creation_policy,
            RendererPopupCreationUserActivation::new(observed_generation, consumed_generation),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_activation_expires_without_clearing_sticky_activation() {
        let start = Instant::now();
        let mut ledger = TransientUserActivationLedger::default();

        assert_eq!(ledger.active_generation_at(start), None);
        assert!(!ledger.has_been_active());
        assert_eq!(ledger.notify_at(start), 1);
        assert_eq!(ledger.active_generation_at(start), Some(1));
        assert!(ledger.has_been_active());
        assert_eq!(
            ledger.active_generation_at(start + TRANSIENT_USER_ACTIVATION_LIFESPAN),
            Some(1)
        );
        assert_eq!(
            ledger.active_generation_at(
                start + TRANSIENT_USER_ACTIVATION_LIFESPAN + Duration::from_nanos(1)
            ),
            None
        );
        assert!(ledger.has_been_active());
    }

    #[test]
    fn transient_activation_is_consumed_once_and_renotify_gets_a_new_generation() {
        let start = Instant::now();
        let mut ledger = TransientUserActivationLedger::default();

        assert_eq!(ledger.notify_at(start), 1);
        assert_eq!(ledger.consume_at(start), Some(1));
        assert_eq!(ledger.consume_at(start), None);
        assert_eq!(ledger.notify_at(start), 2);
        assert_eq!(ledger.active_generation_at(start), Some(2));
    }
}
