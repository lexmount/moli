use super::*;
use crate::host::HostTimerOwner;
use crate::host::PublicEventDispatchResult;
use crate::script_provenance::CompiledStringProvenance;

// This module is the first extraction slice from `document_runtime.rs`.
//
// The goal of this move is intentionally narrow: pull event delivery, public dispatch
// plumbing, and timer queue entrypoints into one place without changing ownership of
// `DocumentRuntime.events` or `DocumentRuntime.timeouts`, and without renaming the public
// methods that the rest of the runtime already calls.
//
// In other words, this is not yet a full "event runtime" abstraction. It is a boundary-
// shaping step that makes the next refactors cheaper:
// - callers still talk to `DocumentRuntime`
// - behavior remains anchored in the same state objects
// - but the code is no longer mixed into unrelated DOM / lifecycle / script logic
//
// That keeps this commit structural rather than semantic, which is the safest way to start
// shrinking `document_runtime.rs`.
impl DocumentRuntime {
    pub(crate) fn clear_event_state_for_document_replacement(
        &mut self,
        document_handle: DomHandle,
        clear_window: bool,
    ) -> std::collections::HashSet<crate::native_bridge::EventCallbackId> {
        // `Document::open()` clears the shadow-including current document tree,
        // the Document, and its Window. A node that was already detached is not
        // reached by that traversal and keeps its listeners.
        let dom_host = &self.dom_host;
        self.events.clear_targets_matching(|target| match target {
            EventTargetHandle::Window => clear_window,
            EventTargetHandle::ChildWindow(_) => clear_window,
            EventTargetHandle::Node(handle) => {
                handle == document_handle
                    || (dom_host.owner_document_handle(handle) == Some(document_handle)
                        && dom_host.is_connected(handle))
            }
        })
    }

    fn dispatch_error_already_reported(message: &str) -> bool {
        message.starts_with("event handler `") || message.starts_with("callback `")
    }

