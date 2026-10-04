use std::time::Instant;

use anyhow::{Result, anyhow};

use crate::local_executor::JsLocalExecutor;
use crate::runtime::page_vm::DocumentLifecycleTurnOutcome;
use crate::runtime::{
    PageVmDocumentCommitPreparation, PageVmFollowNavigationTurnOutcome, PageVmInitStage,
    PageVmPreparedFollowedNavigationCommit, PageVmValidatedFollowedNavigationCommit,
    RendererPageToken, RendererPendingDownloadActivation,
};

use super::bound::run_typed_entry_on_bound_owner_local_store_local_task;
use super::{CommittedNavigationEntry, LivePageEntry, PublishedReplacementDocument};

pub(super) enum CommittedNavigationBootstrapCompletion {
    ContinuePostParseLifecycle {
        page_tasks: Vec<crate::page_task_queue::PostParsePageOwnedWork>,
        stage: PageVmInitStage,
        started: Instant,
    },
    PendingPhaseOne {
        wake_token: RendererPageToken,
    },
    TriggeredNavigation {
        stage: PageVmInitStage,
    },
}

pub(in crate::runtime) enum LivePageNavigationFollowOutcome {
    Completed,
    PostParseLifecycle {
        target_stage: PageVmInitStage,
        outcome: DocumentLifecycleTurnOutcome,
    },
    Download(RendererPendingDownloadActivation),
    /// Navigation yielded during phase one. The caller must first restore the
    /// returned entry, then reconcile the resident continuation against its
    /// stable producer source.
    PendingPhaseOne {
        wake_token: RendererPageToken,
    },
    TriggeredNavigation {
        stage: PageVmInitStage,
    },
}

pub(in crate::runtime) struct LivePageNavigationFollowTurn {
    pub(in crate::runtime) outcome: LivePageNavigationFollowOutcome,
    pub(in crate::runtime) document_commit: Option<PublishedReplacementDocument>,
}

/// Typed result of one checked-out navigation task. A task that ends before
/// commit returns a live entry; a task that fails, panics, or is cancelled
/// during replacement bootstrap returns the committed state explicitly.
pub(in crate::runtime) enum LivePageNavigationFollowEntryAdvance {
    Live {
        entry: LivePageEntry,
        result: Result<LivePageNavigationFollowTurn>,
    },
    Committed {
        entry: CommittedNavigationEntry,
        error: anyhow::Error,
    },
}

enum NavigationLocalTaskEntry {
    Live(LivePageEntry),
    Committed(CommittedNavigationEntry),
    Transitioning,
}

impl NavigationLocalTaskEntry {
    fn live_mut(&mut self) -> &mut LivePageEntry {
        match self {
            Self::Live(entry) => entry,
            Self::Committed(_) | Self::Transitioning => {
                unreachable!("navigation task must be live at this boundary")
            }
        }
    }

    fn committed_mut(&mut self) -> &mut CommittedNavigationEntry {
        match self {
            Self::Committed(entry) => entry,
            Self::Live(_) | Self::Transitioning => {
                unreachable!("navigation task must be committed at this boundary")
            }
        }
    }

    fn commit(&mut self, prepared: PageVmPreparedFollowedNavigationCommit) -> Result<()> {
        let validated: PageVmValidatedFollowedNavigationCommit = self
            .live_mut()
            .page_vm_mut()
            .validate_prepared_followed_location_navigation(prepared)?;
        let (commit, navigation) = validated.into_parts();
        // Only moves remain between removing Live and publishing Committed.
        // Detachment and all source teardown run with the committed tag
        // already held by the local-task guard.
        let Self::Live(entry) = std::mem::replace(self, Self::Transitioning) else {
            unreachable!("navigation commit must consume a live entry")
        };
        *self = Self::Committed(CommittedNavigationEntry::new(entry, navigation));
        self.committed_mut().commit_source(commit);
        Ok(())
    }

    fn finish_commit(&mut self) -> Result<()> {
        // Validate before moving the entry so even a broken bootstrap invariant
        // returns the committed residence through the task guard.
        self.committed_mut().ensure_live_replacement()?;
        let Self::Committed(entry) = std::mem::replace(self, Self::Transitioning) else {
            unreachable!("replacement bootstrap must complete a committed entry")
        };
        *self = Self::Live(entry.into_live());
        Ok(())
    }

