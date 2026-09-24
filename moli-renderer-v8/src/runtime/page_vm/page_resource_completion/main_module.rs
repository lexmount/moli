use anyhow::Result;

use crate::{
    page_resource_completion::{
        MainDynamicImportGraphFetchCompletion, MainModulepreloadFetchCompletion,
        MainParserModuleGraphFetchCompletion, MainRuntimeModuleGraphFetchCompletion,
        PageResourceCompletionBodyActivity, PageResourceCompletionOutputEffect,
        PageResourceCompletionPostCheckpointEffect, PageResourceCompletionTurnAction,
        RendererPageResourceCompletionOwner,
    },
    runtime::RendererOwnerResourceActivitySource,
};

use super::super::PageVm;

/// Proof that the Page lane executor matched one parser module terminal
/// against its complete `{root Document, PendingScript, fetch load}` target.
pub(crate) struct AuthorizedCurrentMainParserModuleGraphFetchCompletion(
    MainParserModuleGraphFetchCompletion,
);

impl AuthorizedCurrentMainParserModuleGraphFetchCompletion {
    fn new(completion: MainParserModuleGraphFetchCompletion) -> Self {
        Self(completion)
    }

    pub(crate) fn into_completion(self) -> MainParserModuleGraphFetchCompletion {
        self.0
    }
}

/// Proof that the Page lane executor matched one runtime-created main module
/// terminal against its complete Document/script-owner/fetch target.
pub(crate) struct AuthorizedCurrentMainRuntimeModuleGraphFetchCompletion(
    MainRuntimeModuleGraphFetchCompletion,
);

impl AuthorizedCurrentMainRuntimeModuleGraphFetchCompletion {
    fn new(completion: MainRuntimeModuleGraphFetchCompletion) -> Self {
        Self(completion)
    }

    pub(crate) fn into_completion(self) -> MainRuntimeModuleGraphFetchCompletion {
        self.0
    }
}

/// Proof that the Page lane executor matched one dynamic-import network
/// terminal against its complete `{root Document, import owner, load id}`
/// target.
pub(crate) struct AuthorizedCurrentMainDynamicImportGraphFetchCompletion(
    MainDynamicImportGraphFetchCompletion,
);

impl AuthorizedCurrentMainDynamicImportGraphFetchCompletion {
    fn new(completion: MainDynamicImportGraphFetchCompletion) -> Self {
        Self(completion)
    }

    pub(crate) fn into_completion(self) -> MainDynamicImportGraphFetchCompletion {
        self.0
    }
}

/// Proof that the Page lane executor matched one modulepreload terminal
/// against an in-flight request in the current ScriptState's module map.
///
/// The connected link and Resource Timing attribution are Document-owned, but
/// Blink keeps the Modulator in V8 per-context data. Consequently
/// `document.open()` may retire the completion's Document owner while leaving
/// this realm-owned fetch live.
pub(crate) struct AuthorizedLiveMainModulepreloadFetchCompletion(MainModulepreloadFetchCompletion);

impl AuthorizedLiveMainModulepreloadFetchCompletion {
    fn new(completion: MainModulepreloadFetchCompletion) -> Self {
        Self(completion)
    }

    pub(crate) fn into_completion(self) -> MainModulepreloadFetchCompletion {
        self.0
    }
}

