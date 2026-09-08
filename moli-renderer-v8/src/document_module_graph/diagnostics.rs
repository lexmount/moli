use crate::types::{ScriptErrorConstructorKind, ScriptErrorValue};
use moli_module_script_tree::ModuleExceptionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModuleLoadStage {
    Fetch,
    Compile,
    Resolve,
    Instantiate,
    Evaluate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleLoadError {
    stage: ModuleLoadStage,
    message: String,
    error_constructor: Option<ScriptErrorConstructorKind>,
    exception_id: Option<ModuleExceptionId>,
    top_level_module_load_failure: bool,
}

impl ModuleLoadError {
    pub(crate) fn new(stage: ModuleLoadStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            error_constructor: None,
            exception_id: None,
            top_level_module_load_failure: false,
        }
    }

    pub(crate) fn with_error_constructor(
        mut self,
        error_constructor: ScriptErrorConstructorKind,
    ) -> Self {
        self.error_constructor = Some(error_constructor);
        self
    }

    pub(crate) fn with_top_level_module_load_failure(mut self) -> Self {
        self.top_level_module_load_failure = true;
        self
    }

    pub(crate) fn with_exception_id(mut self, exception_id: ModuleExceptionId) -> Self {
        self.exception_id = Some(exception_id);
        self
    }

    pub(crate) fn exception_id(&self) -> Option<ModuleExceptionId> {
        self.exception_id
    }

    pub(crate) fn stage(&self) -> ModuleLoadStage {
        self.stage
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn with_message(mut self, message: String) -> Self {
        self.message = message;
        self
    }

    pub(crate) fn error_value(&self) -> Option<ScriptErrorValue> {
        self.exception_id
            .map(ScriptErrorValue::Retained)
            .or_else(|| self.error_constructor.map(ScriptErrorValue::Constructor))
    }

    pub(crate) fn error_constructor(&self) -> Option<ScriptErrorConstructorKind> {
        self.error_constructor
    }

    pub(crate) fn is_top_level_module_load_failure(&self) -> bool {
        self.top_level_module_load_failure
    }
}
