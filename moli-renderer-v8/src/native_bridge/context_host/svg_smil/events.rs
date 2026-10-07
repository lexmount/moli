//! Native SVG timing events use the Document rendering source. Timer wakes do
//! not dispatch script, and every event revalidates the exact Document owner.

use std::collections::{HashMap, VecDeque};

use moli_svg::{
    SvgAnimationEvent, SvgAnimationEventKind, SvgAnimationEvents, SvgAnimationInterval,
};

use super::super::{
    JsContextHost, OwnerDispatchScope, WindowDocumentTaskTarget,
    rendering_updates::PendingRenderingUpdatePayload,
};
use crate::{
    document_runtime::{DomHandle, EventTargetHandle},
    dom::native::{DomHost, DomMutationEffects},
    native_bridge::document::SVG_NS,
    page_task_queue::RendererPageRenderingUpdateTaskKind,
};

#[derive(Default)]
pub(super) struct SvgEventState {
    positions: HashMap<DomHandle, EventPosition>,
    roots: HashMap<DomHandle, (u64, Vec<DomHandle>)>,
    pending: HashMap<DomHandle, VecDeque<Vec<EventStream>>>,
}

#[derive(Clone, Copy)]
struct EventPosition {
    document: DomHandle,
    root: DomHandle,
    time: f64,
}

struct EventStream {
    animation: DomHandle,
    iterator: EventIterator,
    next: Option<SvgAnimationEvent>,
}

enum EventIterator {
    Natural(SvgAnimationEvents),
    Transition(std::vec::IntoIter<SvgAnimationEvent>),
}

impl EventStream {
    fn peek(&mut self) -> Option<SvgAnimationEvent> {
        if self.next.is_none() {
            self.next = match &mut self.iterator {
                EventIterator::Natural(events) => events.next(),
                EventIterator::Transition(events) => events.next(),
            };
        }
        self.next
    }

    fn next_non_repeat_time(&self) -> Option<f64> {
        if let Some(event) = self.next
            && !matches!(event.kind, SvgAnimationEventKind::Repeat(_))
        {
            return Some(event.time);
        }
        match &self.iterator {
            EventIterator::Natural(events) => events.next_non_repeat_time(),
            EventIterator::Transition(events) => events.as_slice().first().map(|event| event.time),
        }
    }

    fn skip_unobserved_repeats_before(&mut self, time: f64) {
        self.next = None;
        if let EventIterator::Natural(events) = &mut self.iterator {
            events.skip_repeats_before(time);
        }
    }
}

impl JsContextHost {
    pub(crate) fn svg_animation_mutation_documents(
        dom: &DomHost,
        effects: &DomMutationEffects,
    ) -> Vec<DomHandle> {
        fn contains_animation(dom: &DomHost, root: DomHandle) -> bool {
            let mut nodes = vec![root];
            while let Some(handle) = nodes.pop() {
                if super::is_animation_element(dom, handle) {
                    return true;
                }
                nodes.extend(dom.child_handles(handle));
            }
            false
        }
        let mut documents = Vec::new();
        let attributes = effects
            .style()
            .attribute_mutations()
            .iter()
            .filter_map(|mutation| {
                (dom.is_connected_to_document(mutation.target())
                    && super::is_animation_element(dom, mutation.target())
                    && mutation.namespace().is_none()
                    && matches!(
                        mutation.local_name(),
                        "begin" | "end" | "dur" | "repeatCount" | "repeatDur" | "restart"
                    ))
                .then_some(mutation.target())
            });
        let child_lists = effects
            .style()
            .child_list_mutations()
            .iter()
            .filter_map(|mutation| {
                (dom.is_connected_to_document(mutation.target())
                    && mutation
                        .added_nodes()
                        .iter()
                        .chain(mutation.removed_nodes())
                        .any(|&node| contains_animation(dom, node)))
                .then_some(mutation.target())
            });
        for target in attributes.chain(child_lists) {
            if let Some(document) = dom.owner_document_handle(target)
                && !documents.contains(&document)
            {
                documents.push(document);
            }
        }
        documents
    }

