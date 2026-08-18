use std::collections::BTreeMap;

#[cfg(test)]
use crate::document_script_scheduler::DocumentModuleGraphReadyWork;
use crate::document_script_scheduler::{
    DocumentOwnedScriptReadyAction, DocumentScriptReadyActionDispatchRoute,
    DocumentScriptReadyActionRoute, DocumentScriptReadyDispatch,
    DocumentScriptReadyDispatchOwnerMismatch, DocumentScriptReadyWork, DocumentScriptScheduler,
    FrameDocumentModuleGraphReadyTarget, ModuleScriptGraphReadyWork,
    ParserDeferredClassicSourceLoadCompletion, ParserDeferredModuleGraphStart,
    ParserDeferredScriptReady, ParserDeferredScriptStartAction, ParserPendingScriptId,
    ParserPendingScriptRoute,
};
use crate::document_task_lane::DocumentTaskQueue;
use crate::module_runtime::ModuleEntryId;
use crate::parser_module_evaluation::{
    ParserModuleEvaluationContinuation, ParserModuleEvaluationStore,
};
use crate::parser_script::action::ParserClassicScriptNextOwnerAction;
use crate::planning::{PreparedScript, SharedScriptSourceLoad};
use crate::stylesheet_blocking::DocumentBlockingStylesheetSignature;
use crate::types::ScriptErrorConstructorKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParserModuleEvaluationReactionUpdate {
    root_entry: ModuleEntryId,
    queued_ready_action_count: usize,
}

impl ParserModuleEvaluationReactionUpdate {
    pub(crate) const fn root_entry(self) -> ModuleEntryId {
        self.root_entry
    }

    pub(crate) const fn queued_ready_action_count(self) -> usize {
        self.queued_ready_action_count
    }
}

#[derive(Debug)]
pub(crate) struct DocumentScriptSchedulerStore<
    Owner,
    Target = FrameDocumentModuleGraphReadyTarget,
    ParserModuleEvaluation = std::convert::Infallible,
    ParserModuleGraphFailure = std::convert::Infallible,
    PendingParserModuleEvaluation = std::convert::Infallible,
    ParserClassicReady = std::convert::Infallible,
    ParserClassicSourceFailure = std::convert::Infallible,
