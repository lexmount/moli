use crate::{
    dynamic_script_owner::DynamicScriptPageTaskClaim,
    frame_owner_model::MainDocumentScriptLoadDelayLease,
    host::ModuleFailurePolicy,
    planning::{
        PreparedScript, PreparedScriptSourceLoadOutcome, SharedScriptSourceLoad,
        prepared_script_with_loaded_source,
    },
    types::{ScriptErrorConstructorKind, SharedNavigationResponseResult},
};

use super::{DocumentScriptExecutionLane, DocumentScriptSourceFailureLane};

/// Typed failure carried by one page-owned script source-failure task.
///
/// Parser source loads currently have only a message. Runtime module loads can
/// additionally retain their exact module-failure and JavaScript error
/// constructor classifications. Those facts cross the main-runtime source
/// with the task instead of being reconstructed from message text later.
#[derive(Debug)]
pub(crate) struct PageOwnedDocumentScriptSourceFailure {
    message: String,
    module_failure_policy: Option<ModuleFailurePolicy>,
    error_constructor: Option<ScriptErrorConstructorKind>,
}

impl PageOwnedDocumentScriptSourceFailure {
    pub(crate) fn from_source_load(message: String) -> Self {
        Self {
            message,
            module_failure_policy: None,
            error_constructor: None,
        }
    }

    pub(crate) fn runtime_terminal(
        message: String,
        module_failure_policy: Option<ModuleFailurePolicy>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Self {
        Self {
            message,
            module_failure_policy,
            error_constructor,
        }
    }

    #[cfg(test)]
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        String,
        Option<ModuleFailurePolicy>,
        Option<ScriptErrorConstructorKind>,
    ) {
        (
            self.message,
            self.module_failure_policy,
            self.error_constructor,
        )
    }
}

/// Main page-owned script work after concrete queue payloads have been
/// projected into shared script execution semantics.
///
/// This remains a main adapter payload: it can be built from `PageTask` /
/// `PostParsePageOwnedWork` bridges, then consumed by
/// `PageOwnedDocumentScriptRunner`. Child frame work must not reuse it.
#[derive(Debug)]
pub(crate) enum PageOwnedDocumentScriptWork {
    AsyncSourceFailure {
        lane: DocumentScriptSourceFailureLane,
        script: Box<PreparedScript>,
        failure: PageOwnedDocumentScriptSourceFailure,
        source_network_result: Option<SharedNavigationResponseResult>,
        runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    },
    ScriptWaitingForSource {
        lane: DocumentScriptExecutionLane,
        script: Box<PreparedScript>,
        source_load: SharedScriptSourceLoad,
        completion_wake_claimed: bool,
        runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    },
    Script {
        lane: DocumentScriptExecutionLane,
        script: Box<PreparedScript>,
        runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
        source_network_result: Option<SharedNavigationResponseResult>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    },
}

impl PageOwnedDocumentScriptWork {
    pub(crate) fn matches_main_document_runtime_target(
        &self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        let (load_delay_owner, runtime_script_owner) = match self {
            Self::AsyncSourceFailure {
                load_delay_binding,
                runtime_script_claim,
                ..
            }
            | Self::ScriptWaitingForSource {
                load_delay_binding,
                runtime_script_claim,
                ..
            }
            | Self::Script {
                load_delay_binding,
                runtime_script_claim,
                ..
            } => (
                load_delay_binding.as_ref().map(|binding| binding.owner()),
                runtime_script_claim.as_ref().map(|claim| claim.owner()),
            ),
        };
        if load_delay_owner.is_some_and(|binding_owner| binding_owner != owner)
            || runtime_script_owner.is_some_and(|claim_owner| claim_owner != owner)
        {
            return false;
        }
        true
    }

