use std::collections::HashSet;

use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    native_bridge::JsContextHost,
};

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct ParserPostStepRuntimeWorkForTest {
    custom_element_reaction_queue_flush_requested: bool,
    focus_resets: Vec<DomHandle>,
    child_browsing_context_drops: Vec<DomHandle>,
}

#[derive(Debug, Default)]
pub(in crate::document_runtime) struct ParserPostStepRuntimeWork {
    // This is intentionally not a mutation/reaction carrier. Parser DOM
    // mutations enqueue custom-element reactions immediately through the
    // runtime-owned CE queue; this queue only records work that must run after
    // the parser step exits and the default V8 context can be entered safely.
    custom_element_reaction_queue_flush_requested: bool,
    focus_resets: Vec<DomHandle>,
    child_browsing_context_drops: Vec<DomHandle>,
}

impl ParserPostStepRuntimeWork {
    pub(super) fn is_empty(&self) -> bool {
        !self.custom_element_reaction_queue_flush_requested
            && self.focus_resets.is_empty()
            && self.child_browsing_context_drops.is_empty()
    }

    fn request_custom_element_reaction_queue_flush(&mut self) {
        self.custom_element_reaction_queue_flush_requested = true;
    }

    fn queue_focus_reset(&mut self, handle: DomHandle) {
        if !self.focus_resets.contains(&handle) {
            self.focus_resets.push(handle);
        }
    }

    pub(super) fn queue_child_browsing_context_drop(&mut self, handle: DomHandle) {
        if !self.child_browsing_context_drops.contains(&handle) {
            self.child_browsing_context_drops.push(handle);
        }
    }

    fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    #[cfg(test)]
    fn extend(&mut self, other: Self) {
        self.custom_element_reaction_queue_flush_requested |=
            other.custom_element_reaction_queue_flush_requested;
        for handle in other.focus_resets {
            self.queue_focus_reset(handle);
        }
        for handle in other.child_browsing_context_drops {
            self.queue_child_browsing_context_drop(handle);
        }
    }

    #[cfg(test)]
    fn from_test(work: ParserPostStepRuntimeWorkForTest) -> Self {
        let (
            custom_element_reaction_queue_flush_requested,
            focus_resets,
            child_browsing_context_drops,
        ) = work.into_pending_parts();
        Self {
            custom_element_reaction_queue_flush_requested,
            focus_resets,
            child_browsing_context_drops,
        }
    }

    #[cfg(test)]
    fn into_test(self) -> ParserPostStepRuntimeWorkForTest {
        ParserPostStepRuntimeWorkForTest::from_pending_parts(
            self.custom_element_reaction_queue_flush_requested,
            self.focus_resets,
            self.child_browsing_context_drops,
        )
    }
}

#[cfg(test)]
impl ParserPostStepRuntimeWorkForTest {
    pub(crate) fn is_empty(&self) -> bool {
        !self.custom_element_reaction_queue_flush_requested
            && self.focus_resets.is_empty()
            && self.child_browsing_context_drops.is_empty()
    }

    pub(crate) fn merge_for_test(work_items: impl IntoIterator<Item = Self>) -> Self {
        let mut custom_element_reaction_queue_flush_requested = false;
        let mut focus_resets = Vec::new();
        let mut child_browsing_context_drops = Vec::new();
        for work in work_items {
            custom_element_reaction_queue_flush_requested |=
                work.custom_element_reaction_queue_flush_requested;
            focus_resets.extend(work.focus_resets);
            for handle in work.child_browsing_context_drops {
                if !child_browsing_context_drops.contains(&handle) {
                    child_browsing_context_drops.push(handle);
                }
            }
        }
        Self {
            custom_element_reaction_queue_flush_requested,
            focus_resets,
            child_browsing_context_drops,
        }
    }

    fn from_pending_parts(
        custom_element_reaction_queue_flush_requested: bool,
        focus_resets: Vec<DomHandle>,
        child_browsing_context_drops: Vec<DomHandle>,
    ) -> Self {
        Self {
            custom_element_reaction_queue_flush_requested,
            focus_resets,
            child_browsing_context_drops,
        }
    }

    fn into_pending_parts(self) -> (bool, Vec<DomHandle>, Vec<DomHandle>) {
        (
            self.custom_element_reaction_queue_flush_requested,
            self.focus_resets,
            self.child_browsing_context_drops,
        )
    }
}

