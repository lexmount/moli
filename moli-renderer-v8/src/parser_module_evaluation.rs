use crate::document_script_scheduler::{
    DocumentScriptReadyActionDispatchRoute, DocumentScriptReadyActionRoute,
};
use crate::document_task_lane::DocumentTaskQueue;
use crate::module_runtime::ModuleEntryId;
use crate::module_script_continuation::ModuleScriptEvaluationReactionState;
use crate::types::ScriptErrorValue;

#[derive(Debug)]
pub(crate) struct ParserModuleEvaluationContinuation<Work> {
    work: Work,
    root_entry: ModuleEntryId,
    reaction_id: u64,
    reaction_state: ModuleScriptEvaluationReactionState,
}

impl<Work> ParserModuleEvaluationContinuation<Work> {
    #[cfg(test)]
    pub(crate) fn pending_for_test(
        work: Work,
        root_entry: ModuleEntryId,
        reaction_id: u64,
    ) -> Self {
        Self {
            work,
            root_entry,
            reaction_id,
            reaction_state: ModuleScriptEvaluationReactionState::Pending,
        }
    }

    pub(crate) fn work(&self) -> &Work {
        &self.work
    }

    #[cfg(test)]
    pub(crate) fn root_entry(&self) -> ModuleEntryId {
        self.root_entry
    }

    #[cfg(test)]
    pub(crate) fn reaction_state(&self) -> &ModuleScriptEvaluationReactionState {
        &self.reaction_state
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Work,
        ModuleEntryId,
        u64,
        ModuleScriptEvaluationReactionState,
    ) {
        (
            self.work,
            self.root_entry,
            self.reaction_id,
            self.reaction_state,
        )
    }

    fn is_ready(&self) -> bool {
        !matches!(
            self.reaction_state,
            ModuleScriptEvaluationReactionState::Pending
        )
    }
}

impl<Owner, Work> DocumentScriptReadyActionRoute<Owner> for ParserModuleEvaluationContinuation<Work>
where
    Work: DocumentScriptReadyActionRoute<Owner>,
{
    fn payload_document_owner(&self) -> Owner {
        self.work.payload_document_owner()
    }
}

impl<Route, Work> DocumentScriptReadyActionDispatchRoute<Route>
    for ParserModuleEvaluationContinuation<Work>
where
    Work: DocumentScriptReadyActionDispatchRoute<Route>,
{
    fn dispatch_route(&self) -> Route {
        self.work.dispatch_route()
    }
}

#[derive(Debug)]
pub(crate) struct ParserModuleEvaluationStore<Work> {
    next_reaction_id: u64,
    evaluations: DocumentTaskQueue<ParserModuleEvaluationContinuation<Work>>,
}

impl<Work> Default for ParserModuleEvaluationStore<Work> {
    fn default() -> Self {
        Self {
            next_reaction_id: 1,
            evaluations: DocumentTaskQueue::default(),
        }
    }
}

impl<Work> ParserModuleEvaluationStore<Work> {
    pub(crate) fn clear(&mut self) {
        self.next_reaction_id = 1;
        self.evaluations.clear();
    }

    pub(crate) fn reserve_pending(&mut self, work: Work, root_entry: ModuleEntryId) -> u64 {
        let reaction_id = self.next_reaction_id;
        self.next_reaction_id = self
            .next_reaction_id
            .checked_add(1)
            .expect("parser module reaction id space exhausted");
        self.push_pending_with_reaction_id(work, root_entry, reaction_id);
        reaction_id
    }

    pub(crate) fn push_pending_with_reaction_id(
        &mut self,
        work: Work,
        root_entry: ModuleEntryId,
        reaction_id: u64,
    ) {
        self.evaluations
            .push_back(ParserModuleEvaluationContinuation {
                work,
                root_entry,
                reaction_id,
                reaction_state: ModuleScriptEvaluationReactionState::Pending,
            });
    }

    pub(crate) fn remove(&mut self, reaction_id: u64) -> bool {
        self.evaluations
            .retain(|evaluation| evaluation.reaction_id != reaction_id)
    }