    pub(crate) fn script(lane: DocumentScriptExecutionLane, script: PreparedScript) -> Self {
        Self::Script {
            lane,
            script: Box::new(script),
            runtime_script_claim: None,
            source_network_result: None,
            load_delay_binding: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn script_waiting_for_source(
        lane: DocumentScriptExecutionLane,
        script: PreparedScript,
        source_load: SharedScriptSourceLoad,
    ) -> Self {
        Self::ScriptWaitingForSource {
            lane,
            script: Box::new(script),
            source_load,
            completion_wake_claimed: false,
            runtime_script_claim: None,
            load_delay_binding: None,
        }
    }

    pub(crate) fn parser_async_script(
        lane: DocumentScriptExecutionLane,
        script: PreparedScript,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Self {
        Self::Script {
            lane,
            script: Box::new(script),
            runtime_script_claim: None,
            source_network_result: None,
            load_delay_binding,
        }
    }

    pub(crate) fn parser_async_script_waiting_for_source(
        lane: DocumentScriptExecutionLane,
        script: PreparedScript,
        source_load: SharedScriptSourceLoad,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Self {
        Self::ScriptWaitingForSource {
            lane,
            script: Box::new(script),
            source_load,
            completion_wake_claimed: false,
            runtime_script_claim: None,
            load_delay_binding,
        }
    }

    pub(crate) fn parser_async_source_failure(
        lane: DocumentScriptSourceFailureLane,
        script: PreparedScript,
        error: String,
        source_network_result: Option<SharedNavigationResponseResult>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Self {
        Self::AsyncSourceFailure {
            lane,
            script: Box::new(script),
            failure: PageOwnedDocumentScriptSourceFailure::from_source_load(error),
            source_network_result,
            runtime_script_claim: None,
            load_delay_binding,
        }
    }

    pub(crate) fn phase_sort_key(&self) -> (u8, usize) {
        match self {
            Self::Script { lane, script, .. }
            | Self::ScriptWaitingForSource { lane, script, .. } => match lane {
                DocumentScriptExecutionLane::ParserBlocking
                | DocumentScriptExecutionLane::ParseTimeAsync
                | DocumentScriptExecutionLane::ClassicDefer
                | DocumentScriptExecutionLane::ModuleDefer => (1, script.position),
                DocumentScriptExecutionLane::AsyncPhase => (5, script.position),
            },
            Self::AsyncSourceFailure { lane, script, .. } => match lane {
                DocumentScriptSourceFailureLane::ParseTimeAsync => (2, script.position),
                DocumentScriptSourceFailureLane::AsyncPhase => (5, script.position),
            },
        }
    }

    pub(crate) fn as_script(&self) -> &PreparedScript {
        match self {
            Self::Script { script, .. }
            | Self::ScriptWaitingForSource { script, .. }
            | Self::AsyncSourceFailure { script, .. } => script,
        }
    }

    pub(crate) fn as_script_mut(&mut self) -> &mut PreparedScript {
        match self {
            Self::Script { script, .. }
            | Self::ScriptWaitingForSource { script, .. }
            | Self::AsyncSourceFailure { script, .. } => script,
        }
    }

    pub(crate) fn is_defer_like(&self) -> bool {
        matches!(
            self,
            Self::Script {
                lane: DocumentScriptExecutionLane::ClassicDefer
                    | DocumentScriptExecutionLane::ModuleDefer,
                ..
            } | Self::ScriptWaitingForSource {
                lane: DocumentScriptExecutionLane::ClassicDefer
                    | DocumentScriptExecutionLane::ModuleDefer,
                ..
            }
        )
    }

    pub(crate) fn starts_after_domcontentloaded_boundary(&self) -> bool {
        matches!(
            self,
            Self::Script {
                lane: DocumentScriptExecutionLane::AsyncPhase,
                ..
            } | Self::ScriptWaitingForSource {
                lane: DocumentScriptExecutionLane::AsyncPhase,
                ..
            } | Self::AsyncSourceFailure {
                lane: DocumentScriptSourceFailureLane::AsyncPhase,
                ..
            }
        )
    }

    pub(crate) fn is_async_phase(&self) -> bool {
        self.starts_after_domcontentloaded_boundary()
    }

    pub(crate) fn is_waiting_for_source_load(&self) -> bool {
        matches!(self, Self::ScriptWaitingForSource { .. })
    }

    pub(crate) fn pending_source_load(&self) -> Option<SharedScriptSourceLoad> {
        match self {
            Self::ScriptWaitingForSource { source_load, .. } => Some(source_load.clone()),
            Self::Script { .. } | Self::AsyncSourceFailure { .. } => None,
        }
    }

    pub(crate) fn take_load_delay_binding(&mut self) -> Option<MainDocumentScriptLoadDelayLease> {
        match self {
            Self::AsyncSourceFailure {
                load_delay_binding, ..
            }
            | Self::ScriptWaitingForSource {
                load_delay_binding, ..
            }
            | Self::Script {
                load_delay_binding, ..
            } => load_delay_binding.take(),
        }
    }

    pub(crate) fn complete_source_load_if_ready(&mut self) -> bool {
        let Self::ScriptWaitingForSource {
            lane,
            script,
            source_load,
            completion_wake_claimed: _,
            runtime_script_claim,
            load_delay_binding,
        } = self
        else {
            return false;
        };
        let Some(outcome) = source_load.try_outcome() else {
            return false;
        };
        let lane = *lane;
        let runtime_script_claim = runtime_script_claim.take();
        let load_delay_binding = load_delay_binding.take();
        let original_script = script.as_ref().clone();
        *self = page_owned_document_script_work_from_source_load_outcome(
            lane,
            original_script,
            runtime_script_claim,
            outcome,
            load_delay_binding,
        );
        true
    }

    pub(crate) fn claim_source_load_completion_wake(&mut self) -> Option<SharedScriptSourceLoad> {
        let Self::ScriptWaitingForSource {
            source_load,
            completion_wake_claimed,
            ..
        } = self
        else {
            return None;
        };
        if *completion_wake_claimed {
            return None;
        }
        *completion_wake_claimed = true;
        Some(source_load.clone())
    }
}

fn page_owned_document_script_work_from_source_load_outcome(
    lane: DocumentScriptExecutionLane,
    script: PreparedScript,
    runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    outcome: PreparedScriptSourceLoadOutcome,
    load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
) -> PageOwnedDocumentScriptWork {
    let PreparedScriptSourceLoadOutcome {
        source_result,
        source_bytes,
        network_result,
    } = outcome;
    match source_result {
        Ok(source) => PageOwnedDocumentScriptWork::Script {
            lane,
            script: Box::new(prepared_script_with_loaded_source(
                script,
                source,
                source_bytes,
            )),
            runtime_script_claim,
            source_network_result: network_result,
            load_delay_binding,
        },
        Err(error) => match lane {
            DocumentScriptExecutionLane::ParseTimeAsync => {
                PageOwnedDocumentScriptWork::AsyncSourceFailure {
                    lane: DocumentScriptSourceFailureLane::ParseTimeAsync,
                    script: Box::new(script),
                    failure: PageOwnedDocumentScriptSourceFailure::from_source_load(error),
                    source_network_result: network_result,
                    runtime_script_claim,
                    load_delay_binding,
                }
            }
            DocumentScriptExecutionLane::AsyncPhase => {
                PageOwnedDocumentScriptWork::AsyncSourceFailure {
                    lane: DocumentScriptSourceFailureLane::AsyncPhase,
                    script: Box::new(script),
                    failure: PageOwnedDocumentScriptSourceFailure::from_source_load(error),
                    source_network_result: network_result,
                    runtime_script_claim,
                    load_delay_binding,
                }
            }
            DocumentScriptExecutionLane::ClassicDefer
            | DocumentScriptExecutionLane::ModuleDefer => {
                // Parser-defer prefetch is an optimization. On failure the
                // ordinary execution path remains responsible for the final
                // fetch and script error semantics.
                PageOwnedDocumentScriptWork::Script {
                    lane,
                    script: Box::new(script),
                    runtime_script_claim,
                    source_network_result: None,
                    load_delay_binding,
                }
            }
            DocumentScriptExecutionLane::ParserBlocking => {
                panic!("parser-blocking scripts cannot wait in post-parse owner work")
            }
        },
    }
}
