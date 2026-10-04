use super::*;
use crate::runtime::RendererPageReplacementError;
use crate::runtime::owner_local_store::{
    commit_prepared_document_response_on_entry_via_local_task,
    finalize_prepared_page_replacement_on_entry_via_local_task,
};

impl RendererOwnerHandle {
    pub(super) async fn commit_prepared_page_replacement(
        &self,
        reservation: RendererPageReservationToken,
        residence: RendererPreparedDocumentResidence,
    ) -> RenderRuntimeDispatchOutcome {
        let Some(admission) = reservation.replacement else {
            return Err(anyhow!(
                "prepared response has no Page replacement admission"
            ))
            .into();
        };
        let context = match self.owner_local_context() {
            Ok(context) => context,
            Err(error) => return Err(error).into(),
        };
        let token = renderer_page_token_for_owner_context(&context, reservation.page_id());
        let entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return Err(error).into(),
        };
        let navigation_reply_policy = residence.request.navigation_reply_policy;
        let reply_boundary = residence.request.reply_boundary;
        let advance = commit_prepared_document_response_on_entry_via_local_task(
            self.state.local_executor.clone(),
            entry,
            admission.expected_vm_creation_id,
            residence,
        )
        .await;
        let (mut entry, result) = match advance {
            LivePageNavigationFollowEntryAdvance::Live { entry, result } => (entry, result),
            LivePageNavigationFollowEntryAdvance::Committed { entry, error } => {
                return self.finish_committed_navigation_failure(
                    token,
                    entry,
                    LivePageNavigationFailureDisposition::ReturnToInitiator(
                        RendererPageReplacementError::document_unavailable(error).into(),
                    ),
                );
            }
        };
        let turn = match result {
            Ok(turn) => turn,
            Err(error) => {
                let committed = entry.page_vm().creation_id != admission.expected_vm_creation_id;
                let error = if committed {
                    RendererPageReplacementError::document_unavailable(error)
                } else {
                    RendererPageReplacementError::document_preserved(error)
                };
                return self
                    .finish_live_page_navigation_failure(
                        token,
                        entry,
                        committed,
                        LivePageNavigationFailureDisposition::ReturnToInitiator(error.into()),
                    )
                    .await;
            }
        };
        if reply_boundary.waits_for_stage() {
            let completion = LivePagePendingNavigationCompletion::CompletePageReplacement {
                navigation_reply_policy,
            };
            let dispatch = match turn.outcome {
                LivePageNavigationFollowOutcome::PendingPhaseOne { wake_token } => {
                    let vm_creation_id = entry.page_vm().creation_id;
                    self.restore_live_page_entry(token, entry);
                    let admission =
                        pending_phase_one_admission_after_restore_on_bound_owner_local_store(
                            wake_token,
                        );
                    self.signal_pending_phase_one_admission(wake_token, admission);
                    RenderRuntimeDispatchOutcome::ContinueAfterPageWake {
                        turn: Box::new(
                            RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                                token,
                                vm_creation_id,
                                follow_count: 0,
                                completion,
                            },
                        ),
                        wake_token,
                    }
                }
                LivePageNavigationFollowOutcome::PostParseLifecycle {
                    target_stage,
                    outcome,
                } => {
                    self.handle_live_page_post_parse_lifecycle_outcome(
                        token,
                        entry,
                        target_stage,
                        0,
                        completion,
                        outcome,
                    )
                    .await
                }
                LivePageNavigationFollowOutcome::TriggeredNavigation { stage }
                    if !navigation_reply_policy.returns_with_pending_navigation() =>
                {
                    self.continue_live_page_pending_navigation(token, entry, stage, 0, completion)
                }
                LivePageNavigationFollowOutcome::TriggeredNavigation { .. }
                | LivePageNavigationFollowOutcome::Completed => {
                    self.finish_live_page_navigation_completion(token, entry, completion)
                        .await
                }
                LivePageNavigationFollowOutcome::Download(_) => {
                    unreachable!("prepared replacement must contain a Document")
                }
            };
            if let Some(document_commit) = turn.document_commit {
                self.signal_replacement_document_view_settled(
                    token,
                    document_commit.vm_creation_id,
                );
            }
            return dispatch;
        }
        let mut post_response_continuation = None;
        let continuation = match turn.outcome {
            LivePageNavigationFollowOutcome::PendingPhaseOne { wake_token } => Some(
                RenderRuntimePageCreationContinuation::AfterCommittedDocumentResponse {
                    turn: Box::new(
                        RenderRuntimeTurn::ContinueLivePagePendingLocationNavigationPhaseOne {
                            token,
                            vm_creation_id: entry.page_vm().creation_id,
                            follow_count: 0,
                            completion:
                                LivePagePendingNavigationCompletion::PublishedPageCreation {
                                    navigation_reply_policy,
                                },
                        },
                    ),
                    wake_token,
                },
            ),
            LivePageNavigationFollowOutcome::PostParseLifecycle {
                target_stage,
                outcome,
            } => match outcome.readiness {
                DocumentLifecycleTurnReadiness::Runnable { document }
                | DocumentLifecycleTurnReadiness::Blocked { document } => {
                    if let Err(error) = entry.defer_document_lifecycle_until_response(document) {
                        return self
                            .finish_live_page_navigation_failure(
                                token,
                                entry,
                                true,
                                LivePageNavigationFailureDisposition::ReturnToInitiator(
                                    RendererPageReplacementError::document_unavailable(error)
                                        .into(),
                                ),
                            )
                            .await;
                    }
                    post_response_continuation = Some(self.post_response_owner_work_continuation(
                        token,
                        RendererPostResponseOwnerWork {
                            document_lifecycle: Some(document),
                            navigation_handoff: None,
                        },
                    ));
                    Some(RenderRuntimePageCreationContinuation::next_turn(
                        RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                            token,
                            document,
                            target_stage,
                            follow_count: 0,
                            completion:
                                LivePagePendingNavigationCompletion::PublishedPageCreation {
                                    navigation_reply_policy,
                                },
                        },
                    ))
                }
                DocumentLifecycleTurnReadiness::Idle => None,
            },
            LivePageNavigationFollowOutcome::TriggeredNavigation { stage } => {
                if navigation_reply_policy.returns_with_pending_navigation() {
                    if let Err(error) = entry
                        .page_vm_mut()
                        .vm_mut()
                        .publish_pending_document_location_navigation()
                    {
                        return self
                            .finish_live_page_navigation_failure(
                                token,
                                entry,
                                true,
                                LivePageNavigationFailureDisposition::ReturnToInitiator(
                                    RendererPageReplacementError::document_unavailable(error)
                                        .into(),
                                ),
                            )
                            .await;
                    }
                    None
                } else {
                    entry.begin_renderer_navigation_follow();
                    Some(RenderRuntimePageCreationContinuation::next_turn(
                        RenderRuntimeTurn::FollowLivePagePendingLocationNavigation {
                            token,
                            stage,
                            follow_count: 0,
                            completion:
                                LivePagePendingNavigationCompletion::PublishedPageCreation {
                                    navigation_reply_policy,
                                },
                        },
                    ))
                }
            }
            LivePageNavigationFollowOutcome::Completed => None,
            LivePageNavigationFollowOutcome::Download(_) => {
                unreachable!("a committed replacement must contain a Document")
            }
        };
        let dispatch = self
            .finish_prepared_page_replacement_reply(
                token,
                entry,
                continuation,
                post_response_continuation,
                None,
            )
            .await;
        if let Some(document_commit) = turn.document_commit {
            self.signal_replacement_document_view_settled(token, document_commit.vm_creation_id);
        }
        dispatch
    }

    pub(super) async fn finish_prepared_page_replacement_reply(
        &self,
        token: RendererPageToken,
        entry: LivePageEntry,
        continuation: Option<RenderRuntimePageCreationContinuation>,
        post_response_continuation: Option<RendererPageCommandPostResponseContinuation>,
        pending_download: Option<RendererPendingDownloadActivation>,
    ) -> RenderRuntimeDispatchOutcome {
        let (entry, result) = finalize_prepared_page_replacement_on_entry_via_local_task(
            self.state.local_executor.clone(),
            token,
            entry,
        )
        .await;
        let (mut replacement, renderer_output) = match result {
            Ok(replacement) => replacement,
            Err(error) => {
                return self
                    .finish_live_page_navigation_failure(
                        token,
                        entry,
                        true,
                        LivePageNavigationFailureDisposition::ReturnToInitiator(
                            RendererPageReplacementError::document_unavailable(error).into(),
                        ),
                    )
                    .await;
            }
        };
        replacement.committed_document_post_response_continuation = post_response_continuation;
        replacement.pending_download = pending_download;
        let lifecycle = replacement.creation_artifacts.lifecycle_snapshot;
        let continuation = continuation.or_else(|| {
            (lifecycle.load.is_none() && lifecycle.terminated.is_none()).then(|| {
                RenderRuntimePageCreationContinuation::next_turn(
                    RenderRuntimeTurn::ResumeLivePageDocumentLifecycleAfterReply {
                        token,
                        document: lifecycle.into(),
                    },
                )
            })
        });
        if let Some(output) = renderer_output {
            self.publish_renderer_output(output);
        }
        self.restore_live_page_entry(token, entry);
        match continuation {
            Some(continuation) => {
                RenderRuntimeDispatchOutcome::PageReplacementCommittedAndContinueNavigation {
                    replacement: Box::new(replacement),
                    continuation,
                }
            }
            None => Ok(RendererOwnerReply::PageReplacementCommitted(Box::new(
                replacement,
            )))
            .into(),
        }
    }
}
