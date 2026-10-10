mod bitmap;
mod bitmap_renderer;
mod canvas_loaded_images;
mod offscreen_canvas_blob;
mod offscreen_canvas_transfer;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::document_script_scheduler::{
    DocumentScriptExecutionLane, PageOwnedDocumentScriptWork, ParserPendingScriptId,
    ParserPendingScriptKey,
};
use crate::dom::{NodeId, native::DomHost};
use crate::dynamic_script_owner::{
    DynamicScriptFailureKind, DynamicScriptOwnerId, DynamicScriptRunnable,
};
use crate::host::ModuleFailurePolicy;
use crate::module_runtime::{
    ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource, ModuleGraphHandle,
    ModuleImportPhase, ModuleKind, ModuleLoadError, ModuleLoadStage, ModuleMapEntryState,
    ModuleMapKey, ModuleSource, NativeModuleGraphFetchRequest, NativeModuleSingleFetchRequest,
};
use crate::module_script_continuation::{
    MainParserDeferredClassicSourceLoadCompletion, MainParserDocumentOwner,
    ModuleScriptContinuation, NativeModuleOwnerActions,
};
use crate::page_resource_completion::{
    MainModuleFetchNetworkAttribution, MainModulepreloadFetchCompletion,
    MainParserDeferredClassicSourceNetworkAttribution, MainParserModuleGraphFetchCompletion,
    MainParserModuleGraphFetchTarget, PageResourceCompletionBodyActivity,
    PageResourceCompletionDocumentEffect, PageResourceCompletionOutputEffect,
    PageResourceCompletionPostCheckpointEffect, PageResourceCompletionTurnAction,
    PageResourceCompletionTurnOutcome, RendererPageResourceCompletion,
    RendererPageResourceCompletionOwner, RendererPageResourceTerminal,
};
use crate::page_task_queue::{
    PageMainDocumentRuntimeActionKind, PageMainDocumentRuntimeTargetEffect,
    PageModulepreloadStartDocumentEffect, PageTask, RendererPageModulepreloadStartOwner,
    RendererPageNetworkingSource,
};
use crate::page_task_queue::{PostParseLifecycleWork, PostParsePageOwnedWork};
use crate::parser::HtmlParser;
use crate::planning::{
    PreparedScript, PreparedScriptSourceLoadOutcome, ScriptFetchMetadata, ScriptSource,
    SharedScriptSourceLoad,
};
use crate::runtime::{
    RendererOwnerResourceActivitySource, RendererPageCommand, RendererRuntimeObservableSourceItem,
    RendererSharedWorkerTargetEvent,
};
use crate::script_vm::{PostParseLifecycleAdvance, PostParseLifecycleCompletionAction};
use crate::types::{
    ChildBlockingStylesheetLoadCompletion, ChildBlockingStylesheetNetworkResult,
    ChildClassicScriptLoadCompletion, ChildClassicScriptNetworkAttribution,
    ChildDynamicImportFetchCompletion, ChildModuleDependencyFetchCompletion,
    ChildModuleFetchNetworkAttribution, ChildModulepreloadFetchCompletion,
    ChildParserModuleRootFetchCompletion, ModuleGraphFetchCompletion, ModuleGraphFetchOrdering,
    ModuleGraphFetchRequester, ScriptKind, ScriptMode, ScriptNetworkOutput,
    ScriptNetworkOutputItem, ScriptObservableOutput, ScriptObservableOutputItem, ScriptRun,
    ScriptRunOutcome, ScriptSkipReason, ScriptSourceKind, SubresourceBodyFinishedResult,
    SubresourceNetworkRecord, SubresourceRequestInitiatorType, WebSocketLifecycleEvent,
    WebSocketNetworkEvent,
};
use crate::types::{SubresourceNetworkOutcome, SubresourceResourceType};
use moli_fetch::FetchConfig;
use moli_module_script_tree as module_tree;
use moli_websocket::test_support::{
    header_value, spawn_abrupt_close_after_open_websocket_server,
    spawn_backpressure_websocket_server, spawn_child_document_and_header_capture_websocket_server,
    spawn_close_after_goodbye_websocket_server, spawn_cookie_echo_websocket_server,
    spawn_delayed_passive_close_websocket_server, spawn_dropping_websocket_server,
    spawn_header_capture_websocket_server, spawn_http_connect_proxy,
    spawn_raw_websocket_response_server, spawn_receive_backpressure_websocket_server,
    spawn_send_backpressure_websocket_server, spawn_server_close_websocket_server,
    spawn_server_close_websocket_server_with_frame, spawn_set_cookie_websocket_server,
    spawn_sleeping_handshake_websocket_server, spawn_subprotocol_websocket_server,
    spawn_text_echo_websocket_server, spawn_tls_header_capture_websocket_server,
    spawn_triggered_text_websocket_server,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use url::Url;

use super::super::{PageId, PageVmInitStage, PageVmNavigationTurnOutcome};
use super::{
    IntoPageTaskCompletion, PageDomManipulationTestFamily, PageSelectedTaskTestSelector, PageVm,
    PageVmEnvConfig, PageVmRuntimeHooks, PostParseLifecycleLoopAdvance,
};
use crate::frame_owner_model::{
    ChildDocumentModuleFetchTarget, ChildFrameSemanticTurnKind, FrameDocumentModuleClientEntryId,
    FrameDocumentModuleClientId, FrameDocumentModuleClientRegistration,
    FrameDocumentModuleClientReservation, FrameDocumentModuleDependencyFetchTask,
    FrameDocumentModuleFetchDisposition, FrameDocumentModulepreloadFetchTask,
    FrameDocumentModulepreloadLinkClient, FrameDocumentStaticDependencyModuleClient,
    FrameDocumentTaskOwner, FrameRealmId, FrameRequestId,
};
use crate::native_bridge::PendingWindowMessageEndpoint;

mod accessibility;
mod async_subresource_completion;
mod broadcast_channel_delivery;
mod child_autofocus;
mod child_classic_source_load_completion;
mod child_document_completion;
mod child_document_lifecycle;
mod child_document_script_ready;
mod child_dynamic_import_completion;
mod child_dynamic_import_owner_action;
mod child_host_load;
mod child_module_dependency_fetch_start;
mod child_module_document_script_ready;
mod child_module_error_reporting;
mod child_module_script_terminal;
mod child_module_script_terminal_completion;
mod child_modulepreload_event_action;
mod child_navigation_commit_completion;
mod child_parser_module_root_start_completion;
mod child_realm_materialization;
mod child_realm_materialization_completion;
mod command_checkpoint;
mod computed_size;
mod cssom_zoom;
mod dedicated_worker_client_event;
mod document_script_completion;
mod element_toggle_event;
mod fetch_xhr;
mod file_entry_file_callback;
mod file_system_directory_reader;
mod grid_item_box_generation;
mod grid_item_paint_order;
mod grid_resolved_track_values;
mod hash_change_delivery;
mod history_traversal;
mod image_load_event;
mod image_root_coordinates;
mod indexed_db;
mod inline_svg_paint;
mod internal_loading_completion;
mod intrinsic_percentage_resolution;
mod lifecycle;
mod main_document_post_parse_completion;
mod main_document_runtime;
mod main_dynamic_import_completion;
mod main_modulepreload_completion;
mod main_native_module_completion;
mod main_parser_continuation_task;
mod main_parser_module_completion;
mod main_parser_owned_module_completion;
mod main_runtime_module_completion;
mod main_runtime_script_completion;
mod media_element_event;
mod message_port_delivery;
mod misc_platform_api;
mod module_error_reporting;
mod module_reaction;
mod modulepreload_start_completion;
mod navigation_api_task;
mod opfs;
mod parser_written_script_residence;
mod popup_autofocus;
mod popup_document_completion;
mod preferred_aspect_ratio;
mod promise_rejection;
mod rendering_update;
mod script_preparation_error;
mod service_worker;
mod service_worker_client_message;
mod service_worker_internal;
mod shared_worker_client_event;
mod storage_event_delivery;
mod stylesheet_task;
mod text_track_default_mode;
mod text_track_load;
mod timer;
mod user_interaction;
mod view_transition;
mod wait_observer;
mod webcrypto;
mod websocket;
mod window_message;
mod worker;
mod worker_host_bridge;

fn has_ready_runtime_script_continuation_for_test(page_vm: &PageVm) -> bool {
    page_vm
        .page_task_executor_sources_for_test()
        .has_main_document_runtime_action_for_executor_test(
            PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
        )
}

pub(super) fn test_page_vm() -> PageVm {
    test_page_vm_with_config(FetchConfig::default(), Vec::new())
}

async fn park_current_document_websocket_for_test(
    page_vm: &mut PageVm,
    event: moli_websocket::Event,
) {
    assert!(
        page_vm
            .vm()
            .websocket_sender_for_test()
            .event_sender()
            .send(event)
            .await,
        "test WebSocket ingress should retain its typed Page route"
    );
    let task = page_vm
        .page_task_executor_sources_for_test()
        .take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::WebSocket { .. }
            )
        })
        .expect("test WebSocket ingress should publish one typed source head");
    let crate::page_task_queue::RendererPageSchedulerTask::WebSocket(task) = task else {
        panic!("WebSocket descriptor dequeued a different task variant")
    };
    task.return_backpressured();
    assert!(
        !page_vm.has_ready_page_websocket_task_for_test(),
        "current-Document backpressure must not remain runnable"
    );
}

