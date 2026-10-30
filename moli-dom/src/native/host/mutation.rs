use super::*;

mod attributes;
mod effects;
mod owner_lifecycle;
mod state;
mod tree;

pub use effects::{
    DomAttributeMutation, DomAttributeMutationOutcome, DomChildListMutation, DomMutationEffects,
    DomMutationObserverEffect, DomMutationRecord, DomMutationRecordBatch, DomMutationRecordKind,
    DomScriptMutationEffects, DomSlotAssignmentChange, DomSlotMutationEffects,
    DomStyleInvalidationInputs, DomStylesheetOwnerChange, DomStylesheetOwnerChangeKind,
    DomStylesheetOwnerTransitions, DomStylesheetOwnerTreeScopes, DomTreeMutationEffects,
    ScriptPrepareTrigger, ScriptPrepareTriggerKind,
};

impl Deref for DomHost {
    type Target = NativeDom;

    fn deref(&self) -> &Self::Target {
        &self.dom
    }
}
