use moli_core::RendererOutputTransportMessage;
use moli_protocol::{
    CompletedDeferredMainDocumentLoadCompletion, DeferredMainDocumentLoadCompletionOutputAction,
    DeferredMainDocumentLoadCompletionOutputInterest, DeferredMainDocumentLoadObservationId,
    PendingDeferredMainDocumentLoadCompletion,
};
use tokio::sync::mpsc;

use super::{CdpScheduler, ProtocolOutputSequence, protocol_residence::ProtocolSchedulerStep};

/// Connection-local scheduling state shared by CDP, BiDi and Classic adapters.
///
/// The protocol scheduler owns concrete output and browser-owner work. This
/// driver owns only the asynchronous adapter boundary needed to select that
/// work in a later client turn:
///
/// - one coalesced self-turn signal;
/// - every exact main-document load observation currently awaiting its
///   target-local terminal.
///
/// It never stores a renderer publication, Page task capability or protocol
/// transport route. Independent Pages may therefore wait for load in parallel,
/// and switching a Classic connection to BiDi keeps the observations alive.
pub(crate) struct ProtocolAdapterScheduler {
    turn_tx: mpsc::UnboundedSender<()>,
    turn_rx: mpsc::UnboundedReceiver<()>,
    turn_scheduled: bool,
    load_completion_tx: mpsc::UnboundedSender<CompletedDeferredMainDocumentLoadCompletion>,
    load_completion_rx: mpsc::UnboundedReceiver<CompletedDeferredMainDocumentLoadCompletion>,
    pending_loads: Vec<PendingAdapterLoadObservation>,
}

struct PendingAdapterLoadObservation {
    observation_id: DeferredMainDocumentLoadObservationId,
    output_interest: DeferredMainDocumentLoadCompletionOutputInterest,
}

pub(crate) enum ProtocolAdapterSchedulerInput {
    Turn,
    DeferredLoadCompletion(Box<CompletedDeferredMainDocumentLoadCompletion>),
}

/// Result of consuming one shared adapter-scheduler input.
///
/// `DeferredLoadStarted` deliberately does not expose the pending observer or
/// its wake interest. The exact identity remains owned by
/// `ProtocolAdapterScheduler` until `DeferredLoadCompleted`.
pub(crate) enum ProtocolAdapterSchedulerAdvance {
    Idle,
    ClientTurnYielded,
    DeferredLoadStarted {
        observation_id: DeferredMainDocumentLoadObservationId,
    },
    ProtocolResidenceCompleted(ProtocolOutputSequence),
    DeferredLoadCompleted {
        observation_id: DeferredMainDocumentLoadObservationId,
        output: ProtocolOutputSequence,
    },
    StaleDeferredLoadCompletion {
        observation_id: DeferredMainDocumentLoadObservationId,
    },
}

impl Default for ProtocolAdapterScheduler {
    fn default() -> Self {
        let (turn_tx, turn_rx) = mpsc::unbounded_channel();
        let (load_completion_tx, load_completion_rx) = mpsc::unbounded_channel();
        Self {
            turn_tx,
            turn_rx,
            turn_scheduled: false,
            load_completion_tx,
            load_completion_rx,
            pending_loads: Vec::new(),
        }
    }
}

impl ProtocolAdapterScheduler {
    pub(crate) fn has_pending_loads(&self) -> bool {
        !self.pending_loads.is_empty()
    }

