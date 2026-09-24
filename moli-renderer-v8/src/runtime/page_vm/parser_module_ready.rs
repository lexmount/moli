use anyhow::Result;

use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::page_task_queue::PostParsePageOwnedWork;

use super::PageVm;
use super::parser_owned_document_script::MainParserOwnedDocumentScriptOwner;
use super::parser_task_completion::MainParserContinuationTaskEffect;

impl PageVm {
    pub(in crate::runtime) fn seal_main_parser_deferred_scripts(
        &mut self,
        task_owner: FrameDocumentTaskOwner,
    ) -> Option<PostParsePageOwnedWork> {
        self.vm_mut().seal_main_parser_deferred_scripts(task_owner)
    }

    pub(super) fn has_ready_parser_owned_document_script_action(&self) -> bool {
        self.vm()
            .document_runtime
            .parser_module_document_scripts()
            .has_ready_work()
    }

    pub(super) fn admit_ready_parser_owned_document_script_action(&mut self) -> bool {
        self.has_ready_parser_owned_document_script_action()
            && self.vm_mut().enqueue_parser_owned_module_continuation()
    }

    pub(super) async fn run_next_ready_parser_owned_document_script_action(
        &mut self,
    ) -> Result<MainParserContinuationTaskEffect> {
        MainParserOwnedDocumentScriptOwner::new(self)
            .run_next_ready_work()
            .await
    }
}