    pub(crate) fn dispatch_public_event_best_effort<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
        dispatch_target: EventTargetHandle,
        event: v8::Local<'s, v8::Object>,
        context: &str,
    ) -> std::result::Result<PublicEventDispatchResult, String> {
        match self.dispatch_public_event(scope, host_ptr, dispatch_target, event) {
            Ok(dispatched) => Ok(dispatched),
            Err(message) => {
                if !Self::dispatch_error_already_reported(&message) {
                    tracing::error!(
                        context,
                        message = message.as_str(),
                        "public event dispatch failed"
                    );
                }
                Err(message)
            }
        }
    }

    pub(crate) fn queue_timeout<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: v8::Local<'s, v8::Function>,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        self.timeouts
            .queue_once(scope, callback, delay_ms, owner, extra_args)
    }

    pub(crate) fn queue_timeout_with_receiver<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: v8::Local<'s, v8::Function>,
        receiver: v8::Local<'s, v8::Object>,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        self.timeouts
            .queue_once_with_receiver(scope, callback, receiver, delay_ms, owner, extra_args)
    }

    pub(crate) fn queue_window_timer_callback<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        target_receiver: v8::Local<'s, v8::Object>,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        self.timeouts.queue_window_timer_callback(
            scope,
            callback,
            target_receiver,
            delay_ms,
            owner,
            extra_args,
        )
    }

    pub(crate) fn queue_window_timer_callback_interval<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        target_receiver: v8::Local<'s, v8::Object>,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        self.timeouts.queue_window_timer_callback_interval(
            scope,
            callback,
            target_receiver,
            delay_ms,
            owner,
            extra_args,
        )
    }

    pub(crate) fn queue_window_animation_frame_callback<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        target_receiver: v8::Local<'s, v8::Object>,
        timestamp: f64,
        delay_ms: u32,
        owner: HostTimerOwner,
    ) -> u32 {
        self.timeouts.queue_window_animation_frame_callback(
            scope,
            callback,
            target_receiver,
            timestamp,
            delay_ms,
            owner,
        )
    }

    pub(crate) fn queue_window_idle_callback<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        target_receiver: v8::Local<'s, v8::Object>,
        timeout_deadline_ms: f64,
        delay_ms: u32,
        owner: HostTimerOwner,
    ) -> u32 {
        self.timeouts.queue_window_idle_callback(
            scope,
            callback,
            target_receiver,
            timeout_deadline_ms,
            delay_ms,
            owner,
        )
    }

    pub(crate) fn queue_window_geolocation_error_callback<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        geolocation: v8::Local<'s, v8::Object>,
        error: v8::Global<v8::Value>,
        owner: HostTimerOwner,
        watch_id: Option<i32>,
    ) -> u32 {
        self.timeouts.queue_window_geolocation_error_callback(
            scope,
            callback,
            geolocation,
            error,
            owner,
            watch_id,
        )
    }

    pub(crate) fn queue_source_timeout_with_receiver<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Context>,
        receiver: v8::Local<'s, v8::Object>,
        source: String,
        provenance: CompiledStringProvenance,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        self.timeouts.queue_source_once_with_receiver(
            scope, context, receiver, source, provenance, delay_ms, owner, extra_args,
        )
    }

    pub(crate) fn queue_source_interval_with_receiver<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Context>,
        receiver: v8::Local<'s, v8::Object>,
        source: String,
        provenance: CompiledStringProvenance,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        self.timeouts.queue_source_interval_with_receiver(
            scope, context, receiver, source, provenance, delay_ms, owner, extra_args,
        )
    }

    pub(crate) fn queue_resource_timing_buffer_full_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Context>,
        performance: v8::Local<'s, v8::Object>,
        buffer_id: crate::native_bridge::ResourceTimingBufferId,
    ) -> u32 {
        self.timeouts
            .queue_resource_timing_buffer_full(scope, context, performance, buffer_id)
    }

    pub(crate) fn cancel_timer(&mut self, id: u32) {
        self.timeouts.cancel(id);
    }

    pub(crate) fn cancel_window_timer_for_receiver<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        receiver: v8::Local<'s, v8::Object>,
        id: u32,
    ) -> bool {
        self.timeouts
            .cancel_window_timer_for_receiver(scope, receiver, id)
    }

    pub(crate) fn cancel_geolocation_watch<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        geolocation: v8::Local<'s, v8::Object>,
        watch_id: i32,
    ) -> bool {
        self.timeouts
            .cancel_geolocation_watch(scope, geolocation, watch_id)
    }

    pub(crate) fn cancel_window_execution_context_timers(
        &mut self,
        owner: crate::native_bridge::WindowExecutionContextOwner,
    ) -> usize {
        self.timeouts.cancel_window_execution_context_timers(owner)
    }

    pub(crate) fn cancel_timers_for_context_token(
        &mut self,
        context_token: crate::native_bridge::RuntimeObservableContextToken,
    ) -> usize {
        self.timeouts.cancel_timers_for_context_token(context_token)
    }

    pub(crate) fn run_next_timeout_body(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        selection: crate::page_task_queue::RendererPageTimerSelection,
    ) -> HostTimeoutRunResult {
        self.timeouts.run_next_body(scope, selection)
    }

    /// Starts the exact timer sequence range owned by one classic defer task.
    ///
    /// Parser-time scripts still execute inside the parser's task and module
    /// evaluation has its own pending-completion semantics. Neither category
    /// should make all of its older timers precede DOMContentLoaded. A classic
    /// defer task, however, can queue a timer before the subsequent DCL task is
    /// queued; record only that task-local interval.
    pub(crate) fn begin_classic_defer_timer_schedule_range(&mut self) {
        debug_assert!(
            self.classic_defer_timer_schedule_start.is_none(),
            "classic defer timer range must not overlap another script task"
        );
        self.classic_defer_timer_schedule_start = Some(self.timeouts.schedule_snapshot());
    }

    pub(crate) fn finish_classic_defer_timer_schedule_range(&mut self) {
        let Some(start) = self.classic_defer_timer_schedule_start.take() else {
            // A defer script may replace the document and clear the old
            // lifecycle state while it is executing.
            return;
        };
        let range = self.timeouts.schedule_range_since(start);
        if !range.is_empty() {
            self.classic_defer_timer_schedule_ranges.push(range);
        }
    }

    pub(crate) fn clear_classic_defer_timer_schedule_ranges(&mut self) {
        self.classic_defer_timer_schedule_start = None;
        self.classic_defer_timer_schedule_ranges.clear();
    }

    pub(crate) fn run_next_timeout_queued_by_classic_defer_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> HostTimeoutRunResult {
        self.timeouts
            .run_next_from_schedule_ranges(scope, &self.classic_defer_timer_schedule_ranges)
    }

    pub(crate) fn has_ready_timeout_queued_by_classic_defer_script(&self) -> bool {
        self.timeouts
            .has_ready_from_schedule_ranges(&self.classic_defer_timer_schedule_ranges)
    }

    #[cfg(test)]
    pub(crate) fn has_ready_timeout(&self) -> bool {
        self.timeouts.has_ready_timer()
    }

    pub(crate) fn next_ready_timeout_deadline(
        &self,
        selection: crate::page_task_queue::RendererPageTimerSelection,
    ) -> Option<std::time::Instant> {
        self.timeouts.next_ready_timer_deadline(selection)
    }

    pub(crate) fn ms_to_next_timeout(&self) -> Option<u64> {
        self.timeouts.ms_to_next()
    }

    pub(crate) fn next_timeout_deadline(&self) -> Option<std::time::Instant> {
        self.timeouts.next_deadline()
    }

    pub(crate) fn event_listener_callback_ids(
        &self,
        target: EventTargetHandle,
        event_type: &str,
        capture: bool,
    ) -> Vec<crate::native_bridge::EventCallbackId> {
        self.events
            .listener_callback_ids(target, event_type, capture)
    }

    pub(crate) fn inspector_event_listener_snapshots(
        &self,
        target: EventTargetHandle,
    ) -> Vec<crate::host::EventListenerInspectorSnapshot> {
        self.events.inspector_listener_snapshots(target)
    }

    pub(crate) fn insert_event_listener(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
        registration: crate::host::EventListenerRegistration,
    ) {
        if !self
            .events
            .has_event_handler_property_entry(target, event_type)
        {
            let content_attribute_owner =
                crate::host::event_handler_content_attribute_owner(self, target, event_type);
            self.events.ensure_event_handler_content_attribute(
                target,
                event_type,
                content_attribute_owner,
            );
        }
        self.events
            .insert_listener(target, event_type, registration);
    }

    pub(crate) fn remove_event_listener_by_id(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
        callback_id: crate::native_bridge::EventCallbackId,
        capture: bool,
    ) -> bool {
        self.events
            .remove_listener_by_id(target, event_type, callback_id, capture)
    }

    pub(crate) fn remove_event_callback_registrations(
        &mut self,
        retired: &std::collections::HashSet<crate::native_bridge::EventCallbackId>,
    ) {
        self.events.remove_callback_registrations(retired);
    }

    pub(crate) fn set_event_handler_property(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
        callback_id: Option<crate::native_bridge::EventCallbackId>,
    ) -> Option<crate::native_bridge::EventCallbackId> {
        self.events
            .set_event_handler_property(target, event_type, callback_id)
    }

    pub(crate) fn set_compiled_event_handler_property(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
        callback_id: Option<crate::native_bridge::EventCallbackId>,
    ) -> Option<crate::native_bridge::EventCallbackId> {
        self.events
            .set_compiled_event_handler_property(target, event_type, callback_id)
    }

    pub(crate) fn set_event_handler_content_attribute(
        &mut self,
        target: EventTargetHandle,
        event_type: &str,
        owner: Option<DomHandle>,
    ) -> Option<crate::native_bridge::EventCallbackId> {
        self.events
            .set_event_handler_content_attribute(target, event_type, owner)
    }

    pub(crate) fn sync_event_handler_content_attribute(
        &mut self,
        handle: DomHandle,
        name: &str,
        namespace: Option<&str>,
        present: bool,
    ) -> Option<crate::native_bridge::EventCallbackId> {
        if namespace.is_some() {
            return None;
        }
        let normalized_name = name.to_ascii_lowercase();
        let event_type = normalized_name
            .strip_prefix("on")
            .filter(|event_type| !event_type.is_empty())?;
        let event_type =
            crate::native_bridge::element::canonical_event_handler_event_type(event_type);
        let is_body_or_frameset = self.dom_host().node(handle).is_some_and(|node| {
            node.is_html_element_named("body") || node.is_html_element_named("frameset")
        });
        let target = match (
            is_body_or_frameset,
            crate::native_bridge::element::body_or_frameset_reflects_window_event_type(event_type),
        ) {
            (true, true)
                if self.dom_host().owner_document_handle(handle)
                    == Some(self.document_handle()) =>
            {
                EventTargetHandle::Window
            }
            (true, true) => return None,
            _ => EventTargetHandle::Node(handle),
        };
        self.set_event_handler_content_attribute(target, event_type, present.then_some(handle))
    }

    pub(crate) fn uncompiled_event_handler_content_attribute_owner(
        &self,
        target: EventTargetHandle,
        event_type: &str,
    ) -> Option<DomHandle> {
        self.events
            .uncompiled_event_handler_content_attribute_owner(target, event_type)
    }

    pub(crate) fn event_handler_property_callback_id(
        &self,
        target: EventTargetHandle,
        event_type: &str,
    ) -> Option<Option<crate::native_bridge::EventCallbackId>> {
        self.events
            .event_handler_property_callback_id(target, event_type)
    }

    pub(crate) fn has_event_listener(&self, target: EventTargetHandle, event_type: &str) -> bool {
        self.events.has_listener(target, event_type)
    }

    pub(crate) fn dispatch_document_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        event_type: &str,
    ) -> std::result::Result<(), String> {
        let (bubbles, cancelable) = host_event_defaults(event_type);
        let document_handle = self.document_handle();
        let document_target = EventTargetHandle::Node(document_handle);
        // `DOMContentLoaded` and similar document-scoped lifecycle events are observable from
        // both `document.addEventListener(...)` and `window.addEventListener(...)`.
        //
        // The older `dispatch_host_event(...)` helper only dispatches against a single concrete
        // target and does not walk the document -> window propagation path. That was good enough
        // for host-only events like `load` on `window`, but it is incorrect for document events:
        // `window.addEventListener('DOMContentLoaded', ...)` would never fire even though the
        // event defaults say it bubbles.
        //
        // Here we intentionally build a real propagation path rooted at `document`, create a host
        // event whose original target is also `document`, and then route it through the same
        // public-event dispatch path used for DOM events. That keeps `event.target === document`
        // while still letting the listener bubble up to `window`.
        let propagation_path = self.build_propagation_path(document_target, false);
        let document_value = event_target_value(scope, host_ptr, document_target)?;
        let event = create_host_event(
            scope,
            event_type,
            document_value,
            document_value,
            bubbles,
            cancelable,
        )?;
        dispatch_public_event_with_original_target(
            &mut self.events,
            scope,
            host_ptr,
            document_target,
            document_target,
            &propagation_path,
            event,
        )
        .map(|_| ())
    }

    pub(crate) fn dispatch_public_event<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
        dispatch_target: EventTargetHandle,
        event: v8::Local<'s, v8::Object>,
    ) -> std::result::Result<PublicEventDispatchResult, String> {
        let event_type = public_event_type(scope, event);
        let composed = event
            .get(scope, v8str(scope, "composed").into())
            .is_some_and(|value| value.boolean_value(scope));
        // A caller-created composed event is not confined to the tree that
        // contains its reference source. Its path crosses shadow boundaries
        // normally while `source` is retargeted for each listener.
        let mut path = if composed {
            self.build_propagation_path(dispatch_target, composed)
        } else if let Some(source_target) =
            source_target_for_reference_event(scope, host_ptr, event)
        {
            self.build_source_scoped_propagation_path(dispatch_target, source_target)
        } else {
            self.build_propagation_path(dispatch_target, composed)
        };
        self.append_child_document_window_to_path(scope, host_ptr, &mut path);
        trim_window_from_subresource_load_path(dispatch_target, event_type.as_deref(), &mut path);
        dispatch_public_event(
            &mut self.events,
            scope,
            host_ptr,
            dispatch_target,
            &path,
            event,
        )
    }

    pub(crate) fn dispatch_public_event_with_original_target<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
        dispatch_target: EventTargetHandle,
        original_target: EventTargetHandle,
        event: v8::Local<'s, v8::Object>,
    ) -> std::result::Result<PublicEventDispatchResult, String> {
        let event_type = public_event_type(scope, event);
        let composed = event
            .get(scope, v8str(scope, "composed").into())
            .is_some_and(|value| value.boolean_value(scope));
        let mut path = self.build_propagation_path(dispatch_target, composed);
        self.append_child_document_window_to_path(scope, host_ptr, &mut path);
        trim_window_from_subresource_load_path(dispatch_target, event_type.as_deref(), &mut path);
        dispatch_public_event_with_original_target(
            &mut self.events,
            scope,
            host_ptr,
            dispatch_target,
            original_target,
            &path,
            event,
        )
    }

    pub(crate) fn build_propagation_path(
        &self,
        target: EventTargetHandle,
        composed: bool,
    ) -> Vec<EventTargetHandle> {
        // Returns [target, direct_parent_or_shadow_root, ..., Window] (ascending).
        let mut path = vec![target];
        if let EventTargetHandle::Node(mut current) = target {
            loop {
                if let Some(slot) = self.dom_host.assigned_slot_for_node(current) {
                    path.push(EventTargetHandle::Node(slot));
                    current = slot;
                    continue;
                }
                if let Some(parent) = self.parent_node(current) {
                    path.push(EventTargetHandle::Node(parent));
                    current = parent;
                    continue;
                }
                if self.dom_host.is_shadow_root(current) {
                    if !composed {
                        let Some(host) = self.dom_host.shadow_root_host(current) else {
                            break;
                        };
                        let EventTargetHandle::Node(original_target) = target else {
                            break;
                        };
                        if !self.light_tree_contains(host, original_target) {
                            break;
                        }
                        path.push(EventTargetHandle::Node(host));
                        current = host;
                        continue;
                    }
                    let Some(host) = self.dom_host.shadow_root_host(current) else {
                        break;
                    };
                    path.push(EventTargetHandle::Node(host));
                    current = host;
                    continue;
                }
                break;
            }
        }
        let ended_at_shadow_root = matches!(
            path.last(),
            Some(EventTargetHandle::Node(handle)) if self.dom_host.is_shadow_root(*handle)
        );
        let ended_at_connected_tree = matches!(
            path.last(),
            Some(EventTargetHandle::Node(handle)) if self.dom_host.is_connected(*handle)
        );
        if !target.is_window() && !ended_at_shadow_root && ended_at_connected_tree {
            path.push(EventTargetHandle::Window);
        }
        path
    }

    fn append_child_document_window_to_path(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        path: &mut Vec<EventTargetHandle>,
    ) {
        if path
            .iter()
            .any(|target| matches!(target, EventTargetHandle::ChildWindow(_)))
        {
            return;
        }
        let trailing_main_window = path.last() == Some(&EventTargetHandle::Window);
        let document_target = if trailing_main_window {
            path.get(path.len().saturating_sub(2)).copied()
        } else {
            path.last().copied()
        };
        let Some(EventTargetHandle::Node(document_handle)) = document_target else {
            return;
        };
        if !self
            .dom_host()
            .node(document_handle)
            .is_some_and(moli_dom::native::Node::is_document)
        {
            return;
        }
        let host = unsafe { &*host_ptr };
        if let Some(child_handle) =
            host.child_browsing_context_handle_by_document_handle(scope, document_handle)
            && let Some(target) = host.current_child_window_event_target(child_handle)
        {
            if trailing_main_window {
                path.pop();
            }
            path.push(EventTargetHandle::ChildWindow(target));
        }
    }

    fn build_source_scoped_propagation_path(
        &self,
        target: EventTargetHandle,
        source: EventTargetHandle,
    ) -> Vec<EventTargetHandle> {
        let full_path = self.build_propagation_path(target, true);
        let (EventTargetHandle::Node(target_handle), EventTargetHandle::Node(source_handle)) =
            (target, source)
        else {
            return full_path;
        };
        let Some(source_root) = self.dom_host.root_node_handle(source_handle) else {
            return full_path;
        };
        let source_root_is_shadow = self.dom_host.is_shadow_root(source_root);
        let source_root_is_document = source_root == self.document_handle();
        let mut filtered = Vec::new();
        for entry in full_path {
            let include = match entry {
                EventTargetHandle::Window | EventTargetHandle::ChildWindow(_) => {
                    source_root_is_document
                }
                EventTargetHandle::Node(handle) => {
                    self.dom_host.root_node_handle(handle) == Some(source_root)
                        || self.flat_tree_contains_for_event_path(handle, target_handle)
                }
            };
            if include {
                filtered.push(entry);
            }
            if source_root_is_shadow && entry == EventTargetHandle::Node(source_root) {
                break;
            }
        }
        if filtered.is_empty() {
            self.build_propagation_path(target, false)
        } else {
            filtered
        }
    }

    fn flat_tree_contains_for_event_path(&self, root: NativeNodeId, target: NativeNodeId) -> bool {
        let mut stack = vec![root];
        let mut visited = Vec::new();
        while let Some(handle) = stack.pop() {
            if handle == target {
                return true;
            }
            if visited.contains(&handle) {
                continue;
            }
            visited.push(handle);
            if self.dom_host.is_html_element_named(handle, "slot")
                && self.dom_host.containing_shadow_root(handle).is_some()
            {
                stack.extend(
                    self.dom_host
                        .assigned_nodes_for_slot_with_options(handle, true)
                        .into_iter()
                        .rev(),
                );
                continue;
            }
            let mut child = self.dom_host.first_child(handle);
            while let Some(child_handle) = child {
                stack.push(child_handle);
                child = self.dom_host.next_sibling(child_handle);
            }
        }
        false
    }

    fn light_tree_contains(&self, root: NativeNodeId, handle: NativeNodeId) -> bool {
        let mut current = Some(handle);
        while let Some(candidate) = current {
            if candidate == root {
                return true;
            }
            current = self
                .dom_host
                .node(candidate)
                .and_then(moli_dom::native::Node::parent_node);
        }
        false
    }
}

