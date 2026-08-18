use std::{fmt, num::NonZeroUsize};

use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    frame_owner_model::{FrameDocumentTaskOwner, FrameRealmId},
    module_runtime::DynamicModuleImportOwner,
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
    types::ScriptErrorConstructorKind,
};

use super::RendererOwnerWakeSender;

/// Exact PageVm-local target captured by a module Promise reaction callback.
///
/// The stable source adds the root `RendererDocumentToken`. Main-document
/// module reactions carry the exact main Document owner. Child reactions carry
/// the exact child Document and realm;
/// dynamic imports retain the execution-context owner captured at import()
/// acceptance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageModuleReactionTarget {
    DocumentModuleScript {
        document_owner: FrameDocumentTaskOwner,
    },
    ChildParserModule {
        document_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    },
    DynamicModuleImport {
        import_owner: DynamicModuleImportOwner,
    },
}

/// One concrete host continuation produced by a V8 module Promise callback.
pub(crate) enum RendererPageModuleReactionEvent {
    DocumentModuleScriptEvaluationFulfilled {
        document_owner: FrameDocumentTaskOwner,
        reaction_id: u64,
    },
    DocumentModuleScriptEvaluationRejected {
        document_owner: FrameDocumentTaskOwner,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
    },
    ChildParserModuleEvaluationFulfilled {
        document_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        reaction_id: u64,
    },
    ChildParserModuleEvaluationRejected {
        document_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
    },
    DynamicModuleEvaluationFulfilled {
        import_owner: DynamicModuleImportOwner,
        reaction_id: u64,
    },
    DynamicModuleEvaluationRejected {
        import_owner: DynamicModuleImportOwner,
        reaction_id: u64,
        reason: v8::Global<v8::Value>,
    },
}

impl RendererPageModuleReactionEvent {
    pub(crate) const fn target(&self) -> RendererPageModuleReactionTarget {
        match self {
            Self::DocumentModuleScriptEvaluationFulfilled { document_owner, .. }
            | Self::DocumentModuleScriptEvaluationRejected { document_owner, .. } => {
                RendererPageModuleReactionTarget::DocumentModuleScript {
                    document_owner: *document_owner,
                }
            }
            Self::ChildParserModuleEvaluationFulfilled {
                document_owner,
                realm_id,
                ..
            }
            | Self::ChildParserModuleEvaluationRejected {
                document_owner,
                realm_id,
                ..
            } => RendererPageModuleReactionTarget::ChildParserModule {
                document_owner: *document_owner,
                realm_id: *realm_id,
            },
            Self::DynamicModuleEvaluationFulfilled { import_owner, .. }
            | Self::DynamicModuleEvaluationRejected { import_owner, .. } => {
                RendererPageModuleReactionTarget::DynamicModuleImport {
                    import_owner: *import_owner,
                }
            }
        }
    }

    pub(crate) const fn reaction_id(&self) -> u64 {
        match self {
            Self::DocumentModuleScriptEvaluationFulfilled { reaction_id, .. }
            | Self::DocumentModuleScriptEvaluationRejected { reaction_id, .. }
            | Self::ChildParserModuleEvaluationFulfilled { reaction_id, .. }
            | Self::ChildParserModuleEvaluationRejected { reaction_id, .. }
            | Self::DynamicModuleEvaluationFulfilled { reaction_id, .. }
            | Self::DynamicModuleEvaluationRejected { reaction_id, .. } => *reaction_id,
        }
    }
}

impl fmt::Debug for RendererPageModuleReactionEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("RendererPageModuleReactionEvent");
        debug
            .field("target", &self.target())
            .field("reaction_id", &self.reaction_id());
        match self {
            Self::DocumentModuleScriptEvaluationRejected {
                reason,
                error_constructor,
                ..
            }
            | Self::ChildParserModuleEvaluationRejected {
                reason,
                error_constructor,
                ..
            } => {
                debug
                    .field("reason", reason)
                    .field("error_constructor", error_constructor);
            }
            Self::DynamicModuleEvaluationRejected { .. } => {
                debug.field("reason", &"<v8::Global<Value>>");
            }
            Self::DocumentModuleScriptEvaluationFulfilled { .. }
            | Self::ChildParserModuleEvaluationFulfilled { .. }
            | Self::DynamicModuleEvaluationFulfilled { .. } => {}
        }
        debug.finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageModuleReactionOwner {
    root_document: RendererDocumentToken,
    target: RendererPageModuleReactionTarget,
}

