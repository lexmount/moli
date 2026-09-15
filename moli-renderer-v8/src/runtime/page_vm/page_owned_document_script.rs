use anyhow::Result;

use crate::document_script_scheduler::{
    PageOwnedDocumentScriptExecution, PageOwnedDocumentScriptRunner, PageOwnedDocumentScriptWork,
};
use crate::frame_owner_model::FrameDocumentTaskOwner;

use super::PageVm;
use super::page_owned_document_script_hooks::MainPageOwnedDocumentScriptHooks;

pub(super) struct MainPageOwnedDocumentScriptOwner<'page> {
    runner: PageOwnedDocumentScriptRunner<MainPageOwnedDocumentScriptHooks<'page>>,
}

impl<'page> MainPageOwnedDocumentScriptOwner<'page> {
    pub(super) fn new(page_vm: &'page mut PageVm) -> Self {
        Self {
            runner: PageOwnedDocumentScriptRunner::new(MainPageOwnedDocumentScriptHooks::new(
                page_vm,
            )),
        }
    }

    pub(super) async fn run_work(
        &mut self,
        work: PageOwnedDocumentScriptWork,
    ) -> Result<PageOwnedDocumentScriptExecution<FrameDocumentTaskOwner>> {
        self.runner.run_work(work).await
    }
}

pub(super) type MainPageOwnedDocumentScriptExecution =
    PageOwnedDocumentScriptExecution<FrameDocumentTaskOwner>;
