use super::*;

impl RendererOwnerHandle {
    pub(super) fn checkout_live_page_entry_for_cancellation(
        &self,
        token: RendererPageToken,
        continuation: &'static str,
    ) -> Option<LivePageEntry> {
        match checkout_entry_for_owner_turn_on_bound_owner_local_store(token) {
            Ok(entry) => Some(entry),
            Err(LivePageEntryCheckoutError::Retired | LivePageEntryCheckoutError::Missing) => None,
            Err(LivePageEntryCheckoutError::Busy) => {
                panic!(
                    "renderer page {} remained checked out while cancelling {continuation}",
                    token.page_id.as_u64()
                )
            }
        }
    }

    pub(super) fn detach_navigation_or_cancel_pending_turn(
        &self,
        turn: RenderRuntimeTurn,
        pending_turns: &mut RenderRuntimePendingTurnQueue,
    ) {
        let (turn, detached) = turn.detach_navigation_command_observer();
        if detached {
            pending_turns.push_back(RenderRuntimePendingTurn {
                reply_tx: None,
                turn,
                allow_command_overtake: true,
                command_admission_output_predecessor: None,
            });
        } else {
            self.cancel_pending_turn_on_owner_local_store(turn);
        }
    }

    pub(super) fn cancel_pending_turn_on_owner_local_store(&self, turn: RenderRuntimeTurn) {
        match turn {
            RenderRuntimeTurn::WaitLivePageScriptTruthy {
                token,
                pending_call: Some(pending_call),
                ..
            }
            | RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                token,
                pending_call: Some(pending_call),
                ..
            } => {
                if let Some(mut entry) = self
                    .checkout_live_page_entry_for_cancellation(token, "pending runtime evaluation")
                {
                    entry
                        .page_vm_mut()
                        .cancel_pending_runtime_evaluate(Some(pending_call));
                    self.restore_live_page_entry(token, entry);
                }
            }
            RenderRuntimeTurn::ContinueAttachedPageCreationLifecycle { pending, .. } => {
                remove_page_on_bound_owner_local_store(pending.token);
            }
            RenderRuntimeTurn::WaitLifecycleNavigation(wait) => {
                remove_page_on_bound_owner_local_store(wait.token());
            }
            RenderRuntimeTurn::ContinueLivePageRuntimeCommandLifecycle {
                token, scope_id, ..
            } => {
                if let Some(mut entry) = self
                    .checkout_live_page_entry_for_cancellation(token, "runtime command lifecycle")
                {
                    entry
                        .page_vm_mut()
                        .abandon_pending_runtime_command_lifecycle(scope_id);
                    self.restore_live_page_entry(token, entry);
                }
            }
            RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                token,
                vm_creation_id,
                ..
            } => {
                if let Some(entry) = self.checkout_live_page_entry_for_cancellation(
                    token,
                    "phase-one location navigation",
                ) {
                    if entry.page_vm().creation_id != vm_creation_id {
                        self.restore_live_page_entry(token, entry);
                        return;
                    }
                    self.retire_pending_phase_one_page_entry(
                        token,
                        entry,
                        "Location navigation was cancelled.",
                    );
                }
            }
            RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                token,
                document,
                ..
            } => {
                if let Some(entry) =
                    self.checkout_live_page_entry_for_cancellation(token, "post-parse navigation")
                {
                    let current = entry.active_page_vm().map(|page_vm| {
                        RendererDocumentLifecycleIdentity::from(
                            page_vm.document_lifecycle.current_snapshot(),
                        )
                    });
                    if current == Some(document) {
                        remove_page_on_bound_owner_local_store(token);
                    }
                    self.restore_live_page_entry(token, entry);
                }
            }
            RenderRuntimeTurn::FinishHtmlCreatePage { .. }
            | RenderRuntimeTurn::DrainSharedWorkerServiceLane
            | RenderRuntimeTurn::DrainServiceWorkerServiceLane
            | RenderRuntimeTurn::RunPageTurn { .. }
            | RenderRuntimeTurn::RunOwnerMaintenance { .. }
            | RenderRuntimeTurn::RunInspectorMainReceiver { .. }
            | RenderRuntimeTurn::RunDevToolsMainCommand { .. }
            | RenderRuntimeTurn::RunLivePageCommand { .. }
            | RenderRuntimeTurn::ResumeLivePageDocumentLifecycleAfterReply { .. }
            | RenderRuntimeTurn::WaitLivePageNetworkIdle { .. }
            | RenderRuntimeTurn::WaitLivePageDomStable { .. }
            | RenderRuntimeTurn::WaitLivePageSelector { .. }
            | RenderRuntimeTurn::WaitLivePageScriptTruthy {
                pending_call: None, ..
            }
            | RenderRuntimeTurn::WaitLivePageRuntimeExpressionAwait {
                pending_call: None, ..
            }
            | RenderRuntimeTurn::WaitLivePageSubresourceResponse { .. }
            | RenderRuntimeTurn::WaitLivePageChildFrameLifecycle { .. }
            | RenderRuntimeTurn::ClaimLivePageTopLevelNavigationHandoff { .. } => {}
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                token,
                completion,
                ..
            } => {
                if completion.retires_page_on_navigation_failure() {
                    remove_page_on_bound_owner_local_store(token);
                }
            }
        }
    }

    pub(super) async fn finish_live_page_entry_with_page_state(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
        reply: RendererPageReply,
        command_output_cursor: Option<RendererOutputCursor>,
    ) -> RenderRuntimeDispatchOutcome {
        self.finish_live_page_entry_with_page_state_and_continuation(
            token,
            entry,
            reply,
            command_output_cursor,
            None,
            super::RendererPageStateCapturePolicy::FullReport,
        )
        .await
    }

    pub(super) async fn finish_live_page_entry_with_page_state_and_continuation(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        reply: RendererPageReply,
        command_output_cursor: Option<RendererOutputCursor>,
        post_response_continuation: Option<RendererPageCommandPostResponseContinuation>,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> RenderRuntimeDispatchOutcome {
        let (runtime_command_output, runtime_session_response) =
            entry.page_vm_mut().take_runtime_command_settlement();
        let expected_command_output_cursor = command_output_cursor;
        #[cfg(test)]
        if self
            .state
            .publish_next_command_output_before_settlement
            .swap(false, Ordering::AcqRel)
        {
            assert!(
                expected_command_output_cursor.is_some(),
                "the command output publication test hook requires command-owned records"
            );
            let published = entry
                .page_vm()
                .renderer_page_script_environment()
                .expect("a live Page command must retain its script environment")
                .output_journal()
                .publish_pending();
            assert!(
                published.is_some(),
                "the command output publication test hook must settle a concrete batch"
            );
            entry.page_vm().append_renderer_output_records(vec![
                PendingRendererOutputRecord::observation(
                    None,
                    RendererProtocolObservation::RuntimeLifecycleError {
                        text: "test trailing output publication".to_owned(),
                        execution_context_id: None,
                    },
                ),
            ]);
        }
        let (mut entry, page_state_result) = commit_page_state_on_entry_via_local_task_with_policy(
            self.state.local_executor.clone(),
            entry,
            capture_policy,
        )
        .await;
        let concrete_output = entry.page_vm_mut().settle_renderer_output_publication();
        // Chromium flushes every notification already queued by the current
        // DevTools command before exposing its response. Appending under the
        // journal lock freezes the exact publication cursor that must contain
        // this command's records. An attachment-scoped producer can publish
        // that batch while page-state capture is awaiting its local task, and
        // can even publish later batches before this owner resumes. The
        // response must still fence only the exact command batch, while the
        // published tail proves that batch was not lost.
        //
        // Do not require a Runtime-only causal marker: DOM commands and WebAPI
        // side effects (for example mutation, CSP and Log records) are
        // produced during this owner turn but do not all carry a
        // `RendererRuntimeCommandCausalIdentity`.
        let renderer_output_cursor = match expected_command_output_cursor {
            Some(expected) => {
                let published_tail = entry.page_vm().renderer_output_tail_cursor();
                assert!(
                    published_tail.is_some_and(|cursor| {
                        cursor.stream() == expected.stream()
                            && cursor.sequence() >= expected.sequence()
                    }),
                    "renderer command records must settle into the concrete Page stream before completion"
                );
                Some(expected)
            }
            None => concrete_output
                .as_ref()
                .map(RendererOutputPublication::cursor),
        };
        let renderer_output_predecessor = renderer_output_cursor
            .map(|cursor| entry.page_vm().declare_renderer_output_fence(cursor));
        self.restore_live_page_entry(token, entry);
        if let Some(output) = concrete_output {
            self.publish_renderer_output(output);
        }
        let completion = match page_state_result {
            Ok(page_state) => match RendererCommandTurnOutput::new(
                reply,
                page_state,
                runtime_command_output,
                post_response_continuation,
                renderer_output_predecessor.clone(),
            ) {
                Ok(output) => Ok(output),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        match (completion, runtime_session_response) {
            (Ok(mut output), Some(publication)) => {
                match publication.commit(renderer_output_predecessor) {
                    Ok(Some(settlement)) => {
                        let (response_predecessor, response_succeeded) = settlement.into_parts();
                        output.merge_renderer_output_predecessor(response_predecessor);
                        Ok(RendererOwnerReply::RuntimeInspectorSessionResponseSettled {
                            output: Box::new(output),
                            response_succeeded,
                        })
                        .into()
                    }
                    Ok(None) => unreachable!(
                        "a DevTools session publication must return its concrete response fence"
                    ),
                    Err(completion) => Err(anyhow!(
                        "Runtime response output closed before call {} was published",
                        completion.call_id
                    ))
                    .into(),
                }
            }
            (Err(error), Some(publication)) => {
                match publication.commit_error(error.to_string(), renderer_output_predecessor) {
                    Ok(Some(settlement)) => {
                        let (response_predecessor, response_succeeded) = settlement.into_parts();
                        debug_assert!(!response_succeeded);
                        Ok(RendererOwnerReply::RuntimeInspectorSessionErrorSettled(
                            response_predecessor,
                        ))
                        .into()
                    }
                    Ok(None) => unreachable!(
                        "a DevTools session publication must return its concrete response fence"
                    ),
                    Err(completion) => Err(anyhow!(
                        "Runtime error output closed before call {} was published",
                        completion.call_id
                    ))
                    .into(),
                }
            }
            (Ok(output), None) => {
                Ok(RendererOwnerReply::AsyncPageCommandRan(Box::new(output))).into()
            }
            (Err(error), None) => Err(error).into(),
        }
    }

    pub(super) fn merge_pending_download_into_reply(
        &self,
        mut reply: RendererPageReply,
        download: RendererPendingDownloadActivation,
    ) -> RendererPageReply {
        if let Some(outcome) = reply.input_dispatch_outcome_mut() {
            outcome.pending_download = Some(download);
        }
        reply
    }

    pub(super) async fn finish_live_page_navigation_completion(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        completion: LivePagePendingNavigationCompletion,
    ) -> RenderRuntimeDispatchOutcome {
        entry.settle_renderer_navigation_follow(true);
        match completion {
            LivePagePendingNavigationCompletion::Background
            | LivePagePendingNavigationCompletion::PublishedPageCreation { .. } => {
                let lifecycle_to_resume = entry.active_page_vm().and_then(|page_vm| {
                    let lifecycle = page_vm.document_lifecycle.current_snapshot();
                    (lifecycle.load.is_none() && lifecycle.terminated.is_none())
                        .then_some(RendererDocumentLifecycleIdentity::from(lifecycle))
                });
                let (entry, page_state_result) = commit_page_state_on_entry_via_local_task(
                    self.state.local_executor.clone(),
                    entry,
                )
                .await;
                self.restore_live_page_entry(token, entry);
                match page_state_result {
                    Ok(_) => {
                        if let Some(document) = lifecycle_to_resume {
                            return RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                                RenderRuntimeTurn::ResumeLivePageDocumentLifecycleAfterReply {
                                    token,
                                    document,
                                },
                            ));
                        }
                        RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
                    }
                    Err(error) => RenderRuntimeDispatchOutcome::BackgroundComplete(Err(error)),
                }
            }
            LivePagePendingNavigationCompletion::CompletePageCreation { pending, .. } => {
                self.restore_live_page_entry(token, entry);
                self.finish_pending_page_creation(pending).await
            }
            LivePagePendingNavigationCompletion::CompletePageReplacement { .. } => {
                self.finish_prepared_page_replacement_reply(token, entry, None, None, None)
                    .await
            }
            LivePagePendingNavigationCompletion::ReplyWithSnapshot {
                reply,
                capture_policy,
            } => {
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    *reply,
                    None,
                    None,
                    capture_policy,
                )
                .await
            }
            LivePagePendingNavigationCompletion::ContinueNetworkIdle { deadline, loader } => {
                if let Err(error) = self.refresh_live_page_before_wait(token, entry).await {
                    return Err(error).into();
                }
                self.wait_live_page_network_idle_turn(
                    token,
                    PageVmNetworkIdleWaitState::default(),
                    deadline,
                    loader,
                )
                .await
            }
            LivePagePendingNavigationCompletion::ContinueDomStable { deadline, loader } => {
                if let Err(error) = self.refresh_live_page_before_wait(token, entry).await {
                    return Err(error).into();
                }
                self.wait_live_page_dom_stable_turn(
                    token,
                    PageVmDomStableWaitState::default(),
                    deadline,
                    loader,
                )
                .await
            }
            LivePagePendingNavigationCompletion::ContinueSubresourceResponse {
                criteria,
                deadline,
                loader,
                capture_policy,
            } => {
                if let Err(error) = self.refresh_live_page_before_wait(token, entry).await {
                    return Err(error).into();
                }
                self.wait_live_page_subresource_response_turn(
                    token,
                    criteria,
                    deadline,
                    loader,
                    capture_policy,
                )
                .await
            }
        }
    }

    /// Capture the already-published replacement before a host-facing wait
    /// resumes. Cross-creation publication is deliberately forbidden here;
    /// it belongs to the typed response/Document commit transition.
    pub(super) async fn refresh_live_page_before_wait(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
    ) -> Result<()> {
        let (entry, page_state_result) =
            commit_page_state_on_entry_via_local_task(self.state.local_executor.clone(), entry)
                .await;
        self.restore_live_page_entry(token, entry);
        page_state_result?;
        Ok(())
    }

    pub(super) async fn finish_live_page_navigation_download(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        completion: LivePagePendingNavigationCompletion,
        download: RendererPendingDownloadActivation,
    ) -> RenderRuntimeDispatchOutcome {
        entry.settle_renderer_navigation_follow(true);
        match completion {
            LivePagePendingNavigationCompletion::Background
            | LivePagePendingNavigationCompletion::PublishedPageCreation { .. } => {
                self.restore_live_page_entry(token, entry);
                drop(download);
                RenderRuntimeDispatchOutcome::BackgroundComplete(Ok(()))
            }
            LivePagePendingNavigationCompletion::CompletePageCreation { pending, .. } => {
                self.restore_live_page_entry(token, entry);
                self.finish_pending_page_creation(pending.with_pending_download(download))
                    .await
            }
            LivePagePendingNavigationCompletion::CompletePageReplacement { .. } => {
                self.finish_prepared_page_replacement_reply(
                    token,
                    entry,
                    None,
                    None,
                    Some(download),
                )
                .await
            }
            LivePagePendingNavigationCompletion::ReplyWithSnapshot {
                reply,
                capture_policy,
            } => {
                let reply = *reply;
                let reply = self.merge_pending_download_into_reply(reply, download);
                self.finish_live_page_entry_with_page_state_and_continuation(
                    token,
                    entry,
                    reply,
                    None,
                    None,
                    capture_policy,
                )
                .await
            }
            LivePagePendingNavigationCompletion::ContinueNetworkIdle { .. }
            | LivePagePendingNavigationCompletion::ContinueDomStable { .. }
            | LivePagePendingNavigationCompletion::ContinueSubresourceResponse { .. } => {
                self.restore_live_page_entry(token, entry);
                Err(anyhow!(
                    "location navigation resolved to a download while waiting on page state"
                ))
                .into()
            }
        }
    }

    pub(super) fn continue_live_page_pending_navigation(
        &self,
        token: RendererPageToken,
        mut entry: LivePageEntry,
        stage: PageVmInitStage,
        follow_count: usize,
        completion: LivePagePendingNavigationCompletion,
    ) -> RenderRuntimeDispatchOutcome {
        if follow_count == 0 {
            entry.begin_renderer_navigation_follow();
        }
        self.restore_live_page_entry(token, entry);
        RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
            RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                token,
                stage,
                follow_count,
                completion,
            },
        ))
    }
}
