use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

/// Read-only, revocable ownership of a pending command's external wait.
/// Retaining this value cannot keep its protocol owner alive.
#[derive(Clone, Debug)]
pub struct CommandWaitLifetime(Weak<AtomicBool>);

impl CommandWaitLifetime {
    pub fn is_active(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|active| active.load(Ordering::Acquire))
    }
}

/// Kept in the existing domain owner. Snapshots may share the marker, so owner
/// teardown revokes it explicitly rather than relying on the last Arc dropping.
#[derive(Clone, Debug)]
pub(crate) struct CommandWaitLifetimeOwner(Arc<AtomicBool>);

impl Default for CommandWaitLifetimeOwner {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }
}

impl PartialEq for CommandWaitLifetimeOwner {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CommandWaitLifetimeOwner {}

impl CommandWaitLifetimeOwner {
    pub(crate) fn lifetime(&self) -> CommandWaitLifetime {
        CommandWaitLifetime(Arc::downgrade(&self.0))
    }

    pub(crate) fn revoke(&self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_lifetime_does_not_keep_owner_alive() {
        let owner = CommandWaitLifetimeOwner::default();
        let lifetime = owner.lifetime();
        assert!(lifetime.is_active());
        drop(owner);
        assert!(!lifetime.is_active());
    }

    #[test]
    fn explicit_revocation_survives_retained_snapshots_and_replacement() {
        let owner = CommandWaitLifetimeOwner::default();
        let snapshot = owner.clone();
        let lifetime = owner.lifetime();
        owner.revoke();
        assert!(!lifetime.is_active());
        assert!(!snapshot.lifetime().is_active());
        let replacement = CommandWaitLifetimeOwner::default();
        assert!(replacement.lifetime().is_active());
        assert!(!lifetime.is_active());
    }
}
