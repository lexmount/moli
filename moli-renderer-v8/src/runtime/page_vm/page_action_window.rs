use std::{collections::HashMap, time::Instant};

use anyhow::{Result, anyhow};
use moli_action_window::{
    ActionBarrier, ActionBatch, ActionSequence, ActionWindow, InputModifiers, PlannedAction, Point,
    ScrollAction, WindowAction,
};

use crate::{
    page_task_queue::RendererPageReadyDescriptor,
    runtime::{
        RendererDocumentLifecycleIdentity, RendererInputDispatchOutcome,
        RendererPointerEventProperties,
    },
};

use super::PageVm;

#[derive(Debug)]
struct QueuedWheelEvent {
    x: f64,
    y: f64,
    button: i32,
    buttons: Option<i32>,
    click_count: i32,
    delta_x: f64,
    delta_y: f64,
    pointer: RendererPointerEventProperties,
    modifiers: u8,
}

pub(super) struct RendererPageActionWindow {
    window: ActionWindow<RendererDocumentLifecycleIdentity>,
    wheel_events: HashMap<ActionSequence, QueuedWheelEvent>,
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
    pub(in crate::runtime) fn next_action_window_deadline(&self) -> Option<Instant> {
        self.page_action_window.window.next_deadline()
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

        let scope = self.document_lifecycle.identity();
        let admission = self.page_action_window.window.push(
            scope,
            WindowAction::Scroll(ScrollAction {
                position: Point::new(x, y),
                delta_x,
                delta_y,
                delta_mode: moli_action_window::ScrollDeltaMode::Pixel,
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
            QueuedWheelEvent {
                x,
                y,
                button,
                buttons,
                click_count,
                delta_x,
                delta_y,
                pointer,
                modifiers,
            },
        );
        Ok(RendererInputDispatchOutcome {
            handled: true,
            triggered_top_level_navigation: false,
            pending_download: None,
            pending_file_chooser: None,
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
        let mut events = Vec::with_capacity(batch.retained_action_count());
        for action in batch.into_actions() {
            match action {
                PlannedAction::Scroll { scope, run } => {
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
                        if scope == batch_document {
                            events.push((scope, event));
                        }
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
        for (document, event) in events {
            if self.document_lifecycle.identity() != document {
                skipped_events += 1;
                continue;
            }
            let result = self
                .vm_mut()
                .dispatch_mouse_event_at_point_with_pointer_and_modifiers_without_checkpoint(
                    event.x,
                    event.y,
                    "wheel",
                    event.button,
                    event.buttons,
                    event.click_count,
                    event.delta_x,
                    event.delta_y,
                    event.pointer,
                    event.modifiers,
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
        if self.page_action_window.window.next_deadline() != Some(deadline) {
            return Ok(());
        }
        let Some(batch) = self.page_action_window.window.take_due(Instant::now()) else {
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