    pub(crate) fn mark_fulfilled(&mut self, reaction_id: u64) -> Option<ModuleEntryId> {
        let evaluation = self
            .evaluations
            .iter_mut()
            .find(|evaluation| evaluation.reaction_id == reaction_id)?;
        evaluation.reaction_state = ModuleScriptEvaluationReactionState::Fulfilled;
        Some(evaluation.root_entry)
    }

    pub(crate) fn mark_rejected(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_value: Option<ScriptErrorValue>,
    ) -> Option<ModuleEntryId> {
        let evaluation = self
            .evaluations
            .iter_mut()
            .find(|evaluation| evaluation.reaction_id == reaction_id)?;
        evaluation.reaction_state = ModuleScriptEvaluationReactionState::Rejected {
            reason,
            error_value,
        };
        Some(evaluation.root_entry)
    }

    pub(crate) fn take_ready(&mut self) -> Option<ParserModuleEvaluationContinuation<Work>> {
        let mut ready = None;
        let mut retained = DocumentTaskQueue::default();
        for evaluation in self.evaluations.drain_all() {
            if ready.is_none() && evaluation.is_ready() {
                ready = Some(evaluation);
            } else {
                retained.push_back(evaluation);
            }
        }
        self.evaluations = retained;
        ready
    }

    pub(crate) fn remove_for_owner<Owner>(&mut self, owner: Owner) -> bool
    where
        Owner: Copy + PartialEq,
        Work: DocumentScriptReadyActionRoute<Owner>,
    {
        self.evaluations
            .retain(|evaluation| evaluation.payload_document_owner() != owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RoutedWork {
        owner: u64,
    }

    impl DocumentScriptReadyActionRoute<u64> for RoutedWork {
        fn payload_document_owner(&self) -> u64 {
            self.owner
        }
    }

    impl DocumentScriptReadyActionDispatchRoute<u64> for RoutedWork {
        fn dispatch_route(&self) -> u64 {
            self.owner
        }
    }

    #[test]
    fn parser_module_evaluation_store_promotes_settled_reactions_in_order() {
        let mut store = ParserModuleEvaluationStore::default();
        let first = store.reserve_pending("first", ModuleEntryId::for_test(1));
        let second = store.reserve_pending("second", ModuleEntryId::for_test(2));

        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert!(store.take_ready().is_none());

        assert_eq!(
            store.mark_fulfilled(second),
            Some(ModuleEntryId::for_test(2))
        );
        let ready = store
            .take_ready()
            .expect("settled parser-module evaluation should become ready");
        assert_eq!(*ready.work(), "second");
        assert_eq!(ready.root_entry(), ModuleEntryId::for_test(2));
        assert_eq!(ready.reaction_id, second);
        assert!(matches!(
            ready.reaction_state(),
            ModuleScriptEvaluationReactionState::Fulfilled
        ));
        assert!(store.take_ready().is_none());

        assert_eq!(
            store.mark_rejected(first, "boom".to_owned(), None),
            Some(ModuleEntryId::for_test(1))
        );
        let ready = store
            .take_ready()
            .expect("rejected parser-module evaluation should become ready");
        assert_eq!(*ready.work(), "first");
        assert_eq!(ready.root_entry(), ModuleEntryId::for_test(1));
        assert!(matches!(
            ready.reaction_state(),
            ModuleScriptEvaluationReactionState::Rejected { .. }
        ));
        assert!(store.take_ready().is_none());
    }

    #[test]
    fn parser_module_evaluation_continuation_routes_payload_owner() {
        let mut store = ParserModuleEvaluationStore::default();
        let reaction_id =
            store.reserve_pending(RoutedWork { owner: 42 }, ModuleEntryId::for_test(1));
        assert_eq!(
            store.mark_fulfilled(reaction_id),
            Some(ModuleEntryId::for_test(1))
        );
        let ready = store
            .take_ready()
            .expect("settled parser-module evaluation should become ready");
        assert_eq!(ready.payload_document_owner(), 42);
        assert_eq!(ready.dispatch_route(), 42);
    }
}
