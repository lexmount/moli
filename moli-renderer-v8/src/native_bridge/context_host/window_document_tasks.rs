use super::{JsContextHost, OwnerDispatchScope, WindowDocumentOwner, WindowDocumentTaskTarget};
use crate::document_runtime::DomHandle;
use crate::runtime::{RendererDocumentLifecycleIdentity, RendererWindowDocumentSource};

/// One PageVm-local payload keyed by a captured Window task target.
///
/// The stable Page queue carries `task_id`, target, and kind; V8/DOM-bearing
/// payload remains in the Host that created it. This record is intentionally
/// shared by task-source families so each migrated API does not rebuild its
/// own current/take/discard ledger.
#[derive(Debug)]
pub(super) struct PendingWindowTask<I, K, P, T> {
    task_id: I,
    target: T,
    kind: K,
    payload: P,
}

impl<I: Copy, K: Copy, P, T: Copy> PendingWindowTask<I, K, P, T> {
    pub(super) fn new(task_id: I, target: T, kind: K, payload: P) -> Self {
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

    pub(super) const fn target(&self) -> T {
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

/// Reusable PageVm-local ledger for captured Window task targets.
///
/// Id allocation, immutable target capture, exact removal, and stale discard
/// live here. Queue coalescing remains family policy expressed by the caller's
/// `find_slot_index` predicate.
#[derive(Debug)]
pub(super) struct WindowTaskLedger<I, K, P, T> {
    pending: Vec<PendingWindowTask<I, K, P, T>>,
    next_task_id: u64,
}

impl<I, K, P, T> Default for WindowTaskLedger<I, K, P, T> {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            next_task_id: 1,
        }
    }
}

impl<I: Copy + Eq, K: Copy + Eq, P, T: Copy + Eq> WindowTaskLedger<I, K, P, T> {
    pub(super) fn allocate_task_id(&mut self, from_raw: impl FnOnce(u64) -> I) -> I {
        let raw = self.next_task_id;
        self.next_task_id = raw.checked_add(1).expect("Window task id overflow");
        from_raw(raw)
    }

    pub(super) fn pending(&self, task_id: I) -> Option<&PendingWindowTask<I, K, P, T>> {
        self.pending
            .iter()
            .find(|pending| pending.task_id() == task_id)
    }

    pub(super) fn find_slot_index(
        &self,
        target: T,
        kind: K,
        payload_matches: impl Fn(&P) -> bool,
    ) -> Option<usize> {
        self.pending.iter().position(|pending| {
            pending.target() == target
                && pending.kind() == kind
                && payload_matches(pending.payload())
        })
    }

    pub(super) fn push(&mut self, pending: PendingWindowTask<I, K, P, T>) {
        self.pending.push(pending);
    }

    pub(super) fn at(&self, index: usize) -> &PendingWindowTask<I, K, P, T> {
        &self.pending[index]
    }

    pub(super) fn retain(&mut self, keep: impl FnMut(&PendingWindowTask<I, K, P, T>) -> bool) {
        self.pending.retain(keep);
    }

    pub(super) fn remove_at(&mut self, index: usize) -> PendingWindowTask<I, K, P, T> {
        self.pending.remove(index)
    }

    pub(super) fn replace(
        &mut self,
        index: usize,
        pending: PendingWindowTask<I, K, P, T>,
    ) -> PendingWindowTask<I, K, P, T> {
        std::mem::replace(&mut self.pending[index], pending)
    }

    pub(super) fn remove(&mut self, task_id: I) -> Option<PendingWindowTask<I, K, P, T>> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.task_id() == task_id)?;
        Some(self.pending.remove(index))
    }

    pub(super) fn remove_exact(
        &mut self,
        task_id: I,
        target: T,
        kind: K,
    ) -> Option<PendingWindowTask<I, K, P, T>> {
        let pending = self.pending(task_id)?;
        if pending.target() != target || pending.kind() != kind {
            return None;
        }
        self.remove(task_id)
    }
}

pub(super) type PendingExactWindowDocumentTask<I, K, P> =
    PendingWindowTask<I, K, P, WindowDocumentTaskTarget>;
pub(super) type ExactWindowDocumentTaskLedger<I, K, P> =
    WindowTaskLedger<I, K, P, WindowDocumentTaskTarget>;

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
                let frame_id = self.frame_owner_frame_id_for_child_handle(handle)?.0;
                RendererWindowDocumentSource::ChildFrame {
                    frame_id,
                    local_window_id: owner.local_window_id.0,
                    document_id: owner.document_id.0,
                }
            }
            (
                WindowDocumentOwner::LightweightPopup(owner),
                OwnerDispatchScope::LightweightPopup(popup_id),
            ) if owner.popup_id() == popup_id => RendererWindowDocumentSource::LightweightPopup {
                popup_id,
                popup_document_id: owner.document_id().as_u64(),
            },
            _ => return None,
        };
        Some((target, root_document, source))
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
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                WindowDocumentOwner::LightweightPopup(
                    self.current_lightweight_popup_document_owner(popup_id)?,
                )
            }
        };
        Some(WindowDocumentTaskTarget::new(owner, dispatch_scope))
    }

    pub(crate) fn window_document_task_target_for_owner(
        &self,
        owner: WindowDocumentOwner,
    ) -> Option<WindowDocumentTaskTarget> {
        let dispatch_scope = match owner {
            WindowDocumentOwner::Frame(frame) => {
                if self.current_main_document_task_owner() == Some(frame) {
                    OwnerDispatchScope::Top
                } else {
                    OwnerDispatchScope::Child(
                        self.frame_owner_store
                            .child_handle_for_document_task_owner(frame)?,
                    )
                }
            }
            WindowDocumentOwner::LightweightPopup(popup) => {
                OwnerDispatchScope::LightweightPopup(popup.popup_id())
            }
        };
        self.window_document_owner_is_current_for_dispatch_scope(owner, dispatch_scope)
            .then(|| WindowDocumentTaskTarget::new(owner, dispatch_scope))
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
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                self.ensure_lightweight_popup_execution_context(scope, popup_id)
                    .then_some(())?;
                self.lightweight_popup_document_handle(popup_id)?
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
