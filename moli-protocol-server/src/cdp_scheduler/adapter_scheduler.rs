use super::{CdpScheduler, ProtocolOutputSequence, protocol_residence::ProtocolSchedulerStep};
use moli_core::RendererOutputTransportMessage;
use tokio::sync::mpsc;

/// Coalesces later client turns for the concrete work shared by all frontends.
pub(crate) struct ProtocolAdapterScheduler {
    turn_tx: mpsc::UnboundedSender<()>,
    turn_rx: mpsc::UnboundedReceiver<()>,
    turn_scheduled: bool,
}

pub(crate) enum ProtocolAdapterSchedulerInput {
    Turn,
}

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
        let step = scheduler.next_protocol_scheduler_step();
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

    pub(crate) async fn ingest_renderer_publication(
        &mut self,
        scheduler: &mut CdpScheduler,
        publication: RendererOutputTransportMessage,
    ) -> ProtocolOutputSequence {
        scheduler
            .ingest_renderer_publication_for_scheduler(publication)
            .await
    }

    pub(crate) async fn advance_input(
        &mut self,
        scheduler: &mut CdpScheduler,
        input: ProtocolAdapterSchedulerInput,
    ) -> ProtocolAdapterSchedulerAdvance {
        let ProtocolAdapterSchedulerInput::Turn = input;
        self.turn_scheduled = false;
        match scheduler.next_protocol_scheduler_step() {
            ProtocolSchedulerStep::SatisfyClientTurnPredecessor => {
                scheduler.satisfy_front_protocol_residence_client_turn_predecessor();
                ProtocolAdapterSchedulerAdvance::ClientTurnYielded
            }
            ProtocolSchedulerStep::CompleteReadyResidence => {
                ProtocolAdapterSchedulerAdvance::ProtocolResidenceCompleted(
                    scheduler.complete_next_protocol_residence().await,
                )
            }
            ProtocolSchedulerStep::Wait => ProtocolAdapterSchedulerAdvance::Idle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_protocol::{
        CdpSchedulerEvent, ProtocolSchedulerWork, test_support::root_frame_stopped_loading_work,
    };
    use tokio::task::LocalSet;
    fn protocol_observation(publish_sequence: u64) -> ProtocolSchedulerWork {
        root_frame_stopped_loading_work(
            publish_sequence,
            vec![Some("SID-adapter".to_owned())],
            "FRAME-adapter".to_owned(),
            "LOADER-adapter".to_owned(),
        )
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
                let mut scheduler = CdpScheduler::new(moli_protocol::test_support::connection());
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
