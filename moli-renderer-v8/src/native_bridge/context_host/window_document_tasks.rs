use super::{
    JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget,
    WindowExecutionContextIdentity,
};
use crate::document_runtime::DomHandle;
use crate::runtime::{
    RendererDocumentLifecycleIdentity, RendererTopLevelNavigationSource,
    RendererWindowDocumentSource,
};

/// One PageVm-local payload keyed by immutable exact-Document identity.
///
/// The stable Page queue carries `task_id`, target, and kind; V8/DOM-bearing
/// payload remains in the Host that created it. This record is intentionally
/// shared by task-source families so each migrated API does not rebuild its
/// own current/take/discard ledger.
#[derive(Debug)]
pub(super) struct PendingExactWindowDocumentTask<I, K, P> {
    task_id: I,
    target: WindowDocumentTaskTarget,
    kind: K,
    payload: P,
}

impl<I: Copy, K: Copy, P> PendingExactWindowDocumentTask<I, K, P> {
    pub(super) fn new(task_id: I, target: WindowDocumentTaskTarget, kind: K, payload: P) -> Self {
        Self {
            task_id,
            target,
            kind,
            payload,
        }
    }

    pub(super) const fn task_id(&self) -> I {
        self.task_id
    }

    pub(super) const fn target(&self) -> WindowDocumentTaskTarget {
        self.target
    }

    pub(super) const fn kind(&self) -> K {
        self.kind
    }

    pub(super) const fn payload(&self) -> &P {
        &self.payload
    }

    pub(super) fn into_payload(self) -> P {
        self.payload
    }
}

/// Reusable PageVm-local ledger for exact Window/Document tasks.
///
/// Id allocation, immutable target capture, exact removal, and stale discard
/// live here. Queue coalescing remains family policy expressed by the caller's
/// `find_slot_index` predicate.
#[derive(Debug)]
pub(super) struct ExactWindowDocumentTaskLedger<I, K, P> {
    pending: Vec<PendingExactWindowDocumentTask<I, K, P>>,
    next_task_id: u64,
}

impl<I, K, P> Default for ExactWindowDocumentTaskLedger<I, K, P> {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            next_task_id: 1,
        }
    }
}

impl<I: Copy + Eq, K: Copy + Eq, P> ExactWindowDocumentTaskLedger<I, K, P> {
    pub(super) fn allocate_task_id(&mut self, from_raw: impl FnOnce(u64) -> I) -> I {
        let raw = self.next_task_id;
        self.next_task_id = raw
            .checked_add(1)
            .expect("exact Window/Document task id overflow");
        from_raw(raw)
    }

    pub(super) fn pending(&self, task_id: I) -> Option<&PendingExactWindowDocumentTask<I, K, P>> {
        self.pending
            .iter()
            .find(|pending| pending.task_id() == task_id)
    }

    pub(super) fn find_slot_index(
        &self,
        target: WindowDocumentTaskTarget,
        kind: K,
        payload_matches: impl Fn(&P) -> bool,
    ) -> Option<usize> {
        self.pending.iter().position(|pending| {
            pending.target() == target
                && pending.kind() == kind
                && payload_matches(pending.payload())
        })
    }

    pub(super) fn push(&mut self, pending: PendingExactWindowDocumentTask<I, K, P>) {
        self.pending.push(pending);
    }

    pub(super) fn at(&self, index: usize) -> &PendingExactWindowDocumentTask<I, K, P> {
        &self.pending[index]
    }

    pub(super) fn remove_at(&mut self, index: usize) -> PendingExactWindowDocumentTask<I, K, P> {
        self.pending.remove(index)
    }

    pub(super) fn replace(
        &mut self,
        index: usize,
        pending: PendingExactWindowDocumentTask<I, K, P>,
    ) -> PendingExactWindowDocumentTask<I, K, P> {
        std::mem::replace(&mut self.pending[index], pending)
    }

    pub(super) fn remove(&mut self, task_id: I) -> Option<PendingExactWindowDocumentTask<I, K, P>> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.task_id() == task_id)?;
        Some(self.pending.remove(index))
    }

    pub(super) fn remove_exact(
        &mut self,
        task_id: I,
        target: WindowDocumentTaskTarget,
        kind: K,
    ) -> Option<PendingExactWindowDocumentTask<I, K, P>> {
        let pending = self.pending(task_id)?;
        if pending.target() != target || pending.kind() != kind {
            return None;
        }
        self.remove(task_id)
    }
}

/// Realm and Document handle resolved for an already-authorized Window task.
///
/// Resolution may materialize a child or popup default context, but it does not
/// decide current/stale ownership. The Page arbiter must match the exact target
/// before handing a task to an executor that calls this helper.
pub(super) struct ResolvedWindowDocumentTaskContext<'s> {
    pub(super) document_handle: DomHandle,
    pub(super) context: v8::Local<'s, v8::Context>,
}

impl JsContextHost {
    /// Freezes the exact Window/Document source of a browser-owner handoff.
    ///
    /// The returned root lifecycle identity is causal metadata for the owning
    /// Page. Child and popup ids identify the concrete initiating Window; a
    /// later protocol consumer must not replace either with the then-current
    /// root frame.
    pub(crate) fn current_renderer_window_document_source(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererDocumentLifecycleIdentity,
        RendererWindowDocumentSource,
    )> {
        let dispatch_scope = self
            .current_runtime_window_execution_context_identity(scope)?
            .dispatch_scope();
        self.renderer_window_document_source_for_dispatch_scope(dispatch_scope)
    }

