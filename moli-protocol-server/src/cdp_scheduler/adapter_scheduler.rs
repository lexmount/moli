use moli_core::RendererOutputTransportMessage;
use tokio::sync::mpsc;

use super::{CdpScheduler, ProtocolOutputSequence, protocol_residence::ProtocolSchedulerStep};

/// Connection-local scheduling state shared by CDP, BiDi and Classic adapters.
///
/// The protocol scheduler owns concrete output and browser-owner work. This
/// driver owns only the asynchronous adapter boundary needed to select that
/// work in a later client turn:
///
/// - one coalesced self-turn signal.
///
/// Exact load interests stay with the scheduler queue until their shared
/// owner workflow hands off its output.
///
/// It never stores a renderer publication, Page task capability or protocol
/// transport route. Independent Pages may therefore wait for load in parallel,
/// and switching a Classic connection to BiDi keeps the observations alive.
pub(crate) struct ProtocolAdapterScheduler {
    turn_tx: mpsc::UnboundedSender<()>,
    turn_rx: mpsc::UnboundedReceiver<()>,
    turn_scheduled: bool,
}

pub(crate) enum ProtocolAdapterSchedulerInput {
    Turn,
}

/// Result of consuming one shared adapter-scheduler input.
///
pub(crate) enum ProtocolAdapterSchedulerAdvance {
    Idle,
    ClientTurnYielded,
    ProtocolResidenceCompleted(ProtocolOutputSequence),
}

impl Default for ProtocolAdapterScheduler {
    fn default() -> Self {
        let (turn_tx, turn_rx) = mpsc::unbounded_channel();
        Self {
            turn_tx,
            turn_rx,
            turn_scheduled: false,
        }
    }
}

impl ProtocolAdapterScheduler {
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

    /// Waits for the coalesced adapter turn. Owner work and load terminals
    /// arrive through the shared protocol task supervisor.
    pub(crate) async fn recv_input(&mut self) -> ProtocolAdapterSchedulerInput {
        if !self.turn_scheduled {
            return std::future::pending().await;
        }
        self.turn_rx
            .recv()
            .await
            .expect("shared adapter self-turn channel must remain open");
        ProtocolAdapterSchedulerInput::Turn
    }

    pub(crate) fn ingest_renderer_publication(
        &mut self,
        scheduler: &mut CdpScheduler,
        publication: RendererOutputTransportMessage,
    ) -> ProtocolOutputSequence {
        scheduler.ingest_renderer_publication_for_scheduler(publication)
    }

    /// Consumes one input and advances at most one concrete scheduler
    /// residence.
    ///
    pub(crate) fn advance_input(
        &mut self,
        scheduler: &mut CdpScheduler,
        input: ProtocolAdapterSchedulerInput,
    ) -> ProtocolAdapterSchedulerAdvance {
        match input {
            ProtocolAdapterSchedulerInput::Turn => {
                self.turn_scheduled = false;
                self.advance_turn(scheduler)
            }
        }
    }

    fn advance_turn(&mut self, scheduler: &mut CdpScheduler) -> ProtocolAdapterSchedulerAdvance {
        match self.next_scheduler_step(scheduler) {
            ProtocolSchedulerStep::SatisfyClientTurnPredecessor => {
                scheduler.satisfy_front_protocol_residence_client_turn_predecessor();
                ProtocolAdapterSchedulerAdvance::ClientTurnYielded
            }
            ProtocolSchedulerStep::CompleteReadyResidence => {
                ProtocolAdapterSchedulerAdvance::ProtocolResidenceCompleted(
                    scheduler.complete_next_protocol_residence(),
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
        ProtocolAdapterScheduler, ProtocolAdapterSchedulerAdvance, ProtocolAdapterSchedulerInput,
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
        let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
        scheduler.apply_scheduler_events(vec![CdpSchedulerEvent::ProtocolWorkPublished {
            work: protocol_observation(1),
        }]);
        let adapter = ProtocolAdapterScheduler::default();
        scheduler.queues.start_main_document_load(
            observation_id,
            deferred_main_document_load_output_interest(page_residence(), None),
        );

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
            !scheduler.queues.pending_loads.is_empty(),
            "running independent work must not release the exact load observation"
        );
    }

    #[test]
    fn multiple_exact_load_observations_retire_independently() {
        let first = deferred_main_document_load_observation_id(1);
        let second = deferred_main_document_load_observation_id(2);
        let stale = deferred_main_document_load_observation_id(3);
        let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
        for (id, page_id) in [(first, 7), (second, 8)] {
            scheduler.queues.start_main_document_load(
                id,
                deferred_main_document_load_output_interest(
                    RendererOutputResidenceIdentity::Page {
                        owner_local_host_id: RendererOwnerLocalHostId::new_for_testing(1),
                        page_id: PageId::new_for_testing(page_id),
                    },
                    None,
                ),
            );
        }
        assert!(scheduler.queues.finish_main_document_load(second));
        assert_eq!(
            scheduler
                .queues
                .pending_loads
                .iter()
                .map(|pending| pending.observation_id)
                .collect::<Vec<_>>(),
            [first]
        );
        assert!(!scheduler.queues.finish_main_document_load(stale));
        assert_eq!(scheduler.queues.pending_loads.len(), 1);
        assert!(scheduler.queues.finish_main_document_load(first));
        assert!(scheduler.queues.pending_loads.is_empty());
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
                let (mut scheduler, _protocol_completions) =
                    CdpScheduler::new(CdpConnection::new());
                scheduler.apply_scheduler_events(vec![CdpSchedulerEvent::ProtocolWorkPublished {
                    work: protocol_observation(1),
                }]);
                let mut adapter = ProtocolAdapterScheduler::default();

                adapter.schedule_turn_if_needed(&scheduler, false);
                adapter.schedule_turn_if_needed(&scheduler, false);
                let first = adapter.recv_input().await;
                assert!(matches!(first, ProtocolAdapterSchedulerInput::Turn));
                assert!(matches!(
                    adapter.advance_input(&mut scheduler, first),
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
                    adapter.advance_input(&mut scheduler, second),
                    ProtocolAdapterSchedulerAdvance::ProtocolResidenceCompleted(_)
                ));
            })
            .await;
    }
}
