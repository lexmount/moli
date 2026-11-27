use std::{collections::HashMap, time::Instant};

use anyhow::{Result, anyhow};
use moli_action_window::{
    ActionBarrier, ActionBatch, ActionSequence, ActionWindow, PlannedAction, WindowAction,
};
use moli_input::{InputModifiers, Point, ScrollAction, ScrollDeltaMode};

use crate::{
    page_task_queue::RendererPageReadyDescriptor,
    runtime::{
        RendererDocumentLifecycleIdentity, RendererInputDispatchOutcome,
        RendererPointerEventProperties,
    },
    script_vm::MouseDispatchCompletion,
};

use super::PageVm;

#[derive(Debug, PartialEq)]
struct WheelEventProperties {
    button: i32,
    buttons: Option<i32>,
    click_count: i32,
    pointer: RendererPointerEventProperties,
}

pub(super) struct RendererPageActionWindow {
    window: ActionWindow<RendererDocumentLifecycleIdentity>,
    wheel_events: HashMap<ActionSequence, WheelEventProperties>,
}

/// Input admission and layout refresh are separate decisions. Realtime uses
/// normal input completion; legacy clients keep their existing wheel window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum RendererInputDispatchPolicy {
    LegacyWindow,
    Interactive,
}

impl Default for RendererPageActionWindow {
    fn default() -> Self {
        Self {
            window: ActionWindow::new(),
            wheel_events: HashMap::new(),
        }
    }
}

impl PageVm {
    pub(in crate::runtime) fn input_dispatch_policy(&self) -> RendererInputDispatchPolicy {
        match self.vm().layout_policy() {
            moli_page_types::LayoutPolicy::Mock | moli_page_types::LayoutPolicy::Lazy => {
                RendererInputDispatchPolicy::LegacyWindow
            }
            moli_page_types::LayoutPolicy::Realtime => RendererInputDispatchPolicy::Interactive,
        }
    }