fn run_next_resource_completion_as_typed_page_turn(
    page_vm: &mut PageVm,
) -> anyhow::Result<PageResourceCompletionTurnOutcome> {
    let mut page_resource_queue = page_vm.page_resource_completion_queue();
    page_vm
        .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
        .ok_or_else(|| anyhow::anyhow!("typed resource completion should consume one owner turn"))
}

async fn wait_for_typed_page_resource_completion(page_vm: &mut PageVm) -> bool {
    page_vm.wait_for_page_resource_completion_for_test().await
}

fn test_child_module_network_attribution(label: &str) -> ChildModuleFetchNetworkAttribution {
    ChildModuleFetchNetworkAttribution::parser(
        Some(format!("{label}-frame")),
        Url::parse(&format!("https://{label}.test/document")).unwrap(),
        Url::parse(&format!("https://{label}.test/module.js")).unwrap(),
    )
}

fn test_child_dynamic_import_network_attribution(
    label: &str,
) -> ChildModuleFetchNetworkAttribution {
    ChildModuleFetchNetworkAttribution::dynamic_import(
        Some(format!("{label}-frame")),
        Url::parse(&format!("https://{label}.test/document")).unwrap(),
        Url::parse(&format!("https://{label}.test/module.js")).unwrap(),
    )
}