impl PageVm {
    pub(super) fn apply_main_parser_module_graph_fetch_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: MainParserModuleGraphFetchCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            let output_effect = PageResourceCompletionOutputEffect::None;
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                output_effect,
            ));
        }

        let output_effect = self.record_current_main_module_network_result(
            completion.request_url(),
            completion.network_result(),
        );
        self.vm_mut()
            .apply_current_main_parser_module_graph_fetch_completion(
                AuthorizedCurrentMainParserModuleGraphFetchCompletion::new(completion),
            )?;
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            output_effect,
        ))
    }

    pub(super) fn apply_main_runtime_module_graph_fetch_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: MainRuntimeModuleGraphFetchCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            let output_effect = PageResourceCompletionOutputEffect::None;
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                output_effect,
            ));
        }

        let output_effect = self.record_current_main_module_network_result(
            completion.request_url(),
            completion.network_result(),
        );
        let document_owner = completion.target().document_owner();
        let actions = self
            .vm_mut()
            .apply_current_main_runtime_module_graph_fetch_completion(
                AuthorizedCurrentMainRuntimeModuleGraphFetchCompletion::new(completion),
            )?;
        let settlement = self.handle_immediate_native_module_owner_actions_body(actions);
        let body_activity = match settlement.activity() {
            crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch => {
                PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch
            }
            crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted => {
                PageResourceCompletionBodyActivity::PageCodeOrEventDispatchAttempted
            }
        };
        Ok(self.runtime_module_graph_resource_action(
            source,
            owner,
            document_owner,
            body_activity,
            output_effect,
            settlement,
        ))
    }

    pub(super) fn apply_main_dynamic_import_graph_fetch_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: MainDynamicImportGraphFetchCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            // A cross-PageVm replacement can naturally reuse every local
            // dynamic-import identity. Its old terminal is historical only:
            // applying any local retirement to the replacement VM would
            // consume unrelated current work. Same-root staleness can still
            // retire an exact resolver wait left by a local Document/realm
            // transition.
            if owner.root_document() == self.document_lifecycle.identity().document {
                let retired_exact_wait = self
                    .vm_mut()
                    .retire_stale_main_dynamic_import_graph_fetch(completion.target());
                if retired_exact_wait {
                    tracing::debug!(
                        target = ?completion.target(),
                        "retired stale main dynamic-import fetch after its typed terminal"
                    );
                }
            }
            let output_effect = PageResourceCompletionOutputEffect::None;
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                output_effect,
            ));
        }

        let output_effect = self.record_current_main_module_network_result(
            completion.request_url(),
            completion.network_result(),
        );
        let document_owner = completion.target().import_owner().task_owner();
        let graph_body: crate::script_vm::MainDynamicImportGraphFetchBodySettlement = self
            .vm_mut()
            .apply_current_main_dynamic_import_graph_fetch_completion_selected_task_body(
                AuthorizedCurrentMainDynamicImportGraphFetchCompletion::new(completion),
            )?;
        let (actions, native_body_activity) = graph_body.into_parts();
        let settlement = self.handle_immediate_native_module_owner_actions_body(actions);
        let body_activity = if matches!(
            native_body_activity,
            crate::script_vm::MainNativeModuleSelectedTaskBodyActivity::PageRealmBodyAttempted
        ) || matches!(
            settlement.activity(),
            crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted
        ) {
            PageResourceCompletionBodyActivity::PageCodeOrEventDispatchAttempted
        } else {
            PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch
        };
        Ok(self.runtime_module_graph_resource_action(
            source,
            owner,
            document_owner,
            body_activity,
            output_effect,
            settlement,
        ))
    }

    pub(super) fn apply_main_modulepreload_fetch_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: MainModulepreloadFetchCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            let output_effect = PageResourceCompletionOutputEffect::None;
            // document.open() keeps the LocalWindow/ScriptState and therefore
            // its Modulator, even though Moli advances the internal
            // Document task owner. Settle only that retained realm cache. Old
            // link clients were removed at replacement, and the historical
            // network path above deliberately avoids replacement-Document
            // Resource Timing or lifecycle effects.
            if owner.root_document() == self.document_lifecycle.identity().document
                && self
                    .vm()
                    .current_main_modulepreload_fetch_target(completion.target().load_id())
                    .is_some()
            {
                self.vm_mut()
                    .apply_live_main_modulepreload_fetch_completion(
                        AuthorizedLiveMainModulepreloadFetchCompletion::new(completion),
                    )?;
            }
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                output_effect,
            ));
        }

        let output_effect = if let Some(network_result) = completion.network_result() {
            self.vm_mut()
                .record_modulepreload_resource_performance_entry(
                    &completion.request_url().clone(),
                    network_result.as_ref(),
                );
            PageResourceCompletionOutputEffect::CaptureRequired
        } else {
            PageResourceCompletionOutputEffect::None
        };
        self.vm_mut()
            .apply_live_main_modulepreload_fetch_completion(
                AuthorizedLiveMainModulepreloadFetchCompletion::new(completion),
            )?;
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            output_effect,
        ))
    }

    fn record_current_main_module_network_result(
        &mut self,
        request_url: &url::Url,
        network_result: Option<&crate::types::SharedNavigationResponseResult>,
    ) -> PageResourceCompletionOutputEffect {
        let Some(network_result) = network_result else {
            return PageResourceCompletionOutputEffect::None;
        };
        self.vm_mut()
            .record_script_resource_timing(request_url.clone(), network_result.as_ref());
        PageResourceCompletionOutputEffect::CaptureRequired
    }

    fn runtime_module_graph_resource_action(
        &self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        body_activity: PageResourceCompletionBodyActivity,
        output_effect: PageResourceCompletionOutputEffect,
        settlement: crate::script_vm::RuntimeOwnedModuleFailureBodySettlement,
    ) -> PageResourceCompletionTurnAction {
        let mut action = match body_activity {
            PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch => {
                PageResourceCompletionTurnAction::applied(source, owner, output_effect)
            }
            PageResourceCompletionBodyActivity::PageCodeOrEventDispatchAttempted => {
                if self.vm().current_main_document_task_owner() == Some(document_owner) {
                    PageResourceCompletionTurnAction::applied_after_page_code(
                        source,
                        owner,
                        output_effect,
                    )
                } else {
                    PageResourceCompletionTurnAction::superseded_after_page_code(
                        source,
                        owner,
                        self.current_page_resource_completion_owner(owner),
                        output_effect,
                    )
                }
            }
        };
        if let Some(owner) = settlement.lifecycle_unblocked_owner() {
            action = action.with_post_checkpoint_effect(
                PageResourceCompletionPostCheckpointEffect::PrimeMainDocumentLifecycle { owner },
            );
        }
        action
    }
}