    /// Coalesces scheduler readiness into one later adapter turn.
    ///
    /// Sending through a local channel is intentional: satisfying a
    /// `ClientTurnPredecessor` must happen after control returns to the adapter
    /// loop, not recursively in the producer or command-completion stack.
    pub(crate) fn schedule_turn_if_needed(
        &mut self,
        scheduler: &CdpScheduler,
        page_javascript_blocked: bool,
    ) {
        if page_javascript_blocked {
            return;
        }
        let step = self.next_scheduler_step(scheduler);
        if self.turn_scheduled || step == ProtocolSchedulerStep::Wait {
            return;
        }
        if moli_trace::cdp_runtime_trace_enabled() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "protocol_adapter_turn_schedule",
                step = ?step,
            );
        }
        self.turn_scheduled = true;
        let turn_tx = self.turn_tx.clone();
        tokio::task::spawn_local(async move {
            let _ = turn_tx.send(());
        });
    }

    /// Waits for either the coalesced self turn or the exact load terminal.
    ///
    /// The driver retains both senders, so a closed internal channel is an
    /// invariant violation rather than an adapter shutdown signal.
    pub(crate) async fn recv_input(&mut self) -> ProtocolAdapterSchedulerInput {
        tokio::select! {
            biased;
            completion = self.load_completion_rx.recv(), if self.has_pending_loads() => {
                ProtocolAdapterSchedulerInput::DeferredLoadCompletion(Box::new(
                    completion.expect("shared adapter load-completion channel must remain open"),
                ))
            }
            turn = self.turn_rx.recv(), if self.turn_scheduled => {
                turn.expect("shared adapter self-turn channel must remain open");
                ProtocolAdapterSchedulerInput::Turn
            }
            // Every adapter selects this future alongside its transport and
            // renderer inputs. A completely idle protocol scheduler therefore
            // means "this source is not ready", not an all-branches-disabled
            // error.
            else => std::future::pending::<ProtocolAdapterSchedulerInput>().await,
        }
    }

    /// Ingests one concrete renderer publication behind every matching exact
    /// load observation currently owned by this connection-local driver.
    pub(crate) async fn ingest_renderer_publication(
        &mut self,
        scheduler: &mut CdpScheduler,
        publication: RendererOutputTransportMessage,
    ) -> ProtocolOutputSequence {
        if self.pending_loads.is_empty() {
            return scheduler
                .ingest_renderer_publication_for_scheduler(publication)
                .await;
        }
        let observation_ids = self
            .pending_loads
            .iter()
            .filter_map(|pending| {
                (scheduler.route_renderer_output_for_deferred_load_completion(
                    &publication,
                    &pending.output_interest,
                ) == DeferredMainDocumentLoadCompletionOutputAction::Queue)
                    .then_some(pending.observation_id)
            })
            .collect();
        scheduler
            .ingest_renderer_publication_after_loads(publication, observation_ids)
            .await
    }

    /// Consumes one input and advances at most one concrete scheduler
    /// residence.
    ///
    pub(crate) async fn advance_input(
        &mut self,
        scheduler: &mut CdpScheduler,
        input: ProtocolAdapterSchedulerInput,
    ) -> ProtocolAdapterSchedulerAdvance {
        match input {
            ProtocolAdapterSchedulerInput::Turn => {
                self.turn_scheduled = false;
                self.advance_turn(scheduler).await
            }
            ProtocolAdapterSchedulerInput::DeferredLoadCompletion(completion) => {
                self.complete_load(scheduler, *completion).await
            }
        }
    }

    async fn advance_turn(
        &mut self,
        scheduler: &mut CdpScheduler,
    ) -> ProtocolAdapterSchedulerAdvance {
        match self.next_scheduler_step(scheduler) {
            ProtocolSchedulerStep::SatisfyClientTurnPredecessor => {
                scheduler.satisfy_front_protocol_residence_client_turn_predecessor();
                ProtocolAdapterSchedulerAdvance::ClientTurnYielded
            }
            ProtocolSchedulerStep::CompleteReadyResidence
                if scheduler.next_ready_protocol_residence_is_main_document_load_action() =>
            {
                let pending = scheduler
                    .start_next_deferred_load_completion()
                    .expect("ready load residence must produce an exact pending observation");
                let observation_id = pending.observation_id();
                let output_interest = pending.output_interest();
                assert!(
                    !self
                        .pending_loads
                        .iter()
                        .any(|pending| pending.observation_id == observation_id),
                    "one exact load observation cannot be started twice"
                );
                self.pending_loads.push(PendingAdapterLoadObservation {
                    observation_id,
                    output_interest,
                });
                self.spawn_load_wait(pending);
                ProtocolAdapterSchedulerAdvance::DeferredLoadStarted { observation_id }
            }
            ProtocolSchedulerStep::CompleteReadyResidence => {
                ProtocolAdapterSchedulerAdvance::ProtocolResidenceCompleted(
                    scheduler.complete_next_protocol_residence().await,
                )
            }
            ProtocolSchedulerStep::Wait => ProtocolAdapterSchedulerAdvance::Idle,
        }
    }

    /// Returns the next concrete-residence transition this adapter may drive.
    /// Exact `load_predecessors` in `CdpScheduler` provide the target-local
    /// ordering; the adapter must not add a connection-wide capacity gate.
    fn next_scheduler_step(&self, scheduler: &CdpScheduler) -> ProtocolSchedulerStep {
        scheduler.next_protocol_scheduler_step()
    }

    fn spawn_load_wait(&self, pending: PendingDeferredMainDocumentLoadCompletion) {
        if moli_trace::cdp_runtime_trace_enabled() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "protocol_adapter_load_wait_spawn",
                observation_id = ?pending.observation_id(),
            );
        }
        let completion_tx = self.load_completion_tx.clone();
        tokio::task::spawn_local(async move {
            let completion = pending.wait().await;
            let _ = completion_tx.send(completion);
        });
    }

    async fn complete_load(
        &mut self,
        scheduler: &mut CdpScheduler,
        completion: CompletedDeferredMainDocumentLoadCompletion,
    ) -> ProtocolAdapterSchedulerAdvance {
        let observation_id = completion.observation_id();
        if self.take_pending_load(observation_id).is_err() {
            return ProtocolAdapterSchedulerAdvance::StaleDeferredLoadCompletion { observation_id };
        }
        let output = scheduler
            .complete_deferred_load_completion(completion)
            .await;
        ProtocolAdapterSchedulerAdvance::DeferredLoadCompleted {
            observation_id,
            output,
        }
    }

    /// Claims only the exact observation that produced a terminal. A delayed
    /// or duplicate terminal must not retire another Page's load wait.
    fn take_pending_load(
        &mut self,
        observation_id: DeferredMainDocumentLoadObservationId,
    ) -> Result<PendingAdapterLoadObservation, ()> {
        let index = self
            .pending_loads
            .iter()
            .position(|pending| pending.observation_id == observation_id)
            .ok_or(())?;
        Ok(self.pending_loads.remove(index))
    }
}

