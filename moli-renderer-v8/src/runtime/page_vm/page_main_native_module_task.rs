//! Completion boundary for selected main native-module tasks.
//!
//! Dynamic-module graph jobs and native module-owner events share body
//! machinery but retain distinct post-execution action types.  Both current
//! concrete tasks own exactly one ordinary checkpoint.  A spent reservation
//! or stale root owner owns none.  Errors are reported only after that proven
//! task end has completed.

use crate::{
    page_task_queue::{
        PageDynamicModuleJobTurnAction, PageMainDocumentRuntimeTurnAction,
        PageMainNativeModuleBodyActivity, PageMainNativeModuleSettlement,
        PageMainNativeModuleTargetEffect, PageNativeModuleOwnerEventTurnAction,
        RendererPageMainDocumentRuntimeOwner,
    },
    script_vm::{
        MainNativeModuleSelectedTaskApplication, MainNativeModuleSelectedTaskBodyActivity,
    },
};

use super::{PageTaskCompletion, PageVm};

fn action_parts(
    application: MainNativeModuleSelectedTaskApplication,
) -> (
    PageMainNativeModuleTargetEffect,
    PageMainNativeModuleSettlement,
) {
    match application {
        MainNativeModuleSelectedTaskApplication::ReservationSpent => (
            PageMainNativeModuleTargetEffect::CurrentOwnerReservationSpent,
            PageMainNativeModuleSettlement::Completed,
        ),
        MainNativeModuleSelectedTaskApplication::Applied(execution) => {
            let (activity, failure) = execution.into_parts();
            let activity = match activity {
                MainNativeModuleSelectedTaskBodyActivity::StateTransitionOnly => {
                    PageMainNativeModuleBodyActivity::StateTransitionOnly
                }
                MainNativeModuleSelectedTaskBodyActivity::PageRealmBodyAttempted => {
                    PageMainNativeModuleBodyActivity::PageRealmBodyAttempted
                }
            };
            let settlement = match failure {
                Some(failure) => PageMainNativeModuleSettlement::Failed(failure),
                None => PageMainNativeModuleSettlement::Completed,
            };
            (
                PageMainNativeModuleTargetEffect::AppliedToSelectedOwner(activity),
                settlement,
            )
        }
    }
}

pub(super) fn dynamic_module_job_action(
    owner: RendererPageMainDocumentRuntimeOwner,
    application: MainNativeModuleSelectedTaskApplication,
) -> PageMainDocumentRuntimeTurnAction {
    let (target_effect, settlement) = action_parts(application);
    PageMainDocumentRuntimeTurnAction::dynamic_module_job(owner, target_effect, settlement)
}

pub(super) fn native_module_owner_event_action(
    owner: RendererPageMainDocumentRuntimeOwner,
    application: MainNativeModuleSelectedTaskApplication,
) -> PageMainDocumentRuntimeTurnAction {
    let (target_effect, settlement) = action_parts(application);
    PageMainDocumentRuntimeTurnAction::native_module_owner_event(owner, target_effect, settlement)
}

fn completion_for_target(target: PageMainNativeModuleTargetEffect) -> PageTaskCompletion {
    match target {
        // The old main native-module carrier performed no callback-style
        // runtime drain. Promise reactions deliberately run here, while any
        // concrete successor they publish waits for normal Page arbitration.
        PageMainNativeModuleTargetEffect::AppliedToSelectedOwner(_) => {
            PageTaskCompletion::CheckpointOnly
        }
        PageMainNativeModuleTargetEffect::CurrentOwnerReservationSpent
        | PageMainNativeModuleTargetEffect::DiscardedStaleOwner => PageTaskCompletion::NoCompletion,
    }
}

impl PageVm {
    async fn finish_selected_page_main_native_module_task(
        &mut self,
        target: PageMainNativeModuleTargetEffect,
        settlement: PageMainNativeModuleSettlement,
    ) -> anyhow::Result<()> {
        self.finish_selected_page_task_completion(completion_for_target(target))
            .await?;
        match settlement {
            PageMainNativeModuleSettlement::Completed => Ok(()),
            PageMainNativeModuleSettlement::Failed(message) => {
                Err(anyhow::anyhow!("main native-module task failed: {message}"))
            }
        }
    }

    pub(super) async fn finish_selected_page_dynamic_module_job(
        &mut self,
        action: PageDynamicModuleJobTurnAction,
    ) -> anyhow::Result<()> {
        let (target, settlement) = action.into_parts();
        self.finish_selected_page_main_native_module_task(target, settlement)
            .await
    }

    pub(super) async fn finish_selected_page_native_module_owner_event(
        &mut self,
        action: PageNativeModuleOwnerEventTurnAction,
    ) -> anyhow::Result<()> {
        let (target, settlement) = action.into_parts();
        self.finish_selected_page_main_native_module_task(target, settlement)
            .await
    }
}
