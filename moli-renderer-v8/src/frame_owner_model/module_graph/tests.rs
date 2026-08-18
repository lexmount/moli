use super::dynamic_import_fetch::FrameDocumentDynamicImportRejectReason;
use super::*;
use crate::document_module_graph::{
    ModuleAttributesKey, ModuleEntryId, ModuleMapFetchDisposition, ModuleMapKey,
    NativeModuleMapSingleModuleClient,
};
use crate::document_runtime::DomHandle;
use crate::dom::NodeId;
use crate::frame_owner_model::{
    ChildDocumentModuleFetchTarget, ChildDynamicModuleCompletedFetchRestoreAction,
    ChildDynamicModuleFetchAction, DocumentId, FrameDocumentDynamicImportEvaluationReadyResult,
    FrameDocumentDynamicImportGraphAdvanceFollowup,
    FrameDocumentDynamicImportJoinedFetchRestoreResult,
    FrameDocumentDynamicImportMissingJoinedTerminalClient, FrameDocumentDynamicImportOwnerAction,
    FrameDocumentDynamicImportOwnerActionHooks, FrameDocumentDynamicImportOwnerActionQueueRequest,
    FrameDocumentDynamicImportOwnerActionRunner,
    FrameDocumentDynamicImportOwnerFetchSettlementResult,
    FrameDocumentDynamicImportSourceReadyResult,
    FrameDocumentDynamicImportTerminalClientFinishResult,
    FrameDocumentDynamicImportTerminalOutcome, FrameDocumentDynamicImportTerminalPreparedAction,
    FrameDocumentDynamicImportTerminalWork, FrameDocumentDynamicImportWaitingFetchScheduleResult,
    FrameDocumentModuleClientEntryId, FrameDocumentModuleClientId,
    FrameDocumentModuleClientRegistration, FrameDocumentModuleClientReservation,
    FrameDocumentModuleDependencyFetchTask, FrameDocumentModuleDependencyTerminalWork,
    FrameDocumentModuleFetchClientStart, FrameDocumentModuleFetchDisposition,
    FrameDocumentModuleFetchTerminalResult, FrameDocumentModuleScriptTerminalBatchTask,
    FrameDocumentModuleScriptTerminalFollowup, FrameDocumentModuleScriptTerminalHooks,
    FrameDocumentModuleScriptTerminalOutcome, FrameDocumentModuleScriptTerminalRunner,
    FrameDocumentModuleScriptTerminalTask, FrameDocumentModuleScriptTerminalWork,
    FrameDocumentModuleTerminalBatch, FrameDocumentModuleTerminalQueueFollowup,
    FrameDocumentModuleTerminalWarning, FrameDocumentModuleTerminalWarningRecord,
    FrameDocumentModulepreloadEventAction, FrameDocumentModulepreloadEventActionHooks,
    FrameDocumentModulepreloadEventActionRunner, FrameDocumentModulepreloadEventBinding,
    FrameDocumentModulepreloadFetchCompletionAction,
    FrameDocumentModulepreloadFetchCompletionHooks,
    FrameDocumentModulepreloadFetchCompletionRunner, FrameDocumentModulepreloadFetchFinishResult,
    FrameDocumentModulepreloadFetchTask, FrameDocumentModulepreloadLinkClient,
    FrameDocumentModulepreloadStartAction, FrameDocumentModulepreloadStartActionHooks,
    FrameDocumentModulepreloadStartActionRunner, FrameDocumentModulepreloadTerminalWork,
    FrameDocumentOwner, FrameDocumentParserModuleTreeAdvanceDependencyFetchResult,
    FrameDocumentParserModuleTreeAdvanceFailureTrace, FrameDocumentParserModuleTreeAdvanceHooks,
    FrameDocumentParserModuleTreeAdvanceRunner, FrameDocumentParserRootModuleClient,
    FrameDocumentParserRootTerminalClient, FrameDocumentParserRootTerminalWork,
    FrameDocumentStaticDependencyModuleClient, FrameDocumentTaskOwner, FrameRealmId,
    FrameRequestId, FrameRequestKind, FrameSchedulerLaneId, LocalWindowId,
    frame_document_parser_module_tree_advance_action,
};
use crate::module_runtime::{
    DynamicModuleFetchFinish, DynamicModuleScheduledFetch, ModuleFetchMetadata,
    ModuleGraphFetchedSource, ModuleGraphHandle, ModuleImportPhase, ModuleKind, ModuleLoadError,
    ModuleLoadStage, ModuleRequestRecord, ModuleSource, NativeDynamicImportSingleModuleClient,
    NativeDynamicModuleImportReady, NativeModuleGraphFetchRequest, NativeModuleGraphJob,
    NativeModuleGraphJobAdvance, NativeModuleScriptSingleModuleClient,
    NativeModuleSingleFetchRequest, NativeParserModuleTreeJobResume, PendingDynamicModuleImport,
};
use crate::planning::{PreparedScript, ScriptFetchMetadata, ScriptSource};
use crate::types::{ScriptKind, ScriptMode, ScriptSourceKind};
use moli_module_script_tree as module_tree;
use url::Url;

