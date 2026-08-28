use crate::page_task_queue::{
    PageV8ForegroundTaskEffect, PageV8ForegroundTaskTurnAction, PageV8ForegroundTaskTurnOutcome,
    RendererPageV8ForegroundTask, RendererPageV8ForegroundTaskKind,
};

use super::PageVm;

impl PageVm {
    pub(in crate::runtime) fn apply_selected_page_v8_foreground_task_turn(
        &mut self,
        task: RendererPageV8ForegroundTask,
    ) -> anyhow::Result<PageV8ForegroundTaskTurnOutcome> {
        let owner = task.owner();
        let effect = match task.into_kind() {
            RendererPageV8ForegroundTaskKind::ScriptAgentTask { task, completion } => {
                if self.vm_mut().run_v8_foreground_task_body(task) {
                    completion.enqueue_peer_checkpoints(owner.page_id());
                    PageV8ForegroundTaskEffect::Ran
                } else {
                    PageV8ForegroundTaskEffect::IgnoredInactiveIsolateRegistration
                }
            }
            RendererPageV8ForegroundTaskKind::ScriptAgentCheckpoint => {
                PageV8ForegroundTaskEffect::RanScriptAgentCheckpoint
            }
        };
        let action = PageV8ForegroundTaskTurnAction { owner, effect };
        Ok(PageV8ForegroundTaskTurnOutcome::new(action))
    }
}
