use super::post_parse::dynamic_script_execute_is_runnable_before_dom_content_loaded;
use super::{
    PostParseDriverStep, PostParseLifecycleAdvance, PostParseLifecycleCompletionAction,
    PostParseLifecycleDriver, PostParseLifecycleRound, PostParsePageOwnedTask,
    PostParseProcessingAction, PostParseRuntimeDriverStep, PostParseStageBoundary,
    PostParseTaskCompletion, PostParseTaskExecutionToken, PostParseTaskInvalidationPolicy,
    ReadyPostParseAction, ScriptVm, ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness,
    select_post_parse_driver_step,
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
use crate::frame_owner_model::ChildFrameSemanticTurnKind;
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
            request_headers: Vec::new().into(),
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
        (&url).into(),
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
            request_headers: Vec::new().into(),
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
                    request_headers: Vec::new().into(),
                    request_body: None,
                    intercept_response: false,
                    handle_auth_requests: false,
                    initial_auth_network_request_headers: None,
                });
            }
            PendingWindowFetchTestStage::Streaming => {
                host.record_streaming_subresource_fetch(
                    crate::types::StreamingSubresourceFetchState {
                        response_filter: None,
                        pending,
                        request_url: url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new().into(),
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
                    request_headers: Vec::new().into(),
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
                        request_headers: Vec::new().into(),
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
                        request_headers: Vec::new().into(),
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
        (&document_url).into(),
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
            request_headers: Vec::new().into(),
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
        vec![("content-type".to_owned(), b"text/plain".to_vec())],
        "redirected Fetch completed".to_owned(),
    );
    response.redirected = true;
    response.redirect_chain = vec![crate::types::NavigationRedirect {
        source: moli_fetch::RedirectSource::Network,
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

fn new_storage_html_test_vm(url: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm(url);
    vm.document_runtime
        .dom_host_mut()
        .reset_html_document_shell();
    vm
}

fn refresh_layout_for_test(vm: &mut StandaloneScriptVmHarness) {
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("test layout refresh should succeed"),
        "test layout refresh requires a connected document element"
    );
}

fn publish_layout_for_test(vm: &mut StandaloneScriptVmHarness) {
    vm.publish_layout_for_test()
        .expect("fixture screenshot should succeed");
}

// Fixture generators yield only at explicit visual publication boundaries.
// This lets multi-scene probes retain their JS bindings while all ordinary
// evaluate and input helpers remain read-only with respect to layout.
fn eval_with_layout_publications(
    vm: &mut StandaloneScriptVmHarness,
    fixture: &str,
) -> anyhow::Result<String> {
    vm.eval(&format!("globalThis.__layoutFixture = {fixture}"))?;
    while vm.eval("globalThis.__layoutStep = __layoutFixture.next(); __layoutStep.done")? == "false"
    {
        vm.publish_layout_for_test()?;
    }
    vm.eval("__layoutStep.value")
}

fn new_rendered_test_vm(url: &str, markup: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm(url, markup);
    publish_layout_for_test(&mut vm);
    vm
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
    let stream = HtmlParser::SCRIPTING_ENABLED.start_document(Url::parse(url).expect("test URL"));
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
    // A selected lifecycle task may perform the first real layout, including
    // CoreText font discovery on macOS. Give the complete behavior the same
    // budget as a production lifecycle task, including its follow-up events.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
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

// Ported from Chromium's WPT copy at
// html/webappapis/dynamic-markup-insertion/document-write/iframe_003.html.

// Ported from Chromium's WPT copy at
// html/webappapis/dynamic-markup-insertion/document-write/iframe_010.html.

// Ported from Chromium's WPT copy at
// html/webappapis/dynamic-markup-insertion/document-write/nested-document-write-2.html.

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
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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

// Mirrors WPT `preload/avoid-delaying-onload-link-modulepreload.html`: the
// response stays pending until after Window load has been observed.

fn new_streamed_parser_test_vm(url: &str, markup: &str) -> StandaloneScriptVmHarness {
    let _js_runtime = crate::JsRuntime::initialize();
    let stream = HtmlParser::SCRIPTING_ENABLED.start_document(Url::parse(url).expect("test url"));
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

mod audio_node_interfaces;
mod bitmap_rendering_context_interface_interfaces;
mod blob_response_headers;
mod browser_api;
mod cache_interfaces;
mod canvas_arguments;
mod canvas_fill_rect;
mod canvas_paths;
mod canvas_transform_snapshots;
mod canvas_webgl;
mod child_dynamic_inline_scripts;
mod credential_interfaces;
mod device_events;

mod gamepad_interfaces;

mod dom_elements;
mod dom_exception_proxy_identity;
mod dom_xhr;
mod element_click;
mod encoded_video_chunk_shell;
mod event_receivers;
mod geometry_point_conversion_order;
mod headers_list;
mod http_fixture;
mod media_owner_playback_interfaces;
mod media_recorder_shell;
mod performance_observer_contract;
mod svg_animation_interfaces;
mod svg_computed_path_precision;
mod svg_transform_sync_consolidation;

mod audio_event_interfaces;
mod hyperlink_null_url_protocol;
mod import_meta;
mod indexed_db;
mod inspector_unwrap;
mod lazy_storage;
mod lazy_window_surfaces;
mod midi_owner_interfaces;
mod mouse_snapshot;
mod native_bridge_identity;
mod native_exception_stack_capture;
mod no_cors_header_fill;
mod observer_callbacks;
mod observer_receivers;
mod offline_audio_float;
mod post_parse;
mod queue_microtask;
mod readable_algorithm_arrays;
mod remote_playback_interface;
mod rendering_update;
mod script_terminal_completion;
mod storage_dense_name_arrays;
mod streams;
mod svg_filter_interfaces;
mod svg_geometry_methods;
mod svg_length_validation;
mod svg_number_validation;
mod svg_root_factory_receivers;
mod svg_switch_mpath_interfaces;
mod text_encoder_utf16_progress;
mod time_ranges;
mod url_components;
mod video_codecs_shell;
mod webgl_interfaces;
mod webidl_collections;
mod webidl_fetch;
mod webidl_receivers;
mod webidl_trusted_types;
mod websocket;
mod webtransport_stream_interface_exposure;
mod window_execution_context;
mod window_getter_consolidation;
mod worklet_interfaces;

mod string_timers;

mod history_replace_forward;

mod extracted;
mod navigation_timing_inheritance;
mod payment_response_interfaces;

mod body_mime_consolidation;
mod response_blob_mime;

mod intersection_target_order;
mod observer_element_arguments;

mod media_device_interfaces;

mod dom_rect_factory_descriptors;
mod dom_rect_structured_clone;
mod domrect_receiver_consolidation;

mod resize_observer_entries;
