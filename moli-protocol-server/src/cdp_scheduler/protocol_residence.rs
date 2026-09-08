use std::collections::VecDeque;

use moli_core::RendererOutputCursor;
use moli_protocol::{
    DeferredMainDocumentLoadObservationId, DeferredMainDocumentLoadPredecessorCandidate,
    ProtocolSchedulerWork, ProtocolSchedulerWorkKind,
};

use super::ProtocolOutputSequence;

/// The next scheduler-owned transition for concrete protocol residence.
///
/// This is deliberately not a generic source signal. Every renderer
/// publication has already been frozen before this decision is made, and
/// the queue contains only frozen output or an exact browser-owner action.
/// The adapter may therefore either wait, satisfy the explicit client-turn
/// predecessor, or complete one ready residence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProtocolSchedulerStep {
    /// No concrete residence may advance in this adapter turn.
    Wait,
    /// The front residence must cross one adapter/client-turn boundary before
    /// it can be selected.
    SatisfyClientTurnPredecessor,
    /// The front concrete residence is ready for its single completion.
    CompleteReadyResidence,
}

#[derive(Debug)]
/// One durable, concrete protocol-scheduler residence.
///
/// A renderer publication is projected before it may enter this queue. Only
/// its frozen protocol events or concrete owner work can remain here. When the
/// publication was observed behind an exact main-document load, every
/// residence it produced inherits that explicit predecessor. The queue
/// therefore never owns a renderer source capability or permission to rescan
/// Page state.
pub(super) enum ProtocolSchedulerResidence {
    /// A frozen renderer-produced event batch whose remaining prerequisites
    /// are its client-turn boundary and, when present, exact main-document
    /// load observations.
    RendererOutputPublication(RendererOutputPublicationWork),
    /// A concrete browser-owner action or protocol continuation.
    ///
    /// The payload already owns its exact route and lifetime identity. The
    /// client-turn predecessor controls only when the scheduler may select it.
    ProtocolWork {
        work: ProtocolSchedulerWork,
        client_turn_predecessor: ClientTurnPredecessor,
        /// Exact Page/Document scope in which a load action published by the
        /// still-pending command turn may replace the provisional
        /// client-turn predecessor with one exact load-observation identity.
        ///
        /// This transient candidate exists only for concrete post-load work
        /// produced while consuming a renderer publication. It contains no
        /// renderer source capability, and the first client-turn yield closes
        /// it.
        future_load_predecessor: Option<DeferredMainDocumentLoadPredecessorCandidate>,
        /// Exact load observations inherited from the renderer publication
        /// that produced this work.
        ///
        /// This is empty for work published outside renderer-output ingress.
        load_predecessors: Vec<DeferredMainDocumentLoadObservationId>,
    },
}

#[derive(Debug)]
pub(super) struct RendererOutputPublicationWork {
    /// Exact concrete stream position whose records produced this batch.
    ///
    /// Stream-local sequencing and explicit cross-stream predecessors are the
    /// only renderer ordering authority. There is no process-global
    /// publication sequence and no source-shaped wake fallback.
    pub(super) renderer_output_cursor: RendererOutputCursor,
    /// Already-frozen protocol events. Releasing this value never rescans the
    /// renderer or asks a source capability to manufacture more work.
    pub(super) output: ProtocolOutputSequence,
    /// The first scheduler turn after ingress is reserved for higher-priority
    /// command completion.
    ///
    /// A command that publishes an exact load observation in that turn binds
    /// its identity below. If no such command completes, the ordinary client
    /// turn predecessor is the only remaining gate.
    client_turn_predecessor: ClientTurnPredecessor,
    /// Exact load observations that must all terminate before `output` may be
    /// selected.
    pub(super) load_predecessors: Vec<DeferredMainDocumentLoadObservationId>,
    /// Exact Page/Document scope in which the still-pending command turn may
    /// publish the load observation that must precede this batch.
    ///
    /// The renderer publication has already been consumed and `output` is
    /// frozen. The first client-turn yield closes this candidate when no
    /// matching load action was published.
    future_load_predecessor: Option<DeferredMainDocumentLoadPredecessorCandidate>,
}

