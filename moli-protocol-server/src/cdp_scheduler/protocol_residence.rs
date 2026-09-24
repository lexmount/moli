use super::ProtocolOutputSequence;
use moli_core::RendererOutputCursor;
use moli_protocol::ProtocolSchedulerWork;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProtocolSchedulerStep {
    Wait,
    SatisfyClientTurnPredecessor,
    CompleteReadyResidence,
}

/// Frozen output and exact owner continuations, never renderer source handles.
#[derive(Debug)]
pub(super) enum ProtocolSchedulerResidence {
    RendererOutputPublication(RendererOutputPublicationWork),
    ProtocolWork {
        work: ProtocolSchedulerWork,
        client_turn_predecessor: ClientTurnPredecessor,
    },
}

#[derive(Debug)]
pub(super) struct RendererOutputPublicationWork {
    pub(super) renderer_output_cursor: RendererOutputCursor,
    pub(super) output: ProtocolOutputSequence,
    client_turn_predecessor: ClientTurnPredecessor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClientTurnPredecessor {
    Pending,
    /// Page effects produced by a renderer publication yield to the current
    /// command completion before a specialized command/load drain may take them.
    PendingPublication,
    Satisfied,
}

impl ProtocolSchedulerResidence {
    fn client_turn_predecessor(&mut self) -> &mut ClientTurnPredecessor {
        match self {
            Self::RendererOutputPublication(work) => &mut work.client_turn_predecessor,
            Self::ProtocolWork {
                client_turn_predecessor,
                ..
            } => client_turn_predecessor,
        }
    }

    pub(super) fn should_yield_to_client_turn(&self) -> bool {
        !self.is_ready_to_complete()
    }

    pub(super) fn is_ready_to_complete(&self) -> bool {
        match self {
            Self::RendererOutputPublication(work) => {
                work.client_turn_predecessor == ClientTurnPredecessor::Satisfied
            }
            Self::ProtocolWork {
                client_turn_predecessor,
                ..
            } => *client_turn_predecessor == ClientTurnPredecessor::Satisfied,
        }
    }

    fn mark_client_turn_yielded(&mut self) {
        *self.client_turn_predecessor() = ClientTurnPredecessor::Satisfied;
    }
}

#[derive(Debug)]
pub(super) struct SchedulerQueues {
    pub(super) protocol_residences: VecDeque<ProtocolSchedulerResidence>,
    next_protocol_work_publish_sequence: u64,
}

impl Default for SchedulerQueues {
    fn default() -> Self {
        Self {
            protocol_residences: VecDeque::new(),
            next_protocol_work_publish_sequence: 1,
        }
    }
}

impl SchedulerQueues {
    pub(super) fn protocol_residence_len(&self) -> usize {
        self.protocol_residences.len()
    }

    pub(super) fn take_protocol_residence_at(
        &mut self,
        index: usize,
    ) -> Option<ProtocolSchedulerResidence> {
        self.protocol_residences.remove(index)
    }

    fn take_snapshot(
        &mut self,
        mut selected: impl FnMut(&ProtocolSchedulerResidence) -> bool,
    ) -> VecDeque<ProtocolSchedulerResidence> {
        let mut snapshot = VecDeque::new();
        let mut retained = VecDeque::with_capacity(self.protocol_residences.len());
        while let Some(residence) = self.protocol_residences.pop_front() {
            if selected(&residence) {
                snapshot.push_back(residence);
            } else {
                retained.push_back(residence);
            }
        }
        self.protocol_residences = retained;
        snapshot
    }

    pub(super) fn restore_snapshot_to_front(
        &mut self,
        mut snapshot: VecDeque<ProtocolSchedulerResidence>,
    ) {
        snapshot.append(&mut self.protocol_residences);
        self.protocol_residences = snapshot;
    }

    pub(super) fn enqueue_renderer_output_publication(
        &mut self,
        renderer_output_cursor: RendererOutputCursor,
        output: ProtocolOutputSequence,
    ) {
        assert!(
            !output.is_empty(),
            "only a concrete nonempty output batch may enter scheduler residence"
        );
        self.protocol_residences
            .push_back(ProtocolSchedulerResidence::RendererOutputPublication(
                RendererOutputPublicationWork {
                    renderer_output_cursor,
                    output,
                    client_turn_predecessor: ClientTurnPredecessor::PendingPublication,
                },
            ));
    }

    pub(super) fn enqueue_protocol_work(
        &mut self,
        work: ProtocolSchedulerWork,
        client_turn_predecessor: ClientTurnPredecessor,
    ) {
        assert_eq!(
            work.publish_sequence().get(),
            self.next_protocol_work_publish_sequence,
            "protocol work must enter scheduler residence in exact publication order"
        );
        self.next_protocol_work_publish_sequence = self
            .next_protocol_work_publish_sequence
            .checked_add(1)
            .expect("scheduler protocol work publish sequence exhausted");
        self.protocol_residences
            .push_back(ProtocolSchedulerResidence::ProtocolWork {
                work,
                client_turn_predecessor,
            });
    }

    /// The biased command-completion opportunity has passed. Each residence
    /// still owns its ordinary client-turn predecessor until selected.
    fn finish_renderer_admission_turn(&mut self) {
        for residence in &mut self.protocol_residences {
            let predecessor = residence.client_turn_predecessor();
            if *predecessor == ClientTurnPredecessor::PendingPublication {
                *predecessor = ClientTurnPredecessor::Pending;
            }
        }
    }

    pub(super) fn satisfy_checked_out_client_turn_predecessor(
        &mut self,
        residence: &mut ProtocolSchedulerResidence,
    ) {
        self.finish_renderer_admission_turn();
        residence.mark_client_turn_yielded();
    }

    pub(super) fn satisfy_client_turn_predecessor_at(&mut self, index: usize) {
        self.finish_renderer_admission_turn();
        self.protocol_residences
            .get_mut(index)
            .expect("selected protocol residence must still exist")
            .mark_client_turn_yielded();
    }

    pub(super) fn take_command_followup_snapshot(
        &mut self,
    ) -> VecDeque<ProtocolSchedulerResidence> {
        self.take_snapshot(|residence| matches!(residence,
            ProtocolSchedulerResidence::ProtocolWork { work, client_turn_predecessor }
                if *client_turn_predecessor != ClientTurnPredecessor::PendingPublication && work.is_command_followup()
        ))
    }

    /// Same-stream order is admitted before this queue. Select only the frozen
    /// output up to the exact cursor owned by the command response.
    pub(super) fn take_renderer_output_predecessor_snapshot(
        &mut self,
        predecessor: RendererOutputCursor,
    ) -> VecDeque<ProtocolSchedulerResidence> {
        self.finish_renderer_admission_turn();
        self.take_snapshot(|residence| {
            matches!(residence,
                ProtocolSchedulerResidence::RendererOutputPublication(work)
                    if work.renderer_output_cursor.stream() == predecessor.stream()
                        && work.renderer_output_cursor.sequence() <= predecessor.sequence()
            )
        })
    }

    pub(super) fn take_external_load_wait_snapshot(
        &mut self,
    ) -> VecDeque<ProtocolSchedulerResidence> {
        self.take_snapshot(|residence| matches!(residence,
            ProtocolSchedulerResidence::ProtocolWork { work, client_turn_predecessor }
                if *client_turn_predecessor != ClientTurnPredecessor::PendingPublication
                    && (work.is_root_frame_stopped_loading() || work.is_top_level_location_navigation_owner_action())
        ))
    }
}