    fn svg_document_roots(&self, document: DomHandle) -> Vec<DomHandle> {
        let version = self.dom_host().dom_version();
        if let Some((cached_version, roots)) = self.svg_smil.borrow().events.roots.get(&document)
            && *cached_version == version
        {
            return roots.clone();
        }
        let mut roots = Vec::new();
        let mut nodes = vec![document];
        while let Some(handle) = nodes.pop() {
            if self.dom_host().node(handle).is_some_and(|node| {
                node.namespace() == Some(SVG_NS) && node.local_name() == Some("svg")
            }) && self.svg_animation_fragment(handle) == Some(handle)
            {
                roots.push(handle);
            }
            nodes.extend(self.dom_host().child_handles_reversed(handle));
        }
        self.svg_smil
            .borrow_mut()
            .events
            .roots
            .insert(document, (version, roots.clone()));
        roots
    }

    fn svg_fragment_animations(&self, root: DomHandle) -> Vec<DomHandle> {
        self.ensure_svg_fragment_effects(root);
        self.svg_smil
            .borrow()
            .effects
            .get(&root)
            .map(|effects| effects.animations.clone())
            .unwrap_or_default()
    }

    pub(super) fn initialize_svg_animation_event_positions(&self, root: DomHandle) {
        let Some(document) = self.dom_host().owner_document_handle(root) else {
            return;
        };
        let animations = self.svg_fragment_animations(root);
        let initial_time = self.with_svg_clock(root, |clock, _| clock.initial_time());
        let mut streams = Vec::new();
        let mut state = self.svg_smil.borrow_mut();
        for animation in animations {
            state.events.positions.insert(
                animation,
                EventPosition {
                    document,
                    root,
                    time: if initial_time == 0.0 {
                        -1.0
                    } else {
                        initial_time
                    },
                },
            );
            if initial_time > 0.0 {
                let timing = self.svg_animation_timing(animation);
                let instances = state.instances.get(&animation).cloned().unwrap_or_default();
                let active = timing.active_interval(&instances, initial_time);
                let events = SvgAnimationEvent::active_transition(None, active, initial_time)
                    .collect::<Vec<_>>();
                if !events.is_empty() {
                    streams.push(EventStream {
                        animation,
                        iterator: EventIterator::Transition(events.into_iter()),
                        next: None,
                    });
                }
            }
        }
        drop(state);
        self.push_svg_event_batch(root, streams);
    }

    pub(crate) fn queue_svg_animation_document_update(&mut self, document: DomHandle) {
        self.retire_disconnected_svg_animations(document);
        let Some(endpoint) = self.window_endpoint_for_document(document) else {
            return;
        };
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(endpoint.dispatch_scope())
        else {
            return;
        };
        let mut has_animation = false;
        for root in self.svg_document_roots(document) {
            let was_started = self
                .svg_smil
                .borrow()
                .clocks
                .get(&root)
                .is_some_and(|clock| clock.has_started());
            let started = self
                .with_svg_clock(root, |clock, now| {
                    let started = clock.has_started();
                    (started, clock.current_time(now))
                })
                .0;
            if started {
                if !was_started {
                    self.initialize_svg_animation_event_positions(root);
                }
                let animations = self.svg_fragment_animations(root);
                has_animation |= !animations.is_empty();
            }
        }
        if !has_animation
            && !self
                .svg_smil
                .borrow()
                .events
                .pending
                .contains_key(&document)
        {
            return;
        }
        self.queue_rendering_update(
            target,
            RendererPageRenderingUpdateTaskKind::SvgAnimationEvents,
            PendingRenderingUpdatePayload::SvgAnimationEvents(document),
        );
    }