#[cfg(test)]
mod tests {
    use moli_core::{PageId, RendererOutputResidenceIdentity, RendererOwnerLocalHostId};
    use moli_protocol::{
        CdpConnection, CdpSchedulerEvent, ProtocolSchedulerWork,
        test_support::{
            deferred_main_document_load_observation_id,
            deferred_main_document_load_output_interest, root_frame_stopped_loading_work,
        },
    };
    use tokio::task::LocalSet;

    use super::{
        PendingAdapterLoadObservation, ProtocolAdapterScheduler, ProtocolAdapterSchedulerAdvance,
        ProtocolAdapterSchedulerInput,
    };
    use crate::cdp_scheduler::{CdpScheduler, protocol_residence::ProtocolSchedulerStep};

    fn protocol_observation(publish_sequence: u64) -> ProtocolSchedulerWork {
        root_frame_stopped_loading_work(
            publish_sequence,
            vec![Some("SID-adapter".to_owned())],
            "FRAME-adapter".to_owned(),
            "LOADER-adapter".to_owned(),
        )
    }

    fn page_residence() -> RendererOutputResidenceIdentity {
        RendererOutputResidenceIdentity::Page {
            owner_local_host_id: RendererOwnerLocalHostId::new_for_testing(1),
            page_id: PageId::new_for_testing(7),
        }
    }