fn test_child_parser_module_root_completion(
    child_handle: crate::dom::native::NativeNodeId,
    owner: FrameDocumentTaskOwner,
    request_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildParserModuleRootFetchCompletion {
    test_child_parser_module_root_completion_for_target(
        ChildDocumentModuleFetchTarget::new(child_handle, owner, FrameRealmId(109)),
        request_id,
        label,
        network_error,
    )
}

fn test_child_parser_module_root_completion_for_target(
    target: ChildDocumentModuleFetchTarget,
    request_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildParserModuleRootFetchCompletion {
    let network_attribution = test_child_module_network_attribution(label);
    ChildParserModuleRootFetchCompletion::new(
        target,
        FrameRequestId(request_id),
        ModuleMapKey::java_script(network_attribution.request_url().clone()),
        Err(format!("{label} root completion")),
        network_error.map(|error| Arc::new(Err(error.to_owned()))),
        network_attribution,
    )
}

fn test_child_module_dependency_completion(
    child_handle: crate::dom::native::NativeNodeId,
    owner: FrameDocumentTaskOwner,
    request_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildModuleDependencyFetchCompletion {
    test_child_module_dependency_completion_for_target(
        ChildDocumentModuleFetchTarget::new(child_handle, owner, FrameRealmId(109)),
        request_id,
        label,
        network_error,
    )
}

fn test_child_module_dependency_completion_for_target(
    target: ChildDocumentModuleFetchTarget,
    request_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildModuleDependencyFetchCompletion {
    let network_attribution = test_child_module_network_attribution(label);
    let task = test_child_module_dependency_fetch_task_for_target(target, request_id, label);
    ChildModuleDependencyFetchCompletion::new(
        target.child_handle(),
        FrameRequestId(request_id),
        task,
        Err(format!("{label} dependency completion")),
        network_error.map(|error| Arc::new(Err(error.to_owned()))),
        network_attribution,
    )
}

fn test_child_module_dependency_fetch_task_for_target(
    target: ChildDocumentModuleFetchTarget,
    request_id: u64,
    label: &str,
) -> FrameDocumentModuleDependencyFetchTask {
    let parent_url = Url::parse(&format!("https://{label}.test/root.js")).unwrap();
    let dependency_url = Url::parse(&format!("https://{label}.test/module.js")).unwrap();
    let parent_key = ModuleMapKey::java_script(parent_url.clone());
    let dependency_key = ModuleMapKey::java_script(dependency_url.clone());
    let parent_entry_id = ModuleEntryId::from_raw(113);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(127),
        sequence: request_id,
    };
    let client = FrameDocumentStaticDependencyModuleClient::new(
        parent_entry_id,
        parent_key.clone(),
        "./module.js".to_owned(),
        ModuleImportPhase::Evaluation,
        tree_client,
    );
    let entry_id = FrameDocumentModuleClientEntryId::from_raw(131);
    let reservation = FrameDocumentModuleClientReservation::new(
        target.task_owner().document_owner(),
        dependency_key.clone(),
        FrameDocumentModuleClientRegistration::new(
            entry_id,
            FrameDocumentModuleClientId::from_raw(request_id),
            FrameDocumentModuleFetchDisposition::StartedFetch(entry_id),
        ),
    );
    FrameDocumentModuleDependencyFetchTask::from_dependency_fetch_parts(
        target.task_owner(),
        target.realm_id(),
        dependency_key.clone(),
        client,
        reservation,
        NativeModuleGraphFetchRequest::new_tree_dependency_for_test(
            dependency_url,
            parent_url,
            ModuleFetchMetadata::default(),
            ModuleKind::JavaScript,
            tree_client,
            dependency_key,
            parent_key,
            parent_entry_id,
            "./module.js".to_owned(),
            ModuleImportPhase::Evaluation,
        ),
    )
}

fn test_child_modulepreload_completion_for_target(
    target: ChildDocumentModuleFetchTarget,
    load_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildModulepreloadFetchCompletion {
    ChildModulepreloadFetchCompletion::new(
        target,
        load_id,
        Err(format!("{label} modulepreload completion")),
        network_error.map(|error| Arc::new(Err(error.to_owned()))),
        test_child_module_network_attribution(label),
    )
}

fn test_child_dynamic_import_completion_for_target(
    target: ChildDocumentModuleFetchTarget,
    load_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildDynamicImportFetchCompletion {
    ChildDynamicImportFetchCompletion::new(
        target,
        load_id,
        Err(format!("{label} dynamic import completion")),
        network_error.map(|error| Arc::new(Err(error.to_owned()))),
        test_child_dynamic_import_network_attribution(label),
    )
}

fn test_child_modulepreload_start_task_for_target(
    target: ChildDocumentModuleFetchTarget,
    label: &str,
) -> FrameDocumentModulepreloadFetchTask {
    let source_url = Url::parse(&format!("https://{label}.test/modulepreload.js"))
        .expect("modulepreload start URL");
    FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        target.realm_id(),
        FrameDocumentModulepreloadLinkClient::new(
            target.child_handle(),
            target.task_owner(),
            crate::dom::native::NativeNodeId::new(997),
        ),
        NativeModuleSingleFetchRequest::new(
            source_url.clone(),
            source_url.clone(),
            source_url.clone(),
            ModuleMapKey::java_script(source_url),
            ModuleFetchMetadata::default(),
        ),
    )
}

fn test_page_vm_with_config(
    config: FetchConfig,
    extra_http_headers: Vec<(String, String)>,
) -> PageVm {
    let loader_owner = crate::network::ResourceRequestClient::new(&config).expect("loader");
    let mut page_vm = test_page_vm_with_loader(&loader_owner, extra_http_headers);
    page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
    page_vm
}

fn test_page_vm_with_loader(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
) -> PageVm {
    test_page_vm_with_loader_and_document_url(
        loader,
        extra_http_headers,
        Url::parse("https://example.com/").unwrap(),
    )
}

fn test_page_vm_with_document_url(document_url: Url) -> PageVm {
    let loader_owner =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let mut page_vm =
        test_page_vm_with_loader_and_document_url(&loader_owner, Vec::new(), document_url);
    page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
    page_vm
}

fn test_page_vm_with_root_frame_id(root_frame_id: &str) -> PageVm {
    let loader_owner =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let dom_host = DomHost::from_dom(HtmlParser::SCRIPTING_ENABLED.parse(
        Url::parse("https://example.com/").expect("test Document URL"),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    ));
    let mut page_vm = test_page_vm_with_loader_dom_host_hooks_and_response_referrer_policy(
        &loader_owner,
        Vec::new(),
        dom_host,
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
        None,
        Some(root_frame_id.to_owned()),
    );
    page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
    page_vm
}

fn test_page_vm_with_response_referrer_policy(
    document_url: Url,
    response_referrer_policy: impl Into<String>,
) -> PageVm {
    let loader_owner =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let mut page_vm = test_page_vm_with_loader_document_url_hooks_and_response_referrer_policy(
        &loader_owner,
        Vec::new(),
        document_url,
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
        Some(response_referrer_policy.into()),
    );
    page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
    page_vm
}

fn dns_failure_fetch_config() -> FetchConfig {
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    config.set_request_timeout_ms(2_000);
    config.set_connect_timeout_ms(Some(500));
    config
}

fn test_page_vm_with_loader_and_document_url(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
    document_url: Url,
) -> PageVm {
    test_page_vm_with_loader_document_url_and_hooks(
        loader,
        extra_http_headers,
        document_url,
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
    )
}

fn test_page_vm_with_loader_document_url_and_hooks(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
    document_url: Url,
    runtime_hooks: PageVmRuntimeHooks,
) -> PageVm {
    test_page_vm_with_loader_document_url_hooks_and_response_referrer_policy(
        loader,
        extra_http_headers,
        document_url,
        runtime_hooks,
        None,
    )
}

/// Builds a low-level PageVm executor fixture with the production Page task
/// sources and wake route bound.
///
/// It deliberately has no `RendererOwnerLocalPageSlot` or scheduler
/// residence. Tests using it may prove producer routing, exact-owner
/// arbitration, and one-task execution, but not owner admission, fairness, or
/// autonomous liveness.
fn page_vm_with_bound_task_sources_and_owner_wake(
    loader: &crate::network::ResourceRequestClient,
    document_url: Url,
) -> (
    PageVm,
    crate::page_task_queue::RendererPageResourceCompletionTestSource,
    tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) {
    let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
        wake_tx,
        crate::runtime::RendererPageToken::new_for_testing(PageId::new_for_testing(1)),
    );
    let hooks = PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
        owner_wake,
    );
    let page_vm =
        test_page_vm_with_loader_document_url_and_hooks(loader, Vec::new(), document_url, hooks);
    let queue = page_vm.page_resource_completion_queue();
    (page_vm, queue, wake_rx)
}

/// Materialize the only child fixture through the production producer,
/// stable source, exact-owner arbiter, and one-turn executor.
fn materialize_child_realm_through_page_turn_for_test(
    page_vm: &mut PageVm,
    element_id: &str,
) -> anyhow::Result<crate::page_task_queue::PageChildRealmMaterializationTurnOutcome> {
    let child_handle = page_vm
        .vm()
        .element_handle_by_id_for_test(element_id)
        .expect("child realm fixture should retain its iframe handle");
    page_vm.vm_mut().eval(&format!(
        "void document.getElementById({element_id:?}).contentWindow.Function; 'queued'"
    ))?;
    let outcome = page_vm
        .run_child_realm_materialization_body_for_test()?
        .expect("child Window exposure should enqueue one typed realm turn");
    assert_eq!(
        outcome.action.owner.target().child_handle(),
        Some(child_handle)
    );
    assert_eq!(
        outcome.action.target_effect,
        crate::page_task_queue::PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
    );
    Ok(outcome)
}

fn run_expected_pending_child_realm_materialization_turn(
    page_vm: &mut PageVm,
    label: &str,
) -> anyhow::Result<crate::page_task_queue::PageChildRealmMaterializationTurnOutcome> {
    let outcome = page_vm
        .run_child_realm_materialization_body_for_test()?
        .unwrap_or_else(|| panic!("{label} should consume one typed child-realm turn"));
    assert_eq!(
        outcome.action.target_effect,
        crate::page_task_queue::PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript,
        "{label} must materialize the exact current child Document"
    );
    Ok(outcome)
}

fn materialize_only_child_realm_execution_context_through_page_turn_for_test(
    page_vm: &mut PageVm,
    element_id: &str,
) -> anyhow::Result<i64> {
    let _ = materialize_child_realm_through_page_turn_for_test(page_vm, element_id)?;
    let realms = page_vm
        .vm_mut()
        .live_child_default_runtime_realm_inventory();
    assert_eq!(
        realms.len(),
        1,
        "single-child PageVm fixture should expose exactly one live child realm"
    );
    Ok(realms[0].context_id)
}

fn test_page_vm_with_loader_document_url_hooks_and_response_referrer_policy(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
    document_url: Url,
    runtime_hooks: PageVmRuntimeHooks,
    response_referrer_policy: Option<String>,
) -> PageVm {
    let dom_host = DomHost::from_dom(HtmlParser::SCRIPTING_ENABLED.parse(
        document_url,
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    ));
    test_page_vm_with_loader_dom_host_hooks_and_response_referrer_policy(
        loader,
        extra_http_headers,
        dom_host,
        runtime_hooks,
        response_referrer_policy,
        None,
    )
}

fn test_page_vm_with_loader_and_dom_host(
    loader: &crate::network::ResourceRequestClient,
    dom_host: DomHost,
) -> PageVm {
    test_page_vm_with_loader_dom_host_hooks_and_response_referrer_policy(
        loader,
        Vec::new(),
        dom_host,
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
        None,
        None,
    )
}

fn test_page_vm_with_loader_dom_host_hooks_and_response_referrer_policy(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
    dom_host: DomHost,
    runtime_hooks: PageVmRuntimeHooks,
    response_referrer_policy: Option<String>,
    root_frame_id: Option<String>,
) -> PageVm {
    let _js_runtime = crate::JsRuntime::initialize();
    let local_executor = crate::local_executor::JsLocalExecutor::new();
    PageVm::new(
        PageId::new_for_testing(1),
        local_executor,
        loader,
        &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
            root_frame_id,
            main_document_commit: None,
            top_level_storage_key: None,
            document_start_scripts: vec![],
            runtime_bindings: vec![],
            runtime_inspector_session_restore_snapshots: vec![],
            runtime_isolated_worlds: vec![],
            permission_overrides: vec![],
            extra_http_headers: extra_http_headers.into(),
            navigator_identity: loader.browser_identity().clone(),
            document_policy_container: crate::document_runtime::DocumentPolicyContainer {
                referrer_policy: response_referrer_policy,
                ..Default::default()
            },
            document_default_language: None,
            document_last_modified: None,
            document_settings: Default::default(),
            network_offline: false,
            blocked_url_patterns: Vec::new(),
            indexed_db_manager: None,
            storage_bucket_store: None,
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
            layout_configuration: moli_page_types::LayoutConfiguration {
                policy: crate::real_layout_test_policy(),
                scrollbars_hidden: false,
            },
            wpt_extensions_enabled: false,
            navigation_bootstrap_entry: None,
            about_document_state: None,
            navigation_history_source: None,
            reserved_service_worker_client_id: None,
        },
        runtime_hooks,
        dom_host,
        Instant::now(),
    )
    .expect("page vm")
}

fn prepared_external_module_for_page_vm_test(page_vm: &PageVm, url: Url) -> PreparedScript {
    prepared_external_module_for_page_vm_test_with_node(page_vm, 9001, url)
}

fn prepared_external_module_for_page_vm_test_with_node(
    page_vm: &PageVm,
    node: u32,
    url: Url,
) -> PreparedScript {
    PreparedScript {
        document_referrer_policy: None,
        position: node as usize,
        node_id: NodeId::new(node as usize),
        kind: ScriptKind::Module,
        mode: ScriptMode::ModuleDefer,
        source_kind: ScriptSourceKind::External,
        fetch_metadata: ScriptFetchMetadata::default(),
        source: ScriptSource::External,
        initiator_url: page_vm.vm().document_runtime.document_url().clone(),
        base_url: url.clone(),
        url,
        host_script_handle: None,
    }
}

async fn drive_child_frame_task_sources_until_resource_completion_ready(
    page_vm: &mut PageVm,
    max_turns: usize,
) -> Vec<ChildFrameSemanticTurnKind> {
    let mut sources = Vec::new();
    for _ in 0..max_turns {
        if page_vm.has_ready_page_websocket_task_for_test() {
            break;
        }
        let Some(source) = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
        else {
            break;
        };
        sources.push(source);
    }
    sources
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildModulepreloadStartupTurn {
    TypedModulepreloadStart,
    ChildSemanticTurn(ChildFrameSemanticTurnKind),
}

async fn run_expected_child_modulepreload_event_action_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) {
    assert!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildModulepreloadEventAction,
                loader,
            )
            .await
            .unwrap_or_else(|error| panic!("{label} selected task should succeed: {error:#}")),
        "{label} should consume one typed modulepreload event action through the selected-task dispatcher"
    );
}

