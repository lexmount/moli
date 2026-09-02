use super::post_parse::dynamic_script_execute_is_runnable_before_dom_content_loaded;
use super::{
    PostParseDriverStep, PostParseLifecycleAdvance, PostParseLifecycleCompletionAction,
    PostParseLifecycleDriver, PostParseLifecycleRound, PostParsePageOwnedTask,
    PostParseProcessingAction, PostParseRuntimeDriverStep, PostParseStageBoundary,
    PostParseTaskCompletion, PostParseTaskExecutionToken, PostParseTaskInvalidationPolicy,
    ReadyPostParseAction, ScriptVm, ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness,
    renderer_document_isolate_critical_pressure_required, select_post_parse_driver_step,
};
use crate::document_runtime::{
    CurrentScriptContextSpec, DeferredPageTask, DeferredPageTaskLane, DeferredPageTaskState,
    DocumentProcessingAction, DomHandle, FollowupPageTaskDisposition, PostParseOwnerDriverStep,
    RuntimeScriptWorkPauseKind, RuntimeScriptWorkState,
};
use crate::dom::{
    NodeId,
    native::{DomHost, NativeDom, Node},
};
use crate::frame_owner_model::{ChildFrameSemanticTurnKind, FrameDocumentTaskOwner};
use crate::host::ScriptHandleSource;
use crate::network::ResourceRequestClient;
use crate::page_task_queue::{
    PageMainDocumentRuntimeActionKind, PageTask, PageTaskQueue, PostParseLifecycleWork,
    RendererPageMainDocumentRuntimeAction, RendererPageModulepreloadStartTestSource,
    RendererResourceCompletionSender, RendererResourceCompletionTestHarness,
};
use crate::page_task_queue::{PostParseLifecycleQueueStats, PostParsePageOwnedWork};
use crate::parser::HtmlParser;
use crate::planning::{
    ParserPlanningReadView, PrepareScriptOutcome, PreparedScript, build_prepared_script,
    classify_parser_script,
};
use crate::runtime::PageDomManipulationTestFamily;
use crate::types::{
    PendingSubresourceContinuation, PendingSubresourceFetchState, StreamingSubresourceFetchState,
};
use crate::types::{
    ScriptExecutionReport, ScriptKind, ScriptMode, ScriptRun, ScriptSkipReason, ScriptSourceKind,
};
use moli_parser::ScriptSource;
use std::ffi::c_void;
use std::sync::OnceLock;
use std::time::Instant;
use url::Url;

use self::http_fixture::{StaticHttpServer, static_http_loader};

const ZHIHU_CAPABILITY_PROBE_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/script_vm/fixtures/zhihu-capability-probe.html"
));
const ZHIHU_BOT_DETECTION_HARNESS_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/script_vm/fixtures/zhihu-bot-detection-harness.html"
));

/// Settle at most one realm-materialization prerequisite, then run exactly the
/// child-family turn named by the caller. The production one-turn helper stays
/// strict; tests using this setup helper explicitly acknowledge the extra turn.
async fn run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
    vm: &mut ScriptVm,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    message: &str,
) {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::RealmMaterialization,
        )
    {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "{message}: exact child realm prerequisite"
        );
    }
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(expected),
        "{message}"
    );
}

/// Page-harness counterpart of the standalone child setup helper.
///
/// Every consumed prerequisite and requested family runs through the
/// production selected-task dispatcher; this helper only acknowledges that
/// realm materialization may precede the requested child action.
async fn run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    message: &str,
) {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && page
            .run_one_child_frame_task_executor_turn(
                ChildFrameSemanticTurnKind::RealmMaterialization,
                loader,
            )
            .await
            .expect("child realm prerequisite should use the selected-task dispatcher")
    {
        // Realm materialization is the only setup task this helper may consume
        // before the exact family requested by the test.
    }
    assert!(
        page.run_one_child_frame_task_executor_turn(expected, loader)
            .await
            .expect("child semantic task should use the selected-task dispatcher"),
        "{message}"
    );
}

async fn run_page_service_worker_internal_task_for_test(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    message: &str,
) {
    assert!(
        page.run_one_service_worker_internal_task_executor_turn(loader)
            .await
            .unwrap_or_else(|error| panic!("{message}: {error}")),
        "{message}: expected one ServiceWorker internal task"
    );
}

#[derive(Clone, Copy)]
enum PendingServiceWorkerInternalRequestForTest {
    Ready(u64),
    Register(u64),
}

impl PendingServiceWorkerInternalRequestForTest {
    fn remains_pending(self, vm: &ScriptVm) -> bool {
        match self {
            Self::Ready(request_id) => vm
                ._context_host
                .borrow()
                .pending_service_worker_ready_owners_for_test()
                .into_iter()
                .any(|(pending_id, _)| pending_id == request_id),
            Self::Register(request_id) => vm
                ._context_host
                .borrow()
                .pending_service_worker_register_owners_for_test()
                .into_iter()
                .any(|(pending_id, _)| pending_id == request_id),
        }
    }
}

/// Preserve the production ServiceWorkerInternal FIFO while waiting for one
/// exact pending request to settle.
///
/// A ready/register workflow can leave an earlier lifecycle callback in the
/// same source. Tests must execute that predecessor rather than selecting a
/// later request by raw id or directly invoking its body.
async fn run_page_service_worker_internal_tasks_until_request_consumed_for_test(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    request: PendingServiceWorkerInternalRequestForTest,
    message: &str,
) {
    for _ in 0..32 {
        if !request.remains_pending(page) {
            return;
        }
        run_page_service_worker_internal_task_for_test(page, loader, message).await;
    }
    panic!("{message}: exact ServiceWorker request exceeded the bounded 32-turn FIFO budget");
}

async fn assert_initial_about_blank_child_completed_through_page_for_test(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    message: &str,
) {
    for family in [
        ChildFrameSemanticTurnKind::NavigationCommit,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        ChildFrameSemanticTurnKind::HostLoad,
    ] {
        assert!(
            !page
                .run_one_child_frame_task_executor_turn(family, loader)
                .await
                .unwrap_or_else(|error| panic!("{message}: {error}")),
            "{message}: synchronous initial about:blank must not leave {family:?} work"
        );
    }
}

async fn run_next_page_media_element_event_for_test(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    context: &str,
) {
    assert!(
        page.run_one_media_element_event_executor_turn(loader)
            .await
            .unwrap_or_else(|error| panic!("{context}: {error}")),
        "{context}: media-element event source was not ready"
    );
}

/// Drain the finite child semantic bootstrap/lifecycle chain through the
/// production Page selected-task dispatcher.
///
/// This is setup for tests whose subject begins after an iframe is live. It
/// does not consume DOM-manipulation, Networking, timer, or other Page task
/// families, so the subject task remains explicit in each test.
async fn drain_pending_page_child_frame_work_for_test(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
) {
    for _ in 0..128 {
        if page.run_next_child_frame_semantic_turn().await.is_none() {
            return;
        }
    }
    panic!("Page child semantic setup drain exceeded its finite turn budget");
}

fn register_pending_window_xhr_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut crate::native_bridge::JsContextHost,
    cancel_handle: moli_fetch::FetchCancelHandle,
) -> (
    u64,
    crate::native_bridge::WindowExecutionContextOwner,
    crate::native_bridge::RuntimeObservableContextToken,
) {
    let execution_context = host
        .current_runtime_window_execution_context_binding(scope)
        .expect("test XHR should capture a Window execution context");
    let owner = execution_context.owner();
    let realm_token = execution_context.realm_token();
    let url = Url::parse("https://xhr-execution-context.test/pending").unwrap();
    let internal_id = host.record_async_subresource_xhr(
        execution_context,
        v8::Global::new(scope, v8::Object::new(scope)),
        Some(cancel_handle),
        moli_fetch::RequestCredentialsMode::SameOrigin,
        None,
        Default::default(),
        crate::types::PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: None,
            document_url: url.clone(),
            url,
            websocket_socket_id: None,
            method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            request_body_bytes: None,
            resource_type: crate::types::SubresourceResourceType::Xhr,
            request_cookie_report: None,
        },
    );
    (internal_id, owner, realm_token)
}

#[derive(Clone, Copy)]
enum PendingWindowFetchTestStage {
    Pending,
    Running,
    Streaming,
    Auth,
    Response,
    ServiceWorkerInFlight,
}

fn register_pending_window_fetch_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut crate::native_bridge::JsContextHost,
    keepalive: bool,
    stage: PendingWindowFetchTestStage,
) -> (
    u64,
    crate::native_bridge::WindowExecutionContextOwner,
    crate::native_bridge::RuntimeObservableContextToken,
    moli_fetch::FetchCancelHandle,
) {
    let execution_context = host
        .current_runtime_window_execution_context_binding(scope)
        .expect("test Fetch should capture an exact Window execution context");
    let owner = execution_context.owner();
    let realm_token = execution_context.realm_token();
    let dispatch_scope = execution_context.dispatch_scope();
    let fetch_context = crate::native_bridge::WindowFetchContext::from_realm(execution_context);
    let connect_policy = host
        .document_connect_policy_snapshot_for_owner(dispatch_scope)
        .expect("test Fetch should snapshot its construction document policy");
    let csp_report_context =
        crate::network_host::capture_window_csp_report_request_context(scope, host, dispatch_scope)
            .expect("test Fetch should capture its construction document report context");
    let resolver = v8::PromiseResolver::new(scope).expect("test Fetch resolver");
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let url = Url::parse("https://fetch-execution-context.test/pending").unwrap();
    let internal_id = host.record_async_subresource_fetch(
        fetch_context,
        v8::Global::new(scope, resolver),
        keepalive,
        connect_policy,
        csp_report_context,
        Some(cancel_handle.clone()),
        moli_fetch::RequestCredentialsMode::SameOrigin,
        moli_fetch::RequestMode::Cors,
        None,
        Default::default(),
        crate::types::PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: None,
            document_url: url.clone(),
            url: url.clone(),
            websocket_socket_id: None,
            method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            request_body_bytes: None,
            resource_type: crate::types::SubresourceResourceType::Fetch,
            request_cookie_report: None,
        },
        false,
    );

    if !matches!(stage, PendingWindowFetchTestStage::Pending) {
        let pending = host
            .take_pending_subresource_fetch(internal_id)
            .expect("test Fetch should move from pending into its requested stage");
        match stage {
            PendingWindowFetchTestStage::Pending => unreachable!(),
            PendingWindowFetchTestStage::Running => {
                pending.load.attach_cancel_handle(cancel_handle.clone());
                host.record_running_subresource_fetch(crate::types::RunningSubresourceFetchState {
                    pending,
                    request_url: url.clone(),
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    intercept_response: false,
                    handle_auth_requests: false,
                    initial_auth_network_request_headers: None,
                });
            }
            PendingWindowFetchTestStage::Streaming => {
                host.record_streaming_subresource_fetch(
                    crate::types::StreamingSubresourceFetchState {
                        pending,
                        request_url: url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        body_source_id: 10_000 + internal_id,
                        head: moli_fetch::ResponseHead {
                            final_url: url.clone(),
                            status: 200,
                            headers: Vec::new(),
                            request_cookie_report: None,
                            cookie_set_reports: Vec::new(),
                            redirected: false,
                            redirect_chain: Vec::new(),
                            from_cache: false,
                            negotiated_http_version: None,
                        },
                        network_request_headers: None,
                        body_writer: Default::default(),
                        event_source_parser: None,
                        xhr_response: None,
                    },
                );
            }
            PendingWindowFetchTestStage::Auth => {
                host.record_pending_subresource_auth(crate::types::PendingSubresourceAuthState {
                    pending,
                    request_url: url.clone(),
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    intercept_response: false,
                    initial_network_request_headers: None,
                    response: crate::types::NavigationResponse::from_text_body(
                        url.clone(),
                        401,
                        Vec::new(),
                        "auth required".to_owned(),
                    ),
                });
            }
            PendingWindowFetchTestStage::Response => {
                host.record_pending_subresource_response(
                    crate::types::PendingSubresourceResponseState {
                        pending,
                        request_url: url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        response: crate::types::NavigationResponse::from_text_body(
                            url.clone(),
                            200,
                            Vec::new(),
                            "pending response".to_owned(),
                        ),
                    },
                );
            }
            PendingWindowFetchTestStage::ServiceWorkerInFlight => {
                host.record_in_flight_worker_subresource_fetch(
                    crate::types::InFlightWorkerSubresourceFetchState {
                        pending,
                        request_url: url,
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                    },
                );
            }
        }
    }

    (internal_id, owner, realm_token, cancel_handle)
}

fn register_pending_window_fetch_with_connect_policy_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut crate::native_bridge::JsContextHost,
    keepalive: bool,
    policy: crate::document_runtime::DocumentPolicyContainer,
    request_url: Url,
) -> (
    u64,
    crate::native_bridge::WindowExecutionContextOwner,
    crate::native_bridge::RuntimeObservableContextToken,
    moli_fetch::FetchCancelHandle,
    crate::native_bridge::WindowDocumentNetworkRequestIdentity,
) {
    let execution_context = host
        .current_runtime_window_execution_context_binding(scope)
        .expect("test Fetch should capture an exact Window execution context");
    let owner = execution_context.owner();
    let realm_token = execution_context.realm_token();
    let dispatch_scope = execution_context.dispatch_scope();
    let fetch_context = crate::native_bridge::WindowFetchContext::from_realm(execution_context);
    let csp_report_context =
        crate::network_host::capture_window_csp_report_request_context(scope, host, dispatch_scope)
            .expect("test Fetch should capture its construction document report context");
    let report_identity = csp_report_context.identity();
    let resolver = v8::PromiseResolver::new(scope).expect("test Fetch resolver");
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let document_url = match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Top => host.document_url().clone(),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => host
            .child_browsing_context_current_url(handle)
            .expect("test child Fetch document URL"),
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(_) => {
            panic!("this focused Fetch policy helper only supports frame Documents")
        }
    };
    let internal_id = host.record_async_subresource_fetch(
        fetch_context,
        v8::Global::new(scope, resolver),
        keepalive,
        crate::document_runtime::DocumentConnectPolicySnapshot::from_policy_container(&policy),
        csp_report_context,
        Some(cancel_handle.clone()),
        moli_fetch::RequestCredentialsMode::SameOrigin,
        moli_fetch::RequestMode::Cors,
        None,
        Default::default(),
        crate::types::PendingSubresourceFetchInfo {
            internal_id: 0,
            network_request_handle: None,
            frame_id: None,
            document_url,
            url: request_url.clone(),
            websocket_socket_id: None,
            method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            request_body_bytes: None,
            resource_type: crate::types::SubresourceResourceType::Fetch,
            request_cookie_report: None,
        },
        false,
    );
    (
        internal_id,
        owner,
        realm_token,
        cancel_handle,
        report_identity,
    )
}

fn redirected_fetch_response(source_url: &Url, final_url: Url) -> crate::types::NavigationResponse {
    let mut response = crate::types::NavigationResponse::from_text_body(
        final_url.clone(),
        200,
        vec![("content-type".to_owned(), "text/plain".to_owned())],
        "redirected Fetch completed".to_owned(),
    );
    response.redirected = true;
    response.redirect_chain = vec![crate::types::NavigationRedirect {
        from_url: source_url.clone(),
        to_url: final_url,
        status: 302,
        headers: Vec::new(),
        network_extra_info_available: true,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: true,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    }];
    response
}

#[derive(Debug, crate::webidl::WebIdlDictionary)]
#[webidl(prefix = "NullableRequiredDictionaryProbe")]
struct NullableRequiredDictionaryProbe {
    #[webidl(required, nullable)]
    value: Option<String>,
}

fn new_storage_test_vm(url: &str) -> StandaloneScriptVmHarness {
    let resource_completion_queue = RendererResourceCompletionTestHarness::new();
    new_storage_test_vm_with_completion_sender(url, resource_completion_queue.sender())
}

fn refresh_layout_for_test(vm: &mut StandaloneScriptVmHarness) {
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("test layout refresh should succeed"),
        "test layout refresh requires a connected document element"
    );
}

fn new_storage_test_vm_without_page_residence(url: &str) -> StandaloneScriptVmHarness {
    let resource_completion_queue = RendererResourceCompletionTestHarness::new();
    new_storage_test_vm_with_resource_mode_and_residence(
        url,
        resource_completion_queue.sender(),
        StandaloneStorageResourceMode::PrivateRuntime,
        false,
    )
}

fn new_broadcast_channel_page_test_vm(url: &str) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    new_broadcast_channel_page_test_vm_with_loader(url, &loader)
}

fn new_child_modulepreload_page_test_vm(
    url: &str,
) -> (
    crate::runtime::PageVmTaskExecutorTestHarness,
    RendererPageModulepreloadStartTestSource,
) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let page = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse(url).expect("child modulepreload test URL"),
        &loader,
    );
    let source = page.modulepreload_start_test_source();
    (page, source)
}

fn new_broadcast_channel_page_test_vm_with_loader(
    url: &str,
    loader: &ResourceRequestClient,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    new_page_task_executor_test_vm_with_loader(url, loader)
}

fn new_page_task_executor_test_vm_with_loader(
    url: &str,
    loader: &ResourceRequestClient,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let page = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse(url).expect("Page task-executor test URL"),
        loader,
    );
    configure_page_task_executor_test_vm(page)
}

fn new_parsed_page_task_executor_test_vm(
    url: &str,
    markup: &str,
    loader: &ResourceRequestClient,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let document =
        HtmlParser::SCRIPTING_ENABLED.parse(Url::parse(url).expect("test URL"), markup.to_owned());
    let page = crate::runtime::PageVmTaskExecutorTestHarness::new_with_dom_host(
        DomHost::from_dom(document),
        loader,
    );
    configure_page_task_executor_test_vm(page)
}

fn new_streamed_parser_page_task_executor_test_vm(
    url: &str,
    markup: &str,
    loader: &ResourceRequestClient,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let mut stream =
        HtmlParser::SCRIPTING_ENABLED.start_document(Url::parse(url).expect("test URL"));
    stream.feed(markup);
    let page = crate::runtime::PageVmTaskExecutorTestHarness::new_with_dom_host(
        stream.take_parser_stream_dom_host(),
        loader,
    );
    configure_page_task_executor_test_vm(page)
}

fn configure_page_task_executor_test_vm(
    mut page: crate::runtime::PageVmTaskExecutorTestHarness,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    page.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(
        &shared_indexed_db_test_manager(),
    )));
    install_test_trusted_key_dispatcher(&mut page);
    page
}

/// Build a storage-capable Page fixture whose asynchronous results are
/// consumed only by the production selected-task dispatcher.
///
/// ScriptVm-only storage tests remain useful for synchronous WebIDL and domain
/// bodies. Workflows that need Promise reactions from more than one OPFS or
/// IndexedDB task must use this fixture instead of recreating Page task-end
/// completion in a low-level test driver.
fn new_storage_page_task_executor_test_vm(
    url: &str,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    new_storage_page_task_executor_test_vm_with_loader(url, &loader)
}

fn new_storage_page_task_executor_test_vm_with_loader(
    url: &str,
    loader: &ResourceRequestClient,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let storage_manager = shared_indexed_db_test_manager();
    let mut page = crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse(url).expect("storage Page task-executor test URL"),
        loader,
    );
    page.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&storage_manager)));
    page.set_storage_bucket_store(
        crate::new_shared_storage_bucket_store_with_indexed_db_manager(&storage_manager),
    );
    install_test_trusted_key_dispatcher(&mut page);
    page
}

fn shared_indexed_db_test_manager() -> crate::SharedIndexedDbManager {
    static STORAGE_MANAGER: OnceLock<crate::SharedIndexedDbManager> = OnceLock::new();
    STORAGE_MANAGER
        .get_or_init(|| {
            crate::new_indexed_db_manager(None)
                .expect("in-memory indexedDB test manager should initialize")
        })
        .clone()
}

/// Constructs the common Window test surface with a production-shaped main
/// Document authority and a caller-selected completion route.
fn new_storage_test_vm_with_completion_sender(
    url: &str,
    resource_completion_tx: RendererResourceCompletionSender,
) -> StandaloneScriptVmHarness {
    new_storage_test_vm_with_resource_mode(
        url,
        resource_completion_tx,
        StandaloneStorageResourceMode::PrivateRuntime,
    )
}

enum StandaloneStorageResourceMode<'a> {
    PrivateRuntime,
    Networked(&'a ResourceRequestClient),
}

fn new_storage_test_vm_with_resource_mode(
    url: &str,
    resource_completion_tx: RendererResourceCompletionSender,
    resource_mode: StandaloneStorageResourceMode<'_>,
) -> StandaloneScriptVmHarness {
    new_storage_test_vm_with_resource_mode_and_residence(
        url,
        resource_completion_tx,
        resource_mode,
        true,
    )
}

fn new_storage_test_vm_with_resource_mode_and_residence(
    url: &str,
    resource_completion_tx: RendererResourceCompletionSender,
    resource_mode: StandaloneStorageResourceMode<'_>,
    install_page_residence: bool,
) -> StandaloneScriptVmHarness {
    let storage_manager = shared_indexed_db_test_manager();
    let _js_runtime = crate::JsRuntime::initialize();
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
    let page_runtime_task_source = page_task_queue.residence();
    let dom_host = DomHost::from_dom(NativeDom::new(url::Url::parse(url).expect("test url")));
    let bootstrap = match resource_mode {
        StandaloneStorageResourceMode::PrivateRuntime => {
            ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_with_resource_completion_sender_for_test(
                dom_host,
                post_domcontentloaded_page_task_sender,
                page_task_front_injection_tx,
                resource_completion_tx,
            )
        }
        StandaloneStorageResourceMode::Networked(loader) => {
            ScriptVmDefaultWorldBootstrap::standalone_networked_from_dom_host_with_resource_completion_sender_for_test(
                dom_host,
                post_domcontentloaded_page_task_sender,
                page_task_front_injection_tx,
                resource_completion_tx,
                loader.clone(),
            )
        }
    };
    bootstrap
        .expect("script vm bootstrap should succeed")
        .finish()
        .map(|mut vm| {
            if install_page_residence {
                static NEXT_STANDALONE_PAGE_ID: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(1_000_000);
                let page_id = crate::PageId::new_for_testing(
                    NEXT_STANDALONE_PAGE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                );
                vm.set_root_document_lifecycle(
                    crate::runtime::RendererDocumentLifecycleJournalHandle::new_initial(page_id),
                );
            }
            vm.install_page_task_residence_for_executor_test(page_runtime_task_source);
            vm.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&storage_manager)));
            vm.set_storage_bucket_store(
                crate::new_shared_storage_bucket_store_with_indexed_db_manager(&storage_manager),
            );
            install_test_trusted_key_dispatcher(&mut vm);
            vm
        })
        .expect("script vm finish should succeed")
}

fn test_window_csp_report_violation(
    document_url: &Url,
    report_url: &Url,
) -> crate::document_runtime::DocumentContentSecurityPolicyViolation {
    crate::content_security_policy::ContentSecurityPolicyUrlViolation {
        effective_directive: "connect-src",
        blocked_uri: "https://blocked-csp-report.test/resource".to_owned(),
        document_uri: document_url.as_str().to_owned(),
        original_policy: format!("connect-src 'none'; report-uri {report_url}"),
        disposition: crate::content_security_policy::ContentSecurityPolicyDisposition::Enforce,
        report_uri_endpoints: vec![report_url.as_str().to_owned()],
        report_to_endpoints: Vec::new(),
        sample: String::new(),
        source_file: String::new(),
        line_number: 0,
        column_number: 0,
    }
}

#[test]
fn main_document_owner_transition_retires_script_vm_local_state_once() {
    let mut vm = new_storage_test_vm("https://main-owner-transition.test/");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    vm.pressed_mouse_buttons = 1;

    vm.eval("document.open(); 'replaced'")
        .expect("document.open should commit the replacement owner transaction");

    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_ne!(current_owner, retired_owner);
    assert_eq!(
        vm.pressed_mouse_buttons, 0,
        "runtime turn exit must retire ScriptVm-local input state from the owner transition"
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .take_pending_main_document_owner_transitions()
            .is_empty(),
        "runtime turn exit must claim the replacement transaction exactly once"
    );

    vm.pressed_mouse_buttons = 1;
    vm.refresh_script_vm_local_document_state();
    assert_eq!(
        vm.pressed_mouse_buttons, 1,
        "refresh without a new owner transition must not replay document retirement"
    );
}

#[test]
fn main_document_open_preserves_already_recorded_runtime_binding_calls() {
    let mut vm = new_storage_test_vm("https://main-runtime-binding-owner.test/");
    vm.install_runtime_binding("ownerBoundRuntimeBinding", None, None)
        .expect("main Runtime binding should install");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("initial main Runtime binding owner");

    vm.eval(r#"ownerBoundRuntimeBinding("retired-document")"#)
        .expect("initial Runtime binding call");
    vm.page_diagnostics_snapshot()
        .expect("the initial call should enter the Page activity residence");

    vm.eval(
        r#"
        document.open();
        ownerBoundRuntimeBinding("replacement-document");
        document.close();
        "done"
        "#,
    )
    .expect("same-realm document.open Runtime binding sequence should evaluate");

    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main Runtime binding owner");
    assert_ne!(retired_owner.document_id, current_owner.document_id);
    assert_eq!(retired_owner.local_window_id, current_owner.local_window_id);
    let calls = vm.take_runtime_binding_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls
            .iter()
            .map(|call| call.payload.as_str())
            .collect::<Vec<_>>(),
        ["retired-document", "replacement-document"],
        "document.open() must not erase a binding invocation which was already accepted and published as Page activity"
    );
    assert_eq!(calls[0].source, calls[1].source);
}

#[test]
fn runtime_binding_calls_freeze_the_invoking_realm_generation() {
    let mut vm = new_storage_test_vm("https://runtime-binding-source-realm.test/");
    vm.install_runtime_binding("mainRealmBinding", None, None)
        .expect("main Runtime binding should install");
    let isolated_context_id = vm
        .create_isolated_world("binding-source-realm", false)
        .expect("isolated Runtime binding world");
    vm.install_runtime_binding("isolatedRealmBinding", None, Some(isolated_context_id))
        .expect("isolated Runtime binding should install");

    vm.eval(r#"mainRealmBinding("main")"#)
        .expect("main binding call");
    vm.exec_in_execution_context(isolated_context_id, r#"isolatedRealmBinding("isolated")"#)
        .expect("isolated binding call");

    let calls = vm.take_runtime_binding_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].source.local_window_id(),
        calls[1].source.local_window_id(),
        "main and isolated worlds belong to the same Window"
    );
    assert_ne!(
        calls[0].source.realm_generation(),
        calls[1].source.realm_generation(),
        "binding calls must retain the exact invoking realm instead of relying on a reusable public execution-context id"
    );
    assert_ne!(calls[0].execution_context_id, calls[1].execution_context_id);
}

#[test]
fn main_document_open_rebinds_preserved_isolated_runtime_binding_context() {
    let mut vm = new_storage_test_vm("https://isolated-runtime-binding-owner.test/");
    let isolated_context_id = vm
        .create_isolated_world("runtime-binding-owner", false)
        .expect("main isolated Runtime binding world should be created");
    vm.install_runtime_binding(
        "isolatedOwnerBoundRuntimeBinding",
        None,
        Some(isolated_context_id),
    )
    .expect("isolated Runtime binding should install");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("initial isolated Runtime binding document owner");

    vm.exec_in_execution_context(
        isolated_context_id,
        r#"
        isolatedOwnerBoundRuntimeBinding("retired-document");
        document.open();
        isolatedOwnerBoundRuntimeBinding("replacement-document");
        document.close();
        "#,
    )
    .expect("preserved isolated Runtime binding context should execute across document.open");

    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement isolated Runtime binding document owner");
    assert_ne!(retired_owner.document_id, current_owner.document_id);
    assert_eq!(retired_owner.local_window_id, current_owner.local_window_id);
    let calls = vm.take_runtime_binding_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls
            .iter()
            .map(|call| call.payload.as_str())
            .collect::<Vec<_>>(),
        ["retired-document", "replacement-document"]
    );
    assert!(
        calls
            .iter()
            .all(|call| call.execution_context_id == isolated_context_id)
    );
    assert_eq!(calls[0].source, calls[1].source);
}

#[tokio::test]
async fn child_navigation_retires_runtime_binding_context_and_stale_function() {
    let mut vm = new_storage_test_vm("https://child-runtime-binding-owner.test/");
    vm.set_stored_runtime_bindings(&[crate::protocol_types::RuntimeBindingRegistration {
        devtools_session: None,
        name: "childOwnerBoundRuntimeBinding".to_owned(),
        execution_context_name: None,
    }]);
    vm.eval(
        r#"
        (() => {
          const root = document.documentElement ||
            document.appendChild(document.createElement("html"));
          const body = document.body || root.appendChild(document.createElement("body"));
          const frame = document.createElement("iframe");
          globalThis.__runtimeBindingOwnerFrame = frame;
          body.appendChild(frame);
          void frame.contentWindow;
        })()
        "#,
    )
    .expect("child Runtime binding frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child Runtime binding initial-empty setup",
    )
    .await;
    let initial_owner = current_single_child_document_owner_for_test(
        &vm,
        "child Runtime binding initial-empty document",
    );
    vm.eval("__runtimeBindingOwnerFrame.srcdoc = '<p>committed</p>'; 'queued'")
        .expect("first child Runtime binding document should queue");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "first child Runtime binding document",
    )
    .await;
    let committed_owner = current_single_child_document_owner_for_test(
        &vm,
        "committed child Runtime binding document",
    );
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "the first secure commit must reuse the initial-empty LocalWindow"
    );
    assert_ne!(committed_owner.document_id, initial_owner.document_id);
    let initial_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("initial child Runtime binding context should exist");
    vm.eval_in_child_default_context(
        initial_context_id,
        r#"childOwnerBoundRuntimeBinding("retired-inner-realm")"#,
    )
    .expect("old child realm Runtime binding call should queue");

    vm.eval(
        r#"
        globalThis.__retiredChildRuntimeBinding =
          __runtimeBindingOwnerFrame.contentWindow.childOwnerBoundRuntimeBinding;
        __retiredChildRuntimeBinding("retired-document");
        __runtimeBindingOwnerFrame.srcdoc = "<p>replacement</p>";
        "queued"
        "#,
    )
    .expect("old child Runtime binding call and replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    let retired_calls = vm.take_runtime_binding_calls();
    assert_eq!(
        retired_calls
            .iter()
            .map(|call| call.payload.as_str())
            .collect::<Vec<_>>(),
        ["retired-inner-realm", "retired-document"],
        "calls accepted before child navigation are historical observations and must survive realm retirement"
    );
    assert!(
        retired_calls
            .iter()
            .all(|call| call.execution_context_id == initial_context_id)
    );

    vm.eval(
        r#"
        __retiredChildRuntimeBinding("stale-function");
        void __runtimeBindingOwnerFrame.contentWindow;
        "requested-replacement-realm"
        "#,
    )
    .expect("stale binding call should fail closed while requesting the replacement realm");
    assert!(
        vm.take_runtime_binding_calls().is_empty(),
        "a function captured from the retired child realm must not target the replacement owner"
    );
    let replacement_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "replacement child Runtime binding context",
    );
    assert_ne!(replacement_context_id, initial_context_id);

    vm.eval(
        r#"
        __runtimeBindingOwnerFrame.contentWindow.childOwnerBoundRuntimeBinding(
          "replacement-document"
        );
        "called"
        "#,
    )
    .expect("replacement child Runtime binding should remain callable");
    let calls = vm.take_runtime_binding_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "childOwnerBoundRuntimeBinding");
    assert_eq!(calls[0].payload, "replacement-document");
    assert_eq!(calls[0].execution_context_id, replacement_context_id);
}

#[tokio::test]
async fn opaque_child_isolated_world_projects_only_its_own_document() {
    let mut vm = new_storage_test_vm("https://opaque-child-isolated-world.test/");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "opaque-isolated-frame";
  frame.name = "opaque-isolated-child";
  frame.sandbox = "allow-scripts";
  frame.srcdoc = "<p id='opaque-marker'>opaque child document</p>";
  body.appendChild(frame);
  void frame.contentWindow;
})()
"#,
    )
    .expect("opaque child isolated-world setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "opaque child isolated-world setup",
    )
    .await;

    let child_realm = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("opaque child default realm should exist");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_realm.context_id)
        .expect("opaque child realm record should exist")
        .child_handle;
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_has_opaque_origin(child_handle),
        "sandbox without allow-same-origin must create an opaque child origin"
    );
    assert_eq!(
        vm.eval("document.getElementById('opaque-isolated-frame').contentDocument === null")
            .expect("top opaque contentDocument visibility should evaluate"),
        "true",
        "top must not gain DOM access to the opaque child"
    );

    let frame_id = vm
        ._context_host
        .borrow()
        .frame_owner_frame_id_for_child_handle(child_handle)
        .expect("opaque child frame id should exist")
        .0;
    let isolated_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "opaque-child-utility", false)
        .expect("opaque child isolated world should be created");
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            "document.getElementById('opaque-marker').textContent",
        )
        .expect("opaque child isolated world should access its own document"),
        "opaque child document"
    );
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            r#"
(() => {
  class IsolatedChildElement extends HTMLElement {}
  customElements.define("isolated-child-element", IsolatedChildElement);
  const element = document.createElement("isolated-child-element");
  globalThis.__opaqueChildOpfsResult = "pending";
  navigator.storage.getDirectory().then(
    () => { globalThis.__opaqueChildOpfsResult = "resolved"; },
    error => { globalThis.__opaqueChildOpfsResult = error.name; }
  );
  return JSON.stringify({
    parentIsSelf: parent === self,
    topIsParent: top === parent,
    name,
    origin,
    navigationName: performance.getEntriesByType("navigation")[0].name,
    customElementUsesIsolatedDefinition:
      Object.getPrototypeOf(element) === IsolatedChildElement.prototype,
    webAssemblyConstructorUsesIsolatedFunctionPrototype:
      Object.getPrototypeOf(WebAssembly.Module) === Function.prototype
  });
})()
"#,
        )
        .expect("opaque child isolated-world state should evaluate"),
        r#"{"parentIsSelf":false,"topIsParent":true,"name":"opaque-isolated-child","origin":"null","navigationName":"about:srcdoc","customElementUsesIsolatedDefinition":true,"webAssemblyConstructorUsesIsolatedFunctionPrototype":true}"#
    );
    assert_eq!(
        vm.eval_in_isolated_context(isolated_context_id, "__opaqueChildOpfsResult")
            .expect("opaque child isolated-world OPFS result should evaluate"),
        "SecurityError",
        "isolated child navigator.storage must use the child opaque storage owner"
    );
}

#[tokio::test]
async fn initial_empty_child_isolated_world_rebinds_committed_document() {
    let mut vm = new_storage_test_vm("https://initial-empty-isolated-world.test/");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "initial-empty-isolated-frame";
  body.appendChild(frame);
  void frame.contentWindow;
})()
"#,
    )
    .expect("initial-empty child isolated-world setup should evaluate");

    assert!(
        vm.run_child_realm_materialization_body_for_test()
            .expect("initial-empty child realm turn should succeed"),
        "Window exposure should enqueue the initial-empty child realm"
    );

    let child_realm = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("initial-empty child default realm should exist");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_realm.context_id)
        .expect("initial-empty child realm record should exist")
        .child_handle;
    let frame_id = vm
        ._context_host
        .borrow()
        .frame_owner_frame_id_for_child_handle(child_handle)
        .expect("initial-empty child frame id should exist")
        .0;
    let isolated_context_id = vm
        .create_isolated_world_for_frame(&frame_id, "initial-empty-utility", false)
        .expect("initial-empty child isolated world should be created");
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            "globalThis.__initialEmptyUtilityExpando = 'preserved'",
        )
        .expect("initial-empty isolated document should evaluate"),
        "preserved"
    );

    vm.eval(
        r#"
document.getElementById("initial-empty-isolated-frame").srcdoc =
  "<!doctype html><body><p id='committed-marker'>committed child document</p></body>";
"navigating"
"#,
    )
    .expect("initial-empty child srcdoc navigation should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "initial-empty child isolated-world commit",
    )
    .await;

    assert_eq!(
        vm.eval_in_isolated_context(
            isolated_context_id,
            "__initialEmptyUtilityExpando + '|' + document.getElementById('committed-marker').textContent",
        )
        .expect("rebound child isolated world should project the committed document"),
        "preserved|committed child document",
        "secure initial-empty reuse must preserve the isolated context while rotating its Document owner"
    );
}

#[tokio::test]
async fn inherited_opaque_srcdoc_reuses_initial_empty_child_local_window() {
    let mut vm = new_storage_test_vm("data:text/html,opaque-parent");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "inherited-opaque-frame";
  body.appendChild(frame);
  void frame.contentWindow;
})()
"#,
    )
    .expect("inherited opaque initial child should evaluate");
    let initial_owner =
        current_single_child_document_owner_for_test(&vm, "inherited opaque initial child owner");

    vm.eval(
        r#"
document.getElementById("inherited-opaque-frame").srcdoc =
  "<!doctype html><body><p id='opaque-marker'>committed opaque child</p></body>";
"navigating"
"#,
    )
    .expect("inherited opaque srcdoc navigation should evaluate");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "inherited opaque srcdoc commit")
        .await;

    let committed_owner =
        current_single_child_document_owner_for_test(&vm, "inherited opaque committed child owner");
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "an inherited opaque origin keeps its nonce identity and securely reuses the initial LocalWindow"
    );
    assert_ne!(
        committed_owner.document_id, initial_owner.document_id,
        "secure LocalWindow reuse must still install a new Document owner"
    );
    assert_eq!(
        vm.eval(
            "document.getElementById('inherited-opaque-frame').contentWindow.document.getElementById('opaque-marker').textContent"
        )
        .expect("opaque creator should access its inherited child origin"),
        "committed opaque child"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn main_document_open_preserves_local_window_owned_webcrypto_task_body_authority() {
    let mut vm = new_storage_test_vm("https://main-owner-webcrypto.test/");
    let before_document_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    let execution_context_owner = crate::native_bridge::WindowExecutionContextOwner::Frame(
        before_document_owner.local_window_id,
    );
    let completion = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let resolver =
                v8::PromiseResolver::new(scope).expect("WebCrypto test resolver should exist");
            let promise = resolver.get_promise(scope);
            let context = scope.get_current_context();
            assert_eq!(
                context.global(scope).set(
                    scope,
                    crate::util::v8str(scope, "__localWindowWebCryptoPromise").into(),
                    promise.into(),
                ),
                Some(true)
            );
            let completion = unsafe { &mut *host_ptr }
                .register_pending_webcrypto_task(scope, resolver)
                .expect("WebCrypto task should capture the current execution context");
            Ok(completion)
        })
        .expect("WebCrypto test task should register");
    vm.eval(
        r#"
        globalThis.__localWindowWebCryptoResult = "pending";
        globalThis.__localWindowWebCryptoPromise.then(value => {
          globalThis.__localWindowWebCryptoResult = String(value);
        });
        "attached"
        "#,
    )
    .expect("WebCrypto test reaction should attach");
    let pending_before_open = vm
        ._context_host
        .borrow()
        .pending_webcrypto_execution_contexts_for_test();
    assert_eq!(pending_before_open.len(), 1);
    assert_eq!(pending_before_open[0].0, execution_context_owner);

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");

    let after_document_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(
        after_document_owner.local_window_id, before_document_owner.local_window_id,
        "document.open must preserve the Window execution context"
    );
    assert_ne!(
        after_document_owner.document_id, before_document_owner.document_id,
        "document.open must still rotate the Document owner"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_webcrypto_execution_contexts_for_test(),
        pending_before_open,
        "document.open must neither retire nor rebind LocalWindow-owned WebCrypto work"
    );

    completion
        .send(Ok(crate::context_bootstrap::WebCryptoTaskResult::Bool(
            true,
        )))
        .expect("preserved WebCrypto completion should enter its production source");
    assert!(
        vm.run_webcrypto_task_body_for_authorization_test()
            .expect("preserved WebCrypto task body should settle")
    );
    assert_eq!(
        vm.eval("globalThis.__localWindowWebCryptoResult")
            .expect("WebCrypto reaction state should remain readable after document.open"),
        "pending",
        "the low-level body authority must leave Promise reactions to the selected-task checkpoint"
    );
    assert_eq!(vm._context_host.borrow().pending_webcrypto_task_count(), 0);
}

#[test]
fn opfs_completion_rechecks_named_bucket_liveness_before_settlement() {
    let origin = "https://stale-opfs-completion.test/";
    let mut vm = new_storage_test_vm(origin);
    let storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(
        &url::Url::parse(origin).unwrap(),
        None,
    )
    .serialized_storage_key();
    let locator = {
        let mut store = vm.storage_bucket_store.lock();
        store.open_bucket(&storage_key, "stale-read").unwrap();
        store
            .bucket_locator(&storage_key, "stale-read")
            .expect("named bucket locator")
    };
    let completion = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let resolver = v8::PromiseResolver::new(scope).expect("OPFS resolver");
            let promise = resolver.get_promise(scope);
            assert_eq!(
                scope.get_current_context().global(scope).set(
                    scope,
                    crate::util::v8str(scope, "__staleOpfsPromise").into(),
                    promise.into(),
                ),
                Some(true)
            );
            let (_, completion) = unsafe { &mut *host_ptr }
                .register_pending_opfs_task(scope, resolver, locator.clone(), None)
                .expect("OPFS task should capture its Window execution context");
            Ok(completion)
        })
        .expect("OPFS task should register");
    vm.eval(
        r#"
        globalThis.__staleOpfsResult = "pending";
        globalThis.__staleOpfsPromise.then(
          () => { globalThis.__staleOpfsResult = "resolved"; },
          error => { globalThis.__staleOpfsResult = error && error.name; }
        );
        "attached"
        "#,
    )
    .expect("OPFS rejection reaction should attach");

    vm.storage_bucket_store
        .lock()
        .delete_bucket(&storage_key, "stale-read")
        .expect("bucket delete should persist its tombstone")
        .expect("named bucket should exist");
    completion
        .send(crate::opfs_task_result::OpfsTaskResult::GetFile(
            crate::opfs_task_result::OpfsGetFileTaskResult {
                result: Ok(Ok(crate::opfs_task_result::OpfsReadFileResult {
                    path: moli_storage_service::OpfsPath::from_components(vec![
                        "stale.txt".to_owned(),
                    ])
                    .expect("valid OPFS path"),
                    snapshot: moli_storage_service::FileSnapshot {
                        name: "stale.txt".to_owned(),
                        modified_ms: 1,
                        bytes: b"must not escape".to_vec(),
                        identity: moli_storage_service::FileSnapshotIdentity::from_raw(1, 1)
                            .expect("test snapshot identity should be non-zero"),
                    },
                })),
            },
        ))
        .expect("stale OPFS completion should enter its production Page source");
    assert!(
        vm.run_opfs_task_body_for_authorization_test()
            .expect("stale OPFS completion body should settle"),
        "the typed OPFS source should contain the exact pending completion"
    );
    assert_eq!(
        vm.eval_without_microtask_checkpoint_for_test("globalThis.__staleOpfsResult")
            .expect("OPFS body-only reaction state"),
        "pending",
        "the body-only OPFS support must leave Promise reactions to the selected-task checkpoint"
    );
    vm.with_default_context_scope_and_checkpoint_for_test(|_scope, _host_ptr| Ok(()))
        .expect("OPFS rejection checkpoint should run");

    assert_eq!(
        vm.eval("globalThis.__staleOpfsResult")
            .expect("stale OPFS outcome"),
        "NotFoundError"
    );
    assert_eq!(vm._context_host.borrow().pending_opfs_task_count(), 0);
}

#[test]
fn isolated_realm_destruction_retires_pending_opfs_task() {
    let origin = "https://isolated-opfs-owner.test/";
    let mut vm = new_storage_test_vm(origin);
    let isolated_context_id = vm
        .create_isolated_world("opfs-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    let locator = moli_storage_service::StorageBucketLocator::default_bucket(
        moli_storage_key::MoliStorageKey::first_party_from_url(
            &url::Url::parse(origin).unwrap(),
            None,
        )
        .serialized_storage_key(),
    );
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(
        isolated_context_ptr,
        |scope, host_ptr| {
            let resolver = v8::PromiseResolver::new(scope).expect("isolated OPFS resolver");
            assert!(
                unsafe { &mut *host_ptr }
                    .register_pending_opfs_task(scope, resolver, locator, None)
                    .is_some()
            );
            Ok(())
        },
    )
    .expect("isolated OPFS task should register");
    assert_eq!(vm._context_host.borrow().pending_opfs_task_count(), 1);

    vm.destroy_isolated_world_context(isolated_context_id);

    assert_eq!(
        vm._context_host.borrow().pending_opfs_task_count(),
        0,
        "destroying the Promise relevant realm must release its OPFS resolver"
    );
}

#[test]
fn window_opfs_owner_state_materializes_only_after_first_opfs_operation() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-owner-state-lazy.test/");

    let navigator_diagnostics = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            crate::context_bootstrap::navigator_storage_wrapper_diagnostics(scope)
                .ok_or_else(|| anyhow::anyhow!("Navigator diagnostics are unavailable"))
        })
        .expect("initial Navigator diagnostics");
    assert!(!navigator_diagnostics.storage_manager_materialized);
    assert!(!navigator_diagnostics.storage_bucket_manager_materialized);
    assert!(
        !vm._context_host.borrow().has_opfs_owner_state(),
        "creating a Window realm must not eagerly allocate OPFS owner state"
    );
    assert_eq!(
        vm.eval("navigator.storage === navigator.storage")
            .expect("StorageManager SameObject probe"),
        "true"
    );
    let navigator_diagnostics = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            crate::context_bootstrap::navigator_storage_wrapper_diagnostics(scope)
                .ok_or_else(|| anyhow::anyhow!("Navigator diagnostics are unavailable"))
        })
        .expect("post-storage Navigator diagnostics");
    assert!(navigator_diagnostics.storage_manager_materialized);
    assert!(!navigator_diagnostics.storage_bucket_manager_materialized);
    assert_eq!(
        vm.eval("navigator.storageBuckets === navigator.storageBuckets")
            .expect("StorageBucketManager SameObject probe"),
        "true"
    );
    let navigator_diagnostics = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            crate::context_bootstrap::navigator_storage_wrapper_diagnostics(scope)
                .ok_or_else(|| anyhow::anyhow!("Navigator diagnostics are unavailable"))
        })
        .expect("post-storageBuckets Navigator diagnostics");
    assert!(navigator_diagnostics.storage_manager_materialized);
    assert!(navigator_diagnostics.storage_bucket_manager_materialized);
    assert!(
        !vm._context_host.borrow().has_opfs_owner_state(),
        "reading navigator storage wrappers must not allocate OPFS owner state"
    );

    vm.exec(
        r#"
        globalThis.__lazyOpfsRoot = "pending";
        navigator.storage.getDirectory().then(root => {
          globalThis.__lazyOpfsRoot = root.kind;
        });
        "#,
        None,
    )
    .expect("OPFS root probe should schedule");
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(globalThis.__lazyOpfsRoot)")
            .expect("OPFS root probe should settle"),
        "directory"
    );
    assert!(
        vm._context_host.borrow().has_opfs_owner_state(),
        "the first OPFS operation must allocate its owner state"
    );
}

#[test]
fn window_storage_constructor_globals_materialize_on_first_value_read() {
    let mut vm = new_storage_test_vm("https://storage-constructors-lazy.test/");
    let materialization_count = |vm: &mut ScriptVm, name| {
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok(crate::context_bootstrap::lazy_constructor_materialization_count(scope, name))
        })
        .expect("lazy constructor diagnostics")
    };

    for name in [
        "StorageManager",
        "StorageEstimate",
        "StorageBucketManager",
        "StorageBucket",
        "FileSystemHandle",
        "FileSystemFileHandle",
        "FileSystemDirectoryHandle",
        "FileSystemWritableFileStream",
    ] {
        assert_eq!(
            materialization_count(&mut vm, name),
            0,
            "{name} must remain lazy during Window bootstrap"
        );
    }

    assert_eq!(
        vm.eval(
            r#"
            [
              "StorageManager" in globalThis,
              Object.hasOwn(globalThis, "StorageManager"),
              Object.keys(globalThis).includes("StorageManager")
            ].join("|")
            "#
        )
        .expect("non-value constructor probes"),
        "true|true|false"
    );
    assert_eq!(
        materialization_count(&mut vm, "StorageManager"),
        0,
        "existence and enumeration probes must not invoke the lazy getter"
    );

    assert_eq!(
        vm.eval(
            r#"
            const firstStorageManager = StorageManager;
            const descriptor =
              Object.getOwnPropertyDescriptor(globalThis, "StorageManager");
            JSON.stringify({
              same: firstStorageManager === StorageManager,
              name: firstStorageManager.name,
              descriptorValue: descriptor.value === firstStorageManager,
              writable: descriptor.writable,
              enumerable: descriptor.enumerable,
              configurable: descriptor.configurable,
              getterType: typeof descriptor.get
            })
            "#
        )
        .expect("StorageManager first value read"),
        r#"{"same":true,"name":"StorageManager","descriptorValue":true,"writable":true,"enumerable":false,"configurable":true,"getterType":"undefined"}"#
    );
    assert_eq!(materialization_count(&mut vm, "StorageManager"), 1);

    assert_eq!(
        vm.eval(
            r#"
            const directoryConstructor = FileSystemDirectoryHandle;
            const basePrototype =
              Object.getPrototypeOf(directoryConstructor.prototype);
            [
              directoryConstructor === FileSystemDirectoryHandle,
              directoryConstructor.name,
              Object.getPrototypeOf(directoryConstructor) === FileSystemHandle,
              basePrototype[Symbol.toStringTag],
              typeof directoryConstructor.prototype.getDirectoryHandle,
              typeof Object.getOwnPropertyDescriptor(basePrototype, "kind").get,
              Object.getOwnPropertyDescriptor(basePrototype, "kind").get.name
            ].join("|")
            "#
        )
        .expect("derived OPFS constructor first value read"),
        "true|FileSystemDirectoryHandle|true|FileSystemHandle|function|function|get kind"
    );
    assert_eq!(
        materialization_count(&mut vm, "FileSystemDirectoryHandle"),
        1
    );
    assert_eq!(
        materialization_count(&mut vm, "FileSystemHandle"),
        1,
        "materializing a derived constructor must bind it to the same-realm public base constructor"
    );

    assert_eq!(
        vm.eval(
            r#"
            Object.getPrototypeOf(FileSystemWritableFileStream) === WritableStream &&
              Object.getPrototypeOf(
                FileSystemWritableFileStream.prototype
              ) === WritableStream.prototype
            "#
        )
        .expect("writable stream constructor inheritance"),
        "true"
    );
    assert_eq!(
        materialization_count(&mut vm, "FileSystemWritableFileStream"),
        1
    );
}

#[test]
fn assigning_lazy_storage_constructor_before_read_skips_materialization() {
    let mut vm = new_storage_test_vm("https://storage-constructor-override.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.StorageEstimate = 17;
            [
              globalThis.StorageEstimate,
              Object.getOwnPropertyDescriptor(globalThis, "StorageEstimate").value
            ].join("|")
            "#
        )
        .expect("lazy constructor override"),
        "17|17"
    );
    let count = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            Ok(
                crate::context_bootstrap::lazy_constructor_materialization_count(
                    scope,
                    "StorageEstimate",
                ),
            )
        })
        .expect("lazy constructor diagnostics");
    assert_eq!(
        count, 0,
        "assignment before first read must replace the lazy property without invoking it"
    );
}

#[test]
fn page_context_teardown_releases_opfs_handle_and_directory_iterator_registrations() {
    let mut vm = new_storage_page_task_executor_test_vm("https://opfs-iterator-teardown.test/");
    vm.exec(
        r#"
        globalThis.__opfsIteratorSetup = "pending";
        navigator.storage.getDirectory().then(root => {
          globalThis.__opfsRoot = root;
          globalThis.__opfsIterators = [];
          for (let index = 0; index < 16; index += 1) {
            globalThis.__opfsIterators.push(root.keys());
          }
          globalThis.__opfsIteratorSetup = String(globalThis.__opfsIterators.length);
        });
        "#,
        None,
    )
    .expect("OPFS iterator teardown probe should schedule");
    assert_eq!(
        vm.eval_after_selected_page_tasks("String(globalThis.__opfsIteratorSetup)")
            .expect("OPFS iterator setup should settle"),
        "16"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .opfs_handle_registry()
            .expect("OPFS handle registry should be materialized")
            .len(),
        1
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .opfs_directory_iterator_registry()
            .expect("OPFS iterator registry should be materialized")
            .len(),
        16
    );

    vm.close_page_context_resources_for_context_teardown();

    assert_eq!(
        vm._context_host
            .borrow()
            .opfs_handle_registry()
            .expect("OPFS handle registry remains owned until host teardown")
            .len(),
        0,
        "page teardown must run handle finalizers before isolate teardown"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .opfs_directory_iterator_registry()
            .expect("OPFS iterator registry remains owned until host teardown")
            .len(),
        0,
        "page teardown must run iterator finalizers before isolate teardown"
    );
}

#[test]
fn isolated_realm_destruction_retires_webcrypto_task_without_retiring_local_window() {
    let mut vm = new_storage_test_vm("https://isolated-webcrypto-owner.test/");
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let isolated_context_id = vm
        .create_isolated_world("webcrypto-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    let producer = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(
            isolated_context_ptr,
            |scope, host_ptr| {
                let resolver = v8::PromiseResolver::new(scope)
                    .expect("isolated WebCrypto resolver should exist");
                unsafe { &mut *host_ptr }
                    .register_pending_webcrypto_task(scope, resolver)
                    .ok_or_else(|| {
                        anyhow::anyhow!("isolated WebCrypto task should capture its realm")
                    })
            },
        )
        .expect("isolated WebCrypto task should register");
    let pending = vm
        ._context_host
        .borrow()
        .pending_webcrypto_execution_contexts_for_test();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].0,
        crate::native_bridge::WindowExecutionContextOwner::Frame(main_owner.local_window_id)
    );

    vm.destroy_isolated_world_context(isolated_context_id);

    assert_eq!(
        vm._context_host.borrow().pending_webcrypto_task_count(),
        0,
        "destroying the Promise relevant realm must release its resolver"
    );
    assert_eq!(
        vm.current_main_document_task_owner()
            .map(|owner| owner.local_window_id),
        Some(main_owner.local_window_id),
        "realm retirement must not retire the owning LocalWindow"
    );

    producer
        .send(Ok(crate::context_bootstrap::WebCryptoTaskResult::Bool(
            true,
        )))
        .expect("retired-realm completion should still enter the stable Page source");
    assert!(
        vm.run_webcrypto_task_body_for_authorization_test()
            .expect("retired-realm WebCrypto task should consume one stale turn")
    );
    assert_eq!(
        vm._context_host.borrow().pending_webcrypto_task_count(),
        0,
        "a queued completion for the retired realm must not recreate or settle a pending Promise"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn popup_replacement_retires_local_window_owned_webcrypto_tasks() {
    let mut vm = new_storage_test_vm("https://popup-owner-webcrypto.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__ownerBoundCryptoPopup = open("about:blank", "crypto-owner-popup");
            String(globalThis.__ownerBoundCryptoPopup !== null)
            "#,
        )
        .expect("popup WebCrypto owner window should open"),
        "true"
    );
    let popup_id = vm
        .take_pending_popup_activations()
        .into_iter()
        .next()
        .and_then(|activation| activation.popup_id())
        .expect("popup WebCrypto owner id");
    let initial_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("initial popup LocalWindow owner");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let previous_popup =
            crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id);
        let resolver = v8::PromiseResolver::new(scope).expect("popup WebCrypto test resolver");
        let registered = unsafe { &mut *host_ptr }
            .register_pending_webcrypto_task(scope, resolver)
            .is_some();
        crate::native_bridge::restore_active_lightweight_popup_scope(scope, previous_popup);
        assert!(
            registered,
            "popup WebCrypto task should bind a Window execution context"
        );
        Ok(())
    })
    .expect("popup WebCrypto task should register");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_webcrypto_execution_contexts_for_test()
            .into_iter()
            .map(|(owner, _)| owner)
            .collect::<Vec<_>>(),
        vec![
            crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
                popup_id,
                local_window_id: initial_local_window_id,
            }
        ],
        "popup WebCrypto work must capture the preparation-time popup LocalWindow"
    );

    assert_eq!(
        vm.eval(
            r#"
            open("about:blank", "crypto-owner-popup");
            "replacement-committed"
            "#,
        )
        .expect("named popup replacement should commit"),
        "replacement-committed"
    );
    let replacement_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("replacement popup LocalWindow owner");
    assert_ne!(replacement_local_window_id, initial_local_window_id);
    assert_eq!(
        vm._context_host.borrow().pending_webcrypto_task_count(),
        0,
        "popup replacement must retire old-LocalWindow WebCrypto resolvers"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn main_document_open_preserves_local_window_owned_dedicated_worker() {
    let mut vm = new_storage_test_vm("https://main-owner-worker.test/");
    let before_document_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    let expected_owner = crate::native_bridge::WindowExecutionContextOwner::Frame(
        before_document_owner.local_window_id,
    );
    let expected_realm = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
            crate::native_bridge::current_runtime_observable_context_token(scope)
                .ok_or_else(|| anyhow::anyhow!("main realm should expose an observable token"))
        })
        .expect("main realm token should be readable");

    vm.eval(
        r#"
        globalThis.__localWindowWorker = new Worker(
          "data:text/javascript,onmessage = () => {}"
        );
        "created"
        "#,
    )
    .expect("main DedicatedWorker should register");
    let workers_before_open = vm
        ._context_host
        .borrow()
        .worker_execution_contexts_for_test();
    assert_eq!(workers_before_open.len(), 1);
    assert_eq!(workers_before_open[0].1, expected_owner);
    assert_eq!(workers_before_open[0].2, expected_realm);

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");

    let after_document_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(
        after_document_owner.local_window_id, before_document_owner.local_window_id,
        "document.open must preserve the Window execution context"
    );
    assert_ne!(
        after_document_owner.document_id,
        before_document_owner.document_id
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test(),
        workers_before_open,
        "document.open must neither retire nor rebind a LocalWindow-owned Worker"
    );

    vm._context_host
        .borrow_mut()
        .forget_worker(workers_before_open[0].0);
}

#[test]
fn isolated_realm_destruction_retires_dedicated_worker_without_retiring_local_window() {
    let mut vm = new_storage_test_vm("https://isolated-worker-owner.test/");
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let isolated_context_id = vm
        .create_isolated_world("worker-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(
        isolated_context_ptr,
        |scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let owner = host
                .current_runtime_window_execution_context_binding(scope)
                .expect("isolated Worker should capture its relevant realm");
            let outside_settings_load = host
                .register_dedicated_worker_outside_settings_load(owner.dispatch_scope())
                .expect("isolated Worker should capture its Document script-load authority");
            let creator_storage_key = host
                .active_storage_context(scope, None)
                .storage_key()
                .clone();
            let top_level_site = creator_storage_key.top_level_site().to_owned();
            host.register_loading_worker(
                scope,
                v8::Object::new(scope),
                top_level_site,
                creator_storage_key,
                String::new(),
                moli_fetch::RequestCredentialsMode::SameOrigin,
                None,
                outside_settings_load,
                owner,
            );
            Ok(())
        },
    )
    .expect("isolated Worker should register");
    assert_eq!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test()
            .len(),
        1
    );

    vm.destroy_isolated_world_context(isolated_context_id);

    assert!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test()
            .is_empty(),
        "destroying the Worker relevant realm must terminate the Worker"
    );
    assert_eq!(
        vm.current_main_document_task_owner()
            .map(|owner| owner.local_window_id),
        Some(main_owner.local_window_id),
        "realm retirement must not retire the owning LocalWindow"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn popup_replacement_retires_local_window_owned_dedicated_worker() {
    let mut vm = new_storage_test_vm("https://popup-owner-worker.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__ownerBoundWorkerPopup = open(
              "about:blank",
              "dedicated-worker-owner-popup"
            );
            String(globalThis.__ownerBoundWorkerPopup !== null)
            "#,
        )
        .expect("popup Worker owner window should open"),
        "true"
    );
    let popup_id = vm
        .take_pending_popup_activations()
        .into_iter()
        .next()
        .and_then(|activation| activation.popup_id())
        .expect("popup Worker owner id");
    let initial_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("initial popup LocalWindow owner");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let previous_popup =
            crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id);
        let host = unsafe { &mut *host_ptr };
        let owner = host
            .current_runtime_window_execution_context_binding(scope)
            .expect("popup Worker should capture its LocalWindow");
        let outside_settings_load = host
            .register_dedicated_worker_outside_settings_load(owner.dispatch_scope())
            .expect("popup Worker should capture its Document script-load authority");
        let creator_storage_key = host
            .active_storage_context(scope, None)
            .storage_key()
            .clone();
        let top_level_site = creator_storage_key.top_level_site().to_owned();
        host.register_loading_worker(
            scope,
            v8::Object::new(scope),
            top_level_site,
            creator_storage_key,
            String::new(),
            moli_fetch::RequestCredentialsMode::SameOrigin,
            None,
            outside_settings_load,
            owner,
        );
        crate::native_bridge::restore_active_lightweight_popup_scope(scope, previous_popup);
        Ok(())
    })
    .expect("popup Worker should register");
    assert_eq!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test()
            .into_iter()
            .map(|(_, owner, _)| owner)
            .collect::<Vec<_>>(),
        vec![
            crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
                popup_id,
                local_window_id: initial_local_window_id,
            }
        ]
    );

    vm.eval(
        r#"
        open("about:blank", "dedicated-worker-owner-popup");
        "replacement-committed"
        "#,
    )
    .expect("named popup replacement should commit");

    assert!(
        vm._context_host
            .borrow()
            .worker_execution_contexts_for_test()
            .is_empty(),
        "popup replacement must actively terminate old-LocalWindow Workers"
    );
}

#[test]
fn main_document_open_preserves_active_xhr_and_its_wrapper() {
    let mut vm = new_storage_test_vm("https://main-owner-xhr.test/");
    vm.eval("globalThis.__preservedXhrWrapper = new XMLHttpRequest(); 'created'")
        .expect("main XHR wrapper should capture its creation execution context");
    let before_document_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let (internal_id, owner, realm_token) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            Ok(register_pending_window_xhr_for_test(
                scope,
                unsafe { &mut *host_ptr },
                cancel_handle.clone(),
            ))
        })
        .expect("main XHR should register");
    assert_eq!(
        owner,
        crate::native_bridge::WindowExecutionContextOwner::Frame(
            before_document_owner.local_window_id
        )
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test(),
        vec![(internal_id, owner, realm_token)]
    );

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");

    let after_document_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(
        after_document_owner.local_window_id, before_document_owner.local_window_id,
        "document.open must preserve the XHR-owning LocalWindow"
    );
    assert_ne!(
        after_document_owner.document_id, before_document_owner.document_id,
        "document.open must still rotate Document identity"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test(),
        vec![(internal_id, owner, realm_token)],
        "document.open must preserve an active XHR owned by the same LocalWindow"
    );
    assert!(!cancel_handle.is_cancelled());
    assert!(
        vm._context_host
            .borrow_mut()
            .abort_subresource_fetch(internal_id),
        "the preserved XHR must remain in the live request registry"
    );
    assert!(cancel_handle.is_cancelled());

    vm.eval(
        r#"
        __preservedXhrWrapper.open("GET", "data:text/plain,preserved");
        __preservedXhrWrapper.send();
        "queued"
        "#,
    )
    .expect("the same-LocalWindow XHR wrapper should remain usable after document.open");
    vm.eval("0")
        .expect("the preserved XHR data URL completion should run");
    assert_eq!(
        vm.eval("__preservedXhrWrapper.responseText")
            .expect("preserved XHR response should be readable"),
        "preserved"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn child_navigation_retires_local_window_owned_xhr() {
    let mut vm = new_storage_test_vm("https://child-owner-xhr.test/");
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__ownerBoundXhrFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child XHR frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child XHR registration setup",
    )
    .await;
    let initial_owner =
        current_single_child_document_owner_for_test(&vm, "child XHR initial-empty document");
    vm.eval("__ownerBoundXhrFrame.srcdoc = '<p>committed</p>'; 'queued'")
        .expect("first child XHR document should queue");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "first child XHR document").await;
    let committed_owner =
        current_single_child_document_owner_for_test(&vm, "committed child XHR document");
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "the first secure commit must reuse the initial-empty LocalWindow"
    );
    assert_ne!(committed_owner.document_id, initial_owner.document_id);
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child XHR realm should exist");
    vm.eval_in_child_default_context(
        child_context_id,
        "parent.__retiredChildXhrWrapper = new XMLHttpRequest(); 'captured'",
    )
    .expect("parent should retain the child-created XHR wrapper for stale-owner proof");
    let child_context_ptr = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child XHR realm record");
        &realm.context as *const _
    };
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let (internal_id, owner, realm_token) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(child_context_ptr, |scope, host_ptr| {
            Ok(register_pending_window_xhr_for_test(
                scope,
                unsafe { &mut *host_ptr },
                cancel_handle.clone(),
            ))
        })
        .expect("child XHR should register");
    let child_owner = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("child browsing context");
        host.current_child_document_task_owner(child_handle)
            .expect("child document owner")
    };
    assert_eq!(
        owner,
        crate::native_bridge::WindowExecutionContextOwner::Frame(child_owner.local_window_id)
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test(),
        vec![(internal_id, owner, realm_token)]
    );

    vm.eval("__ownerBoundXhrFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "NavigationCommit must retire the old child execution context"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test()
            .is_empty()
    );
    assert!(
        cancel_handle.is_cancelled(),
        "child navigation must abort the old LocalWindow XHR transport"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id,
        request_url: Url::parse("https://xhr-execution-context.test/pending").unwrap(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Err("stale retired XHR completion".to_owned()),
    })
    .expect("late completion for retired XHR should be harmless");
    vm.eval(
        r#"
        __retiredChildXhrWrapper.open(
          "GET",
          "https://xhr-execution-context.test/after-navigation"
        );
        __retiredChildXhrWrapper.send();
        "attempted"
        "#,
    )
    .expect("calling send on a retained old-child XHR wrapper should fail closed");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test()
            .is_empty(),
        "a retained old-child wrapper must not bind a new request to the replacement LocalWindow"
    );
}

#[test]
fn isolated_realm_destruction_retires_xhr_without_retiring_local_window() {
    let mut vm = new_storage_test_vm("https://isolated-xhr-owner.test/");
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let isolated_context_id = vm
        .create_isolated_world("xhr-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(
        isolated_context_ptr,
        |scope, host_ptr| {
            let (_, owner, _) = register_pending_window_xhr_for_test(
                scope,
                unsafe { &mut *host_ptr },
                cancel_handle.clone(),
            );
            assert_eq!(
                owner,
                crate::native_bridge::WindowExecutionContextOwner::Frame(
                    main_owner.local_window_id
                )
            );
            Ok(())
        },
    )
    .expect("isolated XHR should register");

    vm.destroy_isolated_world_context(isolated_context_id);

    assert!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test()
            .is_empty(),
        "destroying the XHR relevant realm must release its wrapper and request"
    );
    assert!(cancel_handle.is_cancelled());
    assert_eq!(
        vm.current_main_document_task_owner()
            .map(|owner| owner.local_window_id),
        Some(main_owner.local_window_id),
        "realm retirement must not retire the owning LocalWindow"
    );
}

#[test]
fn popup_replacement_retires_local_window_owned_xhr() {
    let mut vm = new_storage_test_vm("https://popup-owner-xhr.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__ownerBoundXhrPopup = open("about:blank", "xhr-owner-popup");
            String(globalThis.__ownerBoundXhrPopup !== null)
            "#,
        )
        .expect("popup XHR owner window should open"),
        "true"
    );
    let popup_id = vm
        .take_pending_popup_activations()
        .into_iter()
        .next()
        .and_then(|activation| activation.popup_id())
        .expect("popup XHR owner id");
    let initial_local_window_id = vm
        ._context_host
        .borrow()
        .current_lightweight_popup_local_window_id(popup_id)
        .expect("initial popup LocalWindow owner");
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let (_, owner, _) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let previous_popup =
                crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id);
            let registered = register_pending_window_xhr_for_test(
                scope,
                unsafe { &mut *host_ptr },
                cancel_handle.clone(),
            );
            crate::native_bridge::restore_active_lightweight_popup_scope(scope, previous_popup);
            Ok(registered)
        })
        .expect("popup XHR should register");
    assert_eq!(
        owner,
        crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
            popup_id,
            local_window_id: initial_local_window_id,
        }
    );

    vm.eval(r#"open("about:blank", "xhr-owner-popup"); "replacement-committed""#)
        .expect("named popup replacement should commit");

    assert!(
        vm._context_host
            .borrow()
            .pending_window_xhr_execution_contexts_for_test()
            .is_empty(),
        "popup replacement must remove old-LocalWindow XHR state"
    );
    assert!(
        cancel_handle.is_cancelled(),
        "popup replacement must abort old-LocalWindow XHR transport"
    );
}

#[test]
fn window_fetch_retirement_covers_every_host_stage() {
    let mut vm = new_storage_test_vm("https://fetch-stage-retirement.test/");
    let stages = [
        PendingWindowFetchTestStage::Pending,
        PendingWindowFetchTestStage::Running,
        PendingWindowFetchTestStage::Streaming,
        PendingWindowFetchTestStage::Auth,
        PendingWindowFetchTestStage::Response,
        PendingWindowFetchTestStage::ServiceWorkerInFlight,
    ];
    let mut ordinary = Vec::new();
    let mut keepalive = Vec::new();
    let mut expected_owner = None;
    let mut expected_realm = None;
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let host = unsafe { &mut *host_ptr };
        for stage in stages {
            let ordinary_fetch = register_pending_window_fetch_for_test(scope, host, false, stage);
            let keepalive_fetch = register_pending_window_fetch_for_test(scope, host, true, stage);
            expected_owner = Some(ordinary_fetch.1);
            expected_realm = Some(ordinary_fetch.2);
            ordinary.push((ordinary_fetch.0, ordinary_fetch.3));
            keepalive.push((keepalive_fetch.0, keepalive_fetch.3));
        }
        Ok(())
    })
    .expect("all Window Fetch host stages should be registered");
    let expected_owner = expected_owner.expect("Window Fetch owner");
    let expected_realm = expected_realm.expect("Window Fetch realm");

    assert_eq!(
        vm._context_host
            .borrow_mut()
            .retire_window_fetches_for_execution_context_owner(expected_owner),
        (stages.len(), stages.len()),
        "one owner retirement must abort ordinary Fetch and detach keepalive in every host stage"
    );
    assert!(
        ordinary
            .iter()
            .all(|(_, cancel_handle)| cancel_handle.is_cancelled()),
        "ordinary Fetch transport must be cancelled in every host stage"
    );
    assert!(
        keepalive
            .iter()
            .all(|(_, cancel_handle)| !cancel_handle.is_cancelled()),
        "execution-context destruction must not cancel keepalive transport"
    );
    let detached = vm
        ._context_host
        .borrow()
        .pending_window_fetch_execution_contexts_for_test();
    assert_eq!(detached.len(), stages.len());
    assert!(detached.iter().all(|(_, is_detached, owner, realm)| {
        *is_detached && *owner == Some(expected_owner) && *realm == Some(expected_realm)
    }));

    for (internal_id, _) in keepalive {
        assert!(
            vm._context_host
                .borrow_mut()
                .abort_subresource_fetch(internal_id),
            "detached keepalive stage should remain explicitly cancellable"
        );
    }
    assert!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test()
            .is_empty()
    );
}

#[test]
fn window_fetch_request_start_records_keepalive_disposition() {
    let mut vm = new_storage_test_vm("https://fetch-keepalive-output.test/");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let host = unsafe { &mut *host_ptr };
        let _ordinary = register_pending_window_fetch_for_test(
            scope,
            host,
            false,
            PendingWindowFetchTestStage::Pending,
        );
        let _keepalive = register_pending_window_fetch_for_test(
            scope,
            host,
            true,
            PendingWindowFetchTestStage::Pending,
        );
        Ok(())
    })
    .expect("ordinary and keepalive Fetches should register");

    let starts = vm
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request) => {
                Some(request.keepalive())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        starts,
        vec![false, true],
        "renderer network start facts must preserve the resource teardown disposition"
    );
}

#[test]
fn cancel_pending_window_fetch_auth_preserves_401_for_response_stage() {
    let mut vm = new_storage_test_vm("https://fetch-auth-cancel.test/");
    let internal_id = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            Ok(register_pending_window_fetch_for_test(
                scope,
                unsafe { &mut *host_ptr },
                false,
                PendingWindowFetchTestStage::Auth,
            )
            .0)
        })
        .expect("pending Window Fetch auth should register");
    {
        let mut host = vm._context_host.borrow_mut();
        let mut pending = host
            .take_pending_subresource_auth(internal_id)
            .expect("pending auth state");
        pending.intercept_response = true;
        host.record_pending_subresource_auth(pending);
    }

    let _ = vm
        .cancel_pending_subresource_auth_body(internal_id)
        .expect("CancelAuth should expose the challenged response");

    let events = vm.take_pending_subresource_continue_events();
    let [crate::types::PendingSubresourceContinueEvent::ResponsePaused(info)] = events.as_slice()
    else {
        panic!("CancelAuth should emit one response-stage pause, got {events:?}");
    };
    assert_eq!(info.internal_id, internal_id);
    assert_eq!(info.response_status, 401);
    assert_eq!(
        info.response_body.try_bytes().unwrap().as_ref(),
        b"auth required"
    );

    let pending = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_response(internal_id)
        .expect("challenged response should remain pending for Fetch.continueResponse");
    assert_eq!(pending.response.status, 401);
    assert_eq!(pending.response.body_text(), "auth required");
}

#[test]
fn main_document_open_preserves_ordinary_and_keepalive_fetches() {
    let mut vm = new_storage_test_vm("https://main-owner-fetch.test/");
    let before_document_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    let (ordinary, keepalive) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            Ok((
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    false,
                    PendingWindowFetchTestStage::Pending,
                ),
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    true,
                    PendingWindowFetchTestStage::Pending,
                ),
            ))
        })
        .expect("main Fetches should register");
    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");

    let after_document_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(
        after_document_owner.local_window_id, before_document_owner.local_window_id,
        "document.open must preserve the Fetch-owning LocalWindow"
    );
    assert_ne!(
        after_document_owner.document_id, before_document_owner.document_id,
        "document.open must still rotate Document identity"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![
            (ordinary.0, false, Some(ordinary.1), Some(ordinary.2)),
            (keepalive.0, false, Some(keepalive.1), Some(keepalive.2)),
        ],
        "document.open must preserve both Fetches and their JS delivery endpoints"
    );
    assert!(!ordinary.3.is_cancelled());
    assert!(!keepalive.3.is_cancelled());

    assert!(
        vm._context_host
            .borrow_mut()
            .abort_subresource_fetch(ordinary.0),
        "the preserved ordinary Fetch must remain explicitly cancellable"
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .abort_subresource_fetch(keepalive.0)
    );
    assert!(ordinary.3.is_cancelled());
    assert!(keepalive.3.is_cancelled());
}

#[test]
fn main_document_open_fetch_redirect_uses_source_document_csp_report_context() {
    let mut vm = new_storage_test_vm("https://main-fetch-csp-owner.test/source-document");
    vm.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
    );
    let request_url = Url::parse("https://main-fetch-csp-owner.test/accepted").unwrap();
    let final_url = Url::parse("https://redirected-fetch-csp-owner.test/final").unwrap();
    let report_url = Url::parse("https://main-fetch-csp-owner.test/report").unwrap();
    let policy = crate::document_runtime::DocumentPolicyContainer {
        response_content_security_report_only_policies: vec![format!(
            "connect-src 'none'; report-uri {report_url}"
        )],
        ..Default::default()
    };
    let registered = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            Ok(register_pending_window_fetch_with_connect_policy_for_test(
                scope,
                unsafe { &mut *host_ptr },
                false,
                policy,
                request_url.clone(),
            ))
        })
        .expect("main Fetch should capture its source Document CSP context");
    let source_document_owner = registered.4.owner();

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        globalThis.__replacementFetchCspEvents = 0;
        document.addEventListener("securitypolicyviolation", () => {
          globalThis.__replacementFetchCspEvents += 1;
        });
        "replaced"
        "#,
    )
    .expect("document.open should preserve the Fetch execution context");
    let replacement_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main owner");
    assert_eq!(
        Some(replacement_owner.local_window_id),
        match source_document_owner {
            crate::native_bridge::WindowDocumentOwner::Frame(owner) => {
                Some(owner.local_window_id)
            }
            crate::native_bridge::WindowDocumentOwner::LightweightPopup(_) => None,
        }
    );
    assert_ne!(
        source_document_owner,
        crate::native_bridge::WindowDocumentOwner::Frame(replacement_owner)
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: registered.0,
        request_url: request_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: Some("OK".to_owned()),
        skip_fetch_security_validation: true,
        response_filter: None,
        network_error_text: None,
        result: Ok(redirected_fetch_response(&request_url, final_url)),
    })
    .expect("source-owned Fetch redirect should complete in the preserved LocalWindow");

    assert_eq!(
        vm.eval("String(globalThis.__replacementFetchCspEvents)")
            .expect("replacement CSP event count"),
        "0",
        "an old Document's redirect violation must not dispatch into its replacement"
    );
    let reports = vm
        ._context_host
        .borrow()
        .pending_window_csp_report_execution_contexts_for_test();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].1, registered.4);
    assert!(!reports[0].2, "CSP report transport must not retain V8");
    assert!(vm.take_network_output().into_items().any(|item| matches!(
        item,
        crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
            if record.resource_type() == crate::types::SubresourceResourceType::Fetch
                && matches!(record.outcome(), crate::types::SubresourceNetworkOutcome::Success { .. })
    )));
}

#[tokio::test(flavor = "current_thread")]
async fn child_navigation_aborts_fetch_and_detaches_keepalive() {
    let mut vm = new_storage_test_vm("https://child-owner-fetch.test/");
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__ownerBoundFetchFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child Fetch frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child Fetch registration setup",
    )
    .await;
    let initial_owner =
        current_single_child_document_owner_for_test(&vm, "child Fetch initial-empty document");
    vm.eval("__ownerBoundFetchFrame.srcdoc = '<p>committed</p>'; 'queued'")
        .expect("first child Fetch document should queue");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "first child Fetch document").await;
    let committed_owner =
        current_single_child_document_owner_for_test(&vm, "committed child Fetch document");
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "the first secure commit must reuse the initial-empty LocalWindow"
    );
    assert_ne!(committed_owner.document_id, initial_owner.document_id);
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child Fetch realm should exist");
    vm.eval_in_child_default_context(
        child_context_id,
        r#"
        parent.__retiredChildFetch = (...args) => fetch(...args);
        parent.__retiredChildPromiseConstructor = Promise;
        parent.__retiredChildTypeErrorConstructor = TypeError;
        "captured"
        "#,
    )
    .expect("parent should retain an old-child Fetch closure");
    let child_context_ptr = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child Fetch realm record");
        &realm.context as *const _
    };
    let (ordinary, keepalive) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(child_context_ptr, |scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            Ok((
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    false,
                    PendingWindowFetchTestStage::Pending,
                ),
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    true,
                    PendingWindowFetchTestStage::Pending,
                ),
            ))
        })
        .expect("child Fetches should register");

    vm.eval("__ownerBoundFetchFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "NavigationCommit must retire the old child execution context"
    );
    assert!(ordinary.3.is_cancelled());
    assert!(!keepalive.3.is_cancelled());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![(keepalive.0, true, Some(keepalive.1), Some(keepalive.2))]
    );

    vm.eval(
        r#"
        globalThis.__retiredChildFetchResult = "pending";
        const stalePromise =
          __retiredChildFetch("https://fetch-execution-context.test/stale");
        globalThis.__retiredChildFetchPromiseRealm =
          Object.getPrototypeOf(stalePromise) ===
            __retiredChildPromiseConstructor.prototype;
        stalePromise.then(
          () => { __retiredChildFetchResult = "resolved"; },
          error => {
            __retiredChildFetchResult = JSON.stringify([
              error instanceof __retiredChildTypeErrorConstructor,
              error.message
            ]);
          }
        );
        "queued"
        "#,
    )
    .expect("retained old-child Fetch closure should fail closed");
    vm.eval("0").expect("stale Fetch rejection microtask");
    assert_eq!(
        vm.eval("String(__retiredChildFetchPromiseRealm)")
            .expect("stale Fetch Promise realm"),
        "true",
        "binding-time shutdown must reject in the retained function's old child realm"
    );
    assert_eq!(
        vm.eval("__retiredChildFetchResult")
            .expect("stale Fetch result"),
        r#"[true,"Failed to execute 'fetch' on 'Window': The global scope is shutting down."]"#
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![(keepalive.0, true, Some(keepalive.1), Some(keepalive.2))],
        "old child realm must not bind a new request to the replacement LocalWindow"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: ordinary.0,
        request_url: Url::parse("https://fetch-execution-context.test/pending").unwrap(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Err("stale retired Fetch completion".to_owned()),
    })
    .expect("late ordinary Fetch completion should be harmless");
    let _ = vm.take_network_output();
    let final_url = Url::parse("https://fetch-execution-context.test/pending").unwrap();
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: keepalive.0,
        request_url: final_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: Some("OK".to_owned()),
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            final_url,
            200,
            vec![("content-type".to_owned(), "text/plain".to_owned())],
            "keepalive completed".to_owned(),
        )),
    })
    .expect("detached keepalive completion should remain observable without V8");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test()
            .is_empty()
    );
    let records = vm
        .take_network_output()
        .into_items()
        .filter(|item| {
            matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                    if record.url().as_str()
                        == "https://fetch-execution-context.test/pending"
            )
        })
        .count();
    assert_eq!(
        records, 1,
        "detached keepalive must preserve network observation without settling the old Promise"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn detached_keepalive_redirect_reports_source_document_csp_without_v8() {
    let mut vm = new_storage_test_vm("https://detached-fetch-csp-owner.test/");
    vm.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
    );
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__detachedFetchCspFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child Fetch CSP frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child Fetch CSP registration setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child Fetch CSP registration setup",
    );
    let child_context_ptr = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child Fetch CSP realm record");
        &realm.context as *const _
    };
    let report_url = Url::parse("https://detached-fetch-csp-owner.test/report").unwrap();
    let report_only_request =
        Url::parse("https://detached-fetch-csp-owner.test/report-only-source").unwrap();
    let enforce_request =
        Url::parse("https://detached-fetch-csp-owner.test/enforce-source").unwrap();
    let report_only_policy = crate::document_runtime::DocumentPolicyContainer {
        response_content_security_report_only_policies: vec![format!(
            "connect-src 'none'; report-uri {report_url}"
        )],
        ..Default::default()
    };
    let enforce_policy = crate::document_runtime::DocumentPolicyContainer {
        response_content_security_policies: vec![format!(
            "connect-src 'none'; report-uri {report_url}"
        )],
        ..Default::default()
    };
    let (report_only_fetch, enforce_fetch) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(child_context_ptr, |scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            Ok((
                register_pending_window_fetch_with_connect_policy_for_test(
                    scope,
                    host,
                    true,
                    report_only_policy,
                    report_only_request.clone(),
                ),
                register_pending_window_fetch_with_connect_policy_for_test(
                    scope,
                    host,
                    true,
                    enforce_policy,
                    enforce_request.clone(),
                ),
            ))
        })
        .expect("child keepalive Fetches should capture their source Document policy");
    assert_eq!(report_only_fetch.4, enforce_fetch.4);

    vm.eval("__detachedFetchCspFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    assert!(!report_only_fetch.3.is_cancelled());
    assert!(!enforce_fetch.3.is_cancelled());
    let replacement_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("replacement child Fetch CSP realm");
    vm.eval_in_child_default_context(
        replacement_context_id,
        r#"
        globalThis.__replacementFetchCspEvents = 0;
        self.addEventListener("securitypolicyviolation", () => {
          globalThis.__replacementFetchCspEvents += 1;
        });
        "listening"
        "#,
    )
    .expect("replacement child should install its CSP listener");

    let report_only_final = Url::parse("https://report-only-redirect-target.test/final").unwrap();
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: report_only_fetch.0,
        request_url: report_only_request.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: Some("OK".to_owned()),
        skip_fetch_security_validation: true,
        response_filter: None,
        network_error_text: None,
        result: Ok(redirected_fetch_response(
            &report_only_request,
            report_only_final,
        )),
    })
    .expect("detached report-only keepalive should complete without V8");
    let enforce_final = Url::parse("https://enforce-redirect-target.test/final").unwrap();
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: enforce_fetch.0,
        request_url: enforce_request.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: Some("OK".to_owned()),
        skip_fetch_security_validation: true,
        response_filter: None,
        network_error_text: None,
        result: Ok(redirected_fetch_response(&enforce_request, enforce_final)),
    })
    .expect("detached enforcing keepalive should fail without entering V8");

    assert_eq!(
        vm.eval_in_child_default_context(
            replacement_context_id,
            "String(globalThis.__replacementFetchCspEvents)",
        )
        .expect("replacement child CSP event count"),
        "0"
    );
    let reports = vm
        ._context_host
        .borrow()
        .pending_window_csp_report_execution_contexts_for_test();
    assert_eq!(reports.len(), 2);
    assert!(
        reports
            .iter()
            .all(|(_, identity, retained_v8, credentials)| {
                *identity == report_only_fetch.4
                    && !retained_v8
                    && *credentials == moli_fetch::RequestCredentialsMode::SameOrigin
            })
    );
    let fetch_records = vm
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                if record.resource_type() == crate::types::SubresourceResourceType::Fetch =>
            {
                Some(record)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(fetch_records.len(), 2);
    assert!(fetch_records.iter().any(|record| {
        record.url() == &report_only_request
            && matches!(
                record.outcome(),
                crate::types::SubresourceNetworkOutcome::Success { .. }
            )
    }));
    assert!(fetch_records.iter().any(|record| {
        record.url() == &enforce_request
            && matches!(
                record.outcome(),
                crate::types::SubresourceNetworkOutcome::Failure { error_text }
                    if error_text.contains("Content Security Policy")
            )
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn child_navigation_keeps_accepted_beacon_network_only_and_rejects_stale_sender() {
    let mut vm = new_storage_test_vm("https://child-owner-beacon.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__ownerBoundBeaconFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child Beacon frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child Beacon registration setup",
    )
    .await;
    let initial_owner =
        current_single_child_document_owner_for_test(&vm, "child Beacon initial-empty document");
    vm.eval("__ownerBoundBeaconFrame.srcdoc = '<p>committed</p>'; 'queued'")
        .expect("first child Beacon document should queue");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "first child Beacon document")
        .await;
    let committed_owner =
        current_single_child_document_owner_for_test(&vm, "committed child Beacon document");
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "the first secure commit must reuse the initial-empty LocalWindow"
    );
    assert_ne!(committed_owner.document_id, initial_owner.document_id);
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child Beacon realm should exist");
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            r#"
            parent.__retiredChildBeacon = (...args) => navigator.sendBeacon(...args);
            String(navigator.sendBeacon(
              "https://beacon-execution-context.test/accepted",
              "payload"
            ))
            "#,
        )
        .expect("child Beacon should be accepted"),
        "true"
    );

    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    let (internal_id, accepted_identity, retained_v8_context) = accepted[0];
    assert!(
        !retained_v8_context,
        "accepted Beacon must not retain its source V8 context"
    );
    let child_handle = match accepted_identity.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Child(handle) => handle,
        other => panic!("child Beacon used unexpected dispatch scope: {other:?}"),
    };
    assert_eq!(
        accepted_identity.owner(),
        crate::native_bridge::WindowExecutionContextOwner::Frame(
            vm._context_host
                .borrow()
                .current_child_document_task_owner(child_handle)
                .expect("child Beacon owner should be current before navigation")
                .local_window_id,
        )
    );

    vm.eval("__ownerBoundBeaconFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "NavigationCommit must retire the old child execution context"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test(),
        accepted,
        "an accepted Beacon must survive LocalWindow destruction without rebinding"
    );
    assert_ne!(
        vm._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
            .expect("replacement child owner")
            .local_window_id,
        match accepted_identity.owner() {
            crate::native_bridge::WindowExecutionContextOwner::Frame(local_window_id) => {
                local_window_id
            }
            other => panic!("child Beacon used unexpected owner: {other:?}"),
        }
    );
    assert_eq!(
        vm.eval(
            r#"String(__retiredChildBeacon(
              "https://beacon-execution-context.test/stale",
              "stale"
            ))"#,
        )
        .expect("retained old-child Beacon sender should fail closed"),
        "false"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test(),
        accepted,
        "old child realm must not bind a new Beacon to the replacement LocalWindow"
    );

    let request_url = Url::parse("https://beacon-execution-context.test/accepted").unwrap();
    let body_source_id = 60_000 + internal_id;
    vm.start_streaming_async_subresource_fetch(crate::types::AsyncSubresourceStreamingStarted {
        internal_id,
        request_url: request_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: Vec::new(),
        request_body: Some("payload".to_owned()),
        body_source_id,
        network_request_headers: None,
        head: moli_fetch::ResponseHead {
            final_url: request_url,
            status: 204,
            headers: Vec::new(),
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
    })
    .expect("accepted Beacon should start streaming without its retired V8 context");
    vm.append_streaming_async_subresource_fetch_chunk(
        body_source_id,
        b"unobservable response body".to_vec(),
    );
    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("accepted Beacon should finish streaming without its retired V8 context");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test()
            .is_empty(),
        "Beacon terminal must release its network-only host state"
    );
    assert_eq!(
        vm.take_network_output()
            .into_items()
            .filter(|item| matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                    if record.url().as_str()
                        == "https://beacon-execution-context.test/accepted"
            ))
            .count(),
        1,
        "accepted Beacon must remain network-observable after source LocalWindow destruction"
    );
}

#[test]
fn main_document_open_preserves_accepted_beacon_without_rebind() {
    let mut vm = new_storage_test_vm("https://main-owner-beacon.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    let before_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    assert_eq!(
        vm.eval(
            r#"String(navigator.sendBeacon(
              "https://beacon-execution-context.test/main",
              "payload"
            ))"#,
        )
        .expect("main Beacon should be accepted"),
        "true"
    );
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert!(!accepted[0].2, "accepted Beacon must not retain V8");

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should replace only the Document");
    let after_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_eq!(after_owner.local_window_id, before_owner.local_window_id);
    assert_ne!(after_owner.document_id, before_owner.document_id);
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test(),
        accepted,
        "document.open must not retire or rebind accepted LocalWindow Beacon work"
    );

    let request_url = Url::parse("https://beacon-execution-context.test/main").unwrap();
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: accepted[0].0,
        request_url: request_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: Vec::new(),
        request_body: Some("payload".to_owned()),
        response_status_text: Some("No Content".to_owned()),
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            request_url,
            204,
            Vec::new(),
            String::new(),
        )),
    })
    .expect("accepted main Beacon should complete without entering V8");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test()
            .is_empty()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn child_csp_report_keeps_exact_violation_document_without_v8_after_navigation() {
    let mut vm = new_storage_test_vm("https://child-owner-csp-report.test/");
    vm.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
    );
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          globalThis.__ownerBoundCspReportFrame = frame;
          (document.body || document.documentElement || document).appendChild(frame);
          void frame.contentWindow;
        })();
        "frame-ready"
        "#,
    )
    .expect("child CSP report frame should materialize");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child CSP report acceptance setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child CSP report acceptance setup",
    );
    let (child_handle, child_context_ptr) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child CSP report realm record");
        (realm.child_handle, &realm.context as *const _)
    };
    let document_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("child CSP report document owner");
    let document_url = vm
        ._context_host
        .borrow()
        .child_browsing_context_current_url(child_handle)
        .expect("child CSP report document URL");
    let report_url = Url::parse("https://csp-report-owner.test/report").unwrap();
    let violation = test_window_csp_report_violation(&document_url, &report_url);
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(child_context_ptr, |scope, host_ptr| {
        unsafe { &mut *host_ptr }
            .dispatch_child_content_security_policy_violation_event_best_effort(
                scope,
                child_handle,
                &violation,
            );
        Ok(())
    })
    .expect("child CSP violation should dispatch");

    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_csp_report_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    let (internal_id, identity, retained_v8_context, credentials_mode) = accepted[0];
    assert_eq!(
        identity.owner(),
        crate::native_bridge::WindowDocumentOwner::Frame(document_owner)
    );
    assert_eq!(
        identity.dispatch_scope(),
        crate::native_bridge::OwnerDispatchScope::Child(child_handle)
    );
    assert!(!retained_v8_context);
    assert_eq!(
        credentials_mode,
        moli_fetch::RequestCredentialsMode::SameOrigin
    );

    vm.eval("__ownerBoundCspReportFrame.srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test(),
        accepted,
        "accepted report must survive replacement with its original Document identity"
    );

    let fields =
        crate::content_security_policy::ContentSecurityPolicyViolationEventFields::from(&violation);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        crate::network_host::send_content_security_policy_reports_for_window(
            scope,
            unsafe { &mut *host_ptr },
            document_owner,
            Some(child_handle),
            &fields,
            &violation.report_uri_endpoints,
            &violation.report_to_endpoints,
        );
        Ok(())
    })
    .expect("stale exact-owner report attempt should be consumed");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test(),
        accepted,
        "retired Document owner must not bind a new report to the replacement child"
    );

    let body_source_id = 70_000 + internal_id;
    vm.start_streaming_async_subresource_fetch(crate::types::AsyncSubresourceStreamingStarted {
        internal_id,
        request_url: report_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: vec![(
            "Content-Type".to_owned(),
            "application/csp-report".to_owned(),
        )],
        request_body: Some("report".to_owned()),
        body_source_id,
        network_request_headers: None,
        head: moli_fetch::ResponseHead {
            final_url: report_url.clone(),
            status: 204,
            headers: Vec::new(),
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
    })
    .expect("accepted CSP report should stream without its retired V8 context");
    vm.append_streaming_async_subresource_fetch_chunk(
        body_source_id,
        b"unobservable report response".to_vec(),
    );
    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("accepted CSP report should finish without its retired V8 context");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test()
            .is_empty()
    );
    assert_eq!(
        vm.take_network_output()
            .into_items()
            .filter(|item| matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                    if record.url() == &report_url
            ))
            .count(),
        1,
        "report terminal must remain network-observable after Document retirement"
    );
}

#[test]
fn main_document_open_preserves_accepted_csp_report_but_rejects_stale_owner_reuse() {
    let document_url = Url::parse("https://main-owner-csp-report.test/page").unwrap();
    let report_url = Url::parse("https://main-owner-csp-report.test/report").unwrap();
    let violation = test_window_csp_report_violation(&document_url, &report_url);
    let mut vm = new_storage_test_vm(document_url.as_str());
    vm.set_fetch_subresource_interception(
        true,
        Some(crate::types::SubresourceResourceType::CspReport),
    );
    let before_owner = vm
        .current_main_document_task_owner()
        .expect("initial main CSP report owner");
    vm.queue_content_security_policy_violation_event_best_effort(&violation);
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_csp_report_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert_eq!(
        accepted[0].1.owner(),
        crate::native_bridge::WindowDocumentOwner::Frame(before_owner)
    );
    assert!(!accepted[0].2);

    vm.eval(
        r#"
        document.open();
        document.write("<!doctype html><title>replacement</title>");
        document.close();
        "replaced"
        "#,
    )
    .expect("document.open should rotate the report Document owner");
    let after_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main CSP report owner");
    assert_eq!(after_owner.local_window_id, before_owner.local_window_id);
    assert_ne!(after_owner.document_id, before_owner.document_id);
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test(),
        accepted
    );

    let fields =
        crate::content_security_policy::ContentSecurityPolicyViolationEventFields::from(&violation);
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        crate::network_host::send_content_security_policy_reports_for_window(
            scope,
            unsafe { &mut *host_ptr },
            before_owner,
            None,
            &fields,
            &violation.report_uri_endpoints,
            &violation.report_to_endpoints,
        );
        Ok(())
    })
    .expect("stale main report attempt should be consumed");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test(),
        accepted,
        "old main Document owner must not enqueue a second report after document.open"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: accepted[0].0,
        request_url: report_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: Vec::new(),
        request_body: Some("report".to_owned()),
        response_status_text: Some("No Content".to_owned()),
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            report_url,
            204,
            Vec::new(),
            String::new(),
        )),
    })
    .expect("accepted main CSP report should complete without entering V8");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_csp_report_execution_contexts_for_test()
            .is_empty()
    );
}

#[test]
fn isolated_realm_destruction_aborts_fetch_and_detaches_keepalive() {
    let mut vm = new_storage_test_vm("https://isolated-fetch-owner.test/");
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let isolated_context_id = vm
        .create_isolated_world("fetch-owner", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world should be tracked");
        &world.context as *const _
    };
    let (ordinary, keepalive) = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(
            isolated_context_ptr,
            |scope, host_ptr| {
                let host = unsafe { &mut *host_ptr };
                Ok((
                    register_pending_window_fetch_for_test(
                        scope,
                        host,
                        false,
                        PendingWindowFetchTestStage::Pending,
                    ),
                    register_pending_window_fetch_for_test(
                        scope,
                        host,
                        true,
                        PendingWindowFetchTestStage::Pending,
                    ),
                ))
            },
        )
        .expect("isolated Fetches should register");

    vm.destroy_isolated_world_context(isolated_context_id);

    assert!(ordinary.3.is_cancelled());
    assert!(!keepalive.3.is_cancelled());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![(keepalive.0, true, Some(keepalive.1), Some(keepalive.2))]
    );
    assert_eq!(
        vm.current_main_document_task_owner()
            .map(|owner| owner.local_window_id),
        Some(main_owner.local_window_id),
        "realm retirement must not retire the owning LocalWindow"
    );
    let request_url = Url::parse("https://fetch-execution-context.test/pending").unwrap();
    let body_source_id = 50_000 + keepalive.0;
    vm.start_streaming_async_subresource_fetch(crate::types::AsyncSubresourceStreamingStarted {
        internal_id: keepalive.0,
        request_url: request_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        body_source_id,
        network_request_headers: None,
        head: moli_fetch::ResponseHead {
            final_url: request_url,
            status: 200,
            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
    })
    .expect("detached keepalive should accept streaming headers without V8");
    vm.append_streaming_async_subresource_fetch_chunk(
        body_source_id,
        b"detached streaming body".to_vec(),
    );
    vm.finish_streaming_async_subresource_fetch(keepalive.0, body_source_id, Ok(()))
        .expect("detached keepalive stream should finish without V8");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test()
            .is_empty(),
        "detached streaming terminal must release its host state"
    );
    assert!(!keepalive.3.is_cancelled());
    assert_eq!(
        vm.take_network_output()
            .into_items()
            .filter(|item| matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                    if record.url().as_str()
                        == "https://fetch-execution-context.test/pending"
            ))
            .count(),
        1,
        "detached streaming keepalive must preserve terminal network observation"
    );
}

#[test]
fn popup_replacement_aborts_fetch_and_detaches_keepalive() {
    let mut vm = new_storage_test_vm("https://popup-owner-fetch.test/");
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__ownerBoundFetchPopup = open("about:blank", "fetch-owner-popup");
            String(globalThis.__ownerBoundFetchPopup !== null)
            "#,
        )
        .expect("popup Fetch owner window should open"),
        "true"
    );
    let popup_id = vm
        .take_pending_popup_activations()
        .into_iter()
        .next()
        .and_then(|activation| activation.popup_id())
        .expect("popup Fetch owner id");
    let (ordinary, keepalive) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let previous_popup =
                crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id);
            let host = unsafe { &mut *host_ptr };
            let registered = (
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    false,
                    PendingWindowFetchTestStage::Pending,
                ),
                register_pending_window_fetch_for_test(
                    scope,
                    host,
                    true,
                    PendingWindowFetchTestStage::Pending,
                ),
            );
            crate::native_bridge::restore_active_lightweight_popup_scope(scope, previous_popup);
            Ok(registered)
        })
        .expect("popup Fetches should register");

    vm.eval(r#"open("about:blank", "fetch-owner-popup"); "replacement-committed""#)
        .expect("named popup replacement should commit");

    assert!(ordinary.3.is_cancelled());
    assert!(!keepalive.3.is_cancelled());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_fetch_execution_contexts_for_test(),
        vec![(keepalive.0, true, Some(keepalive.1), Some(keepalive.2))]
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .abort_subresource_fetch(keepalive.0)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_window_requests_bind_and_retire_exact_document_owners() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://service-worker-owner.test/page.html",
        &loader,
    );
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_ready_owners_for_test()
            .is_empty(),
        "lazy Navigator bootstrap must not create a service worker ready request"
    );
    vm.eval("void navigator.serviceWorker.ready")
        .expect("top-level service worker ready promise should materialize");
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_ready_owners_for_test()
            .into_iter()
            .any(|(_, owner)| owner.document_owner() == Some(main_owner)),
        "the top-level ready promise must bind the initial main document owner"
    );

    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          (document.body || document.documentElement || document).appendChild(frame);
          globalThis.__serviceWorkerOwnerFrame = frame;
        })();
        "frame-ready"
        "#,
    )
    .expect("service worker owner frame should schedule");
    assert_initial_about_blank_child_completed_through_page_for_test(
        &mut vm,
        &loader,
        "service worker container setup",
    )
    .await;
    vm.eval(
        "void globalThis.__serviceWorkerOwnerFrame.contentWindow.navigator.serviceWorker.ready",
    )
    .expect("child service worker ready promise should materialize");

    let child_owner = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("child browsing context");
        host.current_child_document_task_owner(child_handle)
            .expect("child document owner")
    };
    let ready_owners = vm
        ._context_host
        .borrow()
        .pending_service_worker_ready_owners_for_test();
    assert!(
        ready_owners
            .iter()
            .any(|(_, owner)| owner.document_owner() == Some(main_owner)),
        "materializing child ready must not overwrite the main ready resolver"
    );
    assert!(
        ready_owners
            .iter()
            .any(|(_, owner)| owner.document_owner() == Some(child_owner)),
        "child ready must bind the child document owner"
    );
    let (child_ready_request_id, child_ready_owner) = ready_owners
        .iter()
        .copied()
        .find(|(_, owner)| owner.document_owner() == Some(child_owner))
        .expect("child ready request");
    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerOwnerFrame.contentWindow;
          child.__serviceWorkerOwnerReadyScope = "pending";
          child.navigator.serviceWorker.ready.then(registration => {
            child.__serviceWorkerOwnerReadyScope = registration.scope;
          });
        })()
        "#,
    )
    .expect("child ready observer should install");
    let ready_scope = url::Url::parse("https://service-worker-owner.test/").unwrap();
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_ready(crate::types::ServiceWorkerReadyCompletion {
            request_id: child_ready_request_id,
            document_owner: child_ready_owner.window_document_owner(),
            registration:
                crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot::active_for_binding_test(
                ready_scope.clone(),
                url::Url::parse("https://service-worker-owner.test/ready-worker.js").unwrap(),
            ),
        })
        .expect("owner-bound child ready completion should enter the typed Page source");
    run_page_service_worker_internal_tasks_until_request_consumed_for_test(
        &mut vm,
        &loader,
        PendingServiceWorkerInternalRequestForTest::Ready(child_ready_request_id),
        "owner-bound child ready completion",
    )
    .await;
    assert_eq!(
        vm.eval(
            "globalThis.__serviceWorkerOwnerFrame.contentWindow.__serviceWorkerOwnerReadyScope",
        )
        .expect("child ready result should evaluate"),
        ready_scope.as_str()
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_ready_owners_for_test()
            .into_iter()
            .any(|(_, owner)| owner.document_owner() == Some(main_owner)),
        "settling child ready must leave the main ready request pending"
    );

    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerOwnerFrame.contentWindow;
          child.__serviceWorkerOwnerRegisterResult = "pending";
          child.navigator.serviceWorker.register(
            "https://service-worker-owner.test/worker.js"
          ).then(
            () => { child.__serviceWorkerOwnerRegisterResult = "resolved"; },
            error => {
              child.__serviceWorkerOwnerRegisterResult =
                error.name + ":" + error.message;
            }
          );
        })();
        "register-pending"
        "#,
    )
    .expect("child service worker register should schedule");
    let (request_id, register_owner) = vm
        ._context_host
        .borrow()
        .pending_service_worker_register_owners_for_test()
        .into_iter()
        .next()
        .expect("pending child service worker register");
    assert_eq!(register_owner.document_owner(), Some(child_owner));
    assert!(register_owner.dispatch_scope().child_window().is_some());

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_register(crate::types::ServiceWorkerRegisterCompletion {
            request_id,
            document_owner: register_owner.window_document_owner(),
            result: Err(
                crate::service_worker_runtime::ServiceWorkerRegistrationError::type_error(
                    "forced owner completion",
                ),
            ),
        })
        .expect("owner-bound child register failure should enter the typed Page source");
    run_page_service_worker_internal_tasks_until_request_consumed_for_test(
        &mut vm,
        &loader,
        PendingServiceWorkerInternalRequestForTest::Register(request_id),
        "owner-bound child register failure",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"
            globalThis.__serviceWorkerOwnerFrame.contentWindow
              .__serviceWorkerOwnerRegisterResult
            "#,
        )
        .expect("child register result should evaluate"),
        "TypeError:forced owner completion"
    );
    assert_eq!(
        vm.eval("typeof globalThis.__serviceWorkerOwnerRegisterResult")
            .expect("main register result should evaluate"),
        "undefined",
        "child completion must not settle through the top-level container realm"
    );

    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerOwnerFrame.contentWindow;
          child.__serviceWorkerOwnerRegistrationScope = "pending";
          child.navigator.serviceWorker.register(
            "https://service-worker-owner.test/bound-worker.js"
          ).then(registration => {
            child.__serviceWorkerOwnerRegistration = registration;
            child.__serviceWorkerOwnerRegistrationScope = registration.scope;
          });
        })();
        "binding-register-pending"
        "#,
    )
    .expect("child registration binding should schedule");
    let (binding_request_id, binding_owner) = vm
        ._context_host
        .borrow()
        .pending_service_worker_register_owners_for_test()
        .into_iter()
        .next()
        .expect("pending child registration binding");
    assert_eq!(binding_owner.document_owner(), Some(child_owner));
    let registration_scope = url::Url::parse("https://service-worker-owner.test/").unwrap();
    let registration_snapshot =
        crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot::active_for_binding_test(
            registration_scope.clone(),
            url::Url::parse("https://service-worker-owner.test/bound-worker.js").unwrap(),
        );
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_register(crate::types::ServiceWorkerRegisterCompletion {
            request_id: binding_request_id,
            document_owner: binding_owner.window_document_owner(),
            result: Ok(registration_snapshot.clone()),
        })
        .expect("owner-bound child registration should enter the typed Page source");
    run_page_service_worker_internal_tasks_until_request_consumed_for_test(
        &mut vm,
        &loader,
        PendingServiceWorkerInternalRequestForTest::Register(binding_request_id),
        "owner-bound child registration",
    )
    .await;
    assert_eq!(
        vm.eval(
            "globalThis.__serviceWorkerOwnerFrame.contentWindow.__serviceWorkerOwnerRegistrationScope",
        )
        .expect("child registration scope should evaluate"),
        registration_scope.as_str()
    );
    let (_, _, lifecycle_storage_key) = vm
        ._context_host
        .borrow()
        .service_worker_registration_watchers_for_test()
        .into_iter()
        .find(|(owner, _, _)| owner.document_owner() == Some(child_owner))
        .expect("registration lifecycle watcher must retain the child document owner");
    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerOwnerFrame.contentWindow;
          child.__serviceWorkerOwnerLifecycle = "pending";
          child.__serviceWorkerOwnerRegistration.addEventListener(
            "updatefound",
            () => { child.__serviceWorkerOwnerLifecycle = "child:updatefound"; }
          );
        })()
        "#,
    )
    .expect("child lifecycle listener should install");
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_lifecycle(crate::types::ServiceWorkerLifecycleNotification {
            document_owner: binding_owner.window_document_owner(),
            storage_key: "wrong-partition".to_owned(),
            registration: registration_snapshot.clone(),
            events: vec![crate::types::ServiceWorkerLifecycleClientEvent::UpdateFound],
        })
        .expect("wrong-partition lifecycle completion should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "wrong-partition lifecycle completion",
    )
    .await;
    assert_eq!(
        vm.eval(
            "globalThis.__serviceWorkerOwnerFrame.contentWindow.__serviceWorkerOwnerLifecycle",
        )
        .expect("child lifecycle result should evaluate"),
        "pending",
        "lifecycle notification from another storage partition must not fan out"
    );
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_lifecycle(crate::types::ServiceWorkerLifecycleNotification {
            document_owner: binding_owner.window_document_owner(),
            storage_key: lifecycle_storage_key,
            registration: registration_snapshot,
            events: vec![crate::types::ServiceWorkerLifecycleClientEvent::UpdateFound],
        })
        .expect("owner-bound lifecycle completion should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "owner-bound lifecycle completion",
    )
    .await;
    assert_eq!(
        vm.eval(
            "globalThis.__serviceWorkerOwnerFrame.contentWindow.__serviceWorkerOwnerLifecycle",
        )
        .expect("child lifecycle result should evaluate"),
        "child:updatefound",
        "lifecycle notification must dispatch through the child owner scope"
    );
    assert_eq!(
        vm.eval("typeof globalThis.__serviceWorkerOwnerLifecycle")
            .expect("main lifecycle result should evaluate"),
        "undefined"
    );

    vm.eval(
        r#"
        globalThis.__serviceWorkerOwnerFrame.contentWindow.navigator.serviceWorker
          .register("https://service-worker-owner.test/stale-worker.js");
        globalThis.__serviceWorkerOwnerFrame.srcdoc = "<p>replacement child</p>";
        "stale-register-pending"
        "#,
    )
    .expect("old child register and replacement should schedule");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_service_worker_register_owners_for_test()
            .len(),
        1
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child replacement must commit before stale wrapper reuse is tested",
    )
    .await;
    let replacement_child_owner = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("replacement child browsing context");
        host.current_child_document_task_owner(child_handle)
            .expect("replacement child document owner")
    };
    assert_ne!(replacement_child_owner, child_owner);
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_register_owners_for_test()
            .is_empty(),
        "child navigation must retire pending register work from the old document"
    );
    assert!(
        vm._context_host
            .borrow()
            .service_worker_registration_watchers_for_test()
            .into_iter()
            .all(|(owner, _, _)| owner.document_owner() != Some(child_owner)),
        "child navigation must retire old registration lifecycle watchers"
    );

    vm.eval(
        r#"
        globalThis.__serviceWorkerOwnerFrame.contentWindow.navigator.serviceWorker
          .register("https://service-worker-owner.test/current-worker.js");
        "current-register-pending"
        "#,
    )
    .expect("replacement child register should schedule");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_service_worker_register_owners_for_test()
            .into_iter()
            .map(|(_, owner)| owner.document_owner())
            .collect::<Vec<_>>(),
        vec![Some(replacement_child_owner)]
    );

    vm.eval("document.open(); 'replaced'")
        .expect("main replacement should retire main and descendant service worker owners");
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_register_owners_for_test()
            .is_empty(),
        "detaching the child document must retire its pending register resolver"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_service_worker_ready_owners_for_test()
            .into_iter()
            .all(|(_, owner)| {
                owner.document_owner() != Some(main_owner)
                    && owner.document_owner() != Some(child_owner)
                    && owner.document_owner() != Some(replacement_child_owner)
            }),
        "replacement must retire old main and child ready resolvers"
    );
    assert!(
        vm._context_host
            .borrow()
            .service_worker_registration_watchers_for_test()
            .into_iter()
            .all(|(owner, _, _)| {
                owner.document_owner() != Some(main_owner)
                    && owner.document_owner() != Some(child_owner)
            }),
        "replacement must retire lifecycle watchers bound to old documents"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn main_document_replacement_rebinds_service_worker_lifecycle_watcher() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://service-worker-main-rebind.test/page.html",
        &loader,
    );
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("initial main document owner");
    vm.eval(
        r#"
        globalThis.__mainServiceWorkerRegistration = null;
        navigator.serviceWorker.register(
          "https://service-worker-main-rebind.test/worker.js"
        ).then(registration => {
          globalThis.__mainServiceWorkerRegistration = registration;
        });
        "register-pending"
        "#,
    )
    .expect("main service worker register should schedule");
    let (request_id, register_owner) = vm
        ._context_host
        .borrow()
        .pending_service_worker_register_owners_for_test()
        .into_iter()
        .next()
        .expect("pending main service worker register");
    assert_eq!(register_owner.document_owner(), Some(retired_owner));
    let registration_scope = url::Url::parse("https://service-worker-main-rebind.test/").unwrap();
    let registration_snapshot =
        crate::service_worker_runtime::ServiceWorkerRegistrationSnapshot::active_for_binding_test(
            registration_scope,
            url::Url::parse("https://service-worker-main-rebind.test/worker.js").unwrap(),
        );
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_register(crate::types::ServiceWorkerRegisterCompletion {
            request_id,
            document_owner: register_owner.window_document_owner(),
            result: Ok(registration_snapshot.clone()),
        })
        .expect("main service worker registration should enter the typed Page source");
    run_page_service_worker_internal_tasks_until_request_consumed_for_test(
        &mut vm,
        &loader,
        PendingServiceWorkerInternalRequestForTest::Register(request_id),
        "main service worker registration",
    )
    .await;
    vm.eval(
        r#"
        globalThis.__mainServiceWorkerLifecycle = "pending";
        globalThis.__mainServiceWorkerRegistration.addEventListener(
          "updatefound",
          () => { globalThis.__mainServiceWorkerLifecycle = "updatefound"; }
        );
        "listener-ready"
        "#,
    )
    .expect("main lifecycle listener should install");

    vm.eval("document.open(); 'replaced'")
        .expect("main document replacement should commit");
    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_ne!(current_owner, retired_owner);
    let (rebound_owner, _, storage_key) = vm
        ._context_host
        .borrow()
        .service_worker_registration_watchers_for_test()
        .into_iter()
        .find(|(owner, _, _)| owner.document_owner() == Some(current_owner))
        .expect("same-Window registration watcher must rebind to the replacement document");

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_lifecycle(crate::types::ServiceWorkerLifecycleNotification {
            document_owner: register_owner.window_document_owner(),
            storage_key: storage_key.clone(),
            registration: registration_snapshot.clone(),
            events: vec![crate::types::ServiceWorkerLifecycleClientEvent::UpdateFound],
        })
        .expect("retired-generation lifecycle completion should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "retired-generation lifecycle completion",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__mainServiceWorkerLifecycle")
            .expect("main lifecycle state should evaluate"),
        "pending",
        "the old transport generation must not dispatch through the rebound watcher"
    );

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_lifecycle(crate::types::ServiceWorkerLifecycleNotification {
            document_owner: rebound_owner.window_document_owner(),
            storage_key,
            registration: registration_snapshot,
            events: vec![crate::types::ServiceWorkerLifecycleClientEvent::UpdateFound],
        })
        .expect("rebound lifecycle completion should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "rebound lifecycle completion",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__mainServiceWorkerLifecycle")
            .expect("main rebound lifecycle state should evaluate"),
        "updatefound"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn service_worker_controller_change_targets_exact_child_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://service-worker-client-owner.test/page.html",
        &loader,
    );
    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          (document.body || document.documentElement || document).appendChild(frame);
          globalThis.__serviceWorkerClientOwnerFrame = frame;
        })();
        "frame-ready"
        "#,
    )
    .expect("service worker client owner frame should schedule");
    assert_initial_about_blank_child_completed_through_page_for_test(
        &mut vm,
        &loader,
        "service worker client owner setup",
    )
    .await;

    let child_handle = {
        let host = vm._context_host.borrow();
        host.child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("child browsing context")
    };
    let child_client_id = vm
        ._context_host
        .borrow_mut()
        .register_or_update_service_worker_child_client(child_handle)
        .expect("child service worker client should register");
    let child_owner = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("current child owner snapshot");
        assert_eq!(
            snapshot.settings.service_worker_client_id,
            Some(child_client_id)
        );
        host.current_child_document_task_owner(child_handle)
            .expect("current child document owner")
    };
    let child_target = crate::types::ServiceWorkerWindowClientTarget {
        client_id: child_client_id,
        document_owner: crate::native_bridge::WindowDocumentOwner::Frame(child_owner),
    };
    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerClientOwnerFrame.contentWindow;
          globalThis.__serviceWorkerMainControllerChangeCount = 0;
          child.__serviceWorkerChildControllerChangeCount = 0;
          navigator.serviceWorker.oncontrollerchange = () => {
            globalThis.__serviceWorkerMainControllerChangeCount++;
          };
          child.navigator.serviceWorker.oncontrollerchange = () => {
            child.__serviceWorkerChildControllerChangeCount++;
          };
        })();
        "listeners-ready"
        "#,
    )
    .expect("service worker client owner listeners should install");

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_controller_change(
            crate::types::ServiceWorkerControllerChangeCompletion {
                target: child_target,
            },
        )
        .expect("child controllerchange should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(&mut vm, &loader, "child controllerchange")
        .await;
    assert_eq!(
        vm.eval(
            "String(globalThis.__serviceWorkerClientOwnerFrame.contentWindow.__serviceWorkerChildControllerChangeCount)",
        )
        .expect("child controllerchange count should evaluate"),
        "1"
    );
    assert_eq!(
        vm.eval("String(globalThis.__serviceWorkerMainControllerChangeCount)")
            .expect("main controllerchange count should evaluate"),
        "0"
    );

    vm.eval(
        r#"
        globalThis.__serviceWorkerClientOwnerFrame.srcdoc = "<p>replacement</p>";
        "replacement-pending"
        "#,
    )
    .expect("child replacement should schedule");
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child replacement must commit before stale client completion",
    )
    .await;
    let (replacement_owner, replacement_client_id) = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("replacement child owner snapshot");
        (
            host.current_child_document_task_owner(child_handle)
                .expect("replacement child document owner"),
            snapshot
                .settings
                .service_worker_client_id
                .expect("replacement child service worker client"),
        )
    };
    assert_ne!(replacement_owner, child_owner);
    assert_eq!(
        replacement_client_id, child_client_id,
        "srcdoc replacement currently reuses the browser client id, so document epoch must carry currentness"
    );
    vm.eval(
        r#"
        (() => {
          const child = globalThis.__serviceWorkerClientOwnerFrame.contentWindow;
          child.__serviceWorkerReplacementControllerChangeCount = 0;
          child.navigator.serviceWorker.oncontrollerchange = () => {
            child.__serviceWorkerReplacementControllerChangeCount++;
          };
        })();
        "replacement-listener-ready"
        "#,
    )
    .expect("replacement child controllerchange listener should install");

    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_controller_change(
            crate::types::ServiceWorkerControllerChangeCompletion {
                target: child_target,
            },
        )
        .expect("stale child controllerchange should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "stale child controllerchange",
    )
    .await;
    assert_eq!(
        vm.eval(
            "String(globalThis.__serviceWorkerClientOwnerFrame.contentWindow.__serviceWorkerReplacementControllerChangeCount)",
        )
        .expect("replacement controllerchange count should evaluate"),
        "0",
        "retired child client target must not dispatch into the replacement document"
    );

    let replacement_target = crate::types::ServiceWorkerWindowClientTarget {
        client_id: replacement_client_id,
        document_owner: crate::native_bridge::WindowDocumentOwner::Frame(replacement_owner),
    };
    vm.current_service_worker_task_sender_for_test()
        .send_service_worker_controller_change(
            crate::types::ServiceWorkerControllerChangeCompletion {
                target: replacement_target,
            },
        )
        .expect("current child controllerchange should enter the typed Page source");
    run_page_service_worker_internal_task_for_test(
        &mut vm,
        &loader,
        "current child controllerchange",
    )
    .await;
    assert_eq!(
        vm.eval(
            "String(globalThis.__serviceWorkerClientOwnerFrame.contentWindow.__serviceWorkerReplacementControllerChangeCount)",
        )
        .expect("current replacement controllerchange count should evaluate"),
        "1"
    );
}

fn new_storage_test_vm_with_loader_and_resource_completion_queue(
    url: &str,
    loader: &ResourceRequestClient,
) -> (
    StandaloneScriptVmHarness,
    RendererResourceCompletionTestHarness,
) {
    let resource_completion_queue = RendererResourceCompletionTestHarness::new();
    let vm = new_storage_test_vm_with_resource_mode(
        url,
        resource_completion_queue.sender(),
        StandaloneStorageResourceMode::Networked(loader),
    );
    (vm, resource_completion_queue)
}

fn new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
    url: &str,
    loader: &ResourceRequestClient,
) -> (
    crate::runtime::PageVmTaskExecutorTestHarness,
    crate::runtime::RendererBrowserContextRuntimeOwner,
) {
    let browser_context_owner = crate::runtime::RendererBrowserContextRuntime::new();
    let browser_context_runtime = browser_context_owner.handle();
    let storage_manager = shared_indexed_db_test_manager();
    let mut page = crate::runtime::PageVmTaskExecutorTestHarness::new_with_browser_context_runtime(
        url::Url::parse(url).expect("service worker Page test URL"),
        loader,
        browser_context_runtime.clone(),
    );
    page.set_indexed_db_manager(Some(crate::downgrade_indexed_db_manager(&storage_manager)));
    page.set_storage_bucket_store(
        crate::new_shared_storage_bucket_store_with_indexed_db_manager(&storage_manager),
    );
    install_test_trusted_key_dispatcher(&mut page);
    (page, browser_context_owner)
}

fn new_storage_test_vm_with_loader(
    url: &str,
    loader: &ResourceRequestClient,
) -> StandaloneScriptVmHarness {
    let resource_completion_queue = RendererResourceCompletionTestHarness::new();
    new_storage_test_vm_with_resource_mode(
        url,
        resource_completion_queue.sender(),
        StandaloneStorageResourceMode::Networked(loader),
    )
}

fn install_linked_stylesheet_for_test(
    vm: &mut ScriptVm,
    owner: DomHandle,
    request_url: url::Url,
    source: crate::style_engine::StyloStylesheetSource,
) {
    let css_text = source.serialized_css_text();
    let prepared = vm
        ._context_host
        .borrow_mut()
        .prepare_linked_stylesheet_resource(
            owner,
            &css_text,
            source.base_url().clone(),
            source.sheet_url().clone(),
            source.origin_clean(),
        )
        .expect("linked stylesheet test resource should be admitted");
    vm._context_host.borrow_mut().install_linked_stylesheet(
        crate::document_runtime::InstallLinkedStylesheet::from_prepared(
            owner,
            request_url,
            prepared,
        ),
    );
    vm.apply_pending_stylesheet_source_css_projections();
}

async fn run_child_document_lifecycle_and_host_load_for_test(vm: &mut ScriptVm, message: &str) {
    while vm
        .run_child_realm_materialization_body_for_test()
        .expect("child realm materialization prerequisite should succeed")
    {
        // Only consecutive materialization tasks at the stable family head
        // belong here. Never jump over an earlier DocumentScriptReady task.
    }
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "{message}: DocumentLifecycle should make the installed document interactive"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "{message}: DocumentLifecycle should dispatch DOMContentLoaded for the installed document"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "{message}: DocumentLifecycle should apply complete before load delivery"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "{message}: HostLoad should deliver window/iframe load after complete"
    );
}

async fn assert_initial_about_blank_child_completed_synchronously_for_test(
    vm: &mut ScriptVm,
    message: &str,
) {
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "{message}: initial about:blank should already be installed without a NavigationCommit turn"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentLifecycle
        )
        .await,
        "{message}: initial about:blank should already be complete without a lifecycle turn"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "{message}: synchronous initial about:blank delivery must not leave HostLoad work"
    );
}

fn materialize_single_child_default_realm_for_test(vm: &mut ScriptVm, message: &str) -> i64 {
    let child_handle = {
        let host = vm._context_host.borrow();
        let child_handles = host.child_browsing_context_handles_in_document_order();
        assert_eq!(
            child_handles.len(),
            1,
            "{message}: expected exactly one child browsing context"
        );
        child_handles[0]
    };
    assert_eq!(
        vm.eval("String(document.querySelector('iframe').contentWindow !== null)")
            .unwrap_or_else(|error| panic!("{message}: child Window exposure failed: {error}")),
        "true",
        "{message}: child Window exposure should prebootstrap its default realm"
    );
    assert!(
        vm.run_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed"),
        "{message}: Window exposure should schedule one realm-materialization owner turn"
    );
    vm.live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| {
            vm.child_frame_realm_store
                .get(&realm.context_id)
                .is_some_and(|record| record.child_handle == child_handle)
        })
        .map(|realm| realm.context_id)
        .unwrap_or_else(|| panic!("{message}: child default realm should be materialized"))
}

fn current_single_child_document_owner_for_test(
    vm: &ScriptVm,
    message: &str,
) -> crate::frame_owner_model::FrameDocumentTaskOwner {
    let host = vm._context_host.borrow();
    let child_handles = host.child_browsing_context_handles_in_document_order();
    assert_eq!(
        child_handles.len(),
        1,
        "{message}: expected exactly one child browsing context"
    );
    host.current_child_document_task_owner(child_handles[0])
        .unwrap_or_else(|| panic!("{message}: expected a current child document owner"))
}

async fn run_child_navigation_commit_and_host_load_for_test(vm: &mut ScriptVm, message: &str) {
    loop {
        assert!(
            vm.run_child_frame_task_source_once_for_test(
                ChildFrameSemanticTurnKind::NavigationCommit,
            )
            .await,
            "{message}: each exact navigation reservation must have a stable source task"
        );
        if !vm.has_pending_child_navigation_commit_for_test() {
            break;
        }
        // A replacement may leave an older generation at the stable FIFO
        // head. Production settles it as one stale owner turn and publishes a
        // natural continuation for the still-reserved current generation.
    }
    for _ in 0..2 {
        if !vm.has_pending_child_frame_realm_materialization() {
            break;
        }
        assert!(
            vm.run_child_realm_materialization_body_for_test()
                .expect("child realm owner turn should succeed"),
            "{message}: each pending child realm must have one durable typed task"
        );
    }
    assert!(
        !vm.has_pending_child_frame_realm_materialization(),
        "{message}: NavigationCommit must resolve any stale and current realm tasks before lifecycle"
    );
    run_child_document_lifecycle_and_host_load_for_test(vm, message).await;
}

/// Settle earlier ChildFrameTask family entries, then execute the typed
/// modulepreload event body. Selected-task completion, owner-scheduler
/// liveness, and cross-source fairness are covered by PageVm tests.
async fn run_child_modulepreload_event_after_predecessors_for_test(
    vm: &mut ScriptVm,
    message: &str,
) -> Vec<ChildFrameSemanticTurnKind> {
    let mut predecessors = Vec::new();
    for _ in 0..8 {
        if vm.run_child_modulepreload_event_action_body_for_test() {
            return predecessors;
        }
        let Some(source) = vm.run_next_child_frame_semantic_turn_for_test().await else {
            assert!(
                vm.run_child_modulepreload_event_action_body_for_test(),
                "{message}: typed event remained blocked after stale predecessor readiness was pruned"
            );
            return predecessors;
        };
        predecessors.push(source);
    }
    panic!("{message}: child predecessor sequence did not converge: {predecessors:?}")
}

fn drain_pre_domcontentloaded_non_script_page_tasks_for_test(vm: &mut ScriptVm) -> usize {
    vm.drain_pre_domcontentloaded_content_security_policy_violation_tasks_for_test()
}

impl ScriptVm {
    pub(crate) fn drain_pre_domcontentloaded_content_security_policy_violation_tasks_for_test(
        &mut self,
    ) -> usize {
        let mut task_count = 0;
        while let Some(action) = self
            .document_runtime
            .pop_parser_owned_pre_domcontentloaded_action()
        {
            let DocumentProcessingAction::PostParsePageOwnedWork(work) = action else {
                panic!("focused CSP task runner must not consume script work: {action:?}");
            };
            let PostParsePageOwnedWork::Lifecycle(work) = *work else {
                panic!("focused CSP task runner must not consume document script work");
            };
            assert!(
                matches!(
                    work.as_ref(),
                    PostParseLifecycleWork::DispatchContentSecurityPolicyViolation(_)
                ),
                "focused CSP task runner must not consume unrelated lifecycle work: {work:?}"
            );
            self.execute_post_parse_lifecycle_work_best_effort(*work)
                .expect("focused pre-DCL CSP page task should run");
            task_count += 1;
        }
        task_count
    }
}
trait StoragePageTaskExecutorTestWaitExt {
    fn eval_after_selected_page_tasks(&mut self, source: &str) -> anyhow::Result<String>;
}

impl StoragePageTaskExecutorTestWaitExt for crate::runtime::PageVmTaskExecutorTestHarness {
    /// Drain stable Page work through the production selected-task dispatcher,
    /// then read the final probe without manufacturing another checkpoint.
    ///
    /// Every storage Promise reaction needed to produce the final value must
    /// already have run in the exact selected task that settled it.
    fn eval_after_selected_page_tasks(&mut self, source: &str) -> anyhow::Result<String> {
        anyhow::ensure!(
            tokio::runtime::Handle::try_current().is_err(),
            "synchronous storage Page fixture cannot run inside an existing Tokio runtime"
        );
        let loader =
            ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("storage Page task-executor runtime should build");
        runtime.block_on(async {
            for step in 0..4096 {
                if self
                    .run_one_oldest_ready_page_task_executor_turn(&loader)
                    .await?
                {
                    continue;
                }
                if self.has_pending_opfs_tasks() {
                    let arrived = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        self.wait_for_task_executor_work_arrival(),
                    )
                    .await
                    .unwrap_or(false);
                    anyhow::ensure!(
                        arrived,
                        "storage Page task executor timed out with pending OPFS work at step {step}"
                    );
                    continue;
                }
                return Ok(());
            }
            anyhow::bail!("storage Page task executor exceeded its bounded 4096-turn budget")
        })?;
        self.eval_without_microtask_checkpoint_for_test(source)
    }
}

/// Wait for one low-level network terminal and apply it through the
/// production Page resource owner.
///
/// Tests use this helper only when they intentionally observe the boundary
/// between a resource terminal and the later typed Page task it publishes.
/// The arrival wake is not itself progress and never applies the terminal
/// directly to `ScriptVm`.
async fn wait_for_one_page_resource_completion_executor_test_turn(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    context: &str,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if page
            .apply_one_page_resource_terminal_owner_admission()
            .unwrap_or_else(|error| panic!("{context}: resource owner turn failed: {error:#}"))
        {
            return;
        }
        let arrived = tokio::time::timeout_at(deadline, page.wait_for_task_executor_work_arrival())
            .await
            .unwrap_or_else(|_| panic!("{context}: resource terminal did not arrive"));
        assert!(
            arrived,
            "{context}: resource completion route closed before publishing its terminal"
        );
    }
}

/// Wait until an image response or decode completion has published the exact
/// DOM-manipulation task. A successful raster response first enters the
/// bounded decode worker, so the network terminal alone is not the event
/// readiness boundary.
async fn wait_for_image_load_event_executor_test_task(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    context: &str,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if page.has_ready_dom_manipulation_family_for_test(
            PageDomManipulationTestFamily::ImageLoadEvent,
        ) {
            return;
        }
        let arrived = tokio::time::timeout_at(deadline, page.wait_for_task_executor_work_arrival())
            .await
            .unwrap_or_else(|_| panic!("{context}: image event task did not arrive"));
        assert!(
            arrived,
            "{context}: image event route closed before publishing its task"
        );
    }
}

/// Wait for one Networking resource terminal and execute the complete Page
/// task through the production selected-task dispatcher.
///
/// Unlike `wait_for_one_page_resource_completion_executor_test_turn`, this
/// helper is for end-to-end Page workflows where the terminal may enter V8 or
/// dispatch an event and therefore must receive the production task-end
/// checkpoint and follow-up reconciliation.
async fn wait_for_one_page_resource_completion_selected_task_executor_test_turn(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    context: &str,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if page
            .run_one_page_resource_completion_selected_task_executor_turn(loader)
            .await
            .unwrap_or_else(|error| {
                panic!("{context}: selected resource completion task failed: {error:#}")
            })
        {
            return;
        }
        let arrived = tokio::time::timeout_at(deadline, page.wait_for_task_executor_work_arrival())
            .await
            .unwrap_or_else(|_| panic!("{context}: resource terminal did not arrive"));
        assert!(
            arrived,
            "{context}: resource completion route closed before publishing its terminal"
        );
    }
}

/// Wait for one concrete production Page task to become runnable, then
/// execute it through the selected-task dispatcher.
///
/// A Page fixture may observe an owner wake before the corresponding async
/// IndexedDB terminal is visible, or may only have a real timer deadline.
/// The wake/deadline is therefore only a reason to re-check the stable
/// sources; it is never treated as progress by itself.
async fn wait_for_one_selected_page_task_executor_test_turn(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if page
            .run_one_oldest_ready_page_task_executor_turn(loader)
            .await?
        {
            return Ok(());
        }

        let timer_deadline = page
            .ms_to_next_timeout()
            .map(|ms| tokio::time::Instant::now() + std::time::Duration::from_millis(ms));
        let arrived = tokio::select! {
            arrived = page.wait_for_task_executor_work_arrival() => Some(arrived),
            _ = async {
                match timer_deadline {
                    Some(timer_deadline) => tokio::time::sleep_until(timer_deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => None,
            _ = tokio::time::sleep_until(deadline) => {
                anyhow::bail!("Page task executor timed out waiting for concrete work")
            }
        };
        if let Some(arrived) = arrived {
            anyhow::ensure!(
                arrived,
                "Page task executor work routes closed before concrete work arrived"
            );
        }
    }
}

async fn advance_page_task_executor_until_eval_equals(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    expression: &str,
    expected: &str,
    context: &str,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let value = page
            .eval(expression)
            .expect("wait-driver predicate should evaluate");
        if value == expected {
            return;
        }
        let last = value;
        tokio::time::timeout_at(
            deadline,
            wait_for_one_selected_page_task_executor_test_turn(page, loader),
        )
        .await
        .unwrap_or_else(|_| {
            panic!(
                "{context}: timed out waiting for task-executor work; \
                 expected {expression} to evaluate to {expected}, last={last:?}"
            )
        })
        .unwrap_or_else(|error| {
            panic!(
                "{context}: Page task-executor advance failed while waiting for {expression} \
                 to evaluate to {expected}, last={last:?}: {error}"
            )
        });
    }
}

fn indexed_db_test_root(label: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "moli-renderer-v8-indexeddb-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn indexed_db_origin_file(root: &std::path::Path, origin: &str) -> std::path::PathBuf {
    let mut encoded = String::with_capacity(origin.len() * 2);
    for byte in origin.as_bytes() {
        use std::fmt::Write;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    if encoded.len() > 180 {
        encoded = moli_crypto::sha256_hex(origin.as_bytes());
        encoded.insert_str(0, "h-");
    }
    root.join(format!("{encoded}.json"))
}

#[test]
fn isolated_world_bridge_ref_is_released_with_script_vm() {
    let mut vm = new_storage_test_vm("https://example.test/");
    let context_host = vm.context_host_weak_for_test();

    vm.ensure_isolated_world_for_owner(None, "bridge-ref-regression", false)
        .expect("isolated world should be created");
    assert!(
        context_host.upgrade().is_some(),
        "context host should stay alive while the ScriptVm owns its contexts"
    );

    drop(vm);
    assert!(
        context_host.upgrade().is_none(),
        "dropping ScriptVm should release every V8 bridge Rc ref-count"
    );
}

#[test]
fn promise_reject_context_slot_does_not_retain_context_host_after_script_vm_drop() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><p>promise</p>");
    let context_host = vm.context_host_weak_for_test();

    let retained_slot = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
            scope
                .get_current_context()
                .get_slot::<super::runtime_bindings::PromiseRejectDispatchSlot>()
                .ok_or_else(|| anyhow::anyhow!("promise reject dispatch slot missing"))
        })
        .expect("promise reject dispatch slot should be installed");
    assert!(
        context_host.upgrade().is_some(),
        "context host should stay alive while ScriptVm owns the page context"
    );

    drop(vm);
    assert!(
        context_host.upgrade().is_none(),
        "retaining the V8 context slot must not keep the page context host alive"
    );
    assert!(
        retained_slot.host_weak.upgrade().is_none(),
        "promise rejection slot should only keep a weak host reference"
    );
}

#[test]
fn dom_wrapper_expando_survives_renderer_document_isolate_garbage_collection() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-expando-retention.test/",
        "<!doctype html><main></main>",
    );

    let seeded = vm
        .eval(
            r#"
(() => {
  document.body.__array = [];
  document.body.__object = { ok: true };
  return "seeded";
})()
"#,
        )
        .expect("wrapper expando setup should evaluate");
    assert_eq!(seeded, "seeded");

    vm.collect_renderer_document_isolate_garbage()
        .expect("document isolate garbage collection should run");

    let retained = vm
        .eval(
            r#"
(() => {
  return Array.isArray(document.body.__array) && document.body.__object.ok
    ? "retained"
    : "missing";
})()
"#,
        )
        .expect("wrapper expando probe should evaluate");
    assert_eq!(retained, "retained");
}

#[test]
fn renderer_document_isolate_memory_pressure_escalates_at_one_third_of_heap_limit() {
    assert!(!renderer_document_isolate_critical_pressure_required(0, 0));
    assert!(!renderer_document_isolate_critical_pressure_required(
        33, 100
    ));
    assert!(renderer_document_isolate_critical_pressure_required(
        34, 100
    ));
    assert!(renderer_document_isolate_critical_pressure_required(
        100, 100
    ));
}

#[test]
fn context_wrapper_cache_is_cleared_on_script_vm_teardown() {
    let mut vm = new_parsed_test_vm(
        "https://wrapper-cache-retention.test/",
        "<!doctype html><main></main>",
    );

    let retained_cache = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
            Ok(crate::native_bridge::identity::retain_context_wrapper_cache_for_test(scope))
        })
        .expect("wrapper cache should be retainable for regression testing");

    let baseline = vm
        .eval(
            r#"
(() => {
  void document.body;
  return "baseline";
})()
"#,
        )
        .expect("wrapper cache baseline should evaluate");
    assert_eq!(baseline, "baseline");

    let created = vm
        .eval(
            r#"
(() => {
  for (let index = 0; index < 64; index += 1) {
    document.createElement("span");
  }
  return "created";
})()
"#,
        )
        .expect("wrapper cache setup should evaluate");
    assert_eq!(created, "created");
    assert!(
        retained_cache.wrapper_entry_count() >= 64,
        "transient DOM wrappers should populate the per-context wrapper cache"
    );

    drop(vm);
    assert_eq!(
        retained_cache.wrapper_entry_count(),
        0,
        "page context teardown must clear strong wrapper cache entries before contexts are dropped"
    );
}

#[tokio::test]
async fn script_vm_drop_unregisters_child_service_worker_window_clients() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://child-client-teardown.test/page.html",
            &loader,
        );

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  return "created";
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "created");
    while vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("child frame Page task should apply")
    {}
    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handle_by_index(0)
        .expect("test iframe should have a child browsing context");
    vm._context_host
        .borrow_mut()
        .register_or_update_service_worker_child_client(child_handle)
        .expect("child frame should register a service worker window client");
    let active_diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(
        active_diagnostics.live_client_count, 2,
        "top-level page and child frame should both be live window clients before teardown"
    );

    drop(vm);
    let after_drop_diagnostics = browser_context_runtime
        .service_worker_runtime()
        .diagnostics_snapshot();
    assert_eq!(
        after_drop_diagnostics.live_client_count, 0,
        "dropping ScriptVm should unregister top-level and child window clients"
    );
}

#[test]
fn script_vm_page_context_teardown_is_idempotent() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, browser_context_runtime) =
        new_service_worker_page_test_vm_with_loader_and_browser_context_runtime(
            "https://teardown-idempotent.test/page.html",
            &loader,
        );
    assert_eq!(
        browser_context_runtime
            .service_worker_runtime()
            .diagnostics_snapshot()
            .live_client_count,
        1,
        "new ScriptVm should register its top-level service worker window client"
    );

    vm.close_page_context_resources_for_context_teardown();
    assert_eq!(
        browser_context_runtime
            .service_worker_runtime()
            .diagnostics_snapshot()
            .live_client_count,
        0,
        "first context teardown should unregister the top-level window client"
    );

    vm.close_page_context_resources_for_context_teardown();
    drop(vm);
    assert_eq!(
        browser_context_runtime
            .service_worker_runtime()
            .diagnostics_snapshot()
            .live_client_count,
        0,
        "repeated teardown and ScriptVm drop should not touch already closed page resources"
    );
}

#[test]
fn page_context_teardown_releases_all_context_owned_v8_finalizers() {
    let mut vm = new_parsed_test_vm(
        "https://v8-finalizer-teardown.test/",
        "<!doctype html><body></body>",
    );

    let created = vm
        .eval(
            r#"
(() => {
  globalThis.__finalizerObjects = [];
  for (let index = 0; index < 32; index += 1) {
    const element = document.createElement("div");
    element.style.color = "red";

    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`.item-${index} { color: red; }`);
    sheet.cssRules[0].style.setProperty("color", "blue");

    const blob = new Blob([`payload-${index}`], { type: "text/plain" });
    globalThis.__finalizerObjects.push(element, sheet, blob);
  }
  globalThis.__finalizerPerformance = performance;
  performance.setResourceTimingBufferSize(150);
  return globalThis.__finalizerObjects.length;
})()
"#,
        )
        .expect("context-owned finalizer objects should evaluate");
    assert_eq!(created, "96");
    assert!(
        vm._context_host.borrow().v8_finalizers.len() >= 128,
        "CSS declaration/rule-tree and Blob objects should be tracked by the page context owner"
    );
    assert!(
        vm._context_host
            .borrow()
            .resource_timing_buffer_count_for_test()
            >= 1,
        "the top-level Performance buffer should be owned by the host registry"
    );

    vm.close_page_context_resources_for_context_teardown();
    assert_eq!(
        vm._context_host.borrow().v8_finalizers.len(),
        0,
        "page context teardown must reset every weak handle before isolate teardown"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .resource_timing_buffer_count_for_test(),
        0,
        "Performance finalization must remove host-side buffer state"
    );

    vm.close_page_context_resources_for_context_teardown();
    drop(vm);
}

#[tokio::test]
async fn child_default_context_inventory_keeps_initial_empty_lazy_until_commit() {
    let mut vm = new_storage_test_vm("https://child-default-lazy.test/");
    let initial_bridge_ref_count = vm._context_host.borrow().bridge_ref_count_for_test();
    let initial_native_contexts = vm
        .renderer_document_isolate_heap_usage()
        .expect("initial heap usage should be available")
        .number_of_native_contexts;

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "lazy-child-frame";
  body.appendChild(frame);
  return "created";
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "created");
    let child_handle = vm
        .document_runtime
        .get_element_by_id("lazy-child-frame")
        .expect("lazy child frame handle");

    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "plain child frame lifecycle",
    )
    .await;
    assert!(
        vm.child_frame_realm_store.len() == 0,
        "plain child frame lifecycle should not eagerly create CDP child default contexts"
    );
    assert!(
        !vm._context_host
            .borrow()
            .child_browsing_context_has_cached_snapshot_for_test(child_handle),
        "frame initialization must not represent the internal initial-empty Document as a navigation snapshot"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count,
        "plain child frame lifecycle should not retain a child default bridge ref"
    );
    assert_eq!(
        vm.renderer_document_isolate_heap_usage()
            .expect("post-child-lifecycle heap usage should be available")
            .number_of_native_contexts,
        initial_native_contexts,
        "plain child frame lifecycle should not create a V8 native context"
    );

    let realms = vm.live_child_default_runtime_realm_inventory();
    assert!(
        realms.is_empty(),
        "protocol inventory must not materialize an internal initial-empty child context"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count,
        "initial-empty inventory must not retain a child default bridge ref"
    );

    vm.eval(
        r#"
document.getElementById("lazy-child-frame").srcdoc =
  "<!doctype html><title>committed child</title>";
"queued"
"#,
    )
    .expect("committed child navigation should queue");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "committed lazy child document")
        .await;

    let realms = vm.live_child_default_runtime_realm_inventory();
    assert_eq!(
        realms.len(),
        1,
        "inventory may materialize the committed child default context"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count + 1,
        "the committed child default context should retain one bridge ref"
    );
}

#[test]
fn same_origin_child_window_uses_shared_public_surface_shape() {
    let mut vm = new_storage_test_vm("https://child-window-shared-surface.test/");

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.srcdoc = "<!doctype html><title>child</title>";
  (document.body || document.documentElement || document).appendChild(iframe);

  const main = window;
  const child = iframe.contentWindow;
  const descriptorShape = descriptor => {
    if (descriptor === undefined) {
      return null;
    }
    if ("value" in descriptor) {
      const callable = typeof descriptor.value === "function";
      return {
        kind: "data",
        type: typeof descriptor.value,
        writable: descriptor.writable,
        enumerable: descriptor.enumerable,
        configurable: descriptor.configurable,
        name: callable ? descriptor.value.name : null,
        length: callable ? descriptor.value.length : null
      };
    }
    return {
      kind: "accessor",
      getterName: descriptor.get?.name ?? null,
      getterLength: descriptor.get?.length ?? null,
      setterName: descriptor.set?.name ?? null,
      setterLength: descriptor.set?.length ?? null,
      enumerable: descriptor.enumerable,
      configurable: descriptor.configurable
    };
  };
  const isDynamicOrInternal = name =>
    /^(0|[1-9]\d*)$/.test(name) ||
    name.startsWith("__moli") ||
    name.startsWith("__lm");
  const names = new Set([
    ...Object.getOwnPropertyNames(main),
    ...Object.getOwnPropertyNames(child)
  ]);
  const differences = [];
  for (const name of names) {
    if (isDynamicOrInternal(name)) {
      continue;
    }
    const mainShape = descriptorShape(
      Object.getOwnPropertyDescriptor(main, name)
    );
    const childShape = descriptorShape(
      Object.getOwnPropertyDescriptor(child, name)
    );
    if (JSON.stringify(mainShape) !== JSON.stringify(childShape)) {
      differences.push(name);
    }
  }
  differences.sort();
  return JSON.stringify(differences);
})()
"#,
        )
        .expect("same-origin Window descriptor comparison should evaluate"),
        "[]"
    );
}

#[test]
fn child_window_indexed_access_materializes_only_the_requested_descendant_realm() {
    let mut vm = new_storage_test_vm("https://child-window-indexed-lazy.test/");
    let initial_native_contexts = vm
        .renderer_document_isolate_heap_usage()
        .expect("initial heap usage should be available")
        .number_of_native_contexts;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const outer = document.createElement("iframe");
  outer.id = "outer";
  (document.body || document.documentElement || document).appendChild(outer);

  const nested = outer.contentDocument.createElement("iframe");
  nested.id = "nested";
  (outer.contentDocument.body || outer.contentDocument.documentElement || outer.contentDocument)
    .appendChild(nested);
  globalThis.__indexedLazyOuter = outer;
  globalThis.__indexedLazyNested = nested;
  return "created";
})()
"#,
        )
        .expect("nested initial-empty frame setup should evaluate"),
        "created"
    );
    assert_eq!(
        vm.renderer_document_isolate_heap_usage()
            .expect("outer child heap usage should be available")
            .number_of_native_contexts,
        initial_native_contexts + 1,
        "accessing the outer contentDocument should materialize only its LocalWindow context"
    );

    assert_eq!(
        vm.eval("String(__indexedLazyOuter.contentWindow.length)")
            .expect("outer child frame count should evaluate"),
        "1"
    );
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const child = __indexedLazyOuter.contentWindow;
  const length = Object.getOwnPropertyDescriptor(child, "length");
  const credentialless = Object.getOwnPropertyDescriptor(child, "credentialless");
  const navigator = Object.getOwnPropertyDescriptor(child, "navigator");
  const shape = descriptor => ({
    getter: `${descriptor.get.name}:${descriptor.get.length}`,
    setter: typeof descriptor.set,
    enumerable: descriptor.enumerable,
    configurable: descriptor.configurable
  });
  return JSON.stringify({
    length: shape(length),
    credentialless: shape(credentialless),
    navigator: shape(navigator),
    borrowedLength: length.get.call(child),
    borrowedCredentialless: credentialless.get.call(child),
    borrowedNavigatorIsChild:
      navigator.get.call(child) === child.navigator,
    fakeNavigatorReceiver: (() => {
      try {
        navigator.get.call({});
        return "accepted";
      } catch (error) {
        return error.name;
      }
    })()
  });
})()
"#,
        )
        .expect("child Window accessor descriptors should evaluate"),
        r#"{"length":{"getter":"get length:0","setter":"function","enumerable":true,"configurable":true},"credentialless":{"getter":"get credentialless:0","setter":"undefined","enumerable":true,"configurable":true},"navigator":{"getter":"get navigator:0","setter":"undefined","enumerable":true,"configurable":true},"borrowedLength":1,"borrowedCredentialless":false,"borrowedNavigatorIsChild":true,"fakeNavigatorReceiver":"TypeError"}"#
    );
    assert_eq!(
        vm.renderer_document_isolate_heap_usage()
            .expect("indexed child count heap usage should be available")
            .number_of_native_contexts,
        initial_native_contexts + 1,
        "enumerating indexed children must not eagerly materialize descendant realms"
    );

    assert_eq!(
        vm.eval(
            "String(__indexedLazyOuter.contentWindow[0] === __indexedLazyNested.contentWindow)",
        )
        .expect("indexed nested WindowProxy access should evaluate"),
        "true"
    );
    assert_eq!(
        vm.renderer_document_isolate_heap_usage()
            .expect("indexed nested child heap usage should be available")
            .number_of_native_contexts,
        initial_native_contexts + 2,
        "the indexed getter should materialize only the descendant Window that was requested"
    );
}

#[test]
fn embedded_frame_owners_create_child_contexts_only_for_document_content() {
    let mut vm = new_storage_test_vm("https://embedded-frame-owner-selection.test/");

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const root = document.body || document.documentElement || document;
  const append = (tag, id, attribute, value, type = "") => {
    const element = document.createElement(tag);
    element.id = id;
    if (type) element.type = type;
    element[attribute] = value;
    root.appendChild(element);
  };
  append("iframe", "accepted-iframe", "src", "/child.html?iframe");
  append("frame", "accepted-frame", "src", "/child.html?frame");
  append("embed", "accepted-embed", "src", "/child.html?embed");
  append("object", "accepted-object", "data", "/child.html?object");
  append("embed", "image-embed", "src", "/image.png");
  append("object", "image-object", "data", "/image.png", "image/png");
  append("object", "plugin-object", "data", "/child.html", "application/x-test-plugin");

  for (const tag of ["audio", "video"]) {
    const media = document.createElement(tag);
    const embed = document.createElement("embed");
    embed.id = `${tag}-embed`;
    embed.type = "text/html";
    embed.src = `/${tag}-embed.html`;
    media.appendChild(embed);
    const object = document.createElement("object");
    object.id = `${tag}-object`;
    object.type = "text/html";
    object.data = `/${tag}-object.html`;
    media.appendChild(object);
    root.appendChild(media);
  }
  return "created";
})()
"#,
        )
        .expect("embedded frame-owner selection should evaluate"),
        "created"
    );

    let host = vm._context_host.borrow();
    assert_eq!(host.child_browsing_context_count(), 4);
    for id in [
        "accepted-iframe",
        "accepted-frame",
        "accepted-embed",
        "accepted-object",
    ] {
        let handle = host
            .dom_host()
            .element_handle_by_id(id)
            .expect("accepted frame owner should exist");
        assert!(
            host.child_browsing_context_document_handle(handle)
                .is_some(),
            "{id} should own an initial-empty child document"
        );
    }
    for id in [
        "image-embed",
        "image-object",
        "plugin-object",
        "audio-embed",
        "audio-object",
        "video-embed",
        "video-object",
    ] {
        let handle = host
            .dom_host()
            .element_handle_by_id(id)
            .expect("rejected embedded element should exist");
        assert!(
            host.child_browsing_context_document_handle(handle)
                .is_none(),
            "{id} must not be projected as a child browsing context"
        );
    }
    drop(host);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  document.getElementById("accepted-embed").type = "image/png";
  document.getElementById("accepted-object").data = "/image.png";
  return "reclassified";
})()
"#,
        )
        .expect("connected embedded frame owners should reclassify"),
        "reclassified"
    );
    assert_eq!(
        vm._context_host.borrow().child_browsing_context_count(),
        2,
        "switching accepted embedded content to image content must retire both child contexts"
    );

    assert_eq!(
        vm.eval(
            r#"
(() => {
  document.getElementById("accepted-embed").type = "text/html";
  document.getElementById("accepted-object").data = "/replacement.html";
  return "restored";
})()
"#,
        )
        .expect("connected embedded document owners should restore"),
        "restored"
    );
    assert_eq!(
        vm._context_host.borrow().child_browsing_context_count(),
        4,
        "switching back to document content must create fresh child contexts"
    );
}

#[tokio::test]
async fn child_document_open_nested_frame_uses_inherited_frame_src_policy() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-document-open-frame-csp.test/",
        &loader,
    );
    vm.document_runtime
        .set_response_content_security_policies(&["frame-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__childFrameCspEvents = [];
  addEventListener("message", event => {
    __childFrameCspEvents.push(String(event.data));
  });
  const frame = document.createElement("iframe");
  frame.id = "csp-owner";
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentDocument.write(
    '<script>addEventListener("securitypolicyviolation", event => {' +
    '  top.postMessage(event.violatedDirective, "*");' +
    '});</scr' + 'ipt>' +
    '<iframe src="https://blocked-frame.test/fail.html"></iframe>'
  );
  frame.contentDocument.close();
  return "queued";
})()
"#,
        )
        .expect("child document.open CSP setup should evaluate"),
        "queued"
    );

    for label in [
        "initial-empty outer realm retirement",
        "written outer Document realm materialization",
    ] {
        assert!(
            vm.run_one_child_frame_task_executor_turn(
                ChildFrameSemanticTurnKind::RealmMaterialization,
                &loader,
            )
            .await
            .expect("child realm materialization should use the selected-task dispatcher"),
            "{label} must remain a visible selected child-family turn"
        );
    }
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::NavigationCommit,
            &loader,
        )
        .await
        .expect("nested child navigation commit should use the selected-task dispatcher"),
        "the nested frame navigation should reach the CSP gate after both outer realm turns"
    );
    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("the child CSP report should dispatch to the top Window")
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__childFrameCspEvents)")
            .expect("child frame CSP events should be observable"),
        r#"["frame-src"]"#
    );
}

#[tokio::test]
async fn child_javascript_url_nested_frame_uses_inherited_frame_src_policy() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-javascript-url-frame-csp.test/",
        &loader,
    );
    vm.document_runtime
        .set_response_content_security_policies(&["frame-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__childJavascriptUrlFrameCspEvents = [];
  addEventListener("message", event => {
    __childJavascriptUrlFrameCspEvents.push(String(event.data));
  });
  const source = `javascript:
    addEventListener("securitypolicyviolation", event => {
      parent.postMessage(event.violatedDirective, "*");
    });
    const nested = document.createElement("iframe");
    nested.src = "https://blocked-frame.test/fail.html";
    document.body.appendChild(nested);
  `;
  const frame = document.createElement("iframe");
  frame.src = encodeURI(source.replaceAll("\n", ""));
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
})()
"#,
        )
        .expect("child javascript URL CSP setup should evaluate"),
        "queued"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::NavigationCommit,
            &loader,
        )
        .await
        .expect("javascript URL navigation commit should use the selected-task dispatcher")
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "javascript URL should execute after its initial child realm materializes",
    )
    .await;
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::NavigationCommit,
            &loader,
        )
        .await
        .expect("nested frame navigation should reach the inherited CSP gate")
    );
    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("child CSP report should dispatch to the top Window")
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__childJavascriptUrlFrameCspEvents)")
            .expect("child javascript URL CSP events should be observable"),
        r#"["frame-src"]"#
    );
}

#[tokio::test]
async fn child_document_write_inline_classic_obeys_inherited_response_csp() {
    let mut vm = new_storage_test_vm("https://child-document-write-csp.test/");
    vm.set_response_content_security_policies(&["script-src 'nonce-allowed'".to_owned()]);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__childDocumentWriteCspEvents = [];
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentDocument.open();
  frame.contentDocument.write(`
    <script>parent.__childDocumentWriteCspEvents.push("blocked");<\/script>
    <script nonce="allowed">parent.__childDocumentWriteCspEvents.push("allowed");<\/script>
  `);
  frame.contentDocument.close();
  return __childDocumentWriteCspEvents.join("|");
})()
"#,
        )
        .expect("child document.write CSP probe should evaluate"),
        "allowed"
    );
}

#[test]
fn child_document_write_nested_write_preserves_parser_insertion_point() {
    let mut vm = new_storage_test_vm("https://child-document-write-insertion.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.write(`<!doctype html>
    <html><body><main id="written-main">
      <p id="before-write">before</p>
      <script>document.write("<section id='nested-write'>nested</section>");<\/script>
      <template id="written-template"><span>template</span></template>
      <table id="written-table"><tbody><tr><td>cell</td></tr></tbody></table>
    </main></body></html>`);
  childDocument.close();
  return JSON.stringify({
    order: Array.from(childDocument.getElementById("written-main").children)
      .map(element => element.id || element.localName),
    nestedParent: childDocument.getElementById("nested-write").parentElement.id,
    bodyOrder: Array.from(childDocument.body.children)
      .map(element => element.id || element.localName)
  });
})()
"#,
        )
        .expect("nested child document.write parser insertion probe should evaluate");

    assert_eq!(
        result,
        r#"{"order":["before-write","script","nested-write","written-template","written-table"],"nestedParent":"written-main","bodyOrder":["written-main"]}"#
    );
}

// Ported from Chromium's WPT copy at
// html/webappapis/dynamic-markup-insertion/document-write/iframe_003.html.
#[test]
fn child_document_write_script_restores_character_chunked_tail() {
    let mut vm = new_storage_test_vm("https://child-write-character-tail.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  const source =
    "<script>document.write(\"<i id='a'>Filler Text</i>\")<\/script>" +
    "<b id=b>Filler Text</b>";
  for (const character of source) {
    childDocument.write(character);
  }
  childDocument.close();
  return Array.from(childDocument.body.children)
    .map(element => [element.localName, element.id, element.textContent].join(":"))
    .join("|");
})()
"#,
        )
        .expect("a written script must restore the character-chunked child parser tail");

    assert_eq!(result, "i:a:Filler Text|b:b:Filler Text");
}

// Ported from Chromium's WPT copy at
// html/webappapis/dynamic-markup-insertion/document-write/iframe_010.html.
#[test]
fn child_document_close_inside_written_script_preserves_insertion_stack() {
    let mut vm = new_storage_test_vm("https://child-write-script-close-stack.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  const source =
    "<script>document.write('<table><plaintext>Filler '); document.close();<\/script>";
  for (const character of source) {
    childDocument.write(character);
  }
  const children = childDocument.body.children;
  return [
    children.length,
    children[0]?.localName,
    children[0]?.textContent,
    children[1]?.localName,
  ].join("|");
})()
"#,
        )
        .expect("document.close() in a written script must preserve its parser insertion stack");

    assert_eq!(result, "2|plaintext|Filler |table");
}

// Ported from Chromium's WPT copy at
// html/webappapis/dynamic-markup-insertion/document-write/nested-document-write-2.html.
#[tokio::test]
async fn child_nested_external_document_write_preserves_input_order() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_network_offline(true);
    let mut vm =
        new_storage_test_vm_with_loader("https://child-nested-external-write.test/", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.open();

  const nestedSource =
    '<script src="data:text/javascript,document.write(%22w%22)%3Bdocument.write(%22o%22)%3B"><\/script>r';
  const nestedLiteral = JSON.stringify(nestedSource).replace("</script>", "<\\/script>");
  childDocument.write(
    "<script>document.write(" + nestedLiteral + "); document.write('k');<\/script>e"
  );
  childDocument.write("d");
  childDocument.close();
})()
"#,
    )
    .expect("nested external child document.write fixture should evaluate");

    while vm
        .run_next_child_frame_semantic_turn_for_test()
        .await
        .is_some()
    {
        // Drain the data-script source load, nested writes, and parser resume.
    }

    let result = vm
        .eval(
            r#"
document.querySelector("iframe").contentDocument.body.textContent
"#,
        )
        .expect("the nested external writer must restore all parent parser input");

    assert_eq!(result, "worked");
}

#[test]
fn child_parser_script_document_close_drains_restored_parent_input() {
    let mut vm = new_storage_test_vm("https://child-parser-close.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.write(
    '<script>document.close();<\/script>' +
    '<main id="tail-after-parser-close">tail</main>'
  );
  return String(!!childDocument.getElementById("tail-after-parser-close"));
})()
"#,
        )
        .expect("document.close() from a parser script must drain the restored parent input");

    assert_eq!(result, "true");
}

#[tokio::test]
async fn srcdoc_child_parser_write_script_drains_restored_parent_input() {
    let mut vm = new_storage_test_vm("https://srcdoc-write-drain.test/");
    vm.eval(
        r##"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc =
    '<body><script>document.title = "script-ran"; ' +
    "document.write('<b id=\"inserted\">i</b>'); " +
    '</' + 'script>' +
    '<main id="tail-after-write">tail</main></body>';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"##,
    )
    .expect("srcdoc write frame fixture should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "srcdoc write frame should commit before its parser script",
    )
    .await;
    while vm.has_ready_child_frame_semantic_turn_for_test(
        ChildFrameSemanticTurnKind::RealmMaterialization,
    ) {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "srcdoc write child realm should materialize before its parser script"
        );
    }
    while vm.has_ready_child_frame_semantic_turn_for_test(
        ChildFrameSemanticTurnKind::DocumentScriptReady,
    ) {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "srcdoc write parser script should run on DocumentScriptReady"
        );
    }
    run_child_document_lifecycle_and_host_load_for_test(&mut vm, "srcdoc write frame").await;

    let result = vm
        .eval(
            r#"
(() => {
  const childDocument = document.querySelector("iframe").contentDocument;
  return [
    childDocument.title,
    !!childDocument.getElementById("inserted"),
    !!childDocument.getElementById("tail-after-write")
  ].join("|");
})()
"#,
        )
        .expect("a finite child parser must drain the parent input restored after a write script");

    assert_eq!(result, "script-ran|true|true");
}

#[tokio::test]
async fn child_parser_resumes_write_queued_during_external_script_block() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_network_offline(true);
    let mut vm = new_storage_test_vm_with_loader("https://child-parent-write-block.test/", &loader);

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script src="data:text/javascript,globalThis.__childDataScriptRan%3Dtrue"><\/script>
    <main id="tail-after-block">tail</main>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("parent write block frame fixture should evaluate");
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "parent write block frame should commit"
    );
    while vm.has_ready_child_frame_semantic_turn_for_test(
        ChildFrameSemanticTurnKind::RealmMaterialization,
    ) {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "parent write block child realm should materialize"
        );
    }

    vm.eval(
        r#"
(() => {
  const frame = document.querySelector("iframe");
  frame.contentDocument.write('<b id="from-parent">p</b>');
})()
"#,
    )
    .expect("a parent write during the child parser suspension should evaluate");

    while vm
        .run_next_child_frame_semantic_turn_for_test()
        .await
        .is_some()
    {
        // Drain the data-script source load, its execution, and the parser
        // resume that must consume the parent-queued write input.
    }
    let mut lifecycle_turns = 0;
    while lifecycle_turns < 8
        && vm
            .run_child_frame_task_source_once_for_test(
                ChildFrameSemanticTurnKind::DocumentLifecycle,
            )
            .await
    {
        lifecycle_turns += 1;
    }
    let _ = vm
        .run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
        .await;

    let result = vm
        .eval(
            r#"
(() => {
  const childDocument = document.querySelector("iframe").contentDocument;
  return [
    !!childDocument.getElementById("from-parent"),
    !!childDocument.getElementById("tail-after-block")
  ].join("|");
})()
"#,
        )
        .expect("resuming a child parser with parent-queued write input must drain all input");

    assert_eq!(result, "true|true");
}

#[test]
fn pending_initial_child_navigation_keeps_initial_empty_window_surface() {
    let mut vm = new_storage_test_vm("https://child-pending-window.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "pending-window";
  frame.src = "https://child-pending-window.test/target.html";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("pending child Window setup should evaluate");

    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "the target navigation must remain pending before its typed commit turn"
    );
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const child = document.getElementById("pending-window").contentWindow;
  return [
    child.location.href,
    child.navigation.currentEntry?.url ?? "null",
    child.document.URL,
    child.document.readyState,
  ].join("|");
})()
"#,
        )
        .expect("pending child Window state should evaluate"),
        "about:blank|about:blank|about:blank|complete",
        "a planned target must not be projected as committed while the initial empty Document is still live"
    );

    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()[0];
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_current_url(child_handle)
            .is_some_and(|url| moli_url::is_about_blank(&url)),
        "native current-Document state must agree with the initial empty Window surface"
    );
}

#[tokio::test]
async fn pending_child_navigation_does_not_materialize_initial_empty_preload_realm() {
    let mut vm = new_storage_test_vm("https://child-preload-lazy.test/");
    vm.set_stored_document_start_scripts(&[crate::DocumentStartScript {
        registry_key: Some("initial-empty-lazy-preload".to_owned()),
        devtools_session: None,
        source: "globalThis.__documentStartUrl = document.URL;".to_owned(),
        world_name: None,
        has_bidi_channel_argument: false,
        bidi_channel_handoffs: Vec::new(),
    }]);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "pending-child-navigation";
  frame.srcdoc = "<!doctype html><title>committed child</title>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("pending child navigation setup should evaluate");

    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "the srcdoc navigation must publish a commit turn"
    );
    assert!(
        vm.has_pending_child_document_lifecycle(),
        "a reserved srcdoc navigation commit must keep child lifecycle work pending"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::DocumentLifecycle
        ),
        "the internal initial empty Document must not publish parser-finished lifecycle work"
    );
    assert!(
        vm.live_child_default_runtime_realm_inventory().is_empty(),
        "protocol inventory must not expose a realm for the unobserved initial empty Document"
    );

    run_child_navigation_commit_and_host_load_for_test(&mut vm, "committed child preload document")
        .await;
    let child_realms = vm.live_child_default_runtime_realm_inventory();
    assert_eq!(
        child_realms.len(),
        1,
        "the committed child document must expose exactly one default realm"
    );
    let child_context_id = child_realms[0].context_id;
    assert_eq!(
        vm.eval_in_child_default_context(child_context_id, "__documentStartUrl")
            .expect("committed child preload URL should evaluate"),
        "about:srcdoc",
        "document-start replay must observe the committed child Document"
    );
}

#[tokio::test]
async fn empty_named_preload_materializes_child_isolated_world() {
    let mut vm = new_storage_test_vm("https://child-empty-named-preload.test/");
    vm.set_stored_document_start_scripts(&[crate::DocumentStartScript {
        registry_key: Some("empty-utility-world".to_owned()),
        devtools_session: None,
        source: String::new(),
        world_name: Some("playwright-utility".to_owned()),
        has_bidi_channel_argument: false,
        bidi_channel_handoffs: Vec::new(),
    }]);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc = "<!doctype html><body>child</body>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("empty named preload child setup should evaluate");

    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "empty named preload child document",
    )
    .await;
    let frame_id = {
        let host = vm._context_host.borrow();
        let handles = host.child_browsing_context_handles_in_document_order();
        assert_eq!(handles.len(), 1, "expected one child browsing context");
        host.child_browsing_context_frame_id_by_owner_node_id(handles[0])
            .expect("child browsing context should have a frame id")
    };
    assert!(
        vm.has_isolated_world_named_for_frame(&frame_id, "playwright-utility"),
        "an empty world-scoped preload must still declare the child isolated world"
    );
}

#[tokio::test]
async fn child_body_onload_materializes_default_context_at_host_load() {
    let mut vm = new_storage_test_vm("https://child-body-onload-lazy.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childBodyOnloadEvents = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `<body onload="parent.__childBodyOnloadEvents.push(globalThis === self)">`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child body onload setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child body onload srcdoc should commit before lifecycle",
    )
    .await;
    assert_eq!(
        vm.child_frame_realm_store.len(),
        0,
        "a native body onload attribute should not materialize its realm before load dispatch"
    );
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("child body onload should run its {transition} lifecycle turn"),
        )
        .await;
    }
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "child body onload should dispatch from HostLoad",
    )
    .await;

    assert_eq!(
        vm.eval("__childBodyOnloadEvents.join('|')")
            .expect("child body onload trace should evaluate"),
        "true"
    );
    assert_eq!(
        vm.child_frame_realm_store.len(),
        1,
        "observable child window load work should materialize exactly one default realm"
    );
}

#[tokio::test]
async fn child_default_realm_record_tracks_owner_frame_realm_id() {
    let mut vm = new_storage_test_vm("https://child-owner-realm.test/");

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  return "created";
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "created");

    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child frame owner record",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child frame owner record");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };

    assert_eq!(
        vm.child_frame_realm_store
            .owner_realm_id_for_context_id(child_context_id),
        Some(owner_realm_id)
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .frame_owner_current_child_snapshot(child_handle)
            .and_then(|snapshot| snapshot.realm_id),
        Some(owner_realm_id)
    );

    let result = vm
        .eval_in_child_default_context(child_context_id, "globalThis === self")
        .expect("child owner realm id should enter the child default context");
    assert_eq!(result, "true");
    assert_eq!(
        vm.eval("document.location.href")
            .expect("main document location should remain observable"),
        "https://child-owner-realm.test/",
        "materializing a child realm must not bind its Location to the main Document"
    );
    assert_eq!(
        vm.eval_in_child_default_context(child_context_id, "document.location.href")
            .expect("child document location should evaluate in its owner realm"),
        "about:blank"
    );
}

#[tokio::test]
async fn child_document_script_owner_hooks_select_current_realm() {
    use super::child_document_script_owner_hooks::{
        ChildDocumentScriptOwnerHooks, ChildDocumentScriptRealmSelection,
    };
    use crate::frame_owner_model::FrameRealmId;

    let mut vm = new_storage_test_vm("https://child-script-hooks-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child script hook realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child script hook realm setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child script hook realm setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let script_handle = DomHandle::new(9100);

    let current = ChildDocumentScriptOwnerHooks::new(&mut vm).select_current_realm(
        child_handle,
        None,
        script_handle,
        "test_select_current_realm",
    );
    assert_eq!(
        current,
        ChildDocumentScriptRealmSelection::Current(owner_realm_id)
    );

    let stale = ChildDocumentScriptOwnerHooks::new(&mut vm).select_current_realm(
        child_handle,
        Some(FrameRealmId(owner_realm_id.0 + 1)),
        script_handle,
        "test_select_current_realm",
    );
    assert!(matches!(
        stale,
        ChildDocumentScriptRealmSelection::StaleRealm { .. }
    ));
}

#[tokio::test]
async fn child_dynamic_execution_action_requires_materialized_current_realm() {
    use crate::frame_owner_model::{
        FrameDocumentTaskOwner, FrameRealmId, PendingChildDynamicDocumentScript,
    };

    let mut vm = new_storage_test_vm("https://child-dynamic-current-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child dynamic current-realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child dynamic current-realm setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child dynamic current-realm setup",
    );
    let (child_handle, task_owner, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        let snapshot = vm
            ._context_host
            .borrow()
            .frame_owner_current_child_snapshot(realm.child_handle)
            .expect("child frame should expose a current owner snapshot");
        (
            realm.child_handle,
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            ),
            realm.owner_realm_id,
        )
    };
    let script_handle = DomHandle::new(9001);
    let work_without_realm = PendingChildDynamicDocumentScript {
        child_handle,
        owner: task_owner,
        realm_id: None,
        script_handle,
        source: "globalThis.__childDynamicMaterializedRealm = true;".to_owned(),
        script_nonce: Some("captured-nonce".to_owned()),
        script_integrity: Some("sha256-captured-integrity".to_owned()),
    };

    let action = vm
        ._context_host
        .borrow()
        .child_dynamic_classic_script_execution_action_for_owner(
            &work_without_realm,
            owner_realm_id,
        )
        .expect("current dynamic work should materialize an execution action");
    assert_eq!(action.target().task_owner(), task_owner);
    assert_eq!(action.target().realm_id(), owner_realm_id);
    let job = action.into_job();
    assert_eq!(job.script_nonce.as_deref(), Some("captured-nonce"));
    assert_eq!(
        job.script_integrity.as_deref(),
        Some("sha256-captured-integrity")
    );

    let stale_work = PendingChildDynamicDocumentScript {
        realm_id: Some(FrameRealmId(owner_realm_id.0 + 1)),
        ..work_without_realm
    };
    assert!(
        vm._context_host
            .borrow()
            .child_dynamic_classic_script_execution_action_for_owner(&stale_work, owner_realm_id)
            .is_none(),
        "stale dynamic child work must not produce an execution action"
    );
}

#[tokio::test]
async fn function_constructor_frame_script_job_returns_child_realm_function() {
    let mut vm = new_storage_test_vm("https://child-function-job.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child function job setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child function-constructor job setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child function-constructor job setup",
    );
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let job = vm
        ._context_host
        .borrow()
        .frame_owner_child_function_constructor_script_job(
            child_handle,
            Vec::new(),
            "return globalThis === self ? 27 : -1".to_owned(),
        )
        .expect("child owner should build FunctionConstructor job");
    let function = vm
        .function_from_frame_script_job(job)
        .expect("FunctionConstructor job should return a child realm function");
    let result = vm
        .call_frame_function_for_test(owner_realm_id, &function)
        .expect("child realm function should be callable");
    assert_eq!(result, "27");
}

#[tokio::test]
async fn parser_classic_frame_script_job_executes_in_child_realm() {
    let mut vm = new_storage_test_vm("https://child-classic-job.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child parser classic job setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child parser-classic job setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child parser-classic job setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let job = vm
        ._context_host
        .borrow()
        .frame_owner_child_parser_classic_script_job(
            child_handle,
            None,
            "globalThis.__parserClassicFrameJob = globalThis === self ? 31 : -1;".to_owned(),
        )
        .expect("child owner should build ParserClassic job");
    vm.exec_frame_script_job(job)
        .expect("ParserClassic job should execute in child realm");
    let observed = vm
        .eval_in_frame_realm(owner_realm_id, "String(globalThis.__parserClassicFrameJob)")
        .expect("child parser classic job side effect should be visible in child realm");
    assert_eq!(observed, "31");
    let parent_observed = vm
        .eval("String(globalThis.__parserClassicFrameJob)")
        .expect("parent realm should evaluate");
    assert_eq!(parent_observed, "undefined");
}

#[tokio::test]
async fn child_execution_context_exec_runs_as_frame_script_job() {
    let mut vm = new_storage_test_vm("https://child-context-exec-driver.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child exec frame setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child execution-context setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child execution-context setup");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;

    vm.exec_in_execution_context(
        child_context_id,
        "globalThis.__childContextExecFrameJob = globalThis === self ? 37 : -1;",
    )
    .expect("child execution context source should execute through frame script job");

    let observed = vm
        .eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childContextExecFrameJob)",
        )
        .expect("child execution context side effect should be visible in child realm");
    assert_eq!(observed, "37");
    let parent_observed = vm
        .eval("String(globalThis.__childContextExecFrameJob)")
        .expect("parent realm should evaluate");
    assert_eq!(parent_observed, "undefined");
}

#[tokio::test]
async fn child_srcdoc_inline_classic_script_runs_as_frame_script_job() {
    let mut vm = new_storage_test_vm("https://child-inline-classic-driver.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childClassicDriverEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childClassicDriverFrame = frame;
  frame.onload = () => globalThis.__childClassicDriverEvents.push("load");
  frame.srcdoc = `
    <script id="inline-classic">
      parent.__childClassicDriverEvents.push("child:" + (globalThis === self));
      parent.__childClassicDriverEvents.push("current:" + document.currentScript.id);
      globalThis.__childClassicDriverValue = 41;
    <\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child inline classic driver setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child inline classic srcdoc should commit before parser work",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child inline classic script should execute from DocumentScriptReady",
    )
    .await;

    assert_eq!(
        vm.eval("__childClassicDriverEvents.join('|')")
            .expect("child inline classic driver events before HostLoad should evaluate"),
        "child:true|current:inline-classic",
        "DocumentScriptReady should execute the inline classic script without firing iframe load"
    );
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        assert!(
            vm.run_child_frame_task_source_once_for_test(
                ChildFrameSemanticTurnKind::DocumentLifecycle
            )
            .await,
            "child inline classic should run its {transition} lifecycle transition before HostLoad"
        );
    }
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "iframe load should dispatch from the later HostLoad source turn"
    );
    assert_eq!(
        vm.eval("__childClassicDriverEvents.join('|')")
            .expect("child inline classic driver events after HostLoad should evaluate"),
        "child:true|current:inline-classic|load"
    );
    assert_eq!(
        vm.eval("String(__childClassicDriverFrame.contentDocument.currentScript)")
            .expect("child inline classic currentScript cleanup should evaluate"),
        "null"
    );
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child inline classic script should materialize a child realm");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childClassicDriverValue)"
        )
        .expect("child inline classic side effect should be visible in child realm"),
        "41"
    );
    assert_eq!(
        vm.eval("String(globalThis.__childClassicDriverValue)")
            .expect("parent realm should evaluate"),
        "undefined"
    );
}

#[tokio::test]
async fn child_inline_classic_script_moved_from_original_document_is_skipped() {
    let mut vm = new_storage_test_vm("https://child-inline-classic-moved.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childClassicMovedEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicMovedEvents.push("load");
  frame.srcdoc = `
    <script id="stale-inline">
      parent.__childClassicMovedEvents.push("stale:" + document.currentScript.id);
      globalThis.__staleInlineRan = true;
    <\/script>
    <script id="after-stale-inline">
      parent.__childClassicMovedEvents.push("after:" + document.currentScript.id);
      globalThis.__afterStaleInlineRan = true;
    <\/script>
  `;
  globalThis.__childClassicMovedFrame = frame;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child moved inline classic setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child moved inline classic srcdoc should commit before moving the pending script",
    )
    .await;
    vm.eval(
        r#"
(() => {
  const stale = __childClassicMovedFrame.contentDocument.getElementById("stale-inline");
  (document.body || document.documentElement || document).appendChild(document.adoptNode(stale));
})()
"#,
    )
    .expect("pending child inline classic script should move after srcdoc commit");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "moved stale inline classic work should be consumed by DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("__childClassicMovedEvents.join('|')")
            .expect("child moved inline classic events after stale turn should evaluate"),
        "",
        "stale moved script should be dropped without running and without firing iframe load"
    );
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "parser continuation after moved stale script should run from DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("__childClassicMovedEvents.join('|')")
            .expect("child moved inline classic events before HostLoad should evaluate"),
        "after:after-stale-inline",
        "parser continuation should run the next inline classic script without firing iframe load"
    );
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("moved child classic should run its {transition} lifecycle turn"),
        )
        .await;
    }
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "iframe load should dispatch from the later HostLoad source turn",
    )
    .await;

    assert_eq!(
        vm.eval("__childClassicMovedEvents.join('|')")
            .expect("child moved inline classic events should evaluate"),
        "after:after-stale-inline|load"
    );
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child moved inline classic script should materialize a child realm");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__staleInlineRan) + '|' + String(globalThis.__afterStaleInlineRan)"
        )
        .expect("child moved inline classic side effects should evaluate"),
        "undefined|true"
    );
}

#[tokio::test]
async fn child_pending_external_classic_blocks_later_inline_and_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_network_offline(true);
    let mut vm =
        new_storage_test_vm_with_loader("https://child-external-classic-block.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childClassicBlockEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicBlockEvents.push("load");
  frame.srcdoc = `
    <script src="https://child-external-classic-block.test/blocking.js"><\/script>
    <script>parent.__childClassicBlockEvents.push("after-inline");<\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child external classic block setup should evaluate");
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "srcdoc bootstrap should commit from the explicit NavigationCommit source turn"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "no child document-script ready work should run while the external classic source is pending"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad
        )
        .await,
        "the exact child authority should start the pending classic-script request"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad should not dispatch while external classic source is pending"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "blocked documents must not produce HostLoad delivery work"
    );

    assert_eq!(
        vm.eval("__childClassicBlockEvents.join('|')")
            .expect("child external classic block events should evaluate"),
        "",
        "pending external classic script must block later parser-connected inline script and load"
    );
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::RealmMaterialization),
        "the production-shaped child must consume its realm prerequisite"
    );
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        None,
        "owner pump should observe no HostLoad wake while lifecycle readiness is blocked"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "blocked lifecycle should continue to expose no HostLoad delivery action"
    );
    assert_eq!(
        vm.take_pending_child_frame_tree_events().len(),
        1,
        "the actual attachment projection should be drained before testing scheduler/output isolation"
    );
}

#[tokio::test]
async fn child_lifecycle_queues_only_the_ready_sibling_for_host_load() {
    // Production PageVms install their ResourceRequestClient before child parser work can
    // publish a concrete fetch-start task. Keep this low-level fixture on the
    // same capability topology while leaving the external script unsettled so
    // the sibling-lifecycle assertion does not depend on a network response.
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_network_offline(true);
    let mut vm =
        new_storage_test_vm_with_loader("https://child-host-load-ready-sibling.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childHostLoadSiblingEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));

  const blocked = document.createElement("iframe");
  blocked.onload = () => globalThis.__childHostLoadSiblingEvents.push("blocked-load");
  blocked.srcdoc = `
    <script src="https://child-host-load-ready-sibling.test/blocking.js"><\/script>
    <script>parent.__childHostLoadSiblingEvents.push("blocked-after-inline");<\/script>
  `;
  body.appendChild(blocked);

  const ready = document.createElement("iframe");
  ready.onload = () => globalThis.__childHostLoadSiblingEvents.push("ready-load");
  ready.srcdoc = `<p>ready sibling</p>`;
  body.appendChild(ready);
})()
"#,
    )
    .expect("child host-load sibling setup should evaluate");

    for child in ["blocked", "ready"] {
        run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
            &mut vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            &format!("{child} sibling should commit before lifecycle selection"),
        )
        .await;
    }
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
        "blocked sibling should start its typed classic fetch before lifecycle selection",
    )
    .await;
    for label in [
        "remaining blocked-sibling realm prerequisite",
        "remaining ready-sibling realm prerequisite",
    ] {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn_for_test().await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "{label} must consume one visible child-family turn"
        );
    }
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
        "ready sibling should enter interactive before HostLoad"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "ready sibling should dispatch DOMContentLoaded before HostLoad"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "ready sibling should apply complete before HostLoad"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad should receive only the ready sibling lifecycle action"
    );
    assert_eq!(
        vm.eval("__childHostLoadSiblingEvents.join('|')")
            .expect("child host-load sibling events should evaluate"),
        "ready-load",
        "ready sibling iframe load should dispatch while the earlier blocked child stays pending"
    );

    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "blocked sibling must not have a HostLoad delivery action"
    );
    assert_eq!(
        vm.eval("__childHostLoadSiblingEvents.join('|')")
            .expect("child host-load sibling events after blocked pump should evaluate"),
        "ready-load",
        "blocked child should still not dispatch load without its external classic script"
    );
}

#[tokio::test]
async fn child_external_classic_script_load_executes_as_frame_script_job() {
    let (script_url, request_path_rx, server) =
        spawn_child_external_classic_frame_script_job_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://child-external-classic-driver.test/",
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childExternalClassicJobEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childExternalClassicJobFrame = frame;
  frame.onload = () => globalThis.__childExternalClassicJobEvents.push("load");
  frame.srcdoc = `
    <script id="external-classic" src="{script_url}"><\/script>
    <script id="after-external-classic">
      parent.__childExternalClassicJobEvents.push(
        "inline-current:" + document.currentScript.id
      );
      parent.__childExternalClassicJobEvents.push(
        "inline:" + globalThis.__childExternalClassicValue
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
    ))
    .expect("child external classic frame job setup should evaluate");
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child external classic srcdoc should commit before source loading",
    )
    .await;
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
            &loader,
        )
        .await
        .expect("child classic source-load task should use the selected-task dispatcher"),
        "child external classic source load should start from the classic source-load turn"
    );

    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("pending child external classic events should evaluate"),
        "",
        "pending external classic script must block later inline script and child load"
    );

    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "child external classic completion",
    )
    .await;
    assert!(
        vm.has_pending_child_frame_realm_materialization(),
        "the accepted external script must retain one exact-realm prerequisite"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader,
        )
        .await
        .expect("child realm materialization should use the selected-task dispatcher"),
        "the typed realm turn must materialize the committed Document realm"
    );
    assert!(
        !vm.has_pending_child_frame_realm_materialization(),
        "the exact committed realm request must be settled after its typed turn"
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "realm completion must promote the external script into typed DocumentScriptReady",
    )
    .await;

    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("child external classic events before HostLoad should evaluate"),
        "external:true|external-current:external-classic|external-write:true|script-load",
        "the parser-blocking load event must ignore document.open() and finish without firing iframe load"
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child parser continuation should run from the next DocumentScriptReady turn",
    )
    .await;
    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("child external classic parser-continuation events should evaluate"),
        "external:true|external-current:external-classic|external-write:true|script-load|inline-current:after-external-classic|inline:73",
        "ignored document.open() must leave the parser owner alive for the following inline script"
    );
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
            &mut vm,
            &loader,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("child external classic should run its {transition} lifecycle turn"),
        )
        .await;
    }
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::HostLoad,
        "iframe load should dispatch from the later HostLoad source turn",
    )
    .await;

    assert_eq!(
        request_path_rx
            .await
            .expect("child external classic server should report request path"),
        "/child-classic.js"
    );
    server
        .await
        .expect("child external classic test server should finish");

    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("child external classic events should evaluate"),
        "external:true|external-current:external-classic|external-write:true|script-load|inline-current:after-external-classic|inline:73|load"
    );
    assert_eq!(
        vm.eval("String(__childExternalClassicJobFrame.contentDocument.currentScript)")
            .expect("child external classic currentScript cleanup should evaluate"),
        "null"
    );
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child external classic script should materialize a child realm");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let (task_owner, owner, script_handle) = {
        let host = vm._context_host.borrow();
        let owner = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child owner should expose a current owner snapshot");
        let document_handle = host
            .child_browsing_context_document_handle(child_handle)
            .expect("child document handle should exist");
        let script_handle = host
            .dom_host()
            .script_handles_in_subtree(document_handle)
            .into_iter()
            .find(|handle| {
                host.dom_host().get_attribute(*handle, "id").as_deref() == Some("external-classic")
            })
            .expect("external classic script handle should exist");
        (
            crate::frame_owner_model::FrameDocumentTaskOwner::new(
                owner.scheduler_lane_id,
                owner.local_window_id,
                owner.document_id,
            ),
            crate::frame_owner_model::FrameDocumentOwner::new(
                owner.local_window_id,
                owner.document_id,
            ),
            script_handle,
        )
    };
    let stale_task_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
        task_owner.scheduler_lane_id,
        task_owner.local_window_id,
        crate::frame_owner_model::DocumentId(task_owner.document_id.0 + 1),
    );
    assert!(
        super::child_document_event::ChildDocumentEventOwner::new(&mut vm)
            .dispatch_script_element_event_for_parts_selected_task_body(
                stale_task_owner,
                owner_realm_id,
                script_handle,
                crate::frame_owner_model::FrameDocumentScriptElementEventKind::Load,
            )
            .is_err(),
        "script element event helper with stale owner token should not dispatch"
    );
    let stale_event = crate::frame_owner_model::FrameDocumentScriptElementEvent {
        child_handle,
        owner: crate::frame_owner_model::FrameDocumentOwner::new(
            owner.local_window_id,
            crate::frame_owner_model::DocumentId(owner.document_id.0 + 1),
        ),
        script_handle,
        kind: crate::frame_owner_model::FrameDocumentScriptElementEventKind::Load,
    };
    assert!(
        super::child_document_event::ChildDocumentEventOwner::new(&mut vm)
            .dispatch_script_element_event(stale_event)
            .is_err(),
        "script element event with stale owner token should not dispatch"
    );
    assert_eq!(
        vm.eval("__childExternalClassicJobEvents.join('|')")
            .expect("child external classic events should still evaluate"),
        "external:true|external-current:external-classic|external-write:true|script-load|inline-current:after-external-classic|inline:73|load",
        "stale owner-token event must not fire the script load listener again"
    );
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childExternalClassicValue)"
        )
        .expect("child external classic side effect should be visible in child realm"),
        "73"
    );
    assert_eq!(
        vm.eval("String(globalThis.__childExternalClassicValue)")
            .expect("parent realm should evaluate"),
        "undefined"
    );
}

#[tokio::test]
async fn child_module_producer_boundaries_require_exact_task_owner() {
    let mut vm = new_storage_test_vm("https://child-module-attribution.test/");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "module-attribution-child";
  body.appendChild(frame);
})()
"#,
    )
    .expect("child module attribution fixture should install an iframe");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child module attribution",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child module attribution");
    let (child_handle, realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child module attribution realm should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let owner = current_single_child_document_owner_for_test(&vm, "child module attribution");
    let request_url =
        Url::parse("https://child-module-attribution.test/root.js").expect("module URL");

    let (exact_target, exact_network_attribution) = vm
        ._context_host
        .borrow()
        .capture_child_module_fetch_producer_for_child(
            child_handle,
            owner,
            realm_id,
            request_url.clone(),
        )
        .expect("exact child module owner should capture producer attribution");
    assert_eq!(exact_target.child_handle(), child_handle);
    assert_eq!(exact_target.task_owner(), owner);
    assert_eq!(exact_target.realm_id(), realm_id);
    assert_eq!(exact_network_attribution.request_url(), &request_url);
    assert_eq!(
        exact_network_attribution.document_url().as_str(),
        "about:blank"
    );
    assert!(exact_network_attribution.frame_id().is_some());

    let stale_lane_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
        crate::frame_owner_model::FrameSchedulerLaneId(owner.scheduler_lane_id.0 + 1),
        owner.local_window_id,
        owner.document_id,
    );
    assert!(
        vm._context_host
            .borrow()
            .capture_child_module_fetch_producer_for_child(
                child_handle,
                stale_lane_owner,
                realm_id,
                request_url.clone(),
            )
            .is_none(),
        "producer attribution must not collapse task owner to local-window/document IDs"
    );
    assert!(
        vm._context_host
            .borrow()
            .current_child_module_fetch_target_for_realm(stale_lane_owner, realm_id)
            .is_none(),
        "dependency target lookup must reject the same lane-only stale owner"
    );
    assert!(
        vm._context_host
            .borrow()
            .capture_child_module_fetch_producer_for_child(
                child_handle,
                owner,
                crate::frame_owner_model::FrameRealmId(realm_id.0 + 1),
                request_url.clone(),
            )
            .is_none(),
        "producer lookup must reject an exact task owner paired with the wrong realm"
    );

    let pending_script = crate::planning::PreparedScript {
        position: 1,
        node_id: crate::dom::NodeId::new(1),
        kind: crate::types::ScriptKind::Module,
        mode: crate::types::ScriptMode::ModuleDefer,
        source_kind: crate::types::ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: crate::planning::ScriptSource::External,
        url: request_url.clone(),
        base_url: request_url.clone(),
        initiator_url: request_url,
        host_script_handle: None,
    };
    let document_owner = owner.document_owner();
    vm._context_host
        .borrow_mut()
        .child_document_script_schedulers_mut()
        .register_module_script(document_owner, &pending_script);
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(document_owner),
        1
    );
    assert!(
        !vm._context_host
            .borrow_mut()
            .cancel_child_document_script_work_if_current(child_handle, stale_lane_owner),
        "a stale module-start failure must not cancel replacement Document work"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(document_owner),
        1,
        "rejecting stale failure cleanup must preserve the current Document's module work"
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .cancel_child_document_script_work_if_current(child_handle, owner),
        "the same failure cleanup must still cancel work for its exact current Document"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(document_owner),
        0
    );
}

#[tokio::test]
async fn child_parser_module_ready_lane_evaluates_compiled_graph_in_frame_realm() {
    let mut vm = new_storage_test_vm("https://child-parser-module-eval.test/");

    vm.eval_with_child_record_sync(
        r#"
(() => {
  globalThis.__childParserModuleEvalEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childParserModuleEvalFrame = frame;
  frame.srcdoc = "";
  body.appendChild(frame);
})()
"#,
    )
    .expect("child parser module eval setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "empty srcdoc child should commit about:srcdoc before HostLoad",
    )
    .await;
    assert_eq!(
        vm.eval("__childParserModuleEvalFrame.contentDocument.URL")
            .expect("empty srcdoc child URL should evaluate"),
        "about:srcdoc"
    );

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child parser module eval should materialize a child realm");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const marker = document.createElement("script");
  marker.id = "eval-module-target";
  marker.type = "application/json";
  marker.addEventListener("load", () => parent.__childParserModuleEvalEvents.push("script-load"));
  marker.addEventListener("error", () => parent.__childParserModuleEvalEvents.push("script-error"));
  body.appendChild(marker);
  parent.__childParserModuleEvalEvents.push("realm-ready");
})()
"#,
    )
    .expect("child parser module eval child-realm setup should evaluate");
    assert_eq!(
        vm.eval("__childParserModuleEvalEvents.join('|')")
            .expect("child parser module setup events should evaluate"),
        "realm-ready"
    );
    let (document_owner, script_handle) = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child frame should expose a current owner snapshot");
        let document_owner = crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        );
        let document_handle = host
            .child_browsing_context_document_handle(child_handle)
            .expect("child document handle should exist");
        let script_handle = host
            .dom_host()
            .script_handles_in_subtree(document_handle)
            .into_iter()
            .find(|handle| {
                host.dom_host().get_attribute(*handle, "id").as_deref()
                    == Some("eval-module-target")
            })
            .expect("eval module marker script handle should exist");
        (document_owner, script_handle)
    };
    let task_owner = vm
        ._context_host
        .borrow()
        .current_child_module_route_task_owner(document_owner, owner_realm_id)
        .expect("child frame should expose current document task owner");
    let key = crate::module_runtime::ModuleMapKey::java_script(
        Url::parse("https://child-parser-module-eval.test/module.js").expect("module url"),
    );
    let metadata = crate::module_runtime::ModuleFetchMetadata::default();
    let source = crate::module_runtime::ModuleSource::text(
        r#"parent.__childParserModuleEvalEvents.push("module:" + (globalThis === self));
globalThis.__childParserModuleEvalValue = 144;"#
            .to_owned(),
    );
    let (record, identity) = vm
        .compile_native_module_record_for_frame_realm(
            owner_realm_id,
            key.clone(),
            &source,
            key.url(),
            &metadata,
        )
        .expect("child frame module record should compile");
    let mut document_modulator = vm
        .child_document_modulator_store
        .take_or_create_document_modulator(task_owner.document_owner(), owner_realm_id);
    let root_entry = document_modulator.insert_compiled_record_with_metadata(
        key.clone(),
        record,
        identity,
        metadata,
    );
    let tasks = vm
        .child_document_modulator_store
        .restore_document_modulator(task_owner, owner_realm_id, document_modulator);
    vm.push_child_module_terminal_batch_to_frame_lane(tasks);
    let script = crate::planning::PreparedScript {
        position: 1,
        node_id: crate::dom::NodeId::new(1),
        kind: crate::types::ScriptKind::Module,
        mode: crate::types::ScriptMode::ModuleDefer,
        source_kind: crate::types::ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: crate::planning::ScriptSource::External,
        url: key.url().clone(),
        base_url: key.url().clone(),
        initiator_url: key.url().clone(),
        host_script_handle: None,
    };
    let pending_script_id = vm
        ._context_host
        .borrow_mut()
        .child_document_script_schedulers_mut()
        .register_and_watch_module_script(task_owner.document_owner(), &script)
        .pending_script_id();
    let work = crate::document_script_scheduler::DocumentModuleGraphReadyWork::new(
        task_owner,
        owner_realm_id,
        pending_script_id,
        script,
        script_handle,
        key,
        moli_module_script_tree::ModuleTreeId(1),
        crate::frame_owner_model::DocumentLoadDelayTokenId(1),
        crate::module_runtime::ModuleGraphHandle {
            root_entry,
            entries: vec![root_entry],
        },
    );

    assert!(
        super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(&mut vm)
            .notify_module_script_graph_ready_work(work),
        "child graph-ready work should queue DocumentScriptReady"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "DocumentScriptReady should evaluate the completed child module graph"
    );
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childParserModuleEvalValue)"
        )
        .expect("child parser module side effect should be visible in child realm"),
        "144"
    );
    assert_eq!(
        vm.eval("String(globalThis.__childParserModuleEvalValue)")
            .expect("parent realm should evaluate"),
        "undefined"
    );
    assert_eq!(
        vm.eval("__childParserModuleEvalEvents.join('|')")
            .expect("child parser module eval events should evaluate"),
        "realm-ready|module:true|script-load"
    );
}

#[tokio::test]
async fn parser_discovered_child_modulepreloads_wait_for_one_realm_turn_and_promote_fifo() {
    let (mut vm, modulepreload_source) =
        new_child_modulepreload_page_test_vm("https://child-modulepreload-pre-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "pre-realm-modulepreloads";
  frame.srcdoc = `
    <link rel="modulepreload" href="/first.mjs">
    <link rel="modulepreload" href="/second.mjs">
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("pre-realm modulepreload fixture should evaluate");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "the child commit should discover both modulepreload links"
    );

    let (child_handle, owner, realm_id_before_materialization) = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("modulepreload fixture should retain one child frame");
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("modulepreload fixture should install a child Document");
        (
            child_handle,
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            ),
            snapshot.realm_id,
        )
    };
    assert!(
        realm_id_before_materialization.is_some(),
        "parser discovery must reserve one exact realm identity"
    );
    assert!(
        vm.live_child_default_runtime_realm_inventory().is_empty(),
        "reserved realm identity must not grant execution or protocol visibility before its typed turn"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        2,
        "both Document-owned starts should survive until exact-realm admission"
    );
    assert!(vm.has_pending_child_frame_realm_materialization());
    assert!(
        !modulepreload_source.has_ready_task(),
        "pre-realm work must not enter the typed executable source early"
    );

    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
            .is_some(),
        "one realm-materialization turn should bind all starts for the same child Document"
    );
    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
            .is_none(),
        "two preload links must not manufacture a second realm turn"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        0
    );

    let first = modulepreload_source
        .pop_front()
        .map(|(_, task)| task.into_task());
    let second = modulepreload_source
        .pop_front()
        .map(|(_, task)| task.into_task());
    let exhausted = modulepreload_source.pop_front();
    let first = first.expect("first preload should be promoted");
    let second = second.expect("second preload should be promoted");
    assert_eq!(first.owner(), owner);
    assert_eq!(second.owner(), owner);
    assert_eq!(first.realm_id(), second.realm_id());
    assert_eq!(first.target().child_handle(), child_handle);
    assert_eq!(
        first.request().module_key().url().as_str(),
        "https://child-modulepreload-pre-realm.test/first.mjs"
    );
    assert_eq!(
        second.request().module_key().url().as_str(),
        "https://child-modulepreload-pre-realm.test/second.mjs"
    );
    assert!(exhausted.is_none(), "promotion must not clone a start task");
}

#[tokio::test]
async fn parser_discovered_child_modulepreload_error_waits_for_the_same_realm_boundary() {
    let mut vm = new_storage_test_vm("https://child-modulepreload-error-realm.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__preRealmModulepreloadErrors = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <link rel="modulepreload" href="/invalid.bin" as="image"
      onerror="parent.__preRealmModulepreloadErrors.push('link-error')">
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("pre-realm modulepreload error fixture should evaluate");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        1,
        "invalid and fetchable modulepreloads must share the exact-realm admission boundary"
    );
    assert!(
        !vm.run_child_modulepreload_event_action_body_for_test(),
        "the link error must not be executable before its realm exists"
    );

    assert!(
        vm.run_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        0
    );
    let _predecessors = run_child_modulepreload_event_after_predecessors_for_test(
        &mut vm,
        "realm materialization should promote the exact-Document link error",
    )
    .await;
    assert_eq!(
        vm.eval("__preRealmModulepreloadErrors.join('|')")
            .expect("modulepreload error events should evaluate"),
        "link-error"
    );
}

#[tokio::test]
async fn pre_realm_modulepreload_rejects_the_first_established_realm_after_replacement() {
    let (mut vm, modulepreload_source) = new_child_modulepreload_page_test_vm(
        "https://child-modulepreload-pre-realm-replaced.test/",
    );

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "replace-first-modulepreload-realm";
  frame.srcdoc = `<link rel="modulepreload" href="/must-not-start.mjs">`;
  body.appendChild(frame);
})()
"#,
    )
    .expect("first-realm replacement fixture should evaluate");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()
        .into_iter()
        .next()
        .expect("first-realm replacement fixture should retain one child");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        1
    );

    vm.eval(
        "void document.getElementById('replace-first-modulepreload-realm').contentWindow.Function",
    )
    .expect("first child Window exposure should establish semantic realm identity");
    let first_realm = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .and_then(|snapshot| snapshot.realm_id)
        .expect("first Window exposure should establish a realm id");
    vm._context_host
        .borrow_mut()
        .clear_child_default_execution_context_id(child_handle);
    vm.eval(
        "void document.getElementById('replace-first-modulepreload-realm').contentWindow.Function",
    )
    .expect("second child Window exposure should establish replacement realm identity");
    let replacement_realm = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .and_then(|snapshot| snapshot.realm_id)
        .expect("second Window exposure should establish a replacement realm id");
    assert_ne!(first_realm, replacement_realm);

    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
            .is_some(),
        "the replacement realm still owns one materialization turn"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        0,
        "the stale first-realm task should be consumed as a discard"
    );
    assert!(
        !modulepreload_source.has_ready_task(),
        "work stamped by the first established realm must not rebind to its replacement"
    );
}

#[tokio::test]
async fn child_document_replacement_discards_modulepreload_before_realm_admission() {
    let (mut vm, modulepreload_source) =
        new_child_modulepreload_page_test_vm("https://child-modulepreload-replacement.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "replace-pre-realm-modulepreload";
  frame.srcdoc = `<link rel="modulepreload" href="/retired.mjs">`;
  body.appendChild(frame);
})()
"#,
    )
    .expect("replacement modulepreload fixture should evaluate");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    let (child_handle, retired_owner) = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("replacement fixture should retain one child frame");
        let owner = host
            .current_child_document_task_owner(child_handle)
            .expect("first srcdoc should install an exact Document owner");
        (child_handle, owner)
    };
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        1
    );
    assert!(vm.has_pending_child_frame_realm_materialization());

    vm.eval(
        "document.getElementById('replace-pre-realm-modulepreload').srcdoc = \
         '<!doctype html><p id=\"replacement\">replacement</p>';",
    )
    .expect("replacement srcdoc should queue");
    let replacement_commit = vm
        .run_one_child_navigation_commit_body_for_test()
        .expect("replacement navigation body should succeed")
        .expect("replacement should retain one exact navigation task");
    assert!(
        matches!(
            replacement_commit.action.target_effect,
            crate::page_task_queue::PageChildNavigationCommitTargetEffect::AppliedToCurrentOwner
        ),
        "replacement should commit before the stale realm task is selected"
    );
    let current_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("replacement should install a current child Document");
    assert_ne!(retired_owner, current_owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_child_modulepreload_work_awaiting_realm_for_test(),
        0,
        "the replacement boundary must retire pre-realm work for the old Document"
    );
    let stale = vm
        .run_one_child_realm_materialization_body_for_test()
        .expect("child realm materialization body should succeed")
        .expect("the old Document's durable task must consume one stale owner turn");
    assert!(matches!(
        stale.action.target_effect,
        crate::page_task_queue::PageChildRealmMaterializationTargetEffect::IgnoredStaleOwner { .. }
    ));
    assert!(
        vm.run_one_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed")
            .is_none(),
        "a scriptless replacement must not eagerly create a realm task after retiring the stale exact-Document reservation"
    );
    assert!(
        !modulepreload_source.has_ready_task(),
        "retired pre-realm work must not reach the typed executable source"
    );
}

#[test]
fn child_module_terminal_warning_exposes_followup_progress() {
    let mut vm = new_storage_test_vm("https://child-module-terminal-warning.test/");
    let task_owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
        crate::frame_owner_model::FrameSchedulerLaneId(1),
        crate::frame_owner_model::LocalWindowId(2),
        crate::frame_owner_model::DocumentId(3),
    );
    let realm_id = crate::frame_owner_model::FrameRealmId(4);
    let key = crate::module_runtime::ModuleMapKey::java_script(
        Url::parse("https://child-module-terminal-warning.test/root.mjs").expect("module url"),
    );
    let mut batch = crate::frame_owner_model::FrameDocumentModuleTerminalBatch::default();
    batch.push_warning(
        crate::frame_owner_model::FrameDocumentModuleTerminalWarningRecord::new(
            task_owner,
            realm_id,
            crate::frame_owner_model::FrameDocumentModuleTerminalWarning::ParserRootTerminalWithoutOwnerWork {
                key,
                successful: false,
                parser_root_client_count: 1,
            },
        ),
    );

    let followup = vm.push_child_module_terminal_batch_to_frame_lane(batch);

    assert!(followup.made_progress());
    assert!(followup.terminal_warning_was_recorded());
    assert!(!followup.module_script_terminal_was_queued());
    assert!(!followup.modulepreload_event_action_was_queued());
    assert!(!followup.dynamic_import_owner_action_was_queued());
    assert!(
        vm.runtime_observable_lifecycle_errors_for_testing()
            .iter()
            .any(|warning| warning.contains("produced no owner-local terminal work"))
    );
}

#[tokio::test]
async fn child_module_reaction_target_rejects_a_replaced_realm_with_the_same_document_owner() {
    let mut vm = new_storage_test_vm("https://child-module-reaction-realm.test/");
    vm.eval_with_child_record_sync(
        "const root = document.documentElement || document.appendChild(document.createElement('html')); \
         const body = document.body || root.appendChild(document.createElement('body')); \
         const frame = document.createElement('iframe'); body.appendChild(frame);",
    )
    .expect("child module-reaction fixture should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child module-reaction fixture",
    )
    .await;

    let retired_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "retired child realm");
    let (child_handle, retired_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&retired_context_id)
            .expect("retired child realm record");
        (realm.child_handle, realm.owner_realm_id)
    };
    let document_owner = current_single_child_document_owner_for_test(&vm, "child reaction owner");
    let retired_target =
        crate::page_task_queue::RendererPageModuleReactionTarget::ChildParserModule {
            document_owner,
            realm_id: retired_realm_id,
        };
    assert!(vm.module_reaction_target_is_current(retired_target));

    vm.retire_child_frame_realm_for_test(child_handle);
    let current_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "replacement child realm");
    let current_realm_id = vm
        .child_frame_realm_store
        .get(&current_context_id)
        .expect("replacement child realm record")
        .owner_realm_id;
    assert_ne!(retired_realm_id, current_realm_id);
    assert_eq!(
        current_single_child_document_owner_for_test(&vm, "replacement child reaction owner"),
        document_owner,
        "realm rematerialization must preserve the Document owner in this collision fixture"
    );
    assert!(
        !vm.module_reaction_target_is_current(retired_target),
        "the old realm identity must not authorize work against the replacement realm"
    );
    assert!(vm.module_reaction_target_is_current(
        crate::page_task_queue::RendererPageModuleReactionTarget::ChildParserModule {
            document_owner,
            realm_id: current_realm_id,
        }
    ));
}

#[tokio::test]
async fn child_parser_module_evaluation_start_dispatches_load_before_tla_reaction() {
    let mut vm = new_storage_test_vm("https://child-parser-module-tla.test/");

    vm.eval_with_child_record_sync(
        r#"
(() => {
  globalThis.__childParserModuleTlaEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childParserModuleTlaFrame = frame;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child parser module TLA setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child parser module TLA setup",
    )
    .await;

    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child parser module TLA setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const marker = document.createElement("script");
  marker.id = "pending-module-target";
  marker.type = "application/json";
  marker.addEventListener("load", () => parent.__childParserModuleTlaEvents.push("script-load"));
  marker.addEventListener("error", () => parent.__childParserModuleTlaEvents.push("script-error"));
  body.appendChild(marker);
  parent.__childParserModuleTlaEvents.push("realm-ready");
})()
"#,
    )
    .expect("child parser module TLA child-realm setup should evaluate");
    assert_eq!(
        vm.eval("__childParserModuleTlaEvents.join('|')")
            .expect("child parser module TLA setup events should evaluate"),
        "realm-ready"
    );
    let (task_owner, script_handle) = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child frame should expose a current owner snapshot");
        let document_owner = crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        );
        let task_owner = host
            .current_child_module_route_task_owner(document_owner, owner_realm_id)
            .expect("child frame should expose current document task owner");
        let document_handle = host
            .child_browsing_context_document_handle(child_handle)
            .expect("child document handle should exist");
        let script_handle = host
            .dom_host()
            .script_handles_in_subtree(document_handle)
            .into_iter()
            .find(|handle| {
                host.dom_host().get_attribute(*handle, "id").as_deref()
                    == Some("pending-module-target")
            })
            .expect("pending module marker script handle should exist");
        (task_owner, script_handle)
    };
    let key = crate::module_runtime::ModuleMapKey::java_script(
        Url::parse("https://child-parser-module-tla.test/module.js").expect("module url"),
    );
    let metadata = crate::module_runtime::ModuleFetchMetadata::default();
    let source = crate::module_runtime::ModuleSource::text(
        r#"parent.__childParserModuleTlaEvents.push("module-start:" + (globalThis === self));
await new Promise(resolve => {
  globalThis.__resolveChildParserModuleTla = resolve;
  parent.__childParserModuleTlaEvents.push("module-pending");
});
globalThis.__childParserModuleTlaValue = 377;
parent.__childParserModuleTlaEvents.push("module-after");"#
            .to_owned(),
    );
    let (record, identity) = vm
        .compile_native_module_record_for_frame_realm(
            owner_realm_id,
            key.clone(),
            &source,
            key.url(),
            &metadata,
        )
        .expect("child frame TLA module record should compile");
    let mut document_modulator = vm
        .child_document_modulator_store
        .take_or_create_document_modulator(task_owner.document_owner(), owner_realm_id);
    let root_entry = document_modulator.insert_compiled_record_with_metadata(
        key.clone(),
        record,
        identity,
        metadata,
    );
    let tasks = vm
        .child_document_modulator_store
        .restore_document_modulator(task_owner, owner_realm_id, document_modulator);
    vm.push_child_module_terminal_batch_to_frame_lane(tasks);
    let script = crate::planning::PreparedScript {
        position: 1,
        node_id: crate::dom::NodeId::new(1),
        kind: crate::types::ScriptKind::Module,
        mode: crate::types::ScriptMode::ModuleDefer,
        source_kind: crate::types::ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: crate::planning::ScriptSource::External,
        url: key.url().clone(),
        base_url: key.url().clone(),
        initiator_url: key.url().clone(),
        host_script_handle: None,
    };
    let pending_script_id = vm
        ._context_host
        .borrow_mut()
        .child_document_script_schedulers_mut()
        .register_and_watch_module_script(task_owner.document_owner(), &script)
        .pending_script_id();
    let work = crate::document_script_scheduler::DocumentModuleGraphReadyWork::new(
        task_owner,
        owner_realm_id,
        pending_script_id,
        script,
        script_handle,
        key,
        moli_module_script_tree::ModuleTreeId(1),
        crate::frame_owner_model::DocumentLoadDelayTokenId(1),
        crate::module_runtime::ModuleGraphHandle {
            root_entry,
            entries: vec![root_entry],
        },
    );

    assert!(
        super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(&mut vm)
            .notify_module_script_graph_ready_work(work),
        "child TLA graph-ready work should queue DocumentScriptReady"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "DocumentScriptReady should start child TLA module evaluation"
    );
    assert_eq!(
        vm.eval("__childParserModuleTlaEvents.join('|')")
            .expect("child parser module TLA pending events should evaluate"),
        "realm-ready|module-start:true|module-pending|script-load",
        "module load should dispatch once evaluation starts without waiting for TLA settlement"
    );

    vm.eval_in_frame_realm(
        owner_realm_id,
        "globalThis.__resolveChildParserModuleTla(); 'ok'",
    )
    .expect("child parser module TLA resolver should run");
    assert_eq!(
        vm.run_page_module_reaction_body_for_test()
            .expect("child parser module TLA reaction should run"),
        Some(
            crate::page_task_queue::PageModuleReactionTargetEffect::AppliedToCurrentOwner(
                crate::page_task_queue::PageModuleReactionCurrentEffect::ModuleStateUpdated,
            )
        ),
        "resolving child parser module TLA should queue one exact-owner reaction"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "DocumentScriptReady should finish fulfilled child TLA module evaluation"
    );

    assert_eq!(
        vm.eval("__childParserModuleTlaEvents.join('|')")
            .expect("child parser module TLA fulfilled events should evaluate"),
        "realm-ready|module-start:true|module-pending|script-load|module-after",
        "the fulfilled reaction should continue evaluation without dispatching script load again"
    );
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childParserModuleTlaValue)"
        )
        .expect("child parser module TLA side effect should be visible in child realm"),
        "377"
    );
}

#[tokio::test]
async fn child_parser_module_graph_failure_dispatches_error_from_scheduler_lane() {
    let mut vm = new_storage_test_vm("https://child-parser-module-failure.test/");

    vm.eval_with_child_record_sync(
        r#"
(() => {
  globalThis.__childParserModuleFailureEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childParserModuleFailureFrame = frame;
  frame.srcdoc = "";
  body.appendChild(frame);
})()
"#,
    )
    .expect("child parser module failure setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "child parser module failure setup should initialize through NavigationCommit then HostLoad",
    )
    .await;

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child parser module failure should materialize a child realm");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const marker = document.createElement("script");
  marker.id = "failed-module-target";
  marker.type = "application/json";
  marker.addEventListener("load", () => parent.__childParserModuleFailureEvents.push("script-load"));
  marker.addEventListener("error", () => parent.__childParserModuleFailureEvents.push("script-error"));
  body.appendChild(marker);
  parent.__childParserModuleFailureEvents.push("realm-ready");
})()
"#,
    )
    .expect("child parser module failure child-realm setup should evaluate");
    assert_eq!(
        vm.eval("__childParserModuleFailureEvents.join('|')")
            .expect("child parser module failure setup events should evaluate"),
        "realm-ready"
    );
    let (task_owner, script_handle) = {
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child frame should expose a current owner snapshot");
        let document_owner = crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        );
        let task_owner = host
            .current_child_module_route_task_owner(document_owner, owner_realm_id)
            .expect("child frame should expose current document task owner");
        let document_handle = host
            .child_browsing_context_document_handle(child_handle)
            .expect("child document handle should exist");
        let script_handle = host
            .dom_host()
            .script_handles_in_subtree(document_handle)
            .into_iter()
            .find(|handle| {
                host.dom_host().get_attribute(*handle, "id").as_deref()
                    == Some("failed-module-target")
            })
            .expect("failed module marker script handle should exist");
        (task_owner, script_handle)
    };
    let key = crate::module_runtime::ModuleMapKey::java_script(
        Url::parse("https://child-parser-module-failure.test/module.js").expect("module url"),
    );
    let script = crate::planning::PreparedScript {
        position: 1,
        node_id: crate::dom::NodeId::new(1),
        kind: crate::types::ScriptKind::Module,
        mode: crate::types::ScriptMode::ModuleDefer,
        source_kind: crate::types::ScriptSourceKind::External,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
        source: crate::planning::ScriptSource::External,
        url: key.url().clone(),
        base_url: key.url().clone(),
        initiator_url: key.url().clone(),
        host_script_handle: None,
    };
    let pending_script_id = vm
        ._context_host
        .borrow_mut()
        .child_document_script_schedulers_mut()
        .register_module_script(task_owner.document_owner(), &script);
    assert!(
        vm._context_host
            .borrow_mut()
            .child_document_script_schedulers_mut()
            .watch_module_script(pending_script_id)
            .watched(),
        "child failed parser module should be watched before terminal failure"
    );
    let work = crate::document_script_scheduler::DocumentModuleGraphFailedWork::new(
        task_owner,
        owner_realm_id,
        pending_script_id,
        script,
        script_handle,
        key,
        None,
        crate::frame_owner_model::DocumentLoadDelayTokenId(1),
        crate::module_runtime::ModuleLoadError::new(
            crate::module_runtime::ModuleLoadStage::Fetch,
            "network failed",
        ),
    );
    super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(&mut vm)
        .notify_module_script_graph_failed_action(work);

    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "child graph-failed work should dispatch from DocumentScriptReady"
    );
    assert!(
        !vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .has_ready_work(),
        "child graph-failed work should be consumed by the document script ready source"
    );
    assert_eq!(
        vm.eval("__childParserModuleFailureEvents.join('|')")
            .expect("child parser module failure events should evaluate"),
        "realm-ready|script-error"
    );
}

#[tokio::test]
async fn child_inline_parser_module_executes_from_registered_pending_script() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("http://child-inline-module.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childInlineParserModuleEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>parent.__childInlineParserModuleEvents.push("before");<\/script>
    <script type="module">
      parent.__childInlineParserModuleEvents.push("module:" + (globalThis === self));
      globalThis.__childInlineParserModuleValue = 42;
    <\/script>
    <script>parent.__childInlineParserModuleEvents.push("after:" + String(globalThis.__childInlineParserModuleValue));<\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child inline parser module setup should evaluate");
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "srcdoc navigation should install the child document before parser work"
    );
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "first child parser classic script should execute",
    )
    .await;
    let parser_module_owner = {
        let child_context_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("child inline module handoff should materialize a child realm");
        let child_handle = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child inline module realm should exist")
            .child_handle;
        let snapshot = vm
            ._context_host
            .borrow()
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child inline module frame should expose its document owner");
        crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        )
    };
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(parser_module_owner),
        1,
        "inline module PendingScript must exist before graph start"
    );
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::ParserModuleRootStart,
        "inline module graph should start before the parser continues past its element",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "parser should continue past the deferred inline module after graph start",
    )
    .await;
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "parser EOF should make the child document interactive before defer execution"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "graph-ready inline module should execute from DocumentScriptReady"
    );

    assert_eq!(
        vm.eval("__childInlineParserModuleEvents.join('|')")
            .expect("child inline parser module events should evaluate"),
        "before|after:undefined|module:true"
    );
}

#[tokio::test]
async fn child_external_parser_module_executes_from_document_ready_lane() {
    let (script_url, request_path_rx, server) =
        spawn_child_external_parser_module_ready_lane_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://child-parser-module-driver.test/",
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childExternalParserModuleEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  globalThis.__childExternalParserModuleFrame = frame;
  frame.addEventListener("load", () => {{
    globalThis.__childExternalParserModuleEvents.push("frame-load");
  }});
  frame.srcdoc = `
    <script>parent.__childExternalParserModuleEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childExternalParserModuleEvents.push("script-load");
      }});
      parent.__childExternalParserModuleEvents.push("after:" + String(globalThis.__childExternalParserModuleValue));
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
    ))
    .expect("child external parser module setup should evaluate");
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child external parser module srcdoc should commit before parser work",
    )
    .await;
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child inline parser script should run from DocumentScriptReady",
    )
    .await;
    let parser_module_owner = {
        let child_context_id = vm
            .live_child_default_runtime_realm_inventory()
            .into_iter()
            .map(|realm| realm.context_id)
            .next()
            .expect("child parser module handoff should materialize a child realm");
        let child_handle = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child parser module realm should exist")
            .child_handle;
        let host = vm._context_host.borrow();
        let snapshot = host
            .frame_owner_current_child_snapshot(child_handle)
            .expect("child parser module frame should expose its document owner");
        crate::frame_owner_model::FrameDocumentOwner::new(
            snapshot.local_window_id,
            snapshot.document_id,
        )
    };
    assert_eq!(
        vm._context_host
            .borrow()
            .child_document_script_schedulers()
            .pending_parser_module_script_count_for_test(parser_module_owner),
        1,
        "parser handoff must register PendingScript before the root-fetch source runs"
    );
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::ParserModuleRootStart,
        "ParserModuleRootStart should begin fetch before the parser continues past the element",
    )
    .await;
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "parser should continue past module-defer script from DocumentScriptReady after fetch start",
    )
    .await;
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "child parser module should enter interactive before module-defer execution",
    )
    .await;
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("pre-terminal child HostLoad probe should use the selected-task dispatcher"),
        "registered PendingScript should block HostLoad before root-fetch starts"
    );
    assert_eq!(
        vm.eval("__childExternalParserModuleEvents.join('|')")
            .expect("child external parser module pre-completion events should evaluate"),
        "before:true|after:undefined",
        "parser should continue past module-defer script while root module fetch is pending"
    );

    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "child external parser module completion",
    )
    .await;
    assert!(
        vm.run_one_child_module_script_terminal_executor_turn(&loader)
            .await
            .expect("child module terminal should use the selected-task dispatcher"),
        "child parser module root completion should fan out from the typed terminal source"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            &loader,
        )
        .await
        .expect("child module execution should use the selected-task dispatcher"),
        "child parser module script should execute from DocumentScriptReady"
    );

    assert_eq!(
        request_path_rx
            .await
            .expect("child external parser module server should report request path"),
        "/child-parser-module.js"
    );
    server
        .await
        .expect("child external parser module test server should finish");

    assert_eq!(
        vm.eval("__childExternalParserModuleEvents.join('|')")
            .expect("child external parser module events should evaluate"),
        "before:true|after:undefined|module:true|script-load",
        "script load should dispatch before iframe load"
    );
    for transition in ["DOMContentLoaded", "complete"] {
        run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
            &mut vm,
            &loader,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("child parser module should run its {transition} lifecycle turn"),
        )
        .await;
    }
    assert!(
        vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("child HostLoad should use the selected-task dispatcher"),
        "iframe load should dispatch from the later HostLoad source turn"
    );
    assert_eq!(
        vm.eval("__childExternalParserModuleEvents.join('|')")
            .expect("child external parser module events should evaluate after HostLoad"),
        "before:true|after:undefined|module:true|script-load|frame-load"
    );
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child external parser module should materialize a child realm");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;
    assert_eq!(
        vm.eval_in_frame_realm(
            owner_realm_id,
            "String(globalThis.__childExternalParserModuleValue)"
        )
        .expect("child parser module side effect should be visible in child realm"),
        "188"
    );
    assert_eq!(
        vm.eval("String(globalThis.__childExternalParserModuleValue)")
            .expect("parent realm should evaluate"),
        "undefined"
    );
}

#[tokio::test]
async fn child_dynamic_import_root_fetch_uses_child_import_map_and_initiator_url() {
    let mut vm = new_storage_test_vm("https://parent-dynamic-owner.test/page.html");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="https://child-dynamic-owner.test/nested/frame.html">
    <script type="importmap">
      {"imports":{"mapped-root":"/mapped/dynamic-root.js"}}
    <\/script>
    <script nonce="child-dynamic-import-nonce">
      parent.__childDynamicImportOwnerReady = true;
      import('mapped-root');
    <\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child dynamic import owner setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child dynamic import srcdoc should commit before parser work",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child dynamic import setup inline script should run from DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("String(globalThis.__childDynamicImportOwnerReady)")
            .expect("child dynamic import owner ready flag should evaluate"),
        "true"
    );
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("child dynamic import setup should run its {transition} lifecycle turn"),
        )
        .await;
    }
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "child dynamic import setup frame should finish from a later HostLoad turn",
    )
    .await;

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child default execution context should be created");
    let child_realm = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist");
    let child_handle = child_realm.child_handle;
    let child_realm_id = child_realm.owner_realm_id;
    let child_initiator_url = vm
        .child_browsing_context_module_request_initiator_url(child_handle)
        .expect("child document should expose a module request initiator URL");
    let dynamic_import_owner = vm
        .with_frame_realm_scope_and_checkpoint_for_test(child_realm_id, move |scope, host_ptr| {
            unsafe { &*host_ptr }
                .current_dynamic_module_import_owner(scope, Some(child_handle))
                .ok_or_else(|| anyhow::anyhow!("child dynamic import owner is unavailable"))
        })
        .expect("child dynamic import must bind its current execution context");
    assert_eq!(
        child_initiator_url.as_str(),
        "https://child-dynamic-owner.test/nested/frame.html"
    );

    let (_child_handle, task_owner, realm_id) = dynamic_import_owner
        .child_parts()
        .expect("dynamic import owner should identify the child document");
    assert!(
        !vm.child_document_modulator_store
            .contains_execution_context(task_owner.document_owner()),
        "a child classic script should not eagerly create module state"
    );
    assert!(
        matches!(
            vm.run_next_native_dynamic_module_owner_action_selected_task_body(),
            super::native_module::MainNativeModuleSelectedTaskApplication::Applied(_)
        ),
        "the classic-script import() callback should enqueue a graph job"
    );
    assert!(
        vm.child_document_modulator_store
            .contains_execution_context(task_owner.document_owner()),
        "the first child dynamic import graph start should lazily create its document modulator"
    );
    let root_fetch = vm
        .take_child_dynamic_module_import_fetch(task_owner.document_owner(), realm_id, 1)
        .expect("child dynamic import graph start should retain its root fetch");
    let root_fetch = root_fetch.inflight.request_for_test();

    assert_eq!(
        root_fetch.source_url().as_str(),
        "https://child-dynamic-owner.test/mapped/dynamic-root.js"
    );
    assert_eq!(root_fetch.initiator_url_for_test(), &child_initiator_url);
    assert_eq!(root_fetch.nonce(), Some("child-dynamic-import-nonce"));
}

#[tokio::test]
async fn child_dynamic_import_unexpected_complete_followup_without_graph_records_warning() {
    use crate::frame_owner_model::{
        FrameDocumentDynamicImportGraphAdvanceFollowup,
        FrameDocumentDynamicImportUnexpectedCompleteWarning, FrameDocumentTaskOwner,
    };

    let mut vm = new_storage_test_vm("https://parent-dynamic-owner-action.test/page.html");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child dynamic import owner action setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "empty child dynamic-import warning setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "empty child dynamic-import warning setup",
    );
    let (task_owner, realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        let owner = vm
            ._context_host
            .borrow()
            .frame_owner_current_child_snapshot(realm.child_handle)
            .expect("child frame should expose a current owner snapshot");
        (
            FrameDocumentTaskOwner::new(
                owner.scheduler_lane_id,
                owner.local_window_id,
                owner.document_id,
            ),
            realm.owner_realm_id,
        )
    };
    let outcome = vm.apply_child_dynamic_import_followup(
        FrameDocumentDynamicImportGraphAdvanceFollowup::RecordUnexpectedCompleteWarning(
            FrameDocumentDynamicImportUnexpectedCompleteWarning::new(
                task_owner.document_owner(),
                realm_id,
            ),
        ),
    );
    assert!(outcome.made_progress());
    assert!(
        outcome.terminal_warning_was_recorded(),
        "warning-only follow-up should not require a pending module graph"
    );
    assert!(
        !outcome.dynamic_import_owner_action_was_queued(),
        "warning-only follow-up must not enqueue a dynamic import owner action"
    );
}

#[tokio::test]
async fn child_external_classic_source_error_dispatches_before_later_inline() {
    let (script_url, request_path_rx, server) =
        spawn_child_external_classic_source_error_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://child-external-classic-source-error.test/",
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__childExternalClassicSourceErrorEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childExternalClassicSourceErrorEvents.push("load");
  frame.srcdoc = `
    <script>
      document.addEventListener('DOMContentLoaded', () => {{
        parent.__childExternalClassicSourceErrorEvents.push('dcl');
      }});
    <\/script>
    <script id="missing-classic" src="{script_url}"
      onerror="parent.__childExternalClassicSourceErrorEvents.push('script-error')"><\/script>
    <script>
      parent.__childExternalClassicSourceErrorEvents.push('after-inline');
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
    ))
    .expect("child external classic source error setup should evaluate");
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child external classic source-error srcdoc should commit before parser work",
    )
    .await;
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "initial inline parser script should run before the external source load starts",
    )
    .await;
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
            &loader,
        )
        .await
        .expect("child classic source-load task should use the selected-task dispatcher"),
        "child external classic source-error load should start from the classic source-load turn"
    );
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("pre-terminal child HostLoad probe should use the selected-task dispatcher"),
        "HostLoad should report no progress while parser-blocking classic source is pending"
    );

    assert_eq!(
        vm.eval("__childExternalClassicSourceErrorEvents.join('|')")
            .expect("pending child external classic source error events should evaluate"),
        "",
        "pending failed external classic source must block later inline script and child load"
    );

    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "child external classic source-error completion",
    )
    .await;
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            &loader,
        )
        .await
        .expect("child script-error task should use the selected-task dispatcher"),
        "child external classic source failure should dispatch script error from DocumentScriptReady"
    );
    assert_eq!(
        vm.eval("__childExternalClassicSourceErrorEvents.join('|')")
            .expect("child external classic source error events before HostLoad should evaluate"),
        "script-error",
        "first DocumentScriptReady should dispatch the source error without firing iframe load"
    );
    assert!(
        !vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("blocked child HostLoad probe should use the selected-task dispatcher"),
        "HostLoad should not dispatch while source-failure parser continuation is still queued"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            &loader,
        )
        .await
        .expect("child parser continuation should use the selected-task dispatcher"),
        "source-failure parser continuation should run from the next DocumentScriptReady turn"
    );
    assert_eq!(
        vm.eval("__childExternalClassicSourceErrorEvents.join('|')")
            .expect("child external classic source error continuation events should evaluate"),
        "script-error|after-inline",
        "parser continuation should execute the following inline script without firing iframe load"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &loader,
        )
        .await
        .expect("child interactive task should use the selected-task dispatcher"),
        "parser EOF should dispatch interactive before HostLoad"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &loader,
        )
        .await
        .expect("child DCL task should use the selected-task dispatcher"),
        "DOMContentLoaded should dispatch from its own lifecycle turn"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &loader,
        )
        .await
        .expect("child complete task should use the selected-task dispatcher"),
        "document complete should dispatch from its own lifecycle turn"
    );
    assert!(
        vm.run_one_child_frame_task_executor_turn(ChildFrameSemanticTurnKind::HostLoad, &loader)
            .await
            .expect("child HostLoad task should use the selected-task dispatcher"),
        "iframe load should dispatch from the later HostLoad source turn"
    );

    assert_eq!(
        request_path_rx
            .await
            .expect("child external classic source error server should report request path"),
        "/missing-classic.js"
    );
    server
        .await
        .expect("child external classic source error test server should finish");

    assert_eq!(
        vm.eval("__childExternalClassicSourceErrorEvents.join('|')")
            .expect("child external classic source error events should evaluate"),
        "script-error|after-inline|dcl|load"
    );
}

#[tokio::test]
async fn child_inline_classic_throw_reports_to_child_window_and_continues() {
    let mut vm = new_storage_test_vm("https://child-inline-classic-throw.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childInlineClassicThrowEvents = [];
  window.addEventListener("error", event => {
    globalThis.__childInlineClassicThrowEvents.push(
      "parent-listener-error:" + event.message
    );
  });
  window.onerror = (message) => {
    globalThis.__childInlineClassicThrowEvents.push("parent-error:" + message);
    return true;
  };
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childInlineClassicThrowEvents.push("load");
  frame.srcdoc = `
    <script>
      globalThis.onerror = function(message, source, line, column, error) {
        parent.__childInlineClassicThrowEvents.push(
          "child-error:" + message + ":" + (error && error.message) + ":" + (this === window)
        );
        return true;
      };
      document.addEventListener('DOMContentLoaded', () => {
        parent.__childInlineClassicThrowEvents.push('dcl');
      });
    <\/script>
    <script>
      throw new Error('child-boom');
    <\/script>
    <script>
      parent.__childInlineClassicThrowEvents.push('after-inline:' + (globalThis === window));
    <\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child inline classic throw setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child inline classic error srcdoc should commit before parser work",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "first child inline classic setup script should run from DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("__childInlineClassicThrowEvents.join('|')")
            .expect("child inline classic throw events after setup should evaluate"),
        "",
        "first setup script should not fire child load or errors"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "throwing child inline classic script should run from the next DocumentScriptReady turn"
    );
    assert_eq!(
        vm.eval("__childInlineClassicThrowEvents.join('|')")
            .expect("child inline classic throw events after error should evaluate"),
        "child-error:Uncaught Error: child-boom:child-boom:true",
        "throwing script should report to the child window without firing iframe load"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad should not dispatch while parser continuation work is still queued"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "parser continuation after the throw should run from DocumentScriptReady"
    );
    assert_eq!(
        vm.eval("__childInlineClassicThrowEvents.join('|')")
            .expect("child inline classic throw events after parser continuation should evaluate"),
        "child-error:Uncaught Error: child-boom:child-boom:true|after-inline:true",
        "parser continuation should run the following inline script without firing iframe load"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "parser EOF should dispatch interactive before HostLoad"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "DOMContentLoaded should dispatch from its own lifecycle turn after inline error recovery"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::DocumentLifecycle)
            .await,
        "complete should apply on its own lifecycle turn before iframe load"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "iframe load should dispatch from the later HostLoad source turn"
    );

    assert_eq!(
        vm.eval("__childInlineClassicThrowEvents.join('|')")
            .expect("child inline classic throw events should evaluate"),
        "child-error:Uncaught Error: child-boom:child-boom:true|after-inline:true|dcl|load"
    );
}

async fn spawn_child_external_classic_frame_script_job_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind child external classic frame job server");
    let addr = listener
        .local_addr()
        .expect("child external classic frame job server addr");
    let (path_tx, path_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept child external classic frame job request");
        let mut buffer = [0; 1024];
        let bytes_read = stream
            .read(&mut buffer)
            .await
            .expect("read child external classic frame job request");
        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("")
            .to_owned();
        let _ = path_tx.send(request_path);
        let body = r#"parent.__childExternalClassicJobEvents.push("external:" + (globalThis === self));
parent.__childExternalClassicJobEvents.push("external-current:" + document.currentScript.id);
document.write("<span id='external-write'>written</span>");
parent.__childExternalClassicJobEvents.push(
  "external-write:" + (document.getElementById("external-write") !== null)
);
document.currentScript.addEventListener("load", () => {
  parent.__childExternalClassicJobEvents.push("script-load");
  document.open();
});
globalThis.__childExternalClassicValue = 73;"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write child external classic frame job response");
    });
    (format!("http://{addr}/child-classic.js"), path_rx, server)
}

async fn spawn_gated_media_resource_server(
    status: u16,
) -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gated media resource server");
    let addr = listener
        .local_addr()
        .expect("gated media resource server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept gated media resource request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream
                .read(&mut buffer)
                .await
                .expect("read gated media resource request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let _ = request_tx.send(String::from_utf8_lossy(&request).into_owned());
        let _ = release_rx.await;
        let (status_text, body) = if status == 200 {
            ("OK", "media-body")
        } else {
            ("Not Found", "")
        };
        let response = format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: video/webm\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    });
    (
        format!("http://{addr}/media"),
        request_rx,
        release_tx,
        server,
    )
}

async fn spawn_gated_image_resource_server(
    status: u16,
) -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gated image resource server");
    let addr = listener
        .local_addr()
        .expect("gated image resource server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept gated image resource request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream
                .read(&mut buffer)
                .await
                .expect("read gated image resource request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let _ = request_tx.send(String::from_utf8_lossy(&request).into_owned());
        let _ = release_rx.await;
        const ONE_BY_ONE_GIF: &[u8] = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;";
        let (status_text, content_type, body): (&str, &str, &[u8]) = if status == 200 {
            ("OK", "image/gif", ONE_BY_ONE_GIF)
        } else {
            ("Not Found", "image/png", &[])
        };
        let response_head = format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response_head.as_bytes()).await;
        let _ = stream.write_all(body).await;
    });
    (
        format!("http://{addr}/image.png"),
        request_rx,
        release_tx,
        server,
    )
}

async fn spawn_gated_font_resource_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gated font resource server");
    let addr = listener
        .local_addr()
        .expect("gated font resource server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept gated font resource request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream
                .read(&mut buffer)
                .await
                .expect("read gated font resource request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let _ = request_tx.send(String::from_utf8_lossy(&request).into_owned());
        let _ = release_rx.await;
        const TEST_FONT: &[u8] =
            include_bytes!("../../../../moli-layout/tests/fixtures/moli-ahem.woff2");
        let response_head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: font/woff2\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            TEST_FONT.len()
        );
        let _ = stream.write_all(response_head.as_bytes()).await;
        let _ = stream.write_all(TEST_FONT).await;
    });
    (
        format!("http://{addr}/print-only.woff2"),
        request_rx,
        release_tx,
        server,
    )
}

async fn spawn_imported_font_graph_server() -> (
    Url,
    tokio::sync::mpsc::UnboundedReceiver<String>,
    tokio::sync::watch::Sender<bool>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind imported-font graph server");
    let address = listener.local_addr().expect("imported-font server address");
    let base_url = Url::parse(&format!("http://{address}/")).expect("imported-font base URL");
    let (request_tx, request_rx) = tokio::sync::mpsc::unbounded_channel();
    let (font_release_tx, font_release_rx) = tokio::sync::watch::channel(false);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted.expect("accept imported-font request");
                    let request_tx = request_tx.clone();
                    let mut font_release_rx = font_release_rx.clone();
                    connections.spawn(async move {
                        let mut request = Vec::new();
                        let mut buffer = [0_u8; 2048];
                        loop {
                            let read = stream
                                .read(&mut buffer)
                                .await
                                .expect("read imported-font request");
                            assert_ne!(read, 0, "imported-font request ended before headers");
                            request.extend_from_slice(&buffer[..read]);
                            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                        let request_head = String::from_utf8_lossy(&request);
                        let target = request_head
                            .lines()
                            .next()
                            .and_then(|line| line.split_ascii_whitespace().nth(1))
                            .expect("imported-font request target")
                            .to_owned();
                        let _ = request_tx.send(target.clone());

                        const ROOT_CSS: &[u8] = b"@import '../theme/imported.css';";
                        const IMPORTED_CSS: &[u8] = br#"
                            @font-face {
                                font-family: ImportedAhem;
                                src: url('./fonts/imported.woff2') format('woff2');
                            }
                            #font-probe {
                                display: inline-block;
                                font-family: ImportedAhem;
                                font-size: 20px;
                                line-height: 20px;
                                white-space: pre;
                            }
                        "#;
                        const TEST_FONT: &[u8] = include_bytes!(
                            "../../../../moli-layout/tests/fixtures/moli-ahem.woff2"
                        );
                        let (status, content_type, body, gate_font): (
                            &str,
                            &str,
                            &[u8],
                            bool,
                        ) = match target.as_str() {
                            "/css/root.css" => ("200 OK", "text/css", ROOT_CSS, false),
                            "/theme/imported.css" => {
                                ("200 OK", "text/css", IMPORTED_CSS, false)
                            }
                            "/theme/fonts/imported.woff2" => {
                                ("200 OK", "font/woff2", TEST_FONT, true)
                            }
                            "/css/fonts/imported.woff2" => {
                                ("404 Not Found", "text/plain", b"wrong font base", false)
                            }
                            _ => ("404 Not Found", "text/plain", b"not found", false),
                        };
                        if gate_font {
                            while !*font_release_rx.borrow() {
                                if font_release_rx.changed().await.is_err() {
                                    return;
                                }
                            }
                        }
                        let response_head = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len(),
                        );
                        stream
                            .write_all(response_head.as_bytes())
                            .await
                            .expect("write imported-font response head");
                        stream
                            .write_all(body)
                            .await
                            .expect("write imported-font response body");
                    });
                }
                _ = &mut shutdown_rx => break,
            }
        }
        drop(request_tx);
        while let Some(result) = connections.join_next().await {
            result.expect("imported-font connection task");
        }
    });
    (base_url, request_rx, font_release_tx, shutdown_tx, server)
}

async fn spawn_gated_text_track_resource_server(
    status: u16,
) -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gated text-track resource server");
    let addr = listener
        .local_addr()
        .expect("gated text-track resource server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept gated text-track resource request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream
                .read(&mut buffer)
                .await
                .expect("read gated text-track resource request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let _ = request_tx.send(String::from_utf8_lossy(&request).into_owned());
        let _ = release_rx.await;
        let (status_text, body) = if status == 200 {
            (
                "OK",
                "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nnetwork cue\n",
            )
        } else {
            ("Not Found", "")
        };
        let response = format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: text/vtt\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    });
    (
        format!("http://{addr}/captions.vtt"),
        request_rx,
        release_tx,
        server,
    )
}

async fn spawn_gated_module_resource_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gated module resource server");
    let addr = listener
        .local_addr()
        .expect("gated module resource server addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept gated module resource request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream
                .read(&mut buffer)
                .await
                .expect("read gated module resource request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let _ = request_tx.send(String::from_utf8_lossy(&request).into_owned());
        let _ = release_rx.await;
        let body = "export default 1;";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    });
    (
        format!("http://{addr}/slow-module.mjs"),
        request_rx,
        release_tx,
        server,
    )
}

async fn spawn_child_external_parser_module_ready_lane_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind child external parser module server");
    let addr = listener
        .local_addr()
        .expect("child external parser module server addr");
    let (path_tx, path_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept child external parser module request");
        let mut buffer = [0; 1024];
        let bytes_read = stream
            .read(&mut buffer)
            .await
            .expect("read child external parser module request");
        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("")
            .to_owned();
        let _ = path_tx.send(request_path);
        let body = r#"parent.__childExternalParserModuleEvents.push("module:" + (globalThis === self));
globalThis.__childExternalParserModuleValue = 188;"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write child external parser module response");
    });
    (
        format!("http://{addr}/child-parser-module.js"),
        path_rx,
        server,
    )
}

async fn spawn_child_external_classic_source_error_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind child external classic source error server");
    let addr = listener
        .local_addr()
        .expect("child external classic source error server addr");
    let (path_tx, path_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept child external classic source error request");
        let mut buffer = [0; 1024];
        let bytes_read = stream
            .read(&mut buffer)
            .await
            .expect("read child external classic source error request");
        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("")
            .to_owned();
        let _ = path_tx.send(request_path);
        let response = "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write child external classic source error response");
    });
    (format!("http://{addr}/missing-classic.js"), path_rx, server)
}

#[tokio::test]
async fn child_window_object_event_listener_uses_object_relevant_realm() {
    let mut vm = new_storage_test_vm("https://child-object-listener-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__objectListenerFrame = frame;
})()
"#,
    )
    .expect("child object listener realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child object listener realm setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child object listener realm setup",
    );
    let child_handle = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        realm.child_handle
    };
    vm.eval(
        r#"
(() => {
  const listener = globalThis.__objectListenerFrame.contentWindow.Function("return {}")();
  listener.handleEvent = function() {};
  globalThis.__objectListenerFrame.contentWindow.addEventListener(
    "object-listener-realm",
    listener
  );
})()
"#,
    )
    .expect("parent should register child object listener on child window");

    let identities = vm
        ._context_host
        .borrow()
        .child_window_event_callback_identities_for_test(child_handle, "object-listener-realm");
    assert_eq!(
        identities.len(),
        1,
        "object EventListener should register exactly one callback record"
    );
    let (relevant, incumbent) = identities[0];
    assert_eq!(
        relevant.map(|identity| identity.dispatch_scope()),
        Some(crate::native_bridge::OwnerDispatchScope::Child(
            child_handle
        )),
        "callback-interface relevant realm should come from the listener object"
    );
    assert_eq!(
        incumbent.map(|identity| identity.dispatch_scope()),
        Some(crate::native_bridge::OwnerDispatchScope::Top),
        "object EventListener callback relevant realm should come from the listener object, \
         while incumbent realm should come from the registration call"
    );
}

#[tokio::test]
async fn child_window_event_handler_property_uses_child_relevant_realm() {
    let mut vm = new_storage_test_vm("https://child-handler-property-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child handler property realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child handler property realm setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child handler property realm setup",
    );
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  onload = function() {};
})()
"#,
    )
    .expect("child onload property assignment should evaluate");

    let identities = vm
        ._context_host
        .borrow()
        .child_window_event_callback_identities_for_test(child_handle, "load");
    assert_eq!(
        identities.len(),
        1,
        "child event-handler property should register exactly one callback record"
    );
    let (relevant, incumbent) = identities[0];
    assert_eq!(relevant, incumbent);
    assert_eq!(
        relevant.map(|identity| identity.dispatch_scope()),
        Some(crate::native_bridge::OwnerDispatchScope::Child(
            child_handle
        )),
        "child window event-handler property should register in the child callback realm"
    );
}

#[tokio::test]
async fn child_window_event_handler_mutation_preserves_registration_snapshot_semantics() {
    let mut vm = new_storage_test_vm("https://child-handler-mutation.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__childHandlerMutationTrace = "";
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  body.appendChild(document.createElement("iframe"));
})()
"#,
    )
    .expect("child handler mutation setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child handler mutation setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child handler mutation setup");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .map(|realm| realm.owner_realm_id)
        .expect("child handler mutation realm should exist");

    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  const trace = [];
  let dispatchNumber = 0;
  addEventListener("click", () => {
    dispatchNumber += 1;
    trace.push("before:" + dispatchNumber);
    if (dispatchNumber === 1) {
      onclick = () => {
        trace.push("replacement:1");
        return false;
      };
    } else if (dispatchNumber === 2) {
      onclick = null;
      onclick = () => trace.push("readded:3");
    }
  });
  onclick = () => trace.push("stale");
  addEventListener("click", () => trace.push("after:" + dispatchNumber));

  for (let index = 0; index < 3; index += 1) {
    const result = dispatchEvent(new Event("click", { cancelable: true }));
    trace.push("result:" + result);
  }
  parent.__childHandlerMutationTrace = trace.join(",");
})()
"#,
    )
    .expect("child handler mutation dispatch should evaluate");

    assert_eq!(
        vm.eval("globalThis.__childHandlerMutationTrace")
            .expect("child handler mutation trace should evaluate"),
        "before:1,replacement:1,after:1,result:false,\
         before:2,after:2,result:true,\
         before:3,after:3,readded:3,result:true",
        "non-null replacement must keep the existing registration slot, while remove and re-add \
         must create a registration excluded from the active dispatch snapshot"
    );
}

#[tokio::test]
async fn child_frame_realm_location_navigation_queues_child_navigation() {
    let mut vm = new_storage_test_vm("https://child-location-navigation.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child location navigation setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child location navigation setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child location navigation setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  location.href = "data:text/html,<!doctype html><p>child</p>";
})()
"#,
    )
    .expect("child location navigation should evaluate");

    assert!(
        !vm.has_pending_location_navigation(),
        "child location navigation must not queue top-level pending navigation"
    );
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(child_handle)
        .expect("child location navigation should queue pending child navigation");
    match pending {
        crate::native_bridge::ChildBrowsingContextBootstrap::Url(url) => {
            assert_eq!(url.as_str(), "data:text/html,<!doctype html><p>child</p>");
        }
        other => panic!("child location navigation should use URL bootstrap, got {other:?}"),
    }
    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "child navigation should wake the dedicated commit source"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "HostLoad must not claim or commit pending child navigation"
    );
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_pending_live_navigation_for_test(child_handle)
            .is_some(),
        "attempting HostLoad must leave the pending navigation untouched"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "NavigationCommit should consume the pending child navigation"
    );
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_pending_live_navigation_for_test(child_handle)
            .is_none(),
        "NavigationCommit should clear the pending navigation after commit"
    );
}

#[tokio::test]
async fn empty_srcdoc_keeps_document_url_separate_from_parent_fallback_base() {
    let mut vm = new_storage_test_vm("https://empty-srcdoc.test/path/page.html");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "empty-srcdoc";
  frame.srcdoc = "";
  body.appendChild(frame);
})()
"#,
    )
    .expect("empty srcdoc setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "empty srcdoc should commit about:srcdoc then HostLoad",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const doc = document.getElementById("empty-srcdoc").contentDocument;
  const relative = doc.createElement("a");
  relative.href = "asset.js";
  return [
    doc.URL,
    doc.baseURI,
    relative.href,
    doc.defaultView.navigation.entries().length,
    doc.defaultView.navigation.currentEntry.url
  ].join("|");
})()
"#,
        )
        .expect("empty srcdoc URL and base URL should evaluate"),
        "about:srcdoc|https://empty-srcdoc.test/path/page.html|https://empty-srcdoc.test/path/asset.js|1|about:srcdoc"
    );
}

#[tokio::test]
async fn committed_srcdoc_navigation_uses_document_url_and_appends_history_entry() {
    let mut vm = new_storage_test_vm("https://srcdoc-commit-history.test/path/page.html");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "srcdoc-commit-history";
  globalThis.__srcdocCommitHistorySource = URL.createObjectURL(
    new Blob(["source"], { type: "text/html" })
  );
  frame.src = __srcdocCommitHistorySource;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("committed child setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "ordinary child document should commit before srcdoc replacement",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.getElementById("srcdoc-commit-history");
  return [
    frame.contentDocument.URL === __srcdocCommitHistorySource,
    frame.contentWindow.navigation.entries().length
  ].join("|");
})()
"#,
        )
        .expect("ordinary child history should evaluate"),
        "true|1"
    );

    vm.eval(r#"document.getElementById("srcdoc-commit-history").srcdoc = "replacement""#)
        .expect("srcdoc replacement should queue");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "srcdoc replacement should commit before inspection",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.getElementById("srcdoc-commit-history");
  const doc = frame.contentDocument;
  return [
    doc.URL,
    doc.baseURI,
    frame.contentWindow.location.href,
    frame.contentWindow.navigation.entries().length,
    frame.contentWindow.navigation.entries()[0].url === __srcdocCommitHistorySource,
    frame.contentWindow.navigation.entries()[1].url
  ].join("|");
})()
"#,
        )
        .expect("srcdoc replacement URL and history should evaluate"),
        "about:srcdoc|https://srcdoc-commit-history.test/path/page.html|about:srcdoc|2|true|about:srcdoc"
    );

    vm.eval(r#"document.getElementById("srcdoc-commit-history").removeAttribute("srcdoc")"#)
        .expect("removing srcdoc should queue the src navigation");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "removing srcdoc should commit the original blob source",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.getElementById("srcdoc-commit-history");
  const doc = frame.contentDocument;
  return [
    doc !== null,
    doc.URL === __srcdocCommitHistorySource,
    doc.location.protocol,
    doc.body.textContent
  ].join("|");
})()
"#,
        )
        .expect("restored blob document should remain same-origin"),
        "true|true|blob:|source"
    );
}

#[tokio::test]
async fn srcdoc_replacement_keeps_old_owner_until_navigation_commit() {
    let mut vm = new_storage_test_vm("https://srcdoc-owner-transition.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "srcdoc-owner-transition";
  body.appendChild(frame);
})()
"#,
    )
    .expect("srcdoc transition setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "srcdoc transition setup should install initial about:blank",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "srcdoc transition initial child document",
    );
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .child_handle;
    let old_owner = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .map(|snapshot| {
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            )
        })
        .expect("initial about:blank should have a document owner");
    vm.eval(
        r#"
document.getElementById("srcdoc-owner-transition").srcdoc =
  "<!doctype html><p id='replacement'>replacement</p>";
"#,
    )
    .expect("srcdoc replacement should queue");
    let owner_before_commit = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .map(|snapshot| {
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            )
        })
        .expect("old document should remain installed while srcdoc navigation is pending");
    assert_eq!(old_owner, owner_before_commit);
    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "srcdoc setter should wake NavigationCommit instead of replacing inline"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "navigation scheduling must leave no old document-owned HostLoad delivery action"
    );

    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit)
    );
    let new_owner = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .map(|snapshot| {
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            )
        })
        .expect("srcdoc commit should install a replacement owner");
    assert_ne!(old_owner, new_owner);
    assert_eq!(old_owner.scheduler_lane_id, new_owner.scheduler_lane_id);
    assert!(
        vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::DocumentLifecycle
        ),
        "replacement commit should expose document-owned lifecycle work before HostLoad delivery"
    );
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "replacement must not queue HostLoad before its complete transition"
    );
    assert_eq!(
        vm.eval(
            "document.getElementById('srcdoc-owner-transition').contentDocument.body.textContent"
        )
        .expect("replacement srcdoc should be observable after commit"),
        "replacement"
    );
}

#[tokio::test]
async fn child_detach_retires_document_modulator_at_owner_boundary() {
    let mut vm = new_storage_test_vm("https://child-detach-owner-transition.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "detach-owner-transition";
  body.appendChild(frame);
})()
"#,
    )
    .expect("child detach transition setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child detach transition setup should install initial about:blank",
    )
    .await;
    let context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "child detach transition initial document",
    );
    let (child_handle, realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };
    let task_owner = vm
        ._context_host
        .borrow()
        .frame_owner_current_child_snapshot(child_handle)
        .map(|snapshot| {
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            )
        })
        .expect("child document should expose an owner");
    let _ = vm
        .child_document_modulator_store
        .take_or_create_document_modulator(task_owner.document_owner(), realm_id);

    vm.eval("document.getElementById('detach-owner-transition').remove()")
        .expect("child detach should evaluate");
    assert!(
        !vm.child_document_modulator_store
            .contains_execution_context(task_owner.document_owner()),
        "detach transition must retire ScriptVm modulator state at the enclosing owner boundary"
    );
}

#[tokio::test]
async fn child_window_onload_property_navigation_queues_child_navigation() {
    let mut vm = new_storage_test_vm("https://child-onload-navigation.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child onload navigation setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child onload navigation setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child onload navigation setup");
    let (child_handle, owner_realm_id) = {
        let realm = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child realm record should exist");
        (realm.child_handle, realm.owner_realm_id)
    };

    vm.eval_in_frame_realm(
        owner_realm_id,
        r#"
(() => {
  onload = () => {
    location.href = "data:text/html,<!doctype html><p>child-onload</p>";
  };
})()
"#,
    )
    .expect("child onload property assignment should evaluate");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime_ptr| {
        let global = scope.get_current_context().global(scope);
        let event_ctor = global
            .get(scope, crate::util::v8str(scope, "Event").into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .expect("Event constructor should be installed");
        let event = event_ctor
            .new_instance(scope, &[crate::util::v8str(scope, "load").into()])
            .expect("load Event should construct");
        unsafe { &mut *runtime_ptr }.dispatch_child_window_event(
            scope,
            child_handle,
            "load",
            event,
        );
        Ok(())
    })
    .expect("child load dispatch should complete");

    assert!(
        !vm.has_pending_location_navigation(),
        "child onload navigation must not queue top-level pending navigation"
    );
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(child_handle)
        .expect("child onload navigation should queue pending child navigation");
    match pending {
        crate::native_bridge::ChildBrowsingContextBootstrap::Url(url) => {
            assert_eq!(
                url.as_str(),
                "data:text/html,<!doctype html><p>child-onload</p>"
            );
        }
        other => panic!("child onload navigation should use URL bootstrap, got {other:?}"),
    }
}

#[tokio::test]
async fn child_window_object_event_listener_invokes_callback_object() {
    let mut vm = new_storage_test_vm("https://child-object-listener-dispatch.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__objectListenerDispatchFrame = frame;
})()
"#,
    )
    .expect("child object listener dispatch setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child object listener dispatch setup",
    )
    .await;
    vm.live_child_default_runtime_realm_inventory();

    let result = vm
        .eval(
            r#"
(() => {
  const frame = globalThis.__objectListenerDispatchFrame;
  const listener = frame.contentWindow.Function("return { calls: [] }")();
  listener.handleEvent = function(event) {
    this.calls.push(event.type + ":" + (this === listener));
  };
  frame.contentWindow.addEventListener("object-listener-dispatch", listener);
  frame.contentWindow.dispatchEvent(new Event("object-listener-dispatch"));
  return JSON.stringify(listener.calls);
})()
"#,
        )
        .expect("child object listener dispatch should evaluate");
    assert_eq!(result, r#"["object-listener-dispatch:true"]"#);
}

#[tokio::test]
async fn child_window_same_realm_event_listener_dispatches_on_current_stack() {
    let mut vm = new_storage_test_vm("https://child-same-realm-listener.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
})()
"#,
    )
    .expect("child same-realm listener setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child same-realm listener setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child same-realm listener setup");
    let owner_realm_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .owner_realm_id;

    let result = vm
        .eval_in_frame_realm(
            owner_realm_id,
            r#"
(() => {
  globalThis.__sameRealmListenerEvents = [];
  globalThis.addEventListener("same-realm-listener", function(event) {
    globalThis.__sameRealmListenerEvents.push(event.type + ":" + (this === event.currentTarget));
  });
  globalThis.dispatchEvent(new Event("same-realm-listener"));
  return JSON.stringify(globalThis.__sameRealmListenerEvents);
})()
"#,
        )
        .expect("child same-realm listener should dispatch in child frame realm");
    assert_eq!(result, r#"["same-realm-listener:true"]"#);
}

#[tokio::test]
async fn child_window_parent_realm_listener_uses_callback_relevant_realm() {
    let mut vm = new_storage_test_vm("https://child-parent-listener-route.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__parentRouteFrame = frame;
})()
"#,
    )
    .expect("child parent listener route setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child parent listener route setup",
    )
    .await;
    vm.eval(
        r#"
(() => {
  globalThis.__parentRouteEvents = [];
  globalThis.__parentRouteFrame.contentWindow.addEventListener(
    "parent-route",
    function(event) {
      globalThis.__parentRouteEvents.push(event.type + ":" + (this === event.currentTarget));
    }
  );
})()
"#,
    )
    .expect("parent listener should register on child window");
    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__parentRouteFrame.contentWindow.dispatchEvent(new Event("parent-route"));
  return JSON.stringify(globalThis.__parentRouteEvents);
})()
"#,
        )
        .expect("parent listener dispatch should evaluate");
    assert_eq!(result, r#"["parent-route:true"]"#);
}

#[test]
fn child_window_function_is_available_on_first_exposure() {
    let mut vm = new_storage_test_vm("https://child-function-request.test/");

    let first_result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__requestedChildFunctionWindow = frame.contentWindow;
  return [
    typeof globalThis.__requestedChildFunctionWindow.Function,
    globalThis.__requestedChildFunctionWindow.Function("return 33")(),
    globalThis.__requestedChildFunctionWindow === frame.contentWindow
  ].join("|");
})()
"#,
        )
        .expect("first contentWindow access should expose the real child realm");
    assert_eq!(
        first_result, "function|33|true",
        "first exposure must return the stable WindowProxy backed by the real child realm"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .window_execution_context_registry_counts_for_test(),
        (2, 2),
        "an exposed prebootstrapped child realm must be registered before its owner turn"
    );
    assert!(
        vm.has_pending_child_frame_realm_materialization(),
        "first exposure should enqueue the dedicated child realm-materialization source"
    );
    assert!(
        vm.run_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed"),
        "the owner source should claim one live child realm request"
    );
    assert_eq!(
        vm.child_frame_realm_store.len(),
        1,
        "the dedicated owner source should materialize the requested child FrameRealm"
    );

    let result = vm
        .eval("globalThis.__requestedChildFunctionWindow.Function('return 34')()")
        .expect("follow-up contentWindow.Function call should retain the child realm");
    assert_eq!(result, "34");
}

#[test]
fn child_document_open_preserves_prebootstrapped_execution_context() {
    let mut vm = new_storage_test_vm("https://prebootstrap-document-open.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "prebootstrap-document-open";
  body.appendChild(frame);
  void frame.contentWindow.Function;
})()
"#,
    )
    .expect("first child Window exposure should prebootstrap a context");
    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()
        .into_iter()
        .next()
        .expect("child browsing context should exist");
    let initial_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("initial child document owner should exist");
    let initial_context_token = vm
        .prebootstrapped_child_default_contexts
        .borrow()
        .get(&child_handle)
        .expect("child context should await owner promotion")
        .runtime_observable_context_token;

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById("prebootstrap-document-open");
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write("<body><p id='replacement'>replacement</p></body>");
  childDocument.close();
  return [
    childWindow === frame.contentWindow,
    childDocument === frame.contentDocument,
    childDocument.getElementById("replacement").textContent
  ].join("|");
})()
"#,
        )
        .expect("document.open should update the existing child LocalWindow context");
    assert_eq!(result, "true|true|replacement");

    while vm
        .run_child_realm_materialization_body_for_test()
        .expect("document.open child realm owner turn should succeed")
    {
        // Preserve the production ChildFrameTask FIFO if script work sits
        // between two exact-Document materialization tasks.
    }

    let replacement_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("replacement child document owner should exist");
    assert_eq!(
        replacement_owner.local_window_id,
        initial_owner.local_window_id
    );
    assert_ne!(replacement_owner.document_id, initial_owner.document_id);
    assert!(
        vm.prebootstrapped_child_default_contexts
            .borrow()
            .is_empty(),
        "document.open owner processing should promote the exposed child context"
    );
    let promoted_context = vm
        .child_frame_realm_store
        .iter_by_execution_context_id()
        .next()
        .map(|(_, context)| context)
        .expect("promoted child context should exist");
    assert_eq!(
        promoted_context.local_window_id,
        initial_owner.local_window_id
    );
    assert_eq!(
        promoted_context.runtime_observable_context_token, initial_context_token,
        "document.open must not replace the LocalWindow realm"
    );
}

#[test]
fn child_document_open_context_preflight_failure_preserves_current_document() {
    let mut vm = new_storage_test_vm("https://document-open-preflight.test/");

    let setup = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "document-open-preflight";
  body.appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.body.innerHTML = "<button id='old-child'>old</button>";
  globalThis.__preflightChildDocument = childDocument;
  globalThis.__preflightOldChild = childDocument.getElementById("old-child");
  globalThis.__preflightListenerRuns = 0;
  __preflightOldChild.addEventListener(
    "preflight-probe",
    () => __preflightListenerRuns++,
  );
  return "ready";
})()
"#,
        )
        .expect("child Document preflight fixture should evaluate");
    assert_eq!(setup, "ready");
    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()
        .into_iter()
        .next()
        .expect("child browsing context should exist");
    let initial_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("child Document owner should exist");
    vm._context_host
        .borrow_mut()
        .force_child_default_context_preflight_failure_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  __preflightChildDocument.open();
  __preflightOldChild.dispatchEvent(new Event("preflight-probe"));
  return [
    __preflightChildDocument.getElementById("old-child") === __preflightOldChild,
    __preflightOldChild.textContent,
    __preflightListenerRuns,
  ].join("|");
})()
"#,
        )
        .expect("failed replacement preflight should return the existing child Document");

    assert_eq!(result, "true|old|1");
    assert_eq!(
        vm._context_host
            .borrow()
            .current_child_document_task_owner(child_handle),
        Some(initial_owner),
        "context materialization failure must not rotate the child Document owner",
    );
}

#[test]
fn child_document_open_revalidates_owner_after_descendant_unload_reentry() {
    let mut vm = new_storage_test_vm("https://document-open-unload-reentry.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  const childDocument = frame.contentDocument;
  childDocument.body.innerHTML = "<p id='before'>before</p>";

  const descendant = childDocument.createElement("iframe");
  childDocument.body.appendChild(descendant);
  descendant.contentWindow.addEventListener("unload", () => {
    childDocument.open();
    childDocument.write("<p id='inner-open'>inner</p>");
    childDocument.close();
  });

  childDocument.open();
  childDocument.write("<p id='outer-open'>outer</p>");
  childDocument.close();

  return [
    childDocument.getElementById("before") === null,
    childDocument.getElementById("inner-open") === null,
    childDocument.getElementById("outer-open").textContent,
  ].join("|");
})()
"#,
        )
        .expect("reentrant descendant unload document.open should not stale-commit an owner plan");

    assert_eq!(
        result, "true|true|outer",
        "the outer document.open must resume on the current owner after unload-script reentry",
    );
}

#[test]
fn detached_first_exposure_retires_prebootstrapped_child_realm() {
    let mut vm = new_storage_test_vm("https://stale-child-function-request.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  const heldWindow = frame.contentWindow;
  frame.remove();
  return typeof heldWindow.Function;
})()
"#,
        )
        .expect("stale contentWindow materialization request setup should evaluate");
    assert_eq!(
        result, "function",
        "the real child realm remains usable by the current stack until owner retirement"
    );
    assert!(
        vm.run_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed"),
        "the owner source should consume and stale-drop the detached realm request"
    );
    assert!(
        !vm.run_child_realm_materialization_body_for_test()
            .expect("child realm materialization body should succeed"),
        "stale-drop must consume exactly one durable request"
    );
    assert_eq!(
        vm.child_frame_realm_store.len(),
        0,
        "the owner source must not materialize a child FrameRealm for a detached owner request"
    );
    assert!(
        vm.prebootstrapped_child_default_contexts
            .borrow()
            .is_empty(),
        "the owner source must detach and release an unclaimed realm for a removed LocalWindow"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .window_execution_context_registry_counts_for_test(),
        (1, 1),
        "stale-drop must retire the unclaimed child context binding and realm registration"
    );
}

#[tokio::test]
async fn child_default_bridge_ref_is_released_on_child_context_teardown() {
    let mut vm = new_storage_test_vm("https://child-bridge-ref-regression.test/");
    let initial_bridge_ref_count = vm._context_host.borrow().bridge_ref_count_for_test();

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "bridge-ref-child";
  body.appendChild(frame);
  return typeof frame.contentWindow.Function;
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "function");
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child bridge-ref teardown setup");
    let (child_context_ptr, child_realm_token): (
        *const v8::Global<v8::Context>,
        crate::native_bridge::RuntimeObservableContextToken,
    ) = {
        let context = vm
            .child_frame_realm_store
            .get(&child_context_id)
            .expect("child default context should be tracked");
        (
            &context.context as *const _,
            context.runtime_observable_context_token,
        )
    };
    let retained_child_cache = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(
            child_context_ptr,
            |scope, _runtime_ptr| {
                Ok(crate::native_bridge::identity::retain_context_wrapper_cache_for_test(scope))
            },
        )
        .expect("child wrapper cache should be retainable for regression testing");
    let child_wrappers = vm
        .eval_string_in_context_ptr_runtime_turn(
            child_context_ptr,
            r#"
(() => {
  void (document.body || document.documentElement);
  return "child-wrappers";
})()
"#,
            false,
        )
        .expect("child wrapper cache setup should evaluate");
    assert_eq!(child_wrappers, "child-wrappers");
    let child_wrapper_count = retained_child_cache.wrapper_entry_count_for_realm(child_realm_token);
    assert!(
        child_wrapper_count >= 1,
        "child context wrappers should populate its default-world cache partition: {child_wrapper_count}"
    );
    let top_realm_token = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
            Ok(
                crate::native_bridge::current_runtime_observable_context_token(scope)
                    .expect("top default context should have a runtime token"),
            )
        })
        .expect("top default context token should be readable");
    let top_wrapper_count = retained_child_cache.wrapper_entry_count_for_realm(top_realm_token);
    assert!(
        top_wrapper_count >= 1,
        "the shared default-world cache should contain live top-context wrappers"
    );
    assert!(
        vm.child_default_frame_id_for_execution_context_id(child_context_id)
            .is_some(),
        "child default execution context should map to a live frame"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count + 1,
        "creating a child default context should retain exactly one V8 bridge ref-count token"
    );

    let removed = vm
        .eval(
            r#"
(() => {
  document.getElementById("bridge-ref-child").remove();
  return "removed";
})()
"#,
        )
        .expect("child frame removal should evaluate");
    assert_eq!(removed, "removed");
    assert!(
        vm.live_child_default_runtime_realm_inventory().is_empty(),
        "removed iframe should not keep a live child default context"
    );
    assert_eq!(
        retained_child_cache.wrapper_entry_count_for_realm(child_realm_token),
        0,
        "destroying a child default context must retire its strong wrapper entries"
    );
    assert!(
        retained_child_cache.wrapper_entry_count_for_realm(top_realm_token) >= top_wrapper_count,
        "destroying a child context must preserve wrappers owned by the live top realm"
    );
    assert_eq!(
        vm._context_host.borrow().bridge_ref_count_for_test(),
        initial_bridge_ref_count,
        "child context teardown must drop its JsContextHostBridgeRef instead of retaining it until ScriptVm drop"
    );
}

#[tokio::test]
async fn child_default_global_keeps_native_function_constructor() {
    let mut vm = new_storage_test_vm("https://child-native-function.test/");

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  return "created";
})()
"#,
        )
        .expect("child frame setup should evaluate");
    assert_eq!(created, "created");

    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "child native Function setup",
    )
    .await;
    let child_context_id =
        materialize_single_child_default_realm_for_test(&mut vm, "child native Function setup");

    let result = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  const make = Function("return 7");
  return JSON.stringify({
    functionResult: make(),
    evalResult: eval("1 + 2"),
    globalThisIsSelf: globalThis === self
  });
})()
"#,
        )
        .expect("child default Function/eval intrinsics should evaluate");
    assert_eq!(
        result,
        r#"{"functionResult":7,"evalResult":3,"globalThisIsSelf":true}"#
    );
}

#[tokio::test]
async fn first_exposed_child_realm_survives_lifecycle_bootstrap() {
    let mut vm = new_storage_test_vm("https://child-window-function-sync.test/");

    let created = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = "<body>replacement child realm</body>";
  body.appendChild(frame);
  globalThis.__childFunctionWindow = frame.contentWindow;
  return typeof globalThis.__childFunctionWindow.Function;
})()
"#,
        )
        .expect("child Function sync setup should evaluate");
    assert_eq!(
        created, "function",
        "first WindowProxy exposure must use the real child realm"
    );

    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "child Function sync setup should initialize through NavigationCommit then HostLoad",
    )
    .await;
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child default execution context should be created");
    let child_context_result = vm
        .eval_in_child_default_context(child_context_id, "Function('return 4')()")
        .expect("child native Function should evaluate in child realm");
    assert_eq!(child_context_result, "4");

    let result = vm
        .eval(
            r#"
(() => {
  const make = globalThis.__childFunctionWindow.Function("return 9");
  return JSON.stringify({
    constructorType: typeof globalThis.__childFunctionWindow.Function,
    result: make()
  });
})()
"#,
        )
        .expect("synced child Function should evaluate from contentWindow wrapper");
    assert_eq!(result, r#"{"constructorType":"function","result":9}"#);
}

#[tokio::test]
async fn child_realm_sync_repairs_window_proxy_created_after_materialization() {
    let mut vm = new_storage_test_vm("https://child-window-function-late-wrapper.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__lateChildFunctionFrame = frame;
})()
"#,
    )
    .expect("late child Function wrapper setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "late child Function wrapper setup",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "late child Function wrapper setup",
    );
    let child_context_result = vm
        .eval_in_child_default_context(child_context_id, "Function('return 12')()")
        .expect("child native Function should evaluate before wrapper creation");
    assert_eq!(child_context_result, "12");

    vm.eval("globalThis.__lateChildFunctionWindow = __lateChildFunctionFrame.contentWindow")
        .expect("late contentWindow wrapper should evaluate");

    let result = vm
        .eval("__lateChildFunctionWindow.Function('return 13')()")
        .expect("late WindowProxy wrapper should sync child Function constructor");
    assert_eq!(result, "13");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_protocol_retains_connected_style_event_bodies_without_numeric_termination() {
    const STYLE_COUNT: usize = 129;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader(
        "https://runtime-connected-style-owner-turn.test/",
        &loader,
    );
    let expression = format!(
        r#"
globalThis.__runtimeConnectedStyleLoadCount = 0;
const root = document.documentElement || document.appendChild(document.createElement("html"));
const head = document.head || root.appendChild(document.createElement("head"));
for (let index = 0; index < {STYLE_COUNT}; index += 1) {{
  const style = document.createElement("style");
  style.textContent = `:root{{--runtime-connected-style-${{index}}:${{index}}}}`;
  style.addEventListener("load", () => {{
    globalThis.__runtimeConnectedStyleLoadCount += 1;
  }});
  head.appendChild(style);
}}
globalThis.__runtimeConnectedStyleLoadCount;
"#
    );
    let message = serde_json::json!({
        "id": 17,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "awaitPromise": false,
            "returnByValue": true,
        }
    });

    let messages = vm
        .dispatch_inspector_protocol_message(&message.to_string())
        .expect("Runtime.evaluate should queue connected stylesheet events");
    let response = messages
        .iter()
        .find(|message| message["id"] == serde_json::json!(17))
        .expect("Runtime.evaluate response");
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!(0),
        "the protocol execution turn must not inline-dispatch connected stylesheet events"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "the typed source should expose the first connected stylesheet event body"
    );
    assert_eq!(
        vm.eval("String(globalThis.__runtimeConnectedStyleLoadCount)")
            .expect("first connected stylesheet event count should evaluate"),
        "1",
        "one body application should dispatch exactly one connected stylesheet event"
    );

    let mut dispatched = 1;
    while vm.apply_next_connected_style_event_body_for_test() {
        dispatched += 1;
    }
    assert_eq!(
        dispatched, STYLE_COUNT,
        "the typed source must retain bodies beyond the removed 128-item protocol drain"
    );
    assert_eq!(
        vm.eval("String(globalThis.__runtimeConnectedStyleLoadCount)")
            .expect("final connected stylesheet event count should evaluate"),
        STYLE_COUNT.to_string()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn linked_stylesheet_client_terminal_installs_source_before_its_load_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, _resource_completion_queue) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            "https://stylesheet-client-terminal.test/page.html",
            concat!(
                "<!doctype html><html><head>",
                "<link id='sheet' rel='stylesheet' ",
                "href='data:text/css,.target%7Bcolor%3Argb(1%2C2%2C3)%7D'>",
                "</head><body><div class='target'></div></body></html>",
            ),
            &loader,
        );
    vm.exec(
        r#"
        globalThis.__linkedStyleEvents = [];
        document.getElementById("sheet").addEventListener("load", () => {
          __linkedStyleEvents.push("load");
        });
        document.getElementById("sheet").addEventListener("error", () => {
          __linkedStyleEvents.push("error");
        });
        "#,
        None,
    )
    .expect("linked stylesheet listeners should install");

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            vm.wait_for_and_apply_stylesheet_networking_body_for_test(),
        )
        .await
        .expect("data stylesheet terminal should reach its Page Networking source")
    );

    assert_eq!(
        vm.eval("getComputedStyle(document.querySelector('.target')).color")
            .expect("linked stylesheet computed color"),
        "rgb(1, 2, 3)",
        "the exact client terminal must install the retained response body"
    );
    assert_eq!(
        vm.eval("__linkedStyleEvents.join(',')")
            .expect("pre-event linked stylesheet state"),
        "",
        "source installation must not synchronously dispatch the link event"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "linked stylesheet load event body should be ready"
    );
    assert_eq!(
        vm.eval("__linkedStyleEvents.join(',')")
            .expect("linked stylesheet event state"),
        "load"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn failed_linked_stylesheet_client_terminal_installs_empty_source_before_its_error_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, _resource_completion_queue) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            "https://failed-stylesheet-client-terminal.test/page.html",
            concat!(
                "<!doctype html><html><head>",
                "<link id='sheet' rel='stylesheet' ",
                "href='data:text/plain,.target%7Bcolor%3Argb(11%2C12%2C13)%7D'>",
                "</head><body><div class='target'></div></body></html>",
            ),
            &loader,
        );
    vm.exec(
        r#"
        globalThis.__failedLinkedStyleEvents = [];
        document.getElementById("sheet").addEventListener("load", () => {
          __failedLinkedStyleEvents.push("load");
        });
        document.getElementById("sheet").addEventListener("error", () => {
          __failedLinkedStyleEvents.push("error");
        });
        "#,
        None,
    )
    .expect("failed linked stylesheet listeners should install");

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            vm.wait_for_and_apply_stylesheet_networking_body_for_test(),
        )
        .await
        .expect("failed data stylesheet terminal should reach its Page Networking source")
    );

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
              sheetIsNull: document.getElementById("sheet").sheet === null,
              styleSheetCount: document.styleSheets.length,
              color: getComputedStyle(document.querySelector(".target")).color,
              events: __failedLinkedStyleEvents,
            })"#,
        )
        .expect("failed linked stylesheet state"),
        r#"{"sheetIsNull":false,"styleSheetCount":1,"color":"rgb(0, 0, 0)","events":[]}"#,
        "an unusable typed terminal must install an empty stylesheet without applying its body or synchronously dispatching"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "failed linked stylesheet error event body should be ready"
    );
    assert_eq!(
        vm.eval("__failedLinkedStyleEvents.join(',')")
            .expect("failed linked stylesheet event state"),
        "error"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn style_preload_client_terminal_dispatches_load_without_installing_source() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, _resource_completion_queue) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            "https://stylesheet-preload-client.test/page.html",
            concat!(
                "<!doctype html><html><head>",
                "<link id='preload' rel='preload' as='style' ",
                "href='data:text/css,.target%7Bcolor%3Argb(7%2C8%2C9)%7D'>",
                "</head><body><div class='target'></div></body></html>",
            ),
            &loader,
        );
    vm.exec(
        r#"
        globalThis.__stylePreloadEvents = [];
        document.getElementById("preload").addEventListener("load", () => {
          __stylePreloadEvents.push("load");
        });
        document.getElementById("preload").addEventListener("error", () => {
          __stylePreloadEvents.push("error");
        });
        "#,
        None,
    )
    .expect("style preload listeners should install");

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            vm.wait_for_page_task_executor_work_arrival_for_test(),
        )
        .await
        .expect("style preload terminal should wake the Page executor"),
        "style preload terminal should publish a Page task"
    );
    assert!(
        vm.apply_next_stylesheet_networking_body_for_test(),
        "style preload terminal should reach its Page Networking source"
    );

    assert_ne!(
        vm.eval("getComputedStyle(document.querySelector('.target')).color")
            .expect("preload target computed color"),
        "rgb(7, 8, 9)",
        "a preload client must not install the retained CSS source"
    );
    assert_eq!(
        vm.eval("String(document.styleSheets.length)")
            .expect("stylesheet list length"),
        "0"
    );
    assert_eq!(
        vm.eval("__stylePreloadEvents.join(',')")
            .expect("pre-event style preload state"),
        "",
        "the preload event must remain asynchronous"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "style preload event body should be ready"
    );
    assert_eq!(
        vm.eval("__stylePreloadEvents.join(',')")
            .expect("style preload event state"),
        "load"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn ownerless_stylesheet_terminal_installs_for_a_late_link_client() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let stylesheet_url =
        Url::parse("data:text/css,.target%7Bcolor%3Argb(4%2C5%2C6)%7D").expect("stylesheet URL");
    let (mut vm, _resource_completion_queue) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            "https://ownerless-stylesheet.test/page.html",
            concat!(
                "<!doctype html><html><head>",
                "<link id='sheet' rel='stylesheet' ",
                "href='data:text/css,.target%7Bcolor%3Argb(4%2C5%2C6)%7D'>",
                "</head><body><div class='target'></div></body></html>",
            ),
            &loader,
        );
    vm.exec(
        r#"
        globalThis.__ownerlessStyleEvents = [];
        document.getElementById("sheet").addEventListener("load", () => {
          __ownerlessStyleEvents.push("load");
        });
        document.getElementById("sheet").addEventListener("error", () => {
          __ownerlessStyleEvents.push("error");
        });
        "#,
        None,
    )
    .expect("linked stylesheet listeners should install");

    let speculative_fetch = vm
        .document_runtime
        .preload_stylesheet(
            stylesheet_url,
            crate::stylesheet_blocking::StylesheetFetchOptions::default(),
        )
        .expect("response CSP should admit the ownerless resource");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            vm.wait_for_and_apply_stylesheet_networking_body_for_test(),
        )
        .await
        .expect("ownerless data stylesheet should reach its Page Networking source")
    );
    assert!(
        speculative_fetch
            .terminal()
            .is_some_and(|terminal| terminal.is_ready())
    );

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    assert!(
        !vm.apply_next_stylesheet_networking_body_for_test(),
        "late client attachment must not enqueue another physical network terminal"
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.querySelector('.target')).color")
            .expect("late linked stylesheet computed color"),
        "rgb(4, 5, 6)"
    );
    assert_eq!(
        vm.eval("__ownerlessStyleEvents.join(',')")
            .expect("pre-event linked stylesheet state"),
        "",
        "late terminal delivery must keep the link event asynchronous"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "linked stylesheet load event body should be ready"
    );
    assert_eq!(
        vm.eval("__ownerlessStyleEvents.join(',')")
            .expect("linked stylesheet event state"),
        "load"
    );
}

#[test]
fn connected_stylesheet_plan_commits_its_lease_before_same_turn_apply() {
    let mut vm = new_parsed_test_vm(
        "https://style-plan-commit.test/",
        concat!(
            "<!doctype html><html><head>",
            "<style id='sheet'>body { color: black; }</style>",
            "</head><body></body></html>",
        ),
    );
    let owner = vm
        .current_main_document_task_owner()
        .expect("main Document owner");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.replace_document_resource_runtime(&loader);
    let mut prepared = vm.document_runtime.prepare_initial_connected_style_loads();
    assert_eq!(prepared.len(), 1);
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "the pure prepare phase must not touch the Document load gate"
    );

    let prepared = prepared.pop().expect("one prepared style owner");
    let inline_source = vm
        ._context_host
        .borrow()
        .owner_style_sheet_processing_source(prepared.owner());
    let admission = vm
        ._context_host
        .borrow_mut()
        .commit_connected_style_load_event_plan(prepared.event_plan())
        .expect("the current main Document should commit the style lease");
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(true),
        "commit must acquire the lease synchronously, before runtime apply"
    );

    let host_ptr = vm._context_host.as_ref().as_ptr();
    vm.document_runtime.apply_prepared_connected_style_load(
        prepared,
        inline_source,
        admission,
        host_ptr,
    );
    let ready = vm
        .take_next_connected_style_event_body_for_test()
        .expect("same-turn apply should publish the inline style event")
        .into_ready();
    assert_eq!(
        ready
            .load_event_binding()
            .expect("stylesheet ready event must retain the committed lease")
            .owner(),
        owner
    );
}

#[test]
fn main_connected_style_event_delays_complete_until_event_task_settles() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm(
        "https://main-style-lifecycle.test/",
        concat!(
            "<!doctype html><html><head>",
            "<style id='sheet'>body { color: black; }</style>",
            "</head><body></body></html>",
        ),
    );
    vm.replace_document_resource_runtime(&loader);
    vm.exec(
        r#"
        globalThis.__mainStyleLifecycleEvents = [];
        document.getElementById("sheet").addEventListener("load", () => {
          __mainStyleLifecycleEvents.push(`style:${document.readyState}`);
        });
        window.addEventListener("load", () => {
          __mainStyleLifecycleEvents.push(`window:${document.readyState}`);
        });
        "#,
        None,
    )
    .expect("main style lifecycle listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");

    vm.queue_initial_connected_style_loads_for_current_owner();
    let ready = vm
        .take_next_connected_style_event_body_for_test()
        .expect("inline stylesheet should queue one ready element event");
    let ready = ready.into_ready();
    let binding = ready
        .load_event_binding()
        .expect("main style event should retain its exact lifecycle binding");
    assert_eq!(
        binding.element(),
        vm.document_runtime
            .get_element_by_id("sheet")
            .expect("stylesheet link")
    );
    assert_eq!(binding.owner(), owner);

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the exact style event binding must delay complete after DOMContentLoaded"
    );

    assert!(vm.dispatch_connected_style_load(ready));
    assert_eq!(
        vm.eval("__mainStyleLifecycleEvents.join('|')")
            .expect("style event trace"),
        "style:interactive",
        "the element event must run before window load"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "dispatching the event must not release its load delay early"
    );

    assert_eq!(
        vm.settle_connected_style_load(Some(binding)),
        crate::page_task_queue::PageConnectedStyleLoadDelayEffect::ReleasedExactBinding
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "settling the event task must release the exact lifecycle delay"
    );
}

#[tokio::test]
async fn main_static_image_event_delays_complete_and_runs_before_window_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-image-lifecycle.test/",
        concat!(
            "<!doctype html><html><head></head><body>",
            "<img id='hero' src='data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'>",
            "</body></html>",
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__mainImageLifecycleEvents = [];
        document.getElementById("hero").addEventListener("load", () => {
          __mainImageLifecycleEvents.push(`image:${document.readyState}`);
        });
        window.addEventListener("load", () => {
          __mainImageLifecycleEvents.push(`window:${document.readyState}`);
        });
        "#,
        None,
    )
    .expect("main image lifecycle listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let image = vm
        .document_runtime
        .get_element_by_id("hero")
        .expect("static image");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should register static images");
    let pending = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("static image should own one pending event");
    assert!(matches!(
        pending.owner(),
        crate::native_bridge::PendingImageLoadEventOwner::Main(binding)
            if binding.owner() == owner
                && binding.element() == image
                && binding.load_delay_token().is_some()
    ));

    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the image event task must delay complete without a pending-image scan"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("the image event owner turn should run")
    );
    assert_eq!(
        vm.eval("__mainImageLifecycleEvents.join('|')")
            .expect("image event trace"),
        "image:interactive",
        "the image event must be a separate turn before Window load"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "the image event callback must settle its exact lifecycle delay"
    );
    assert!(
        vm.dispatch_main_document_window_load_lifecycle(owner)
            .expect("main Window load lifecycle should apply")
            .is_none()
    );
    assert_eq!(
        vm.eval("__mainImageLifecycleEvents.join('|')")
            .expect("complete image lifecycle trace"),
        "image:interactive|window:complete",
        "DCL, the image DOM task, and Window load must remain separate ordered turns"
    );
}

#[tokio::test]
async fn main_document_replacement_retires_pending_image_request_sequence() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-image-replacement.test/",
        "<!doctype html><html><head></head><body></body></html>",
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__staleMainImageEventCount = 0;
        const image = document.createElement("img");
        image.id = "stale-image";
        image.src = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
        image.addEventListener("load", () => {
          globalThis.__staleMainImageEventCount += 1;
        });
        document.body.appendChild(image);
        "#,
        None,
    )
    .expect("stale main image should queue");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("retired main document owner");
    let image = vm
        .document_runtime
        .get_element_by_id("stale-image")
        .expect("stale image handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_some()
    );

    vm.exec(
        "document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();",
        None,
    )
    .expect("main document replacement should complete");
    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_ne!(retired_owner, current_owner);
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none()
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("stale image task should be consumed without dispatch")
    );
    assert_eq!(
        vm.eval("String(globalThis.__staleMainImageEventCount)")
            .expect("stale image event count"),
        "0"
    );
    assert_eq!(
        vm.current_main_document_task_owner(),
        Some(current_owner),
        "stale image event must not replace or mutate the new owner"
    );
}

#[tokio::test]
async fn web_font_requests_and_registration_follow_effective_stylesheet_media() {
    let (font_url, mut request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
    let document_url = font_url.replace("/print-only.woff2", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, mut resource_completions) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  const style = document.createElement("style");
  style.media = "print";
  style.textContent = `
    @font-face {{ font-family: PrintOnly; src: url({font_url:?}); }}
  `;
  (document.head || document.documentElement || document).appendChild(style);
  (document.body || document.documentElement).style.fontFamily = "PrintOnly, sans-serif";
}})()
"#
    ))
    .expect("print-only web-font fixture should evaluate");

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("screen layout refresh should succeed")
    );
    assert_eq!(vm.document_web_font_counts_for_test(), (0, 0, 0));
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0,
        "a print-only font must not create a screen-media request or load delay"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut request_rx)
            .await
            .is_err(),
        "the server must not observe a request while owner media is ineffective"
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("print layout refresh should succeed")
    );
    assert_eq!(vm.document_web_font_counts_for_test(), (1, 0, 0));
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        1,
        "activating print media must admit exactly one font request"
    );
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), &mut request_rx)
        .await
        .expect("the effective font request should reach the server")
        .expect("font request channel should remain open");
    assert!(request.starts_with("GET /print-only.woff2 HTTP/1.1"));

    release_tx.send(()).expect("release font response");
    server.await.expect("font server should finish");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resource_completions.wait_for_arrival_without_timeout(),
        )
        .await
        .expect("web font completion should reach the Networking source")
    );
    let completion = resource_completions
        .pop_next_async_subresource_event()
        .expect("web font completion must retain its typed terminal");
    let _ = vm
        .complete_async_subresource_fetch_event_body(completion)
        .expect("web font completion should apply to its current document owner");
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (1, 1, 1),
        "an effective completed font must be registered for layout"
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("restored screen layout refresh should succeed")
    );
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (0, 0, 0),
        "leaving print media must remove the slot and registered layout font"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
}

#[tokio::test(flavor = "current_thread")]
async fn imported_web_font_keeps_its_response_base_slot_through_layout_reconciliation() {
    let (base_url, mut requests, font_release, shutdown, server) =
        spawn_imported_font_graph_server().await;
    let document_url = base_url.join("page.html").expect("document URL");
    let root_stylesheet_url = base_url.join("css/root.css").expect("root stylesheet URL");
    let html = format!(
        concat!(
            "<!doctype html><html><head>",
            "<link rel='stylesheet' href='{}'>",
            "</head><body><span id='font-probe'>MMMM</span></body></html>",
        ),
        root_stylesheet_url,
    );
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, mut resource_completions) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            document_url.as_str(),
            &html,
            &loader,
        );

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    for phase in ["root stylesheet", "import graph"] {
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                vm.wait_for_and_apply_stylesheet_networking_body_for_test(),
            )
            .await
            .unwrap_or_else(|_| panic!("{phase} completion timed out")),
            "{phase} completion should reach the stylesheet task source",
        );
    }

    let mut observed_paths = Vec::new();
    while !observed_paths
        .iter()
        .any(|path| path == "/theme/fonts/imported.woff2")
    {
        let path = tokio::time::timeout(std::time::Duration::from_secs(2), requests.recv())
            .await
            .expect("expected imported stylesheet/font request")
            .expect("imported-font server request stream");
        observed_paths.push(path);
    }
    assert!(observed_paths.iter().any(|path| path == "/css/root.css"));
    assert!(
        observed_paths
            .iter()
            .any(|path| path == "/theme/imported.css")
    );

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("layout reconciliation should succeed")
    );
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (1, 0, 0),
        "retained reconciliation must preserve the one pending response-base slot",
    );
    let fallback_probe_width = vm
        .eval("String(document.getElementById('font-probe').getBoundingClientRect().width)")
        .expect("fallback probe width")
        .parse::<f64>()
        .expect("numeric fallback probe width");
    if let Ok(Some(path)) =
        tokio::time::timeout(std::time::Duration::from_millis(150), requests.recv()).await
    {
        observed_paths.push(path);
    }
    while let Ok(path) = requests.try_recv() {
        observed_paths.push(path);
    }
    assert!(
        !observed_paths
            .iter()
            .any(|path| path == "/css/fonts/imported.woff2"),
        "layout reconciliation must not request the imported rule against the root CSS base: {observed_paths:?}",
    );

    font_release
        .send(true)
        .expect("release imported Ahem response");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resource_completions.wait_for_arrival_without_timeout(),
        )
        .await
        .expect("imported font completion should reach the resource source")
    );
    let completion = resource_completions
        .pop_next_async_subresource_event()
        .expect("imported font completion body");
    let _ = vm
        .complete_async_subresource_fetch_event_body(completion)
        .expect("the response-base slot should accept its font completion");
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (1, 1, 1),
        "the correct slot and registered layout font must survive reconciliation",
    );

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("web-font layout refresh should succeed")
    );
    let probe_width = vm
        .eval("String(document.getElementById('font-probe').getBoundingClientRect().width)")
        .expect("Ahem probe width")
        .parse::<f64>()
        .expect("numeric Ahem probe width");
    assert!(
        (probe_width - 48.0).abs() <= 0.05,
        "the four-glyph probe should use the deterministic Ahem fixture metrics after registration; got {probe_width}",
    );
    assert!(
        (probe_width - fallback_probe_width).abs() > 0.5,
        "registered Ahem metrics must replace the pending fallback metrics; before={fallback_probe_width}, after={probe_width}",
    );

    let _ = shutdown.send(());
    server.await.expect("imported-font server should finish");
}

#[tokio::test]
async fn pending_web_font_response_is_stale_after_stylesheet_media_stops_matching() {
    let (font_url, request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
    let document_url = font_url.replace("/print-only.woff2", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, mut resource_completions) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(&document_url, &loader);
    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    vm.eval(&format!(
        r#"
const style = document.createElement("style");
style.media = "print";
style.textContent = '@font-face {{ font-family: PendingPrint; src: url({font_url:?}); }}';
(document.head || document.documentElement || document).appendChild(style);
"#
    ))
    .expect("pending print font fixture should evaluate");

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("print layout refresh should succeed")
    );
    request_rx
        .await
        .expect("effective pending font request should reach the server");
    assert_eq!(vm.document_web_font_counts_for_test(), (1, 0, 0));

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("screen layout refresh should succeed")
    );
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (0, 0, 0),
        "the new resource generation must revoke a now-ineffective pending slot"
    );

    release_tx.send(()).expect("release stale font response");
    server.await.expect("font server should finish");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resource_completions.wait_for_arrival_without_timeout(),
        )
        .await
        .expect("stale font completion should reach the Networking source")
    );
    let completion = resource_completions
        .pop_next_async_subresource_event()
        .expect("stale font completion must retain its typed terminal");
    let _ = vm
        .complete_async_subresource_fetch_event_body(completion)
        .expect("stale font completion should settle without mutating layout fonts");
    assert_eq!(vm.document_web_font_counts_for_test(), (0, 0, 0));
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
}

#[tokio::test]
async fn far_lazy_image_network_request_waits_for_sampled_scroll_reveal() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(404).await;
    let document_url = image_url.replace("/image.png", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__lazyNetworkImageEvents = [];
  const spacer = document.createElement("div");
  spacer.style.height = "3000px";
  document.body.appendChild(spacer);
  const image = document.createElement("img");
  image.id = "far-lazy-network-image";
  image.loading = "lazy";
  image.width = 32;
  image.height = 32;
  image.onload = () => __lazyNetworkImageEvents.push("load:" + image.complete);
  image.onerror = () => __lazyNetworkImageEvents.push("error:" + image.complete);
  image.src = {image_url:?};
  document.body.appendChild(image);
}})()
"#
    ))
    .expect("far lazy network image setup should evaluate");
    let image = vm
        .document_runtime
        .get_element_by_id("far-lazy-network-image")
        .expect("far lazy image handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none(),
        "setting src must not start a lazy request without sampled eligibility"
    );

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("lazy-image layout refresh should succeed")
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none(),
        "a fragment beyond the Chromium-aligned preload margin must remain unadmitted"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0,
        "sampling far geometry must not create a network request"
    );

    vm.eval("document.getElementById('far-lazy-network-image').scrollIntoView()")
        .expect("scroll reveal should evaluate");
    let pending = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("live scroll delta should admit the far lazy image");
    assert!(pending.network_request_id().is_some());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        1
    );
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), request_rx)
        .await
        .expect("admitted lazy-image request should reach the server")
        .expect("lazy-image request channel should remain open");
    assert!(request.starts_with("GET /image.png HTTP/1.1"));

    release_tx.send(()).expect("release lazy image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "lazy image network completion",
    )
    .await;
    wait_for_image_load_event_executor_test_task(&mut vm, "lazy image decode completion").await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("lazy image load event turn")
    );
    assert_eq!(
        vm.eval("__lazyNetworkImageEvents.join('|')")
            .expect("lazy image event trace"),
        "error:true",
        "an admitted failed request must deliver its terminal instead of becoming lazy-deferred again"
    );
    server.await.expect("lazy image server should finish");
}

#[tokio::test]
async fn main_image_network_terminal_queues_later_load_event_and_retires_delay() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(200).await;
    let document_url = image_url.replace("/image.png", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__networkImageEvents = [];
  const image = document.createElement("img");
  image.id = "network-image";
  image.addEventListener("load", () => __networkImageEvents.push("load:" + image.complete));
  image.addEventListener("error", () => __networkImageEvents.push("error:" + image.complete));
  image.src = {image_url:?};
  (document.body || document.documentElement || document).appendChild(image);
}})()
"#
    ))
    .expect("network image setup should evaluate");
    let image = vm
        .document_runtime
        .get_element_by_id("network-image")
        .expect("network image handle");
    let pending = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("network image sequence");
    assert!(pending.network_request_id().is_some());
    assert!(matches!(
        pending.owner(),
        crate::native_bridge::PendingImageLoadEventOwner::Main(binding)
            if binding.element() == image && binding.load_delay_token().is_some()
    ));

    let request = request_rx.await.expect("image request should arrive");
    let request_lower = request.to_ascii_lowercase();
    assert!(request.starts_with("GET /image.png HTTP/1.1"));
    assert!(request_lower.contains("sec-fetch-dest: image"));
    assert!(request_lower.contains("sec-fetch-mode: no-cors"));
    assert!(request_lower.contains("accept: image/avif,image/webp"));
    assert_eq!(
        vm.eval("JSON.stringify({events: __networkImageEvents, complete: document.getElementById('network-image').complete})")
            .expect("pre-terminal image state"),
        r#"{"events":[],"complete":false}"#
    );

    release_tx.send(()).expect("release image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "main image network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__networkImageEvents.join('|')")
            .expect("image completion trace"),
        "",
        "resource completion may only enqueue the image event follow-up"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_some()
    );
    wait_for_image_load_event_executor_test_task(&mut vm, "network image decode completion").await;

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("network image load event turn")
    );
    assert_eq!(
        vm.eval("__networkImageEvents.join('|')")
            .expect("image load trace"),
        "load:true"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none()
    );
    server.await.expect("image server should finish");
}

#[tokio::test]
async fn removing_pending_image_preserves_request_and_document_delay_until_event() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(200).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &image_url.replace("/image.png", "/page"),
        &loader,
    );
    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__detachedImageEvents = [];
  const image = document.createElement("img");
  image.id = "detached-pending-image";
  image.onload = () => __detachedImageEvents.push("load");
  image.src = {image_url:?};
  (document.body || document.documentElement || document).appendChild(image);
}})()
"#
    ))
    .expect("pending image setup should evaluate");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let image = vm
        .document_runtime
        .get_element_by_id("detached-pending-image")
        .expect("pending image handle");
    let before = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("connected image sequence");
    assert!(matches!(
        before.owner(),
        crate::native_bridge::PendingImageLoadEventOwner::Main(binding)
            if binding.load_delay_token().is_some()
    ));
    request_rx
        .await
        .expect("pending image request should arrive");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false)
    );

    vm.eval("document.getElementById('detached-pending-image').remove()")
        .expect("pending image removal should evaluate");
    let after = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("detached image must retain its request sequence");
    assert_eq!(after.id(), before.id());
    assert_eq!(after.network_request_id(), before.network_request_id());
    assert!(matches!(
        after.owner(),
        crate::native_bridge::PendingImageLoadEventOwner::Main(binding)
            if binding.load_delay_token().is_some()
    ));
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "same-document removal must preserve the request's exact document delay"
    );

    release_tx
        .send(())
        .expect("release detached image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "detached image network completion",
    )
    .await;
    wait_for_image_load_event_executor_test_task(&mut vm, "detached image decode completion").await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("detached image event should release the document delay")
    );
    assert_eq!(
        vm.eval("__detachedImageEvents.join('|')")
            .expect("detached image event trace"),
        "load"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "the later detached image event must release its exact document delay"
    );
    server.await.expect("detached image server should finish");
}

#[tokio::test]
async fn main_image_http_failure_dispatches_error_only_on_later_event_turn() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(404).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &image_url.replace("/image.png", "/page"),
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__failedImageEvents = [];
  const image = document.createElement("img");
  image.id = "failed-image";
  image.onload = () => __failedImageEvents.push("load");
  image.onerror = () => __failedImageEvents.push("error:" + image.complete);
  image.src = {image_url:?};
  (document.body || document.documentElement || document).appendChild(image);
}})()
"#
    ))
    .expect("failed image setup should evaluate");
    let image = vm
        .document_runtime
        .get_element_by_id("failed-image")
        .expect("failed image handle");
    request_rx
        .await
        .expect("failed image request should arrive");
    release_tx.send(()).expect("release failed image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "failed image network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__failedImageEvents.join('|')")
            .expect("failed image completion trace"),
        ""
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_some()
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("failed image error event turn")
    );
    assert_eq!(
        vm.eval("__failedImageEvents.join('|')")
            .expect("failed image event trace"),
        "error:true"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none()
    );
    server.await.expect("failed image server should finish");
}

#[tokio::test]
async fn main_image_source_restart_cancels_exact_request_and_drops_stale_terminal() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(200).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &image_url.replace("/image.png", "/page"),
        &loader,
    );

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__restartedImageEvents = [];
  const image = document.createElement("img");
  image.id = "restarted-image";
  image.onload = () => __restartedImageEvents.push("load:" + image.currentSrc);
  image.onerror = () => __restartedImageEvents.push("error");
  image.src = {image_url:?};
  (document.body || document.documentElement || document).appendChild(image);
}})()
"#
    ))
    .expect("network image restart setup should evaluate");
    let image = vm
        .document_runtime
        .get_element_by_id("restarted-image")
        .expect("restarted image handle");
    let first = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("first image sequence");
    let first_request_id = first
        .network_request_id()
        .expect("first image sequence should bind its request");
    request_rx.await.expect("first image request should arrive");

    vm.eval(
        "document.getElementById('restarted-image').src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';",
    )
    .expect("replacement image source should evaluate");
    let second = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("replacement image sequence");
    assert_ne!(first.id(), second.id());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0,
        "source restart must cancel the exact pending image request"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: first_request_id,
        request_url: Url::parse(&image_url).expect("image request URL"),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Err("stale cancelled image completion".to_owned()),
    })
    .expect("stale cancelled image completion should be harmless");
    wait_for_image_load_event_executor_test_task(&mut vm, "replacement image decode completion")
        .await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("replacement image event turn")
    );
    assert!(
        vm.eval("__restartedImageEvents.join('|')")
            .expect("replacement image trace")
            .starts_with("load:data:image/gif")
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none()
    );

    release_tx.send(()).expect("release cancelled image server");
    server.await.expect("cancelled image server should finish");
}

#[test]
fn media_preload_none_defers_automatic_load_until_explicit_load() {
    let mut vm = new_parsed_test_vm(
        "https://media-preload-none.test/",
        concat!(
            "<!doctype html><html><head></head><body>",
            "<audio id='clip' preload='none'>",
            "<source src='data:audio/mpeg;base64,AA=='>",
            "</audio></body></html>",
        ),
    );
    vm.exec(
        r#"
        globalThis.__preloadNoneEvents = [];
        const clip = document.getElementById("clip");
        clip.addEventListener("loadstart", () => __preloadNoneEvents.push("loadstart"));
        "#,
        None,
    )
    .expect("preload-none listener should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("preload-none media element");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none(),
        "automatic resource selection must not start preload=none media"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ready: clip.readyState, network: clip.networkState, events: __preloadNoneEvents})"
        )
        .expect("deferred media state should evaluate"),
        r#"{"ready":0,"network":1,"events":[]}"#
    );
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "preload=none must not create a window-load delay"
    );

    vm.exec("clip.load()", None)
        .expect("explicit media load should evaluate");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some(),
        "load() must override preload=none deferral"
    );
    assert_eq!(
        vm.eval("clip.networkState")
            .expect("explicit media network state should evaluate"),
        "2"
    );
}

#[tokio::test]
async fn main_static_media_load_delays_complete_until_loadeddata_owner_turn() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-media-lifecycle.test/",
        concat!(
            "<!doctype html><html><head></head><body>",
            "<video id='clip' src='data:video/webm;base64,AA=='>",
            "</video></body></html>",
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__mainMediaLifecycleEvents = [];
        const clip = document.getElementById("clip");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => {
            __mainMediaLifecycleEvents.push(`${type}:${document.readyState}`);
          });
        }
        window.addEventListener("load", () => {
          __mainMediaLifecycleEvents.push(`window:${document.readyState}`);
        });
        "#,
        None,
    )
    .expect("main media lifecycle listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("static media element");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should register static media");
    let pending = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("static media should own one pending sequence");
    assert!(matches!(
        pending.owner(),
        crate::native_bridge::PendingMediaLoadOwner::Main {
            owner: binding_owner,
            load_delay: Some(binding),
        } if binding_owner == owner
            && binding.owner() == owner
            && binding.element() == media
            && binding.load_delay_token().is_some()
    ));

    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "media resource selection must delay complete without blocking DOMContentLoaded"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "loadstart owner turn").await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "loadedmetadata owner turn").await;
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "metadata must not release the media delay"
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "loadeddata owner turn").await;
    assert_eq!(
        vm.eval("__mainMediaLifecycleEvents.join('|')")
            .expect("main media event trace"),
        "loadstart:interactive|loadedmetadata:interactive|loadeddata:interactive"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "loadeddata dispatch must settle the exact media delay"
    );
    assert!(matches!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .expect("canplay continuation should remain queued")
            .owner(),
        crate::native_bridge::PendingMediaLoadOwner::Main {
            owner: binding_owner,
            load_delay: None,
        } if binding_owner == owner
    ));

    run_next_page_media_element_event_for_test(&mut vm, &loader, "canplay owner turn").await;
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none(),
        "the terminal media event must retire the sequence"
    );
}

#[tokio::test]
async fn main_media_source_mutation_replaces_sequence_without_stale_settlement() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-media-source-restart.test/",
        "<!doctype html><html><head></head><body><video id='clip'></video></body></html>",
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__mainMediaRestartEvents = [];
        const clip = document.getElementById("clip");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => __mainMediaRestartEvents.push(type));
        }
        clip.setAttribute("src", "data:video/webm;base64,AA==");
        "#,
        None,
    )
    .expect("first media source should queue");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("media element");
    let first = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("first source sequence");

    vm.exec(
        "document.getElementById('clip').setAttribute('src', 'data:video/webm;base64,AQ==');",
        None,
    )
    .expect("second media source should replace the sequence");
    let second = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("second source sequence");
    assert_ne!(first.id(), second.id());

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false)
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "stale first-source task should be consumed",
    )
    .await;
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .expect("new sequence must survive the stale callback")
            .id(),
        second.id()
    );
    assert_eq!(
        vm.eval("__mainMediaRestartEvents.join('|')")
            .expect("media restart event trace"),
        ""
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the stale callback must not settle the replacement sequence"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "new loadstart owner turn").await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "new loadedmetadata owner turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "new loadeddata owner turn").await;
    assert_eq!(
        vm.eval("__mainMediaRestartEvents.join('|')")
            .expect("new media event trace"),
        "loadstart|loadedmetadata|loadeddata"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true)
    );

    vm.exec(
        "document.getElementById('clip').removeAttribute('src');",
        None,
    )
    .expect("removing media source should cancel the canplay continuation");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none()
    );
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "cancelled canplay task should be consumed",
    )
    .await;
    assert_eq!(
        vm.eval("__mainMediaRestartEvents.join('|')")
            .expect("cancelled media event trace"),
        "loadstart|loadedmetadata|loadeddata"
    );
}

#[tokio::test]
async fn default_video_fetch_policy_synthesizes_lifecycle_without_a_network_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://media-policy.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__defaultVideoPolicyEvents = [];
  const video = document.createElement("video");
  video.id = "default-video-policy";
  for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {
    video.addEventListener(type, () => __defaultVideoPolicyEvents.push(type));
  }
  video.src = "https://media-policy.test/large-video.mp4";
  (document.body || document.documentElement || document).appendChild(video);
})()
"#,
    )
    .expect("default-policy video should initialize");

    let media = vm
        .document_runtime
        .get_element_by_id("default-video-policy")
        .expect("video handle");
    let pending = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("policy-skipped media keeps its event owner until terminal dispatch");
    assert!(
        pending.network_request_id().is_none(),
        "the default policy must reject video before binding a network request"
    );
    assert!(
        !vm.take_network_output().into_items().any(|item| {
            matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                    if request.resource_type()
                        == crate::types::SubresourceResourceType::Video
            )
        }),
        "policy-skipped video must not emit a requestWillBeSent source record"
    );

    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("default-policy video {phase} turn"),
        )
        .await;
    }
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __defaultVideoPolicyEvents, ready: document.getElementById('default-video-policy').readyState, network: document.getElementById('default-video-policy').networkState})"
        )
        .expect("default-policy video terminal state"),
        r#"{"events":["loadstart","loadedmetadata","loadeddata","canplay"],"ready":4,"network":1}"#
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none(),
        "the synthetic terminal must retire the exact media owner"
    );
}

#[tokio::test]
async fn media_bit_does_not_enable_an_html_video_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader
        .set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::MEDIA);
    let (mut vm, _resource_completion_queue) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(
            "https://media-bit-isolation.test/page.html",
            &loader,
        );

    vm.eval(
        r#"
const video = document.createElement("video");
video.id = "media-bit-video";
video.src = "https://media-bit-isolation.test/video.mp4";
(document.body || document.documentElement || document).appendChild(video);
"#,
    )
    .expect("media-bit video should initialize");

    let media = vm
        .document_runtime
        .get_element_by_id("media-bit-video")
        .expect("video handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some_and(|pending| pending.network_request_id().is_none()),
        "the generic Media bit must remain distinct from the Video bit"
    );
    assert!(!vm.take_network_output().into_items().any(|item| matches!(
        item,
        crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(_)
    )));
}

#[tokio::test]
async fn main_media_network_terminal_drives_readiness_on_later_event_turns() {
    let (media_url, request_rx, release_tx, server) = spawn_gated_media_resource_server(200).await;
    let document_url = media_url.replace("/media", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Video, true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__networkMediaEvents = [];
  const media = document.createElement("video");
  media.id = "network-media";
  for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {{
    media.addEventListener(type, () => __networkMediaEvents.push(type));
  }}
  media.src = {media_url:?};
  (document.body || document.documentElement || document).appendChild(media);
}})()
"#
    ))
    .expect("network media setup should evaluate");

    let request = request_rx.await.expect("media request should arrive");
    assert!(request.starts_with("GET /media HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: video")
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-mode: no-cors")
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "network media loadstart turn")
        .await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __networkMediaEvents, ready: document.getElementById('network-media').readyState, network: document.getElementById('network-media').networkState})"
        )
        .expect("pre-terminal media state should evaluate"),
        r#"{"events":["loadstart"],"ready":0,"network":2}"#,
        "loadstart must not synthesize media readiness before the network terminal"
    );
    assert!(
        !vm.apply_one_page_resource_terminal_owner_admission()
            .expect("pre-terminal media resource source should be readable")
    );

    release_tx.send(()).expect("release media response");
    wait_for_one_page_resource_completion_executor_test_turn(&mut vm, "media network completion")
        .await;
    assert_eq!(
        vm.eval("__networkMediaEvents.join('|')")
            .expect("media events after network completion should evaluate"),
        "loadstart",
        "resource completion may only enqueue the media event follow-up"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "network media metadata turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "network media data turn").await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "network media canplay turn")
        .await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __networkMediaEvents, ready: document.getElementById('network-media').readyState, network: document.getElementById('network-media').networkState})"
        )
        .expect("terminal media state should evaluate"),
        r#"{"events":["loadstart","loadedmetadata","loadeddata","canplay"],"ready":4,"network":1}"#
    );
    server.await.expect("media server should finish");
}

#[tokio::test]
async fn main_media_http_failure_dispatches_error_later_and_retires_delay() {
    let (media_url, request_rx, release_tx, server) = spawn_gated_media_resource_server(404).await;
    let document_url = media_url.replace("/media", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Audio, true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__failedMediaEvents = [];
  const media = document.createElement("audio");
  media.id = "failed-media";
  for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {{
    media.addEventListener(type, () => __failedMediaEvents.push(type));
  }}
  media.src = {media_url:?};
  (document.body || document.documentElement || document).appendChild(media);
}})()
"#
    ))
    .expect("failed media setup should evaluate");
    let media = vm
        .document_runtime
        .get_element_by_id("failed-media")
        .expect("failed media handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some(),
        "network media must own a lifecycle delay before completion"
    );
    let request = request_rx
        .await
        .expect("failed media request should arrive");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: audio")
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "failed media loadstart turn")
        .await;
    release_tx.send(()).expect("release failed media response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "failed media network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__failedMediaEvents.join('|')")
            .expect("failed media completion trace should evaluate"),
        "loadstart",
        "network failure must not dispatch the element error inline"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "failed media error turn").await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __failedMediaEvents, ready: document.getElementById('failed-media').readyState, network: document.getElementById('failed-media').networkState})"
        )
        .expect("failed media terminal state should evaluate"),
        r#"{"events":["loadstart","error"],"ready":0,"network":3}"#
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none(),
        "the error event turn must settle and retire the exact lifecycle sequence"
    );
    server.await.expect("failed media server should finish");
}

#[tokio::test]
async fn main_media_source_restart_cancels_exact_network_request_and_stale_terminal() {
    let (media_url, request_rx, release_tx, server) = spawn_gated_media_resource_server(200).await;
    let document_url = media_url.replace("/media", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Video, true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__restartedNetworkMediaEvents = [];
  const media = document.createElement("video");
  media.id = "restarted-network-media";
  for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay", "error"]) {{
    media.addEventListener(type, () => __restartedNetworkMediaEvents.push(type));
  }}
  media.src = {media_url:?};
  (document.body || document.documentElement || document).appendChild(media);
}})()
"#
    ))
    .expect("network media restart setup should evaluate");
    let media = vm
        .document_runtime
        .get_element_by_id("restarted-network-media")
        .expect("restarted network media handle");
    let first = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("first network media sequence");
    let first_request_id = first
        .network_request_id()
        .expect("first media sequence should bind its exact network request");
    request_rx.await.expect("first media request should arrive");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        1
    );

    vm.eval(
        "document.getElementById('restarted-network-media').src = 'data:video/webm;base64,AQ==';",
    )
    .expect("replacement media source should evaluate");
    let second = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("replacement media sequence");
    assert_ne!(first.id(), second.id());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0,
        "source restart must cancel and remove the exact pending media request"
    );

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id: first_request_id,
        request_url: Url::parse(&media_url).expect("media request URL"),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Err("stale cancelled media completion".to_owned()),
    })
    .expect("stale cancelled media completion should be harmless");

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "stale first media loadstart task",
    )
    .await;
    assert_eq!(
        vm.eval("__restartedNetworkMediaEvents.join('|')")
            .expect("stale media callback trace should evaluate"),
        ""
    );
    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("replacement media {phase} turn"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("__restartedNetworkMediaEvents.join('|')")
            .expect("replacement media trace should evaluate"),
        "loadstart|loadedmetadata|loadeddata|canplay"
    );
    release_tx.send(()).expect("release cancelled media server");
    server.await.expect("cancelled media server should finish");
}

#[tokio::test]
async fn main_media_source_children_reselect_through_parent_sequence() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-media-source-child.test/",
        concat!(
            "<!doctype html><html><head></head><body>",
            "<video id='clip'><source id='source' src='data:video/webm;base64,AA=='></video>",
            "</body></html>",
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__sourceChildMediaEvents = [];
        const clip = document.getElementById("clip");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => __sourceChildMediaEvents.push(type));
        }
        "#,
        None,
    )
    .expect("source child media listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive should select the source child");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("source child media handle");
    let first = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("source child should create a parent media sequence");

    vm.exec(
        "document.getElementById('source').src = 'data:video/webm;base64,AQ==';",
        None,
    )
    .expect("source child mutation should restart parent selection");
    let second = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("source child mutation should replace the parent sequence");
    assert_ne!(first.id(), second.id());
    run_next_page_media_element_event_for_test(&mut vm, &loader, "stale source child media task")
        .await;
    for phase in ["loadstart", "loadedmetadata", "loadeddata", "canplay"] {
        run_next_page_media_element_event_for_test(
            &mut vm,
            &loader,
            &format!("source child media {phase} turn"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("__sourceChildMediaEvents.join('|')")
            .expect("source child media trace should evaluate"),
        "loadstart|loadedmetadata|loadeddata|canplay"
    );

    vm.exec(
        r#"
        const source = document.getElementById("source");
        source.remove();
        const replacement = document.createElement("source");
        replacement.src = "data:video/webm;base64,Ag==";
        document.getElementById("clip").appendChild(replacement);
        "#,
        None,
    )
    .expect("source child removal and insertion should reselect");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some(),
        "inserting a replacement source child must create a new parent sequence"
    );
}

#[tokio::test]
async fn main_static_text_track_starts_at_interactive_before_window_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-track-lifecycle.test/",
        concat!(
            "<!doctype html><html><head></head><body><video>",
            "<track id='captions' default ",
            "src='data:text/vtt,WEBVTT%0A%0A00%3A00%3A00.000%20--%3E%2000%3A00%3A01.000%0Ahello'>",
            "</video></body></html>",
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__mainTrackLifecycleEvents = [];
        const captions = document.getElementById("captions");
        captions.addEventListener("load", () => {
          __mainTrackLifecycleEvents.push(`track:${document.readyState}`);
        });
        void captions.track;
        "#,
        None,
    )
    .expect("main text-track lifecycle listener should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should seed static text tracks");

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("default mode DOM-manipulation turn"),
        "static default track should queue one typed mode-selection task"
    );
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("duplicate default-mode probe"),
        "parser insertion and interactive discovery must coalesce the same exact track"
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("track load-start networking turn")
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("track terminal networking turn")
    );
    assert_eq!(
        vm.eval("__mainTrackLifecycleEvents.join('|')")
            .expect("main track event trace"),
        "track:interactive",
        "static tracks must start at interactive rather than inside Window load"
    );
}

#[tokio::test]
async fn default_text_track_policy_loads_an_empty_track_without_a_network_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let markup = concat!(
        "<!doctype html><html><body>",
        "<video id='clip' src='data:video/webm;base64,AA=='>",
        "<track id='captions' default src='https://track-policy.test/captions.vtt'>",
        "</video></body></html>"
    );
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://track-policy.test/page.html",
        markup,
        &loader,
    );
    vm.exec(
        r#"
globalThis.__defaultTrackPolicyEvents = [];
const captions = document.getElementById("captions");
captions.addEventListener("load", () => __defaultTrackPolicyEvents.push("load"));
captions.addEventListener("error", () => __defaultTrackPolicyEvents.push("error"));
void captions.track;
"#,
        None,
    )
    .expect("default-policy track listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let track = vm
        .document_runtime
        .get_element_by_id("captions")
        .expect("track handle");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive should discover the default track");
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("default track mode-selection task")
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("default-policy track start task")
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_some_and(|pending| pending.network_request_id().is_none()),
        "the default policy must reject text-track network ownership"
    );
    assert!(!vm.take_network_output().into_items().any(|item| {
        matches!(
            item,
            crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                if request.resource_type()
                    == crate::types::SubresourceResourceType::TextTrack
        )
    }));

    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("default-policy track terminal task")
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __defaultTrackPolicyEvents, ready: captions.readyState, cues: captions.track.cues.length})"
        )
        .expect("default-policy track terminal state"),
        r#"{"events":["load"],"ready":2,"cues":0}"#
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_none()
    );
}

#[tokio::test]
async fn main_text_track_network_terminal_gates_canplay_without_delaying_complete() {
    let (track_url, request_rx, release_tx, server) =
        spawn_gated_text_track_resource_server(200).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_enabled(
        crate::types::SubresourceResourceType::TextTrack,
        true,
    );
    let document_url = track_url.replace("/captions.vtt", "/page.html");
    let markup = format!(
        concat!(
            "<!doctype html><html><body>",
            "<video id='clip' src='data:video/webm;base64,AA=='>",
            "<track id='captions' default src='{track_url}'>",
            "</video></body></html>"
        ),
        track_url = track_url,
    );
    let mut vm = new_parsed_page_task_executor_test_vm(&document_url, &markup, &loader);
    vm.exec(
        r#"
        globalThis.__trackMediaEvents = [];
        const clip = document.getElementById("clip");
        const captions = document.getElementById("captions");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => __trackMediaEvents.push(type));
        }
        captions.addEventListener("load", () => {
          __trackMediaEvents.push(`track-load:${captions.readyState}:${captions.track.cues.length}`);
        });
        captions.addEventListener("error", () => __trackMediaEvents.push("track-error"));
        void captions.track;
        "#,
        None,
    )
    .expect("text-track/media listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("media handle");
    let track = vm
        .document_runtime
        .get_element_by_id("captions")
        .expect("track handle");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive should start media and text-track owner sequences");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    let media_sequence = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("media sequence")
        .id();
    let track_sequence = vm
        ._context_host
        .borrow()
        .pending_text_track_load_sequence(track)
        .expect("text-track sequence");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_text_track_count(media, media_sequence),
        Some(1),
        "resource selection must snapshot the enabled text track"
    );
    assert!(track_sequence.network_request_id().is_none());

    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("text-track stable-state networking turn")
    );
    let request = request_rx.await.expect("text-track request should arrive");
    let request = request.to_ascii_lowercase();
    assert!(request.starts_with("get /captions.vtt http/1.1"));
    assert!(request.contains("accept: text/vtt,*/*;q=0.1"));
    assert!(request.contains("sec-fetch-dest: track"));
    assert!(request.contains("sec-fetch-mode: same-origin"));
    let text_track_request_count = vm
        .take_network_output()
        .into_items()
        .filter(|item| {
            matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                    if request.resource_type() == crate::types::SubresourceResourceType::TextTrack
            )
        })
        .count();
    assert_eq!(
        text_track_request_count, 1,
        "the element sequence must be the sole text-track request producer"
    );

    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadstart owner turn")
        .await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("already-applied default track mode turn"),
        "interactive selection should leave one coalesced default-mode task"
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadedmetadata owner turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadeddata owner turn")
        .await;
    assert_eq!(
        vm.eval("__trackMediaEvents.join('|')")
            .expect("pre-track-terminal event trace"),
        "loadstart|loadedmetadata|loadeddata",
        "canplay must wait for the selection-time text track"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "loadeddata still releases the media load delay while the track remains pending"
    );

    release_tx.send(()).expect("release text-track response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "text-track network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__trackMediaEvents.join('|')")
            .expect("resource completion event trace"),
        "loadstart|loadedmetadata|loadeddata",
        "resource completion may only queue the later track event"
    );

    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("text-track load-event networking turn")
    );
    assert_eq!(
        vm.eval("__trackMediaEvents.join('|')")
            .expect("track event trace"),
        "loadstart|loadedmetadata|loadeddata|track-load:2:1"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_text_track_count(media, media_sequence),
        Some(0)
    );
    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "media canplay follow-up owner turn",
    )
    .await;
    assert_eq!(
        vm.eval("__trackMediaEvents.join('|')")
            .expect("canplay event trace"),
        "loadstart|loadedmetadata|loadeddata|track-load:2:1|canplay"
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none()
    );
    server.await.expect("text-track server should finish");
}

#[tokio::test]
async fn stale_text_track_start_releases_media_canplay_gate() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://text-track-stale-gate.test/page.html",
        concat!(
            "<!doctype html><html><body>",
            "<video id='clip' src='data:video/webm;base64,AA=='>",
            "<track id='captions' default src='data:text/vtt,WEBVTT'>",
            "</video></body></html>"
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__staleTrackGateEvents = [];
        const clip = document.getElementById("clip");
        const captions = document.getElementById("captions");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => __staleTrackGateEvents.push(type));
        }
        for (const type of ["load", "error"]) {
          captions.addEventListener(type, () => __staleTrackGateEvents.push(`track-${type}`));
        }
        void captions.track;
        "#,
        None,
    )
    .expect("stale-gate listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("media handle");
    let track = vm
        .document_runtime
        .get_element_by_id("captions")
        .expect("track handle");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive should start media and text-track sequences");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    let media_sequence = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("media sequence")
        .id();

    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadstart owner turn")
        .await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("default-mode DOM turn")
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadedmetadata owner turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadeddata owner turn")
        .await;
    assert_eq!(
        vm.eval("__staleTrackGateEvents.join('|')")
            .expect("pre-stale event trace"),
        "loadstart|loadedmetadata|loadeddata"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_text_track_count(media, media_sequence),
        Some(1),
        "loadeddata must remain gated by the selected text track"
    );

    vm.exec("document.getElementById('captions').remove();", None)
        .expect("track removal should make the queued start stale");
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("stale text-track networking task should retire")
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_none()
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_text_track_count(media, media_sequence),
        Some(0),
        "stale retirement must settle the media selection gate"
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "media canplay follow-up owner turn",
    )
    .await;
    assert_eq!(
        vm.eval("__staleTrackGateEvents.join('|')")
            .expect("post-stale event trace"),
        "loadstart|loadedmetadata|loadeddata|canplay",
        "settling a stale track must naturally publish the blocked media follow-up"
    );
}

#[tokio::test]
async fn main_document_replacement_retires_pending_media_and_text_track_sequences() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://main-media-replacement.test/",
        "<!doctype html><html><head></head><body></body></html>",
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__staleMainMediaEventCount = 0;
        globalThis.__staleMainTextTrackEventCount = 0;
        const media = document.createElement("video");
        media.id = "stale-media";
        media.addEventListener("loadstart", () => {
          globalThis.__staleMainMediaEventCount += 1;
        });
        media.src = "data:video/webm;base64,AA==";
        const track = document.createElement("track");
        track.id = "stale-track";
        track.src = "data:text/vtt,WEBVTT%0A%0A00%3A00%3A00.000%20--%3E%2000%3A00%3A01.000%0Astale";
        for (const type of ["load", "error"]) {
          track.addEventListener(type, () => {
            globalThis.__staleMainTextTrackEventCount += 1;
          });
        }
        media.appendChild(track);
        document.body.appendChild(media);
        track.track.mode = "hidden";
        "#,
        None,
    )
    .expect("stale main media should queue");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("retired main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("stale-media")
        .expect("stale media handle");
    let track = vm
        .document_runtime
        .get_element_by_id("stale-track")
        .expect("stale text-track handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some()
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_some()
    );

    vm.exec(
        "document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();",
        None,
    )
    .expect("main document replacement should complete");
    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main document owner");
    assert_ne!(retired_owner, current_owner);
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none()
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_none()
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "stale media event should be consumed without dispatch",
    )
    .await;
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("stale text-track networking task should settle")
    );
    assert_eq!(
        vm.eval("String(globalThis.__staleMainMediaEventCount)")
            .expect("stale media event count"),
        "0"
    );
    assert_eq!(
        vm.eval("String(globalThis.__staleMainTextTrackEventCount)")
            .expect("stale text-track event count"),
        "0"
    );
    assert_eq!(vm.current_main_document_task_owner(), Some(current_owner));
}

fn new_parsed_test_vm(url: &str, markup: &str) -> StandaloneScriptVmHarness {
    let _js_runtime = crate::JsRuntime::initialize();
    let document =
        HtmlParser::SCRIPTING_ENABLED.parse(Url::parse(url).expect("test url"), markup.to_owned());
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
    let page_runtime_task_source = page_task_queue.residence();
    ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        page_task_front_injection_tx,
    )
    .expect("script vm bootstrap should succeed")
    .finish()
    .map(|mut vm| {
        vm.install_page_task_residence_for_executor_test(page_runtime_task_source);
        install_test_trusted_key_dispatcher(&mut vm);
        vm
    })
    .expect("script vm finish should succeed")
}

fn new_parsed_test_vm_with_loader_and_resource_completion_queue(
    url: &str,
    markup: &str,
    loader: &ResourceRequestClient,
) -> (
    StandaloneScriptVmHarness,
    RendererResourceCompletionTestHarness,
) {
    let _js_runtime = crate::JsRuntime::initialize();
    let document =
        HtmlParser::SCRIPTING_ENABLED.parse(Url::parse(url).expect("test url"), markup.to_owned());
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
    let page_runtime_task_source = page_task_queue.residence();
    let resource_completion_queue = RendererResourceCompletionTestHarness::new();
    let vm =
        ScriptVmDefaultWorldBootstrap::standalone_networked_from_dom_host_with_resource_completion_sender_for_test(
            DomHost::from_dom(document),
            post_domcontentloaded_page_task_sender,
            page_task_front_injection_tx,
            resource_completion_queue.sender(),
            loader.clone(),
        )
        .expect("script vm bootstrap should succeed")
        .finish()
        .map(|mut vm| {
            vm.install_page_task_residence_for_executor_test(page_runtime_task_source);
            install_test_trusted_key_dispatcher(&mut vm);
            vm
        })
        .expect("script vm finish should succeed");
    (vm, resource_completion_queue)
}

#[test]
fn cached_connected_modulepreload_never_acquires_a_load_delay() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/page.html",
        concat!(
            "<!doctype html><html><head>",
            "<link rel='modulepreload' href='/slow-module.mjs'>",
            "</head><body></body></html>",
        ),
    );
    let owner = vm
        .current_main_document_task_owner()
        .expect("modulepreload fixture must retain a current Document owner");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.replace_document_resource_runtime(&loader);
    vm.document_runtime.insert_native_module_source(
        crate::module_runtime::ModuleMapKey::java_script(
            Url::parse("https://example.test/slow-module.mjs").expect("module URL"),
        ),
        crate::module_runtime::ModuleSource::text("export {};".to_owned()),
    );
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false)
    );

    vm.queue_initial_connected_style_loads_for_current_owner();

    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "modulepreload network admission must retain only owner identity, not a load-delay token"
    );

    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    let ready = vm
        .take_next_connected_style_event_body_for_test()
        .expect("cached modulepreload terminal should publish its link event")
        .into_ready();
    assert_eq!(
        ready.load_event_binding(),
        None,
        "a ready modulepreload terminal must not acquire a short-lived stylesheet lease"
    );
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "modulepreload terminal publication must leave the Document load gate untouched"
    );
}

// Mirrors WPT `preload/avoid-delaying-onload-link-modulepreload.html`: the
// response stays pending until after Window load has been observed.
#[tokio::test]
async fn in_flight_connected_modulepreload_does_not_delay_window_load() {
    let (module_url, request_rx, release_tx, server) = spawn_gated_module_resource_server().await;
    let document_url = module_url.replace("/slow-module.mjs", "/page.html");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let markup = format!(
        concat!(
            "<!doctype html><html><head>",
            "<link id='preload' rel='modulepreload' href='{module_url}'>",
            "</head><body></body></html>",
        ),
        module_url = module_url,
    );
    let mut vm = new_parsed_page_task_executor_test_vm(&document_url, &markup, &loader);
    vm.exec(
        r#"
        globalThis.__modulepreloadLifecycleEvents = [];
        document.getElementById("preload").addEventListener("load", () => {
          __modulepreloadLifecycleEvents.push(`link:${document.readyState}`);
        });
        window.addEventListener("load", () => {
          __modulepreloadLifecycleEvents.push(`window:${document.readyState}`);
        });
        "#,
        None,
    )
    .expect("modulepreload lifecycle listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("modulepreload fixture must retain a current Document owner");

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), request_rx)
        .await
        .expect("modulepreload request should reach the server")
        .expect("modulepreload request channel should remain open");
    assert!(request.starts_with("GET /slow-module.mjs HTTP/1.1"));
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "an in-flight modulepreload request must not hold the Document load gate"
    );

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "window load must become ready while modulepreload remains in flight"
    );
    assert!(
        vm.dispatch_main_document_window_load_lifecycle(owner)
            .expect("main Window load lifecycle should apply")
            .is_none()
    );
    assert_eq!(
        vm.eval("__modulepreloadLifecycleEvents.join('|')")
            .expect("pre-terminal lifecycle event trace"),
        "window:complete",
        "window load must run before the pending modulepreload terminal"
    );

    release_tx.send(()).expect("release modulepreload response");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "modulepreload network completion",
    )
    .await;
    assert_eq!(
        vm.eval("__modulepreloadLifecycleEvents.join('|')")
            .expect("network-terminal lifecycle event trace"),
        "window:complete",
        "the network terminal may only queue the later link event"
    );
    assert!(
        vm.has_ready_native_module_owner_actions(),
        "the module-map terminal must publish its joined link-client notification"
    );
    assert!(
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("modulepreload owner-notification turn"),
        "the joined link client must be notified in a later selected task"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ConnectedStyleEvent,
            &loader,
        )
        .await
        .expect("modulepreload link-event turn")
    );
    assert_eq!(
        vm.eval("__modulepreloadLifecycleEvents.join('|')")
            .expect("complete modulepreload lifecycle event trace"),
        "window:complete|link:complete"
    );
    server.await.expect("modulepreload server should finish");
}

fn new_streamed_parser_test_vm(url: &str, markup: &str) -> StandaloneScriptVmHarness {
    let _js_runtime = crate::JsRuntime::initialize();
    let mut stream =
        HtmlParser::SCRIPTING_ENABLED.start_document(Url::parse(url).expect("test url"));
    stream.feed(markup);
    let dom_host = stream.take_parser_stream_dom_host();
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
    let page_runtime_task_source = page_task_queue.residence();
    ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        dom_host,
        post_domcontentloaded_page_task_sender,
        page_task_front_injection_tx,
    )
    .expect("script vm bootstrap should succeed")
    .finish()
    .map(|mut vm| {
        vm.install_page_task_residence_for_executor_test(page_runtime_task_source);
        install_test_trusted_key_dispatcher(&mut vm);
        vm
    })
    .expect("script vm finish should succeed")
}

fn install_test_trusted_key_dispatcher(vm: &mut ScriptVm) {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime_ptr| {
        let global = scope.get_current_context().global(scope);
        let data = v8::External::new(scope, runtime_ptr as *mut c_void);
        let function = v8::Function::builder(test_trusted_key_dispatch_callback)
            .data(data.into())
            .build(scope)
            .ok_or_else(|| anyhow::anyhow!("failed to create trusted key test dispatcher"))?;
        let _ = global.define_own_property(
            scope,
            crate::util::v8str(scope, "__moliDispatchTrustedKey").into(),
            function.into(),
            v8::PropertyAttribute::DONT_ENUM,
        );
        Ok(())
    })
    .expect("trusted key test dispatcher should install");
}

#[test]
fn resource_owner_id_is_available_from_current_context_slot() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><p>owner</p>");
    let expected = vm.resource_owner_id;

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        assert_eq!(
            scope
                .get_current_context()
                .get_slot::<crate::resource_owner::ResourceOwnerId>()
                .as_deref()
                .copied(),
            Some(expected)
        );
        assert_eq!(
            crate::resource_owner::current_resource_owner_id(scope),
            Some(expected)
        );
        assert!(
            scope
                .get_slot::<crate::resource_owner::ResourceOwnerId>()
                .is_none()
        );
        Ok(())
    })
    .expect("resource owner id should be visible from current context");
}

#[test]
fn runtime_observable_context_token_is_available_from_current_context_slot() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><p>runtime</p>");
    let expected = vm.page_default_runtime_observable_context_token;

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        assert_eq!(
            scope
                .get_current_context()
                .get_slot::<crate::native_bridge::RuntimeObservableContextToken>()
                .as_deref()
                .copied(),
            Some(expected)
        );
        assert_eq!(
            crate::native_bridge::current_runtime_observable_context_token(scope),
            Some(expected)
        );
        assert!(
            scope
                .get_slot::<crate::native_bridge::RuntimeObservableContextToken>()
                .is_none()
        );
        Ok(())
    })
    .expect("runtime observable context token should be visible from current context");
}

#[test]
fn promise_reject_dispatch_is_available_from_current_context_slot() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><p>promise</p>");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        assert!(
            scope
                .get_current_context()
                .get_slot::<super::runtime_bindings::PromiseRejectDispatchSlot>()
                .is_some()
        );
        assert!(super::runtime_bindings::promise_reject_dispatch_is_available_for_test(scope));
        assert!(
            scope
                .get_slot::<super::runtime_bindings::PromiseRejectDispatchSlot>()
                .is_none()
        );
        Ok(())
    })
    .expect("promise reject dispatch should be visible from current context");
}

#[test]
fn parser_discovered_modulepreload_invalid_as_warns_each_link_once() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/page.html",
        concat!(
            "<!doctype html><html><head>",
            "<link id='first' rel='modulepreload' href='/bad.bin' as='image'>",
            "<link id='second' rel='modulepreload' href='/bad.bin' as='IMAGE'>",
            "</head><body></body></html>",
        ),
    );
    let link_handles = ["first", "second"].map(|id| {
        vm.document_runtime
            .get_element_by_id(id)
            .expect("parser modulepreload link should exist")
    });

    assert!(
        vm.accept_parser_discovered_native_modulepreloads(link_handles),
        "parser-discovered invalid modulepreload should produce observable owner progress"
    );
    assert!(
        !vm.accept_parser_discovered_native_modulepreloads(link_handles),
        "replaying the same exact candidates should not repeat the warning"
    );

    assert_eq!(
        vm.runtime_observable_lifecycle_errors_for_testing(),
        vec![
            "<link rel=modulepreload> has an invalid `as` value image".to_owned(),
            "<link rel=modulepreload> has an invalid `as` value image".to_owned()
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn parser_discovered_modulepreload_invalid_as_dispatches_link_error_events() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/page.html",
        concat!(
            "<!doctype html><html><head>",
            "<link id='first' rel='modulepreload' href='/bad.bin' as='image'>",
            "<link id='second' rel='modulepreload' href='/bad.bin' as='IMAGE'>",
            "</head><body></body></html>",
        ),
    );
    vm.exec(
        r#"
        globalThis.__modulepreloadInvalidAsEvents = [];
        for (const link of document.querySelectorAll("link")) {
          link.addEventListener("load", () => {
            globalThis.__modulepreloadInvalidAsEvents.push(`${link.id}:load`);
          });
          link.addEventListener("error", () => {
            globalThis.__modulepreloadInvalidAsEvents.push(`${link.id}:error`);
          });
        }
        "#,
        None,
    )
    .expect("event listeners should install");
    let link_handles = ["first", "second"].map(|id| {
        vm.document_runtime
            .get_element_by_id(id)
            .expect("parser modulepreload link should exist")
    });

    assert!(
        vm.accept_parser_discovered_native_modulepreloads(link_handles),
        "parser-discovered invalid modulepreloads should queue link error tasks"
    );
    while vm.has_ready_native_module_owner_actions() {
        vm.drain_ready_native_module_owner_actions()
            .expect("parser-discovered invalid modulepreload owner event should dispatch");
    }
    vm.queue_initial_connected_style_loads_for_current_owner();
    assert!(
        !vm.apply_connected_style_lifecycle_bodies_for_test(),
        "initial connected scan must not dispatch duplicate parser invalid-as errors"
    );

    assert_eq!(
        vm.eval("globalThis.__modulepreloadInvalidAsEvents.join(',')")
            .expect("events should be readable"),
        "first:error,second:error"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn connected_modulepreload_invalid_as_dispatches_link_error_event() {
    let mut vm = new_parsed_test_vm(
        "https://example.test/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );
    vm.exec(
        r#"
        globalThis.__connectedModulepreloadInvalidAsEvents = [];
        const link = document.createElement("link");
        link.rel = "modulepreload";
        link.href = "/bad.bin";
        link.setAttribute("as", "image");
        link.addEventListener("load", () => {
          globalThis.__connectedModulepreloadInvalidAsEvents.push("load");
        });
        link.addEventListener("error", () => {
          globalThis.__connectedModulepreloadInvalidAsEvents.push("error");
        });
        document.head.appendChild(link);
        "#,
        None,
    )
    .expect("runtime-inserted invalid modulepreload should append");
    let owner = vm
        .current_main_document_task_owner()
        .expect("invalid modulepreload fixture must retain a current Document owner");

    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "invalid modulepreload admission must retain only event identity before its error task"
    );

    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "connected invalid modulepreload should dispatch through its document-owned link lane"
    );
    assert_eq!(
        vm.runtime_observable_lifecycle_errors_for_testing(),
        vec!["<link rel=modulepreload> has an invalid `as` value image".to_owned()]
    );
    assert!(
        !vm.has_ready_native_module_owner_actions(),
        "main connected link errors must not be duplicated through the child/module owner lane"
    );

    assert_eq!(
        vm.eval("globalThis.__connectedModulepreloadInvalidAsEvents.join(',')")
            .expect("events should be readable"),
        "error"
    );
}

#[test]
fn dynamic_import_invalid_attribute_key_rejects_with_type_error() {
    let mut vm = new_storage_test_vm("https://dynamic-import-attributes.test/page.html");

    vm.exec(
        r#"
        globalThis.__invalidDynamicImportAttribute = "pending";
        import("data:text/javascript,export%20default%201", { with: { foo: "bar" } })
          .then(() => {
            globalThis.__invalidDynamicImportAttribute = "unexpected";
          }, (error) => {
            globalThis.__invalidDynamicImportAttribute = JSON.stringify({
              name: error && error.name,
              type: error instanceof TypeError,
              message: String(error && error.message),
            });
          });
        "#,
        None,
    )
    .expect("dynamic import rejection setup should run");

    assert_eq!(
        vm.eval("globalThis.__invalidDynamicImportAttribute")
            .expect("dynamic import rejection should be observable"),
        r#"{"name":"TypeError","type":true,"message":"Invalid attribute key \"foo\"."}"#
    );
}

#[test]
fn string_timer_dynamic_import_keeps_captured_incumbent_script_base_url() {
    let mut vm = new_storage_test_vm("https://dynamic-import-timer.test/page/index.html");
    let script_url =
        Url::parse("https://dynamic-import-timer.test/scripts/nested/entry.js").unwrap();

    vm.exec(
        r#"setTimeout("import('../dependency.js')", 0);"#,
        Some(&script_url),
    )
    .expect("external script should queue a string timer");
    vm.exec(
        r#"
        const html = document.createElement('html');
        const head = document.createElement('head');
        html.append(head);
        document.append(html);
        const base = document.createElement('base');
        base.href = 'https://changed.example.test/assets/';
        head.append(base);
        "#,
        None,
    )
    .expect("document base mutation should run before the timer");
    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("string timer should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));

    let request = vm
        .document_runtime
        .take_next_native_dynamic_module_import()
        .expect("timer source should queue a dynamic import")
        .into_dynamic_import_request();
    assert_eq!(request.specifier(), "../dependency.js");
    assert_eq!(request.base_url(), &script_url);
}

#[test]
fn reflected_event_handler_dynamic_import_uses_document_base_url() {
    let mut vm = new_storage_test_vm("https://dynamic-import-handler.test/page/index.html");
    let script_url =
        Url::parse("https://dynamic-import-handler.test/scripts/nested/entry.js").unwrap();

    vm.exec(
        r#"
const html = document.createElement('html');
const head = document.createElement('head');
const body = document.createElement('body');
html.append(head, body);
document.appendChild(html);
const base = document.createElement('base');
base.href = '../assets/';
head.appendChild(base);
const target = document.createElement('div');
target.setAttribute('onclick', "import('./dependency.js')");
body.appendChild(target);
target.onclick();
"#,
        Some(&script_url),
    )
    .expect("external script should invoke the reflected event handler");

    let request = vm
        .document_runtime
        .take_next_native_dynamic_module_import()
        .expect("event handler source should queue a dynamic import")
        .into_dynamic_import_request();
    assert_eq!(request.specifier(), "./dependency.js");
    assert_eq!(
        request.base_url().as_str(),
        "https://dynamic-import-handler.test/assets/"
    );
}

#[test]
fn indexed_db_manager_is_available_from_context_slots() {
    let mut vm = new_storage_test_vm("https://indexeddb-context-slot.test/");

    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _runtime_ptr| {
        assert!(crate::context_bootstrap::indexed_db_manager_context_slot_present_for_test(scope));
        assert!(!crate::context_bootstrap::indexed_db_manager_isolate_slot_present_for_test(scope));
        Ok(())
    })
    .expect("indexedDB manager should be visible from default context");

    let isolated_context_id = vm
        .create_isolated_world("indexeddb-context-slot", false)
        .expect("isolated world should be created");
    let isolated_context_ptr = {
        let world = vm
            .page_isolated_world_contexts
            .context(isolated_context_id)
            .expect("isolated world context should be tracked");
        &world.context as *const _
    };
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(
        isolated_context_ptr,
        |scope, _runtime_ptr| {
            assert!(
                crate::context_bootstrap::indexed_db_manager_context_slot_present_for_test(scope)
            );
            assert!(
                !crate::context_bootstrap::indexed_db_manager_isolate_slot_present_for_test(scope)
            );
            Ok(())
        },
    )
    .expect("indexedDB manager should be visible from isolated context");
}

#[test]
fn inspector_context_created_matches_same_name_child_isolated_world_by_frame_id() {
    let mut vm = new_storage_test_vm("https://isolated-world-frame-match.test/");
    vm.root_frame_id = Some("root-frame".to_owned());

    let root_context_id = vm
        .create_isolated_world("shared-utility", false)
        .expect("root isolated world should be created");
    let child_context_id = vm
        .create_new_isolated_world(
            None,
            "shared-utility",
            false,
            Some("child-frame".to_owned()),
            None,
        )
        .expect("child-frame isolated world should be created");
    assert_ne!(root_context_id, child_context_id);
    assert_eq!(vm.page_isolated_world_contexts.len(), 2);

    let root_frame_id = vm.root_frame_id.clone();
    vm.page_isolated_world_contexts
        .record_inspector_context_state(
            &[serde_json::json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": child_context_id,
                        "uniqueId": "child-frame-replayed-realm",
                        "name": "shared-utility",
                        "auxData": {
                            "type": "isolated",
                            "frameId": "child-frame"
                        }
                    }
                }
            })],
            root_frame_id.as_deref(),
        );

    assert!(
        vm.page_isolated_world_contexts
            .has_execution_context_id(root_context_id),
        "child-frame inspector event must not re-key the root isolated world"
    );
    let child_world = vm
        .page_isolated_world_contexts
        .context(child_context_id)
        .expect("child isolated world should remain keyed by its execution context id");
    assert_eq!(child_world.frame_id.as_deref(), Some("child-frame"));
    assert_eq!(
        child_world.inspector_execution_context_realm_id.as_deref(),
        Some("child-frame-replayed-realm")
    );
    assert_eq!(vm.page_isolated_world_contexts.len(), 2);
}

#[test]
fn same_name_isolated_worlds_are_scoped_to_devtools_session_and_detach() {
    let mut vm = new_storage_test_vm("https://isolated-world-session.test/");
    let session_a = moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("session-a"));
    let session_b = moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("session-b"));

    let context_a = vm
        .ensure_isolated_world_for_owner(Some(&session_a), "utility", false)
        .expect("session A isolated world should be created");
    let context_b = vm
        .ensure_isolated_world_for_owner(Some(&session_b), "utility", false)
        .expect("session B same-name isolated world should be distinct");
    assert_ne!(context_a, context_b);
    vm.eval_in_isolated_context(context_a, "globalThis.owner = 'session-a'")
        .expect("session A isolated world should evaluate");
    vm.eval_in_isolated_context(context_b, "globalThis.owner = 'session-b'")
        .expect("session B isolated world should evaluate");
    assert_eq!(
        vm.eval_in_isolated_context(context_a, "owner")
            .expect("session A isolated world should retain its state"),
        "session-a"
    );
    assert_eq!(
        vm.eval_in_isolated_context(context_b, "owner")
            .expect("session B isolated world should retain its state"),
        "session-b"
    );

    assert!(vm.detach_runtime_inspector_session(Some("session-a")));
    assert!(
        vm.page_isolated_world_contexts.context(context_a).is_none(),
        "detaching session A must retire only its isolated world"
    );
    assert_eq!(
        vm.eval_in_isolated_context(context_b, "owner")
            .expect("session B isolated world should survive peer detach"),
        "session-b"
    );

    let replacement_session =
        moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("session-c"));
    let replacement_context = vm
        .ensure_isolated_world_for_owner(Some(&replacement_session), "utility", false)
        .expect("replacement session isolated world should be created");
    assert_eq!(
        vm.eval_in_isolated_context(replacement_context, "typeof owner")
            .expect("replacement session isolated world should evaluate"),
        "undefined"
    );
}

fn test_trusted_key_dispatch_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(external) = v8::Local::<v8::External>::try_from(args.data()) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let runtime_ptr = external.value() as *mut crate::native_bridge::JsContextHost;
    if runtime_ptr.is_null() {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    }

    let event_type = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "keydown".to_owned());
    let key = args
        .get(1)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let code = args
        .get(2)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let alt = args.get(3).boolean_value(scope);
    let ctrl = args.get(4).boolean_value(scope);
    let meta = args.get(5).boolean_value(scope);
    let shift = args.get(6).boolean_value(scope);

    let runtime = unsafe { &*runtime_ptr };
    let Some(target) = runtime
        .active_element_handle()
        .or_else(|| runtime.document_focus_fallback_handle())
    else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let Some(event) = crate::native_bridge::element::construct_keyboard_event(
        scope,
        &event_type,
        &key,
        &code,
        alt,
        ctrl,
        meta,
        shift,
        false,
    ) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let outcome =
        crate::native_bridge::element::dispatch_public_event(scope, runtime_ptr, target, event);
    rv.set(v8::Boolean::new(scope, outcome.allows_default()).into());
}

fn extract_first_inline_script(markup: &str) -> &str {
    let start = markup
        .find("<script>")
        .map(|idx| idx + "<script>".len())
        .expect("fixture should contain an inline <script> block");
    let end = markup[start..]
        .find("</script>")
        .map(|idx| start + idx)
        .expect("fixture should contain a closing </script> tag");
    &markup[start..end]
}

fn eval_probe_fixture_output(url: &str, markup: &str) -> serde_json::Value {
    let mut vm = new_parsed_test_vm(url, markup);
    let script = extract_first_inline_script(markup);
    vm.exec(script, None)
        .expect("fixture inline script should execute");
    let output = vm
        .eval(r#"document.getElementById("out").textContent"#)
        .expect("fixture output should be readable");
    serde_json::from_str(&output).expect("fixture output should be valid json")
}

#[test]
fn script_turn_watchdog_terminates_runaway_script_and_recovers_isolate() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><body></body>");
    let started = Instant::now();
    let error = vm
        .exec("for (;;) {}", None)
        .expect_err("runaway script should be terminated");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "script watchdog should terminate the turn promptly"
    );
    assert!(
        error.to_string().contains("script execution exceeded"),
        "unexpected watchdog error: {error}"
    );
    assert_eq!(
        vm.eval("String(1 + 1)")
            .expect("isolate should remain usable after termination"),
        "2"
    );
}

#[test]
fn microtask_checkpoint_watchdog_terminates_runaway_queue_and_recovers_isolate() {
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><body></body>");
    let started = Instant::now();
    let error = vm
        .exec(
            "Promise.resolve().then(function loop() { queueMicrotask(loop); });",
            None,
        )
        .expect_err("runaway microtask checkpoint should be terminated");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "microtask watchdog should terminate the checkpoint promptly"
    );
    assert!(
        error.to_string().contains("microtask checkpoint exceeded"),
        "unexpected watchdog error: {error}"
    );
    assert_eq!(
        vm.eval("String(2 + 2)")
            .expect("isolate should remain usable after microtask termination"),
        "4"
    );
}

fn force_gc_and_report_inspector_policy_for_test(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let inspector_policy_is_scoped = scope.get_microtasks_policy() == v8::MicrotasksPolicy::Scoped;
    scope.memory_pressure_notification(v8::MemoryPressureLevel::Critical);
    scope.low_memory_notification();
    rv.set(v8::Boolean::new(scope, inspector_policy_is_scoped).into());
}

#[test]
fn runtime_await_promise_sync_result_survives_queued_allocation_gc() {
    let mut vm = new_parsed_test_vm(
        "https://runtime-await-promise-queued-gc.test/",
        "<!doctype html><body></body>",
    );

    // This callback is installed only in this test realm. It makes the race
    // deterministic: the page microtask runs after Inspector has converted the
    // synchronous string to an awaitable promise, but before Inspector's
    // reaction publishes the Runtime.evaluate response.
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context;
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let callback = v8::Function::new(scope, force_gc_and_report_inspector_policy_for_test)
                .expect("test GC callback");
            let key = v8::String::new(scope, "__moliForceGcAndReportInspectorPolicyForTest")
                .expect("test GC callback key");
            assert!(
                context
                    .global(scope)
                    .set(scope, key.into(), callback.into())
                    == Some(true),
                "test GC callback should install"
            );
            Ok(())
        })
        .expect("test GC callback should install in the page realm");

    let messages = vm
        .dispatch_inspector_protocol_message(
            &serde_json::json!({
                "id": 91,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"(() => {
                        queueMicrotask(() => {
                            const values = [];
                            for (let index = 0; index < 5000; index += 1) {
                                values.push({ index, text: "x".repeat(128) });
                            }
                            globalThis.__queuedAllocationCount = values.length;
                            globalThis.__inspectorPolicyWasScoped =
                                __moliForceGcAndReportInspectorPolicyForTest();
                        });
                        return "sync-result";
                    })()"#,
                    "awaitPromise": true,
                    "returnByValue": true,
                }
            })
            .to_string(),
        )
        .expect("Runtime.evaluate dispatch");
    let response = messages
        .iter()
        .find(|message| message["id"] == serde_json::json!(91))
        .unwrap_or_else(|| panic!("Runtime.evaluate response missing: {messages:#?}"));
    assert!(
        response.get("error").is_none(),
        "a synchronous awaitPromise result must remain reachable through the Inspector checkpoint: {response:#?}"
    );
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!("sync-result")
    );
    assert_eq!(
        vm.eval("String(globalThis.__queuedAllocationCount)")
            .expect("queued allocation marker"),
        "5000",
        "the page microtask must run in the Runtime command checkpoint"
    );
    assert_eq!(
        vm.eval("String(globalThis.__inspectorPolicyWasScoped)")
            .expect("Inspector policy marker"),
        "true",
        "the queued page microtask must run inside Inspector's scoped checkpoint"
    );
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            assert_eq!(
                isolate.get_microtasks_policy(),
                v8::MicrotasksPolicy::Explicit,
                "the document isolate must leave the Runtime dispatch with its owner policy restored"
            );
            Ok(())
        })
        .expect("inspect restored microtasks policy");

    let messages = vm
        .dispatch_inspector_protocol_message(
            &serde_json::json!({
                "id": 92,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "__moliForceGcAndReportInspectorPolicyForTest()",
                    "returnByValue": true,
                }
            })
            .to_string(),
        )
        .expect("non-await Runtime.evaluate dispatch");
    let response = messages
        .iter()
        .find(|message| message["id"] == serde_json::json!(92))
        .unwrap_or_else(|| panic!("non-await Runtime.evaluate response missing: {messages:#?}"));
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!(true),
        "non-await Inspector commands must use the same scoped dispatch boundary"
    );
    vm.renderer_document_isolate
        .clone()
        .with_entered_renderer_document_isolate(|isolate| {
            assert_eq!(
                isolate.get_microtasks_policy(),
                v8::MicrotasksPolicy::Explicit,
                "a non-await Inspector command must restore the document owner policy"
            );
            Ok(())
        })
        .expect("inspect policy after non-await command");
}

#[tokio::test]
async fn timer_callback_watchdog_terminates_runaway_timer_and_recovers_isolate() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm("https://example.test/", "<!doctype html><body></body>");
    vm.exec(
        "setTimeout(() => { for (;;) {} }, 0); window.__afterTimer = 1;",
        None,
    )
    .expect("timer setup should run");

    let started = Instant::now();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("runaway timer should be reported without poisoning the isolate");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "timer watchdog should terminate the callback promptly"
    );
    assert_eq!(
        vm.eval("String(window.__afterTimer + 1)")
            .expect("isolate should remain usable after timer termination"),
        "2"
    );
}

fn collect_probe_error_paths(value: &serde_json::Value, path: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.len() == 1
                && let Some(serde_json::Value::String(message)) = map.get("error")
            {
                out.push(format!("{path}: {message}"));
            }
            for (key, nested) in map {
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                collect_probe_error_paths(nested, &next, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, nested) in items.iter().enumerate() {
                collect_probe_error_paths(nested, &format!("{path}[{index}]"), out);
            }
        }
        _ => {}
    }
}

fn decode_png_dimensions_from_data_url(data_url: &str) -> (u32, u32) {
    let encoded = data_url
        .strip_prefix("data:image/png;base64,")
        .expect("png data url prefix");
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .expect("valid base64 png");
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = decoder.read_info().expect("png header should decode");
    let info = reader.info();
    (info.width, info.height)
}

mod browser_api;
mod canvas_webgl;
mod dom_elements;
mod dom_xhr;
mod http_fixture;
mod indexed_db;
mod inspector_unwrap;
mod lazy_storage;
mod lazy_window_surfaces;
mod observer_callbacks;
mod post_parse;
mod queue_microtask;
mod rendering_update;
mod script_terminal_completion;
mod streams;
mod webidl_collections;
mod webidl_fetch;
mod webidl_trusted_types;
mod websocket;
mod window_execution_context;