impl DocumentRuntime {
    pub(super) fn queue_parser_post_step_focus_reset(&mut self, handle: DomHandle) {
        self.pending_parser_post_step_runtime_work
            .queue_focus_reset(handle);
    }

    pub(crate) fn ensure_parser_custom_element_reaction_queue(
        &mut self,
        host_ptr: *mut JsContextHost,
    ) {
        if !self.parser_reentry.custom_element_reaction_queue_active {
            custom_elements::push_parser_custom_element_reaction_queue(host_ptr);
            self.parser_reentry.custom_element_reaction_queue_active = true;
        }
        self.pending_parser_post_step_runtime_work
            .request_custom_element_reaction_queue_flush();
    }

    pub(crate) fn has_pending_parser_post_step_runtime_work(&self) -> bool {
        !self.pending_parser_post_step_runtime_work.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn take_pending_parser_post_step_runtime_work_for_test(
        &mut self,
    ) -> ParserPostStepRuntimeWorkForTest {
        self.pending_parser_post_step_runtime_work
            .take()
            .into_test()
    }

    #[cfg(test)]
    pub(crate) fn queue_pending_parser_post_step_runtime_work_for_test(
        &mut self,
        work: ParserPostStepRuntimeWorkForTest,
    ) {
        let work = ParserPostStepRuntimeWork::from_test(work);
        self.pending_parser_post_step_runtime_work.extend(work);
    }

    pub(crate) fn run_pending_parser_post_step_runtime_work(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) {
        let followups = self.pending_parser_post_step_runtime_work.take();
        self.run_parser_post_step_runtime_work(scope, host_ptr, &followups);
    }

    fn run_parser_post_step_runtime_work(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        followups: &ParserPostStepRuntimeWork,
    ) {
        if followups.is_empty() {
            return;
        }
        self.dispatch_parser_focus_resets(scope, host_ptr, &followups.focus_resets);
        if followups.custom_element_reaction_queue_flush_requested {
            crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections(
                scope,
            );
            custom_elements::flush_parser_custom_element_reaction_queue(scope, host_ptr);
            self.parser_reentry.custom_element_reaction_queue_active = false;
        }
        self.drop_child_browsing_context_subtrees(
            scope,
            host_ptr,
            &followups.child_browsing_context_drops,
        );
    }

    fn dispatch_parser_focus_resets(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handles: &[DomHandle],
    ) {
        if handles.is_empty() {
            return;
        }
        let mut visited = HashSet::new();
        for &handle in handles {
            if !visited.insert(handle) {
                continue;
            }
            if self.active_element_handle() == Some(handle) {
                crate::native_bridge::element::update_focus(scope, host_ptr, None);
            } else if self.document.active_element() == Some(handle) {
                crate::native_bridge::element::reset_focus_from_previous_handle(
                    scope, host_ptr, handle,
                );
            }
        }
    }

    pub(super) fn enqueue_parser_adoption_custom_element_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        lifecycle_connected_roots: &[DomHandle],
        custom_elements: &custom_elements::CustomElementAdoptionPlan,
    ) -> bool {
        let mut enqueued = false;
        if !lifecycle_connected_roots.is_empty()
            && self.enqueue_custom_element_disconnected_callbacks_in_subtrees(
                scope,
                host_ptr,
                lifecycle_connected_roots,
            )
        {
            enqueued = true;
        }
        custom_elements::apply_registry_association_retargets(
            host_ptr,
            &custom_elements.registry_retargets,
        );
        if custom_elements::enqueue_adopted_callbacks(scope, host_ptr, &custom_elements.targets) {
            enqueued = true;
        }
        enqueued
    }

    pub(super) fn enqueue_parser_form_state_custom_element_reactions(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) -> bool {
        if roots.is_empty() || unsafe { &*host_ptr }.custom_elements_subtree_lifecycle_quiescent() {
            return false;
        }
        let mut visited_roots = HashSet::new();
        let mut enqueued = false;
        for &root in roots {
            if visited_roots.insert(root) {
                if self.enqueue_custom_element_form_association_callbacks_in_subtree(
                    scope, host_ptr, root,
                ) {
                    enqueued = true;
                }
                if self.enqueue_custom_element_form_disabled_callbacks_in_subtree(
                    scope, host_ptr, root,
                ) {
                    enqueued = true;
                }
            }
        }
        enqueued
    }
}
