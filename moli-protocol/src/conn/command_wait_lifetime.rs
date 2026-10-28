use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

/// Read-only, revocable ownership of a pending command's external wait.
/// Retaining this value cannot keep its protocol owner alive.
#[derive(Clone, Debug)]
pub struct CommandWaitLifetime {
    state: Weak<CommandWaitLifetimeState>,
    revoked: Arc<tokio::sync::Notify>,
}

#[derive(Debug)]
struct CommandWaitLifetimeState {
    active: AtomicBool,
    revoked: Arc<tokio::sync::Notify>,
}

impl Drop for CommandWaitLifetimeState {
    fn drop(&mut self) {
        self.revoked.notify_waiters();
    }
}

impl CommandWaitLifetime {
    pub fn is_active(&self) -> bool {
        self.state
            .upgrade()
            .is_some_and(|state| state.active.load(Ordering::Acquire))
    }
    pub async fn wait_revoked(&self) {
        loop {
            let revoked = self.revoked.notified();
            tokio::pin!(revoked);
            revoked.as_mut().enable();
            if !self.is_active() {
                return;
            }
            revoked.await;
        }
    }
}

/// Kept in the existing domain owner. Snapshots may share the marker, so owner
/// teardown revokes it explicitly rather than relying on the last Arc dropping.
#[derive(Clone, Debug)]
pub(crate) struct CommandWaitLifetimeOwner(Arc<CommandWaitLifetimeState>);

impl Default for CommandWaitLifetimeOwner {
    fn default() -> Self {
        Self(Arc::new(CommandWaitLifetimeState {
            active: AtomicBool::new(true),
            revoked: Arc::new(tokio::sync::Notify::new()),
        }))
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
        CommandWaitLifetime {
            state: Arc::downgrade(&self.0),
            revoked: self.0.revoked.clone(),
        }
    }

    pub(crate) fn revoke(&self) {
        self.0.active.store(false, Ordering::Release);
        self.0.revoked.notify_waiters();
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
    #[tokio::test]
    async fn revocation_and_owner_drop_wake_waiters_without_extending_owner_lifetime() {
        for explicit in [true, false] {
            let owner = CommandWaitLifetimeOwner::default();
            let lifetime = owner.lifetime();
            let revoked = lifetime.wait_revoked();
            tokio::pin!(revoked);
            assert!(futures_util::poll!(&mut revoked).is_pending());
            if explicit {
                owner.revoke();
                assert!(!lifetime.is_active());
                assert!(futures_util::poll!(&mut revoked).is_ready());
            } else {
                drop(owner);
                assert!(!lifetime.is_active());
                assert!(futures_util::poll!(&mut revoked).is_ready());
            }
        }
    }
}
