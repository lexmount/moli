//! Shared capture/lifecycle hooks with no external input queue.
use super::PageVm;
use crate::{
    page_task_queue::RendererPageReadyDescriptor, runtime::RendererDocumentLifecycleIdentity,
};
use anyhow::Result;
use moli_action_window::ActionBarrier;
use std::time::Instant;
#[derive(Default)]
pub(super) struct RendererPageActionWindow;
impl PageVm {
    pub(in crate::runtime) fn next_action_window_deadline(&self) -> Option<Instant> {
        None
    }
    pub(in crate::runtime) fn due_page_action_window_ready_descriptor(
        &self,
        _now: Instant,
    ) -> Option<RendererPageReadyDescriptor> {
        None
    }
    pub(in crate::runtime) fn flush_page_action_window(
        &mut self,
        _barrier: ActionBarrier,
    ) -> Result<bool> {
        Ok(false)
    }
    pub(super) fn retire_document_actions(
        &mut self,
        _document: RendererDocumentLifecycleIdentity,
    ) -> Result<()> {
        Ok(())
    }
    pub(in crate::runtime) fn apply_selected_page_action_window_turn(
        &mut self,
        _deadline: Instant,
    ) -> Result<()> {
        Ok(())
    }
}