/// Explicit scheduler predecessor for concrete protocol work that must not
/// share its producer's client-visible turn.
///
/// This is scheduler readiness only. The payload, route and browser-owner
/// responsibility already live in `ProtocolSchedulerWork`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClientTurnPredecessor {
    /// The producer's client-visible turn has not yet yielded.
    Pending,
    /// A distinct client turn may now select the concrete work.
    Satisfied,
}

impl ProtocolSchedulerResidence {
    pub(super) fn should_yield_to_client_turn(&self) -> bool {
        matches!(
            self,
            Self::RendererOutputPublication(RendererOutputPublicationWork {
                client_turn_predecessor: ClientTurnPredecessor::Pending,
                ..
            }) | Self::ProtocolWork {
                client_turn_predecessor: ClientTurnPredecessor::Pending,
                ..
            }
        )
    }

    pub(super) fn is_ready_to_complete(&self) -> bool {
        match self {
            Self::RendererOutputPublication(work) => {
                work.client_turn_predecessor == ClientTurnPredecessor::Satisfied
                    && work.load_predecessors.is_empty()
            }
            Self::ProtocolWork {
                client_turn_predecessor,
                load_predecessors,
                ..
            } => {
                *client_turn_predecessor == ClientTurnPredecessor::Satisfied
                    && load_predecessors.is_empty()
            }
        }
    }

    fn mark_client_turn_yielded(&mut self) {
        match self {
            Self::ProtocolWork {
                client_turn_predecessor,
                future_load_predecessor,
                ..
            } => {
                *client_turn_predecessor = ClientTurnPredecessor::Satisfied;
                *future_load_predecessor = None;
            }
            Self::RendererOutputPublication(work) => {
                work.client_turn_predecessor = ClientTurnPredecessor::Satisfied;
                work.future_load_predecessor = None;
            }
        }
    }

    fn future_load_predecessor(&self) -> Option<DeferredMainDocumentLoadPredecessorCandidate> {
        match self {
            Self::RendererOutputPublication(work) => work.future_load_predecessor,
            Self::ProtocolWork {
                future_load_predecessor,
                ..
            } => *future_load_predecessor,
        }
    }

    fn bind_load_predecessor(&mut self, observation_id: DeferredMainDocumentLoadObservationId) {
        let load_predecessors = match self {
            Self::RendererOutputPublication(work) => {
                work.future_load_predecessor = None;
                &mut work.load_predecessors
            }
            Self::ProtocolWork {
                future_load_predecessor,
                load_predecessors,
                ..
            } => {
                *future_load_predecessor = None;
                load_predecessors
            }
        };
        if !load_predecessors.contains(&observation_id) {
            load_predecessors.push(observation_id);
        }
    }

    fn satisfy_load_predecessor(&mut self, observation_id: DeferredMainDocumentLoadObservationId) {
        let load_predecessors = match self {
            Self::RendererOutputPublication(work) => &mut work.load_predecessors,
            Self::ProtocolWork {
                load_predecessors, ..
            } => load_predecessors,
        };
        load_predecessors.retain(|pending| *pending != observation_id);
    }
}

#[derive(Debug)]
pub(super) struct SchedulerQueues {
    /// Concrete protocol output and owner work in scheduler admission order.
    ///
    /// Renderer source publications are concrete before admission. A projected
    /// event batch may briefly accept the load observation still being
    /// published by the current command turn; once exact, concrete owner work
    /// from the same ingress turn inherits that predecessor as well. Publication
    /// stream cursor remains the admission invariant, while an exact
    /// predecessor may deliberately insert its load owner action before work
    /// that was admitted earlier. Selection normally follows this order; a
    /// target-local navigation gate may let an unrelated target advance while
    /// preserving order within each target lane.
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

    pub(super) fn enqueue_renderer_output_publication(
        &mut self,
        renderer_output_cursor: RendererOutputCursor,
        output: ProtocolOutputSequence,
        load_predecessors: Vec<DeferredMainDocumentLoadObservationId>,
        future_load_predecessor: Option<DeferredMainDocumentLoadPredecessorCandidate>,
    ) {
        assert!(
            !output.is_empty(),
            "only a concrete nonempty output batch may enter scheduler residence"
        );
        assert!(
            load_predecessors.is_empty() || future_load_predecessor.is_none(),
            "a concrete output batch cannot await both a future and an exact load predecessor"
        );
        self.protocol_residences
            .push_back(ProtocolSchedulerResidence::RendererOutputPublication(
                RendererOutputPublicationWork {
                    renderer_output_cursor,
                    output,
                    client_turn_predecessor: if future_load_predecessor.is_some() {
                        ClientTurnPredecessor::Pending
                    } else {
                        ClientTurnPredecessor::Satisfied
                    },
                    load_predecessors,
                    future_load_predecessor,
                },
            ));
    }

