use super::*;

impl ScriptVm {
    pub(crate) fn report_long_tasks(
        &mut self,
        long_tasks: impl IntoIterator<Item = crate::long_task::LongTask>,
    ) {
        let mut long_tasks = long_tasks.into_iter().peekable();
        if long_tasks.peek().is_none() {
            return;
        }
        if let Err(error) = self.with_current_main_world_scope(|scope, host| {
            for long_task in long_tasks {
                unsafe { &mut *host }.report_long_task(scope, long_task);
            }
            Ok(())
        }) {
            tracing::debug!(%error, "could not report Window long tasks");
        }
    }
}