/// Adjacent module-map helper that preserves distinct modulepreload and child
/// semantic turns. The modulepreload start enters the production selected-task
/// dispatcher; the returned sequence still must not be used as owner-scheduler
/// or cross-source fairness evidence.
async fn drive_child_modulepreload_startup_until_resource_completion_ready(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    max_turns: usize,
) -> Vec<ChildModulepreloadStartupTurn> {
    let mut turns = Vec::new();
    for _ in 0..max_turns {
        if page_vm.has_ready_page_websocket_task_for_test() {
            break;
        }
        if page_vm
            .page_task_executor_sources_for_test()
            .modulepreload_start()
            .has_ready_task()
        {
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ModulepreloadStart,
                        loader,
                    )
                    .await
                    .expect("selected child modulepreload start should succeed"),
                "typed child modulepreload start should consume one source head through the production dispatcher",
            );
            turns.push(ChildModulepreloadStartupTurn::TypedModulepreloadStart);
            continue;
        }
        let Some(source) = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
        else {
            break;
        };
        turns.push(ChildModulepreloadStartupTurn::ChildSemanticTurn(source));
    }
    turns
}

/// Settle at most one exact realm-materialization prerequisite before running
/// the requested child-family turn. The production semantic helper remains a
/// strict one-turn operation.
async fn run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
    page_vm: &mut PageVm,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    label: &str,
) -> ChildFrameSemanticTurnKind {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && page_vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::RealmMaterialization,
        )
    {
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "{label} should first materialize the exact child realm"
        );
    }
    let Some(source) = page_vm
        .run_next_child_frame_task_source_for_semantic_test()
        .await
    else {
        panic!("{label} should produce {expected:?}, but no child frame task work was ready");
    };
    assert_eq!(
        source, expected,
        "{label} should run the expected child frame task source"
    );
    source
}

async fn run_expected_child_realm_materialization_for_wait(
    page_vm: &mut PageVm,
    label: &str,
) -> ChildFrameSemanticTurnKind {
    let source = page_vm
        .run_next_child_frame_task_source_for_semantic_test()
        .await;
    assert_eq!(
        source,
        Some(ChildFrameSemanticTurnKind::RealmMaterialization),
        "{label} should materialize its exact child realm in one explicit turn"
    );
    ChildFrameSemanticTurnKind::RealmMaterialization
}

async fn run_expected_child_module_script_terminal_turn(page_vm: &mut PageVm, label: &str) {
    let loader = page_vm.request_client.clone();
    let claimed = page_vm
        .claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::ChildModuleScriptTerminal,
        )
        .unwrap_or_else(|| panic!("{label} should produce one selected module-terminal task"));
    let owner = claimed
        .child_module_script_terminal_owner()
        .expect("exact terminal selector must retain its typed owner");
    assert_eq!(
        owner.root_document(),
        page_vm.document_lifecycle.identity().document,
        "{label} must belong to the current root Document",
    );
    assert_eq!(
        page_vm
            .vm()
            .current_child_module_script_terminal_owner(owner.document_owner(), owner.realm_id(),),
        Some(owner.document_owner()),
        "{label} must belong to the current child Document/realm",
    );
    page_vm
        .run_claimed_selected_page_task_for_test(claimed, &loader)
        .await
        .unwrap_or_else(|error| panic!("{label} selected task failed: {error:#}"));
}

async fn run_child_domcontentloaded_then_host_load_for_wait(
    page_vm: &mut PageVm,
    label: &str,
) -> ChildFrameSemanticTurnKind {
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        &format!("{label} DOMContentLoaded transition"),
    )
    .await;
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        &format!("{label} complete transition"),
    )
    .await;
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::HostLoad,
        label,
    )
    .await
}

async fn run_child_interactive_domcontentloaded_then_host_load_for_wait(
    page_vm: &mut PageVm,
    label: &str,
) -> ChildFrameSemanticTurnKind {
    if page_vm.has_ready_child_frame_semantic_turn_for_test(
        ChildFrameSemanticTurnKind::RealmMaterialization,
    ) {
        run_expected_child_realm_materialization_for_wait(page_vm, label).await;
    }
    if matches!(page_vm.page_task_executor_sources_for_test().next_child_frame_task_target(), Some(crate::page_task_queue::RendererPageChildFrameTaskTarget::DocumentLifecycle(target)) if matches!(target.action(), crate::frame_owner_model::FrameDocumentLifecycleAction::Interactive(_)))
    {
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            page_vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("{label} queued interactive transition"),
        )
        .await;
    }
    run_child_domcontentloaded_then_host_load_for_wait(page_vm, label).await
}

fn prepared_inline_module_for_page_vm_test(
    page_vm: &PageVm,
    node: u32,
    source: &str,
) -> PreparedScript {
    PreparedScript {
        document_referrer_policy: None,
        position: node as usize,
        node_id: NodeId::new(node as usize),
        kind: ScriptKind::Module,
        mode: ScriptMode::ModuleDefer,
        source_kind: ScriptSourceKind::Inline,
        fetch_metadata: ScriptFetchMetadata::default(),
        source: ScriptSource::Inline(source.to_owned()),
        initiator_url: page_vm.vm().document_runtime.document_url().clone(),
        base_url: page_vm.vm().document_runtime.document_url().clone(),
        url: page_vm.vm().document_runtime.document_url().clone(),
        host_script_handle: None,
    }
}

fn prepared_loaded_classic_for_page_vm_test(
    page_vm: &PageVm,
    node: u32,
    source: &str,
) -> PreparedScript {
    PreparedScript {
        document_referrer_policy: None,
        position: node as usize,
        node_id: NodeId::new(node as usize),
        kind: ScriptKind::Classic,
        mode: ScriptMode::Normal,
        source_kind: ScriptSourceKind::Inline,
        fetch_metadata: ScriptFetchMetadata::default(),
        source: ScriptSource::Loaded(source.to_owned()),
        initiator_url: page_vm.vm().document_runtime.document_url().clone(),
        base_url: page_vm.vm().document_runtime.document_url().clone(),
        url: page_vm.vm().document_runtime.document_url().clone(),
        host_script_handle: None,
    }
}

