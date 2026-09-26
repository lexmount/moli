use super::*;

mod attributes;
mod effects;
mod owner_lifecycle;
mod state;
mod text_control;
mod tree;

pub use effects::{
    DomAttributeMutation, DomAttributeMutationOutcome, DomChildListMutation, DomMutationEffects,
    DomMutationRecord, DomMutationRecordBatch, DomMutationRecordKind, DomScriptMutationEffects,
    DomSlotAssignmentChange, DomSlotMutationEffects, DomStyleInvalidationInputs,
    DomStylesheetOwnerChange, DomStylesheetOwnerChangeKind, DomStylesheetOwnerTransitions,
    DomStylesheetOwnerTreeScopes, DomTextareaValueChange, DomTreeMutationEffects,
    ScriptPrepareTrigger, ScriptPrepareTriggerKind,
};

impl Deref for DomHost {
    type Target = NativeDom;

    fn deref(&self) -> &Self::Target {
        &self.dom
    }
}
