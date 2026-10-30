//! Body-only OPFS support for standalone `ScriptVm` domain fixtures.
//!
//! These fixtures intentionally have no Page owner loop, so they cannot prove
//! root-Page authorization, task-end checkpoint, scheduling, or fairness.
//! They may consume a current exact pending entry and apply its storage result,
//! but Promise reactions remain pending. Full task and stale/replacement tests
//! belong in the `PageVm` production selected-task harness.

use anyhow::Result;

use super::ScriptVm;
use crate::runtime::AuthorizedCurrentPageOpfsTask;

impl ScriptVm {
    pub(crate) fn register_pending_opfs_task_producer_for_test(
        &mut self,
    ) -> Result<crate::page_task_queue::RendererPageOpfsTaskProducer> {
        self.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let resolver =
                v8::PromiseResolver::new(scope).expect("OPFS executor test resolver should exist");
            let locator = moli_storage_service::StorageBucketLocator::default_bucket(
                "https://opfs-executor.test",
            );
            let (_, producer) = unsafe { &mut *host_ptr }
                .register_pending_opfs_task(scope, resolver, locator, None, None)
                .ok_or_else(|| {
                    anyhow::anyhow!("OPFS executor test must capture the current Window realm")
                })?;
            Ok(producer)
        })
    }

    /// Apply one current exact OPFS result without completing an HTML task.
    ///
    /// This helper deliberately rejects stale or foreign-Page tasks instead of
    /// duplicating the `PageVm` arbiter. Tests for those cases must use the
    /// production selected-task harness.
    pub(crate) fn run_opfs_task_body_for_authorization_test(&mut self) -> Result<bool> {
        let residence = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("OPFS body fixture must retain its production Page source");
        let root_document = residence.root_document();
        let source = residence.task_sources();
        let Some(task) = source.take_opfs_task_for_executor_test() else {
            return Ok(false);
        };
        let owner = task.owner();
        anyhow::ensure!(
            owner.root_document() == root_document
                && self.current_pending_opfs_task_execution_context(owner.task())
                    == Some(owner.execution_context()),
            "body-only OPFS fixture cannot arbitrate a stale or foreign-Page task; use the PageVm selected-task harness"
        );
        self.apply_current_opfs_task_body(AuthorizedCurrentPageOpfsTask::new_for_executor_test(
            task,
        ))?;
        Ok(true)
    }
}
