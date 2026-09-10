use super::workers::WorkerExecutionState;
use super::*;

impl JsContextHost {
    pub(crate) fn begin_ordinary_page_turn_navigation_handoff(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.ordinary_page_turn_navigation_handoff_active,
            "ordinary Page-turn navigation handoff scopes cannot overlap"
        );
        self.ordinary_page_turn_navigation_handoff_active = true;
        Ok(())
    }

    pub(crate) fn end_ordinary_page_turn_navigation_handoff(&mut self) {
        self.ordinary_page_turn_navigation_handoff_active = false;
    }

    pub(crate) fn handoff_ordinary_page_turn_navigation(
        &self,
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    ) {
        if self.ordinary_page_turn_navigation_handoff_active {
            self.top_level_navigation_handoff_tx.send(handoff);
        }
    }

    pub(crate) fn bind_output_journal(
        &mut self,
        output_journal: crate::runtime::RendererTurnOutputJournal,
    ) {
        if let Some(existing) = &self.output_journal {
            assert_eq!(
                existing.stream(),
                output_journal.stream(),
                "one JS context host cannot change renderer output streams"
            );
            return;
        }
        self.output_journal = Some(output_journal);
    }

    /// Appends a browser-owner action to the one concrete sink for this turn.
    ///
    /// Command turns retain a distinct response boundary, but they use the
    /// same `RendererOutputRecord` representation as ordinary Page turns.
    /// Returning `false` is reserved for standalone fixtures that have no
    /// concrete output sink; production must never write both the journal and
    /// a legacy pending queue.
    pub(crate) fn append_live_turn_owner_action(
        &self,
        action: crate::runtime::RendererOwnerAction,
    ) -> bool {
        self.append_owner_action_with_cause(self.active_runtime_command_cause.clone(), action)
    }

    /// Appends one already-frozen protocol fact to the current output owner.
    ///
    /// A renderer command has a response-local recorder while an ordinary Page
    /// task writes to the Page journal. Producers must choose here, when the
    /// fact is created; a later activity turn must never rediscover it by
    /// scanning cumulative renderer state.
    pub(crate) fn append_live_turn_observation(
        &self,
        observation: crate::runtime::RendererProtocolObservation,
    ) -> bool {
        let causal_command = self.active_runtime_command_cause.clone();
        if let Some(recorder) = self.command_turn_output.as_ref() {
            recorder.push_observation(causal_command, observation);
            return true;
        }
        let Some(output_journal) = self.output_journal.as_ref() else {
            return false;
        };
        output_journal.append(crate::runtime::PendingRendererOutputRecord::observation(
            causal_command,
            observation,
        ));
        true
    }

    /// Appends adjacent facts through one selected concrete output sink.
    ///
    /// Some Web operations synchronously produce both an observable protocol
    /// fact and a later browser-owner action. Choosing the sink once keeps
    /// those records adjacent and prevents one half from falling back to a
    /// legacy queue while the other half enters the concrete Page stream.
    pub(crate) fn append_live_turn_items(
        &self,
        items: impl IntoIterator<Item = crate::runtime::RendererOutputItem>,
    ) -> bool {
        let causal_command = self.active_runtime_command_cause.clone();
        let records = items.into_iter().map(|item| {
            crate::runtime::PendingRendererOutputRecord::from_parts(causal_command.clone(), item)
        });
        if let Some(recorder) = self.command_turn_output.as_ref() {
            for record in records {
                recorder.push_record(record);
            }
            return true;
        }
        let Some(output_journal) = self.output_journal.as_ref() else {
            return false;
        };
        output_journal.append_records(records);
        true
    }

    /// Appends an action whose causal command was frozen before the action
    /// crossed a later renderer-owner boundary.
    ///
    /// Top-level navigation is deliberately materialized only after lifecycle
    /// arbitration decides whether browser or renderer owns the request. The
    /// active dynamic V8 command scope may already have been restored by then,
    /// so the pending request's captured cause is authoritative.
    pub(crate) fn append_owner_action_with_cause(
        &self,
        causal_command: Option<crate::runtime::RendererRuntimeCommandCausalIdentity>,
        action: crate::runtime::RendererOwnerAction,
    ) -> bool {
        if let Some(recorder) = self.command_turn_output.as_ref() {
            recorder.push_owner_action(causal_command, action);
            return true;
        }
        let Some(output_journal) = self.output_journal.as_ref() else {
            return false;
        };
        output_journal.append(crate::runtime::PendingRendererOutputRecord::owner_action(
            causal_command,
            action,
        ));
        true
    }

    /// Publishes the concrete prefix required to resolve a synchronous owner
    /// suspension such as `alert()`/`confirm()`/`prompt()`.
    ///
    /// The command recorder is normally settled into the Page stream before
    /// the command completion is returned. A modal dialog blocks that return,
    /// so its prefix must be settled early. Draining the recorder, appending it
    /// to the same journal and allocating one stream sequence preserves the
    /// exact order of console/Inspector output that preceded the dialog.
    pub(crate) fn publish_live_turn_output_prefix(&self) -> bool {
        let Some(output_journal) = self.output_journal.as_ref() else {
            return false;
        };
        if let Some(recorder) = self.command_turn_output.as_ref() {
            output_journal.append_records(recorder.drain_records());
        }
        output_journal.publish_pending().is_some()
    }

    pub(crate) fn begin_command_turn_output(
        &mut self,
        recorder: crate::runtime::RendererCommandTurnOutputRecorder,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.command_turn_output.is_none(),
            "renderer command-turn output scopes cannot overlap"
        );
        self.command_turn_output = Some(recorder);
        Ok(())
    }

    pub(crate) fn end_command_turn_output(
        &mut self,
        recorder: &crate::runtime::RendererCommandTurnOutputRecorder,
    ) {
        if self
            .command_turn_output
            .as_ref()
            .is_some_and(|active| active.records_into_same_sink(recorder))
        {
            self.command_turn_output = None;
        }
    }

    fn layout_cache_generation_for_handle(&self, handle: DomHandle) -> u64 {
        let document = self
            .dom_host()
            .owner_document_handle(handle)
            .unwrap_or_else(|| self.document_handle());
        self.style_engine
            .computed_cache_generation_for_document(document)
    }

    pub(crate) fn dedicated_worker_loading_count_for_diagnostics(&self) -> usize {
        self.workers
            .values()
            .filter(|worker| matches!(worker.execution, WorkerExecutionState::Loading { .. }))
            .count()
    }

    pub(crate) fn dedicated_worker_running_worker_isolate_count_for_diagnostics(&self) -> usize {
        self.workers
            .values()
            .filter(|worker| matches!(worker.execution, WorkerExecutionState::Running { .. }))
            .count()
    }

    pub(crate) fn layout_metric_trace_snapshot(&self) -> LayoutMetricTrace {
        *self.layout_metric_trace.borrow()
    }

    pub(crate) fn record_client_rect_trace(&self, elapsed: std::time::Duration) {
        let mut trace = self.layout_metric_trace.borrow_mut();
        trace.client_rect_count = trace.client_rect_count.saturating_add(1);
        trace.client_rect_ns = trace.client_rect_ns.saturating_add(elapsed.as_nanos());
    }

    pub(crate) fn record_offset_parent_trace(&self, elapsed: std::time::Duration) {
        let mut trace = self.layout_metric_trace.borrow_mut();
        trace.offset_parent_count = trace.offset_parent_count.saturating_add(1);
        trace.offset_parent_ns = trace.offset_parent_ns.saturating_add(elapsed.as_nanos());
    }

    pub(crate) fn cached_mock_client_rect(
        &self,
        handle: DomHandle,
        compute: impl FnOnce(&Self, DomHandle) -> crate::native_bridge::element::ClientRect,
    ) -> crate::native_bridge::element::ClientRect {
        let generation = self.layout_cache_generation_for_handle(handle);
        if let Some((cached_generation, rect)) =
            self.layout_rect_cache.borrow().get(&handle).copied()
            && cached_generation == generation
        {
            return rect;
        }
        let rect = compute(self, handle);
        self.layout_rect_cache
            .borrow_mut()
            .insert(handle, (generation, rect));
        rect
    }

    pub(crate) fn cached_mock_flow_top(
        &self,
        handle: DomHandle,
        compute: impl FnOnce(&Self, DomHandle) -> f64,
    ) -> f64 {
        let generation = self.layout_cache_generation_for_handle(handle);
        if let Some((cached_generation, top)) =
            self.layout_flow_top_cache.borrow().get(&handle).copied()
            && cached_generation == generation
        {
            return top;
        }
        let top = compute(self, handle);
        self.layout_flow_top_cache
            .borrow_mut()
            .insert(handle, (generation, top));
        top
    }

    pub(crate) fn cached_mock_rendered_element(
        &self,
        handle: DomHandle,
        compute: impl FnOnce(&Self, DomHandle) -> bool,
    ) -> bool {
        let generation = self.layout_cache_generation_for_handle(handle);
        if let Some((cached_generation, rendered)) = self
            .layout_mock_rendered_element_cache
            .borrow()
            .get(&handle)
            .copied()
            && cached_generation == generation
        {
            return rendered;
        }
        let rendered = compute(self, handle);
        self.layout_mock_rendered_element_cache
            .borrow_mut()
            .insert(handle, (generation, rendered));
        rendered
    }

    pub(crate) fn cached_preceding_mock_flow_count(&self, handle: DomHandle) -> Option<usize> {
        let generation = self.layout_cache_generation_for_handle(handle);
        self.layout_preceding_flow_count_cache
            .borrow()
            .get(&handle)
            .filter(|(cached_generation, _)| *cached_generation == generation)
            .map(|(_, count)| *count)
    }

    pub(crate) fn cache_preceding_mock_flow_counts(
        &self,
        counts: impl IntoIterator<Item = (DomHandle, usize)>,
    ) {
        let mut cache = self.layout_preceding_flow_count_cache.borrow_mut();
        for (handle, count) in counts {
            let generation = self.layout_cache_generation_for_handle(handle);
            cache.insert(handle, (generation, count));
        }
    }

    pub(crate) fn cached_mock_flow_prefix_cursor(
        &self,
        parent: DomHandle,
    ) -> Option<(Option<DomHandle>, usize)> {
        let generation = self.layout_cache_generation_for_handle(parent);
        self.layout_flow_prefix_cursor_cache
            .borrow()
            .get(&parent)
            .filter(|(cached_generation, _, _)| *cached_generation == generation)
            .map(|(_, next_child, count)| (*next_child, *count))
    }

    pub(crate) fn cache_mock_flow_prefix_cursor(
        &self,
        parent: DomHandle,
        next_child: Option<DomHandle>,
        count: usize,
    ) {
        let generation = self.layout_cache_generation_for_handle(parent);
        self.layout_flow_prefix_cursor_cache
            .borrow_mut()
            .insert(parent, (generation, next_child, count));
    }

    #[cfg(test)]
    pub(crate) fn note_mock_flow_subtree_node_visit_for_test(&self) {
        self.layout_flow_subtree_node_visits
            .set(self.layout_flow_subtree_node_visits.get().saturating_add(1));
    }

    pub(crate) fn clear_layout_rect_cache(&self) {
        let profile_enabled = moli_trace::cpu_profile_enabled();
        let total_started = profile_enabled.then(std::time::Instant::now);
        let (rect_len, rect_capacity, rect_us) = {
            let mut cache = self.layout_rect_cache.borrow_mut();
            let len = cache.len();
            let capacity = cache.capacity();
            let started = profile_enabled.then(std::time::Instant::now);
            cache.clear();
            (
                len,
                capacity,
                started
                    .map(|started| started.elapsed().as_micros())
                    .unwrap_or_default(),
            )
        };
        let (flow_len, flow_capacity, flow_us) = {
            let mut cache = self.layout_flow_top_cache.borrow_mut();
            let len = cache.len();
            let capacity = cache.capacity();
            let started = profile_enabled.then(std::time::Instant::now);
            cache.clear();
            (
                len,
                capacity,
                started
                    .map(|started| started.elapsed().as_micros())
                    .unwrap_or_default(),
            )
        };
        let rendered_len = {
            let mut cache = self.layout_mock_rendered_element_cache.borrow_mut();
            let len = cache.len();
            cache.clear();
            len
        };
        let preceding_len = {
            let mut cache = self.layout_preceding_flow_count_cache.borrow_mut();
            let len = cache.len();
            cache.clear();
            len
        };
        let prefix_cursor_len = {
            let mut cache = self.layout_flow_prefix_cursor_cache.borrow_mut();
            let len = cache.len();
            cache.clear();
            len
        };
        if let Some(started) = total_started {
            let total_us = started.elapsed().as_micros();
            if total_us >= 500 {
                tracing::info!(
                    target: "moli_cpu_profile",
                    stage = "clear_layout_rect_cache",
                    rect_len,
                    rect_capacity,
                    rect_us,
                    flow_len,
                    flow_capacity,
                    flow_us,
                    rendered_len,
                    preceding_len,
                    prefix_cursor_len,
                    total_us,
                );
            }
        }
    }
}
