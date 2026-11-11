use std::collections::{HashMap, HashSet, VecDeque};

use moli_core::RendererOutputCursor;
use moli_protocol::{
    DeferredMainDocumentLoadObservationId, DeferredMainDocumentLoadPredecessorCandidate,
    ProtocolNavigationDependency, ProtocolSchedulerWork, ProtocolSchedulerWorkKind,
};

use super::ProtocolOutputSequence;

/// The next scheduler-owned transition for concrete protocol residence.
///
/// This is deliberately not a generic source signal. Every renderer
/// publication has already been frozen before this decision is made, and
/// the queue contains only frozen output or an exact browser-owner action.
/// The adapter may wait, close a provisional load-binding window, or
/// complete one ready residence. Ordinary queueing provides non-reentrance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProtocolSchedulerStep {
    /// No concrete residence may advance in this adapter turn.
    Wait,
    /// Close the provisional load-binding window after the biased command
    /// completion opportunity. This does not require a second adapter turn.
    CloseLoadBindingWindow,
    /// The selected concrete residence is ready for its single completion.
    CompleteReadyResidence,
}

#[derive(Debug)]
/// One durable, concrete protocol-scheduler residence.
///
/// An admitted publication reserves its original cursor while its owned
/// workflow projects the frozen records outside the actor. Completion fills
/// that same position with concrete protocol events. Work produced during
/// projection inherits the original source and exact load predecessors. The
/// queue owns no renderer source capability or permission to rescan Page state.
pub(super) enum ProtocolSchedulerResidence {
    /// A frozen renderer-produced event batch with exact output and load
    /// prerequisites.
    RendererOutputPublication(RendererOutputPublicationWork),
    /// A concrete browser-owner action or protocol continuation.
    ///
    /// The payload already owns its exact route and lifetime identity.
    ProtocolWork {
        work: ProtocolSchedulerWork,
        /// Exact Page/Document scope in which a load action published by the
        /// still-pending command turn may publish one exact load-observation
        /// prerequisite.
        ///
        /// This transient candidate exists only for concrete post-load work
        /// produced while consuming a renderer publication. It contains no
        /// renderer source capability. The next adapter turn closes it.
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
    /// Materialized protocol events, initially empty while the original
    /// projection is pending. Releasing them never rescans the renderer.
    pub(super) output: ProtocolOutputSequence,
    pub(super) insertion: Option<RendererOutputInsertion>,
    projection_pending: bool,
    delivery_predecessor: Option<RendererOutputCursor>,
    predecessors: Vec<moli_core::RendererOutputFence>,
    /// Exact load observations that must all terminate before `output` may be
    /// selected.
    pub(super) load_predecessors: Vec<DeferredMainDocumentLoadObservationId>,
    /// Exact Page/Document scope in which the still-pending command turn may
    /// publish the load observation that must precede this batch.
    ///
    /// The renderer publication has already been consumed and `output` is
    /// frozen. The next adapter turn closes this candidate when no matching
    /// load action was published.
    future_load_predecessor: Option<DeferredMainDocumentLoadPredecessorCandidate>,
}

/// An independent, complete Native publication inserted between this
/// projection's protocol events. Neither cursor has a partial delivery state.
#[derive(Debug)]
pub(super) struct RendererOutputInsertion {
    pub(super) boundary: moli_core::RendererOutputFence,
    pub(super) after: ProtocolOutputSequence,
}

impl RendererOutputPublicationWork {
    fn append_output(&mut self, output: ProtocolOutputSequence) {
        match &mut self.insertion {
            Some(insertion) => insertion.after.append(output),
            None => self.output.append(output),
        }
    }
}

impl ProtocolSchedulerResidence {
    pub(super) fn waits_for_command_output(&self) -> bool {
        matches!(self, Self::ProtocolWork { work, .. }
            if work.is_waiting_for_command_output())
    }

    pub(super) fn navigation_dependency(&self) -> ProtocolNavigationDependency {
        match self {
            // These events were already authorized and projected from a
            // concrete renderer publication. Their exact load and output
            // prerequisites control delivery. A new NavigationRequest on the
            // same target must not also suspend the active document's output
            // until that unrelated response arrives.
            Self::RendererOutputPublication(_) => ProtocolNavigationDependency::Independent,
            Self::ProtocolWork { work, .. } => work.navigation_dependency(),
        }
    }

