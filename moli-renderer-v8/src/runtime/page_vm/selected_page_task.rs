use anyhow::Result;

use crate::page_task_queue::RendererPageSchedulerTask;

use super::{IntoPageTaskCompletion, PageVm, page_timer::PageTimerTurnAction};

impl PageVm {
    /// Apply one concrete task that has already been admitted and removed from
    /// its stable Page source.
    ///
    /// This is the single dispatch boundary shared by the production
    /// owner-loop and low-level PageVm executor fixtures. Source selection,
    /// scheduler fairness, lifecycle reconciliation, and restoring the Page
    /// residence remain responsibilities of the caller; the family-specific
    /// executor and its exact-owner authorization live here.
    pub(in crate::runtime) async fn apply_selected_page_scheduler_task(
        &mut self,
        task: RendererPageSchedulerTask,
        loader: &crate::network::ResourceRequestClient,
    ) -> Result<()> {
        match task {
            RendererPageSchedulerTask::ActionWindow { deadline } => {
                self.apply_selected_page_action_window_turn(deadline)
            }
            RendererPageSchedulerTask::DomManipulation(task) => {
                let outcome = self.apply_selected_page_dom_manipulation_turn(task)?;
                self.finish_selected_page_dom_manipulation_task(outcome.action, loader)
                    .await?;
                Ok(())
            }
            RendererPageSchedulerTask::UserInteraction(task) => {
                let outcome = self.apply_selected_page_user_interaction_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::FileReading(task) => {
                let outcome = self.apply_selected_page_file_reading_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::MiscPlatformApi(task) => {
                let outcome = self.apply_selected_page_misc_platform_api_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::NavigationAndTraversal(task) => {
                let outcome = self.apply_selected_page_navigation_and_traversal_turn(task)?;
                match outcome.action {
                    crate::page_task_queue::PageNavigationAndTraversalTurnAction::HistoryTraversal(
                        action,
                    ) => {
                        self.finish_selected_page_task_completion(
                            action.into_page_task_completion(),
                            loader,
                        )
                        .await?;
                    }
                    crate::page_task_queue::PageNavigationAndTraversalTurnAction::NavigationApi(
                        action,
                    ) => {
                        self.finish_selected_page_task_completion(
                            action.into_page_task_completion(),
                            loader,
                        )
                        .await?;
                    }
                    crate::page_task_queue::PageNavigationAndTraversalTurnAction::ChildNavigationCommit(
                        action,
                    ) => {
                        self.finish_selected_page_task_completion(
                            action.into_page_task_completion(),
                            loader,
                        )
                        .await?;
                    }
                }
                Ok(())
            }
            RendererPageSchedulerTask::RenderingUpdate(task) => {
                let outcome = self.apply_selected_page_rendering_update_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::MediaElementEvent(task) => {
                let outcome = self.apply_selected_page_media_element_event_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::DedicatedWorkerClientEvent(task) => {
                let outcome = self.apply_selected_page_dedicated_worker_client_event_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::SharedWorkerClientEvent(task) => {
                let outcome = self.apply_selected_page_shared_worker_client_event_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::ServiceWorkerInternal(task) => {
                let outcome = self.apply_selected_page_service_worker_internal_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::ServiceWorkerClientMessage(task) => {
                let outcome = self.apply_selected_page_service_worker_client_message_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::WebCryptoTask(task) => {
                let outcome = self.apply_selected_page_webcrypto_task_turn(task)?;
                if outcome.action.settled_current_owner() {
                    self.finish_selected_page_task_checkpoint()?;
                }
                Ok(())
            }
            RendererPageSchedulerTask::IndexedDbTask(task) => {
                let outcome = self.apply_selected_page_indexed_db_task_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::OpfsTask(task) => {
                let outcome = self.apply_selected_page_opfs_task_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::InternalLoading(task) => {
                let outcome = self.apply_selected_page_internal_loading_turn(task);
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::MainDocumentRuntime(task) => {
                let outcome = self
                    .apply_selected_page_main_document_runtime_turn(task, loader)
                    .await?;
                let action = outcome.action;
                let completion = match action {
                    crate::page_task_queue::PageMainDocumentRuntimeTurnAction::RuntimeScriptAdmission(
                        action,
                    ) => Some(action.into_page_task_completion()),
                    crate::page_task_queue::PageMainDocumentRuntimeTurnAction::ParserAsyncModuleAdmission(
                        action,
                    ) => Some(action.into_page_task_completion()),
                    crate::page_task_queue::PageMainDocumentRuntimeTurnAction::RuntimeScriptContinuation(
                        action,
                    ) => Some(action.into_page_task_completion()),
                    crate::page_task_queue::PageMainDocumentRuntimeTurnAction::RuntimeOwnedModuleContinuation(
                        action,
                    ) => Some(action.into_page_task_completion()),
                    crate::page_task_queue::PageMainDocumentRuntimeTurnAction::ParserOwnedModuleContinuation(
                        action,
                    ) => {
                        self.finish_selected_page_parser_owned_module_continuation(action)?;
                        None
                    }
                    crate::page_task_queue::PageMainDocumentRuntimeTurnAction::DynamicModuleJob(
                        action,
                    ) => {
                        self.finish_selected_page_dynamic_module_job(action, loader)
                            .await?;
                        None
                    }
                    crate::page_task_queue::PageMainDocumentRuntimeTurnAction::NativeModuleOwnerEvent(
                        action,
                    ) => {
                        self.finish_selected_page_native_module_owner_event(action, loader)
                            .await?;
                        None
                    }
                    crate::page_task_queue::PageMainDocumentRuntimeTurnAction::PostParseWork(
                        action,
                    ) => {
                        if let Some((owner, execution)) = action.into_execution() {
                            tracing::debug!(
                                ?owner,
                                kind = execution.kind(),
                                "submitting selected post-parse task completion"
                            );
                            self.finish_main_document_post_parse_execution(execution)?;
                        }
                        None
                    }
                };
                if let Some(completion) = completion {
                    self.finish_selected_page_task_completion(completion, loader)
                        .await?;
                }
                Ok(())
            }
            RendererPageSchedulerTask::ChildModuleDependencyFetchStart(task) => {
                let outcome =
                    self.apply_selected_page_child_module_dependency_fetch_start_turn(*task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::ChildModuleScriptTerminal(task) => {
                let outcome = self.apply_selected_page_child_module_script_terminal_turn(task);
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::ChildModulepreloadEventAction(task) => {
                let outcome = self.apply_selected_page_child_modulepreload_event_action_turn(task);
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::ChildFrameTask(task) => {
                self.apply_selected_page_child_frame_task_turn(task, loader)
                    .await
            }
            RendererPageSchedulerTask::V8ForegroundTask(task) => {
                let outcome = self.apply_selected_page_v8_foreground_task_turn(task)?;
                if outcome.action.entered_isolate() {
                    self.finish_selected_page_task_checkpoint()?;
                }
                Ok(())
            }
            RendererPageSchedulerTask::ModuleReaction(task) => {
                let outcome = self.apply_selected_page_module_reaction_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::WindowMessage(task) => {
                let outcome = self.apply_selected_page_window_message_turn(task)?;
                match outcome.action.target_effect {
                    crate::page_task_queue::PageWindowMessageTargetEffect::AppliedToCurrentOwner => {
                        self.finish_selected_page_callback_task(loader).await?;
                    }
                    crate::page_task_queue::PageWindowMessageTargetEffect::CurrentOwnerHadNoPendingMessage => {
                        // The current exact task entered its Window context,
                        // but its local payload had already been consumed. The
                        // old helper still performed the task checkpoint, but
                        // had no callback follow-up to reconcile.
                        self.finish_selected_page_task_checkpoint()?;
                    }
                    crate::page_task_queue::PageWindowMessageTargetEffect::DiscardedStaleOwner {
                        ..
                    } => {}
                }
                Ok(())
            }
            RendererPageSchedulerTask::MessagePortDelivery {
                task,
                same_attachment_task_is_ready,
            } => {
                let outcome = self.apply_selected_page_message_port_delivery_turn(
                    task,
                    same_attachment_task_is_ready,
                )?;
                match outcome.action.target_effect {
                    crate::page_task_queue::PageMessagePortDeliveryTargetEffect::ConsumedByCurrentOwner {
                        ..
                    } => {
                        self.finish_selected_page_callback_task(loader).await?;
                    }
                    crate::page_task_queue::PageMessagePortDeliveryTargetEffect::CurrentOwnerHadNoReadyEvent => {
                        // The old context helper still checkpointed an exact
                        // current attachment even when its registry event was
                        // not yet dispatchable, but no callback follow-up ran.
                        self.finish_selected_page_task_checkpoint()?;
                    }
                    crate::page_task_queue::PageMessagePortDeliveryTargetEffect::IgnoredStaleOwner {
                        ..
                    } => {}
                }
                Ok(())
            }
            RendererPageSchedulerTask::DynamicImportOwnerAction(task) => {
                let outcome = self.apply_selected_page_dynamic_import_owner_action_turn(task);
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::ModulepreloadStart(task) => {
                let outcome = self.apply_selected_page_modulepreload_start_turn(task);
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::Networking(task) => {
                let outcome = self.apply_selected_page_networking_turn(task)?;
                self.finish_selected_page_networking_task(outcome.action, loader)
                    .await?;
                Ok(())
            }
            RendererPageSchedulerTask::WebSocket(task) => {
                let outcome = self.apply_selected_page_websocket_turn(task)?;
                self.finish_selected_page_task_completion(
                    outcome.action.into_page_task_completion(),
                    loader,
                )
                .await?;
                Ok(())
            }
            RendererPageSchedulerTask::Timer { deadline } => {
                let outcome = self.apply_selected_page_timer_turn(deadline)?;
                if matches!(outcome.action, PageTimerTurnAction::Consumed { .. }) {
                    self.finish_selected_page_callback_task(loader).await?;
                }
                Ok(())
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn apply_selected_page_scheduler_task_on_owner_lane_for_test(
        &mut self,
        task: RendererPageSchedulerTask,
        loader: crate::network::ResourceRequestClient,
    ) -> Result<()> {
        let local_executor = self.local_executor.clone();
        let mut page_vm_ref = super::AwaitedOwnerLocalPageVm::new(self);
        super::run_named_owner_local_task(
            local_executor,
            "selected Page task test executor local task channel closed",
            async move {
                page_vm_ref
                    .get_mut()
                    .apply_selected_page_scheduler_task(task, &loader)
                    .await
            },
        )
        .await
    }
}
