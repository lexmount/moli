use super::*;

pub(in crate::runtime) enum LivePagePendingNavigationPhaseOneAdvance {
    /// The resumed parser yielded again. The caller restores the entry before
    /// deriving a one-shot admission wake from the stable Page slot.
    Pending {
        wake_token: RendererPageToken,
    },
    TriggeredNavigation {
        stage: PageVmInitStage,
    },
    PostParseLifecycle {
        target_stage: PageVmInitStage,
        outcome: DocumentLifecycleTurnOutcome,
    },
}

/// Result of advancing a checked-out phase-one navigation residence.
///
/// Resuming phase one temporarily consumes the Page's only active VM. Most
/// outcomes install that VM (or a new pending residence) back into the live
/// entry, but an unrecoverable resume failure leaves only a teardown shell.
/// Keeping those states in separate variants prevents the owner from passing
/// that shell to a live-entry restore API.
pub(in crate::runtime) enum PendingPhaseOneEntryAdvance {
    Live {
        entry: LivePageEntry,
        result: Result<LivePagePendingNavigationPhaseOneAdvance>,
    },
    Retiring {
        entry: RetiringPageEntry,
        error: anyhow::Error,
    },
}

pub(in crate::runtime) async fn advance_pending_phase_one_navigation_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
) -> PendingPhaseOneEntryAdvance {
    let (entry, result) =
        run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
            Box::pin(async move {
                let pending = entry.take_pending_phase_one_navigation()?;
                let (residence, mut metadata) = pending.into_parts();
                let browser_context_runtime = residence
                    .page_vm()
                    .runtime_hooks
                    .browser_context_runtime
                    .clone();
                let phase_one_outcome = match residence.resume().await {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        metadata.reject(
                            None,
                            &browser_context_runtime,
                            format!("Cannot navigate to URL: {error}"),
                        );
                        return Err(error);
                    }
                };
                let phase_one_outcome = match phase_one_outcome {
                    PendingPhaseOneResumeOutcome::Progress(outcome) => outcome,
                    PendingPhaseOneResumeOutcome::MainResourceLoadFailed { page_vm, error } => {
                        metadata.reject(
                            None,
                            &browser_context_runtime,
                            format!("Cannot navigate to URL: {error}"),
                        );
                        entry.install_resumed_phase_one_page_vm(page_vm);
                        return Err(error);
                    }
                };
                match phase_one_outcome {
                    ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) => {
                        let pending = PageVmPendingPhaseOneNavigation::new(residence, metadata);
                        let wake_token = entry.restore_pending_phase_one_navigation(pending)?;
                        Ok(LivePagePendingNavigationPhaseOneAdvance::Pending { wake_token })
                    }
                    ParseTimePageVmCreationOutcome::TriggeredNavigation { mut page_vm, stage } => {
                        metadata.complete_service_worker_follow(&mut page_vm);
                        entry.install_resumed_phase_one_page_vm(page_vm);
                        Ok(LivePagePendingNavigationPhaseOneAdvance::TriggeredNavigation { stage })
                    }
                    ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                        mut page_vm,
                        page_tasks,
                        stage,
                        started,
                    } => {
                        metadata.complete_service_worker_follow(&mut page_vm);
                        entry.install_resumed_phase_one_page_vm(page_vm);
                        let (page_vm, pending_document_lifecycle_turn) =
                            entry.page_vm_and_document_lifecycle_turn_mut();
                        let outcome = page_vm
                            .begin_post_parse_lifecycle_on_named_owner_lane(
                                pending_document_lifecycle_turn,
                                page_tasks,
                                stage,
                                started,
                            )
                            .await?;
                        Ok(
                            LivePagePendingNavigationPhaseOneAdvance::PostParseLifecycle {
                                target_stage: stage,
                                outcome,
                            },
                        )
                    }
                }
            })
        })
        .await;

    classify_pending_phase_one_entry_advance(entry, result)
}

pub(super) fn classify_pending_phase_one_entry_advance(
    entry: LivePageEntry,
    result: Result<LivePagePendingNavigationPhaseOneAdvance>,
) -> PendingPhaseOneEntryAdvance {
    if entry.active_page_vm().is_some() {
        PendingPhaseOneEntryAdvance::Live { entry, result }
    } else {
        let error = result.err().unwrap_or_else(|| {
            anyhow!("phase-one navigation advance completed without restoring an active PageVm")
        });
        PendingPhaseOneEntryAdvance::Retiring {
            entry: RetiringPageEntry::new(entry),
            error,
        }
    }
}
