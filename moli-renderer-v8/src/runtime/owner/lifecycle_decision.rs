//! Owner-turn state machine for lifecycle-target Page creation decisions.
//!
//! This module is the only place that couples the synchronous host decision
//! contract to Page-creation gates and successor-navigation scheduling. The
//! generic owner loop only stores and dispatches the typed continuation.

use super::{
    LivePagePendingNavigationCompletion, NavigationReplyPolicy, RenderRuntimeDispatchOutcome,
    RenderRuntimeTurn, RendererOwnerHandle, RendererOwnerWakeSource, RendererPageToken,
    RendererPendingPageCreation, checked_live_page_wait_deadline,
    release_lifecycle_gate_on_bound_owner_local_store,
    take_entry_for_command_on_bound_owner_local_store,
};
use crate::runtime::document_lifecycle::RendererDocumentLifecycleWaitOutcome;
use crate::runtime::lifecycle_decision::{
    RendererLifecycleDecider, RendererLifecycleDecision, RendererLifecycleSnapshot,
};
use crate::runtime::owner_local_store::LivePageEntry;
use crate::runtime::page_vm::renderer_document_lifecycle_milestone_for_stage;
use anyhow::{Context, Result, anyhow, ensure};
use std::time::Instant;

pub(super) struct PendingLifecycleNavigation {
    pending: RendererPendingPageCreation,
    snapshot: RendererLifecycleSnapshot,
    navigation_grace_ms: u64,
    navigation_deadline: Instant,
    resume_parked_page_turn: bool,
}

impl PendingLifecycleNavigation {
    pub(super) const fn token(&self) -> RendererPageToken {
        self.pending.token
    }
}

fn capture_lifecycle_snapshot(
    entry: &LivePageEntry,
    target_stage: crate::PageVmInitStage,
) -> Result<RendererLifecycleSnapshot> {
    let document = entry.page_vm().document_lifecycle.identity();
    ensure!(
        matches!(
            entry.page_vm().document_lifecycle_wait_outcome(
                renderer_document_lifecycle_milestone_for_stage(target_stage)
            ),
            RendererDocumentLifecycleWaitOutcome::Reached(_)
        ),
        "renderer page {} tried to invoke its lifecycle decider before {target_stage:?}",
        entry.slot.page_id().as_u64()
    );

    // Read only response identity from the active VM. The stable Page state
    // may still describe the predecessor when navigation replaced a Document
    // before this first target was reached, and a full PageVm state capture
    // here would add unrelated work to the lifecycle boundary.
    let stable_page_state = entry.slot.active_page_state()?;
    let (requested_url, status) = entry
        .page_vm()
        .navigation_response
        .as_ref()
        .map(|response| (response.requested_url.clone(), response.status))
        .unwrap_or_else(|| {
            (
                stable_page_state.requested_url.clone(),
                stable_page_state.status,
            )
        });
    Ok(RendererLifecycleSnapshot {
        stage: target_stage,
        document,
        requested_url,
        final_url: entry.page_vm().vm().document_runtime.document_url().clone(),
        status,
    })
}

fn invoke_lifecycle_decider(
    decider: RendererLifecycleDecider,
    snapshot: RendererLifecycleSnapshot,
) -> Result<RendererLifecycleDecision> {
    // A host panic must retire only this pending Page, not unwind the renderer
    // owner lane. AssertUnwindSafe is appropriate because the one-shot decider is
    // consumed regardless of its outcome and no reference escapes this call.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decider.decide(snapshot))) {
        Ok(result) => result.context("page lifecycle decider failed"),
        Err(payload) => {
            let description = panic_payload_description(payload.as_ref());
            anyhow::bail!("page lifecycle decider panicked: {description}")
        }
    }
}

fn lifecycle_navigation_timeout_error(
    snapshot: &RendererLifecycleSnapshot,
    navigation_grace_ms: u64,
) -> anyhow::Error {
    let status = http::StatusCode::from_u16(snapshot.status)
        .map(|status| status.to_string())
        .unwrap_or_else(|_| snapshot.status.to_string());
    anyhow!(
        "lifecycle target document `{}` returned {status} and did not start a successor navigation within the {navigation_grace_ms} ms grace period",
        snapshot.final_url
    )
}