impl RendererPageModuleReactionOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        target: RendererPageModuleReactionTarget,
    ) -> Self {
        Self {
            root_document,
            target,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn target(self) -> RendererPageModuleReactionTarget {
        self.target
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageModuleReactionTask {
    owner: RendererPageModuleReactionOwner,
    event: RendererPageModuleReactionEvent,
}

impl RendererPageModuleReactionTask {
    fn new(root_document: RendererDocumentToken, event: RendererPageModuleReactionEvent) -> Self {
        let owner = RendererPageModuleReactionOwner::new(root_document, event.target());
        Self { owner, event }
    }

    pub(crate) const fn owner(&self) -> RendererPageModuleReactionOwner {
        self.owner
    }

    pub(crate) fn into_event(self) -> RendererPageModuleReactionEvent {
        self.event
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageModuleReactionRouteClosed;

#[derive(Clone, Debug)]
pub(crate) struct RendererPageModuleReactionRoute {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageModuleReactionTask>,
        RendererPageModuleReactionReadySignal,
    >,
}

impl RendererPageModuleReactionRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageModuleReactionSender {
        RendererPageModuleReactionSender {
            task_route: self.task_route.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageModuleReactionSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped producer installed in `JsContextHost` before page script can
/// create a module evaluation Promise.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageModuleReactionSender {
    task_route: OwnerReadyTaskRoute<
        ReadyPageTask<RendererPageModuleReactionTask>,
        RendererPageModuleReactionReadySignal,
    >,
    root_document: RendererDocumentToken,
}

impl RendererPageModuleReactionSender {
    pub(crate) fn send(
        &self,
        event: RendererPageModuleReactionEvent,
    ) -> Result<(), RendererPageModuleReactionRouteClosed> {
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(
                RendererPageModuleReactionTask::new(self.root_document, event),
            ))
            .map_err(|_| RendererPageModuleReactionRouteClosed)
    }
}

#[derive(Clone, Debug)]
struct RendererPageModuleReactionReadySignal {
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for RendererPageModuleReactionReadySignal {
    fn signal_ready(&self) {
        self.owner_wake.signal_module_reaction();
    }
}

/// Unique stable Page consumer for module reaction continuation records.
#[derive(Debug)]
pub(crate) struct RendererPageModuleReactionSource {
    source: OwnerReadyTaskSource<
        ReadyPageTask<RendererPageModuleReactionTask>,
        RendererPageModuleReactionReadySignal,
    >,
}

impl RendererPageModuleReactionSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RendererPageModuleReactionReadySignal { owner_wake }),
        }
    }

    pub(crate) fn route(&self) -> RendererPageModuleReactionRoute {
        RendererPageModuleReactionRoute {
            task_route: self.source.route(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageModuleReactionOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageModuleReactionTask,
    )> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageModuleReactionRoute) -> bool {
        route.same_route_as(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageModuleReactionCurrentEffect {
    /// A document or child module-evaluation record advanced its strongly
    /// typed owner state and may have published a later continuation task.
    ModuleStateUpdated,
    /// A dynamic-import evaluation record settled the exact user-facing
    /// import Promise. Its reactions remain queued until task completion.
    DynamicImportPromiseSettled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageModuleReactionTargetEffect {
    AppliedToCurrentOwner(PageModuleReactionCurrentEffect),
    /// The route ticket was current, but its one-shot reaction payload had
    /// already been retired or consumed. No current task body was applied.
    DiscardedMissingReaction,
    IgnoredStaleOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageModuleReactionFollowup {
    None,
    MainParserOwnedEvaluations { ready_action_count: NonZeroUsize },
    RuntimeOwnedModuleContinuation,
}

impl PageModuleReactionFollowup {
    pub(crate) fn main_parser_owned_evaluations(ready_action_count: usize) -> Self {
        NonZeroUsize::new(ready_action_count)
            .map(|ready_action_count| Self::MainParserOwnedEvaluations { ready_action_count })
            .unwrap_or(Self::None)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageModuleReactionApplication {
    Applied {
        current_effect: PageModuleReactionCurrentEffect,
        followup: PageModuleReactionFollowup,
    },
    NoPendingReaction,
}

impl PageModuleReactionApplication {
    pub(crate) const fn module_state_updated(followup: PageModuleReactionFollowup) -> Self {
        Self::Applied {
            current_effect: PageModuleReactionCurrentEffect::ModuleStateUpdated,
            followup,
        }
    }

    pub(crate) const fn dynamic_import_promise_settled() -> Self {
        Self::Applied {
            current_effect: PageModuleReactionCurrentEffect::DynamicImportPromiseSettled,
            followup: PageModuleReactionFollowup::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageModuleReactionTurnAction {
    owner: RendererPageModuleReactionOwner,
    target_effect: PageModuleReactionTargetEffect,
}

impl PageModuleReactionTurnAction {
    pub(crate) const fn new(
        owner: RendererPageModuleReactionOwner,
        target_effect: PageModuleReactionTargetEffect,
    ) -> Self {
        Self {
            owner,
            target_effect,
        }
    }

    pub(crate) const fn target_effect(self) -> PageModuleReactionTargetEffect {
        self.target_effect
    }
}

pub(crate) type PageModuleReactionTurnOutcome = PageOwnerTurnOutcome<PageModuleReactionTurnAction>;