    fn push_svg_event_batch(&self, root: DomHandle, streams: Vec<EventStream>) {
        if streams.is_empty() {
            return;
        }
        if let Some(document) = self.dom_host().owner_document_handle(root) {
            self.svg_smil
                .borrow_mut()
                .events
                .pending
                .entry(document)
                .or_default()
                .push_back(streams);
        }
    }

    pub(super) fn prepare_svg_fragment_events(&self, root: DomHandle) {
        let Some(document) = self.dom_host().owner_document_handle(root) else {
            return;
        };
        let time = self.svg_presentation_time(root);
        if !self.with_svg_clock(root, |clock, _| clock.has_started()) {
            return;
        }
        let animations = self.svg_fragment_animations(root);
        let mut streams = Vec::new();
        for animation in animations {
            let timing = self.svg_animation_timing(animation);
            let mut state = self.svg_smil.borrow_mut();
            let previous = state
                .events
                .positions
                .get(&animation)
                .copied()
                .filter(|position| position.root == root && position.document == document);
            let instances = state.instances.get(&animation).cloned().unwrap_or_default();
            let iterator = match previous {
                Some(previous) => {
                    EventIterator::Natural(timing.events_between(&instances, previous.time, time))
                }
                None => {
                    let after = timing.active_interval(&instances, time);
                    EventIterator::Transition(
                        SvgAnimationEvent::active_transition(None, after, time)
                            .collect::<Vec<_>>()
                            .into_iter(),
                    )
                }
            };
            state.events.positions.insert(
                animation,
                EventPosition {
                    document,
                    root,
                    time,
                },
            );
            drop(state);
            let mut stream = EventStream {
                animation,
                iterator,
                next: None,
            };
            if stream.peek().is_some() {
                streams.push(stream);
            }
        }
        self.push_svg_event_batch(root, streams);
    }

    pub(super) fn prepare_svg_seek_events(&self, root: DomHandle, before: f64, after: f64) {
        let Some(document) = self.dom_host().owner_document_handle(root) else {
            return;
        };
        if !self.with_svg_clock(root, |clock, _| clock.has_started()) {
            return;
        }
        let mut streams = Vec::new();
        for animation in self.svg_fragment_animations(root) {
            let timing = self.svg_animation_timing(animation);
            let mut state = self.svg_smil.borrow_mut();
            let instances = state.instances.get(&animation).cloned().unwrap_or_default();
            let events = SvgAnimationEvent::active_transition(
                timing.active_interval(&instances, before),
                timing.active_interval(&instances, after),
                after,
            )
            .collect::<Vec<_>>();
            state.events.positions.insert(
                animation,
                EventPosition {
                    document,
                    root,
                    time: after,
                },
            );
            if !events.is_empty() {
                streams.push(EventStream {
                    animation,
                    iterator: EventIterator::Transition(events.into_iter()),
                    next: None,
                });
            }
        }
        self.push_svg_event_batch(root, streams);
    }

    pub(super) fn prepare_svg_instance_events(
        &self,
        root: DomHandle,
        animation: DomHandle,
        before: Option<SvgAnimationInterval>,
        after: Option<SvgAnimationInterval>,
        time: f64,
    ) {
        if !self.with_svg_clock(root, |clock, _| clock.has_started()) {
            return;
        }
        let events = SvgAnimationEvent::active_transition(before, after, time).collect::<Vec<_>>();
        self.push_svg_event_batch(
            root,
            if events.is_empty() {
                Vec::new()
            } else {
                vec![EventStream {
                    animation,
                    iterator: EventIterator::Transition(events.into_iter()),
                    next: None,
                }]
            },
        );
    }