    pub(super) fn has_open_load_binding_window(&self) -> bool {
        if self.waits_for_command_output() {
            return false;
        }
        match self {
            Self::RendererOutputPublication(work) => {
                !work.projection_pending && work.future_load_predecessor.is_some()
            }
            Self::ProtocolWork {
                future_load_predecessor,
                ..
            } => future_load_predecessor.is_some(),
        }
    }

    fn close_load_binding_window(&mut self) {
        if self.waits_for_command_output() {
            return;
        }
        match self {
            Self::RendererOutputPublication(work) => work.future_load_predecessor = None,
            Self::ProtocolWork {
                future_load_predecessor,
                ..
            } => *future_load_predecessor = None,
        }
    }

    fn renderer_insertion_boundary(&self) -> Option<(RendererOutputCursor, RendererOutputCursor)> {
        match self {
            Self::RendererOutputPublication(work) => work
                .insertion
                .as_ref()
                .map(|insertion| (work.renderer_output_cursor, insertion.boundary.cursor())),
            Self::ProtocolWork { .. } => None,
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
    pub(super) pending_loads: Vec<PendingMainDocumentLoadObservation>,
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
    /// Complete publications not yet handed to ordered output. Snapshot drains
    /// may temporarily remove residences, but do not satisfy delivery.
    undelivered_renderer_outputs: HashSet<RendererOutputCursor>,
}

impl Default for SchedulerQueues {
    fn default() -> Self {
        Self {
            protocol_residences: VecDeque::new(),
            pending_loads: Vec::new(),
            next_protocol_work_publish_sequence: 1,
            undelivered_renderer_outputs: HashSet::new(),
        }
    }
}

impl SchedulerQueues {
    pub(super) fn start_main_document_load(
        &mut self,
        observation_id: DeferredMainDocumentLoadObservationId,
        output_interest: moli_protocol::DeferredMainDocumentLoadCompletionOutputInterest,
    ) {
        assert!(
            !self
                .pending_loads
                .iter()
                .any(|pending| pending.observation_id == observation_id),
            "one exact load observation cannot be started twice"
        );
        self.pending_loads.push(PendingMainDocumentLoadObservation {
            observation_id,
            output_interest,
        });
    }

    pub(super) fn finish_main_document_load(
        &mut self,
        observation_id: DeferredMainDocumentLoadObservationId,
    ) -> bool {
        let Some(index) = self
            .pending_loads
            .iter()
            .position(|pending| pending.observation_id == observation_id)
        else {
            return false;
        };
        self.pending_loads.remove(index);
        self.satisfy_load_predecessor(observation_id);
        true
    }

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
        self.undelivered_renderer_outputs
            .insert(renderer_output_cursor);
        let delivery_predecessor = self.same_stream_insertion_predecessor(renderer_output_cursor);
        self.protocol_residences
            .push_back(ProtocolSchedulerResidence::RendererOutputPublication(
                RendererOutputPublicationWork {
                    renderer_output_cursor,
                    output,
                    insertion: None,
                    projection_pending: false,
                    delivery_predecessor,
                    predecessors: Vec::new(),
                    load_predecessors,
                    future_load_predecessor,
                },
            ));
    }

    pub(super) fn enqueue_pending_renderer_output_projection(
        &mut self,
        cursor: RendererOutputCursor,
        load_predecessors: Vec<DeferredMainDocumentLoadObservationId>,
        future_load_predecessor: Option<DeferredMainDocumentLoadPredecessorCandidate>,
    ) {
        if self.renderer_output_order(cursor).is_some() {
            return;
        }
        self.undelivered_renderer_outputs.insert(cursor);
        let delivery_predecessor = self.same_stream_insertion_predecessor(cursor);
        let position = self.protocol_residences.iter().position(|residence| matches!(residence,
            ProtocolSchedulerResidence::RendererOutputPublication(work)
                if work.renderer_output_cursor.stream() == cursor.stream() && work.renderer_output_cursor.sequence() > cursor.sequence()
        )).unwrap_or(self.protocol_residences.len());
        self.protocol_residences.insert(
            position,
            ProtocolSchedulerResidence::RendererOutputPublication(RendererOutputPublicationWork {
                renderer_output_cursor: cursor,
                output: ProtocolOutputSequence::empty(),
                insertion: None,
                projection_pending: true,
                delivery_predecessor,
                predecessors: Vec::new(),
                load_predecessors,
                future_load_predecessor,
            }),
        );
    }

    pub(super) fn renderer_output_order(
        &self,
        cursor: RendererOutputCursor,
    ) -> Option<(
        Vec<DeferredMainDocumentLoadObservationId>,
        Option<DeferredMainDocumentLoadPredecessorCandidate>,
    )> {
        self.protocol_residences
            .iter()
            .find_map(|residence| match residence {
                ProtocolSchedulerResidence::RendererOutputPublication(work)
                    if work.renderer_output_cursor == cursor =>
                {
                    Some((work.load_predecessors.clone(), work.future_load_predecessor))
                }
                _ => None,
            })
    }

    pub(super) fn complete_renderer_output_projection(
        &mut self,
        cursor: RendererOutputCursor,
        output: ProtocolOutputSequence,
    ) -> bool {
        let Some(work) =
            self.protocol_residences
                .iter_mut()
                .find_map(|residence| match residence {
                    ProtocolSchedulerResidence::RendererOutputPublication(work)
                        if work.renderer_output_cursor == cursor && work.projection_pending =>
                    {
                        Some(work)
                    }
                    _ => None,
                })
        else {
            return false;
        };
        work.projection_pending = false;
        let mut output = output;
        output.append(std::mem::replace(
            &mut work.output,
            ProtocolOutputSequence::empty(),
        ));
        work.output = output;
        true
    }

    pub(super) fn append_renderer_output_projection(
        &mut self,
        cursor: RendererOutputCursor,
        output: ProtocolOutputSequence,
    ) -> Result<(), ProtocolOutputSequence> {
        let Some(work) =
            self.protocol_residences
                .iter_mut()
                .find_map(|residence| match residence {
                    ProtocolSchedulerResidence::RendererOutputPublication(work)
                        if work.renderer_output_cursor == cursor =>
                    {
                        Some(work)
                    }
                    _ => None,
                })
        else {
            return Err(output);
        };
        work.append_output(output);
        Ok(())
    }

    pub(super) fn take_unconstrained_renderer_output(
        &mut self,
        cursor: RendererOutputCursor,
    ) -> Option<ProtocolOutputSequence> {
        if self.renderer_output_is_inserted(cursor) {
            return None;
        }
        let index = self.protocol_residences.iter().position(|residence| {
            matches!(residence, ProtocolSchedulerResidence::RendererOutputPublication(work)
                if work.renderer_output_cursor == cursor && !work.projection_pending
                    && work.insertion.is_none()
                    && !self.delivery_predecessor_is_pending(work.delivery_predecessor)
                    && work.predecessors.is_empty()
                    && work.load_predecessors.is_empty() && work.future_load_predecessor.is_none())
        })?;
        match self.protocol_residences.remove(index) {
            Some(ProtocolSchedulerResidence::RendererOutputPublication(work)) => {
                self.finish_renderer_output_delivery(work.renderer_output_cursor);
                Some(work.output)
            }
            _ => unreachable!("selected the original completed renderer output"),
        }
    }

    pub(super) fn renderer_delivery_is_pending(&self, cursor: RendererOutputCursor) -> bool {
        self.protocol_residences.iter().any(|residence| matches!(residence,
            ProtocolSchedulerResidence::RendererOutputPublication(work)
                if work.renderer_output_cursor == cursor && self.delivery_predecessor_is_pending(work.delivery_predecessor)
        ))
    }

    pub(super) fn bind_renderer_delivery_predecessor(
        &mut self,
        cursor: RendererOutputCursor,
        predecessor: Option<RendererOutputCursor>,
    ) {
        let insertion_predecessor = self.same_stream_insertion_predecessor(cursor);
        if let Some(work) =
            self.protocol_residences
                .iter_mut()
                .find_map(|residence| match residence {
                    ProtocolSchedulerResidence::RendererOutputPublication(work)
                        if work.renderer_output_cursor == cursor =>
                    {
                        Some(work)
                    }
                    _ => None,
                })
        {
            work.delivery_predecessor = insertion_predecessor
                .or(work.delivery_predecessor.filter(|earlier| {
                    earlier.stream() == cursor.stream() && earlier.sequence() < cursor.sequence()
                }))
                .or(predecessor);
        }
    }

    pub(super) fn finish_renderer_output_delivery(&mut self, cursor: RendererOutputCursor) {
        assert!(
            self.undelivered_renderer_outputs.remove(&cursor),
            "one complete publication is handed to output once"
        );
    }

    fn delivery_predecessor_is_pending(&self, predecessor: Option<RendererOutputCursor>) -> bool {
        predecessor.is_some_and(|cursor| self.undelivered_renderer_outputs.contains(&cursor))
    }

    fn same_stream_insertion_predecessor(
        &self,
        cursor: RendererOutputCursor,
    ) -> Option<RendererOutputCursor> {
        self.protocol_residences
            .iter()
            .filter_map(ProtocolSchedulerResidence::renderer_insertion_boundary)
            .filter(|(source, boundary)| {
                source.stream() == cursor.stream()
                    && source.sequence() < cursor.sequence()
                    && !inserted_cursor(cursor, *source, *boundary)
            })
            .map(|(source, _)| source)
            .max_by_key(|source| source.sequence())
    }

    fn bind_same_stream_insertion_successors(
        &mut self,
        source: RendererOutputCursor,
        boundary: RendererOutputCursor,
    ) {
        for residence in &mut self.protocol_residences {
            if let ProtocolSchedulerResidence::RendererOutputPublication(work) = residence
                && work.renderer_output_cursor.stream() == source.stream()
                && work.renderer_output_cursor.sequence() > source.sequence()
                && !inserted_cursor(work.renderer_output_cursor, source, boundary)
                && work.delivery_predecessor.is_none_or(|earlier| {
                    earlier.stream() != source.stream() || earlier.sequence() < source.sequence()
                })
            {
                // Same-stream successors share their source's creation
                // dependency. This chain retains it transitively while also
                // keeping their output after the complete insertion.
                work.delivery_predecessor = Some(source);
            }
        }
    }

    pub(super) fn renderer_output_source_is_pending(
        &self,
        residence: &ProtocolSchedulerResidence,
    ) -> bool {
        // Projection authorizes the facts, but post-load actions must also
        // wait for their producing batch to leave output residence. Specialized
        // drains must not turn "projected" into "delivered".
        let (source, requires_delivery) = match residence {
            ProtocolSchedulerResidence::RendererOutputPublication(work) => {
                return self.renderer_output_is_inserted(work.renderer_output_cursor)
                    || work.projection_pending
                    || self.delivery_predecessor_is_pending(work.delivery_predecessor);
            }
            ProtocolSchedulerResidence::ProtocolWork { work, .. } => (
                work.renderer_output_source(),
                work.kind() == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction,
            ),
        };
        let delivery = match residence {
            ProtocolSchedulerResidence::ProtocolWork { work, .. } => {
                work.renderer_delivery_predecessor()
            }
            _ => None,
        };
        if self.delivery_predecessor_is_pending(delivery) {
            return true;
        }
        if requires_delivery && self.delivery_predecessor_is_pending(source) {
            return true;
        }
        source.is_some_and(|source| self.protocol_residences.iter().any(|residence| matches!(residence,
            ProtocolSchedulerResidence::RendererOutputPublication(work)
                if work.renderer_output_cursor == source && (work.projection_pending || requires_delivery)
        )))
    }

    pub(super) fn residence_is_ready_to_complete(
        &self,
        residence: &ProtocolSchedulerResidence,
        conn: &moli_protocol::ProtocolWorkflowState<'_>,
    ) -> bool {
        self.residence_scheduler_step(residence, conn)
            == ProtocolSchedulerStep::CompleteReadyResidence
    }

    pub(super) fn residence_scheduler_step(
        &self,
        residence: &ProtocolSchedulerResidence,
        conn: &moli_protocol::ProtocolWorkflowState<'_>,
    ) -> ProtocolSchedulerStep {
        if residence.waits_for_command_output() || self.renderer_output_source_is_pending(residence)
        {
            return ProtocolSchedulerStep::Wait;
        }
        if let ProtocolSchedulerResidence::RendererOutputPublication(work) = residence {
            return self.publication_scheduler_step(work, conn, &mut HashSet::new());
        }
        if residence.has_open_load_binding_window() {
            ProtocolSchedulerStep::CloseLoadBindingWindow
        } else if let ProtocolSchedulerResidence::ProtocolWork {
            work,
            load_predecessors,
            ..
        } = residence
            && load_predecessors.is_empty()
            && conn.protocol_work_predecessor_is_projected(work)
        {
            ProtocolSchedulerStep::CompleteReadyResidence
        } else {
            ProtocolSchedulerStep::Wait
        }
    }

    fn publication_scheduler_step(
        &self,
        work: &RendererOutputPublicationWork,
        conn: &moli_protocol::ProtocolWorkflowState<'_>,
        visiting: &mut HashSet<RendererOutputCursor>,
    ) -> ProtocolSchedulerStep {
        if work.projection_pending
            || self.delivery_predecessor_is_pending(work.delivery_predecessor)
        {
            return ProtocolSchedulerStep::Wait;
        }
        if work.future_load_predecessor.is_some() {
            return ProtocolSchedulerStep::CloseLoadBindingWindow;
        }
        if !work.load_predecessors.is_empty()
            || !work
                .predecessors
                .iter()
                .all(|fence| conn.renderer_output_cursor_is_projected(fence.cursor()))
        {
            return ProtocolSchedulerStep::Wait;
        }
        if let Some(insertion) = &work.insertion {
            if !visiting.insert(work.renderer_output_cursor)
                || !conn.renderer_output_cursor_is_projected(insertion.boundary.cursor())
            {
                return ProtocolSchedulerStep::Wait;
            }
            for native in
                self.inserted_publications(work.renderer_output_cursor, insertion.boundary.cursor())
            {
                let step = self.publication_scheduler_step(native, conn, visiting);
                if step != ProtocolSchedulerStep::CompleteReadyResidence {
                    return step;
                }
            }
        }
        visiting.remove(&work.renderer_output_cursor);
        ProtocolSchedulerStep::CompleteReadyResidence
    }

    fn inserted_publications(
        &self,
        source: RendererOutputCursor,
        boundary: RendererOutputCursor,
    ) -> impl Iterator<Item = &RendererOutputPublicationWork> {
        self.protocol_residences
            .iter()
            .filter_map(move |residence| match residence {
                ProtocolSchedulerResidence::RendererOutputPublication(work)
                    if inserted_cursor(work.renderer_output_cursor, source, boundary) =>
                {
                    Some(work)
                }
                _ => None,
            })
    }

    fn renderer_output_is_inserted(&self, cursor: RendererOutputCursor) -> bool {
        self.protocol_residences.iter().any(|residence| {
            matches!(residence,
            ProtocolSchedulerResidence::RendererOutputPublication(work)
                if work.insertion.as_ref().is_some_and(|insertion|
                    inserted_cursor(cursor, work.renderer_output_cursor, insertion.boundary.cursor())))
        })
    }

    pub(super) fn take_inserted_renderer_output(
        &mut self,
        source: RendererOutputCursor,
        boundary: RendererOutputCursor,
    ) -> VecDeque<ProtocolSchedulerResidence> {
        // Use the same checkout rules as ordinary drains: a nested insertion
        // stays in the FIFO until its own source consumes it at that position.
        self.take_snapshot(|residence| {
            matches!(residence,
            ProtocolSchedulerResidence::RendererOutputPublication(work)
                if inserted_cursor(work.renderer_output_cursor, source, boundary))
        })
    }

    pub(super) fn admit_protocol_work_publish_sequence(
        &mut self,
        publish_sequence: moli_protocol::ProtocolWorkPublishSequence,
    ) {
        assert_eq!(
            publish_sequence.get(),
            self.next_protocol_work_publish_sequence,
            "protocol work must be admitted in exact publication order"
        );
        self.next_protocol_work_publish_sequence = self
            .next_protocol_work_publish_sequence
            .checked_add(1)
            .expect("scheduler protocol work publish sequence exhausted");
    }

    pub(super) fn enqueue_protocol_work(&mut self, work: ProtocolSchedulerWork) {
        self.admit_protocol_work_publish_sequence(work.publish_sequence());
        // A projection may materialize events behind an independent Native
        // fence. Keep those events and their full predecessors on the original
        // publication, so its delivery receipt cannot precede its creation fact.
        let work = if let Some(source) = work.renderer_output_source()
            && self.renderer_output_order(source).is_some()
        {
            match work.into_renderer_fenced_output() {
                Ok(output) => {
                    let insertion = output.insertion_boundary;
                    let insertion_cursor = insertion.as_ref().map(|fence| fence.cursor());
                    let predecessors = output.predecessors;
                    let events = output.events;
                    let publication = self
                        .protocol_residences
                        .iter_mut()
                        .find_map(|residence| match residence {
                            ProtocolSchedulerResidence::RendererOutputPublication(work)
                                if work.renderer_output_cursor == source =>
                            {
                                Some(work)
                            }
                            _ => None,
                        })
                        .expect("source publication retains its output");
                    let output = ProtocolOutputSequence::from_background_events(events);
                    if let Some(boundary) = insertion {
                        assert!(
                            publication.insertion.is_none(),
                            "one projection has one Native insertion"
                        );
                        publication.insertion = Some(RendererOutputInsertion {
                            boundary,
                            after: output,
                        });
                    } else {
                        publication.append_output(output);
                    }
                    publication
                        .predecessors
                        .extend(predecessors.into_iter().filter(|fence| {
                            fence.cursor() != source && Some(fence.cursor()) != insertion_cursor
                        }));
                    if let Some(boundary) = insertion_cursor {
                        self.bind_same_stream_insertion_successors(source, boundary);
                    }
                    return;
                }
                Err(work) => *work,
            }
        } else {
            work
        };
        let (load_predecessors, future_load_predecessor) = work
            .renderer_output_source()
            .and_then(|cursor| self.renderer_output_order(cursor))
            .unwrap_or_default();
        // A replacement must reach its owner before the pending load completes,
        // so it can cancel that load. Keep the action's owner checks, but
        // never make it a successor of that load.
        let (load_predecessors, future_load_predecessor) =
            if work.navigation_dependency() == ProtocolNavigationDependency::ReplacesPendingLoad {
                (Vec::new(), None)
            } else {
                (load_predecessors, future_load_predecessor)
            };
        let load_observation_id = work.main_document_load_observation_id();
        if load_observation_id.is_some()
            && let Some(source) = work.renderer_output_source()
        {
            self.close_load_publication_window(source);
        }
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
                load_predecessors,
                future_load_predecessor: if load_observation_id.is_none() {
                    future_load_predecessor
                } else {
                    None
                },
            },
        );
    }