    pub(in crate::runtime) fn next_action_window_deadline(&self) -> Option<Instant> {
        [
            self.page_action_window.window.next_deadline(),
            self.vm().wheel_input.borrow().next_deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub(in crate::runtime) fn due_page_action_window_ready_descriptor(
        &self,
        now: Instant,
    ) -> Option<RendererPageReadyDescriptor> {
        self.next_action_window_deadline()
            .filter(|deadline| *deadline <= now)
            .map(|deadline| RendererPageReadyDescriptor::ActionWindow { deadline })
    }

    pub(in crate::runtime) fn queue_wheel_event(
        &mut self,
        x: f64,
        y: f64,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome> {
        let admitted_at = Instant::now();
        if let Some(batch) = self.page_action_window.window.take_due(admitted_at) {
            self.apply_page_action_batch(batch)?;
        }
        self.vm().mouse_input_surface_hit(x, y, "wheel")?;

        let scope = self.document_lifecycle.identity();
        let admission = self.page_action_window.window.push(
            scope,
            WindowAction::Scroll(ScrollAction {
                position: Point::new(x, y),
                delta_x,
                delta_y,
                delta_mode: ScrollDeltaMode::Pixel,
                modifiers: InputModifiers::from_bits(modifiers),
            }),
            admitted_at,
        );
        debug_assert!(
            admission.ready_batch().is_none(),
            "the due batch was consumed immediately before admission"
        );
        let sequence = admission.sequence();
        self.page_action_window.wheel_events.insert(
            sequence,
            WheelEventProperties {
                button,
                buttons,
                click_count,
                pointer,
            },
        );
        Ok(RendererInputDispatchOutcome {
            handled: true,
            wheel: None,
            triggered_top_level_navigation: false,
            pending_download: None,
            pending_file_chooser: None,
            intercepted_drag: None,
        })
    }

    pub(in crate::runtime) fn flush_page_action_window(
        &mut self,
        barrier: ActionBarrier,
    ) -> Result<bool> {
        let Some(batch) = self
            .page_action_window
            .window
            .flush(barrier, Instant::now())
        else {
            return Ok(false);
        };
        self.apply_page_action_batch(batch)?;
        Ok(true)
    }

    pub(super) fn retire_document_actions(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
    ) -> Result<()> {
        let sequences = self.page_action_window.window.cancel_scope(&document);
        for sequence in &sequences {
            anyhow::ensure!(
                self.page_action_window
                    .wheel_events
                    .remove(sequence)
                    .is_some(),
                "action-window cancellation lost wheel payload {}",
                sequence.get()
            );
        }
        if !sequences.is_empty() {
            tracing::debug!(
                ?document,
                canceled_action_count = sequences.len(),
                "canceled renderer actions for a retired Document"
            );
        }
        Ok(())
    }

    fn apply_page_action_batch(
        &mut self,
        batch: ActionBatch<RendererDocumentLifecycleIdentity>,
    ) -> Result<()> {
        let batch_id = batch.id().get();
        let cause = batch.cause();
        let batch_document = self.document_lifecycle.identity();
        let mut events: Vec<(ScrollAction, WheelEventProperties)> =
            Vec::with_capacity(batch.retained_action_count());
        for action in batch.into_actions() {
            match action {
                PlannedAction::Scroll { scope, run } => {
                    let run_start = events.len();
                    for step in run.steps() {
                        let event = self
                            .page_action_window
                            .wheel_events
                            .remove(&step.sequence())
                            .ok_or_else(|| {
                                anyhow!(
                                    "action-window batch {batch_id} lost wheel payload {}",
                                    step.sequence().get()
                                )
                            })?;
                        if scope != batch_document {
                            continue;
                        }
                        if let Some((previous_scroll, previous_event)) =
                            events[run_start..].last_mut()
                            && *previous_event == event
                            && previous_scroll.coalesce(step.value())
                        {
                            continue;
                        }
                        events.push((step.value().clone(), event));
                    }
                }
                PlannedAction::Click { .. } | PlannedAction::Ordered { .. } => {
                    return Err(anyhow!(
                        "renderer wheel action window contained an unsupported action kind"
                    ));
                }
            }
        }
        if events.is_empty() {
            tracing::debug!(batch_id, ?cause, "discarded stale renderer action batch");
            return Ok(());
        }

        self.vm_mut().begin_batched_mouse_event_dispatch();
        let mut first_error = None;
        let mut skipped_events = 0_usize;
        for (scroll, event) in events {
            if self.document_lifecycle.identity() != batch_document {
                skipped_events += 1;
                continue;
            }
            let result = self.vm_mut().dispatch_mouse_input(
                scroll.position.x,
                scroll.position.y,
                "wheel",
                event.button,
                event.buttons,
                event.click_count,
                scroll.delta_x,
                scroll.delta_y,
                event.pointer,
                scroll.modifiers.bits(),
                false,
                MouseDispatchCompletion::DeferToBatch,
            );
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if skipped_events != 0 {
            tracing::debug!(
                batch_id,
                skipped_action_count = skipped_events,
                "discarded remaining renderer actions after an in-batch Document replacement"
            );
        }
        let dispatch_result = first_error.map_or(Ok(()), Err);
        let document_unchanged = self.document_lifecycle.identity() == batch_document;
        let result = self
            .vm_mut()
            .finish_batched_mouse_event_dispatch(dispatch_result, document_unchanged);
        tracing::debug!(batch_id, ?cause, "applied renderer action batch");
        result
    }

    pub(in crate::runtime) fn apply_selected_page_action_window_turn(
        &mut self,
        deadline: Instant,
    ) -> Result<()> {
        if self.next_action_window_deadline() != Some(deadline) {
            return Ok(());
        }
        let now = Instant::now();
        self.vm().wheel_input.borrow_mut().expire(now);
        let Some(batch) = self.page_action_window.window.take_due(now) else {
            return Ok(());
        };
        self.apply_page_action_batch(batch)
    }

    #[cfg(test)]
    pub(super) fn pending_action_counts_for_test(&self) -> (usize, usize) {
        (
            self.page_action_window
                .window
                .pending_retained_action_count(),
            self.page_action_window.wheel_events.len(),
        )
    }
}