    pub(crate) fn renderer_window_document_source_for_dispatch_scope(
        &self,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererDocumentLifecycleIdentity,
        RendererWindowDocumentSource,
    )> {
        let root_document = self.root_document_lifecycle_identity()?;
        let target = self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)?;
        let source = match (target.owner(), target.dispatch_scope()) {
            (WindowDocumentOwner::Frame(_), OwnerDispatchScope::Top) => {
                RendererWindowDocumentSource::RootFrame
            }
            (WindowDocumentOwner::Frame(owner), OwnerDispatchScope::Child(handle)) => {
                let (frame_id, _) = self.child_browsing_context_request_scope(handle)?;
                RendererWindowDocumentSource::ChildFrame {
                    frame_id,
                    local_window_id: owner.local_window_id.0,
                    document_id: owner.document_id.0,
                }
            }
        };
        Some((target, root_document, source))
    }

    pub(crate) fn renderer_window_document_source_for_identity(
        &self,
        identity: WindowExecutionContextIdentity,
    ) -> Option<(
        WindowDocumentTaskTarget,
        RendererDocumentLifecycleIdentity,
        RendererWindowDocumentSource,
    )> {
        self.window_execution_context_identity_is_current(identity)
            .then(|| identity.dispatch_scope())
            .and_then(|scope| self.renderer_window_document_source_for_dispatch_scope(scope))
    }

    /// Captures the source-side facts for a top-level navigation before target
    /// selection enters another Window realm or hands scheduling to another
    /// Page. The source URL follows the inherited creator URL for initial
    /// `about:` Documents, matching the policy-container authority already
    /// used by child-frame and popup navigation requests.
    pub(crate) fn renderer_top_level_navigation_source_for_dispatch_scope(
        &self,
        dispatch_scope: OwnerDispatchScope,
        suppress_referrer: bool,
    ) -> Option<RendererTopLevelNavigationSource> {
        let (_, root_document, window) =
            self.renderer_window_document_source_for_dispatch_scope(dispatch_scope)?;
        let (document, policy) = match dispatch_scope {
            OwnerDispatchScope::Top => (
                self.document_handle(),
                self.document_policy_container().clone(),
            ),
            OwnerDispatchScope::Child(handle) => (
                self.child_browsing_context_document_handle(handle)?,
                self.child_browsing_context_policy_container_snapshot(handle)?,
            ),
        };
        let raw_source_url = self.document_url_for_handle(document);
        let source_url = if raw_source_url.scheme() == "about" {
            url::Url::parse(&policy.document_referrer).unwrap_or(raw_source_url)
        } else {
            raw_source_url
        };
        Some(RendererTopLevelNavigationSource::new(
            root_document,
            window,
            source_url.to_string(),
            policy.referrer_policy,
            suppress_referrer,
        ))
    }

    pub(crate) fn renderer_top_level_navigation_source_for_identity(
        &self,
        identity: WindowExecutionContextIdentity,
        suppress_referrer: bool,
    ) -> Option<RendererTopLevelNavigationSource> {
        self.window_execution_context_identity_is_current(identity)
            .then(|| identity.dispatch_scope())
            .and_then(|scope| {
                self.renderer_top_level_navigation_source_for_dispatch_scope(
                    scope,
                    suppress_referrer,
                )
            })
    }

    pub(crate) fn renderer_top_level_navigation_source_for_node(
        &self,
        node: DomHandle,
        suppress_referrer: bool,
    ) -> Option<RendererTopLevelNavigationSource> {
        self.renderer_top_level_navigation_source_for_dispatch_scope(
            self.owner_dispatch_scope_for_node(node)?,
            suppress_referrer,
        )
    }

    pub(crate) fn current_window_document_task_target_for_dispatch_scope(
        &self,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<WindowDocumentTaskTarget> {
        let owner = match dispatch_scope {
            OwnerDispatchScope::Top => {
                WindowDocumentOwner::Frame(self.current_main_document_task_owner()?)
            }
            OwnerDispatchScope::Child(handle) => {
                WindowDocumentOwner::Frame(self.current_child_document_task_owner(handle)?)
            }
        };
        Some(WindowDocumentTaskTarget::new(owner, dispatch_scope))
    }

    pub(crate) fn current_window_document_task_target(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<WindowDocumentTaskTarget> {
        let dispatch_scope = self
            .current_runtime_window_execution_context_identity(scope)?
            .dispatch_scope();
        self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
    }

    /// Resolve the task target from a node's owner Document, not from the realm
    /// whose JavaScript happened to mutate it. This matters when parent script
    /// directly changes an element in a same-origin child Document.
    ///
    /// Inert Documents created by APIs such as DOMParser do not own a modeled
    /// Window/scheduler residence. Their asynchronous element work is projected
    /// onto the incumbent Window task source, which is the only runnable realm
    /// that can observe their event listeners in this runtime.
    pub(crate) fn window_document_task_target_for_node(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        node: DomHandle,
    ) -> Option<WindowDocumentTaskTarget> {
        match self.owner_dispatch_scope_for_node(node) {
            Some(dispatch_scope) => {
                self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
            }
            None => self.current_window_document_task_target(scope),
        }
    }

    pub(super) fn resolve_authorized_window_document_task_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: WindowDocumentTaskTarget,
    ) -> Option<ResolvedWindowDocumentTaskContext<'s>> {
        let dispatch_scope = target.dispatch_scope();
        let document_handle = match dispatch_scope {
            OwnerDispatchScope::Top => self.document_handle(),
            OwnerDispatchScope::Child(handle) => {
                self.ensure_prebootstrapped_child_default_context(scope, handle)
                    .ok()?;
                self.child_browsing_context_document_handle(handle)?
            }
        };
        let identity = self.current_registered_window_execution_context_identity(dispatch_scope)?;
        let (_, context) =
            self.window_execution_context(scope, identity.owner(), dispatch_scope)?;
        Some(ResolvedWindowDocumentTaskContext {
            document_handle,
            context,
        })
    }
}