    pub(super) fn enqueue_protocol_work(
        &mut self,
        work: ProtocolSchedulerWork,
        load_predecessors: Vec<DeferredMainDocumentLoadObservationId>,
        future_load_predecessor: Option<DeferredMainDocumentLoadPredecessorCandidate>,
    ) {
        let publish_sequence = work.publish_sequence();
        assert_eq!(
            publish_sequence.get(),
            self.next_protocol_work_publish_sequence,
            "protocol work must enter scheduler residence in exact publication order"
        );
        self.next_protocol_work_publish_sequence = self
            .next_protocol_work_publish_sequence
            .checked_add(1)
            .expect("scheduler protocol work publish sequence exhausted");
        let load_observation_id = work.main_document_load_observation_id();
        assert!(
            load_predecessors.is_empty()
                || (future_load_predecessor.is_none() && load_observation_id.is_none()),
            "protocol work cannot await both a future and an exact load predecessor"
        );
        let insertion_index = match (
            load_observation_id,
            work.main_document_load_output_interest(),
        ) {
            (Some(observation_id), Some(interest)) => {
                self.bind_main_document_load_predecessor(observation_id, &interest)
            }
            (None, None) => self.protocol_residences.len(),
            _ => unreachable!("main-document load work must retain both identity and wake scope"),
        };
        self.protocol_residences.insert(
            insertion_index,
            ProtocolSchedulerResidence::ProtocolWork {
                work,
                client_turn_predecessor: ClientTurnPredecessor::Pending,
                load_predecessors,
                future_load_predecessor: if load_observation_id.is_none() {
                    future_load_predecessor
                } else {
                    None
                },
            },
        );
    }

    pub(super) fn front_needs_client_turn_predecessor(&self) -> bool {
        self.protocol_residences
            .front()
            .is_some_and(ProtocolSchedulerResidence::should_yield_to_client_turn)
    }

    pub(super) fn satisfy_front_client_turn_predecessor(&mut self) {
        self.close_future_load_predecessor_window();
        if let Some(residence) = self.protocol_residences.front_mut() {
            residence.mark_client_turn_yielded();
        }
    }

    /// Satisfies a residence temporarily checked out by a specialized drain.
    ///
    /// Keeping this operation on the queue makes closing retained provisional
    /// candidates inseparable from satisfying the checked-out work's turn
    /// predecessor.
    pub(super) fn satisfy_checked_out_client_turn_predecessor(
        &mut self,
        residence: &mut ProtocolSchedulerResidence,
    ) {
        self.close_future_load_predecessor_window();
        residence.mark_client_turn_yielded();
    }

    /// Closes every provisional load-binding window admitted before the next
    /// client-visible turn.
    ///
    /// Specialized command/load drains can temporarily check concrete work
    /// out of the main FIFO. They must call this at the same boundary where
    /// they satisfy that work's client-turn predecessor; otherwise candidates
    /// retained in the FIFO could outlive the very yield that closes them.
    pub(super) fn close_future_load_predecessor_window(&mut self) {
        // A client-turn yield proves that the biased command-completion
        // opportunity has passed for every publication already admitted.
        // Close all provisional candidates, even when an older residence
        // without a candidate happens to be at the front; otherwise a later
        // unrelated load could retroactively bind a candidate behind it.
        for residence in &mut self.protocol_residences {
            match residence {
                ProtocolSchedulerResidence::RendererOutputPublication(work) => {
                    work.future_load_predecessor = None;
                }
                ProtocolSchedulerResidence::ProtocolWork {
                    future_load_predecessor,
                    ..
                } => *future_load_predecessor = None,
            }
        }
    }

    pub(super) fn should_complete_next_residence(&self) -> bool {
        self.protocol_residences
            .front()
            .is_some_and(ProtocolSchedulerResidence::is_ready_to_complete)
    }

    pub(super) fn pop_next_protocol_residence(&mut self) -> Option<ProtocolSchedulerResidence> {
        self.protocol_residences.pop_front()
    }