fn modulepreload_request(path: &str) -> NativeModuleSingleFetchRequest {
    let base_url =
        Url::parse("https://child-module-graph.test/base/").expect("base url should parse");
    let source_url = base_url.join(path).expect("modulepreload url should parse");
    NativeModuleSingleFetchRequest::new(
        source_url.clone(),
        base_url.clone(),
        base_url,
        ModuleMapKey::java_script(source_url),
        ModuleFetchMetadata::default(),
    )
}

fn modulepreload_link_client(link_handle: DomHandle) -> FrameDocumentModulepreloadLinkClient {
    FrameDocumentModulepreloadLinkClient::new(DomHandle::new(1), link_handle)
}

fn modulepreload_event_binding(link_handle: DomHandle) -> FrameDocumentModulepreloadEventBinding {
    FrameDocumentModulepreloadEventBinding::new(
        modulepreload_link_client(link_handle),
        Some(crate::frame_owner_model::DocumentLoadDelayTokenId(91)),
    )
}

fn pending_dynamic_module_import() -> PendingDynamicModuleImport {
    pending_dynamic_module_import_with_phase(ModuleImportPhase::Evaluation)
}

fn pending_dynamic_module_import_with_phase(
    phase: ModuleImportPhase,
) -> PendingDynamicModuleImport {
    let _js_runtime = crate::JsRuntime::initialize();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
    PendingDynamicModuleImport::new(
        v8::Global::new(scope, scope.get_current_context()),
        v8::Global::new(scope, resolver),
        crate::module_runtime::DynamicModuleImportOwner::main_for_test(),
        "./dynamic.mjs",
        Url::parse("https://child-module-graph.test/app/page.html").unwrap(),
        ModuleAttributesKey::empty(),
        phase,
    )
}

fn dynamic_fetch_request(path: &str) -> NativeModuleGraphFetchRequest {
    let base_url =
        Url::parse("https://child-module-graph.test/app/").expect("base url should parse");
    NativeModuleGraphFetchRequest::new_for_test(
        base_url
            .join(path)
            .expect("dynamic import url should parse"),
        base_url,
        ModuleFetchMetadata::default(),
        ModuleKind::JavaScript,
    )
}

fn dynamic_tree_fetch_request(
    path: &str,
    tree_client: module_tree::SingleModuleClientToken,
) -> NativeModuleGraphFetchRequest {
    let base_url =
        Url::parse("https://child-module-graph.test/app/").expect("base url should parse");
    let source_url = base_url
        .join(path)
        .expect("dynamic import url should parse");
    let key = ModuleMapKey::java_script(source_url.clone());
    let parent_url =
        Url::parse("https://child-module-graph.test/app/parent.mjs").expect("parent url");
    let parent_key = ModuleMapKey::java_script(parent_url);
    NativeModuleGraphFetchRequest::new_tree_dependency_for_test(
        source_url,
        base_url,
        ModuleFetchMetadata::default(),
        ModuleKind::JavaScript,
        tree_client,
        key,
        parent_key,
        ModuleEntryId::from_raw(7),
        "./dynamic-dep.mjs".to_owned(),
        ModuleImportPhase::Evaluation,
    )
}

fn parser_root_script(handle: usize, url: &Url) -> PreparedScript {
    PreparedScript {
        position: handle,
        node_id: NodeId::new(handle),
        kind: ScriptKind::Module,
        mode: ScriptMode::ModuleDefer,
        source_kind: ScriptSourceKind::External,
        fetch_metadata: ScriptFetchMetadata::default(),
        source: ScriptSource::External,
        url: url.clone(),
        base_url: url.clone(),
        initiator_url: url.clone(),
        host_script_handle: None,
    }
}

fn parser_root_load_delay_token(
    handle: usize,
) -> crate::frame_owner_model::DocumentLoadDelayTokenId {
    crate::frame_owner_model::DocumentLoadDelayTokenId(handle as u64 + 1)
}

fn parser_root_client(handle: usize, url: &Url) -> FrameDocumentParserRootModuleClient {
    let script = parser_root_script(handle, url);
    FrameDocumentParserRootModuleClient::new(
        crate::document_script_scheduler::ParserPendingScriptKey::from_script(&script),
        script,
        DomHandle::new(handle),
        url.clone(),
        ScriptFetchMetadata::default(),
        true,
        parser_root_load_delay_token(handle),
    )
}

mod document_modulator;
mod dynamic_import;
mod followups;
mod parser_roots;
mod parser_tree_jobs;