    fn has_svg_repeat_listener(&self, animation: DomHandle) -> bool {
        let path = self.build_propagation_path(EventTargetHandle::Node(animation), false);
        let window = match self.owner_dispatch_scope_for_node(animation) {
            Some(OwnerDispatchScope::Top) => EventTargetHandle::Window,
            Some(OwnerDispatchScope::Child(handle)) => {
                let Some(target) = self.current_child_window_event_target(handle) else {
                    return false;
                };
                EventTargetHandle::ChildWindow(target)
            }
            Some(OwnerDispatchScope::LightweightPopup(id)) => {
                let Some(target) = self.current_popup_window_event_target(id) else {
                    return false;
                };
                EventTargetHandle::PopupWindow(target)
            }
            None => return false,
        };
        path.into_iter()
            .map(|target| {
                if target == EventTargetHandle::Window {
                    window
                } else {
                    target
                }
            })
            .any(|target| match target {
                EventTargetHandle::ChildWindow(target) => {
                    self.child_window_has_event_listener(target, "repeatEvent")
                }
                _ => self.has_event_listener(target, "repeatEvent"),
            })
    }

    fn take_svg_event(&self, document: DomHandle) -> Option<(DomHandle, SvgAnimationEvent)> {
        let mut state = self.svg_smil.borrow_mut();
        let batches = state.events.pending.get_mut(&document)?;
        loop {
            let Some(streams) = batches.front_mut() else {
                state.events.pending.remove(&document);
                return None;
            };
            let index = streams
                .iter_mut()
                .enumerate()
                .filter_map(|(index, stream)| stream.peek().map(|event| (index, event)))
                .min_by(|(a, first), (b, second)| {
                    first
                        .time
                        .total_cmp(&second.time)
                        .then(first.kind.order().cmp(&second.kind.order()))
                        .then(a.cmp(b))
                })
                .map(|(index, _)| index);
            let Some(index) = index else {
                batches.pop_front();
                continue;
            };
            let event = streams[index].next.expect("selected event");
            if matches!(event.kind, SvgAnimationEventKind::Repeat(_))
                && !self.has_svg_repeat_listener(streams[index].animation)
            {
                // Begin/end handlers can add a repeat listener. Do not skip
                // beyond another stream's next opportunity to run author code.
                let until = streams
                    .iter()
                    .filter_map(EventStream::next_non_repeat_time)
                    .min_by(f64::total_cmp)
                    .unwrap_or(f64::INFINITY);
                streams[index].skip_unobserved_repeats_before(until);
                continue;
            }
            let stream = &mut streams[index];
            stream.next = None;
            return Some((stream.animation, event));
        }
    }

    fn svg_document_has_future_events(&self, document: DomHandle) -> bool {
        if self
            .svg_smil
            .borrow()
            .events
            .pending
            .contains_key(&document)
        {
            return true;
        }
        for root in self.svg_document_roots(document) {
            let current_time = self.svg_presentation_time(root);
            let through = if self.svg_animations_paused(root) {
                current_time
            } else {
                f64::INFINITY
            };
            for animation in self.svg_fragment_animations(root) {
                let timing = self.svg_animation_timing(animation);
                let state = self.svg_smil.borrow();
                // Callbacks and microtasks can cross a boundary after the
                // rendering source sampled it. Such a boundary still needs a
                // wake, even when it is now in the presentation clock's past.
                let time = state
                    .events
                    .positions
                    .get(&animation)
                    .filter(|position| position.root == root && position.document == document)
                    .map_or(current_time, |position| position.time);
                let instances = state.instances.get(&animation).cloned().unwrap_or_default();
                if timing
                    .events_between(&instances, time, through)
                    .next()
                    .is_some()
                {
                    return true;
                }
            }
        }
        false
    }