    #[test]
    fn pending_exact_load_observation_allows_independent_protocol_residence() {
        let observation_id = deferred_main_document_load_observation_id(1);
        let mut scheduler = CdpScheduler::new(CdpConnection::new());
        scheduler.apply_scheduler_events(vec![CdpSchedulerEvent::ProtocolWorkPublished {
            work: protocol_observation(1),
        }]);
        let adapter = ProtocolAdapterScheduler {
            pending_loads: vec![PendingAdapterLoadObservation {
                observation_id,
                output_interest: deferred_main_document_load_output_interest(
                    page_residence(),
                    None,
                ),
            }],
            ..Default::default()
        };

        assert_eq!(
            adapter.next_scheduler_step(&scheduler),
            ProtocolSchedulerStep::SatisfyClientTurnPredecessor,
            "an exact load observation must not block an independent client-turn boundary"
        );
        scheduler.satisfy_front_protocol_residence_client_turn_predecessor();
        assert_eq!(
            adapter.next_scheduler_step(&scheduler),
            ProtocolSchedulerStep::CompleteReadyResidence,
            "independent protocol work must remain runnable while the exact observation waits"
        );
        assert!(
            adapter.has_pending_loads(),
            "running independent work must not release the exact load observation"
        );
    }

    #[test]
    fn multiple_exact_load_observations_retire_independently() {
        let first = deferred_main_document_load_observation_id(1);
        let second = deferred_main_document_load_observation_id(2);
        let stale = deferred_main_document_load_observation_id(3);
        let mut adapter = ProtocolAdapterScheduler {
            pending_loads: vec![
                PendingAdapterLoadObservation {
                    observation_id: first,
                    output_interest: deferred_main_document_load_output_interest(
                        page_residence(),
                        None,
                    ),
                },
                PendingAdapterLoadObservation {
                    observation_id: second,
                    output_interest: deferred_main_document_load_output_interest(
                        RendererOutputResidenceIdentity::Page {
                            owner_local_host_id: RendererOwnerLocalHostId::new_for_testing(1),
                            page_id: PageId::new_for_testing(8),
                        },
                        None,
                    ),
                },
            ],
            ..Default::default()
        };

        assert_eq!(adapter.pending_loads.len(), 2);
        adapter
            .take_pending_load(second)
            .expect("the second Page terminal must claim only its observation");
        assert_eq!(
            adapter
                .pending_loads
                .iter()
                .map(|pending| pending.observation_id)
                .collect::<Vec<_>>(),
            [first],
            "an out-of-order terminal must preserve the first Page wait"
        );
        assert!(
            adapter.take_pending_load(stale).is_err(),
            "an unknown terminal must not retire another Page wait"
        );
        adapter
            .take_pending_load(first)
            .expect("the first Page terminal should remain claimable");
        assert!(!adapter.has_pending_loads());
    }

    #[tokio::test]
    async fn idle_adapter_input_remains_pending() {
        let mut adapter = ProtocolAdapterScheduler::default();
        tokio::select! {
            biased;
            _ = adapter.recv_input() => {
                panic!("an idle shared scheduler source must remain pending");
            }
            _ = std::future::ready(()) => {}
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn self_turn_is_coalesced_and_preserves_the_client_turn_boundary() {
        LocalSet::new()
            .run_until(async {
                let mut scheduler = CdpScheduler::new(CdpConnection::new());
                scheduler.apply_scheduler_events(vec![CdpSchedulerEvent::ProtocolWorkPublished {
                    work: protocol_observation(1),
                }]);
                let mut adapter = ProtocolAdapterScheduler::default();

                adapter.schedule_turn_if_needed(&scheduler, false);
                adapter.schedule_turn_if_needed(&scheduler, false);
                let first = adapter.recv_input().await;
                assert!(matches!(first, ProtocolAdapterSchedulerInput::Turn));
                assert!(matches!(
                    adapter.advance_input(&mut scheduler, first).await,
                    ProtocolAdapterSchedulerAdvance::ClientTurnYielded
                ));
                assert!(
                    adapter.turn_rx.try_recv().is_err(),
                    "coalescing must not leave a duplicate adapter turn queued"
                );

                adapter.schedule_turn_if_needed(&scheduler, false);
                let second = adapter.recv_input().await;
                assert!(matches!(second, ProtocolAdapterSchedulerInput::Turn));
                assert!(matches!(
                    adapter.advance_input(&mut scheduler, second).await,
                    ProtocolAdapterSchedulerAdvance::ProtocolResidenceCompleted(_)
                ));
            })
            .await;
    }
}
