use super::parser_blocking_owner::MainParserBlockingSourceLoadWaitOwner;
use super::*;
use crate::document_script_scheduler::ParseTimeDocumentScriptEvent;

pub(super) struct ParseTimeDriverState {
    pub(super) final_url: Url,
    pub(super) document_character_set: String,
    pub(super) parser_session: DocumentParserSession,
    pub(super) scheduler: DocumentScriptScheduler,
    pub(super) pending_parsing_blocking_script: PendingParsingBlockingClassicScriptRunner,
    pub(super) buffered_document_preloads: Box<BufferedDocumentPreloadState>,
    pub(super) service_worker_preload_context: Option<ServiceWorkerScriptPreloadContext>,
    pub(super) input_closed: bool,
}

impl ParseTimeDriverState {
    #[cfg(test)]
    pub(super) fn new_with_scripting_enabled_for_test(final_url: Url) -> Self {
        Self::new_with_scripting_enabled(final_url, true)
    }

    pub(super) fn new_with_scripting_enabled(final_url: Url, scripting_enabled: bool) -> Self {
        Self {
            parser_session: DocumentParserSession::start_main_document(
                final_url.clone(),
                scripting_enabled,
            ),
            final_url,
            document_character_set: "UTF-8".to_owned(),
            scheduler: DocumentScriptScheduler::new(),
            pending_parsing_blocking_script: PendingParsingBlockingClassicScriptRunner::empty(),
            buffered_document_preloads: Box::default(),
            service_worker_preload_context: None,
            input_closed: false,
        }
    }

    pub(super) fn new_xml(final_url: Url) -> Self {
        Self {
            parser_session: DocumentParserSession::start_main_xml_document(final_url.clone()),
            final_url,
            document_character_set: "UTF-8".to_owned(),
            scheduler: DocumentScriptScheduler::new(),
            pending_parsing_blocking_script: PendingParsingBlockingClassicScriptRunner::empty(),
            buffered_document_preloads: Box::default(),
            service_worker_preload_context: None,
            input_closed: false,
        }
    }