fn append_parser_owned_external_classic_defer_for_page_vm_test(
    page_vm: &mut PageVm,
    position: usize,
    element_id: &str,
    script_url: Url,
    source: ScriptSource,
    completion_attribute: (&str, &str),
) -> PreparedScript {
    let body = page_vm
        .vm()
        .snapshot_live_document()
        .document_body_handle()
        .expect("test document body");
    let script_node = page_vm
        .vm_mut()
        .document_runtime
        .dom_host_mut()
        .create_parser_element_without_attributes(
            "script".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
    {
        let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
        assert!(dom_host.set_attribute(script_node, "id", element_id));
        assert!(dom_host.set_attribute(script_node, "src", script_url.as_str()));
        assert!(dom_host.set_attribute(
            script_node,
            completion_attribute.0,
            completion_attribute.1,
        ));
        assert!(dom_host.append_child(body, script_node));
    }
    let host_script_handle = page_vm
        .vm_mut()
        .document_runtime
        .bind_parser_owned_script_handle_for_node(script_node);
    PreparedScript {
        document_referrer_policy: None,
        position,
        node_id: NodeId::new(script_node.index()),
        kind: ScriptKind::Classic,
        mode: ScriptMode::Defer,
        source_kind: ScriptSourceKind::External,
        fetch_metadata: ScriptFetchMetadata::default(),
        source,
        initiator_url: page_vm.vm().document_runtime.document_url().clone(),
        base_url: script_url.clone(),
        url: script_url,
        host_script_handle: Some(host_script_handle),
    }
}

fn enqueue_parser_owned_module_script_fetch_completion_for_test(
    page_vm: &mut PageVm,
    load_id: u64,
    module_url: &Url,
    source: &str,
) {
    let target = page_vm
        .vm()
        .current_main_parser_module_graph_fetch_target(load_id)
        .unwrap_or_else(|| panic!("parser module fetch {load_id} must retain its exact target"));
    enqueue_parser_owned_module_script_fetch_completion_for_target_for_test(
        page_vm, target, module_url, source,
    );
}

fn enqueue_parser_owned_module_script_fetch_completion_for_target_for_test(
    page_vm: &mut PageVm,
    target: MainParserModuleGraphFetchTarget,
    module_url: &Url,
    source: &str,
) {
    let document_url = page_vm.vm().document_runtime.document_url().clone();
    page_vm
        .vm()
        .resource_completion_sender_for_test()
        .send_main_parser_module_graph_fetch(MainParserModuleGraphFetchCompletion::new(
            target,
            Ok(ModuleGraphFetchedSource::new(
                module_url.clone(),
                false,
                ModuleSource::text(source.to_owned()),
            )),
            None,
            MainModuleFetchNetworkAttribution::new(document_url, module_url.clone()),
        ))
        .expect("module graph completion should enqueue");
}

fn enqueue_parser_owned_module_script_fetch_error_for_test(
    page_vm: &mut PageVm,
    load_id: u64,
    module_url: &Url,
    message: &str,
) {
    let target = page_vm
        .vm()
        .current_main_parser_module_graph_fetch_target(load_id)
        .unwrap_or_else(|| panic!("parser module fetch {load_id} must retain its exact target"));
    let document_url = page_vm.vm().document_runtime.document_url().clone();
    page_vm
        .vm()
        .resource_completion_sender_for_test()
        .send_main_parser_module_graph_fetch(MainParserModuleGraphFetchCompletion::new(
            target,
            Err(message.to_owned()),
            None,
            MainModuleFetchNetworkAttribution::new(document_url, module_url.clone()),
        ))
        .expect("module graph error completion should enqueue");
}

fn run_next_main_module_fetch_terminal_for_test(
    page_vm: &mut PageVm,
) -> anyhow::Result<Option<RendererOwnerResourceActivitySource>> {
    let outcome = run_next_resource_completion_as_typed_page_turn(page_vm)?;
    anyhow::ensure!(
        outcome.action.source == RendererOwnerResourceActivitySource::ModuleGraphFetch,
        "expected a typed main module fetch terminal, got {:?}",
        outcome.action.source
    );
    Ok(Some(outcome.action.source))
}

async fn run_next_native_module_owner_event_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) {
    let outcome = page_vm
        .run_page_main_document_runtime_body_for_test(loader)
        .await
        .unwrap_or_else(|error| panic!("{label} owner event should run: {error}"))
        .unwrap_or_else(|| panic!("{label} should retain one native module owner-event turn"));
    assert_eq!(
        outcome.action.kind(),
        crate::page_task_queue::PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent,
        "{label} must resume joined module-map clients through their typed owner source"
    );
}

fn enqueue_main_modulepreload_fetch_error_for_test(
    page_vm: &mut PageVm,
    load_id: u64,
    module_url: &Url,
    message: &str,
) {
    let target = page_vm
        .vm()
        .current_main_modulepreload_fetch_target(load_id)
        .unwrap_or_else(|| panic!("modulepreload fetch {load_id} must retain its exact target"));
    let document_url = page_vm.vm().document_runtime.document_url().clone();
    page_vm
        .vm()
        .resource_completion_sender_for_test()
        .send_main_modulepreload_fetch(MainModulepreloadFetchCompletion::new(
            target,
            Err(message.to_owned()),
            None,
            MainModuleFetchNetworkAttribution::new(document_url, module_url.clone()),
        ))
        .expect("modulepreload graph completion should enqueue");
}

async fn run_parser_module_completion_turns_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    expected_ready_turns: usize,
    label: &str,
) {
    page_vm
        .vm_mut()
        .perform_script_task_checkpoint(None)
        .expect("module evaluation reaction checkpoint should run");
    while page_vm
        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::ModuleReaction, loader)
        .await
        .expect("module reactions should run")
    {}
    for turn in 0..expected_ready_turns {
        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(page_vm, loader)
                .await
                .expect("module evaluation completion should run"),
            "{label} should run parser-owned ready turn {} of {expected_ready_turns}",
            turn + 1
        );
    }
    assert!(
        !page_vm.has_ready_parser_owned_document_script_action(),
        "{label} should not leave another concrete parser-owned module action ready"
    );
}

async fn run_one_parser_owned_main_document_runtime_turn_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<bool> {
    page_vm.admit_ready_parser_owned_document_script_action();
    page_vm
        .run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::MainDocumentRuntime(
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation,
            ),
            loader,
        )
        .await
}

fn post_parse_document_script_work(
    lane: DocumentScriptExecutionLane,
    script: PreparedScript,
) -> PostParsePageOwnedWork {
    PostParsePageOwnedWork::document_script_work(PageOwnedDocumentScriptWork::script(lane, script))
}

fn install_parser_module_defer_work(
    page_vm: &mut PageVm,
    script: PreparedScript,
) -> PostParsePageOwnedWork {
    let task_owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("parser module test requires a current document owner");
    assert!(
        page_vm
            .vm_mut()
            .claim_main_parser_deferred_script(task_owner, script, None, None, Default::default(),)
            .expect("parser module PendingScript acceptance should start its graph")
    );
    page_vm
        .seal_main_parser_deferred_scripts(task_owner)
        .expect("parser module PendingScript should install into after-parsing order")
}

/// Execute only the parser-deferred domain body.
///
/// This helper intentionally does not prove selected-task checkpoint or exact
/// DCL handoff behavior. Tests asserting a complete parser task must use
/// `run_and_finish_ready_parser_deferred_task_for_test()` instead.
async fn run_ready_parser_deferred_body_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) {
    let action = poll_post_parse_document_processing_action_for_test(page_vm)
        .unwrap_or_else(|| panic!("{label} should expose a ready parser-deferred owner action"));
    let crate::document_runtime::DocumentProcessingAction::PostParsePageOwnedWork(work) = action
    else {
        panic!("{label} should produce page-owned parser-deferred work");
    };
    assert!(
        work.main_parser_deferred_scripts_owner().is_some(),
        "{label} should produce the armed parser-deferred marker, got {work:?}"
    );
    page_vm
        .execute_post_parse_page_owned_task_on_named_owner_lane(loader, *work)
        .await
        .unwrap_or_else(|error| panic!("{label} parser-deferred turn should run: {error}"));
}

