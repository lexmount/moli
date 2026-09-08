use super::{
    ModuleFailurePolicy, ScriptEventDispatchPolicy, ScriptEventKind, ScriptEventPolicy,
    ScriptEventSkipReason, ScriptFailurePageTaskPolicy, ScriptHandleExecutionSubject,
    ScriptHandleSource, ScriptHandleStartState, ScriptHandleState, ScriptHostEventSubject,
    ScriptStartCommitKind,
};
use crate::types::{ScriptKind, ScriptSourceKind};

impl ScriptHostEventSubject {
    pub(super) fn unknown() -> Self {
        Self {
            source: ScriptHandleSource::Unknown,
            execution: ScriptHandleExecutionSubject::PendingOrUnknown,
        }
    }

    pub(super) fn for_handle_state(state: Option<ScriptHandleState>) -> Self {
        let Some(state) = state else {
            return Self::unknown();
        };

        let execution = match state.start_state {
            ScriptHandleStartState::Committed(ScriptStartCommitKind::ExecuteInline) => {
                ScriptHandleExecutionSubject::InlineClassicExecution
            }
            ScriptHandleStartState::Committed(ScriptStartCommitKind::ExecutePrepared) => {
                ScriptHandleExecutionSubject::PreparedExecution
            }
            ScriptHandleStartState::Committed(
                ScriptStartCommitKind::RegisterImportMap | ScriptStartCommitKind::RejectImportMap,
            ) => ScriptHandleExecutionSubject::NonExecutable,
            ScriptHandleStartState::Committed(ScriptStartCommitKind::Queue) => {
                ScriptHandleExecutionSubject::QueuedExecution
            }
            ScriptHandleStartState::Committed(ScriptStartCommitKind::QueueFailed) => {
                ScriptHandleExecutionSubject::FailedQueuedExecution
            }
            ScriptHandleStartState::Committed(ScriptStartCommitKind::Skip) => {
                ScriptHandleExecutionSubject::SkippedExecution
            }
            ScriptHandleStartState::Ready | ScriptHandleStartState::Preparing => {
                ScriptHandleExecutionSubject::PendingOrUnknown
            }
        };

        Self {
            source: state.source,
            execution,
        }
    }
}

impl ScriptEventPolicy {
    pub(super) fn default_dispatching() -> Self {
        Self {
            load: ScriptEventDispatchPolicy::Dispatch,
            error: ScriptEventDispatchPolicy::Dispatch,
        }
    }

    pub(super) fn for_subject(subject: ScriptHostEventSubject) -> Self {
        match (subject.source, subject.execution) {
            (_, ScriptHandleExecutionSubject::InlineClassicExecution) => Self {
                load: ScriptEventDispatchPolicy::Skip(ScriptEventSkipReason::InlineClassicLoad),
                error: ScriptEventDispatchPolicy::Dispatch,
            },
            _ => Self::default_dispatching(),
        }
    }

    pub(super) fn for_script(
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
        subject: ScriptHostEventSubject,
    ) -> Self {
        if source_kind == ScriptSourceKind::Inline {
            let load_skip_reason = match kind {
                ScriptKind::Classic => Some(ScriptEventSkipReason::InlineClassicLoad),
                ScriptKind::Module => Some(ScriptEventSkipReason::InlineModuleLoad),
                ScriptKind::ImportMap | ScriptKind::DataBlock => None,
            };
            if let Some(reason) = load_skip_reason {
                return Self {
                    load: ScriptEventDispatchPolicy::Skip(reason),
                    error: ScriptEventDispatchPolicy::Dispatch,
                };
            }
        }
        Self::for_subject(subject)
    }

    pub(super) fn dispatch_policy(self, kind: ScriptEventKind) -> ScriptEventDispatchPolicy {
        match kind {
            ScriptEventKind::Load => self.load,
            ScriptEventKind::Error => self.error,
        }
    }

    pub(super) fn task_dispatch_policy(self, kind: ScriptEventKind) -> ScriptEventDispatchPolicy {
        self.dispatch_policy(kind)
    }
}

impl ScriptFailurePageTaskPolicy {
    pub(super) fn classify_module_failure(message: &str) -> ModuleFailurePolicy {
        if message.starts_with("failed to fetch script `")
            || message.starts_with("script request `")
        {
            return ModuleFailurePolicy::TopLevelLoadFailure;
        }

        ModuleFailurePolicy::GraphFailure
    }

    pub(super) fn for_script(
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
        subject: ScriptHostEventSubject,
        message: &str,
        module_failure_policy: Option<ModuleFailurePolicy>,
    ) -> Self {
        let event_policy = ScriptEventPolicy::for_script(kind, source_kind, subject);
        if kind == ScriptKind::Module {
            let module_failure_policy =
                module_failure_policy.unwrap_or_else(|| Self::classify_module_failure(message));
            return match module_failure_policy {
                ModuleFailurePolicy::TopLevelLoadFailure
                | ModuleFailurePolicy::ModuleTreeLoadFailure => Self {
                    load_event: event_policy.dispatch_policy(ScriptEventKind::Load),
                    error_event: event_policy.dispatch_policy(ScriptEventKind::Error),
                    report_window_failure: false,
                    load_event_after_window_failure: false,
                },
                // A script with a fetched module graph still completes its
                // external-script load step when running that graph rejects.
                ModuleFailurePolicy::GraphFailure | ModuleFailurePolicy::EvaluationFailure => {
                    Self {
                        load_event: event_policy.dispatch_policy(ScriptEventKind::Load),
                        error_event: ScriptEventDispatchPolicy::Skip(
                            ScriptEventSkipReason::ModuleGraphFailure,
                        ),
                        report_window_failure: true,
                        load_event_after_window_failure: true,
                    }
                }
            };
        }
        Self {
            load_event: event_policy.dispatch_policy(ScriptEventKind::Load),
            error_event: if kind == ScriptKind::ImportMap && source_kind == ScriptSourceKind::Inline
            {
                ScriptEventDispatchPolicy::Skip(ScriptEventSkipReason::InlineImportMapError)
            } else {
                event_policy.dispatch_policy(ScriptEventKind::Error)
            },
            report_window_failure: kind == ScriptKind::Module
                || (kind == ScriptKind::ImportMap && source_kind == ScriptSourceKind::Inline),
            load_event_after_window_failure: false,
        }
    }
}