fn public_event_type(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) -> Option<String> {
    object_string_property(scope, event, "type")
}

fn trim_window_from_subresource_load_path(
    dispatch_target: EventTargetHandle,
    event_type: Option<&str>,
    path: &mut Vec<EventTargetHandle>,
) {
    // Chromium keeps element/resource `load` events off Window even for capture
    // listeners. The document/window lifecycle load is dispatched with Window
    // as the concrete target and must keep its Window target path.
    if event_type != Some("load") || dispatch_target.is_window() {
        return;
    }
    if path.last().is_some_and(|target| target.is_window()) {
        path.pop();
    }
}

fn source_target_for_reference_event(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    event: v8::Local<'_, v8::Object>,
) -> Option<EventTargetHandle> {
    let event_type = event
        .get(scope, v8str(scope, "type").into())?
        .to_string(scope)?
        .to_rust_string_lossy(scope);
    let source_property = match event_type.as_str() {
        "beforetoggle" | "command" | "interest" | "loseinterest" | "toggle" => "source",
        "submit" => "submitter",
        _ => return None,
    };
    let source = event.get(scope, v8str(scope, source_property).into())?;
    if source.is_null_or_undefined() || !source.is_object() {
        return None;
    }
    let object = v8::Local::<v8::Object>::try_from(source).ok()?;
    let (runtime_ptr, handle) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, object).ok()?;
    (runtime_ptr == host_ptr).then_some(EventTargetHandle::Node(handle))
}