async fn run_and_finish_ready_parser_deferred_task_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) {
    let action = poll_post_parse_document_processing_action_for_test(page_vm)
        .unwrap_or_else(|| panic!("{label} should expose a ready parser-deferred owner action"));
    let crate::document_runtime::DocumentProcessingAction::PostParsePageOwnedWork(work) = action
    else {
        panic!("{label} should produce page-owned parser-deferred work");
    };
    assert!(
        work.main_parser_deferred_scripts_owner().is_some(),
        "{label} should produce the armed parser-deferred marker, got {work:?}"
    );
    let completion = page_vm
        .execute_post_parse_page_owned_task_on_named_owner_lane(loader, *work)
        .await
        .unwrap_or_else(|error| panic!("{label} parser-deferred body should run: {error}"));
    let super::parser_completion::SelectedPostParsePageOwnedCompletion::MainParser(completion) =
        completion
    else {
        panic!("{label} must retain its main-parser completion authority");
    };
    page_vm
        .finish_parse_time_main_parser_boundary(completion)
        .await
        .unwrap_or_else(|error| panic!("{label} parser task completion should run: {error}"));
}

fn poll_post_parse_document_processing_action_for_test(
    page_vm: &mut PageVm,
) -> Option<crate::document_runtime::DocumentProcessingAction> {
    let (vm, task_queue) = (&mut page_vm.vm, &mut page_vm.page_task_queue);
    vm.as_mut()
        .expect("test PageVm should retain ScriptVm")
        .document_runtime
        .poll_document_processing_action(task_queue, Option::<&crate::dom::native::NativeDom>::None)
}

async fn run_main_async_script_load_delay_settlement_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    label: &str,
) {
    for turn in 0..2 {
        if let Some(outcome) = page_vm
            .run_page_main_document_runtime_body_for_test(loader)
            .await
            .unwrap_or_else(|error| panic!("{label} follow-up turn should run: {error}"))
        {
            assert_eq!(
                outcome.action.kind(),
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::PostParseWork
            );
            assert_eq!(outcome.action.owner().document_owner(), owner);
            assert_eq!(
                outcome.action.target_effect(),
                crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
            );
        } else {
            // Parse-time async classic work can finish before DCL. Its event
            // and settlement therefore remain lifecycle-owned rather than
            // entering the post-DCL MainDocumentRuntime source.
            let action = page_vm
                .vm_mut()
                .document_runtime
                .pop_parser_owned_pre_domcontentloaded_action()
                .or_else(|| poll_post_parse_document_processing_action_for_test(page_vm))
                .unwrap_or_else(|| panic!("{label} should enqueue an explicit follow-up"));
            let crate::document_runtime::DocumentProcessingAction::PostParsePageOwnedWork(work) =
                action
            else {
                panic!("{label} async follow-up must remain page-owned work");
            };
            if let Some(PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(binding)) =
                work.as_lifecycle_work()
            {
                assert_eq!(binding.owner(), owner);
            }
            page_vm
                .execute_post_parse_page_owned_task_on_named_owner_lane(loader, *work)
                .await
                .unwrap_or_else(|error| panic!("{label} lifecycle follow-up should run: {error}"));
        }
        if page_vm
            .vm()
            .current_main_document_has_async_script_load_delay(owner)
            == Some(false)
        {
            return;
        }
        assert_eq!(
            turn, 0,
            "{label} must settle after at most one observable event turn"
        );
    }
    panic!("{label} left its exact main-Document load delay unsettled");
}

fn classic_defer_work(script: PreparedScript) -> PostParsePageOwnedWork {
    post_parse_document_script_work(DocumentScriptExecutionLane::ClassicDefer, script)
}

fn split_network_output_items(
    output: ScriptNetworkOutput,
) -> (
    Vec<SubresourceNetworkRecord>,
    Vec<WebSocketNetworkEvent>,
    Vec<WebSocketLifecycleEvent>,
) {
    let mut records = Vec::new();
    let mut frame_events = Vec::new();
    let mut lifecycle_events = Vec::new();
    for item in output.into_items() {
        match item {
            ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => records.push(*record),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(event) => frame_events.push(event),
            ScriptNetworkOutputItem::WebSocketLifecycleEvent(event) => {
                lifecycle_events.push(event);
            }
            ScriptNetworkOutputItem::SubresourceRequestStarted(_)
            | ScriptNetworkOutputItem::SubresourceResponseStarted(_)
            | ScriptNetworkOutputItem::SubresourceDataReceived(_)
            | ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_)
            | ScriptNetworkOutputItem::SubresourceBodyFinished(_) => {}
        }
    }
    (records, frame_events, lifecycle_events)
}

fn detached_test_run() -> ScriptRun {
    ScriptRun::skipped(
        NodeId::new(99),
        ScriptKind::Classic,
        ScriptMode::Async,
        ScriptSourceKind::External,
        Url::parse("https://example.com/detached.js").unwrap(),
        ScriptSkipReason::NotInMainDocument,
    )
}

async fn run_page_vm_async_test<F, R>(future: F) -> R
where
    F: std::future::Future<Output = R> + 'static,
    R: 'static,
{
    tokio::task::LocalSet::new()
        .run_until(async move {
            tokio::task::spawn_local(future)
                .await
                .expect("page_vm async test task should finish")
        })
        .await
}

async fn run_on_page_vm_local_executor<F, R>(
    local_executor: crate::local_executor::JsLocalExecutor,
    future: F,
) -> R
where
    F: std::future::Future<Output = R> + 'static,
    R: 'static,
{
    local_executor.run(future).await
}

fn run_page_vm_local_runtime_test<F, Fut>(thread_name: &'static str, build: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(thread_name.to_owned())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build_local(tokio::runtime::LocalOptions::default())
                .expect("local-runtime page_vm test runtime should build")
                .block_on(build());
        })
        .expect("local-runtime page_vm test thread should spawn")
        .join()
        .expect("local-runtime page_vm test thread should finish");
}

fn run_page_vm_local_runtime_async_test<F, Fut>(thread_name: &'static str, build: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    run_page_vm_local_runtime_test(thread_name, || async move {
        run_page_vm_async_test(build()).await;
    });
}

fn run_page_vm_large_stack_async_test<F, Fut>(thread_name: &'static str, build: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(thread_name.to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("large-stack page_vm test runtime should build")
                .block_on(run_page_vm_async_test(build()));
        })
        .expect("large-stack page_vm test thread should spawn")
        .join()
        .expect("large-stack page_vm test thread should finish");
}

async fn read_http_request_head(stream: &mut tokio::net::TcpStream) -> std::io::Result<String> {
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = stream.read(&mut byte).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "client closed before sending complete request",
            ));
        }
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            return Ok(String::from_utf8_lossy(&request).into_owned());
        }
    }
}

async fn read_http_request_with_body(
    stream: &mut tokio::net::TcpStream,
) -> std::io::Result<String> {
    let head = read_http_request_head(stream).await?;
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = vec![0_u8; content_length];
    if content_length > 0 {
        stream.read_exact(&mut body).await?;
    }
    Ok(format!("{head}{}", String::from_utf8_lossy(&body)))
}

async fn spawn_single_response_http_server(
    status_line: &'static str,
    body: String,
    delay: Duration,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local worker test server");
    let addr = listener.local_addr().expect("server local addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker script request");
        read_http_request_head(&mut stream)
            .await
            .expect("read worker script request");
        if !delay.is_zero() {
            sleep(delay).await;
        }
        let response = format!(
            "{status_line}\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker script response");
    });
    (format!("http://{addr}"), server)
}

