use crate::document_runtime::DomHandle;

#[derive(Debug, Clone, Default)]
pub(in crate::native_bridge::context_host) struct ChildClassicScriptDocumentState {
    // Capture currentScript at execution entry. Adoption or a move into a
    // shadow tree during execution must not change the active value.
    current_script_stack: Vec<Option<DomHandle>>,
    // Counts nested synchronous parser-blocking execution scopes for this
    // exact Document, including load/error completion dispatched before parser
    // resume. Parser-inserted ownership, deferred-script execution, DOM
    // connectedness, parser activity, currentScript, and document.write
    // insertion-point availability are separate state.
    parser_script_nesting_level: usize,
}

impl ChildClassicScriptDocumentState {
    pub(super) fn clear_current_script_stack(&mut self) {
        self.current_script_stack.clear();
    }

    pub(super) fn clear(&mut self) {
        self.clear_current_script_stack();
        self.parser_script_nesting_level = 0;
    }

    pub(super) fn enter_parser_script_nesting(&mut self) {
        self.parser_script_nesting_level = self
            .parser_script_nesting_level
            .checked_add(1)
            .expect("child parser script nesting level overflow");
    }

    pub(super) fn exit_parser_script_nesting(&mut self) {
        assert!(
            self.parser_script_nesting_level > 0,
            "child parser script nesting scope exited without matching enter"
        );
        self.parser_script_nesting_level -= 1;
    }

    pub(super) fn is_executing_parser_script(&self) -> bool {
        self.parser_script_nesting_level > 0
    }

    pub(super) fn push_current_script(&mut self, script_handle: Option<DomHandle>) {
        self.current_script_stack.push(script_handle);
    }

    pub(super) fn pop_current_script(&mut self, script_handle: Option<DomHandle>) {
        if self.current_script_stack.last().copied() == Some(script_handle) {
            self.current_script_stack.pop();
            return;
        }
        tracing::warn!(
            ?script_handle,
            current = ?self.current_script_stack.last(),
            "child currentScript stack pop did not match the active script"
        );
    }

    pub(super) fn current_script(&self) -> Option<DomHandle> {
        self.current_script_stack.last().copied().flatten()
    }
}