    pub(super) fn satisfy_client_turn_predecessor_at(&mut self, index: usize) {
        self.close_future_load_predecessor_window();
        self.protocol_residences
            .get_mut(index)
            .expect("selected protocol residence must still exist")
            .mark_client_turn_yielded();
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

    pub(super) fn take_command_followup_snapshot(
        &mut self,
    ) -> VecDeque<ProtocolSchedulerResidence> {
        self.take_snapshot(protocol_residence_is_command_followup_activity)
    }

    /// Takes frozen output from the exact concrete Page stream up to one
    /// Runtime response cursor.
    ///
    /// Same-stream order has already been admitted by
    /// `OrderedRendererOutputIngress`; selecting `<= cursor` here closes only
    /// the provisional client-turn window for those records. Exact load
    /// predecessors remain authoritative and are never bypassed.
    pub(super) fn take_renderer_output_predecessor_snapshot(
        &mut self,
        predecessor: RendererOutputCursor,
    ) -> VecDeque<ProtocolSchedulerResidence> {
        self.close_future_load_predecessor_window();
        self.take_snapshot(|residence| {
            matches!(
                residence,
                ProtocolSchedulerResidence::RendererOutputPublication(work)
                    if work.renderer_output_cursor.stream() == predecessor.stream()
                        && work.renderer_output_cursor.sequence() <= predecessor.sequence()
                        && work.load_predecessors.is_empty()
            )
        })
    }

    pub(super) fn take_external_load_wait_snapshot(
        &mut self,
    ) -> VecDeque<ProtocolSchedulerResidence> {
        self.take_snapshot(protocol_residence_is_external_load_wait_activity)
    }

    pub(super) fn restore_snapshot_to_front(
        &mut self,
        mut snapshot: VecDeque<ProtocolSchedulerResidence>,
    ) {
        snapshot.append(&mut self.protocol_residences);
        self.protocol_residences = snapshot;
    }

    pub(super) fn satisfy_load_predecessor(
        &mut self,
        observation_id: DeferredMainDocumentLoadObservationId,
    ) {
        for residence in &mut self.protocol_residences {
            residence.satisfy_load_predecessor(observation_id);
        }
    }

    /// Makes a newly published load action the exact predecessor of concrete
    /// renderer output admitted while that command was still pending.
    ///
    /// Earlier protocol work retains its connection-local publication order.
    /// Every concrete residence admitted through the provisional
    /// command-completion boundary receives the exact observation identity
    /// before the load action is inserted ahead of it. Those residences stay
    /// blocked even while the async load action is checked out of this queue.
    pub(super) fn bind_main_document_load_predecessor(
        &mut self,
        observation_id: DeferredMainDocumentLoadObservationId,
        interest: &moli_protocol::DeferredMainDocumentLoadCompletionOutputInterest,
    ) -> usize {
        let Some(insertion_index) = self.protocol_residences.iter().position(|residence| {
            residence
                .future_load_predecessor()
                .is_some_and(|candidate| interest.observes_predecessor_candidate(candidate))
        }) else {
            return self.protocol_residences.len();
        };
        for residence in self.protocol_residences.iter_mut().skip(insertion_index) {
            if residence
                .future_load_predecessor()
                .is_some_and(|candidate| interest.observes_predecessor_candidate(candidate))
            {
                residence.bind_load_predecessor(observation_id);
            }
        }
        insertion_index
    }
}

fn protocol_residence_is_command_followup_activity(residence: &ProtocolSchedulerResidence) -> bool {
    matches!(
        residence,
        ProtocolSchedulerResidence::ProtocolWork {
            work,
            future_load_predecessor,
            load_predecessors,
            ..
        } if future_load_predecessor.is_none()
            && load_predecessors.is_empty()
            && work.is_command_followup()
    )
}

fn protocol_residence_is_external_load_wait_activity(
    residence: &ProtocolSchedulerResidence,
) -> bool {
    matches!(
        residence,
        ProtocolSchedulerResidence::ProtocolWork {
            work,
            future_load_predecessor,
            load_predecessors,
            ..
        }
            if future_load_predecessor.is_none()
                && load_predecessors.is_empty()
                && (work.is_root_frame_stopped_loading()
                || work.kind() == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
                || work.is_top_level_location_navigation_owner_action())
    )
}
