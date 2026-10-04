use super::*;

impl JsContextHost {
    pub(crate) fn set_pointer_capture_active(&mut self, pointer_id: i32, active: bool) {
        if active {
            self.active_pointer_capture_ids.insert(pointer_id);
        } else {
            self.active_pointer_capture_ids.remove(&pointer_id);
        }
    }

    pub(crate) fn pointer_capture_is_active(&self, pointer_id: i32) -> bool {
        self.active_pointer_capture_ids.contains(&pointer_id)
    }

    pub(crate) fn set_pending_pointer_capture_target(
        &mut self,
        pointer_id: i32,
        target: DomHandle,
    ) {
        self.pending_pointer_capture_targets
            .insert(pointer_id, target);
    }

    pub(crate) fn release_pending_pointer_capture_target(&mut self, pointer_id: i32) {
        self.pending_pointer_capture_targets.remove(&pointer_id);
    }

    pub(crate) fn has_pending_pointer_capture_target(
        &self,
        pointer_id: i32,
        target: DomHandle,
    ) -> bool {
        self.pending_pointer_capture_targets
            .get(&pointer_id)
            .is_some_and(|current| *current == target)
    }

    pub(crate) fn active_pointer_capture_target(&self, pointer_id: i32) -> Option<DomHandle> {
        self.pointer_capture_targets
            .get(&pointer_id)
            .copied()
            .filter(|target| self.dom_host().is_connected(*target))
    }

    pub(crate) fn pointer_capture_target_is_connected(&self, target: DomHandle) -> bool {
        self.dom_host().is_connected(target)
    }

    pub(crate) fn clear_pointer_capture_target_if_matches(
        &mut self,
        pointer_id: i32,
        target: DomHandle,
    ) {
        if self
            .pointer_capture_targets
            .get(&pointer_id)
            .is_some_and(|current| *current == target)
        {
            self.pointer_capture_targets.remove(&pointer_id);
        }
        if self
            .pending_pointer_capture_targets
            .get(&pointer_id)
            .is_some_and(|pending| *pending == target)
        {
            self.pending_pointer_capture_targets.remove(&pointer_id);
        }
    }

    pub(crate) fn lost_pointer_capture_target_after_got(
        &mut self,
        pointer_id: i32,
        target: DomHandle,
    ) -> Option<DomHandle> {
        if self.dom_host().is_connected(target) {
            return None;
        }
        self.clear_pointer_capture_target_if_matches(pointer_id, target);
        Some(self.document_handle())
    }

    pub(crate) fn clear_pending_pointer_capture_targets_in_disconnected_subtree(
        &mut self,
        root: DomHandle,
    ) {
        if self.pending_pointer_capture_targets.is_empty() {
            return;
        }
        let pending_targets = self
            .pending_pointer_capture_targets
            .iter()
            .map(|(pointer_id, target)| (*pointer_id, *target))
            .collect::<Vec<_>>();
        for (pointer_id, target) in pending_targets {
            if self.pointer_capture_subtree_contains(root, target) {
                self.pending_pointer_capture_targets.remove(&pointer_id);
            }
        }
    }

    fn pointer_capture_subtree_contains(&self, root: DomHandle, target: DomHandle) -> bool {
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            if handle == target {
                return true;
            }
            if let Some(shadow_root) = self.dom_host().shadow_root_handle(handle) {
                stack.push(shadow_root);
            }
            stack.extend(self.dom_host().child_handles(handle));
        }
        false
    }

    pub(crate) fn process_pending_pointer_capture(
        &mut self,
        pointer_id: i32,
    ) -> Vec<PointerCaptureDispatchEvent> {
        let current_target = self.pointer_capture_targets.get(&pointer_id).copied();
        let pending_target = self
            .pending_pointer_capture_targets
            .get(&pointer_id)
            .copied()
            .filter(|target| self.dom_host().is_connected(*target));

        if pending_target.is_none() {
            self.pending_pointer_capture_targets.remove(&pointer_id);
        }

        if current_target == pending_target {
            return Vec::new();
        }

        let mut events = Vec::new();
        if let Some(current_target) = current_target {
            let target = if self.dom_host().is_connected(current_target) {
                current_target
            } else {
                self.document_handle()
            };
            self.pointer_capture_targets.remove(&pointer_id);
            events.push(PointerCaptureDispatchEvent {
                event_name: "lostpointercapture",
                target,
            });
        }

        if let Some(pending_target) = pending_target {
            self.pointer_capture_targets
                .insert(pointer_id, pending_target);
            events.push(PointerCaptureDispatchEvent {
                event_name: "gotpointercapture",
                target: pending_target,
            });
        }

        events
    }

    pub(crate) fn clear_pointer_capture_state(&mut self) {
        self.active_pointer_capture_ids.clear();
        self.pending_pointer_capture_targets.clear();
        self.pointer_capture_targets.clear();
    }
}