async fn spawn_path_response_http_server(
    response_specs: Vec<(&'static str, &'static str, String, Duration)>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local multi-response worker test server");
    let addr = listener.local_addr().expect("server local addr");
    let server = tokio::spawn(async move {
        let mut response_queues = std::collections::HashMap::<
            String,
            std::collections::VecDeque<(&'static str, String, Duration)>,
        >::new();
        for (path, status, body, delay) in response_specs {
            response_queues
                .entry(path.to_owned())
                .or_default()
                .push_back((status, body, delay));
        }
        while response_queues.values().any(|queue| !queue.is_empty()) {
            let (mut stream, _) = listener.accept().await.expect("accept worker request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("request path");
            let (status_line, body, delay) = response_queues
                .get_mut(path)
                .and_then(|queue| queue.pop_front())
                .unwrap_or_else(|| panic!("unexpected worker test request path: {path}"));
            if !delay.is_zero() {
                sleep(delay).await;
            }
            let path_without_query = path.split_once('?').map_or(path, |(path, _)| path);
            let content_type =
                if path_without_query.ends_with(".js") || path_without_query.ends_with(".mjs") {
                    "text/javascript"
                } else if path_without_query.ends_with(".css") {
                    "text/css"
                } else {
                    "text/html"
                };
            let response = format!(
                "{status_line}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write worker test response");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_moved_child_defer_http_server() -> (String, oneshot::Sender<()>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind defer fixture");
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (release_classic, classic_released) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut classic_released = Some(classic_released);
        let mut responses = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept script request");
            let request = read_http_request_head(&mut stream).await.unwrap();
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let (body, gate) = match path {
                "/moved-child-defer.js" => (
                    "parent.__movedChildDeferEvents.push('classic-ran');",
                    Some(classic_released.take().expect("one classic request")),
                ),
                "/later-moved-module.js" => {
                    ("parent.__movedChildDeferEvents.push('module-ran');", None)
                }
                _ => panic!("unexpected script path: {path}"),
            };
            responses.push(tokio::spawn(async move {
                if let Some(gate) = gate {
                    gate.await.expect("test should release classic response");
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }));
        }
        for response in responses {
            response.await.expect("script response task");
        }
    });
    (base_url, release_classic, task)
}

async fn spawn_concurrent_path_response_http_server(
    response_specs: Vec<(&'static str, &'static str, String, Duration)>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local concurrent multi-response worker test server");
    let addr = listener.local_addr().expect("server local addr");
    let request_count = response_specs.len();
    let response_queues = {
        let mut queues = std::collections::HashMap::<
            String,
            std::collections::VecDeque<(&'static str, String, Duration)>,
        >::new();
        for (path, status, body, delay) in response_specs {
            queues
                .entry(path.to_owned())
                .or_default()
                .push_back((status, body, delay));
        }
        std::sync::Arc::new(tokio::sync::Mutex::new(queues))
    };
    let server = tokio::spawn(async move {
        let mut response_tasks = Vec::with_capacity(request_count);
        for _ in 0..request_count {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept concurrent worker request");
            let response_queues = response_queues.clone();
            response_tasks.push(tokio::spawn(async move {
                let request = read_http_request_head(&mut stream)
                    .await
                    .expect("read concurrent worker request");
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("request path");
                let (status_line, body, delay) = {
                    let mut response_queues = response_queues.lock().await;
                    response_queues
                        .get_mut(path)
                        .and_then(|queue| queue.pop_front())
                        .unwrap_or_else(|| {
                            panic!("unexpected concurrent worker test request path: {path}")
                        })
                };
                if !delay.is_zero() {
                    sleep(delay).await;
                }
                let path_without_query = path.split_once('?').map_or(path, |(path, _)| path);
                let content_type =
                    if path_without_query.ends_with(".js") || path_without_query.ends_with(".mjs")
                    {
                        "text/javascript"
                    } else if path_without_query.ends_with(".css") {
                        "text/css"
                    } else {
                        "text/html"
                    };
                let response = format!(
                    "{status_line}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write concurrent worker test response");
            }));
        }
        for task in response_tasks {
            task.await.expect("concurrent worker response task");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_shutdown_path_response_http_server(
    response_specs: Vec<(&'static str, &'static str, String, Duration)>,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    JoinHandle<Vec<String>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local shutdown multi-response worker test server");
    let addr = listener.local_addr().expect("server local addr");
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut response_queues = std::collections::HashMap::<
            String,
            std::collections::VecDeque<(&'static str, String, Duration)>,
        >::new();
        for (path, status, body, delay) in response_specs {
            response_queues
                .entry(path.to_owned())
                .or_default()
                .push_back((status, body, delay));
        }
        let mut requested_paths = Vec::new();
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted.expect("accept worker request");
                    let request = read_http_request_head(&mut stream)
                        .await
                        .expect("read worker request");
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .expect("request path");
                    requested_paths.push(path.to_owned());
                    let (status_line, body, delay) = response_queues
                        .get_mut(path)
                        .and_then(|queue| queue.pop_front())
                        .unwrap_or_else(|| panic!("unexpected worker test request path: {path}"));
                    if !delay.is_zero() {
                        sleep(delay).await;
                    }
                    let path_without_query = path.split_once('?').map_or(path, |(path, _)| path);
                    let content_type = if path_without_query.ends_with(".js")
                        || path_without_query.ends_with(".mjs")
                    {
                        "text/javascript"
                    } else {
                        "text/html"
                    };
                    let response = format!(
                        "{status_line}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write worker test response");
                }
            }
        }
        requested_paths
    });
    (format!("http://{addr}"), shutdown_tx, server)
}

async fn spawn_connection_drop_http_server(path: &'static str) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind connection-drop http server");
    let addr = listener.local_addr().expect("connection-drop server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept connection-drop request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read connection-drop request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("connection-drop request path");
        assert_eq!(request_path, path);
        drop(stream);
    });
    (format!("http://{addr}"), server)
}

async fn spawn_redirect_loop_http_server(path: &'static str) -> (String, JoinHandle<()>) {
    const REDIRECT_LOOP_REQUESTS: usize = 21;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect-loop http server");
    let addr = listener.local_addr().expect("redirect-loop server addr");
    let server = tokio::spawn(async move {
        for _ in 0..REDIRECT_LOOP_REQUESTS {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .expect("redirect loop should reach its limit without an earlier rejection")
                .expect("accept redirect-loop request");
            read_http_request_head(&mut stream)
                .await
                .expect("read redirect-loop request");
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {path}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write redirect-loop response");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_single_redirect_http_server(
    path: &'static str,
    location: impl Into<String>,
) -> (String, JoinHandle<()>) {
    let location = location.into();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind single-redirect http server");
    let addr = listener.local_addr().expect("single-redirect server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept single-redirect request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read single-redirect request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("single-redirect request path");
        assert_eq!(request_path, path);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write single-redirect response");
    });
    (format!("http://{addr}"), server)
}

async fn spawn_cross_origin_redirect_without_cors_http_servers(
    source_path: &'static str,
    target_path: &'static str,
) -> (String, String, JoinHandle<()>, JoinHandle<()>) {
    let target_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect target without CORS server");
    let target_addr = target_listener
        .local_addr()
        .expect("redirect target without CORS addr");
    let target_base_url = format!("http://{target_addr}");
    let target_server = tokio::spawn(async move {
        let (mut stream, _) = target_listener
            .accept()
            .await
            .expect("accept redirect target request");
        read_http_request_head(&mut stream)
            .await
            .expect("read redirect target request");
        let body = "cors-denied-target";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write redirect target without CORS response");
    });

    let source_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect source server");
    let source_addr = source_listener.local_addr().expect("redirect source addr");
    let source_base_url = format!("http://{source_addr}");
    let target_location = format!("{target_base_url}{target_path}");
    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source_listener
            .accept()
            .await
            .expect("accept redirect source request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read redirect source request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("redirect source request path");
        assert_eq!(path, source_path);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target_location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write redirect source response");
    });

    (
        source_base_url,
        target_base_url,
        source_server,
        target_server,
    )
}

async fn spawn_cross_origin_redirect_with_cors_http_servers(
    source_path: &'static str,
    target_path: &'static str,
    target_body: &'static str,
) -> (String, String, JoinHandle<()>, JoinHandle<()>) {
    let target_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect target with CORS server");
    let target_addr = target_listener
        .local_addr()
        .expect("redirect target with CORS addr");
    let target_base_url = format!("http://{target_addr}");
    let target_server = tokio::spawn(async move {
        let (mut stream, _) = target_listener
            .accept()
            .await
            .expect("accept redirect target request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read redirect target request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("redirect target request path");
        assert_eq!(path, target_path);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            target_body.len(),
            target_body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write redirect target with CORS response");
    });

    let source_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind CORS redirect source server");
    let source_addr = source_listener
        .local_addr()
        .expect("CORS redirect source addr");
    let source_base_url = format!("http://{source_addr}");
    let target_location = format!("{target_base_url}{target_path}");
    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source_listener
            .accept()
            .await
            .expect("accept CORS redirect source request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read CORS redirect source request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("CORS redirect source request path");
        assert_eq!(path, source_path);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target_location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write CORS redirect source response");
    });

    (
        source_base_url,
        target_base_url,
        source_server,
        target_server,
    )
}

async fn spawn_header_capture_http_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local header-capture http server");
    let addr = listener.local_addr().expect("server local addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept captured http request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read captured http request");
        let _ = request_tx.send(request);
        let body = "ok";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write captured http response");
    });
    (format!("http://{addr}"), request_rx, server)
}

async fn spawn_request_capture_http_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local request-capture http server");
    let addr = listener.local_addr().expect("server local addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept captured http request");
        let request = read_http_request_with_body(&mut stream)
            .await
            .expect("read captured http request");
        let _ = request_tx.send(request);
        let response = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write captured http response");
    });
    (format!("http://{addr}"), request_rx, server)
}

