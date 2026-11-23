use crate::page_task_queue::{
    HtmlFragmentModuleTaskTarget, HtmlFragmentScriptTask, HtmlFragmentScriptTaskKind,
    PageDomManipulationTurnAction, PageDomManipulationTurnOutcome, RendererPageDomManipulationTask,
};

use super::PageVm;

impl PageVm {
    pub(in crate::runtime) async fn apply_selected_page_dom_manipulation_turn(
        &mut self,
        task: RendererPageDomManipulationTask,
    ) -> anyhow::Result<PageDomManipulationTurnOutcome> {
        match task {
            RendererPageDomManipulationTask::ChildHostLoad(task) => Ok(self
                .apply_selected_page_child_host_load_turn(task)
                .map_action(PageDomManipulationTurnAction::ChildHostLoad)),
            RendererPageDomManipulationTask::ChildDocumentLifecycle(task) => Ok(self
                .apply_selected_page_child_document_lifecycle_turn(task)
                .map_action(PageDomManipulationTurnAction::ChildDocumentLifecycle)),
            RendererPageDomManipulationTask::MainDocumentLifecycle(task) => self
                .apply_selected_page_main_document_lifecycle_turn(task)
                .await
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::MainDocumentLifecycle)
                }),
            RendererPageDomManipulationTask::BroadcastChannel(task) => self
                .apply_selected_page_broadcast_channel_delivery_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::BroadcastChannel)),
            RendererPageDomManipulationTask::StorageEvent(task) => self
                .apply_selected_page_storage_event_delivery_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::StorageEvent)),
            RendererPageDomManipulationTask::HashChange(task) => self
                .apply_selected_page_hash_change_delivery_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::HashChange)),
            RendererPageDomManipulationTask::ElementToggle(task) => self
                .apply_selected_page_element_toggle_event_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::ElementToggle)),
            RendererPageDomManipulationTask::FileEntryFileCallback(task) => self
                .apply_selected_page_file_entry_file_callback_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::FileEntryFileCallback)
                }),
            RendererPageDomManipulationTask::HtmlFragmentScript(task) => {
                let dispatched =
                    if task.owner.root_document() == self.document_lifecycle.identity().document {
                        self.apply_selected_html_fragment_script_turn(task).await?
                    } else {
                        false
                    };
                Ok(PageDomManipulationTurnOutcome::new(
                    PageDomManipulationTurnAction::HtmlFragmentScript { dispatched },
                ))
            }
            RendererPageDomManipulationTask::ServiceWorkerQuery(task) => self
                .apply_selected_page_service_worker_query_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::ServiceWorkerQuery)
                }),
            RendererPageDomManipulationTask::ScriptPreparationError(task) => self
                .apply_selected_page_script_preparation_error_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::ScriptPreparationError)
                }),
            RendererPageDomManipulationTask::ObjectResourceSelection(task) => self
                .apply_selected_page_object_resource_selection_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::ObjectResourceSelection)
                }),
            RendererPageDomManipulationTask::PromiseRejection(task) => self
                .apply_selected_page_promise_rejection_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::PromiseRejection)),
            RendererPageDomManipulationTask::ImageLoadEvent(task) => self
                .apply_selected_page_image_load_event_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::ImageLoadEvent)),
            RendererPageDomManipulationTask::PopupDocumentLifecycle(task) => self
                .apply_selected_page_popup_document_lifecycle_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::PopupDocumentLifecycle)
                }),
            RendererPageDomManipulationTask::PopupClose(task) => self
                .apply_selected_page_popup_close_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::PopupClose)),
            RendererPageDomManipulationTask::ConnectedStyleEvent(task) => self
                .apply_selected_page_connected_style_event_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::ConnectedStyleEvent)
                }),
            RendererPageDomManipulationTask::FormNavigation(task) => self
                .apply_selected_page_form_navigation_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::FormNavigation)),
            RendererPageDomManipulationTask::TextTrackDefaultMode(task) => self
                .apply_selected_page_text_track_default_mode_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::TextTrackDefaultMode)
                }),
            RendererPageDomManipulationTask::TextTrackLoad(task) => self
                .apply_selected_page_text_track_load_turn(task)
                .map(|outcome| outcome.map_action(PageDomManipulationTurnAction::TextTrackLoad)),
            RendererPageDomManipulationTask::ViewTransitionUpdate(task) => self
                .apply_selected_page_view_transition_update_turn(task)
                .map(|outcome| {
                    outcome.map_action(PageDomManipulationTurnAction::ViewTransitionUpdate)
                }),
        }
    }

    async fn apply_selected_html_fragment_script_turn(
        &mut self,
        task: HtmlFragmentScriptTask,
    ) -> anyhow::Result<bool> {
        match task.kind {
            HtmlFragmentScriptTaskKind::StartModule { job } => self
                .vm_mut()
                .start_html_fragment_module(HtmlFragmentModuleTaskTarget {
                    document: task.owner.target(),
                    stream: task.stream,
                    job,
                }),
            HtmlFragmentScriptTaskKind::RunModule { job } => {
                self.run_selected_html_fragment_module(HtmlFragmentModuleTaskTarget {
                    document: task.owner.target(),
                    stream: task.stream,
                    job,
                })
                .await
            }
            _ => self.vm_mut().apply_html_fragment_script_task(task),
        }
    }

    async fn run_selected_html_fragment_module(
        &mut self,
        route: HtmlFragmentModuleTaskTarget,
    ) -> anyhow::Result<bool> {
        use crate::native_bridge::{HtmlFragmentModuleTerminal, WindowDocumentOwner};
        let Some(terminal) = self.vm_mut().take_html_fragment_module_terminal(route)? else {
            return Ok(false);
        };
        let mut failure = None;
        let effect = match terminal {
            HtmlFragmentModuleTerminal::Main(work) => {
                let WindowDocumentOwner::Frame(owner) = route.document.owner() else {
                    unreachable!("main fragment graph retains a frame owner");
                };
                let Some(loader) = self
                    .vm()
                    .document_runtime
                    .current_document_resource_loader()
                else {
                    self.vm_mut()
                        .finish_html_fragment_module_turn(route, None)?;
                    return Ok(false);
                };
                super::parser_owned_document_script::MainParserOwnedDocumentScriptOwner::new(
                    self,
                    loader.request_client(),
                )
                .run_html_fragment_module_terminal(owner, *work)
                .await
                .map(|effect| {
                    !matches!(
                        effect,
                        super::parser_task_completion::MainParserContinuationTaskEffect::NotApplied
                    )
                })
            }
            HtmlFragmentModuleTerminal::Child(work) => Ok(self
                .vm_mut()
                .run_child_html_fragment_module_terminal(*work)
                .await),
            HtmlFragmentModuleTerminal::Unavailable(node) => {
                failure = Some(node);
                Ok(true)
            }
        };
        self.vm_mut()
            .finish_html_fragment_module_turn(route, failure)?;
        effect
    }
}