    pub(in crate::native_bridge::context_host) fn dispatch_authorized_svg_animation_events(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        target: WindowDocumentTaskTarget,
        expected_document: DomHandle,
    ) -> bool {
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            self.discard_svg_document_events(expected_document);
            return false;
        };
        if resolved.document_handle != expected_document {
            self.discard_svg_document_events(expected_document);
            return false;
        }
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let previous = target.dispatch_scope().enter(scope);
        let watchdog = crate::v8_execution_watchdog::V8ExecutionWatchdog::arm(
            crate::v8_execution_watchdog::V8ExecutionWatchdogKind::SvgAnimationEvents,
            scope.thread_safe_handle(),
            crate::v8_execution_watchdog::SCRIPT_TURN_WATCHDOG_TIMEOUT,
        );
        for root in self.svg_document_roots(expected_document) {
            self.prepare_svg_fragment_events(root);
        }
        let mut invoked = false;
        // Yield through the existing rendering source instead of monopolizing
        // a script turn when a long clock jump has many observed repetitions.
        for _ in 0..256 {
            if scope.is_execution_terminating() {
                break;
            }
            if !self.window_document_owner_is_current_for_dispatch_scope(
                target.owner(),
                target.dispatch_scope(),
            ) {
                self.discard_svg_document_events(expected_document);
                break;
            }
            let Some((animation, event)) = self.take_svg_event(expected_document) else {
                break;
            };
            // Already queued boundaries keep their target after removal. The
            // removal step terminates its interval and stops future sampling;
            // its begin/end tasks still belong to this exact Document.
            if self.dom_host().owner_document_handle(animation) != Some(expected_document) {
                continue;
            }
            let Some(event) = crate::context_bootstrap::construct_svg_time_event(scope, event.kind)
            else {
                continue;
            };
            let _ = self.dispatch_public_event_best_effort(
                scope,
                host_ptr,
                EventTargetHandle::Node(animation),
                event,
                "SVG timing event",
            );
            invoked = true;
        }
        if watchdog.disarm() == crate::v8_execution_watchdog::V8ExecutionWatchdogOutcome::TimedOut {
            tracing::warn!("SVG timing events exceeded their execution deadline");
        }
        if self.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        ) && self.svg_document_has_future_events(expected_document)
        {
            self.queue_svg_animation_rendering_wake(scope, target);
        }
        target.dispatch_scope().restore(scope, previous);
        invoked
    }

    pub(in crate::native_bridge::context_host) fn discard_svg_document_events(
        &self,
        document: DomHandle,
    ) {
        let mut state = self.svg_smil.borrow_mut();
        state.events.pending.remove(&document);
        state.events.roots.remove(&document);
        state
            .events
            .positions
            .retain(|_, position| position.document != document);
    }

    fn retire_disconnected_svg_animations(&self, document: DomHandle) {
        let current = |animation, root| {
            self.dom_host().is_connected_to_document(animation)
                && self.dom_host().owner_document_handle(animation) == Some(document)
                && self.svg_animation_fragment(animation) == Some(root)
        };
        let retired: Vec<_> = self
            .svg_smil
            .borrow()
            .events
            .positions
            .iter()
            .filter(|&(&animation, position)| {
                position.document == document && !current(animation, position.root)
            })
            .map(|(&animation, position)| (animation, *position))
            .collect();
        for (animation, position) in retired {
            let time = self.svg_presentation_time(position.root);
            let timing = self.svg_animation_timing(animation);
            let mut state = self.svg_smil.borrow_mut();
            let instances = state.instances.get(&animation).cloned().unwrap_or_default();
            state.events.positions.remove(&animation);
            let mut natural = EventStream {
                animation,
                iterator: EventIterator::Natural(timing.events_between(
                    &instances,
                    position.time,
                    time,
                )),
                next: None,
            };
            let active = timing.active_interval(&instances, time);
            let end = SvgAnimationEvent::active_transition(active, None, time).collect::<Vec<_>>();
            let batches = state.events.pending.entry(document).or_default();
            if natural.peek().is_some() {
                batches.push_back(vec![natural]);
            }
            if !end.is_empty() {
                batches.push_back(vec![EventStream {
                    animation,
                    iterator: EventIterator::Transition(end.into_iter()),
                    next: None,
                }]);
            }
            if batches.is_empty() {
                state.events.pending.remove(&document);
            }
        }
    }
}