async fn spawn_disconnect_observing_http_server()
-> (String, tokio::sync::oneshot::Receiver<bool>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local disconnect-observing http server");
    let addr = listener.local_addr().expect("server local addr");
    let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept disconnect-observing http request");
        read_http_request_head(&mut stream)
            .await
            .expect("read disconnect-observing http request");
        stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nhello",
                )
                .await
                .expect("write disconnect-observing http response");
        sleep(Duration::from_millis(150)).await;
        let mut tail = [0u8; 1];
        let disconnected = matches!(
            tokio::time::timeout(Duration::from_secs(2), stream.read(&mut tail)).await,
            Ok(Ok(0))
        );
        let _ = disconnect_tx.send(disconnected);
    });
    (format!("http://{addr}"), disconnect_rx, server)
}

async fn spawn_request_seen_disconnect_observing_http_server() -> (
    String,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Receiver<bool>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local request-seen disconnect-observing http server");
    let addr = listener.local_addr().expect("server local addr");
    let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
    let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept request-seen disconnect-observing http request");
        read_http_request_head(&mut stream)
            .await
            .expect("read request-seen disconnect-observing http request");
        let _ = request_seen_tx.send(());
        let mut tail = [0u8; 1];
        let disconnected = matches!(
            tokio::time::timeout(Duration::from_secs(3), stream.read(&mut tail)).await,
            Ok(Ok(0))
        );
        let _ = disconnect_tx.send(disconnected);
    });
    (
        format!("http://{addr}"),
        request_seen_rx,
        disconnect_rx,
        server,
    )
}

async fn drive_page_work_until_done_with_explicit_producer_admission(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
    mut admit_external_producer_work: impl FnMut(&PageVm),
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        admit_external_producer_work(page_vm);
        while page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
            .is_some()
        {}
        let loader = page_vm.main_document_resource_loader();
        if page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
            .await?
        {
            continue;
        }
        if page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
            .is_some()
        {
            continue;
        }
        page_vm
            .advance_timers_until_deadline_for_test(loader.request_client())
            .await?;
        if page_vm.vm_mut().eval(done_expression)? == "true" {
            return Ok(());
        }
        page_vm
            .advance_timers_until_deadline_for_test(loader.request_client())
            .await?;
        if page_vm.vm_mut().eval(done_expression)? == "true" {
            return Ok(());
        }
        let arrived = tokio::time::timeout(
            Duration::from_secs(1),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await
        .unwrap_or(false);
        if !arrived {
            while page_vm
                .run_exact_page_websocket_selected_task_for_test()
                .await?
                .is_some()
            {}
            let loader = page_vm.main_document_resource_loader();
            page_vm
                .advance_timers_until_deadline_for_test(loader.request_client())
                .await?;
            if page_vm.vm_mut().eval(done_expression)? == "true" {
                return Ok(());
            }
        }
    }
    admit_external_producer_work(page_vm);
    while page_vm
        .run_exact_page_websocket_selected_task_for_test()
        .await?
        .is_some()
    {}
    let loader = page_vm.main_document_resource_loader();
    while page_vm
        .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
        .await?
    {}
    page_vm
        .advance_timers_until_deadline_for_test(loader.request_client())
        .await?;
    if page_vm.vm_mut().eval(done_expression)? != "true" {
        let events = page_vm
            .vm_mut()
            .eval("JSON.stringify(globalThis.__wsEvents ?? globalThis.__wsStreamEvents ?? null)")
            .unwrap_or_else(|error| {
                format!("<failed to read __wsEvents/__wsStreamEvents: {error}>")
            });
        let done = page_vm
            .vm_mut()
            .eval("String(globalThis.__wsDone ?? globalThis.__wsStreamDone)")
            .unwrap_or_else(|error| format!("<failed to read __wsDone/__wsStreamDone: {error}>"));
        let stream_events = page_vm
            .vm_mut()
            .eval("JSON.stringify(globalThis.__wsStreamEvents ?? null)")
            .unwrap_or_else(|error| format!("<failed to read __wsStreamEvents: {error}>"));
        let stream_done = page_vm
            .vm_mut()
            .eval("String(globalThis.__wsStreamDone ?? null)")
            .unwrap_or_else(|error| format!("<failed to read __wsStreamDone: {error}>"));
        panic!(
            "{context}; events={events}; done={done}; stream_events={stream_events}; stream_done={stream_done}"
        );
    }
    Ok(())
}

pub(super) async fn drive_websocket_until_done(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
) -> anyhow::Result<()> {
    drive_page_work_until_done_with_explicit_producer_admission(
        page_vm,
        done_expression,
        context,
        |_| {},
    )
    .await
}

/// Drive a direct PageVm SharedWorker workflow without borrowing WebSocket
/// execution as an implicit browser-context producer pump.
///
/// Production performs this admission in the render-owner service turn. This
/// fixture has no owner loop, so it names and performs that responsibility
/// before selecting already-resident Page tasks.
pub(super) async fn drive_shared_worker_until_done(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
) -> anyhow::Result<()> {
    drive_page_work_until_done_with_explicit_producer_admission(
        page_vm,
        done_expression,
        context,
        |page_vm| {
            page_vm
                .runtime_hooks
                .browser_context_runtime
                .drain_shared_worker_service_lane();
        },
    )
    .await
}

pub(super) async fn drain_page_work_until_no_pending_subresources(
    page_vm: &mut PageVm,
    context: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        while page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
            .is_some()
        {}
        let loader = page_vm.main_document_resource_loader();
        while page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
            .await?
        {}
        if page_vm.pending_subresource_request_count() == 0 {
            return Ok(());
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await;
    }
    while page_vm
        .run_exact_page_websocket_selected_task_for_test()
        .await?
        .is_some()
    {}
    let loader = page_vm.main_document_resource_loader();
    while page_vm
        .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
        .await?
    {}
    anyhow::bail!(
        "{context}; pending={}",
        page_vm.pending_subresource_request_count()
    )
}
mod extracted;

mod canvas_blob_serialization;

mod rtc_data_channel_init;
mod rtc_stats;
mod rtp_transceivers;