    pub(super) fn close_input(&mut self) {
        if self.input_closed {
            return;
        }
        self.parser_session.request_finish();
        self.input_closed = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseTimeOwner {
    Parser,
    Document,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingParsingBlockingWait {
    None,
    LegacyDocumentProcessing,
    PageTaskBlockingStylesheet,
    PageNetworkingDocumentWriteExternalScript,
}

impl PendingParsingBlockingWait {
    pub(super) const fn is_pending(self) -> bool {
        !matches!(self, Self::None)
    }

    pub(super) const fn waits_for_legacy_document_processing(self) -> bool {
        matches!(self, Self::LegacyDocumentProcessing)
    }

    pub(super) const fn waits_for_page_task(self) -> bool {
        matches!(
            self,
            Self::PageTaskBlockingStylesheet | Self::PageNetworkingDocumentWriteExternalScript
        )
    }
}

pub(in crate::runtime) struct ConcurrentParseTimeRuntime {
    pub(super) loader: ResourceRequestClient,
    pub(super) stage: PageVmInitStage,
    pub(super) state: ParseTimeDriverState,
    pub(super) page_vm: PageVm,
    pub(super) parser_document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    pub(super) owner: ParseTimeOwner,
    pub(super) parser_step_ready: bool,
    pub(super) pending_parsing_blocking_wait: PendingParsingBlockingWait,
}

impl ConcurrentParseTimeRuntime {
    pub(in crate::runtime) fn publish_processing_main_document_phase(&self) {
        self.page_vm.runtime_hooks.set_page_creation_phase(
            crate::runtime::RendererPageCreationPhase::ProcessingMainDocument,
        );
    }

    pub(in crate::runtime) fn publish_pending_page_creation_phase(&self) {
        let phase = match self.pending_parsing_blocking_wait {
            PendingParsingBlockingWait::PageTaskBlockingStylesheet => {
                crate::runtime::RendererPageCreationPhase::WaitingForParserBlockingStylesheet
            }
            PendingParsingBlockingWait::LegacyDocumentProcessing
            | PendingParsingBlockingWait::PageNetworkingDocumentWriteExternalScript => {
                crate::runtime::RendererPageCreationPhase::WaitingForParserBlockingScript
            }
            PendingParsingBlockingWait::None
                if self.has_unready_pending_parser_blocking_source_load() =>
            {
                crate::runtime::RendererPageCreationPhase::WaitingForParserBlockingScript
            }
            PendingParsingBlockingWait::None if !self.state.input_closed => {
                crate::runtime::RendererPageCreationPhase::StreamingMainBody
            }
            PendingParsingBlockingWait::None => {
                crate::runtime::RendererPageCreationPhase::ProcessingMainDocument
            }
        };
        self.page_vm.runtime_hooks.set_page_creation_phase(phase);
    }

    pub(super) fn publish_target_lifecycle_phase(&self) {
        let phase = match self.stage {
            PageVmInitStage::DomContentLoaded => {
                crate::runtime::RendererPageCreationPhase::WaitingForDomContentLoaded
            }
            PageVmInitStage::Load => crate::runtime::RendererPageCreationPhase::WaitingForLoad,
        };
        self.page_vm.runtime_hooks.set_page_creation_phase(phase);
    }

    pub(in crate::runtime) fn page_vm(&self) -> &PageVm {
        &self.page_vm
    }

    pub(in crate::runtime) fn page_vm_mut(&mut self) -> &mut PageVm {
        &mut self.page_vm
    }

    pub(super) fn retire_main_parser_continuation(
        &mut self,
    ) -> (
        Vec<ParseTimeDocumentScriptEvent>,
        Vec<PostParsePageOwnedWork>,
    ) {
        self.page_vm
            .vm_mut()
            .document_runtime
            .deactivate_main_parser_continuation();
        (
            self.page_vm
                .page_task_queue
                .take_parse_time_document_script_events(),
            self.page_vm
                .page_task_queue
                .take_parse_time_lifecycle_work(),
        )
    }

    /// Stop a committed parser after its main-resource body reaches a failed
    /// terminal while retaining the partial Document as the active Page.
    ///
    /// Blink's `DocumentLoader::LoadFailed` stops parsing and reports a failed
    /// navigation; it does not destroy the committed LocalFrame. Mirror that
    /// ownership transition here: parser-only state is discarded, while the
    /// PageVm and its observable DOM remain resident for automation and a
    /// subsequent navigation.
    pub(super) fn into_main_resource_load_failed_page_vm(mut self) -> PageVm {
        self.state
            .parser_session
            .stop(ParserStopReason::MainResourceLoadFailure);
        drop(self.retire_main_parser_continuation());
        let _ = self.page_vm.document_lifecycle.request_termination(
            self.page_vm.document_lifecycle.identity(),
            RendererDocumentTerminationReason::MainResourceLoadFailed,
        );
        self.page_vm
    }

    /// Leave phase one after script requested a top-level navigation.
    ///
    /// The current `PageVm` stays live while the navigation is fetched, but
    /// the phase-one driver is consumed by this transition. Retire its parser
    /// continuation before returning the Page so an already-queued Networking
    /// task cannot grant an admission to a driver that no longer exists.
    pub(super) fn into_navigation_triggered_page_vm(mut self) -> PageVm {
        drop(self.retire_main_parser_continuation());
        self.page_vm
    }

    pub(super) fn new_parser_owner(
        loader: ResourceRequestClient,
        stage: PageVmInitStage,
        state: ParseTimeDriverState,
        mut page_vm: PageVm,
    ) -> Self {
        page_vm.set_target_stage(stage);
        page_vm
            .vm_mut()
            .document_runtime
            .bind_main_document_script_preload_store(
                state
                    .buffered_document_preloads
                    .document_script_preload_store(),
            );
        let parser_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("phase-one parser runtime requires an installed main document owner");
        page_vm
            .vm_mut()
            .document_runtime
            .activate_main_parser_continuation(parser_document_owner);
        Self {
            loader,
            stage,
            state,
            page_vm,
            parser_document_owner,
            owner: ParseTimeOwner::Parser,
            parser_step_ready: false,
            pending_parsing_blocking_wait: PendingParsingBlockingWait::None,
        }
    }

    pub(super) fn has_pending_parser_blocking_source_load(&self) -> bool {
        self.pending_parser_blocking_source_load().is_some()
    }

    /// Consume the permission granted by one selected Networking continuation.
    ///
    /// The task never carries parser state. It merely lets this sole runtime
    /// re-enter the parser after the producer has committed input/resource
    /// state in its authoritative store.
    pub(super) fn admit_selected_main_parser_continuation(&mut self) -> bool {
        if !self
            .page_vm
            .vm_mut()
            .document_runtime
            .take_main_parser_continuation_admission()
        {
            return false;
        }
        let _ = self
            .state
            .parser_session
            .admit_delayed_finish_continuation();
        self.owner = ParseTimeOwner::Parser;
        self.parser_step_ready = true;
        self.pending_parsing_blocking_wait = PendingParsingBlockingWait::None;
        true
    }

    pub(super) fn pending_parser_blocking_source_load(
        &self,
    ) -> Option<crate::planning::SharedScriptSourceLoad> {
        let mut owner = MainParserBlockingSourceLoadWaitOwner;
        self.state
            .pending_parsing_blocking_script
            .current_parser_blocking_source_load_wait_action_with_owner(&mut owner)
    }

    pub(super) fn has_unready_pending_parser_blocking_source_load(&self) -> bool {
        self.pending_parser_blocking_source_load()
            .is_some_and(|load| load.try_outcome().is_none())
    }

    pub(super) async fn run_one_page_creation_event_loop_turn(
        &mut self,
    ) -> Result<PageTaskTurnResult> {
        let ConcurrentParseTimeRuntime { state, page_vm, .. } = self;
        let mut context = DocumentTurnContext {
            scheduler: &mut state.scheduler,
            parser_session: &state.parser_session,
        };
        context.run_parse_time_turn(page_vm).await
    }

    pub(in crate::runtime) fn owner_wake_token(&self) -> Option<crate::runtime::RendererPageToken> {
        self.page_vm
            .runtime_hooks
            .owner_wake()
            .map(|owner_wake| owner_wake.token())
    }
}