    async fn bootstrap_committed_navigation(&mut self) -> Result<LivePageNavigationFollowOutcome> {
        let bootstrap_outcome = self.committed_mut().bootstrap_replacement().await?;
        let completion = self
            .committed_mut()
            .install_bootstrap_outcome(bootstrap_outcome)?;
        self.finish_commit()?;
        match completion {
            CommittedNavigationBootstrapCompletion::ContinuePostParseLifecycle {
                page_tasks,
                stage,
                started,
            } => {
                let lifecycle = {
                    let (page_vm, pending_document_lifecycle_turn) =
                        self.live_mut().page_vm_and_document_lifecycle_turn_mut();
                    page_vm
                        .begin_post_parse_lifecycle_on_named_owner_lane(
                            pending_document_lifecycle_turn,
                            page_tasks,
                            stage,
                            started,
                        )
                        .await?
                };
                Ok(LivePageNavigationFollowOutcome::PostParseLifecycle {
                    target_stage: stage,
                    outcome: lifecycle,
                })
            }
            CommittedNavigationBootstrapCompletion::PendingPhaseOne { wake_token } => {
                Ok(LivePageNavigationFollowOutcome::PendingPhaseOne { wake_token })
            }
            CommittedNavigationBootstrapCompletion::TriggeredNavigation { stage } => {
                Ok(LivePageNavigationFollowOutcome::TriggeredNavigation { stage })
            }
        }
    }
}

pub(in crate::runtime) async fn follow_pending_location_navigation_one_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    stage: PageVmInitStage,
) -> LivePageNavigationFollowEntryAdvance {
    let (entry, result) = run_typed_entry_on_bound_owner_local_store_local_task(
        local_executor,
        NavigationLocalTaskEntry::Live(entry),
        move |entry| {
            Box::pin(async move {
                let preparation = {
                    let (page_vm, pending_document_lifecycle_turn) =
                        entry.live_mut().page_vm_and_document_lifecycle_turn_mut();
                    page_vm
                        .prepare_pending_location_navigation_document_commit_one_turn_async(
                            pending_document_lifecycle_turn,
                            stage,
                        )
                        .await
                };
                let outcome = match preparation? {
                    PageVmDocumentCommitPreparation::Prepared(prepared) => {
                        entry.commit(*prepared)?;
                        entry.bootstrap_committed_navigation().await?
                    }
                    PageVmDocumentCommitPreparation::Uncommitted(outcome) => match *outcome {
                        PageVmFollowNavigationTurnOutcome::Completed => {
                            LivePageNavigationFollowOutcome::Completed
                        }
                        PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                            target_stage,
                            outcome,
                        } => LivePageNavigationFollowOutcome::PostParseLifecycle {
                            target_stage,
                            outcome,
                        },
                        PageVmFollowNavigationTurnOutcome::Download(download) => {
                            LivePageNavigationFollowOutcome::Download(download)
                        }
                        PageVmFollowNavigationTurnOutcome::TriggeredNavigation { stage } => {
                            LivePageNavigationFollowOutcome::TriggeredNavigation { stage }
                        }
                    },
                };
                let document_commit = if entry.live_mut().has_uncommitted_page_vm() {
                    Some(entry.live_mut().publish_replacement_document_commit()?)
                } else {
                    None
                };
                Ok(LivePageNavigationFollowTurn {
                    outcome,
                    document_commit,
                })
            })
        },
    )
    .await;
    match entry {
        NavigationLocalTaskEntry::Live(entry) => {
            LivePageNavigationFollowEntryAdvance::Live { entry, result }
        }
        NavigationLocalTaskEntry::Committed(entry) => {
            let error = result.err().unwrap_or_else(|| {
                anyhow!("committed navigation task completed without a replacement PageVm")
            });
            LivePageNavigationFollowEntryAdvance::Committed { entry, error }
        }
        NavigationLocalTaskEntry::Transitioning => {
            unreachable!("navigation task cannot return while changing typestate")
        }
    }
}