fn panic_payload_description(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

impl RendererOwnerHandle {
    pub(super) async fn apply_lifecycle_decision(
        &self,
        pending: RendererPendingPageCreation,
        target_stage: crate::PageVmInitStage,
        decider: RendererLifecycleDecider,
    ) -> RenderRuntimeDispatchOutcome {
        let token = pending.token;
        let (snapshot, renderer_output) = {
            let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
                Ok(entry) => entry,
                Err(error) => return self.retire_failed_page_creation(token, error).await,
            };
            let snapshot = capture_lifecycle_snapshot(&entry, target_stage);
            let renderer_output = entry.page_vm_mut().settle_renderer_output_publication();
            self.restore_live_page_entry(token, entry);
            let snapshot = match snapshot {
                Ok(snapshot) => snapshot,
                Err(error) => return self.retire_failed_page_creation(token, error).await,
            };
            (snapshot, renderer_output)
        };
        if let Some(output) = renderer_output {
            self.publish_renderer_output(output);
        }

        let decision = match invoke_lifecycle_decider(decider, snapshot.clone()) {
            Ok(decision) => decision,
            Err(error) => return self.retire_failed_page_creation(token, error).await,
        };

        match decision {
            RendererLifecycleDecision::Finish => {
                self.finalize_pending_page_creation_reply(pending).await
            }
            RendererLifecycleDecision::FollowNextDocument {
                navigation_grace_ms,
            } => {
                let navigation_deadline = match checked_live_page_wait_deadline(
                    navigation_grace_ms,
                    "lifecycle-target successor navigation grace",
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => return self.retire_failed_page_creation(token, error).await,
                };
                let released = match release_lifecycle_gate_on_bound_owner_local_store(token) {
                    Ok(released) => released,
                    Err(error) => {
                        return self.retire_failed_page_creation(token, error).await;
                    }
                };
                if released.target_stage != target_stage {
                    return self
                        .retire_failed_page_creation(
                            token,
                            anyhow!(
                                "renderer page lifecycle-target gate changed from {target_stage:?} to {:?}",
                                released.target_stage
                            ),
                        )
                        .await;
                }
                RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                    RenderRuntimeTurn::WaitLifecycleNavigation(PendingLifecycleNavigation {
                        pending,
                        snapshot,
                        navigation_grace_ms,
                        navigation_deadline,
                        resume_parked_page_turn: released.resume_parked_page_turn,
                    }),
                ))
            }
        }
    }

    pub(super) async fn wait_lifecycle_navigation_turn(
        &self,
        wait: PendingLifecycleNavigation,
    ) -> RenderRuntimeDispatchOutcome {
        let PendingLifecycleNavigation {
            pending,
            snapshot,
            navigation_grace_ms,
            navigation_deadline,
            resume_parked_page_turn,
        } = wait;
        let token = pending.token;
        let mut entry = match take_entry_for_command_on_bound_owner_local_store(token) {
            Ok(entry) => entry,
            Err(error) => return self.retire_failed_page_creation(token, error).await,
        };
        let current = entry.page_vm().document_lifecycle.identity();
        if current != snapshot.document {
            self.restore_live_page_entry(token, entry);
            return RenderRuntimeDispatchOutcome::ContinueNextTurn(Box::new(
                RenderRuntimeTurn::ContinueLivePageNavigationPostParseLifecycle {
                    token,
                    document: current,
                    target_stage: snapshot.stage,
                    follow_count: 0,
                    completion: LivePagePendingNavigationCompletion::CompletePageCreation {
                        pending,
                        navigation_reply_policy: NavigationReplyPolicy::FollowBeforeReply,
                    },
                },
            ));
        }
        if entry.page_vm().vm().has_pending_location_navigation()
            && entry.begin_standalone_navigation_follow()
        {
            return self.continue_live_page_pending_navigation(
                token,
                entry,
                snapshot.stage,
                0,
                LivePagePendingNavigationCompletion::CompletePageCreation {
                    pending,
                    navigation_reply_policy: NavigationReplyPolicy::FollowBeforeReply,
                },
            );
        }
        if Instant::now() >= navigation_deadline {
            self.restore_live_page_entry(token, entry);
            return self
                .retire_failed_page_creation(
                    token,
                    lifecycle_navigation_timeout_error(&snapshot, navigation_grace_ms),
                )
                .await;
        }

        self.restore_live_page_entry(token, entry);
        if resume_parked_page_turn {
            // Give the lifecycle-target continuation the first chance to claim
            // a navigation already pending at the retained boundary. Resume
            // the scheduler source only after that inspection.
            self.signal_internal_page_turn_source(
                token,
                RendererOwnerWakeSource::SchedulerContinuation,
            );
        }
        RenderRuntimeDispatchOutcome::ContinueAfterPageWakeOrDeadline {
            turn: Box::new(RenderRuntimeTurn::WaitLifecycleNavigation(
                PendingLifecycleNavigation {
                    pending,
                    snapshot,
                    navigation_grace_ms,
                    navigation_deadline,
                    resume_parked_page_turn: false,
                },
            )),
            wake_token: token,
            ready_at: navigation_deadline,
        }
    }
}