> {
    documents: BTreeMap<
        Owner,
        DocumentScriptScheduler<
            Target,
            ParserModuleEvaluation,
            ParserModuleGraphFailure,
            ParserClassicReady,
            ParserClassicSourceFailure,
        >,
    >,
    parser_module_evaluations: ParserModuleEvaluationStore<PendingParserModuleEvaluation>,
    // Queue owners with document-script ready work. Keep the store boundary
    // ready-work-shaped so later script lanes can join without another owner
    // queue.
    ready_work_owners: DocumentTaskQueue<Owner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModuleScriptWatchResult<Owner> {
    pending_script_id: ParserPendingScriptId<Owner>,
    watched: bool,
    queued_ready_work: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParserDeferredClassicSourceLoadApplyResult {
    Applied,
    MissingDocument,
    MissingPendingScript,
}

impl<Owner: Copy> ModuleScriptWatchResult<Owner> {
    fn new(
        pending_script_id: ParserPendingScriptId<Owner>,
        watched: bool,
        queued_ready_work: bool,
    ) -> Self {
        Self {
            pending_script_id,
            watched,
            queued_ready_work,
        }
    }

    pub(crate) fn pending_script_id(self) -> ParserPendingScriptId<Owner> {
        self.pending_script_id
    }

    pub(crate) fn watched(self) -> bool {
        self.watched
    }

    pub(crate) fn queued_ready_work(self) -> bool {
        self.queued_ready_work
    }
}

impl<
    Owner,
    Target,
    ParserModuleEvaluation,
    ParserModuleGraphFailure,
    PendingParserModuleEvaluation,
    ParserClassicReady,
    ParserClassicSourceFailure,
> Default
    for DocumentScriptSchedulerStore<
        Owner,
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        PendingParserModuleEvaluation,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >
{
    fn default() -> Self {
        Self {
            documents: BTreeMap::new(),
            parser_module_evaluations: ParserModuleEvaluationStore::default(),
            ready_work_owners: DocumentTaskQueue::default(),
        }
    }
}

impl<
    Owner,
    Target,
    ParserModuleEvaluation,
    ParserModuleGraphFailure,
    PendingParserModuleEvaluation,
    ParserClassicReady,
    ParserClassicSourceFailure,
>
    DocumentScriptSchedulerStore<
        Owner,
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        PendingParserModuleEvaluation,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >
where
    Owner: Copy + Ord,
{
    pub(crate) fn clear(&mut self) {
        self.documents.clear();
        self.parser_module_evaluations.clear();
        self.ready_work_owners = DocumentTaskQueue::default();
    }

    pub(crate) fn remove_document(&mut self, owner: Owner)
    where
        PendingParserModuleEvaluation: DocumentScriptReadyActionRoute<Owner>,
    {
        self.documents.remove(&owner);
        self.parser_module_evaluations.remove_for_owner(owner);
        self.ready_work_owners.retain(|queued| *queued != owner);
    }

    pub(crate) fn register_module_script(
        &mut self,
        owner: Owner,
        script: &PreparedScript,
    ) -> ParserPendingScriptId<Owner> {
        let key = self
            .documents
            .entry(owner)
            .or_default()
            .register_module_script(script);
        ParserPendingScriptId::from_key(owner, key)
    }

    pub(crate) fn claim_parser_deferred_script(
        &mut self,
        owner: Owner,
        script: PreparedScript,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        blocking_signatures_before: std::collections::HashSet<DocumentBlockingStylesheetSignature>,
        load_delay_token: crate::frame_owner_model::DocumentLoadDelayTokenId,
    ) -> Option<ParserDeferredScriptStartAction<Owner>> {
        let module_script =
            (script.kind == crate::types::ScriptKind::Module).then(|| script.clone());
        let claim = self
            .documents
            .entry(owner)
            .or_default()
            .claim_parser_non_async_post_parse_script_with_shared_load_and_document_character_set(
                script,
                shared_load,
                document_character_set,
                blocking_signatures_before,
                load_delay_token,
            )?;
        let pending_script_id = ParserPendingScriptId::from_key(owner, claim.key());
        if let Some(script) = module_script {
            return Some(ParserDeferredScriptStartAction::ModuleGraph(
                ParserDeferredModuleGraphStart::new(pending_script_id, script),
            ));
        }
        Some(match claim.into_classic_source_load() {
            Some(source_load) => {
                ParserDeferredScriptStartAction::ClassicSource(source_load.bind_owner(owner))
            }
            None => ParserDeferredScriptStartAction::NoFetch,
        })
    }

    #[cfg(test)]
    pub(crate) fn claim_ready_parser_deferred_script_for_test(
        &mut self,
        owner: Owner,
        script: PreparedScript,
        blocking_signatures_before: std::collections::HashSet<DocumentBlockingStylesheetSignature>,
    ) {
        let load_delay_token =
            crate::frame_owner_model::DocumentLoadDelayTokenId(script.position as u64 + 1);
        let shared_load = (script.kind == crate::types::ScriptKind::Classic
            && script.source_kind == crate::types::ScriptSourceKind::External
            && matches!(&script.source, crate::planning::ScriptSource::External))
        .then(|| SharedScriptSourceLoad::ready_err("synthetic parser-deferred source miss"));
        let start_action = self.claim_parser_deferred_script(
            owner,
            script,
            shared_load,
            None,
            blocking_signatures_before,
            load_delay_token,
        );
        match start_action {
            Some(ParserDeferredScriptStartAction::ClassicSource(source_load_request)) => {
                let source_load = source_load_request.start_with_injected_source_load_for_test();
                let (pending_script_id, source_load) = source_load.into_parts();
                let outcome = source_load
                    .try_outcome()
                    .expect("test parser-deferred source terminal should be ready");
                assert_eq!(
                    self.complete_parser_deferred_classic_source_load(
                        ParserDeferredClassicSourceLoadCompletion::new(pending_script_id, outcome),
                    ),
                    ParserDeferredClassicSourceLoadApplyResult::Applied
                );
            }
            Some(
                ParserDeferredScriptStartAction::NoFetch
                | ParserDeferredScriptStartAction::ModuleGraph(_),
            ) => {}
            None => panic!("test parser-deferred script should be accepted"),
        }
    }

    pub(crate) fn seal_parser_deferred_scripts(
        &mut self,
        owner: Owner,
    ) -> Result<usize, crate::document_script_scheduler::ParserPendingScriptKey> {
        let Some(scheduler) = self.documents.get_mut(&owner) else {
            return Ok(0);
        };
        scheduler.seal_parser_deferred_scripts()
    }

    pub(crate) fn complete_parser_deferred_classic_source_load(
        &mut self,
        completion: ParserDeferredClassicSourceLoadCompletion<Owner>,
    ) -> ParserDeferredClassicSourceLoadApplyResult {
        let (pending_script_id, outcome) = completion.into_parts();
        let Some(scheduler) = self.documents.get_mut(&pending_script_id.owner()) else {
            return ParserDeferredClassicSourceLoadApplyResult::MissingDocument;
        };
        if scheduler.complete_parser_deferred_classic_source_load(pending_script_id.key(), outcome)
        {
            ParserDeferredClassicSourceLoadApplyResult::Applied
        } else {
            ParserDeferredClassicSourceLoadApplyResult::MissingPendingScript
        }
    }

    pub(crate) fn cancel_parser_deferred_script(
        &mut self,
        pending_script_id: ParserPendingScriptId<Owner>,
    ) -> Option<crate::frame_owner_model::DocumentLoadDelayTokenId> {
        self.documents
            .get_mut(&pending_script_id.owner())?
            .cancel_parser_deferred_script(pending_script_id.key())
    }

    pub(crate) fn has_after_parsing_script(&self, owner: Owner) -> bool {
        self.documents
            .get(&owner)
            .is_some_and(DocumentScriptScheduler::has_after_parsing_script)
    }

    pub(crate) fn next_after_parsing_blocking_signatures(
        &self,
        owner: Owner,
    ) -> Option<&std::collections::HashSet<crate::DocumentBlockingStylesheetSignature>> {
        self.documents
            .get(&owner)?
            .next_after_parsing_blocking_signatures()
    }

    pub(crate) fn next_after_parsing_script_is_ready(&self, owner: Owner) -> bool {
        self.documents
            .get(&owner)
            .is_some_and(DocumentScriptScheduler::next_after_parsing_script_is_ready)
    }

    pub(crate) fn take_next_after_parsing_ready_script(
        &mut self,
        owner: Owner,
    ) -> Option<ParserDeferredScriptReady<Target, ParserModuleGraphFailure>> {
        self.documents
            .get_mut(&owner)?
            .take_next_after_parsing_ready_script()
    }

    pub(crate) fn watch_module_script(
        &mut self,
        pending_script_id: ParserPendingScriptId<Owner>,
    ) -> ModuleScriptWatchResult<Owner> {
        let owner = pending_script_id.owner();
        let ready_count_before = self.pending_ready_work_count(owner);
        let watched = self
            .documents
            .get_mut(&owner)
            .map(|scheduler| scheduler.watch_module_script(pending_script_id.key()))
            .unwrap_or(false);
        let queued_ready_work =
            self.queue_ready_owner_if_ready_count_increased(owner, ready_count_before);
        ModuleScriptWatchResult::new(pending_script_id, watched, queued_ready_work)
    }

    pub(crate) fn register_and_watch_module_script(
        &mut self,
        owner: Owner,
        script: &PreparedScript,
    ) -> ModuleScriptWatchResult<Owner> {
        let pending_script_id = self.register_module_script(owner, script);
        self.watch_module_script(pending_script_id)
    }

    pub(crate) fn accept_parser_ordered_module_script(
        &mut self,
        owner: Owner,
        script: &PreparedScript,
        blocking_stylesheet_signatures: std::collections::HashSet<
            crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
        >,
    ) -> Option<ParserPendingScriptId<Owner>> {
        let expected_id = ParserPendingScriptId::from_key(
            owner,
            crate::document_script_scheduler::ParserPendingScriptKey::from_script(script),
        );
        let key = self
            .documents
            .entry(owner)
            .or_default()
            .accept_parser_ordered_module_script(script, blocking_stylesheet_signatures)?;
        let pending_script_id = ParserPendingScriptId::from_key(owner, key);
        if pending_script_id != expected_id {
            return None;
        }
        Some(pending_script_id)
    }

    pub(crate) fn pending_script_id_for_script(
        &self,
        script: &PreparedScript,
    ) -> Option<ParserPendingScriptId<Owner>> {
        let key = crate::document_script_scheduler::ParserPendingScriptKey::from_script(script);
        let mut matching_owners = self
            .documents
            .iter()
            .filter_map(|(owner, scheduler)| scheduler.has_module_script(key).then_some(*owner));
        let owner = matching_owners.next()?;
        if matching_owners.next().is_some() {
            return None;
        }
        Some(ParserPendingScriptId::from_key(owner, key))
    }

    #[cfg(test)]
    /// Test-only projection of the parser module watch authority.
    ///
    /// The state itself lives only in `ParserModuleScriptRunner`; the store
    /// does not retain a second readiness bit.
    pub(crate) fn module_script_is_watching_for_test(
        &self,
        pending_script_id: ParserPendingScriptId<Owner>,
    ) -> bool {
        self.documents
            .get(&pending_script_id.owner())
            .is_some_and(|scheduler| {
                scheduler.module_script_is_watching_for_test(pending_script_id.key())
            })
    }

    pub(crate) fn has_module_script(
        &self,
        pending_script_id: ParserPendingScriptId<Owner>,
    ) -> bool {
        self.documents
            .get(&pending_script_id.owner())
            .is_some_and(|scheduler| scheduler.has_module_script(pending_script_id.key()))
    }

    pub(crate) fn discard_module_script(
        &mut self,
        pending_script_id: ParserPendingScriptId<Owner>,
    ) -> bool {
        self.documents
            .get_mut(&pending_script_id.owner())
            .is_some_and(|scheduler| scheduler.discard_module_script(pending_script_id.key()))
    }

    #[cfg(test)]
    pub(crate) fn parser_ordered_module_terminal_is_ready(
        &self,
        pending_script_id: ParserPendingScriptId<Owner>,
    ) -> bool {
        self.documents
            .get(&pending_script_id.owner())
            .is_some_and(|scheduler| {
                scheduler.parser_ordered_module_terminal_is_ready(pending_script_id.key())
            })
    }

    pub(crate) fn prepare_parser_ordered_module_terminal(
        &mut self,
        pending_script_id: ParserPendingScriptId<Owner>,
    ) -> crate::document_script_scheduler::ParserOrderedModuleTerminalState {
        self.documents
            .get_mut(&pending_script_id.owner())
            .map(|scheduler| {
                scheduler.prepare_parser_ordered_module_terminal(pending_script_id.key())
            })
            .unwrap_or(crate::document_script_scheduler::ParserOrderedModuleTerminalState::Missing)
    }

    pub(crate) fn parser_ordered_module_blocking_stylesheet_signatures(
        &self,
        pending_script_id: ParserPendingScriptId<Owner>,
    ) -> Option<
        &std::collections::HashSet<crate::stylesheet_blocking::DocumentBlockingStylesheetSignature>,
    > {
        self.documents
            .get(&pending_script_id.owner())?
            .module_script_blocking_stylesheet_signatures(pending_script_id.key())
    }

    pub(crate) fn promote_parser_ordered_module_terminal(
        &mut self,
        pending_script_id: ParserPendingScriptId<Owner>,
    ) -> bool {
        let owner = pending_script_id.owner();
        let ready_count_before = self.pending_ready_work_count(owner);
        let promoted = self.documents.get_mut(&owner).is_some_and(|scheduler| {
            scheduler.promote_parser_ordered_module_terminal(pending_script_id.key())
        });
        promoted && self.queue_ready_owner_if_ready_count_increased(owner, ready_count_before)
    }

    pub(crate) fn notify_module_script_graph_ready_work(
        &mut self,
        work: ModuleScriptGraphReadyWork<Target>,
    ) -> bool
    where
        ModuleScriptGraphReadyWork<Target>:
            DocumentScriptReadyActionRoute<Owner> + ParserPendingScriptRoute<Owner>,
    {
        let pending_script_id = work.parser_pending_script_id();
        let owner = pending_script_id.owner();
        debug_assert!(owner == work.payload_document_owner());
        let ready_count_before = self.pending_ready_work_count(owner);
        let Some(scheduler) = self.documents.get_mut(&owner) else {
            return false;
        };
        if !scheduler.notify_module_script_graph_ready_work(pending_script_id.key(), work) {
            return false;
        }
        self.queue_ready_owner_if_ready_count_increased(owner, ready_count_before)
    }

    pub(crate) fn notify_module_script_graph_failed_action(
        &mut self,
        failure: ParserModuleGraphFailure,
    ) -> bool
    where
        ParserModuleGraphFailure:
            DocumentScriptReadyActionRoute<Owner> + ParserPendingScriptRoute<Owner>,
    {
        let pending_script_id = failure.parser_pending_script_id();
        let owner = pending_script_id.owner();
        debug_assert!(owner == failure.payload_document_owner());
        let ready_count_before = self.pending_ready_work_count(owner);
        let Some(scheduler) = self.documents.get_mut(&owner) else {
            return false;
        };
        if !scheduler.notify_module_script_graph_failed_action(pending_script_id.key(), failure) {
            return false;
        }
        self.queue_ready_owner_if_ready_count_increased(owner, ready_count_before)
    }

    pub(crate) fn notify_module_script_evaluation_completed(
        &mut self,
        evaluation: ParserModuleEvaluation,
    ) where
        ParserModuleEvaluation: DocumentScriptReadyActionRoute<Owner>,
    {
        let owner = evaluation.payload_document_owner();
        let ready_count_before = self.pending_ready_work_count(owner);
        self.documents
            .entry(owner)
            .or_default()
            .notify_module_script_evaluation_completed(evaluation);
        self.queue_ready_owner_if_ready_count_increased(owner, ready_count_before);
    }

    pub(crate) fn notify_parser_classic_ready_work(
        &mut self,
        owner: Owner,
        ready: ParserClassicReady,
    ) {
        let ready_count_before = self.pending_ready_work_count(owner);
        self.documents
            .entry(owner)
            .or_default()
            .notify_parser_classic_ready_work(ready);
        self.queue_ready_owner_if_ready_count_increased(owner, ready_count_before);
    }

    pub(crate) fn notify_parser_classic_source_failure_work(
        &mut self,
        owner: Owner,
        failure: ParserClassicSourceFailure,
    ) {
        let ready_count_before = self.pending_ready_work_count(owner);
        self.documents
            .entry(owner)
            .or_default()
            .notify_parser_classic_source_failure_work(failure);
        self.queue_ready_owner_if_ready_count_increased(owner, ready_count_before);
    }

    pub(crate) fn notify_parser_classic_next_owner_action(
        &mut self,
        action: ParserClassicScriptNextOwnerAction<ParserClassicReady, ParserClassicSourceFailure>,
    ) where
        ParserClassicReady: DocumentScriptReadyActionRoute<Owner>,
        ParserClassicSourceFailure: DocumentScriptReadyActionRoute<Owner>,
    {
        match action {
            ParserClassicScriptNextOwnerAction::Ready(ready) => {
                self.notify_parser_classic_ready_work(ready.payload_document_owner(), ready);
            }
            ParserClassicScriptNextOwnerAction::SourceFailed(failure) => {
                self.notify_parser_classic_source_failure_work(
                    failure.payload_document_owner(),
                    failure,
                );
            }
        }
    }

    pub(crate) fn reserve_pending_parser_module_evaluation(
        &mut self,
        work: PendingParserModuleEvaluation,
        root_entry: ModuleEntryId,
    ) -> u64 {
        self.parser_module_evaluations
            .reserve_pending(work, root_entry)
    }

    pub(crate) fn push_pending_parser_module_evaluation_with_reaction_id(
        &mut self,
        work: PendingParserModuleEvaluation,
        root_entry: ModuleEntryId,
        reaction_id: u64,
    ) {
        self.parser_module_evaluations
            .push_pending_with_reaction_id(work, root_entry, reaction_id);
    }

    pub(crate) fn remove_pending_parser_module_evaluation(&mut self, reaction_id: u64) -> bool {
        self.parser_module_evaluations.remove(reaction_id)
    }

    pub(crate) fn mark_parser_module_evaluation_fulfilled<Convert>(
        &mut self,
        reaction_id: u64,
        mut convert: Convert,
    ) -> Option<ParserModuleEvaluationReactionUpdate>
    where
        Convert: FnMut(
            ParserModuleEvaluationContinuation<PendingParserModuleEvaluation>,
        ) -> ParserModuleEvaluation,
        ParserModuleEvaluation: DocumentScriptReadyActionRoute<Owner>,
    {
        let root_entry = self.parser_module_evaluations.mark_fulfilled(reaction_id)?;
        let queued_ready_action_count = self.promote_ready_parser_module_evaluations(&mut convert);
        Some(ParserModuleEvaluationReactionUpdate {
            root_entry,
            queued_ready_action_count,
        })
    }

    pub(crate) fn mark_parser_module_evaluation_rejected<Convert>(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
        mut convert: Convert,
    ) -> Option<ParserModuleEvaluationReactionUpdate>
    where
        Convert: FnMut(
            ParserModuleEvaluationContinuation<PendingParserModuleEvaluation>,
        ) -> ParserModuleEvaluation,
        ParserModuleEvaluation: DocumentScriptReadyActionRoute<Owner>,
    {
        let root_entry =
            self.parser_module_evaluations
                .mark_rejected(reaction_id, reason, error_constructor)?;
        let queued_ready_action_count = self.promote_ready_parser_module_evaluations(&mut convert);
        Some(ParserModuleEvaluationReactionUpdate {
            root_entry,
            queued_ready_action_count,
        })
    }

    fn promote_ready_parser_module_evaluations<Convert>(&mut self, convert: &mut Convert) -> usize
    where
        Convert: FnMut(
            ParserModuleEvaluationContinuation<PendingParserModuleEvaluation>,
        ) -> ParserModuleEvaluation,
        ParserModuleEvaluation: DocumentScriptReadyActionRoute<Owner>,
    {
        let mut promoted = 0;
        while let Some(evaluation) = self.parser_module_evaluations.take_ready() {
            let ready_evaluation = convert(evaluation);
            self.notify_module_script_evaluation_completed(ready_evaluation);
            promoted += 1;
        }
        promoted
    }

    #[cfg(test)]
    pub(crate) fn take_next_ready_work(
        &mut self,
    ) -> Option<
        DocumentScriptReadyWork<
            Target,
            ParserModuleEvaluation,
            ParserModuleGraphFailure,
            ParserClassicReady,
            ParserClassicSourceFailure,
        >,
    > {
        while let Some(owner) = self.ready_work_owners.pop_front() {
            let Some(work) = self.take_next_document_script_ready_work(owner) else {
                continue;
            };
            if self.pending_ready_work_count(owner) > 0 {
                self.ready_work_owners.push_unique(owner);
            }
            return Some(work);
        }
        None
    }

    fn take_next_owned_ready_work(
        &mut self,
    ) -> Option<
        DocumentOwnedScriptReadyAction<
            Owner,
            DocumentScriptReadyWork<
                Target,
                ParserModuleEvaluation,
                ParserModuleGraphFailure,
                ParserClassicReady,
                ParserClassicSourceFailure,
            >,
        >,
    > {
        while let Some(owner) = self.ready_work_owners.pop_front() {
            let Some(work) = self.take_next_document_script_ready_work(owner) else {
                continue;
            };
            if self.pending_ready_work_count(owner) > 0 {
                self.ready_work_owners.push_unique(owner);
            }
            return Some(DocumentOwnedScriptReadyAction::new(owner, work));
        }
        None
    }

    /// Take one ready action whose Document owner is currently runnable.
    ///
    /// Owners rejected by `owner_is_runnable` keep both their payload and
    /// their relative place in the owner rotation. This is the admission
    /// boundary needed by child Documents whose script work can become ready
    /// before their exact realm is executable: a blocked owner must not be
    /// consumed merely so a later owner can run.
    fn take_next_owned_ready_work_matching(
        &mut self,
        mut owner_is_runnable: impl FnMut(Owner) -> bool,
    ) -> Option<
        DocumentOwnedScriptReadyAction<
            Owner,
            DocumentScriptReadyWork<
                Target,
                ParserModuleEvaluation,
                ParserModuleGraphFailure,
                ParserClassicReady,
                ParserClassicSourceFailure,
            >,
        >,
    > {
        let owners_to_inspect = self.ready_work_owners.len();
        for _ in 0..owners_to_inspect {
            let owner = self
                .ready_work_owners
                .pop_front()
                .expect("bounded ready-owner scan must retain its observed owner");
            if !owner_is_runnable(owner) {
                self.ready_work_owners.push_unique(owner);
                continue;
            }
            let Some(work) = self.take_next_document_script_ready_work(owner) else {
                continue;
            };
            if self.pending_ready_work_count(owner) > 0 {
                self.ready_work_owners.push_unique(owner);
            }
            return Some(DocumentOwnedScriptReadyAction::new(owner, work));
        }
        None
    }

    pub(crate) fn take_next_ready_dispatch<Route>(
        &mut self,
    ) -> Option<
        Result<
            DocumentScriptReadyDispatch<
                Owner,
                DocumentScriptReadyWork<
                    Target,
                    ParserModuleEvaluation,
                    ParserModuleGraphFailure,
                    ParserClassicReady,
                    ParserClassicSourceFailure,
                >,
                Route,
            >,
            DocumentScriptReadyDispatchOwnerMismatch<Owner, Route>,
        >,
    >
    where
        DocumentScriptReadyWork<
            Target,
            ParserModuleEvaluation,
            ParserModuleGraphFailure,
            ParserClassicReady,
            ParserClassicSourceFailure,
        >: DocumentScriptReadyActionDispatchRoute<Route> + DocumentScriptReadyActionRoute<Owner>,
    {
        self.take_next_owned_ready_work()
            .map(DocumentOwnedScriptReadyAction::into_dispatch)
    }

    pub(crate) fn take_next_ready_dispatch_matching<Route>(
        &mut self,
        owner_is_runnable: impl FnMut(Owner) -> bool,
    ) -> Option<
        Result<
            DocumentScriptReadyDispatch<
                Owner,
                DocumentScriptReadyWork<
                    Target,
                    ParserModuleEvaluation,
                    ParserModuleGraphFailure,
                    ParserClassicReady,
                    ParserClassicSourceFailure,
                >,
                Route,
            >,
            DocumentScriptReadyDispatchOwnerMismatch<Owner, Route>,
        >,
    >
    where
        DocumentScriptReadyWork<
            Target,
            ParserModuleEvaluation,
            ParserModuleGraphFailure,
            ParserClassicReady,
            ParserClassicSourceFailure,
        >: DocumentScriptReadyActionDispatchRoute<Route> + DocumentScriptReadyActionRoute<Owner>,
    {
        self.take_next_owned_ready_work_matching(owner_is_runnable)
            .map(DocumentOwnedScriptReadyAction::into_dispatch)
    }

    pub(crate) fn take_next_claimed_ready_dispatch<Route, Claimed, Claim, ReportMismatch>(
        &mut self,
        mut claim: Claim,
        mut report_mismatch: ReportMismatch,
    ) -> Option<Claimed>
    where
        DocumentScriptReadyWork<
            Target,
            ParserModuleEvaluation,
            ParserModuleGraphFailure,
            ParserClassicReady,
            ParserClassicSourceFailure,
        >: DocumentScriptReadyActionDispatchRoute<Route> + DocumentScriptReadyActionRoute<Owner>,
        Claim: FnMut(
            DocumentScriptReadyDispatch<
                Owner,
                DocumentScriptReadyWork<
                    Target,
                    ParserModuleEvaluation,
                    ParserModuleGraphFailure,
                    ParserClassicReady,
                    ParserClassicSourceFailure,
                >,
                Route,
            >,
        ) -> Option<Claimed>,
        ReportMismatch: FnMut(DocumentScriptReadyDispatchOwnerMismatch<Owner, Route>),
    {
        while let Some(dispatch) = self.take_next_ready_dispatch::<Route>() {
            match dispatch {
                Ok(dispatch) => {
                    if let Some(claimed) = claim(dispatch) {
                        return Some(claimed);
                    }
                }
                Err(mismatch) => report_mismatch(mismatch),
            }
        }
        None
    }

    pub(crate) fn has_ready_work(&self) -> bool {
        !self.ready_work_owners.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn has_load_blocking_document_script_work(&self, owner: Owner) -> bool {
        self.documents
            .get(&owner)
            .is_some_and(DocumentScriptScheduler::has_load_blocking_document_script_work)
    }

    fn queue_ready_owner_if_ready_count_increased(
        &mut self,
        owner: Owner,
        ready_count_before: usize,
    ) -> bool {
        if self.pending_ready_work_count(owner) > ready_count_before {
            self.ready_work_owners.push_unique(owner);
            return true;
        }
        false
    }

    fn pending_ready_work_count(&self, owner: Owner) -> usize {
        self.documents
            .get(&owner)
            .map(DocumentScriptScheduler::pending_ready_work_count)
            .unwrap_or_default()
    }

    fn take_next_document_script_ready_work(
        &mut self,
        owner: Owner,
    ) -> Option<
        DocumentScriptReadyWork<
            Target,
            ParserModuleEvaluation,
            ParserModuleGraphFailure,
            ParserClassicReady,
            ParserClassicSourceFailure,
        >,
    > {
        self.documents.get_mut(&owner)?.take_next_ready_work()
    }

    #[cfg(test)]
    pub(crate) fn pending_parser_module_script_count_for_test(&self, owner: Owner) -> usize {
        self.documents
            .get(&owner)
            .map(DocumentScriptScheduler::pending_parser_module_script_count_for_test)
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn pending_module_graph_ready_count_for_test(&self, owner: Owner) -> usize {
        self.documents
            .get(&owner)
            .map(DocumentScriptScheduler::pending_module_graph_ready_count)
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn pending_parser_module_evaluation_count_for_test(&self, owner: Owner) -> usize {
        self.documents
            .get(&owner)
            .map(DocumentScriptScheduler::pending_parser_module_evaluation_count)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_runtime::DomHandle;
    use crate::document_script_scheduler::{ParserModuleGraphTerminalWork, ParserPendingScriptKey};
    use crate::dom::NodeId;
    use crate::frame_owner_model::{
        DocumentId, DocumentLoadDelayTokenId, FrameDocumentOwner, FrameDocumentTaskOwner,
        FrameRealmId, FrameSchedulerLaneId, LocalWindowId,
    };
    use crate::module_runtime::{ModuleEntryId, ModuleMapKey};
    use crate::planning::{PreparedScript, ScriptFetchMetadata, ScriptSource};
    use crate::types::{ScriptKind, ScriptMode, ScriptSourceKind};
    use moli_module_script_tree as module_tree;

    fn parser_module_script(url: &str, node_id: usize, position: usize) -> PreparedScript {
        let script_url = url::Url::parse(url).expect("module script url");
        PreparedScript {
            position,
            node_id: NodeId::new(node_id),
            kind: ScriptKind::Module,
            mode: ScriptMode::ModuleDefer,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: script_url.clone(),
            base_url: script_url.clone(),
            initiator_url: script_url,
            host_script_handle: None,
        }
    }

    fn parser_classic_defer_script(url: &str, node_id: usize, position: usize) -> PreparedScript {
        let mut script = parser_module_script(url, node_id, position);
        script.kind = ScriptKind::Classic;
        script.mode = ScriptMode::Defer;
        script
    }

    fn parser_deferred_load_delay_token(id: u64) -> DocumentLoadDelayTokenId {
        DocumentLoadDelayTokenId(id)
    }

    fn graph_ready_work(
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        script: PreparedScript,
        script_handle: DomHandle,
        tree_id: module_tree::ModuleTreeId,
        entries: Vec<ModuleEntryId>,
    ) -> DocumentModuleGraphReadyWork {
        let key = ModuleMapKey::java_script(script.url.clone());
        let pending_script_id = ParserPendingScriptId::new(task_owner.document_owner(), &script);
        DocumentModuleGraphReadyWork::new(
            task_owner,
            realm_id,
            pending_script_id,
            script,
            script_handle,
            key,
            tree_id,
            crate::frame_owner_model::DocumentLoadDelayTokenId(1),
            crate::module_runtime::ModuleGraphHandle {
                root_entry: entries[0],
                entries,
            },
        )
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MainLikeModuleReadyTarget {
        owner: u64,
        route: u64,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RoutedReadyAction {
        owner: u64,
        route: u64,
        value: &'static str,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct TestModuleGraphFailure {
        owner: u64,
        pending_script_key: crate::document_script_scheduler::ParserPendingScriptKey,
        message: &'static str,
    }

    impl TestModuleGraphFailure {
        fn new(owner: u64, script: &PreparedScript, message: &'static str) -> Self {
            Self {
                owner,
                pending_script_key:
                    crate::document_script_scheduler::ParserPendingScriptKey::from_script(script),
                message,
            }
        }

        fn script_node_id(&self) -> NodeId {
            self.pending_script_key.script_node_id()
        }
    }

    impl DocumentScriptReadyActionRoute<u64> for TestModuleGraphFailure {
        fn payload_document_owner(&self) -> u64 {
            self.owner
        }
    }

    impl ParserPendingScriptRoute<u64> for TestModuleGraphFailure {
        fn parser_pending_script_id(&self) -> ParserPendingScriptId<u64> {
            ParserPendingScriptId::from_key(self.owner, self.pending_script_key)
        }
    }

    impl DocumentScriptReadyActionRoute<u64> for RoutedReadyAction {
        fn payload_document_owner(&self) -> u64 {
            self.owner
        }
    }

    impl DocumentScriptReadyActionDispatchRoute<u64> for RoutedReadyAction {
        fn dispatch_route(&self) -> u64 {
            self.route
        }
    }

    impl DocumentScriptReadyActionRoute<u64> for ModuleScriptGraphReadyWork<MainLikeModuleReadyTarget> {
        fn payload_document_owner(&self) -> u64 {
            self.target().owner
        }
    }

    impl ParserPendingScriptRoute<u64> for ModuleScriptGraphReadyWork<MainLikeModuleReadyTarget> {
        fn parser_pending_script_id(&self) -> ParserPendingScriptId<u64> {
            ParserPendingScriptId::new(self.target().owner, self.script())
        }
    }

    impl DocumentScriptReadyActionDispatchRoute<u64>
        for ModuleScriptGraphReadyWork<MainLikeModuleReadyTarget>
    {
        fn dispatch_route(&self) -> u64 {
            self.target().route
        }
    }

    fn main_like_graph_ready_work(
        target: MainLikeModuleReadyTarget,
        script: PreparedScript,
        entries: Vec<ModuleEntryId>,
    ) -> ModuleScriptGraphReadyWork<MainLikeModuleReadyTarget> {
        ModuleScriptGraphReadyWork::with_target(
            target,
            script,
            crate::module_runtime::ModuleGraphHandle {
                root_entry: entries[0],
                entries,
            },
        )
    }

    #[test]
    fn document_script_scheduler_store_routes_graph_ready_work_by_owner() {
        let mut store: DocumentScriptSchedulerStore<FrameDocumentOwner> =
            DocumentScriptSchedulerStore::default();
        let task_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
        let document_owner = task_owner.document_owner();
        let other_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(4), DocumentId(5))
                .document_owner();
        let script = parser_module_script("https://document-scripts.test/module.js", 7, 7);
        let watch = store.register_and_watch_module_script(document_owner, &script);
        assert!(watch.watched());
        let work = graph_ready_work(
            task_owner,
            FrameRealmId(6),
            script,
            DomHandle::new(7),
            module_tree::ModuleTreeId(8),
            vec![ModuleEntryId::from_raw(9), ModuleEntryId::from_raw(10)],
        );

        store.notify_module_script_graph_ready_work(work.clone());

        assert!(store.has_ready_work());
        assert_eq!(
            store.pending_module_graph_ready_count_for_test(document_owner),
            1
        );
        assert_eq!(
            store.pending_module_graph_ready_count_for_test(other_owner),
            0
        );
        let claimed = store
            .take_next_ready_work()
            .expect("document-owned graph-ready work should be queued under its owner")
            .into_module_script_graph_ready();
        assert_eq!(claimed.owner(), work.owner());
        assert_eq!(claimed.realm_id(), work.realm_id());
        assert_eq!(claimed.script().node_id, work.script().node_id);
        assert_eq!(claimed.script_handle(), work.script_handle());
        assert_eq!(claimed.request_key(), work.request_key());
        assert_eq!(claimed.tree_id(), work.tree_id());
        assert_eq!(claimed.graph().entries, work.graph().entries);
        assert!(!store.has_ready_work());
    }

    #[test]
    fn parser_ordered_module_retains_its_terminal_without_blocking_async_module_release() {
        let mut store: DocumentScriptSchedulerStore<FrameDocumentOwner> =
            DocumentScriptSchedulerStore::default();
        let task_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
        let document_owner = task_owner.document_owner();
        let first = parser_module_script("https://document-scripts.test/parser-ordered.js", 10, 10);
        let mut later_async =
            parser_module_script("https://document-scripts.test/async.js", 20, 20);
        later_async.mode = ScriptMode::Async;

        let first_id = store
            .accept_parser_ordered_module_script(document_owner, &first, Default::default())
            .expect("parser-ordered module should be accepted");
        let async_watch = store.register_and_watch_module_script(document_owner, &later_async);
        assert!(!store.module_script_is_watching_for_test(first_id));
        assert!(async_watch.watched());

        assert!(
            store.notify_module_script_graph_ready_work(graph_ready_work(
                task_owner,
                FrameRealmId(4),
                later_async.clone(),
                DomHandle::new(20),
                module_tree::ModuleTreeId(20),
                vec![ModuleEntryId::from_raw(20)],
            ))
        );
        assert!(
            !store.notify_module_script_graph_ready_work(graph_ready_work(
                task_owner,
                FrameRealmId(4),
                first.clone(),
                DomHandle::new(10),
                module_tree::ModuleTreeId(10),
                vec![ModuleEntryId::from_raw(10)],
            )),
            "parser-ordered terminal must stay on its exact PendingScript"
        );
        assert!(store.parser_ordered_module_terminal_is_ready(first_id));

        let async_ready = store
            .take_next_ready_work()
            .expect("async module should not wait behind parser-deferred order")
            .into_module_script_graph_ready();
        assert_eq!(async_ready.script().node_id, later_async.node_id);
        assert!(store.promote_parser_ordered_module_terminal(first_id));
        let parser_ready = store
            .take_next_ready_work()
            .expect("parser order owner should release the retained exact terminal")
            .into_module_script_graph_ready();
        assert_eq!(parser_ready.script().node_id, first.node_id);
    }

    #[test]
    fn document_script_scheduler_store_routes_graph_failure_by_payload_owner() {
        let mut store: DocumentScriptSchedulerStore<
            u64,
            std::convert::Infallible,
            std::convert::Infallible,
            TestModuleGraphFailure,
        > = DocumentScriptSchedulerStore::default();
        let owner = 42;
        let other_owner = 77;
        let script = parser_module_script("https://document-scripts.test/failed-module.js", 24, 24);

        let pending_script_id = store.register_module_script(owner, &script);
        assert!(store.watch_module_script(pending_script_id).watched());
        assert!(
            store.notify_module_script_graph_failed_action(TestModuleGraphFailure::new(
                owner,
                &script,
                "graph failed",
            ))
        );

        let owned_work = store
            .take_next_owned_ready_work()
            .expect("graph failure should be queued under its payload owner");
        assert_eq!(*owned_work.owner(), owner);
        let failure = owned_work.into_action().into_module_script_graph_failed();
        assert_eq!(failure.owner, owner);
        assert_eq!(failure.script_node_id(), script.node_id);
        assert_eq!(failure.message, "graph failed");
        assert_eq!(store.pending_ready_work_count(other_owner), 0);
        assert!(!store.has_ready_work());
    }

    #[test]
    fn remove_document_clears_parser_pending_ready_and_evaluation_state() {
        let mut store: DocumentScriptSchedulerStore<
            u64,
            MainLikeModuleReadyTarget,
            RoutedReadyAction,
            TestModuleGraphFailure,
            RoutedReadyAction,
        > = DocumentScriptSchedulerStore::default();
        let owner = 42;
        let script =
            parser_module_script("https://document-scripts.test/cancelled-module.js", 25, 25);
        let pending_script_id = store.register_module_script(owner, &script);
        assert!(store.watch_module_script(pending_script_id).watched());
        let reaction_id = store.reserve_pending_parser_module_evaluation(
            RoutedReadyAction {
                owner,
                route: 1,
                value: "pending evaluation",
            },
            ModuleEntryId::from_raw(25),
        );
        assert!(
            store.notify_module_script_graph_ready_work(main_like_graph_ready_work(
                MainLikeModuleReadyTarget { owner, route: 2 },
                script,
                vec![ModuleEntryId::from_raw(25)],
            ))
        );
        assert!(store.has_load_blocking_document_script_work(owner));

        store.remove_document(owner);

        assert!(!store.has_load_blocking_document_script_work(owner));
        assert!(!store.has_ready_work());
        assert_eq!(store.pending_ready_work_count(owner), 0);
        assert!(
            store
                .mark_parser_module_evaluation_fulfilled(reaction_id, |evaluation| {
                    let (work, _, _, _) = evaluation.into_parts();
                    work
                })
                .is_none(),
            "removed document must not retain a pending evaluation reaction"
        );
    }

    #[test]
    fn document_script_scheduler_store_returns_owner_tagged_ready_work() {
        let mut store: DocumentScriptSchedulerStore<FrameDocumentOwner> =
            DocumentScriptSchedulerStore::default();
        let task_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
        let document_owner = task_owner.document_owner();
        let script = parser_module_script("https://document-scripts.test/owner-work.js", 41, 41);
        let watch = store.register_and_watch_module_script(document_owner, &script);
        assert!(watch.watched());
        let work = graph_ready_work(
            task_owner,
            FrameRealmId(7),
            script,
            DomHandle::new(41),
            module_tree::ModuleTreeId(42),
            vec![ModuleEntryId::from_raw(43)],
        );

        store.notify_module_script_graph_ready_work(work.clone());

        let owned_work = store
            .take_next_owned_ready_work()
            .expect("ready work should retain its document owner");
        assert_eq!(*owned_work.owner(), document_owner);
        let claimed = owned_work.into_action().into_module_script_graph_ready();
        assert_eq!(claimed.owner().document_owner(), document_owner);
        assert_eq!(claimed.script().node_id, work.script().node_id);
        assert_eq!(claimed.graph().entries, work.graph().entries);
        assert!(!store.has_ready_work());
    }

    #[test]
    fn document_script_scheduler_store_accepts_non_child_ready_target() {
        let mut store: DocumentScriptSchedulerStore<u64, MainLikeModuleReadyTarget> =
            DocumentScriptSchedulerStore::default();
        let owner = 42;
        let script = parser_module_script("https://document-scripts.test/main-module.js", 21, 21);
        let target = MainLikeModuleReadyTarget { owner, route: 77 };
        let work = main_like_graph_ready_work(
            target.clone(),
            script.clone(),
            vec![ModuleEntryId::from_raw(22), ModuleEntryId::from_raw(23)],
        );

        let pending_script_id = store.register_module_script(owner, &script);
        store.notify_module_script_graph_ready_work(work.clone());

        assert!(!store.has_ready_work());
        let watch = store.watch_module_script(pending_script_id);
        assert!(watch.watched());
        assert!(watch.queued_ready_work());
        let claimed = store
            .take_next_ready_work()
            .expect("non-child graph-ready work should share the scheduler ready lane")
            .into_module_script_graph_ready();
        assert_eq!(claimed.target(), &target);
        assert_eq!(claimed.script().node_id, work.script().node_id);
        assert_eq!(claimed.graph().entries, work.graph().entries);
        assert!(!store.has_ready_work());
    }

    #[test]
    fn document_script_scheduler_store_claimed_dispatch_skips_mismatch_and_stale_work() {
        let mut store: DocumentScriptSchedulerStore<
            u64,
            MainLikeModuleReadyTarget,
            std::convert::Infallible,
            std::convert::Infallible,
            std::convert::Infallible,
            RoutedReadyAction,
        > = DocumentScriptSchedulerStore::default();

        store.notify_parser_classic_ready_work(
            1,
            RoutedReadyAction {
                owner: 2,
                route: 20,
                value: "mismatch",
            },
        );
        store.notify_parser_classic_ready_work(
            3,
            RoutedReadyAction {
                owner: 3,
                route: 30,
                value: "stale",
            },
        );
        store.notify_parser_classic_ready_work(
            4,
            RoutedReadyAction {
                owner: 4,
                route: 40,
                value: "claimed",
            },
        );

        let mut stale_routes = Vec::new();
        let mut mismatches = Vec::new();
        let claimed = store
            .take_next_claimed_ready_dispatch::<u64, _, _, _>(
                |dispatch| {
                    let route = *dispatch.route();
                    if route == 30 {
                        stale_routes.push(route);
                        return None;
                    }
                    let (work, route) = dispatch.into_action_and_route();
                    let DocumentScriptReadyWork::ParserClassicReady(ready) = work else {
                        panic!("expected parser-classic ready work");
                    };
                    Some((ready.value, route))
                },
                |mismatch| {
                    mismatches.push((*mismatch.queued_owner(), *mismatch.payload_owner()));
                },
            )
            .expect("first claimable ready work should be returned");

        assert_eq!(mismatches, vec![(1, 2)]);
        assert_eq!(stale_routes, vec![30]);
        assert_eq!(claimed.1, 40);
        assert_eq!(claimed.0, "claimed");
        assert!(!store.has_ready_work());
    }

    #[test]
    fn document_script_scheduler_store_routes_evaluation_ready_work_by_owner() {
        let mut store: DocumentScriptSchedulerStore<
            u64,
            FrameDocumentModuleGraphReadyTarget,
            RoutedReadyAction,
        > = DocumentScriptSchedulerStore::default();
        let owner = 1;
        let other_owner = 2;

        store.notify_module_script_evaluation_completed(RoutedReadyAction {
            owner,
            route: 10,
            value: "settled",
        });

        assert!(store.has_ready_work());
        assert_eq!(
            store.pending_parser_module_evaluation_count_for_test(owner),
            1
        );
        assert_eq!(
            store.pending_parser_module_evaluation_count_for_test(other_owner),
            0
        );
        let Some(DocumentScriptReadyWork::ModuleScriptEvaluationCompleted(reaction_id)) =
            store.take_next_ready_work()
        else {
            panic!("document-owned evaluation-ready work should be queued under its owner");
        };
        assert_eq!(reaction_id.value, "settled");
        assert!(!store.has_ready_work());
    }

    #[test]
    fn document_script_scheduler_store_routes_parser_classic_ready_work_by_owner() {
        let mut store: DocumentScriptSchedulerStore<
            FrameDocumentOwner,
            FrameDocumentModuleGraphReadyTarget,
            std::convert::Infallible,
            std::convert::Infallible,
            std::convert::Infallible,
            u64,
            &'static str,
        > = DocumentScriptSchedulerStore::default();
        let owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
                .document_owner();
        let other_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(4), DocumentId(5))
                .document_owner();

        store.notify_parser_classic_ready_work(owner, 42);
        store.notify_parser_classic_source_failure_work(other_owner, "source failed");

        assert!(store.has_ready_work());
        let Some(DocumentScriptReadyWork::ParserClassicReady(ready)) = store.take_next_ready_work()
        else {
            panic!("parser classic ready work should be queued by document owner");
        };
        assert_eq!(*ready, 42);

        let Some(DocumentScriptReadyWork::ParserClassicSourceFailed(failure)) =
            store.take_next_ready_work()
        else {
            panic!("parser classic source-failure work should be queued by document owner");
        };
        assert_eq!(*failure, "source failed");
        assert!(!store.has_ready_work());
    }

    #[test]
    fn document_script_scheduler_store_promotes_ready_parser_module_evaluations() {
        #[derive(Debug)]
        struct PendingEvaluation {
            owner: u64,
            value: &'static str,
        }

        impl DocumentScriptReadyActionRoute<u64> for PendingEvaluation {
            fn payload_document_owner(&self) -> u64 {
                self.owner
            }
        }

        let mut store: DocumentScriptSchedulerStore<
            u64,
            MainLikeModuleReadyTarget,
            PendingEvaluation,
            std::convert::Infallible,
            PendingEvaluation,
        > = DocumentScriptSchedulerStore::default();

        let first = store.reserve_pending_parser_module_evaluation(
            PendingEvaluation {
                owner: 1,
                value: "first",
            },
            ModuleEntryId::from_raw(1),
        );
        let second = store.reserve_pending_parser_module_evaluation(
            PendingEvaluation {
                owner: 2,
                value: "second",
            },
            ModuleEntryId::from_raw(2),
        );

        assert!(!store.has_ready_work());
        assert!(
            !store.has_load_blocking_document_script_work(1),
            "pending parser-module evaluation must not reacquire lifecycle ownership"
        );
        assert!(
            !store.has_load_blocking_document_script_work(2),
            "TLA reaction bookkeeping must remain separate from document load delay"
        );

        let second_update = store
            .mark_parser_module_evaluation_fulfilled(second, |evaluation| {
                let (work, _, _, _) = evaluation.into_parts();
                work
            })
            .expect("second reaction should be accepted");
        assert_eq!(second_update.root_entry(), ModuleEntryId::from_raw(2));
        assert_eq!(second_update.queued_ready_action_count(), 1);
        assert!(store.has_ready_work());
        assert!(
            store.has_load_blocking_document_script_work(2),
            "settled evaluation should still block load while ready work is queued"
        );
        assert_eq!(store.pending_parser_module_evaluation_count_for_test(1), 0);
        assert_eq!(store.pending_parser_module_evaluation_count_for_test(2), 1);
        let Some(DocumentScriptReadyWork::ModuleScriptEvaluationCompleted(value)) =
            store.take_next_ready_work()
        else {
            panic!("ready evaluation should be queued as document-script work");
        };
        assert_eq!(value.value, "second");
        assert!(
            !store.has_load_blocking_document_script_work(2),
            "consumed evaluation-ready work should release load for that owner"
        );
        assert!(
            !store.has_load_blocking_document_script_work(1),
            "another pending TLA reaction must not block its document"
        );

        let first_update = store
            .mark_parser_module_evaluation_fulfilled(first, |evaluation| {
                let (work, _, _, _) = evaluation.into_parts();
                work
            })
            .expect("first reaction should be accepted");
        assert_eq!(first_update.root_entry(), ModuleEntryId::from_raw(1));
        assert_eq!(first_update.queued_ready_action_count(), 1);
        let Some(DocumentScriptReadyWork::ModuleScriptEvaluationCompleted(value)) =
            store.take_next_ready_work()
        else {
            panic!("second ready evaluation should be queued as document-script work");
        };
        assert_eq!(value.value, "first");
        assert!(!store.has_ready_work());
        assert!(
            !store.has_load_blocking_document_script_work(1),
            "consumed final evaluation-ready work should release load"
        );
    }

    #[test]
    fn document_script_scheduler_store_waits_for_module_script_watch() {
        let mut store: DocumentScriptSchedulerStore<FrameDocumentOwner> =
            DocumentScriptSchedulerStore::default();
        let task_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
        let document_owner = task_owner.document_owner();
        let script =
            parser_module_script("https://document-scripts.test/watched-module.js", 11, 11);
        let work = graph_ready_work(
            task_owner,
            FrameRealmId(6),
            script.clone(),
            DomHandle::new(11),
            module_tree::ModuleTreeId(12),
            vec![ModuleEntryId::from_raw(13)],
        );

        let pending_script_id = store.register_module_script(document_owner, &script);
        assert!(
            !store.has_load_blocking_document_script_work(document_owner),
            "prestarted module graph must not block load before the parser runner watches it"
        );
        store.notify_module_script_graph_ready_work(work.clone());

        assert!(!store.has_ready_work());
        assert!(
            !store.has_load_blocking_document_script_work(document_owner),
            "terminal graph must stay non-blocking until the parser runner watches its PendingScript"
        );
        assert_eq!(
            store.pending_parser_module_script_count_for_test(document_owner),
            1
        );
        assert_eq!(
            store.pending_module_graph_ready_count_for_test(document_owner),
            0,
            "completed graph should stay terminal on the pending script until the module-script watch"
        );

        let watch = store.watch_module_script(pending_script_id);
        assert!(watch.watched());
        assert!(watch.queued_ready_work());
        assert!(store.has_ready_work());
        assert!(
            store.has_load_blocking_document_script_work(document_owner),
            "queued module graph-ready work should block load until the ready lane consumes it"
        );
        assert_eq!(
            store.pending_parser_module_script_count_for_test(document_owner),
            0
        );
        let claimed = store
            .take_next_ready_work()
            .expect("watching a terminal module script should enqueue graph-ready work")
            .into_module_script_graph_ready();
        assert_eq!(claimed.script().node_id, work.script().node_id);
        assert_eq!(claimed.graph().entries, work.graph().entries);
    }

    #[test]
    fn duplicate_registration_preserves_the_original_pending_script_identity() {
        let mut store: DocumentScriptSchedulerStore<u64, MainLikeModuleReadyTarget> =
            DocumentScriptSchedulerStore::default();
        let owner = 42;
        let original = parser_module_script("https://document-scripts.test/identity.js", 17, 10);
        let mut duplicate = original.clone();
        duplicate.position = 99;

        let original_id = store.register_module_script(owner, &original);
        let duplicate_id = store.register_module_script(owner, &duplicate);

        assert_eq!(duplicate_id, original_id);
        assert!(store.watch_module_script(original_id).watched());
        assert!(store.module_script_is_watching_for_test(original_id));
        assert!(
            !store
                .module_script_is_watching_for_test(ParserPendingScriptId::new(owner, &duplicate,))
        );
        assert_eq!(
            store.pending_script_id_for_script(&original),
            Some(original_id)
        );
        assert_eq!(store.pending_script_id_for_script(&duplicate), None);
    }

    #[test]
    fn pending_script_lookup_fails_closed_when_owner_identity_is_ambiguous() {
        let mut store: DocumentScriptSchedulerStore<u64, MainLikeModuleReadyTarget> =
            DocumentScriptSchedulerStore::default();
        let script =
            parser_module_script("https://document-scripts.test/ambiguous-owner.js", 18, 18);
        let first_id = store.register_module_script(1, &script);
        store.register_module_script(2, &script);

        assert_eq!(store.pending_script_id_for_script(&script), None);

        store.remove_document(2);
        assert_eq!(store.pending_script_id_for_script(&script), Some(first_id));
    }

    #[test]
    fn unregistered_or_stale_terminal_does_not_materialize_ready_state() {
        let mut store: DocumentScriptSchedulerStore<FrameDocumentOwner> =
            DocumentScriptSchedulerStore::default();
        let task_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
        let document_owner = task_owner.document_owner();
        let script =
            parser_module_script("https://document-scripts.test/stale-terminal.js", 19, 19);
        let unregistered_work = graph_ready_work(
            task_owner,
            FrameRealmId(4),
            script.clone(),
            DomHandle::new(19),
            module_tree::ModuleTreeId(20),
            vec![ModuleEntryId::from_raw(21)],
        );

        assert!(!store.notify_module_script_graph_ready_work(unregistered_work));
        assert!(store.documents.is_empty());
        assert!(!store.has_ready_work());

        let mut replacement = script.clone();
        replacement.position = 77;
        let replacement_id = store.register_module_script(document_owner, &replacement);
        assert!(store.watch_module_script(replacement_id).watched());
        let stale_work = graph_ready_work(
            task_owner,
            FrameRealmId(4),
            script,
            DomHandle::new(19),
            module_tree::ModuleTreeId(22),
            vec![ModuleEntryId::from_raw(23)],
        );

        assert!(!store.notify_module_script_graph_ready_work(stale_work));
        assert_eq!(
            store.pending_parser_module_script_count_for_test(document_owner),
            1
        );
        assert!(!store.has_ready_work());
    }

    #[test]
    fn document_script_scheduler_store_releases_parser_modules_in_contiguous_document_order() {
        let mut store: DocumentScriptSchedulerStore<
            u64,
            MainLikeModuleReadyTarget,
            std::convert::Infallible,
            TestModuleGraphFailure,
        > = DocumentScriptSchedulerStore::default();
        let owner = 10;
        let first = parser_module_script("https://document-scripts.test/first-module.js", 31, 10);
        let second = parser_module_script("https://document-scripts.test/second-module.js", 32, 20);
        let third = parser_module_script("https://document-scripts.test/third-module.js", 33, 30);

        for (script, token) in [
            (&first, parser_deferred_load_delay_token(31)),
            (&second, parser_deferred_load_delay_token(32)),
            (&third, parser_deferred_load_delay_token(33)),
        ] {
            let Some(ParserDeferredScriptStartAction::ModuleGraph(start)) = store
                .claim_parser_deferred_script(
                    owner,
                    script.clone(),
                    None,
                    None,
                    Default::default(),
                    token,
                )
            else {
                panic!("parser module-defer must atomically register before graph start");
            };
            let (pending_script_id, start_script) = start.into_parts();
            assert_eq!(start_script.node_id, script.node_id);
            assert!(store.has_module_script(pending_script_id));
        }
        assert_eq!(store.seal_parser_deferred_scripts(owner), Ok(3));

        assert!(
            store.has_load_blocking_document_script_work(owner),
            "pending parser-connected module scripts should block load"
        );
        assert_eq!(store.pending_parser_module_script_count_for_test(owner), 3);

        let second_work = main_like_graph_ready_work(
            MainLikeModuleReadyTarget { owner, route: 2 },
            second.clone(),
            vec![ModuleEntryId::from_raw(200)],
        );
        assert!(
            !store.notify_module_script_graph_ready_work(second_work),
            "later graph-ready terminal should be retained behind the earlier parser module"
        );
        assert!(!store.has_ready_work());
        assert_eq!(store.pending_parser_module_script_count_for_test(owner), 3);

        assert!(
            !store.notify_module_script_graph_failed_action(TestModuleGraphFailure::new(
                owner,
                &third,
                "third graph failed",
            )),
            "later graph-failed terminal should also be retained behind the earlier parser module"
        );
        assert!(!store.has_ready_work());
        assert_eq!(store.pending_parser_module_script_count_for_test(owner), 3);

        let first_work = main_like_graph_ready_work(
            MainLikeModuleReadyTarget { owner, route: 1 },
            first.clone(),
            vec![ModuleEntryId::from_raw(100)],
        );
        assert!(
            !store.notify_module_script_graph_ready_work(first_work),
            "parser-owned terminals must not enter the broad document ready lane"
        );
        assert!(!store.has_ready_work());
        assert!(store.next_after_parsing_script_is_ready(owner));

        let Some(ParserDeferredScriptReady::Module(ready)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("first ordered terminal should be graph-ready work");
        };
        let (terminal, token) = ready.into_parts();
        let ParserModuleGraphTerminalWork::Ready(work) = terminal else {
            panic!("first ordered terminal should be graph-ready work");
        };
        assert_eq!(token, parser_deferred_load_delay_token(31));
        assert_eq!(work.script().node_id, first.node_id);
        assert_eq!(work.target().route, 1);

        let Some(ParserDeferredScriptReady::Module(ready)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("second ordered terminal should be graph-ready work");
        };
        let (terminal, token) = ready.into_parts();
        let ParserModuleGraphTerminalWork::Ready(work) = terminal else {
            panic!("second ordered terminal should be graph-ready work");
        };
        assert_eq!(token, parser_deferred_load_delay_token(32));
        assert_eq!(work.script().node_id, second.node_id);
        assert_eq!(work.target().route, 2);

        let Some(ParserDeferredScriptReady::Module(ready)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("third ordered terminal should be graph-failed work");
        };
        let (terminal, token) = ready.into_parts();
        let ParserModuleGraphTerminalWork::Failed(failure) = terminal else {
            panic!("third ordered terminal should be graph-failed work");
        };
        assert_eq!(token, parser_deferred_load_delay_token(33));
        assert_eq!(failure.message, "third graph failed");
        assert_eq!(store.pending_parser_module_script_count_for_test(owner), 0);
        assert!(!store.has_after_parsing_script(owner));
        assert!(!store.has_ready_work());
        assert!(
            !store.has_load_blocking_document_script_work(owner),
            "consuming the ordered terminal batch should release the owner load gate"
        );
    }

    #[test]
    fn parser_deferred_classic_registers_before_source_start_and_seals_without_waiting() {
        let mut store: DocumentScriptSchedulerStore<u64> = Default::default();
        let owner = 8;
        let script =
            parser_classic_defer_script("https://document-scripts.test/pending-classic.js", 31, 10);
        let Some(ParserDeferredScriptStartAction::ClassicSource(source_request)) = store
            .claim_parser_deferred_script(
                owner,
                script.clone(),
                Some(SharedScriptSourceLoad::ready_ok(
                    "globalThis.__pendingClassic = 1;",
                )),
                None,
                Default::default(),
                parser_deferred_load_delay_token(31),
            )
        else {
            panic!("external classic defer should return a source-start request");
        };

        assert!(
            store.has_load_blocking_document_script_work(owner),
            "PendingScript must exist before the owner adapter starts source work"
        );
        assert!(
            !store.has_after_parsing_script(owner),
            "parser-deferred work cannot execute before EOF"
        );
        assert_eq!(store.seal_parser_deferred_scripts(owner), Ok(1));
        assert!(
            !store.next_after_parsing_script_is_ready(owner),
            "a ready SharedScriptSourceLoad is not applied inline during EOF seal"
        );

        let (pending_script_id, source_load) = source_request
            .start_with_injected_source_load_for_test()
            .into_parts();
        let outcome = source_load
            .try_outcome()
            .expect("synthetic source terminal should be ready");
        assert_eq!(
            store.complete_parser_deferred_classic_source_load(
                ParserDeferredClassicSourceLoadCompletion::new(pending_script_id, outcome),
            ),
            ParserDeferredClassicSourceLoadApplyResult::Applied
        );
        assert!(store.next_after_parsing_script_is_ready(owner));
        let Some(ParserDeferredScriptReady::Classic(script)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("source terminal should release the original PendingScript");
        };
        assert!(matches!(
            &script.script().source,
            ScriptSource::Loaded(source) if source == "globalThis.__pendingClassic = 1;"
        ));
    }

    #[test]
    fn parser_deferred_classic_source_terminal_drops_with_retired_owner() {
        let mut store: DocumentScriptSchedulerStore<u64> = Default::default();
        let owner = 9;
        let Some(ParserDeferredScriptStartAction::ClassicSource(source_request)) = store
            .claim_parser_deferred_script(
                owner,
                parser_classic_defer_script(
                    "https://document-scripts.test/retired-classic.js",
                    32,
                    10,
                ),
                Some(SharedScriptSourceLoad::ready_ok("stale source")),
                None,
                Default::default(),
                parser_deferred_load_delay_token(32),
            )
        else {
            panic!("external classic defer should return a source-start request");
        };
        let (pending_script_id, source_load) = source_request
            .start_with_injected_source_load_for_test()
            .into_parts();
        store.remove_document(owner);

        assert_eq!(
            store.complete_parser_deferred_classic_source_load(
                ParserDeferredClassicSourceLoadCompletion::new(
                    pending_script_id,
                    source_load
                        .try_outcome()
                        .expect("synthetic stale source terminal should be ready"),
                ),
            ),
            ParserDeferredClassicSourceLoadApplyResult::MissingDocument,
            "retired document completion must not recreate parser state"
        );
        assert!(!store.has_load_blocking_document_script_work(owner));
    }

    #[test]
    fn parser_deferred_module_retains_terminal_that_arrives_before_eof() {
        let mut store: DocumentScriptSchedulerStore<u64, MainLikeModuleReadyTarget> =
            Default::default();
        let owner = 10;
        let script =
            parser_module_script("https://document-scripts.test/atomic-module.mjs", 33, 11);
        let Some(ParserDeferredScriptStartAction::ModuleGraph(start)) = store
            .claim_parser_deferred_script(
                owner,
                script.clone(),
                None,
                None,
                Default::default(),
                parser_deferred_load_delay_token(33),
            )
        else {
            panic!("module defer should return a graph-start action");
        };
        let (pending_script_id, start_script) = start.into_parts();

        assert_eq!(start_script.node_id, script.node_id);
        assert_eq!(pending_script_id.owner(), owner);
        assert_eq!(
            pending_script_id.key(),
            ParserPendingScriptKey::from_script(&script)
        );
        assert!(
            store.has_module_script(pending_script_id),
            "graph start must be returned only after module PendingScript registration"
        );
        assert!(
            !store.module_script_is_watching_for_test(pending_script_id),
            "parser acceptance must not install a queue-head watcher before EOF"
        );
        assert!(
            store.has_load_blocking_document_script_work(owner),
            "graph start must be returned only after parser order registration"
        );
        assert!(
            !store.has_after_parsing_script(owner),
            "accepted module-defer cannot execute before EOF"
        );

        assert!(
            !store.notify_module_script_graph_ready_work(main_like_graph_ready_work(
                MainLikeModuleReadyTarget { owner, route: 1 },
                script.clone(),
                vec![ModuleEntryId::from_raw(33)],
            )),
            "an EOF-early terminal must remain on the parser-owned PendingScript"
        );
        assert!(
            !store.has_ready_work(),
            "an EOF-early parser module terminal must not escape into the broad ready lane"
        );
        assert!(
            !store.next_after_parsing_script_is_ready(owner),
            "a ready graph cannot execute before parser EOF"
        );

        assert_eq!(store.seal_parser_deferred_scripts(owner), Ok(1));
        assert!(
            store.next_after_parsing_script_is_ready(owner),
            "EOF seal should expose the terminal retained by the accepted PendingScript"
        );
        assert!(
            !store.module_script_is_watching_for_test(pending_script_id),
            "an already-ready queue head does not need a load watcher"
        );
        let Some(ParserDeferredScriptReady::Module(ready)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("the retained module terminal should occupy its parser-order slot");
        };
        let (terminal, token) = ready.into_parts();
        let ParserModuleGraphTerminalWork::Ready(work) = terminal;
        assert_eq!(work.script().node_id, script.node_id);
        assert_eq!(token, parser_deferred_load_delay_token(33));
    }

    #[test]
    fn parser_deferred_module_start_failure_cancels_pending_identity_and_token() {
        let mut store: DocumentScriptSchedulerStore<u64> = Default::default();
        let owner = 14;
        let script =
            parser_module_script("https://document-scripts.test/start-failure.mjs", 34, 12);
        let token = parser_deferred_load_delay_token(34);
        let Some(ParserDeferredScriptStartAction::ModuleGraph(start)) = store
            .claim_parser_deferred_script(owner, script, None, None, Default::default(), token)
        else {
            panic!("module defer should return a graph-start action");
        };
        let (pending_script_id, _) = start.into_parts();

        assert_eq!(
            store.cancel_parser_deferred_script(pending_script_id),
            Some(token),
            "start failure must return the exact lifecycle token owned by the PendingScript"
        );
        assert!(!store.has_module_script(pending_script_id));
        assert!(!store.has_load_blocking_document_script_work(owner));
        assert_eq!(
            store.cancel_parser_deferred_script(pending_script_id),
            None,
            "cancellation must consume the PendingScript exactly once"
        );
    }

    #[test]
    fn parser_after_parsing_queue_preserves_mixed_classic_and_module_document_order() {
        let mut store: DocumentScriptSchedulerStore<
            u64,
            MainLikeModuleReadyTarget,
            std::convert::Infallible,
            TestModuleGraphFailure,
        > = DocumentScriptSchedulerStore::default();
        let owner = 10;
        let module = parser_module_script("https://document-scripts.test/first-module.js", 41, 10);
        let classic =
            parser_classic_defer_script("https://document-scripts.test/second-classic.js", 42, 20);
        store.register_module_script(owner, &module);
        store.claim_ready_parser_deferred_script_for_test(
            owner,
            classic.clone(),
            Default::default(),
        );
        store.claim_ready_parser_deferred_script_for_test(
            owner,
            module.clone(),
            Default::default(),
        );
        assert_eq!(
            store.seal_parser_deferred_scripts(owner),
            Ok(2),
            "EOF seal should preserve one parser-owned queue sorted by parser position"
        );
        assert!(
            !store.next_after_parsing_script_is_ready(owner),
            "later ready classic defer must wait for the earlier module graph terminal"
        );

        let module_work = main_like_graph_ready_work(
            MainLikeModuleReadyTarget { owner, route: 1 },
            module.clone(),
            vec![ModuleEntryId::from_raw(410)],
        );
        assert!(
            !store.notify_module_script_graph_ready_work(module_work),
            "after-parsing module terminal stays in the parser queue instead of the broad ready lane"
        );
        assert!(store.next_after_parsing_script_is_ready(owner));

        let Some(ParserDeferredScriptReady::Module(ready)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("earlier module terminal must be released first");
        };
        let (terminal, load_delay_token) = ready.into_parts();
        let ParserModuleGraphTerminalWork::Ready(work) = terminal else {
            panic!("earlier module terminal must be graph-ready work");
        };
        assert_eq!(load_delay_token, parser_deferred_load_delay_token(11));
        assert_eq!(work.script().node_id, module.node_id);

        let Some(ParserDeferredScriptReady::Classic(script)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("later classic defer must follow the earlier module");
        };
        assert_eq!(script.script().node_id, classic.node_id);
        assert!(!store.has_after_parsing_script(owner));
    }

    #[test]
    fn parser_after_parsing_queue_releases_graph_failure_before_later_classic() {
        let mut store: DocumentScriptSchedulerStore<
            u64,
            MainLikeModuleReadyTarget,
            std::convert::Infallible,
            TestModuleGraphFailure,
        > = DocumentScriptSchedulerStore::default();
        let owner = 11;
        let module = parser_module_script("https://document-scripts.test/failed-module.js", 51, 10);
        let classic =
            parser_classic_defer_script("https://document-scripts.test/after-failure.js", 52, 20);
        store.register_module_script(owner, &module);
        store.claim_ready_parser_deferred_script_for_test(
            owner,
            module.clone(),
            Default::default(),
        );
        store.claim_ready_parser_deferred_script_for_test(
            owner,
            classic.clone(),
            Default::default(),
        );
        assert_eq!(store.seal_parser_deferred_scripts(owner), Ok(2));

        assert!(
            !store.notify_module_script_graph_failed_action(TestModuleGraphFailure::new(
                owner,
                &module,
                "graph failed",
            )),
            "ordered graph failure must not enter the broad ready lane"
        );
        let Some(ParserDeferredScriptReady::Module(ready)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("module graph failure must occupy its parser document-order slot");
        };
        let (terminal, load_delay_token) = ready.into_parts();
        let ParserModuleGraphTerminalWork::Failed(failure) = terminal else {
            panic!("module graph failure must remain typed");
        };
        assert_eq!(load_delay_token, parser_deferred_load_delay_token(11));
        assert_eq!(failure.message, "graph failed");

        let Some(ParserDeferredScriptReady::Classic(script)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("later classic defer must run after the failed module slot");
        };
        assert_eq!(script.script().node_id, classic.node_id);
    }

    #[test]
    fn parser_eof_seal_fails_closed_without_module_pending_script() {
        let mut store: DocumentScriptSchedulerStore<u64, MainLikeModuleReadyTarget> =
            DocumentScriptSchedulerStore::default();
        let owner = 12;
        let classic = parser_classic_defer_script(
            "https://document-scripts.test/retained-classic.js",
            61,
            10,
        );
        let missing_module =
            parser_module_script("https://document-scripts.test/missing-module.js", 62, 20);

        store.claim_ready_parser_deferred_script_for_test(owner, classic, Default::default());
        let Some(ParserDeferredScriptStartAction::ModuleGraph(start)) = store
            .claim_parser_deferred_script(
                owner,
                missing_module.clone(),
                None,
                None,
                Default::default(),
                parser_deferred_load_delay_token(62),
            )
        else {
            panic!("module defer should atomically register before graph start");
        };
        let (pending_script_id, _) = start.into_parts();
        assert!(
            store.discard_module_script(pending_script_id),
            "test setup must remove the accepted module PendingScript"
        );
        assert_eq!(
            store.seal_parser_deferred_scripts(owner),
            Err(ParserPendingScriptKey::from_script(&missing_module)),
            "a module without its preparation-time PendingScript must reject EOF sealing"
        );
        assert!(
            !store.has_after_parsing_script(owner),
            "failed installation must not retain a partial classic-only queue"
        );
    }

    #[test]
    fn parser_deferred_acceptance_keeps_one_queue_until_eof_seal() {
        let mut store: DocumentScriptSchedulerStore<u64, MainLikeModuleReadyTarget> =
            DocumentScriptSchedulerStore::default();
        let owner = 13;
        let first =
            parser_classic_defer_script("https://document-scripts.test/first-batch.js", 71, 10);
        let second =
            parser_classic_defer_script("https://document-scripts.test/second-batch.js", 72, 20);

        store.claim_ready_parser_deferred_script_for_test(owner, first.clone(), Default::default());
        store.claim_ready_parser_deferred_script_for_test(
            owner,
            second.clone(),
            Default::default(),
        );
        assert!(
            !store.has_after_parsing_script(owner),
            "accepted PendingScripts cannot execute before parser EOF"
        );
        assert_eq!(store.seal_parser_deferred_scripts(owner), Ok(2));

        let Some(ParserDeferredScriptReady::Classic(script)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("first batch should retain document order");
        };
        assert_eq!(script.script().node_id, first.node_id);
        let Some(ParserDeferredScriptReady::Classic(script)) =
            store.take_next_after_parsing_ready_script(owner)
        else {
            panic!("second batch should follow the first");
        };
        assert_eq!(script.script().node_id, second.node_id);
    }
}
