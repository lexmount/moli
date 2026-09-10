use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use moli_shared_worker::SharedWorkerInstanceId;

/// Stable lifetime of one protocol session attached to one renderer-owned
/// SharedWorker target.
///
/// The target registry owns this scope while the attachment is live. Prepared
/// output observes it through a weak identity so a normal session detach makes
/// held output stale without consulting whichever worker target is current at
/// drain time. Target destruction transfers the strong scope to the prepared
/// detach output; this preserves already accepted `console -> detached` and
/// `attached -> detached` ordering until that exact detach is consumed.
#[derive(Clone, Debug)]
pub(crate) struct TargetSharedWorkerProtocolAttachmentScope {
    inner: Arc<TargetSharedWorkerProtocolAttachmentScopeInner>,
}

#[derive(Debug)]
struct TargetSharedWorkerProtocolAttachmentScopeInner {
    current: AtomicBool,
}

/// Exact protocol attachment for one renderer SharedWorker instance.
///
/// `target_id` and `session_id` are connection-local opaque ids, while the
/// renderer host and instance ids bind the protocol projection to the worker
/// that produced it. The weak scope is the lifetime authority: ids alone must
/// never let held output follow a detached session or a later worker target.
#[derive(Clone, Debug)]
pub(crate) struct TargetSharedWorkerProtocolAttachmentIdentity {
    browser_context_id: String,
    renderer_instance_id: SharedWorkerInstanceId,
    owner_target_id: Option<String>,
    target_id: String,
    session_id: String,
    scope: Weak<TargetSharedWorkerProtocolAttachmentScopeInner>,
}

/// Move-owned authority to publish and retire one SharedWorker detachment.
///
/// Removing the worker target transfers its per-session scope here instead of
/// dropping it immediately. Earlier prepared output for the same attachment
/// therefore remains authorized until the ordered detach output runs. Calling
/// `retire` invalidates every weak identity for that attachment.
#[derive(Clone, Debug)]
pub(crate) struct TargetSharedWorkerProtocolAttachmentRetirement {
    identity: TargetSharedWorkerProtocolAttachmentIdentity,
    scope: TargetSharedWorkerProtocolAttachmentScope,
}

impl TargetSharedWorkerProtocolAttachmentScope {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(TargetSharedWorkerProtocolAttachmentScopeInner {
                current: AtomicBool::new(true),
            }),
        }
    }

    pub(crate) fn bind(
        &self,
        browser_context_id: impl Into<String>,
        renderer_instance_id: SharedWorkerInstanceId,
        owner_target_id: Option<String>,
        target_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> TargetSharedWorkerProtocolAttachmentIdentity {
        TargetSharedWorkerProtocolAttachmentIdentity {
            browser_context_id: browser_context_id.into(),
            renderer_instance_id,
            owner_target_id,
            target_id: target_id.into(),
            session_id: session_id.into(),
            scope: Arc::downgrade(&self.inner),
        }
    }

    pub(crate) fn into_retirement(
        self,
        identity: TargetSharedWorkerProtocolAttachmentIdentity,
    ) -> TargetSharedWorkerProtocolAttachmentRetirement {
        assert!(
            self.observes(&identity),
            "shared-worker attachment retirement must consume its own live identity"
        );
        TargetSharedWorkerProtocolAttachmentRetirement {
            identity,
            scope: self,
        }
    }

    fn observes(&self, identity: &TargetSharedWorkerProtocolAttachmentIdentity) -> bool {
        let Some(observed) = identity.scope.upgrade() else {
            return false;
        };
        Arc::ptr_eq(&self.inner, &observed) && observed.current.load(Ordering::Acquire)
    }
}

impl PartialEq for TargetSharedWorkerProtocolAttachmentScope {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for TargetSharedWorkerProtocolAttachmentScope {}

impl TargetSharedWorkerProtocolAttachmentIdentity {
    pub(crate) fn browser_context_id(&self) -> &str {
        &self.browser_context_id
    }

    pub(crate) fn renderer_instance_id(&self) -> SharedWorkerInstanceId {
        self.renderer_instance_id
    }

    pub(crate) fn owner_target_id(&self) -> Option<&str> {
        self.owner_target_id.as_deref()
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn is_current(&self) -> bool {
        self.scope
            .upgrade()
            .is_some_and(|scope| scope.current.load(Ordering::Acquire))
    }
}

impl PartialEq for TargetSharedWorkerProtocolAttachmentIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.browser_context_id == other.browser_context_id
            && self.renderer_instance_id == other.renderer_instance_id
            && self.owner_target_id == other.owner_target_id
            && self.target_id == other.target_id
            && self.session_id == other.session_id
            && Weak::ptr_eq(&self.scope, &other.scope)
    }
}

impl Eq for TargetSharedWorkerProtocolAttachmentIdentity {}

impl TargetSharedWorkerProtocolAttachmentRetirement {
    pub(crate) fn identity(&self) -> &TargetSharedWorkerProtocolAttachmentIdentity {
        &self.identity
    }

    pub(crate) fn is_current(&self) -> bool {
        self.scope.observes(&self.identity)
    }

    pub(crate) fn retire(self) {
        self.scope.inner.current.store(false, Ordering::Release);
    }
}

impl PartialEq for TargetSharedWorkerProtocolAttachmentRetirement {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity && self.scope == other.scope
    }
}

impl Eq for TargetSharedWorkerProtocolAttachmentRetirement {}

#[cfg(test)]
mod tests {
    use moli_shared_worker::SharedWorkerInstanceId;

    use super::TargetSharedWorkerProtocolAttachmentScope;

    fn binding(
        scope: &TargetSharedWorkerProtocolAttachmentScope,
    ) -> super::TargetSharedWorkerProtocolAttachmentIdentity {
        scope.bind(
            "BID-1",
            SharedWorkerInstanceId::from_u64(11),
            Some("TID-owner".to_owned()),
            "TID-worker",
            "SID-worker",
        )
    }

    #[test]
    fn attachment_identity_expires_with_ordinary_owner_drop() {
        let scope = TargetSharedWorkerProtocolAttachmentScope::new();
        let identity = binding(&scope);
        assert!(identity.is_current());

        drop(scope);

        assert!(!identity.is_current());
    }

    #[test]
    fn retirement_keeps_ordered_output_live_then_invalidates_every_observer() {
        let scope = TargetSharedWorkerProtocolAttachmentScope::new();
        let identity = binding(&scope);
        let retirement = scope.into_retirement(identity.clone());

        assert!(identity.is_current());
        assert!(retirement.is_current());

        retirement.retire();

        assert!(!identity.is_current());
    }
}
