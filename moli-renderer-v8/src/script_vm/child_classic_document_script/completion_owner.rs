use crate::document_script_scheduler::{
    ParserClassicDocumentScriptCompletionPlan, ParserClassicDocumentScriptContinuation,
};
use crate::frame_owner_model::{
    FrameDocumentClassicCompletionFinishAction, FrameDocumentClassicCompletionFollowup,
    FrameDocumentClassicCompletionLifecycleFollowup,
    FrameDocumentClassicCompletionScriptEventAction,
    FrameDocumentClassicCompletionScriptEventFollowup, FrameDocumentClassicParserResumeApplication,
    FrameDocumentClassicScriptCompletionAction, FrameDocumentClassicScriptCompletionTarget,
    FrameDocumentClassicScriptScheduling, FrameDocumentScriptElementEventKind,
};

use super::super::{ScriptVm, child_document_script_owner_hooks::ChildDocumentScriptOwnerHooks};

pub(in crate::script_vm) struct ChildClassicCompletionOwner<'vm> {
    vm: &'vm mut ScriptVm,
}

impl<'vm> ChildClassicCompletionOwner<'vm> {
    pub(in crate::script_vm) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }

    pub(in crate::script_vm) fn prepare_completion_plan(
        &mut self,
        completion: FrameDocumentClassicScriptCompletionAction,
    ) -> ParserClassicDocumentScriptCompletionPlan<
        FrameDocumentClassicCompletionFinishAction,
        FrameDocumentClassicScriptCompletionTarget,
    > {
        let action = FrameDocumentClassicCompletionFinishAction::from_completion(completion);
        let target = action.target();
        ParserClassicDocumentScriptCompletionPlan::new(action, target.scheduling(), target)
    }

    pub(in crate::script_vm) fn apply_completion_action(
        &mut self,
        action: FrameDocumentClassicCompletionFinishAction,
    ) -> anyhow::Result<FrameDocumentClassicCompletionScriptEventFollowup> {
        let mut script_event_followup =
            FrameDocumentClassicCompletionScriptEventFollowup::default();
        if let Some(event_action) = action.script_element_event_action() {
            self.dispatch_script_element_event(event_action, &mut script_event_followup);
        }
        Ok(script_event_followup)
    }

    pub(in crate::script_vm) fn apply_completion_continuation(
        &mut self,
        continuation: ParserClassicDocumentScriptContinuation<
            FrameDocumentClassicScriptCompletionTarget,
        >,
        script_event_followup: FrameDocumentClassicCompletionScriptEventFollowup,
    ) -> anyhow::Result<FrameDocumentClassicCompletionFollowup> {
        let lifecycle_followup = match continuation {
            ParserClassicDocumentScriptContinuation::ResumeParser(target) => {
                let resume = ChildDocumentScriptOwnerHooks::new(self.vm)
                    .resume_child_classic_parser_after_completion(
                    crate::frame_owner_model::FrameDocumentClassicParserResumeCompletionAction::new(
                        target,
                    ),
                );
                self.apply_parser_resume_application(resume)
            }
            ParserClassicDocumentScriptContinuation::ReleaseDeferred(target) => {
                self.apply_deferred_completion(target)
            }
        };
        Ok(FrameDocumentClassicCompletionFollowup::from_parts(
            script_event_followup,
            lifecycle_followup,
        ))
    }

    fn dispatch_script_element_event(
        &mut self,
        action: FrameDocumentClassicCompletionScriptEventAction,
        followup: &mut FrameDocumentClassicCompletionScriptEventFollowup,
    ) {
        let target = action.target();
        let event = action.event();
        let realm_id = target.realm_id();
        let parser_blocking = matches!(
            target.scheduling(),
            FrameDocumentClassicScriptScheduling::ParserBlocking
        );
        // Fetch failures bypass script execution, which normally unblocks the
        // tokenizer before invoking page code. The error handler still runs at
        // that parser insertion point, including synchronous document.write().
        // Do not resume again for load: the script may have installed a new
        // nested parser blocker while executing.
        if parser_blocking
            && matches!(event.kind, FrameDocumentScriptElementEventKind::Error)
            && !ChildDocumentScriptOwnerHooks::new(self.vm).resume_parser_for_classic_execution(
                target.child_handle(),
                target.task_owner(),
                event.script_handle,
            )
        {
            tracing::debug!(
                ?target,
                script_handle = ?event.script_handle,
                "dropping child classic error event with a stale parser resume permit"
            );
            followup.note_script_event_dispatch_failed();
            return;
        }
        let _parser_script_nesting = parser_blocking
            .then(|| {
                ChildDocumentScriptOwnerHooks::new(self.vm)
                    .enter_parser_script_nesting(target.child_handle(), target.task_owner())
            })
            .flatten();
        let dispatch = ChildDocumentScriptOwnerHooks::new(self.vm)
            .dispatch_script_element_event_for_parts_selected_task_body(
                target.task_owner(),
                realm_id,
                event.script_handle,
                event.kind,
            );
        match dispatch {
            Ok(()) => followup.note_script_event_dispatched(),
            Err(error) => {
                tracing::warn!(
                    ?target,
                    ?error,
                    "child classic script completion event dispatch failed"
                );
                followup.note_script_event_dispatch_failed();
            }
        }
    }

    fn apply_parser_resume_application(
        &mut self,
        resume: FrameDocumentClassicParserResumeApplication,
    ) -> FrameDocumentClassicCompletionLifecycleFollowup {
        let mut followup = FrameDocumentClassicCompletionLifecycleFollowup::default();
        followup.note_parser_resume_attempted();
        let parser_was_resumed = resume.parser_was_resumed();
        let skip_reason = resume.skip_reason();
        let scheduler_work = resume.into_scheduler_work();
        if parser_was_resumed {
            followup.note_parser_resumed();
            if let Some(work) = scheduler_work {
                ChildDocumentScriptOwnerHooks::new(self.vm)
                    .notify_parser_classic_next_owner_action(work);
                followup.note_document_script_ready_queued();
            }
        } else if let Some(reason) = skip_reason {
            followup.note_parser_resume_skipped(reason);
        }
        followup
    }

    fn apply_deferred_completion(
        &mut self,
        target: crate::frame_owner_model::FrameDocumentClassicScriptCompletionTarget,
    ) -> FrameDocumentClassicCompletionLifecycleFollowup {
        let application = ChildDocumentScriptOwnerHooks::new(self.vm)
            .complete_child_deferred_classic_script(target);
        let order_slot_released = application.order_slot_was_released();
        let domcontentloaded_queued = application.domcontentloaded_was_queued();
        let document_script_ready_queued = application.document_script_ready_was_queued();
        let scheduler_work = application.into_scheduler_work();
        let mut followup = FrameDocumentClassicCompletionLifecycleFollowup::default();
        if order_slot_released {
            followup.note_parser_deferred_order_released();
        }
        if let Some(work) = scheduler_work {
            ChildDocumentScriptOwnerHooks::new(self.vm)
                .notify_parser_classic_next_owner_action(work);
            followup.note_document_script_ready_queued();
        }
        if document_script_ready_queued {
            followup.note_document_script_ready_queued();
        }
        if domcontentloaded_queued {
            followup.note_domcontentloaded_queued();
        }
        followup
    }
}
