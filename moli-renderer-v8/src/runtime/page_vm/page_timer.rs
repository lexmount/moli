use std::time::Instant;

use anyhow::{Result, ensure};

use crate::page_task_queue::RendererPageTimerSelection;
use crate::runtime::PageOwnerTurnOutcome;

use super::PageVm;

/// Result of executing the timer-heap head selected by the Page scheduler.
///
/// The heap retains the exact Document/realm-bound payload. The descriptor
/// carries only the observed head deadline, which is revalidated immediately
/// before execution so a stale deadline wake cannot consume a different timer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum PageTimerTurnAction {
    Consumed {
        deadline: Instant,
    },
    NoLongerRunnable {
        expected_deadline: Instant,
        actual_deadline: Option<Instant>,
    },
}

impl PageVm {
    /// Run the exact ready timer selected by the post-parse lifecycle's
    /// classic-defer schedule ranges, then commit the ordinary callback task
    /// completion once.
    pub(in crate::runtime) async fn run_classic_defer_timer_before_domcontentloaded(
        &mut self,
        loader: &crate::network::ResourceRequestClient,
    ) -> Result<()> {
        let body = self.vm_mut().run_next_classic_defer_timer_callback_body()?;
        ensure!(
            body.consumed_heap_head(),
            "a ready classic-defer timer selected before DOMContentLoaded must consume its heap entry"
        );
        self.finish_selected_page_callback_task(loader).await
    }

    /// Execute one already-selected timer body without committing its
    /// task-end checkpoint.
    ///
    /// `Consumed` means the validated heap head was removed; its exact
    /// callback realm may have retired before execution. The central selected
    /// Page-task dispatcher owns the single callback completion for every
    /// consumed timer turn.
    pub(in crate::runtime) fn apply_selected_page_timer_turn(
        &mut self,
        expected_deadline: Instant,
        selection: RendererPageTimerSelection,
    ) -> Result<PageOwnerTurnOutcome<PageTimerTurnAction>> {
        let actual_deadline = self.vm().next_ready_timeout_deadline(selection);
        let action = if actual_deadline == Some(expected_deadline) {
            let body = self.vm_mut().run_next_due_timer_callback_body(selection)?;
            ensure!(
                body.consumed_heap_head(),
                "a validated due timer descriptor must consume its selected heap head"
            );
            PageTimerTurnAction::Consumed {
                deadline: expected_deadline,
            }
        } else {
            PageTimerTurnAction::NoLongerRunnable {
                expected_deadline,
                actual_deadline,
            }
        };
        Ok(PageOwnerTurnOutcome::new(action))
    }
}