    pub(super) fn front_has_open_load_binding_window(&self) -> bool {
        self.protocol_residences
            .front()
            .is_some_and(ProtocolSchedulerResidence::has_open_load_binding_window)
    }

    pub(super) fn close_front_load_binding_window(&mut self) {
        if !self.protocol_residences.is_empty() {
            self.close_load_binding_window_at(0);
        }
    }

    /// Satisfies a residence temporarily checked out by a specialized drain.
    ///
    /// The same scheduling boundary closes both retained candidates and the
    /// candidate carried by checked-out work.
    pub(super) fn close_checked_out_load_binding_window(
        &mut self,
        residence: &mut ProtocolSchedulerResidence,
    ) {
        self.close_future_load_predecessor_window();
        residence.close_load_binding_window();
    }

    /// Closes every provisional load-binding window admitted before the next
    /// adapter turn.
    ///
    /// Specialized command/load drains can temporarily check concrete work
    /// out of the main FIFO. They close the same window as the adapter;
    /// retained candidates must not bind an unrelated later load.
    pub(super) fn close_future_load_predecessor_window(&mut self) {
        // Entering the scheduler proves that the biased command-completion
        // opportunity has passed for every publication already admitted.
        // Close all provisional candidates, even when an older residence
        // without a candidate happens to be selected; otherwise a later
        // unrelated load could retroactively bind a candidate behind it.
        let pending = self
            .protocol_residences
            .iter()
            .filter_map(|residence| match residence {
                ProtocolSchedulerResidence::RendererOutputPublication(work)
                    if work.projection_pending =>
                {
                    Some(work.renderer_output_cursor)
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        for residence in &mut self.protocol_residences {
            match residence {
                ProtocolSchedulerResidence::RendererOutputPublication(work) => {
                    if !work.projection_pending {
                        work.future_load_predecessor = None;
                    }
                }
                ProtocolSchedulerResidence::ProtocolWork {
                    work,
                    future_load_predecessor,
                    ..
                } => {
                    if !work
                        .renderer_output_source()
                        .is_some_and(|source| pending.contains(&source))
                    {
                        *future_load_predecessor = None;
                    }
                }
            }
        }
    }

    /// The publication that produced Load must precede its post-load action;
    /// it cannot acquire that same action as a future load predecessor.
    pub(super) fn close_load_publication_window(&mut self, source: RendererOutputCursor) {
        for residence in &mut self.protocol_residences {
            if let ProtocolSchedulerResidence::RendererOutputPublication(work) = residence
                && work.renderer_output_cursor == source
            {
                work.future_load_predecessor = None;
            }
        }
    }

    pub(super) fn should_complete_next_residence(
        &self,
        conn: &moli_protocol::ProtocolWorkflowState<'_>,
    ) -> bool {
        self.protocol_residences
            .front()
            .is_some_and(|residence| self.residence_is_ready_to_complete(residence, conn))
    }

    pub(super) fn pop_next_protocol_residence(&mut self) -> Option<ProtocolSchedulerResidence> {
        self.protocol_residences.pop_front()
    }

    pub(super) fn close_load_binding_window_at(&mut self, index: usize) {
        self.close_future_load_predecessor_window();
        let residence = self
            .protocol_residences
            .get_mut(index)
            .expect("selected protocol residence must still exist");
        residence.close_load_binding_window();
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
        // Keep inserted Native publications available to their source even
        // when a specialized drain checks that source out of the main FIFO.
        let insertions = self
            .protocol_residences
            .iter()
            .filter_map(ProtocolSchedulerResidence::renderer_insertion_boundary)
            .collect::<Vec<_>>();
        let sources = self
            .protocol_residences
            .iter()
            .filter_map(|residence| match residence {
                ProtocolSchedulerResidence::RendererOutputPublication(work) => {
                    Some((work.renderer_output_cursor, work.projection_pending))
                }
                _ => None,
            })
            .fold(
                HashMap::<_, bool>::new(),
                |mut sources, (cursor, pending)| {
                    *sources.entry(cursor).or_default() |= pending;
                    sources
                },
            );
        let mut snapshot = VecDeque::new();
        let mut retained = VecDeque::with_capacity(self.protocol_residences.len());
        while let Some(residence) = self.protocol_residences.pop_front() {
            let pending = match &residence {
                ProtocolSchedulerResidence::RendererOutputPublication(work) => {
                    work.projection_pending
                        || insertions.iter().any(|(source, boundary)| {
                            inserted_cursor(work.renderer_output_cursor, *source, *boundary)
                        })
                }
                ProtocolSchedulerResidence::ProtocolWork { work, .. } => work
                    .renderer_output_source()
                    .and_then(|source| sources.get(&source))
                    .is_some_and(|projection_pending| {
                        *projection_pending
                            || work.kind() == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
                    }),
            };
            if !pending && selected(&residence) {
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
    /// the provisional load-binding window for those records. Exact load
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

#[derive(Debug)]
pub(super) struct PendingMainDocumentLoadObservation {
    pub(super) observation_id: DeferredMainDocumentLoadObservationId,
    pub(super) output_interest: moli_protocol::DeferredMainDocumentLoadCompletionOutputInterest,
}

fn inserted_cursor(
    cursor: RendererOutputCursor,
    source: RendererOutputCursor,
    boundary: RendererOutputCursor,
) -> bool {
    cursor != source
        && cursor.stream() == boundary.stream()
        && cursor.sequence() <= boundary.sequence()
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
                || work.navigation_dependency() == ProtocolNavigationDependency::ReplacesPendingLoad)
    )
}
