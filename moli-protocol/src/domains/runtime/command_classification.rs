use crate::domains::actions::RuntimeAction;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MainRuntimeInspectorCommand {
    action: RuntimeAction,
}

impl MainRuntimeInspectorCommand {
    const fn new(action: RuntimeAction) -> Self {
        Self { action }
    }

    pub(super) const fn action(self) -> RuntimeAction {
        self.action
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimeBindingCommand {
    Add,
    Remove,
}

impl RuntimeBindingCommand {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Add => "addBinding",
            Self::Remove => "removeBinding",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimeDevToolsScriptCommand {
    Evaluate,
    CallFunctionOn,
}

impl RuntimeDevToolsScriptCommand {
    pub(super) const fn action(self) -> RuntimeAction {
        match self {
            Self::Evaluate => RuntimeAction::Evaluate,
            Self::CallFunctionOn => RuntimeAction::CallFunctionOn,
        }
    }
}

/// The unique main-Page dispatch family for a parsed Runtime action.
///
/// The exhaustive match in `classify` is the only action inventory for this
/// layer. Adding a `RuntimeAction` therefore cannot silently fall through to a
/// generic Inspector route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MainRuntimeCommand {
    Enable,
    Disable,
    Binding(RuntimeBindingCommand),
    DiscardConsoleEntries,
    RunIfWaitingForDebugger,
    DevToolsScript(RuntimeDevToolsScriptCommand),
    Inspector(MainRuntimeInspectorCommand),
}

impl MainRuntimeCommand {
    pub(super) const fn classify(action: RuntimeAction) -> Self {
        match action {
            RuntimeAction::Enable => Self::Enable,
            RuntimeAction::Disable => Self::Disable,
            RuntimeAction::AddBinding => Self::Binding(RuntimeBindingCommand::Add),
            RuntimeAction::RemoveBinding => Self::Binding(RuntimeBindingCommand::Remove),
            RuntimeAction::DiscardConsoleEntries => Self::DiscardConsoleEntries,
            RuntimeAction::RunIfWaitingForDebugger => Self::RunIfWaitingForDebugger,
            RuntimeAction::Evaluate => Self::DevToolsScript(RuntimeDevToolsScriptCommand::Evaluate),
            RuntimeAction::CallFunctionOn => {
                Self::DevToolsScript(RuntimeDevToolsScriptCommand::CallFunctionOn)
            }
            RuntimeAction::TerminateExecution => {
                Self::Inspector(MainRuntimeInspectorCommand::new(action))
            }
            RuntimeAction::GetProperties
            | RuntimeAction::AwaitPromise
            | RuntimeAction::GetExceptionDetails
            | RuntimeAction::ReleaseObject => {
                Self::Inspector(MainRuntimeInspectorCommand::new(action))
            }
            RuntimeAction::QueryObjects => {
                Self::Inspector(MainRuntimeInspectorCommand::new(action))
            }
            RuntimeAction::CompileScript
            | RuntimeAction::RunScript
            | RuntimeAction::GlobalLexicalScopeNames
            | RuntimeAction::GetIsolateId
            | RuntimeAction::GetHeapUsage
            | RuntimeAction::ReleaseObjectGroup
            | RuntimeAction::SetAsyncCallStackDepth
            | RuntimeAction::SetCustomObjectFormatterEnabled
            | RuntimeAction::SetMaxCallStackSizeToCapture => {
                Self::Inspector(MainRuntimeInspectorCommand::new(action))
            }
        }
    }

    pub(super) const fn requires_v8_method_support_check(self) -> bool {
        matches!(self, Self::DevToolsScript(_) | Self::Inspector(_))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkerRuntimeCommandKind {
    Enable,
    Disable,
    DiscardConsoleEntries,
    RunIfWaitingForDebugger,
    Inspector,
}

/// A Runtime command prepared for either SharedWorker/DedicatedWorker or
/// ServiceWorker dispatch.
///
/// Worker lifetime and error handling remain target-specific. Only the pure
/// action and payload classification is shared here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WorkerRuntimeCommand {
    action: RuntimeAction,
    kind: WorkerRuntimeCommandKind,
    binding: Option<RuntimeBindingCommand>,
}

impl WorkerRuntimeCommand {
    pub(super) const fn classify(action: RuntimeAction) -> Self {
        use WorkerRuntimeCommandKind::{
            Disable, DiscardConsoleEntries, Enable, Inspector, RunIfWaitingForDebugger,
        };

        let (kind, binding) = match action {
            RuntimeAction::Enable => (Enable, None),
            RuntimeAction::Disable => (Disable, None),
            RuntimeAction::DiscardConsoleEntries => (DiscardConsoleEntries, None),
            RuntimeAction::RunIfWaitingForDebugger => (RunIfWaitingForDebugger, None),
            RuntimeAction::AddBinding => (Inspector, Some(RuntimeBindingCommand::Add)),
            RuntimeAction::RemoveBinding => (Inspector, Some(RuntimeBindingCommand::Remove)),
            RuntimeAction::CallFunctionOn => (Inspector, None),
            RuntimeAction::GetProperties
            | RuntimeAction::AwaitPromise
            | RuntimeAction::GetExceptionDetails
            | RuntimeAction::ReleaseObject => (Inspector, None),
            RuntimeAction::QueryObjects => (Inspector, None),
            RuntimeAction::CompileScript
            | RuntimeAction::TerminateExecution
            | RuntimeAction::RunScript
            | RuntimeAction::Evaluate
            | RuntimeAction::GlobalLexicalScopeNames
            | RuntimeAction::GetIsolateId
            | RuntimeAction::GetHeapUsage
            | RuntimeAction::ReleaseObjectGroup
            | RuntimeAction::SetAsyncCallStackDepth
            | RuntimeAction::SetCustomObjectFormatterEnabled
            | RuntimeAction::SetMaxCallStackSizeToCapture => (Inspector, None),
        };
        Self {
            action,
            kind,
            binding,
        }
    }

    pub(super) const fn action(self) -> RuntimeAction {
        self.action
    }

    pub(super) const fn kind(self) -> WorkerRuntimeCommandKind {
        self.kind
    }

    pub(super) const fn binding(self) -> Option<RuntimeBindingCommand> {
        self.binding
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_binding_and_devtools_commands_cannot_enter_generic_inspector_dispatch() {
        assert_eq!(
            MainRuntimeCommand::classify(RuntimeAction::AddBinding),
            MainRuntimeCommand::Binding(RuntimeBindingCommand::Add)
        );
        assert_eq!(
            MainRuntimeCommand::classify(RuntimeAction::RemoveBinding),
            MainRuntimeCommand::Binding(RuntimeBindingCommand::Remove)
        );
        assert_eq!(
            MainRuntimeCommand::classify(RuntimeAction::Evaluate),
            MainRuntimeCommand::DevToolsScript(RuntimeDevToolsScriptCommand::Evaluate)
        );
        assert_eq!(
            MainRuntimeCommand::classify(RuntimeAction::CallFunctionOn),
            MainRuntimeCommand::DevToolsScript(RuntimeDevToolsScriptCommand::CallFunctionOn)
        );
    }

    #[test]
    fn main_object_commands_route_to_the_native_inspector() {
        let MainRuntimeCommand::Inspector(terminate) =
            MainRuntimeCommand::classify(RuntimeAction::TerminateExecution)
        else {
            panic!("terminateExecution must be an Inspector command");
        };
        assert_eq!(terminate.action(), RuntimeAction::TerminateExecution);

        let MainRuntimeCommand::Inspector(get_properties) =
            MainRuntimeCommand::classify(RuntimeAction::GetProperties)
        else {
            panic!("getProperties must be an Inspector command");
        };
        assert_eq!(get_properties.action(), RuntimeAction::GetProperties);
    }

    #[test]
    fn worker_call_function_and_binding_keep_distinct_dispatch_semantics() {
        let call_function = WorkerRuntimeCommand::classify(RuntimeAction::CallFunctionOn);
        assert_eq!(call_function.kind(), WorkerRuntimeCommandKind::Inspector);

        let add_binding = WorkerRuntimeCommand::classify(RuntimeAction::AddBinding);
        assert_eq!(add_binding.kind(), WorkerRuntimeCommandKind::Inspector);

        assert_eq!(add_binding.binding(), Some(RuntimeBindingCommand::Add));
    }
}
