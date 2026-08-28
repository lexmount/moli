use anyhow::Result;

use super::super::PageVm;
use crate::{
    page_resource_completion::{
        PageResourceCompletionOutputEffect, PageResourceCompletionTurnAction,
        RendererPageResourceCompletionOwner,
    },
    runtime::RendererOwnerResourceActivitySource,
    types::ChildDocumentLoadCompletion,
};

/// Proof that the Page lane executor authorized one child navigation terminal
/// against its complete root/child/navigation/request target. The destination
/// Document and realm do not exist until this terminal commits them.
pub(crate) struct AuthorizedCurrentChildDocumentLoadCompletion(ChildDocumentLoadCompletion);

impl AuthorizedCurrentChildDocumentLoadCompletion {
    fn new(completion: ChildDocumentLoadCompletion) -> Self {
        Self(completion)
    }

    pub(crate) fn into_completion(self) -> ChildDocumentLoadCompletion {
        self.0
    }
}

pub(crate) enum CurrentChildDocumentLoadApplication {
    Applied {
        body_activity: crate::native_bridge::ChildDocumentLoadBodyActivity,
    },
    SupersededDuringApplication {
        historical_network_recorded: bool,
        body_activity: crate::native_bridge::ChildDocumentLoadBodyActivity,
    },
}

impl PageVm {
    pub(super) fn apply_child_document_load_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: ChildDocumentLoadCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            let output_effect = PageResourceCompletionOutputEffect::capture_if(
                self.vm_mut()
                    .record_historical_child_document_load_network(&completion),
            );
            if owner.root_document() == self.document_lifecycle.identity().document {
                self.vm_mut()
                    .discard_stale_child_document_load_completion(completion.target());
            }
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                output_effect,
            ));
        }

        let application = self.vm_mut().apply_current_child_document_load_completion(
            AuthorizedCurrentChildDocumentLoadCompletion::new(completion),
        )?;
        Ok(match application {
            CurrentChildDocumentLoadApplication::Applied { body_activity } => match body_activity {
                crate::native_bridge::ChildDocumentLoadBodyActivity::NoPageCodeOrEventDispatch => {
                    PageResourceCompletionTurnAction::applied(
                        source,
                        owner,
                        PageResourceCompletionOutputEffect::CaptureRequired,
                    )
                }
                crate::native_bridge::ChildDocumentLoadBodyActivity::PageCodeOrEventDispatch => {
                    PageResourceCompletionTurnAction::applied_after_page_code(
                        source,
                        owner,
                        PageResourceCompletionOutputEffect::CaptureRequired,
                    )
                }
            },
            CurrentChildDocumentLoadApplication::SupersededDuringApplication {
                historical_network_recorded,
                body_activity,
            } => match body_activity {
                crate::native_bridge::ChildDocumentLoadBodyActivity::NoPageCodeOrEventDispatch => {
                    PageResourceCompletionTurnAction::superseded_without_page_code(
                        source,
                        owner,
                        self.current_page_resource_completion_owner(owner),
                        PageResourceCompletionOutputEffect::capture_if(historical_network_recorded),
                    )
                }
                crate::native_bridge::ChildDocumentLoadBodyActivity::PageCodeOrEventDispatch => {
                    PageResourceCompletionTurnAction::superseded_after_page_code(
                        source,
                        owner,
                        self.current_page_resource_completion_owner(owner),
                        PageResourceCompletionOutputEffect::capture_if(historical_network_recorded),
                    )
                }
            },
        })
    }
}
