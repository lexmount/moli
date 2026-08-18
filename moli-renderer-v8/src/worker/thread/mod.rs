//! Worker thread — owns a V8 isolate and Tokio `LocalRuntime`, runs the
//! worker event loop, and handles message dispatch.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::pin::pin;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::broadcast_channel_runtime::new_broadcast_channel_registry;
use crate::broadcast_channel_runtime::{
    BroadcastChannelStorageKey, SharedBroadcastChannelRegistry,
};
use crate::content_security_policy::{
    ContentSecurityPolicyDisposition, ContentSecurityPolicyRedirectStatus,
    ContentSecurityPolicyReportingEndpoints, ContentSecurityPolicyUrlViolation,
    content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints,
};
use crate::context_bootstrap::flush_one_pending_file_reader;
use crate::exception_reporting::{V8ExceptionReport, build_event_handler_exception_report};
#[cfg(test)]
use crate::network::ResourceRequestClientOwner;
use crate::network::{
    ResourceRequestClient,
    context::{WorkerResourceLoader, WorkerResourceOwner},
    loads::{ResourceLoadDisposition, ResourceLoadKind},
};
use crate::runtime::RendererWorkerContextRuntime;
use crate::service_worker_runtime::{
    ServiceWorkerClientId, ServiceWorkerClientType, ServiceWorkerRuntimeService,
};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, trace};

mod dispatch;
mod isolate;
mod runtime_inspector;

pub(crate) use dispatch::dispatch_current_worker_callback_exception;

use dispatch::{
    abort_service_worker_fetch_request_signal, cancel_service_worker_fetch_stream,
    create_script_origin, dispatch_broadcast_channel_event, dispatch_message_event,
    dispatch_message_port_event, dispatch_queued_worker_promise_rejections,
    dispatch_service_worker_controller_change_event, dispatch_service_worker_fetch_event,
    dispatch_service_worker_lifecycle_event, dispatch_service_worker_message_event,
    dispatch_service_worker_notification_event, dispatch_service_worker_periodic_sync_event,
    dispatch_service_worker_push_event, dispatch_service_worker_sync_event,
    dispatch_shared_worker_connect_event, dispatch_worker_error_event,
    dispatch_worker_exception_with_phase, dispatch_worker_exception_with_phase_and_source,
    enqueue_service_worker_navigation_preload_stream_chunk,
    fail_service_worker_navigation_preload_response,
    finish_service_worker_navigation_preload_stream, fire_timer_callback,
    install_worker_promise_rejection_dispatch,
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections,
    report_exception_to_parent, start_service_worker_navigation_preload_response,
};
use isolate::WorkerIsolateState;
use moli_fetch::RequestCredentialsMode;
use moli_storage_key::MoliStorageKey;
use runtime_inspector::WorkerRuntimeInspector;

use super::global_scope::{
    WorkerFetchEvent, WorkerGlobalState, WorkerIsolateTimerQueues, WorkerOpfsCompletion,
    WorkerWebCryptoCompletion, WorkerXhrCompletion, close_worker_owned_broadcast_channels,
    close_worker_owned_message_ports, continue_pending_worker_csp_report,
    continue_pending_worker_fetch, continue_pending_worker_fetch_response,
    continue_pending_worker_xhr, continue_pending_worker_xhr_response,
    dispatch_nested_worker_event, dispatch_worker_csp_violation_event,
    dispatch_worker_websocket_event, drain_service_worker_client_focus_result,
    drain_service_worker_client_navigate_result, drain_service_worker_client_query_result,
    drain_service_worker_clients_open_window_result, drain_service_worker_get_notifications_result,
    drain_service_worker_periodic_sync_get_tags_result,
    drain_service_worker_periodic_sync_registration_result,
    drain_service_worker_periodic_sync_unregistration_result,
    drain_service_worker_push_get_subscription_result, drain_service_worker_push_subscribe_result,
    drain_service_worker_push_unsubscribe_result, drain_service_worker_show_notification_result,
    drain_service_worker_sync_get_tags_result, drain_service_worker_sync_registration_result,
    drain_worker_fetch_completion, drain_worker_opfs_completion, drain_worker_webcrypto_completion,
    drain_worker_xhr_completion, fail_pending_worker_csp_report, fail_pending_worker_fetch,
    fail_pending_worker_fetch_auth, fail_pending_worker_fetch_response, fail_pending_worker_xhr,
    fail_pending_worker_xhr_auth, fail_pending_worker_xhr_response,
    fulfill_pending_worker_csp_report, fulfill_pending_worker_fetch,
    fulfill_pending_worker_fetch_response, fulfill_pending_worker_xhr,
    fulfill_pending_worker_xhr_response, install_worker_global_scope,
    service_worker_fetch_handler_type,
};
use super::handle::{
    WorkerBootstrapCompletion, WorkerBootstrapFailure, WorkerBootstrapSuccess,
    WorkerDevToolsHandle, WorkerErrorPhase, WorkerErrorSource, WorkerFetchHandlerType,
    WorkerHandle, WorkerMessage, WorkerNetworkPolicy, WorkerParentErrorEventKind,
    WorkerRuntimeInspectorMessageBatch, WorkerScriptResource, WorkerScriptResourceKind,
    WorkerToParentMessage,
};
use super::inspector_task_runner::{
    WorkerInspectorTask, WorkerInspectorTaskMode, WorkerInspectorTaskRunner,
};
use super::module_runtime::{
    WorkerBootstrapError, WorkerDynamicModuleImportAdvance, WorkerModuleBootstrapResume,
    WorkerModuleBootstrapStart, WorkerModuleEvaluationCompletion, WorkerModuleFetchedSource,
    WorkerModuleGraphFetchBatch, WorkerModuleGraphFetchCompletion, WorkerModuleGraphFetchCspSource,
    WorkerModuleGraphFetchRequest, WorkerModuleKind, WorkerModulePendingBootstrap,
    WorkerModuleSource, evaluate_module_worker_bootstrap_source,
    install_classic_worker_dynamic_module_runtime, resume_worker_dynamic_module_evaluation,
    resume_worker_dynamic_module_fetch, run_next_worker_dynamic_module_import,
    worker_dynamic_module_import_waits_for_fetch, worker_has_pending_dynamic_module_imports,
    worker_has_runnable_dynamic_module_imports,
};

pub(super) type WorkerExceptionError = Box<(V8ExceptionReport, Option<v8::Global<v8::Value>>)>;

const WORKER_STACK_SIZE: usize = 4 * 1024 * 1024; // 4 MiB

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, crate::webidl::WebIdlEnum)]
#[webidl(name = "WorkerType", parse_with = Self::parse_webidl_token)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum WorkerScriptKind {
    Classic,
    Module,
}

impl WorkerScriptKind {
    fn parse_webidl_token(value: &str) -> Option<Self> {
        value.parse().ok()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum WorkerScriptSource {
    Text(String),
    Binary(Vec<u8>),
}

impl WorkerScriptSource {
    pub(crate) fn text(source: String) -> Self {
        Self::Text(source)
    }

    pub(crate) fn binary(bytes: Vec<u8>) -> Self {
        Self::Binary(bytes)
    }

    fn text_source(&self) -> Option<&str> {
        match self {
            Self::Text(source) => Some(source),
            Self::Binary(_) => None,
        }
    }

    fn module_source(&self) -> WorkerModuleSource {
        match self {
            Self::Text(source) => WorkerModuleSource::text(source.clone()),
            Self::Binary(bytes) => WorkerModuleSource::binary(bytes.clone()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorkerGlobalKind {
    Dedicated {
        name: String,
    },
    Shared {
        name: String,
        storage_key: MoliStorageKey,
    },
    Service {
        registration_id: crate::runtime::ServiceWorkerRegistrationId,
        version_id: crate::runtime::ServiceWorkerVersionId,
        scope_url: url::Url,
    },
}

pub(crate) struct WorkerSpawnOptions {
    pub(crate) script_source: WorkerScriptSource,
    pub(crate) script_url: String,
    pub(crate) request_client: ResourceRequestClient,
    pub(crate) script_kind: WorkerScriptKind,
    pub(crate) module_static_import_initiator_url: Option<url::Url>,
    pub(crate) module_credentials_mode: RequestCredentialsMode,
    pub(crate) referrer_policy: Option<String>,
    pub(crate) module_static_import_content_security_policies: Vec<String>,
    pub(crate) content_security_policies: Vec<String>,
    pub(crate) content_security_report_only_policies: Vec<String>,
    pub(crate) content_security_reporting_endpoints: ContentSecurityPolicyReportingEndpoints,
    pub(crate) network_policy: WorkerNetworkPolicy,
    pub(crate) policy_context: crate::types::SubresourcePolicyContext,
    pub(crate) worker_context_runtime: RendererWorkerContextRuntime,
    pub(crate) global_kind: WorkerGlobalKind,
    pub(crate) api_storage_key: Option<BroadcastChannelStorageKey>,
    /// Top-level site used only when the worker global storage key cannot be
    /// inherited from an explicit API, creator, or shared-worker instance key.
    pub(crate) broadcast_channel_top_level_site: Option<String>,
    pub(crate) creator_storage_key: Option<MoliStorageKey>,
    pub(crate) service_worker_runtime: Option<ServiceWorkerRuntimeService>,
    pub(crate) reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
    pub(crate) indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub(crate) storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub(crate) bootstrap_completion_tx: Option<mpsc::UnboundedSender<WorkerBootstrapCompletion>>,
    pub(crate) pause_evaluation_until_debugger: bool,
    #[cfg(test)]
    test_request_client_owner: Option<ResourceRequestClientOwner>,
}

#[cfg(test)]
pub(crate) enum WorkerTestRequestClient {
    Handle(ResourceRequestClient),
    Owned(ResourceRequestClientOwner),
}

#[cfg(test)]
pub(crate) struct WorkerTestHandle {
    handle: Option<WorkerHandle>,
    _request_client_owner: Option<ResourceRequestClientOwner>,
}

#[cfg(test)]
impl std::ops::Deref for WorkerTestHandle {
    type Target = WorkerHandle;

    fn deref(&self) -> &Self::Target {
        self.handle
            .as_ref()
            .expect("test worker handle was already joined")
    }
}

#[cfg(test)]
impl std::ops::DerefMut for WorkerTestHandle {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.handle
            .as_mut()
            .expect("test worker handle was already joined")
    }
}

#[cfg(test)]
impl WorkerTestHandle {
    pub(crate) fn terminate_and_join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.terminate_and_join();
        }
    }
}

#[cfg(test)]
impl Drop for WorkerTestHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.terminate_and_join();
        }
    }
}

#[cfg(test)]
impl From<ResourceRequestClient> for WorkerTestRequestClient {
    fn from(client: ResourceRequestClient) -> Self {
        Self::Handle(client)
    }
}

#[cfg(test)]
impl From<ResourceRequestClientOwner> for WorkerTestRequestClient {
    fn from(owner: ResourceRequestClientOwner) -> Self {
        Self::Owned(owner)
    }
}

impl WorkerSpawnOptions {
    #[cfg(test)]
    pub(crate) fn new(script_source: String, script_url: String) -> Self {
        Self::with_source(WorkerScriptSource::text(script_source), script_url)
    }

    #[cfg(test)]
    pub(crate) fn with_source(script_source: WorkerScriptSource, script_url: String) -> Self {
        let request_client_owner = worker_test_request_client();
        let mut options = Self::with_source_and_request_client(
            script_source,
            script_url,
            request_client_owner.handle(),
        );
        options.test_request_client_owner = Some(request_client_owner);
        options
    }

    pub(crate) fn new_with_request_client(
        script_source: String,
        script_url: String,
        request_client: ResourceRequestClient,
    ) -> Self {
        Self::with_source_and_request_client(
            WorkerScriptSource::text(script_source),
            script_url,
            request_client,
        )
    }

    pub(crate) fn with_source_and_request_client(
        script_source: WorkerScriptSource,
        script_url: String,
        request_client: ResourceRequestClient,
    ) -> Self {
        Self {
            script_source,
            script_url,
            request_client,
            script_kind: WorkerScriptKind::Classic,
            module_static_import_initiator_url: None,
            module_credentials_mode: RequestCredentialsMode::SameOrigin,
            referrer_policy: None,
            module_static_import_content_security_policies: Vec::new(),
            content_security_policies: Vec::new(),
            content_security_report_only_policies: Vec::new(),
            content_security_reporting_endpoints: ContentSecurityPolicyReportingEndpoints::default(
            ),
            network_policy: WorkerNetworkPolicy::default(),
            policy_context: Default::default(),
            worker_context_runtime: RendererWorkerContextRuntime::new(
                crate::message_port_runtime::new_message_port_registry(),
                crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
            ),
            global_kind: WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            api_storage_key: None,
            broadcast_channel_top_level_site: None,
            creator_storage_key: None,
            service_worker_runtime: None,
            reserved_service_worker_client_id: None,
            indexed_db_manager: None,
            storage_bucket_store: None,
            bootstrap_completion_tx: None,
            pause_evaluation_until_debugger: false,
            #[cfg(test)]
            test_request_client_owner: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_request_client(
        mut self,
        request_client: impl Into<WorkerTestRequestClient>,
    ) -> Self {
        match request_client.into() {
            WorkerTestRequestClient::Handle(request_client) => {
                self.request_client = request_client;
                self.test_request_client_owner = None;
            }
            WorkerTestRequestClient::Owned(owner) => {
                self.request_client = owner.handle();
                self.test_request_client_owner = Some(owner);
            }
        }
        self
    }

    pub(crate) fn with_script_kind(mut self, script_kind: WorkerScriptKind) -> Self {
        self.script_kind = script_kind;
        self
    }

    pub(crate) fn with_module_credentials_mode(
        mut self,
        credentials_mode: RequestCredentialsMode,
    ) -> Self {
        self.module_credentials_mode = credentials_mode;
        self
    }

    pub(crate) fn with_module_static_import_initiator_url(mut self, url: url::Url) -> Self {
        self.module_static_import_initiator_url = Some(url);
        self
    }

    pub(crate) fn with_referrer_policy(mut self, referrer_policy: Option<String>) -> Self {
        self.referrer_policy = referrer_policy;
        self
    }

    pub(crate) fn with_content_security_policies(mut self, policies: Vec<String>) -> Self {
        self.content_security_policies = policies;
        self
    }

    pub(crate) fn with_module_static_import_content_security_policies(
        mut self,
        policies: Vec<String>,
    ) -> Self {
        self.module_static_import_content_security_policies = policies;
        self
    }

    pub(crate) fn with_content_security_report_only_policies(
        mut self,
        policies: Vec<String>,
    ) -> Self {
        self.content_security_report_only_policies = policies;
        self
    }

    pub(crate) fn with_content_security_reporting_endpoints(
        mut self,
        endpoints: ContentSecurityPolicyReportingEndpoints,
    ) -> Self {
        self.content_security_reporting_endpoints = endpoints;
        self
    }

    pub(crate) fn with_network_policy(mut self, network_policy: WorkerNetworkPolicy) -> Self {
        self.network_policy = network_policy;
        self
    }

    pub(crate) fn with_policy_context(
        mut self,
        policy_context: crate::types::SubresourcePolicyContext,
    ) -> Self {
        self.policy_context = policy_context;
        self
    }

    pub(crate) fn with_worker_context_runtime(
        mut self,
        runtime: RendererWorkerContextRuntime,
    ) -> Self {
        self.worker_context_runtime = runtime;
        self
    }

    pub(crate) fn with_global_kind(mut self, global_kind: WorkerGlobalKind) -> Self {
        self.global_kind = global_kind;
        self
    }

    pub(crate) fn with_storage_key_top_level_site(self, top_level_site: Option<String>) -> Self {
        self.with_broadcast_channel_top_level_site(top_level_site)
    }

    pub(crate) fn with_api_storage_key(
        mut self,
        storage_key: Option<BroadcastChannelStorageKey>,
    ) -> Self {
        self.api_storage_key = storage_key;
        self
    }

    pub(crate) fn with_broadcast_channel_top_level_site(
        mut self,
        top_level_site: Option<String>,
    ) -> Self {
        self.broadcast_channel_top_level_site = top_level_site;
        self
    }

    pub(crate) fn with_creator_storage_key(mut self, storage_key: MoliStorageKey) -> Self {
        self.creator_storage_key = Some(storage_key);
        self
    }

    pub(crate) fn with_service_worker_runtime(
        mut self,
        runtime: ServiceWorkerRuntimeService,
    ) -> Self {
        self.service_worker_runtime = Some(runtime);
        self
    }

    pub(crate) fn with_reserved_service_worker_client_id(
        mut self,
        client_id: ServiceWorkerClientId,
    ) -> Self {
        self.reserved_service_worker_client_id = Some(client_id);
        self
    }

    pub(crate) fn with_indexed_db_manager(
        mut self,
        indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    ) -> Self {
        self.indexed_db_manager = indexed_db_manager;
        self
    }

    pub(crate) fn with_storage_bucket_store(
        mut self,
        storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    ) -> Self {
        self.storage_bucket_store = storage_bucket_store;
        self
    }

    pub(crate) fn with_bootstrap_completion_sender(
        mut self,
        sender: mpsc::UnboundedSender<WorkerBootstrapCompletion>,
    ) -> Self {
        self.bootstrap_completion_tx = Some(sender);
        self
    }

    pub(crate) fn with_pause_evaluation_until_debugger(mut self, pause: bool) -> Self {
        self.pause_evaluation_until_debugger = pause;
        self
    }
}

#[cfg(test)]
mod worker_script_kind_tests {
    use super::WorkerScriptKind;
    use std::str::FromStr;

    #[test]
    fn worker_script_kind_parses_worker_options_type_tokens() {
        assert_eq!(
            WorkerScriptKind::from_str("classic"),
            Ok(WorkerScriptKind::Classic)
        );
        assert_eq!(
            WorkerScriptKind::from_str("module"),
            Ok(WorkerScriptKind::Module)
        );
        assert!(WorkerScriptKind::from_str("Module").is_err());
        assert!(WorkerScriptKind::from_str("shared").is_err());
    }
}

pub(super) struct ActiveTimer {
    id: u32,
    callback: super::timer_callback::WorkerTimerCallback,
    delay: Duration,
    is_interval: bool,
    extra_args: Vec<v8::Global<v8::Value>>,
    next_fire: tokio::time::Instant,
}

fn worker_has_pending_async(state: &Rc<RefCell<WorkerGlobalState>>) -> bool {
    let state = state.borrow();
    !state.pending_fetches.is_empty()
        || !state.pending_xhrs.is_empty()
        || !state.websockets.is_empty()
        || !state.pending_webcrypto.is_empty()
        || state
            .opfs_owner_state
            .as_ref()
            .is_some_and(|opfs| opfs.has_pending_tasks())
        || !state.pending_service_worker_client_queries.is_empty()
        || !state.pending_service_worker_client_navigates.is_empty()
        || !state.pending_service_worker_client_focuses.is_empty()
        || !state.pending_service_worker_clients_open_windows.is_empty()
        || !state.pending_service_worker_show_notifications.is_empty()
        || !state.pending_service_worker_get_notifications.is_empty()
        || !state.pending_service_worker_sync_registrations.is_empty()
        || !state.pending_service_worker_sync_get_tags.is_empty()
        || !state
            .pending_service_worker_periodic_sync_registrations
            .is_empty()
        || !state
            .pending_service_worker_periodic_sync_get_tags
            .is_empty()
        || !state
            .pending_service_worker_periodic_sync_unregistrations
            .is_empty()
        || !state.pending_service_worker_push_subscriptions.is_empty()
        || !state
            .pending_service_worker_push_get_subscriptions
            .is_empty()
        || !state.pending_service_worker_push_unsubscriptions.is_empty()
        || !state.pending_service_worker_push_events.is_empty()
        || !state.pending_service_worker_sync_events.is_empty()
}

fn module_graph_csp_violation_message(violation: &ContentSecurityPolicyUrlViolation) -> String {
    format!(
        "Module worker dependency fetch blocked by Content Security Policy for `{}`.",
        violation.blocked_uri
    )
}

fn start_worker_module_graph_fetch(
    request: WorkerModuleGraphFetchRequest,
    loader: WorkerResourceLoader,
    network_partition_key: Option<String>,
    module_static_import_content_security_policies: Vec<String>,
    worker_global_content_security_policies: Vec<String>,
    worker_global_content_security_report_only_policies: Vec<String>,
    worker_global_content_security_reporting_endpoints: ContentSecurityPolicyReportingEndpoints,
    completion_tx: mpsc::UnboundedSender<WorkerModuleGraphFetchCompletion>,
) {
    let fetch_id = request.fetch_id();
    let (
        content_security_policies,
        content_security_report_only_policies,
        content_security_reporting_endpoints,
        resource_kind,
    ) =
        match request.csp_source() {
        WorkerModuleGraphFetchCspSource::StaticModuleGraph => (
            module_static_import_content_security_policies,
            Vec::new(),
            ContentSecurityPolicyReportingEndpoints::default(),
            crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerStaticModuleImport,
        ),
        WorkerModuleGraphFetchCspSource::DynamicImportGraph => (
            worker_global_content_security_policies,
            worker_global_content_security_report_only_policies,
            worker_global_content_security_reporting_endpoints,
            crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerScript,
        ),
    };
    let initial_csp_report_only_violation =
        content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
            &content_security_report_only_policies,
            request.initiator_url(),
            request.url(),
            resource_kind,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Report,
            &content_security_reporting_endpoints,
        );
    if let Some(violation) = content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
        &content_security_policies,
        request.initiator_url(),
        request.url(),
        resource_kind,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
        ContentSecurityPolicyDisposition::Enforce,
        &content_security_reporting_endpoints,
    ) {
        let message = module_graph_csp_violation_message(&violation);
        let mut completion = WorkerModuleGraphFetchCompletion::new(fetch_id, Err(message))
            .with_csp_violation(violation);
        if let Some(report_only_violation) = initial_csp_report_only_violation {
            completion = completion.with_csp_report_only_violation(report_only_violation);
        }
        let _ = completion_tx.send(completion);
        return;
    }
    let browser_request_metadata = request.browser_request_metadata();
    let mut fetch_request =
        match moli_fetch::Request::new("GET", request.url().as_str(), None, vec![]) {
            Ok(request) => request
                .with_page_network_policy()
                .with_network_partition_key(network_partition_key.clone())
                .with_browser_request_metadata(browser_request_metadata),
            Err(error) => {
                let _ = completion_tx.send(WorkerModuleGraphFetchCompletion::new(
                    fetch_id,
                    Err(error.to_string()),
                ));
                return;
            }
        };
    if let Some(referrer_policy) = request.referrer_policy() {
        fetch_request =
            fetch_request.with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
                document_referrer_policy: Some(referrer_policy.to_owned()),
                ..moli_fetch::ScriptFetchRequestMetadata::default()
            });
    }
    fetch_request = fetch_request
        .with_initiator_url(request.initiator_url())
        .with_credentials_mode(request.credentials_mode());
    let requested_url = request.url().clone();
    let request_initiator_url = request.initiator_url().clone();
    let requested_module_type = request.module_type().map(str::to_owned);
    let requested_kind = request.kind();
    let request_credentials_mode = request.credentials_mode();
    let response_content_security_policies = content_security_policies.clone();
    let response_content_security_report_only_policies =
        content_security_report_only_policies.clone();
    let response_content_security_reporting_endpoints =
        content_security_reporting_endpoints.clone();
    let response_resource_kind = resource_kind;
    let completion_tx_for_callback = completion_tx.clone();
    let response_started_at = Instant::now();
    let send_completion = move |result: Result<moli_fetch::Response, anyhow::Error>| {
        let mut csp_violation = None;
        let mut csp_report_only_violation = initial_csp_report_only_violation;
        let result = result
            .map_err(|error| {
                format!("failed to fetch module worker dependency `{requested_url}`: {error}")
            })
            .and_then(|response| {
                moli_fetch::ensure_http_status_success(
                    response.final_url.as_str(),
                    response.status,
                    false,
                )
                .map_err(|error| error.to_string())?;
                let redirect_status = if response.redirect_chain.is_empty() {
                    ContentSecurityPolicyRedirectStatus::NoRedirect
                } else {
                    ContentSecurityPolicyRedirectStatus::FollowedRedirect
                };
                if redirect_status == ContentSecurityPolicyRedirectStatus::FollowedRedirect
                    && csp_report_only_violation.is_none()
                {
                    csp_report_only_violation =
                        content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
                            &response_content_security_report_only_policies,
                            &request_initiator_url,
                            &response.final_url,
                            response_resource_kind,
                            redirect_status,
                            ContentSecurityPolicyDisposition::Report,
                            &response_content_security_reporting_endpoints,
                        );
                }
                if let Some(violation) = content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
                    &response_content_security_policies,
                    &request_initiator_url,
                    &response.final_url,
                    response_resource_kind,
                    redirect_status,
                    ContentSecurityPolicyDisposition::Enforce,
                    &response_content_security_reporting_endpoints,
                ) {
                    let message = module_graph_csp_violation_message(&violation);
                    csp_violation = Some(violation);
                    return Err(message);
                }
                crate::network_host::validate_fetch_response_security_policy(
                    &request_initiator_url,
                    &response.final_url,
                    &response.headers,
                    moli_fetch::RequestMode::Cors,
                    request_credentials_mode,
                    Default::default(),
                )?;
                if requested_kind == WorkerModuleKind::WebAssembly {
                    crate::worker::ensure_worker_wasm_module_mime(&response)?;
                } else {
                    match requested_module_type.as_deref() {
                        Some("json") => crate::worker::ensure_worker_json_module_mime(&response)?,
                        _ => crate::worker::ensure_worker_script_mime_acceptable(
                            &response.final_url,
                            &response.headers,
                            response.body_bytes(),
                        )
                        .map_err(|error| {
                            error.replace(
                                "unsupported script MIME type",
                                "unsupported module script MIME type",
                            )
                        })?,
                    }
                }
                let response_time_ms = response_started_at
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64;
                let (head, body, body_bytes) = response.into_parts();
                let response_referrer_policy =
                    crate::referrer_policy::response_referrer_policy_from_headers(&head.headers);
                let resource = WorkerScriptResource::from_response_parts(
                    requested_url.clone(),
                    &head,
                    &body_bytes,
                    response_time_ms,
                )
                .with_kind(worker_script_resource_kind_for_module(requested_kind));
                let final_url = head.final_url.clone();
                let source = if requested_kind == WorkerModuleKind::WebAssembly {
                    WorkerModuleSource::binary(body_bytes)
                } else {
                    WorkerModuleSource::text(body)
                };
                Ok(WorkerModuleFetchedSource::new(final_url, source)
                    .with_resource(resource)
                    .with_response_referrer_policy(response_referrer_policy))
            });
        let mut completion = WorkerModuleGraphFetchCompletion::new(fetch_id, result);
        if let Some(violation) = csp_report_only_violation {
            completion = completion.with_csp_report_only_violation(violation);
        }
        if let Some(violation) = csp_violation {
            completion = completion.with_csp_violation(violation);
        }
        let _ = completion_tx_for_callback.send(completion);
    };
    let Some(load) = loader.register_load(
        ResourceLoadKind::Script,
        ResourceLoadDisposition::Ordinary,
        None,
    ) else {
        let _ = completion_tx.send(WorkerModuleGraphFetchCompletion::new(
            fetch_id,
            Err("worker module dependency fetch rejected during shutdown".to_owned()),
        ));
        return;
    };
    if let Err(error) = loader
        .request_client()
        .fetch_cacheable_script_text_callback_with_load(fetch_request, load, send_completion)
    {
        let mut completion = WorkerModuleGraphFetchCompletion::new(
            fetch_id,
            Err(format!(
                "failed to start module worker dependency fetch `{}`: {error}",
                request.url()
            )),
        );
        if let Some(violation) =
            content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
                &content_security_report_only_policies,
                request.initiator_url(),
                request.url(),
                resource_kind,
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Report,
                &content_security_reporting_endpoints,
            )
        {
            completion = completion.with_csp_report_only_violation(violation);
        }
        let _ = completion_tx.send(completion);
    }
}

fn start_worker_module_graph_fetch_batch(
    requests: WorkerModuleGraphFetchBatch,
    state: &Rc<RefCell<WorkerGlobalState>>,
    module_graph_fetch_tx: &mpsc::UnboundedSender<WorkerModuleGraphFetchCompletion>,
) {
    for request in requests.iter().cloned() {
        start_worker_module_graph_fetch(
            request,
            state.borrow().loader.clone(),
            state.borrow().network_partition_key.clone(),
            state
                .borrow()
                .module_static_import_content_security_policies
                .clone(),
            state.borrow().content_security_policies.clone(),
            state.borrow().content_security_report_only_policies.clone(),
            state.borrow().content_security_reporting_endpoints.clone(),
            module_graph_fetch_tx.clone(),
        );
    }
}

fn worker_script_resource_kind_for_module(kind: WorkerModuleKind) -> WorkerScriptResourceKind {
    match kind {
        WorkerModuleKind::JavaScript => WorkerScriptResourceKind::JavaScript,
        WorkerModuleKind::Json => WorkerScriptResourceKind::JsonModule,
        WorkerModuleKind::WebAssembly => WorkerScriptResourceKind::WebAssemblyModule,
    }
}

fn drain_worker_file_reader_queue(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
) {
    loop {
        let scope = pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let ctx = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, ctx);
        if !flush_one_pending_file_reader(scope) {
            break;
        }
        perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    }
}

fn worker_has_pending_module_runtime_activity(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
) -> bool {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    worker_has_pending_dynamic_module_imports(scope)
}

fn worker_has_runnable_module_runtime_activity(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
) -> bool {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    worker_has_runnable_dynamic_module_imports(scope)
}

fn worker_has_pending_indexed_db_tasks(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
) -> bool {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    crate::context_bootstrap::indexed_db_has_pending_tasks(scope)
}

fn flush_one_worker_indexed_db_task(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    module_graph_fetch_tx: &mpsc::UnboundedSender<WorkerModuleGraphFetchCompletion>,
) {
    let flushed = crate::context_bootstrap::flush_next_indexed_db_task(scope);
    if !flushed {
        return;
    }
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    drain_worker_dynamic_module_imports(scope, state, module_graph_fetch_tx);
}

fn handle_worker_dynamic_module_import_advance(
    advance: WorkerDynamicModuleImportAdvance,
    state: &Rc<RefCell<WorkerGlobalState>>,
    module_graph_fetch_tx: &mpsc::UnboundedSender<WorkerModuleGraphFetchCompletion>,
) {
    match advance {
        WorkerDynamicModuleImportAdvance::Complete
        | WorkerDynamicModuleImportAdvance::WaitingFetches
        | WorkerDynamicModuleImportAdvance::WaitingEvaluation { .. } => {}
        WorkerDynamicModuleImportAdvance::NeedFetches(requests) => {
            start_worker_module_graph_fetch_batch(requests, state, module_graph_fetch_tx);
        }
    }
}

fn report_service_worker_module_script_resource(
    state: &Rc<RefCell<WorkerGlobalState>>,
    resource: WorkerScriptResource,
) {
    let state = state.borrow();
    let WorkerGlobalKind::Service {
        registration_id,
        version_id,
        ..
    } = &state.global_kind
    else {
        return;
    };
    let _ = state
        .parent_tx
        .send(WorkerToParentMessage::ServiceWorkerImportedScriptLoaded {
            registration_id: *registration_id,
            version_id: *version_id,
            resource,
        });
}

fn drain_worker_dynamic_module_imports(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    module_graph_fetch_tx: &mpsc::UnboundedSender<WorkerModuleGraphFetchCompletion>,
) {
    const MAX_DYNAMIC_IMPORT_DRAIN_STEPS: usize = 1_000;
    for _ in 0..MAX_DYNAMIC_IMPORT_DRAIN_STEPS {
        let Some(advance) = run_next_worker_dynamic_module_import(scope) else {
            break;
        };
        handle_worker_dynamic_module_import_advance(advance, state, module_graph_fetch_tx);
    }
}

fn drain_worker_dynamic_module_imports_for_context(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    module_graph_fetch_tx: &mpsc::UnboundedSender<WorkerModuleGraphFetchCompletion>,
) {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    drain_worker_dynamic_module_imports(scope, state, module_graph_fetch_tx);
}

fn dispatch_worker_inspector_task(
    worker_isolate: &mut WorkerIsolateState,
    context: &v8::Global<v8::Context>,
    task: WorkerInspectorTask,
    state: &Rc<RefCell<WorkerGlobalState>>,
    module_graph_fetch_tx: &mpsc::UnboundedSender<WorkerModuleGraphFetchCompletion>,
) {
    let (isolate, inspector) = worker_isolate.worker_isolate_and_runtime_inspector();
    inspector.execute_task(isolate, task);
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    drain_worker_dynamic_module_imports(scope, state, module_graph_fetch_tx);
}

async fn run_worker_pre_bootstrap_debugger_pause(
    worker_isolate: &mut WorkerIsolateState,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    module_graph_fetch_tx: &mpsc::UnboundedSender<WorkerModuleGraphFetchCompletion>,
    script_url: &str,
    rx: &mut mpsc::UnboundedReceiver<WorkerMessage>,
    pending_bootstrap_messages: &mut VecDeque<WorkerMessage>,
    inspector_task_runner: &WorkerInspectorTaskRunner,
) -> bool {
    loop {
        if inspector_task_runner.take_resume_requested() {
            return true;
        }
        match rx.recv().await {
            Some(WorkerMessage::RunInspectorTask(mode)) => {
                if let Some(task) = inspector_task_runner.claim_task(mode) {
                    dispatch_worker_inspector_task(
                        worker_isolate,
                        context,
                        task,
                        state,
                        module_graph_fetch_tx,
                    );
                }
                if mode == WorkerInspectorTaskMode::Interrupt {
                    inspector_task_runner.request_interrupt_if_needed();
                }
                if inspector_task_runner.take_resume_requested() {
                    return true;
                }
            }
            Some(WorkerMessage::SetExtraHttpHeaders(headers)) => {
                let (loader, headers_for_loader) = {
                    let mut state = state.borrow_mut();
                    state.extra_http_headers = headers;
                    (state.loader.clone(), state.extra_http_headers.clone())
                };
                loader
                    .request_client()
                    .set_extra_http_headers(&headers_for_loader);
            }
            Some(WorkerMessage::SetNetworkOffline(offline)) => {
                let loader = {
                    let mut state = state.borrow_mut();
                    state.network_offline = offline;
                    state.loader.clone()
                };
                loader.request_client().set_network_offline(offline);
            }
            Some(WorkerMessage::SetBlockedUrlPatterns(patterns)) => {
                let (loader, patterns_for_loader) = {
                    let mut state = state.borrow_mut();
                    state.blocked_url_patterns = patterns;
                    (state.loader.clone(), state.blocked_url_patterns.clone())
                };
                loader
                    .request_client()
                    .set_blocked_url_patterns(&patterns_for_loader);
            }
            Some(WorkerMessage::SetFetchSubresourceInterception {
                enabled,
                resource_type,
            }) => {
                let mut state = state.borrow_mut();
                state.fetch_subresource_interception_enabled = enabled;
                state.fetch_subresource_interception_resource_type = resource_type;
            }
            Some(WorkerMessage::Terminate) => {
                trace!(url = %script_url, "worker terminated before bootstrap by parent");
                return false;
            }
            Some(message) => {
                pending_bootstrap_messages.push_back(message);
            }
            None => {
                trace!(url = %script_url, "worker channel closed before bootstrap");
                return false;
            }
        }
    }
}

fn drain_worker_runtime_protocol_messages(
    inspector: &WorkerRuntimeInspector,
) -> Vec<WorkerRuntimeInspectorMessageBatch> {
    inspector.take_pending_messages()
}

#[cfg(test)]
fn worker_resource_owner_slot_diagnostics(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    state: &Rc<RefCell<WorkerGlobalState>>,
) -> Result<crate::worker::handle::WorkerResourceOwnerSlotDiagnostics, String> {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);
    let context_owner = scope
        .get_current_context()
        .get_slot::<crate::resource_owner::ResourceOwnerId>()
        .as_deref()
        .copied();
    let navigator_diagnostics = crate::context_bootstrap::navigator_storage_wrapper_diagnostics(
        scope,
    )
    .ok_or_else(|| "worker navigator storage-wrapper diagnostics are unavailable".to_owned())?;
    Ok(crate::worker::handle::WorkerResourceOwnerSlotDiagnostics {
        context_slot_has_owner: context_owner.is_some(),
        current_owner_matches_context: crate::resource_owner::current_resource_owner_id(scope)
            == context_owner,
        isolate_slot_has_owner: scope
            .get_slot::<crate::resource_owner::ResourceOwnerId>()
            .is_some(),
        opfs_owner_state_materialized: state.borrow().opfs_owner_state.is_some(),
        materialized_interfaces: crate::context_bootstrap::lazy_materialized_constructor_names(
            scope,
        ),
        storage_constructor_materializations:
            crate::context_bootstrap::lazy_storage_constructor_materialization_count(scope),
        storage_manager_materialized: navigator_diagnostics.storage_manager_materialized,
        storage_bucket_manager_materialized: navigator_diagnostics
            .storage_bucket_manager_materialized,
    })
}

fn forward_pending_worker_runtime_protocol_messages(
    inspector: &WorkerRuntimeInspector,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
) {
    let messages = drain_worker_runtime_protocol_messages(inspector);
    if !messages.is_empty() {
        let _ = parent_tx.send(WorkerToParentMessage::RuntimeInspectorMessages(messages));
    }
}

fn forward_worker_script_loaded(
    inspector: &WorkerRuntimeInspector,
    parent_tx: &mpsc::UnboundedSender<WorkerToParentMessage>,
) {
    // Blink notifies each attached worker Inspector agent only after top-level
    // evaluation completes. Flush V8 notifications first so console and
    // exception events retain their Chromium ordering before this terminal.
    forward_pending_worker_runtime_protocol_messages(inspector, parent_tx);
    let messages = inspector.worker_script_loaded_messages();
    if !messages.is_empty() {
        let _ = parent_tx.send(WorkerToParentMessage::RuntimeInspectorMessages(messages));
    }
}

/// Spawn a dedicated worker thread.
///
/// `script_source` is the JavaScript source to evaluate in the worker context.
/// `script_url` is the URL associated with the script (for error reporting).
///
/// Returns a `WorkerHandle` the parent can use for communication.
#[cfg(test)]
pub(crate) fn spawn_worker(script_source: String, script_url: String) -> WorkerTestHandle {
    spawn_test_worker_with_options(WorkerSpawnOptions::new(script_source, script_url))
}

#[cfg(test)]
pub(crate) fn spawn_worker_with_request_client(
    script_source: String,
    script_url: String,
    loader: impl Into<WorkerTestRequestClient>,
) -> WorkerTestHandle {
    spawn_test_worker_with_options(
        WorkerSpawnOptions::new(script_source, script_url)
            .with_request_client(loader)
            .with_script_kind(WorkerScriptKind::Classic),
    )
}

#[cfg(test)]
pub(crate) fn spawn_worker_with_request_client_and_kind(
    script_source: String,
    script_url: String,
    loader: impl Into<WorkerTestRequestClient>,
    script_kind: WorkerScriptKind,
) -> WorkerTestHandle {
    spawn_test_worker_with_options(
        WorkerSpawnOptions::new(script_source, script_url)
            .with_request_client(loader)
            .with_script_kind(script_kind),
    )
}

#[cfg(test)]
pub(crate) fn spawn_worker_with_request_client_and_blocked_url_patterns(
    script_source: String,
    script_url: String,
    loader: impl Into<WorkerTestRequestClient>,
    blocked_url_patterns: Vec<String>,
) -> WorkerTestHandle {
    spawn_test_worker_with_options(
        WorkerSpawnOptions::new(script_source, script_url)
            .with_request_client(loader)
            .with_network_policy(WorkerNetworkPolicy {
                blocked_url_patterns,
                ..WorkerNetworkPolicy::default()
            }),
    )
}

#[cfg(test)]
pub(crate) fn spawn_worker_with_request_client_and_network_policy(
    script_source: String,
    script_url: String,
    loader: impl Into<WorkerTestRequestClient>,
    network_policy: WorkerNetworkPolicy,
) -> WorkerTestHandle {
    spawn_test_worker_with_options(
        WorkerSpawnOptions::new(script_source, script_url)
            .with_request_client(loader)
            .with_network_policy(network_policy),
    )
}

#[cfg(test)]
pub(crate) fn spawn_worker_with_request_client_and_kind_and_network_policy(
    script_source: String,
    script_url: String,
    loader: impl Into<WorkerTestRequestClient>,
    script_kind: WorkerScriptKind,
    network_policy: WorkerNetworkPolicy,
) -> WorkerTestHandle {
    spawn_test_worker_with_options(
        WorkerSpawnOptions::new(script_source, script_url)
            .with_request_client(loader)
            .with_script_kind(script_kind)
            .with_network_policy(network_policy),
    )
}

#[cfg(test)]
pub(crate) fn spawn_worker_with_request_client_and_kind_network_policy_and_broadcast_channel_registry(
    script_source: String,
    script_url: String,
    loader: impl Into<WorkerTestRequestClient>,
    script_kind: WorkerScriptKind,
    network_policy: WorkerNetworkPolicy,
    broadcast_channel_registry: SharedBroadcastChannelRegistry,
    storage_key_top_level_site: Option<String>,
    indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
) -> WorkerTestHandle {
    spawn_test_worker_with_options(
        WorkerSpawnOptions::new(script_source, script_url)
            .with_request_client(loader)
            .with_script_kind(script_kind)
            .with_network_policy(network_policy)
            .with_worker_context_runtime(RendererWorkerContextRuntime::new(
                crate::message_port_runtime::new_message_port_registry(),
                broadcast_channel_registry,
            ))
            .with_storage_key_top_level_site(storage_key_top_level_site)
            .with_indexed_db_manager(indexed_db_manager),
    )
}

#[cfg(test)]
pub(crate) fn spawn_worker_with_source_and_kind_and_network_policy(
    script_source: WorkerScriptSource,
    script_url: String,
    request_client: impl Into<WorkerTestRequestClient>,
    script_kind: WorkerScriptKind,
    network_policy: WorkerNetworkPolicy,
) -> WorkerTestHandle {
    spawn_worker_with_source_and_kind_network_policy_and_broadcast_channel_registry(
        script_source,
        script_url,
        request_client,
        script_kind,
        network_policy,
        new_broadcast_channel_registry(),
        None,
        None,
    )
}

#[cfg(test)]
pub(crate) fn spawn_worker_with_source_and_kind_network_policy_and_broadcast_channel_registry(
    script_source: WorkerScriptSource,
    script_url: String,
    request_client: impl Into<WorkerTestRequestClient>,
    script_kind: WorkerScriptKind,
    network_policy: WorkerNetworkPolicy,
    broadcast_channel_registry: SharedBroadcastChannelRegistry,
    storage_key_top_level_site: Option<String>,
    indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
) -> WorkerTestHandle {
    spawn_test_worker_with_options(
        WorkerSpawnOptions::with_source(script_source, script_url)
            .with_request_client(request_client)
            .with_script_kind(script_kind)
            .with_network_policy(network_policy)
            .with_worker_context_runtime(RendererWorkerContextRuntime::new(
                crate::message_port_runtime::new_message_port_registry(),
                broadcast_channel_registry,
            ))
            .with_storage_key_top_level_site(storage_key_top_level_site)
            .with_indexed_db_manager(indexed_db_manager),
    )
}

#[cfg(test)]
fn worker_test_request_client() -> ResourceRequestClientOwner {
    ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("Worker test request client should initialize")
}

#[cfg(test)]
pub(crate) fn spawn_test_worker_with_options(mut options: WorkerSpawnOptions) -> WorkerTestHandle {
    let request_client_owner = options.test_request_client_owner.take();
    let handle = spawn_worker_with_options(options);
    WorkerTestHandle {
        handle: Some(handle),
        _request_client_owner: request_client_owner,
    }
}

pub(crate) fn spawn_worker_with_options(options: WorkerSpawnOptions) -> WorkerHandle {
    #[cfg(test)]
    assert!(
        options.test_request_client_owner.is_none(),
        "test-owned request clients must use spawn_test_worker_with_options"
    );
    let WorkerSpawnOptions {
        script_source,
        script_url,
        request_client,
        script_kind,
        module_static_import_initiator_url,
        module_credentials_mode,
        referrer_policy,
        module_static_import_content_security_policies,
        content_security_policies,
        content_security_report_only_policies,
        content_security_reporting_endpoints,
        network_policy,
        policy_context,
        worker_context_runtime,
        global_kind,
        api_storage_key,
        broadcast_channel_top_level_site,
        creator_storage_key,
        service_worker_runtime,
        reserved_service_worker_client_id,
        indexed_db_manager,
        storage_bucket_store,
        bootstrap_completion_tx,
        pause_evaluation_until_debugger,
        #[cfg(test)]
            test_request_client_owner: _,
    } = options;
    let (parent_to_worker_tx, parent_to_worker_rx) = mpsc::unbounded_channel::<WorkerMessage>();
    let (worker_to_parent_tx, worker_to_parent_rx) =
        mpsc::unbounded_channel::<WorkerToParentMessage>();
    let worker_wake_tx = parent_to_worker_tx.clone();
    let isolate_handle = Arc::new(Mutex::new(None));
    let worker_isolate_handle = Arc::clone(&isolate_handle);
    let devtools =
        WorkerDevToolsHandle::new(parent_to_worker_tx.clone(), Arc::clone(&isolate_handle));
    let worker_inspector_tasks = devtools.inspector_tasks().clone();
    let termination_requested = Arc::new(AtomicBool::new(false));
    let worker_termination_requested = Arc::clone(&termination_requested);

    let join_handle = std::thread::Builder::new()
        .name(format!("worker:{script_url}"))
        .stack_size(WORKER_STACK_SIZE)
        .spawn(move || {
            let mut runtime_builder = tokio::runtime::Builder::new_current_thread();
            runtime_builder
                .max_blocking_threads(crate::tokio_blocking_budget::tokio_blocking_thread_budget())
                .enable_all();
            let runtime = runtime_builder
                .build_local(tokio::runtime::LocalOptions::default())
                .expect("failed to build worker runtime");
            runtime.block_on(worker_main(
                script_source,
                script_url,
                request_client,
                script_kind,
                module_static_import_initiator_url,
                module_credentials_mode,
                referrer_policy,
                module_static_import_content_security_policies,
                content_security_policies,
                content_security_report_only_policies,
                content_security_reporting_endpoints,
                network_policy,
                policy_context,
                worker_context_runtime,
                global_kind,
                api_storage_key,
                broadcast_channel_top_level_site,
                creator_storage_key,
                service_worker_runtime,
                reserved_service_worker_client_id,
                indexed_db_manager,
                storage_bucket_store,
                bootstrap_completion_tx,
                pause_evaluation_until_debugger,
                worker_wake_tx,
                parent_to_worker_rx,
                worker_to_parent_tx,
                worker_isolate_handle,
                worker_termination_requested,
                worker_inspector_tasks,
            ));
        })
        .expect("failed to spawn worker thread");

    WorkerHandle::new_with_termination_requested_and_devtools(
        parent_to_worker_tx,
        worker_to_parent_rx,
        join_handle,
        isolate_handle,
        termination_requested,
        devtools,
    )
}

fn worker_broadcast_channel_storage_key(
    script_url: Option<&url::Url>,
    worker_storage_key: &MoliStorageKey,
    global_kind: &WorkerGlobalKind,
) -> MoliStorageKey {
    if script_url.is_some_and(|url| url.scheme() == "data") {
        return worker_storage_key.clone();
    }
    if let WorkerGlobalKind::Shared { storage_key, .. } = global_kind {
        return storage_key.clone();
    }
    worker_storage_key.clone()
}

fn worker_global_storage_key(
    script_url: Option<&url::Url>,
    api_storage_key: Option<BroadcastChannelStorageKey>,
    broadcast_channel_top_level_site: Option<String>,
    creator_storage_key: Option<MoliStorageKey>,
    registry: &SharedBroadcastChannelRegistry,
    global_kind: &WorkerGlobalKind,
) -> MoliStorageKey {
    if script_url.is_some_and(|url| url.scheme() != "data") {
        if let WorkerGlobalKind::Shared { storage_key, .. } = global_kind {
            return storage_key.clone();
        }
        if let Some(storage_key) = api_storage_key {
            return storage_key;
        }
        if let Some(storage_key) = creator_storage_key {
            return storage_key;
        }
    }
    let Some(script_url) = script_url else {
        let top_level_site = broadcast_channel_top_level_site.unwrap_or_else(|| "null".to_owned());
        return MoliStorageKey::new(
            "null".to_owned(),
            top_level_site.clone(),
            Some(registry.next_opaque_context_nonce()),
            moli_storage_key::StoragePartitionRelation::Unknown,
        );
    };
    let top_level_site = broadcast_channel_top_level_site
        .unwrap_or_else(|| moli_storage_key::site_for_url(script_url));
    let opaque_nonce = if moli_storage_key::url_needs_opaque_nonce(script_url) {
        Some(registry.next_opaque_context_nonce())
    } else {
        None
    };
    MoliStorageKey::from_url_and_top_level_site(script_url, top_level_site, opaque_nonce)
}

fn resource_loader_for_worker_context(
    request_client: ResourceRequestClient,
    network_policy: &WorkerNetworkPolicy,
    global_kind: &WorkerGlobalKind,
    task_runner: crate::network::RendererResourceTaskRunner,
) -> WorkerResourceLoader {
    // Workers share the creator's browser transport/cache, but own their
    // mutable execution-context policy. Parent policy changes are forwarded
    // through typed Worker messages below; sharing the Page Arc here would
    // make a Worker mutation write back into its creator.
    let worker_request_client = request_client.fork_with_isolated_worker_network_policy();
    worker_request_client.set_extra_http_headers(&network_policy.extra_http_headers);
    worker_request_client.set_network_offline(network_policy.network_offline);
    worker_request_client.set_blocked_url_patterns(&network_policy.blocked_url_patterns);
    let owner = match global_kind {
        WorkerGlobalKind::Dedicated { name } => WorkerResourceOwner::Dedicated {
            name: name.clone().into_boxed_str(),
        },
        WorkerGlobalKind::Shared { name, .. } => WorkerResourceOwner::Shared {
            name: name.clone().into_boxed_str(),
        },
        WorkerGlobalKind::Service {
            registration_id,
            version_id,
            ..
        } => WorkerResourceOwner::Service {
            registration_id: registration_id.as_u64(),
            version_id: version_id.as_u64(),
        },
    };
    WorkerResourceLoader::new(worker_request_client, owner, task_runner)
}

/// Worker main loop.
async fn worker_main(
    script_source: WorkerScriptSource,
    script_url: String,
    request_client: ResourceRequestClient,
    script_kind: WorkerScriptKind,
    module_static_import_initiator_url: Option<url::Url>,
    module_credentials_mode: RequestCredentialsMode,
    referrer_policy: Option<String>,
    module_static_import_content_security_policies: Vec<String>,
    content_security_policies: Vec<String>,
    content_security_report_only_policies: Vec<String>,
    content_security_reporting_endpoints: ContentSecurityPolicyReportingEndpoints,
    network_policy: WorkerNetworkPolicy,
    policy_context: crate::types::SubresourcePolicyContext,
    worker_context_runtime: RendererWorkerContextRuntime,
    global_kind: WorkerGlobalKind,
    api_storage_key: Option<BroadcastChannelStorageKey>,
    broadcast_channel_top_level_site: Option<String>,
    creator_storage_key: Option<MoliStorageKey>,
    service_worker_runtime: Option<ServiceWorkerRuntimeService>,
    reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
    indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    bootstrap_completion_tx: Option<mpsc::UnboundedSender<WorkerBootstrapCompletion>>,
    pause_evaluation_until_debugger: bool,
    worker_wake_tx: mpsc::UnboundedSender<WorkerMessage>,
    mut rx: mpsc::UnboundedReceiver<WorkerMessage>,
    parent_tx: mpsc::UnboundedSender<WorkerToParentMessage>,
    isolate_handle: Arc<Mutex<Option<v8::IsolateHandle>>>,
    termination_requested: Arc<AtomicBool>,
    inspector_task_runner: WorkerInspectorTaskRunner,
) {
    debug!(url = %script_url, "worker started");
    let mut bootstrap_completion = WorkerBootstrapCompletionReporter::new(bootstrap_completion_tx);
    let resource_task_runner = crate::network::RendererResourceTaskRunner::from_current_tokio()
        .expect("Worker owner loop must expose its resource task runner");
    let loader = resource_loader_for_worker_context(
        request_client,
        &network_policy,
        &global_kind,
        resource_task_runner,
    );

    // ── Create worker-level V8 isolate ─────────────────────────────────────
    let worker_indexed_db_manager = indexed_db_manager.clone();
    let storage_bucket_store = storage_bucket_store.unwrap_or_else(|| {
        indexed_db_manager
            .as_ref()
            .and_then(crate::context_bootstrap::WeakIndexedDbManager::upgrade)
            .map(|manager| {
                crate::context_bootstrap::new_shared_storage_bucket_store_with_indexed_db_manager(
                    &manager,
                )
            })
            .unwrap_or_else(crate::context_bootstrap::new_shared_storage_bucket_store)
    });
    let worker_storage_bucket_store = Some(storage_bucket_store.clone());
    let (worker_runtime_wake_tx, mut worker_runtime_wake_rx) = mpsc::unbounded_channel::<()>();
    let resource_owner_id = crate::resource_owner::ResourceOwnerId::new();
    let mut worker_isolate = WorkerIsolateState::new(
        crate::v8_platform::V8ForegroundTaskWake::worker(worker_runtime_wake_tx.clone()),
        inspector_task_runner.clone(),
        parent_tx.clone(),
        matches!(global_kind, WorkerGlobalKind::Shared { .. }),
    );
    install_worker_promise_rejection_dispatch(
        worker_isolate.worker_isolate_mut(),
        parent_tx.clone(),
        worker_wake_tx.clone(),
        script_url.clone(),
    );

    // Worker-owned isolate timer queues are shared between V8 callbacks and the
    // worker event loop. Document/page runtimes must not depend on this slot.
    let worker_timer_queues = WorkerIsolateTimerQueues::default();
    worker_isolate
        .worker_isolate_mut()
        .set_slot(worker_timer_queues.clone());

    // Worker global state (accessible from JS callbacks).
    let (fetch_completion_tx, mut fetch_completion_rx) =
        mpsc::unbounded_channel::<WorkerFetchEvent>();
    let (xhr_completion_tx, mut xhr_completion_rx) =
        mpsc::unbounded_channel::<WorkerXhrCompletion>();
    let (module_graph_fetch_tx, mut module_graph_fetch_rx) =
        mpsc::unbounded_channel::<WorkerModuleGraphFetchCompletion>();
    let (module_evaluation_tx, mut module_evaluation_rx) =
        mpsc::unbounded_channel::<WorkerModuleEvaluationCompletion>();
    let (websocket_event_tx, mut websocket_event_rx) =
        tokio::sync::mpsc::channel::<moli_websocket::Event>(1);
    let (webcrypto_completion_tx, mut webcrypto_completion_rx) =
        mpsc::unbounded_channel::<WorkerWebCryptoCompletion>();
    let (opfs_completion_tx, mut opfs_completion_rx) =
        mpsc::unbounded_channel::<WorkerOpfsCompletion>();
    let message_port_registry = worker_context_runtime.message_port_registry();
    let broadcast_channel_registry = worker_context_runtime.broadcast_channel_registry();
    let current_script_url = url::Url::parse(&script_url).ok();
    let secure_context = network_policy.secure_context
        || current_script_url
            .as_ref()
            .is_some_and(|url| crate::worker::worker_secure_context_for_script_url(url, false));
    let storage_key = worker_global_storage_key(
        current_script_url.as_ref(),
        api_storage_key.clone(),
        broadcast_channel_top_level_site.clone(),
        creator_storage_key,
        &broadcast_channel_registry,
        &global_kind,
    );
    let broadcast_channel_storage_key = worker_broadcast_channel_storage_key(
        current_script_url.as_ref(),
        &storage_key,
        &global_kind,
    );
    let service_worker_worker_client = register_service_worker_worker_client(
        service_worker_runtime.clone(),
        reserved_service_worker_client_id,
        current_script_url.as_ref(),
        &storage_key,
        &global_kind,
        secure_context,
        worker_wake_tx.clone(),
    );
    let service_worker_client_id = service_worker_worker_client
        .as_ref()
        .map(|client| client.client_id);
    let state = Rc::new(RefCell::new(WorkerGlobalState {
        v8_finalizers: crate::v8_finalizer::V8FinalizerRegistry::default(),
        parent_tx: parent_tx.clone(),
        worker_wake_tx,
        termination_requested: Arc::clone(&termination_requested),
        closed: false,
        next_timer_id: 0,
        loader,
        global_kind,
        script_kind,
        current_script_url,
        referrer_policy,
        module_static_import_content_security_policies,
        content_security_policies,
        content_security_report_only_policies,
        content_security_reporting_endpoints,
        secure_context,
        permission_overrides: network_policy.permission_overrides,
        extra_http_headers: network_policy.extra_http_headers,
        network_offline: network_policy.network_offline,
        blocked_url_patterns: network_policy.blocked_url_patterns,
        network_partition_key: network_policy.network_partition_key,
        policy_context,
        fetch_subresource_interception_enabled: network_policy
            .fetch_subresource_interception_enabled,
        fetch_subresource_interception_resource_type: network_policy
            .fetch_subresource_interception_resource_type,
        fetch_completion_tx,
        pending_fetches: std::collections::HashMap::new(),
        pending_network_body_sources: std::collections::HashMap::new(),
        pending_network_body_clones: std::collections::HashMap::new(),
        next_fetch_id: 0,
        xhr_completion_tx,
        pending_xhrs: std::collections::HashMap::new(),
        next_xhr_id: 0,
        pending_csp_reports: std::collections::HashMap::new(),
        text_codecs: crate::text_codec::TextCodecStore::default(),
        abort: Rc::new(RefCell::new(super::abort::WorkerAbortStore::default())),
        worker_context_runtime,
        service_worker_runtime,
        service_worker_client_id,
        message_port_registry,
        message_port_wrappers: std::collections::HashMap::new(),
        shared_worker_connection_ports: std::collections::HashSet::new(),
        broadcast_channel_registry,
        broadcast_channel_storage_key,
        storage_key,
        broadcast_channel_wrappers: std::collections::HashMap::new(),
        indexed_db_manager: worker_indexed_db_manager,
        storage_bucket_store: worker_storage_bucket_store,
        websocket_event_tx,
        websockets: std::collections::HashMap::new(),
        next_websocket_id: 0,
        next_nested_worker_id: 1,
        nested_worker_wrappers: std::collections::HashMap::new(),
        webcrypto_completion_tx,
        pending_webcrypto: std::collections::HashMap::new(),
        next_webcrypto_task_id: 1,
        opfs_completion_tx,
        opfs_owner_state: None,
        pending_service_worker_periodic_sync_events: std::collections::HashMap::new(),
        pending_service_worker_client_queries: std::collections::HashMap::new(),
        next_service_worker_client_query_id: 1,
        pending_service_worker_client_navigates: std::collections::HashMap::new(),
        next_service_worker_client_navigate_id: 1,
        pending_service_worker_client_focuses: std::collections::HashMap::new(),
        next_service_worker_client_focus_id: 1,
        pending_service_worker_clients_open_windows: std::collections::HashMap::new(),
        next_service_worker_clients_open_window_id: 1,
        pending_service_worker_show_notifications: std::collections::HashMap::new(),
        next_service_worker_show_notification_id: 1,
        pending_service_worker_get_notifications: std::collections::HashMap::new(),
        next_service_worker_get_notifications_id: 1,
        pending_service_worker_sync_registrations: std::collections::HashMap::new(),
        next_service_worker_sync_registration_id: 1,
        pending_service_worker_sync_get_tags: std::collections::HashMap::new(),
        next_service_worker_sync_get_tags_id: 1,
        pending_service_worker_periodic_sync_registrations: std::collections::HashMap::new(),
        next_service_worker_periodic_sync_registration_id: 1,
        pending_service_worker_periodic_sync_get_tags: std::collections::HashMap::new(),
        next_service_worker_periodic_sync_get_tags_id: 1,
        pending_service_worker_periodic_sync_unregistrations: std::collections::HashMap::new(),
        next_service_worker_periodic_sync_unregistration_id: 1,
        pending_service_worker_push_subscriptions: std::collections::HashMap::new(),
        next_service_worker_push_subscription_id: 1,
        pending_service_worker_push_get_subscriptions: std::collections::HashMap::new(),
        next_service_worker_push_get_subscription_id: 1,
        pending_service_worker_push_unsubscriptions: std::collections::HashMap::new(),
        next_service_worker_push_unsubscription_id: 1,
        service_worker_window_interaction_allowed_count: 0,
        pending_service_worker_lifecycle_events: std::collections::HashMap::new(),
        pending_service_worker_fetch_events: std::collections::HashMap::new(),
        pending_service_worker_navigation_preloads: std::collections::HashMap::new(),
        pending_service_worker_message_events: std::collections::HashMap::new(),
        pending_service_worker_notification_events: std::collections::HashMap::new(),
        pending_service_worker_push_events: std::collections::HashMap::new(),
        pending_service_worker_sync_events: std::collections::HashMap::new(),
    }));
    let context;
    let mut pending_module_bootstrap: Option<Box<WorkerModulePendingBootstrap>> = None;
    // Parent messages received while the initial script is not ready are task
    // queued until bootstrap has installed user handlers.
    let mut pending_bootstrap_messages: VecDeque<WorkerMessage> = VecDeque::new();
    let mut bootstrap_failed = false;
    let mut install_global_failed = false;
    {
        let (isolate, runtime_inspector) = worker_isolate.worker_isolate_and_runtime_inspector();
        let scope = pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        *isolate_handle.lock() = Some(scope.thread_safe_handle());
        let ctx = v8::Context::new(scope, Default::default());
        crate::resource_owner::install_resource_owner_for_context(ctx, resource_owner_id);
        crate::context_bootstrap::set_indexed_db_manager_for_context(
            ctx,
            indexed_db_manager.clone(),
        );
        crate::context_bootstrap::set_worker_indexed_db_task_wake_for_context(
            ctx,
            worker_runtime_wake_tx.clone(),
        );
        crate::context_bootstrap::set_storage_bucket_store_for_context(
            ctx,
            Some(storage_bucket_store.clone()),
        );
        context = v8::Global::new(scope, ctx);
        runtime_inspector.attach_context(ctx, v8::Global::new(scope, ctx), &script_url);

        let scope = &mut v8::ContextScope::new(scope, ctx);
        let global = ctx.global(scope);
        if let Err(e) = install_worker_global_scope(scope, global, state.clone()) {
            tracing::error!(url = %script_url, error = %e, "failed to install worker global scope");
            bootstrap_completion.mark_install_global_failure(&script_url, e.to_string());
            install_global_failed = true;
        } else if script_kind == WorkerScriptKind::Classic {
            let referrer_policy = { state.borrow().referrer_policy.clone() };
            install_classic_worker_dynamic_module_runtime(
                scope,
                referrer_policy,
                module_evaluation_tx.clone(),
            );
        }
    }
    if install_global_failed {
        inspector_task_runner.dispose("Worker global installation failed");
        *isolate_handle.lock() = None;
        worker_isolate.unregister_worker_isolate_platform();
        return;
    }
    // Commands can queue before the isolate exists. Arm V8 interrupts only
    // after the worker context and its Inspector routing are fully installed.
    inspector_task_runner.activate_isolate();

    let mut terminated_before_bootstrap = false;
    if pause_evaluation_until_debugger
        && !run_worker_pre_bootstrap_debugger_pause(
            &mut worker_isolate,
            &context,
            &state,
            &module_graph_fetch_tx,
            &script_url,
            &mut rx,
            &mut pending_bootstrap_messages,
            &inspector_task_runner,
        )
        .await
    {
        state.borrow_mut().closed = true;
        terminated_before_bootstrap = true;
    }

    if !terminated_before_bootstrap {
        let (isolate, _) = worker_isolate.worker_isolate_and_runtime_inspector();
        let scope = pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let ctx = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, ctx);
        let global = ctx.global(scope);
        let referrer_policy = { state.borrow().referrer_policy.clone() };

        // ── Evaluate the worker script ─────────────────────────────────────
        match evaluate_worker_bootstrap_script(
            scope,
            &script_source,
            &script_url,
            script_kind,
            module_static_import_initiator_url,
            module_credentials_mode,
            referrer_policy,
            module_evaluation_tx.clone(),
        ) {
            WorkerBootstrapStart::Complete => {
                bootstrap_completion.mark_success(worker_bootstrap_success(scope, global, &state));
            }
            WorkerBootstrapStart::Pending(pending) => {
                if let Some(requests) = pending.pending_requests().cloned() {
                    start_worker_module_graph_fetch_batch(requests, &state, &module_graph_fetch_tx);
                }
                pending_module_bootstrap = Some(pending);
            }
            WorkerBootstrapStart::Failed { error, phase } => {
                let (report, exception, parent_event_kind) = *error;
                let exception = exception.as_ref().map(|value| v8::Local::new(scope, value));
                let error_source = if script_kind == WorkerScriptKind::Classic {
                    WorkerErrorSource::InitialScriptEvaluation
                } else {
                    WorkerErrorSource::Runtime
                };
                bootstrap_completion.mark_failure(
                    &report,
                    &script_url,
                    parent_event_kind,
                    phase,
                    error_source,
                );
                let handled = dispatch_worker_exception_with_phase_and_source(
                    scope,
                    global,
                    report,
                    exception,
                    parent_event_kind,
                    phase,
                    error_source,
                    &parent_tx,
                    &script_url,
                );
                bootstrap_failed = phase == WorkerErrorPhase::Bootstrap
                    && !handled
                    && parent_event_kind == WorkerParentErrorEventKind::Event;
            }
        }

        // Run microtask checkpoint after initial script evaluation.
        perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
        drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
    }
    if !terminated_before_bootstrap && pending_module_bootstrap.is_none() {
        forward_worker_script_loaded(worker_isolate.worker_runtime_inspector(), &parent_tx);
    }
    if bootstrap_failed {
        state.borrow_mut().closed = true;
    }

    // ── Event loop ─────────────────────────────────────────────────────────
    // Process incoming messages and timers until closed or terminated.
    let mut active_timers: Vec<ActiveTimer> = Vec::new();

    loop {
        if termination_requested.load(Ordering::Acquire) {
            trace!(url = %script_url, "worker termination won before the next task");
            break;
        }
        drain_worker_file_reader_queue(worker_isolate.worker_isolate_mut(), &context);

        if state.borrow().closed {
            trace!(url = %script_url, "worker closed via self.close()");
            break;
        }

        // Collect newly registered timers from JS.
        {
            for timer in worker_timer_queues.drain_pending() {
                let delay = Duration::from_millis(timer.delay_ms);
                active_timers.push(ActiveTimer {
                    id: timer.id,
                    callback: timer.callback,
                    delay,
                    is_interval: timer.is_interval,
                    extra_args: timer.extra_args,
                    next_fire: tokio::time::Instant::now() + delay,
                });
            }
        }

        // Remove cancelled timers.
        {
            let cancelled_timers = worker_timer_queues.drain_cancelled();
            if !cancelled_timers.is_empty() {
                let cancelled: std::collections::HashSet<u32> =
                    cancelled_timers.into_iter().collect();
                active_timers.retain(|t| !cancelled.contains(&t.id));
            }
        }

        // Find the nearest timer deadline.
        let nearest_timer = active_timers.iter().map(|t| t.next_fire).min();
        let has_pending_module_runtime_activity = worker_has_pending_module_runtime_activity(
            worker_isolate.worker_isolate_mut(),
            &context,
        );
        let has_runnable_module_runtime_activity = worker_has_runnable_module_runtime_activity(
            worker_isolate.worker_isolate_mut(),
            &context,
        );
        let has_pending_indexed_db_tasks =
            worker_has_pending_indexed_db_tasks(worker_isolate.worker_isolate_mut(), &context);

        enum WorkerLoopWake {
            Message(Option<WorkerMessage>),
            Fetch(Option<WorkerFetchEvent>),
            Xhr(Option<WorkerXhrCompletion>),
            ModuleGraphFetch(Option<Box<WorkerModuleGraphFetchCompletion>>),
            ModuleEvaluation(Option<WorkerModuleEvaluationCompletion>),
            ModuleRuntime,
            IndexedDb,
            RuntimeWake(Option<()>),
            WebSocket(Option<moli_websocket::Event>),
            WebCrypto(Option<WorkerWebCryptoCompletion>),
            Opfs(Option<WorkerOpfsCompletion>),
            Timer,
        }

        // Wait for either a message, a fetch completion, or a timer.
        let wake = if pending_module_bootstrap.is_none()
            && let Some(message) = pending_bootstrap_messages.pop_front()
        {
            WorkerLoopWake::Message(Some(message))
        } else if has_runnable_module_runtime_activity {
            WorkerLoopWake::ModuleRuntime
        } else if let Some(deadline) = nearest_timer {
            tokio::select! {
                msg = rx.recv() => WorkerLoopWake::Message(msg),
                completion = fetch_completion_rx.recv() => WorkerLoopWake::Fetch(completion),
                completion = xhr_completion_rx.recv() => WorkerLoopWake::Xhr(completion),
                completion = module_graph_fetch_rx.recv() => WorkerLoopWake::ModuleGraphFetch(completion.map(Box::new)),
                completion = module_evaluation_rx.recv() => WorkerLoopWake::ModuleEvaluation(completion),
                wake = worker_runtime_wake_rx.recv() => WorkerLoopWake::RuntimeWake(wake),
                event = websocket_event_rx.recv() => WorkerLoopWake::WebSocket(event),
                completion = webcrypto_completion_rx.recv() => WorkerLoopWake::WebCrypto(completion),
                completion = opfs_completion_rx.recv() => WorkerLoopWake::Opfs(completion),
                _ = async {}, if has_pending_indexed_db_tasks => WorkerLoopWake::IndexedDb,
                _ = tokio::time::sleep_until(deadline) => WorkerLoopWake::Timer,
            }
        } else {
            tokio::select! {
                msg = rx.recv() => WorkerLoopWake::Message(msg),
                completion = fetch_completion_rx.recv() => WorkerLoopWake::Fetch(completion),
                completion = xhr_completion_rx.recv() => WorkerLoopWake::Xhr(completion),
                completion = module_graph_fetch_rx.recv() => WorkerLoopWake::ModuleGraphFetch(completion.map(Box::new)),
                completion = module_evaluation_rx.recv() => WorkerLoopWake::ModuleEvaluation(completion),
                wake = worker_runtime_wake_rx.recv() => WorkerLoopWake::RuntimeWake(wake),
                event = websocket_event_rx.recv() => WorkerLoopWake::WebSocket(event),
                completion = webcrypto_completion_rx.recv() => WorkerLoopWake::WebCrypto(completion),
                completion = opfs_completion_rx.recv() => WorkerLoopWake::Opfs(completion),
                _ = async {}, if has_pending_indexed_db_tasks => WorkerLoopWake::IndexedDb,
            }
        };

        // `MessagePortWake` and `Terminate` can be sent through different
        // clones of the worker channel. The explicit lifecycle bit, rather
        // than cross-sender queue order, decides whether the selected task is
        // still allowed to enter V8.
        if termination_requested.load(Ordering::Acquire) {
            trace!(url = %script_url, "worker termination discarded a selected task");
            break;
        }

        // Process wake if any.
        match wake {
            WorkerLoopWake::Message(Some(WorkerMessage::Post(payload))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(WorkerMessage::Post(payload));
                    continue;
                }
                dispatch_message_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &payload,
                    &parent_tx,
                    &script_url,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::MessagePortWake(port_id))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(WorkerMessage::MessagePortWake(port_id));
                    continue;
                }
                let execution_terminated = dispatch_message_port_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    port_id,
                    &parent_tx,
                    &script_url,
                );
                if execution_terminated || termination_requested.load(Ordering::Acquire) {
                    trace!(url = %script_url, "worker termination interrupted MessagePort delivery");
                    break;
                }
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::SharedWorkerConnect(port_id))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::SharedWorkerConnect(port_id));
                    continue;
                }
                dispatch_shared_worker_connect_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    port_id,
                    &parent_tx,
                    &script_url,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::BroadcastChannelWake(channel_id))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::BroadcastChannelWake(channel_id));
                    continue;
                }
                dispatch_broadcast_channel_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    channel_id,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerLifecycleEvent(event))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerLifecycleEvent(event));
                    continue;
                }
                dispatch_service_worker_lifecycle_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *event,
                    &parent_tx,
                    &script_url,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerFetchEvent(event))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerFetchEvent(event));
                    continue;
                }
                dispatch_service_worker_fetch_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *event,
                    &parent_tx,
                    &script_url,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(
                WorkerMessage::ServiceWorkerNavigationPreloadResponseStarted(started),
            )) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerNavigationPreloadResponseStarted(started),
                    );
                    continue;
                }
                start_service_worker_navigation_preload_response(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *started,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(
                WorkerMessage::ServiceWorkerNavigationPreloadStreamChunk(chunk),
            )) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerNavigationPreloadStreamChunk(chunk),
                    );
                    continue;
                }
                enqueue_service_worker_navigation_preload_stream_chunk(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    chunk,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(
                WorkerMessage::ServiceWorkerNavigationPreloadStreamFinished(finished),
            )) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerNavigationPreloadStreamFinished(finished),
                    );
                    continue;
                }
                finish_service_worker_navigation_preload_stream(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *finished,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(
                WorkerMessage::ServiceWorkerNavigationPreloadFailure(failure),
            )) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerNavigationPreloadFailure(failure),
                    );
                    continue;
                }
                fail_service_worker_navigation_preload_response(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *failure,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerFetchStreamCancel {
                event_id,
                body_source_id,
            })) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerFetchStreamCancel {
                            event_id,
                            body_source_id,
                        },
                    );
                    continue;
                }
                cancel_service_worker_fetch_stream(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    event_id,
                    body_source_id,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(
                WorkerMessage::ServiceWorkerFetchRequestSignalAbort { event_id, reason },
            )) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerFetchRequestSignalAbort { event_id, reason },
                    );
                    continue;
                }
                abort_service_worker_fetch_request_signal(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    event_id,
                    reason,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerMessageEvent(event))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerMessageEvent(event));
                    continue;
                }
                dispatch_service_worker_message_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *event,
                    &parent_tx,
                    &script_url,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerNotificationEvent(event))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerNotificationEvent(event));
                    continue;
                }
                dispatch_service_worker_notification_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *event,
                    &parent_tx,
                    &script_url,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerPushEvent(event))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerPushEvent(event));
                    continue;
                }
                dispatch_service_worker_push_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *event,
                    &parent_tx,
                    &script_url,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerSyncEvent(event))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerSyncEvent(event));
                    continue;
                }
                dispatch_service_worker_sync_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *event,
                    &parent_tx,
                    &script_url,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerPeriodicSyncEvent(event))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerPeriodicSyncEvent(event));
                    continue;
                }
                dispatch_service_worker_periodic_sync_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    *event,
                    &parent_tx,
                    &script_url,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerControllerChange)) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerControllerChange);
                    continue;
                }
                dispatch_service_worker_controller_change_event(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                );
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerClientQueryResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerClientQueryResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_client_query_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerClientNavigateResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerClientNavigateResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_client_navigate_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerClientFocusResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerClientFocusResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_client_focus_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerClientsOpenWindowResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerClientsOpenWindowResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_clients_open_window_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerShowNotificationResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerShowNotificationResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_show_notification_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerGetNotificationsResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerGetNotificationsResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_get_notifications_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerSyncRegistrationResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerSyncRegistrationResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_sync_registration_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerSyncGetTagsResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerSyncGetTagsResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_sync_get_tags_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(
                WorkerMessage::ServiceWorkerPeriodicSyncRegistrationResult(result),
            )) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerPeriodicSyncRegistrationResult(result),
                    );
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_periodic_sync_registration_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(
                WorkerMessage::ServiceWorkerPeriodicSyncGetTagsResult(result),
            )) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerPeriodicSyncGetTagsResult(result),
                    );
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_periodic_sync_get_tags_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(
                WorkerMessage::ServiceWorkerPeriodicSyncUnregistrationResult(result),
            )) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerPeriodicSyncUnregistrationResult(result),
                    );
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_periodic_sync_unregistration_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerPushSubscribeResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerPushSubscribeResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_push_subscribe_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(
                WorkerMessage::ServiceWorkerPushGetSubscriptionResult(result),
            )) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages.push_back(
                        WorkerMessage::ServiceWorkerPushGetSubscriptionResult(result),
                    );
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_push_get_subscription_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ServiceWorkerPushUnsubscribeResult(
                result,
            ))) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ServiceWorkerPushUnsubscribeResult(result));
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_service_worker_push_unsubscribe_result(scope, &state, result);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::DispatchPendingPromiseRejections)) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::DispatchPendingPromiseRejections);
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                dispatch_queued_worker_promise_rejections(scope);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::NestedWorkerEvent {
                worker_id,
                message,
            })) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::NestedWorkerEvent { worker_id, message });
                    continue;
                }
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                let result = dispatch_nested_worker_event(scope, &state, worker_id, &message);
                if let Some(error) = result.unhandled_error {
                    let global = ctx.global(scope);
                    let report = V8ExceptionReport {
                        summary: error.message,
                        source: Some(error.filename),
                        line: Some(error.lineno as usize),
                        column: Some(error.colno as usize),
                        source_line: None,
                        stack: None,
                        callback_context: None,
                        exception: None,
                    };
                    if !dispatch_worker_error_event(
                        scope,
                        global,
                        &report,
                        None,
                        &parent_tx,
                        &script_url,
                    ) {
                        report_exception_to_parent(
                            &report,
                            &script_url,
                            error.event_kind,
                            &parent_tx,
                        );
                    }
                }
                if result.dispatched {
                    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(
                        scope,
                    );
                    drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
                }
            }
            WorkerLoopWake::Message(Some(WorkerMessage::RunInspectorTask(mode))) => {
                if let Some(task) = inspector_task_runner.claim_task(mode) {
                    dispatch_worker_inspector_task(
                        &mut worker_isolate,
                        &context,
                        task,
                        &state,
                        &module_graph_fetch_tx,
                    );
                }
                if mode == WorkerInspectorTaskMode::Interrupt {
                    inspector_task_runner.request_interrupt_if_needed();
                }
            }
            #[cfg(test)]
            WorkerLoopWake::Message(Some(WorkerMessage::ResourceOwnerSlotDiagnostics {
                response_tx,
            })) => {
                if pending_module_bootstrap.is_some() {
                    pending_bootstrap_messages
                        .push_back(WorkerMessage::ResourceOwnerSlotDiagnostics { response_tx });
                    continue;
                }
                let result = worker_resource_owner_slot_diagnostics(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                );
                let _ = response_tx.send(result);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::SetExtraHttpHeaders(headers))) => {
                let (loader, headers_for_loader) = {
                    let mut state = state.borrow_mut();
                    state.extra_http_headers = headers;
                    (state.loader.clone(), state.extra_http_headers.clone())
                };
                loader
                    .request_client()
                    .set_extra_http_headers(&headers_for_loader);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::SetNetworkOffline(offline))) => {
                let loader = {
                    let mut state = state.borrow_mut();
                    state.network_offline = offline;
                    state.loader.clone()
                };
                loader.request_client().set_network_offline(offline);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::SetBlockedUrlPatterns(patterns))) => {
                let (loader, patterns_for_loader) = {
                    let mut state = state.borrow_mut();
                    state.blocked_url_patterns = patterns;
                    (state.loader.clone(), state.blocked_url_patterns.clone())
                };
                loader
                    .request_client()
                    .set_blocked_url_patterns(&patterns_for_loader);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::SetFetchSubresourceInterception {
                enabled,
                resource_type,
            })) => {
                let mut state = state.borrow_mut();
                state.fetch_subresource_interception_enabled = enabled;
                state.fetch_subresource_interception_resource_type = resource_type;
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ContinuePendingFetch(request))) => {
                continue_pending_worker_fetch(&state, request);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ContinuePendingXhr(request))) => {
                continue_pending_worker_xhr(&state, request);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ContinuePendingCspReport(request))) => {
                continue_pending_worker_csp_report(&state, request);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ContinuePendingFetchResponse {
                request,
                response_code,
                response_headers,
            })) => {
                continue_pending_worker_fetch_response(
                    &state,
                    request,
                    response_code,
                    response_headers,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::ContinuePendingXhrResponse {
                request,
                response_code,
                response_headers,
            })) => {
                continue_pending_worker_xhr_response(
                    &state,
                    request,
                    response_code,
                    response_headers,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FailPendingFetch {
                request,
                error_text,
            })) => {
                fail_pending_worker_fetch(&state, request, error_text);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FailPendingXhr {
                request,
                error_text,
            })) => {
                fail_pending_worker_xhr(&state, request, error_text);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FailPendingCspReport {
                request,
                error_text,
            })) => {
                fail_pending_worker_csp_report(&state, request, error_text);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FailPendingFetchAuth {
                request,
                error_text,
            })) => {
                fail_pending_worker_fetch_auth(&state, request, error_text);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FailPendingXhrAuth {
                request,
                error_text,
            })) => {
                fail_pending_worker_xhr_auth(&state, request, error_text);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FailPendingFetchResponse {
                request,
                error_text,
            })) => {
                fail_pending_worker_fetch_response(&state, request, error_text);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FailPendingXhrResponse {
                request,
                error_text,
            })) => {
                fail_pending_worker_xhr_response(&state, request, error_text);
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FulfillPendingFetch {
                request,
                response_code,
                response_headers,
                response_body,
            })) => {
                fulfill_pending_worker_fetch(
                    &state,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FulfillPendingXhr {
                request,
                response_code,
                response_headers,
                response_body,
            })) => {
                fulfill_pending_worker_xhr(
                    &state,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FulfillPendingCspReport {
                request,
                response_code,
                response_headers,
                response_body,
            })) => {
                fulfill_pending_worker_csp_report(
                    &state,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FulfillPendingFetchResponse {
                request,
                response_code,
                response_headers,
                response_body,
            })) => {
                fulfill_pending_worker_fetch_response(
                    &state,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::FulfillPendingXhrResponse {
                request,
                response_code,
                response_headers,
                response_body,
            })) => {
                fulfill_pending_worker_xhr_response(
                    &state,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                );
            }
            WorkerLoopWake::Message(Some(WorkerMessage::Terminate)) => {
                trace!(url = %script_url, "worker terminated by parent");
                break;
            }
            WorkerLoopWake::Message(None)
                if nearest_timer.is_none()
                    && !has_pending_indexed_db_tasks
                    && pending_module_bootstrap.is_none()
                    && !has_pending_module_runtime_activity
                    && !worker_has_pending_async(&state) =>
            {
                trace!(url = %script_url, "worker channel closed");
                break;
            }
            WorkerLoopWake::Message(None) | WorkerLoopWake::Timer => {
                // No message, but we might have timers or pending async work to process.
            }
            WorkerLoopWake::ModuleRuntime => {
                drain_worker_dynamic_module_imports_for_context(
                    worker_isolate.worker_isolate_mut(),
                    &context,
                    &state,
                    &module_graph_fetch_tx,
                );
            }
            WorkerLoopWake::IndexedDb => {
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                flush_one_worker_indexed_db_task(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::RuntimeWake(Some(())) => {
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                if crate::context_bootstrap::indexed_db_has_pending_tasks(scope) {
                    flush_one_worker_indexed_db_task(scope, &state, &module_graph_fetch_tx);
                } else {
                    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(
                        scope,
                    );
                    drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
                }
            }
            WorkerLoopWake::RuntimeWake(None) => {
                // The platform registration owns the sender until isolate
                // cleanup; if it is gone, other wake sources can still drive
                // shutdown and pending work.
            }
            WorkerLoopWake::Fetch(Some(completion)) => {
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_worker_fetch_completion(scope, &state, completion);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Fetch(None)
                if nearest_timer.is_none()
                    && !has_pending_indexed_db_tasks
                    && pending_module_bootstrap.is_none()
                    && !has_pending_module_runtime_activity
                    && !worker_has_pending_async(&state) =>
            {
                trace!(url = %script_url, "worker fetch channel closed");
                break;
            }
            WorkerLoopWake::Fetch(None) => {
                // Fetch sender dropped, but other work may still be pending.
            }
            WorkerLoopWake::Xhr(Some(completion)) => {
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_worker_xhr_completion(scope, &state, completion);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Xhr(None)
                if nearest_timer.is_none()
                    && !has_pending_indexed_db_tasks
                    && pending_module_bootstrap.is_none()
                    && !has_pending_module_runtime_activity
                    && !worker_has_pending_async(&state) =>
            {
                trace!(url = %script_url, "worker xhr channel closed");
                break;
            }
            WorkerLoopWake::Xhr(None) => {
                // XHR sender dropped, but other work may still be pending.
            }
            WorkerLoopWake::WebCrypto(Some(completion)) => {
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_worker_webcrypto_completion(scope, &state, completion);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::WebCrypto(None)
                if nearest_timer.is_none()
                    && !has_pending_indexed_db_tasks
                    && pending_module_bootstrap.is_none()
                    && !has_pending_module_runtime_activity
                    && !worker_has_pending_async(&state) =>
            {
                trace!(url = %script_url, "worker webcrypto channel closed");
                break;
            }
            WorkerLoopWake::WebCrypto(None) => {
                // WebCrypto sender dropped, but other work may still be pending.
            }
            WorkerLoopWake::Opfs(Some(completion)) => {
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                drain_worker_opfs_completion(scope, &state, completion);
                perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
            }
            WorkerLoopWake::Opfs(None)
                if nearest_timer.is_none()
                    && !has_pending_indexed_db_tasks
                    && pending_module_bootstrap.is_none()
                    && !has_pending_module_runtime_activity
                    && !worker_has_pending_async(&state) =>
            {
                trace!(url = %script_url, "worker OPFS channel closed");
                break;
            }
            WorkerLoopWake::Opfs(None) => {
                // OPFS sender dropped, but other work may still be pending.
            }
            WorkerLoopWake::ModuleGraphFetch(Some(completion)) => {
                let (isolate, runtime_inspector) =
                    worker_isolate.worker_isolate_and_runtime_inspector();
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                let global = ctx.global(scope);
                if worker_dynamic_module_import_waits_for_fetch(scope, completion.fetch_id()) {
                    if let Some(violation) = completion.csp_report_only_violation() {
                        let loader = state.borrow().loader.clone();
                        dispatch_worker_csp_violation_event(scope, &loader, violation);
                    }
                    if let Some(violation) = completion.csp_violation() {
                        let loader = state.borrow().loader.clone();
                        dispatch_worker_csp_violation_event(scope, &loader, violation);
                    }
                    if let Some(advance) = resume_worker_dynamic_module_fetch(scope, *completion) {
                        handle_worker_dynamic_module_import_advance(
                            advance,
                            &state,
                            &module_graph_fetch_tx,
                        );
                        drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
                    }
                } else if pending_module_bootstrap.is_some() {
                    if let Some(violation) = completion.csp_report_only_violation() {
                        let loader = state.borrow().loader.clone();
                        dispatch_worker_csp_violation_event(scope, &loader, violation);
                    }
                    if let Some(violation) = completion.csp_violation() {
                        let loader = state.borrow().loader.clone();
                        dispatch_worker_csp_violation_event(scope, &loader, violation);
                    }
                    let bootstrap_fetch_matches = pending_module_bootstrap
                        .as_ref()
                        .and_then(|bootstrap| bootstrap.pending_requests())
                        .is_some_and(|requests| requests.contains_fetch_id(completion.fetch_id()));
                    let module_script_resource = if bootstrap_fetch_matches {
                        completion
                            .result()
                            .ok()
                            .and_then(|source| source.resource().cloned())
                    } else {
                        None
                    };
                    let resume = pending_module_bootstrap
                        .as_mut()
                        .expect("checked pending module bootstrap")
                        .resume(scope, *completion);
                    if let Some(resource) = module_script_resource {
                        report_service_worker_module_script_resource(&state, resource);
                    }
                    match resume {
                        WorkerModuleBootstrapResume::Complete => {
                            pending_module_bootstrap = None;
                            bootstrap_completion
                                .mark_success(worker_bootstrap_success(scope, global, &state));
                            perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                            drain_worker_dynamic_module_imports(
                                scope,
                                &state,
                                &module_graph_fetch_tx,
                            );
                            forward_worker_script_loaded(&runtime_inspector, &parent_tx);
                        }
                        WorkerModuleBootstrapResume::NeedFetches(requests) => {
                            start_worker_module_graph_fetch_batch(
                                requests,
                                &state,
                                &module_graph_fetch_tx,
                            );
                        }
                        WorkerModuleBootstrapResume::WaitingFetches => {}
                        WorkerModuleBootstrapResume::WaitingEvaluation => {}
                        WorkerModuleBootstrapResume::Failed(error) => {
                            let (report, exception, parent_event_kind) = *error;
                            let exception =
                                exception.as_ref().map(|value| v8::Local::new(scope, value));
                            let global = ctx.global(scope);
                            bootstrap_completion.mark_failure(
                                &report,
                                &script_url,
                                parent_event_kind,
                                WorkerErrorPhase::Bootstrap,
                                WorkerErrorSource::Runtime,
                            );
                            dispatch_worker_exception_with_phase(
                                scope,
                                global,
                                report,
                                exception,
                                parent_event_kind,
                                WorkerErrorPhase::Bootstrap,
                                &parent_tx,
                                &script_url,
                            );
                            forward_worker_script_loaded(&runtime_inspector, &parent_tx);
                            break;
                        }
                    }
                } else {
                    let result_summary = completion
                        .result()
                        .map(|source| {
                            format!(
                                "ok final_url={} bytes={}",
                                source.final_url(),
                                source.source().len()
                            )
                        })
                        .unwrap_or_else(|error| format!("error {error}"));
                    trace!(
                        url = %script_url,
                        fetch_id = completion.fetch_id(),
                        result = result_summary.as_str(),
                        "worker module graph fetch completion received without active job"
                    );
                }
            }
            WorkerLoopWake::ModuleGraphFetch(None) => {
                // The worker thread owns the sender; None can only happen during teardown.
            }
            WorkerLoopWake::ModuleEvaluation(Some(completion)) => {
                let (isolate, runtime_inspector) =
                    worker_isolate.worker_isolate_and_runtime_inspector();
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                let global = ctx.global(scope);
                if resume_worker_dynamic_module_evaluation(scope, &completion) {
                    drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
                } else if pending_module_bootstrap.is_some() {
                    let resume = pending_module_bootstrap
                        .as_mut()
                        .expect("checked pending module bootstrap")
                        .resume_evaluation(scope, completion);
                    match resume {
                        WorkerModuleBootstrapResume::Complete => {
                            pending_module_bootstrap = None;
                            bootstrap_completion
                                .mark_success(worker_bootstrap_success(scope, global, &state));
                            perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(scope);
                            drain_worker_dynamic_module_imports(
                                scope,
                                &state,
                                &module_graph_fetch_tx,
                            );
                            forward_worker_script_loaded(&runtime_inspector, &parent_tx);
                        }
                        WorkerModuleBootstrapResume::NeedFetches(requests) => {
                            start_worker_module_graph_fetch_batch(
                                requests,
                                &state,
                                &module_graph_fetch_tx,
                            );
                        }
                        WorkerModuleBootstrapResume::WaitingFetches => {}
                        WorkerModuleBootstrapResume::WaitingEvaluation => {}
                        WorkerModuleBootstrapResume::Failed(error) => {
                            let (report, exception, parent_event_kind) = *error;
                            let exception =
                                exception.as_ref().map(|value| v8::Local::new(scope, value));
                            let global = ctx.global(scope);
                            bootstrap_completion.mark_failure(
                                &report,
                                &script_url,
                                parent_event_kind,
                                WorkerErrorPhase::Bootstrap,
                                WorkerErrorSource::Runtime,
                            );
                            dispatch_worker_exception_with_phase(
                                scope,
                                global,
                                report,
                                exception,
                                parent_event_kind,
                                WorkerErrorPhase::Bootstrap,
                                &parent_tx,
                                &script_url,
                            );
                            forward_worker_script_loaded(&runtime_inspector, &parent_tx);
                            break;
                        }
                    }
                } else {
                    trace!(
                        url = %script_url,
                        evaluation_id = completion.evaluation_id(),
                        result = ?completion.result(),
                        "worker module evaluation completion received without active job"
                    );
                }
            }
            WorkerLoopWake::ModuleEvaluation(None) => {
                // The worker thread owns the sender; None can only happen during teardown.
            }
            WorkerLoopWake::WebSocket(Some(event)) => {
                let scope = pin!(v8::HandleScope::new(worker_isolate.worker_isolate_mut()));
                let scope = &mut scope.init();
                let ctx = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, ctx);
                if dispatch_worker_websocket_event(scope, &state, event) {
                    perform_worker_microtask_checkpoint_and_report_pending_promise_rejections(
                        scope,
                    );
                    drain_worker_dynamic_module_imports(scope, &state, &module_graph_fetch_tx);
                }
            }
            WorkerLoopWake::WebSocket(None) => {
                // The worker state owns the sender; None can only happen during teardown.
            }
        }

        // Fire any timers that are ready.
        let now = tokio::time::Instant::now();
        let mut fired_ids: Vec<usize> = Vec::new();
        for (i, timer) in active_timers.iter().enumerate() {
            if now >= timer.next_fire {
                fired_ids.push(i);
            }
        }
        fired_ids.sort_by_key(|&idx| (active_timers[idx].next_fire, active_timers[idx].id));

        // Fire ready callbacks by deadline. Under load, multiple worker timers
        // can become ready before this loop wakes.
        for &idx in &fired_ids {
            let timer = &active_timers[idx];
            fire_timer_callback(
                worker_isolate.worker_isolate_mut(),
                timer,
                &parent_tx,
                &script_url,
            );
        }
        if !fired_ids.is_empty() {
            drain_worker_dynamic_module_imports_for_context(
                worker_isolate.worker_isolate_mut(),
                &context,
                &state,
                &module_graph_fetch_tx,
            );
        }

        // Update or remove fired timers.
        let mut to_remove = Vec::new();
        for &idx in fired_ids.iter() {
            if active_timers[idx].is_interval {
                active_timers[idx].next_fire = now + active_timers[idx].delay;
            } else {
                to_remove.push(idx);
            }
        }
        to_remove.sort_unstable();
        for idx in to_remove.into_iter().rev() {
            active_timers.remove(idx);
        }

        forward_pending_worker_runtime_protocol_messages(
            worker_isolate.worker_runtime_inspector(),
            &parent_tx,
        );

        if state.borrow().closed {
            trace!(url = %script_url, "worker closed via self.close()");
            break;
        }
    }

    // ── Cleanup ────────────────────────────────────────────────────────────
    // Stop admitting inside-settings work before any V8-owned completion
    // routes are destroyed. Ordinary transports are cancelled here;
    // explicitly keepalive loads are reduced to browser-runtime network-only
    // records and therefore cannot retain this WorkerGlobalScope.
    inspector_task_runner.dispose("Worker exited before Inspector task dispatch");
    let resource_loader = state.borrow().loader.clone();
    resource_loader.begin_detach();
    if matches!(state.borrow().global_kind, WorkerGlobalKind::Shared { .. }) {
        let _ = parent_tx.send(WorkerToParentMessage::SharedWorkerClosed);
    }
    {
        let (isolate, runtime_inspector) = worker_isolate.worker_isolate_and_runtime_inspector();
        let scope = pin!(v8::HandleScope::new(isolate));
        let scope = &mut scope.init();
        let ctx = v8::Local::new(scope, &context);
        runtime_inspector.context_destroyed(ctx);
    }
    state
        .borrow_mut()
        .v8_finalizers
        .clear_for_context_teardown();
    close_worker_owned_broadcast_channels(&state);
    close_worker_owned_message_ports(&state);
    resource_loader.finish_detach();
    crate::blob::cleanup_owner_resources(resource_owner_id);
    *isolate_handle.lock() = None;
    drop(active_timers);
    drop(context);
    drop(state);
    drop(service_worker_worker_client);
    worker_isolate.unregister_worker_isolate_platform();
    debug!(url = %script_url, "worker exited");
}

struct RegisteredServiceWorkerWorkerClient {
    runtime: ServiceWorkerRuntimeService,
    client_id: ServiceWorkerClientId,
}

impl Drop for RegisteredServiceWorkerWorkerClient {
    fn drop(&mut self) {
        self.runtime.unregister_client(self.client_id);
    }
}

fn register_service_worker_worker_client(
    runtime: Option<ServiceWorkerRuntimeService>,
    reserved_client_id: Option<ServiceWorkerClientId>,
    script_url: Option<&url::Url>,
    storage_key: &MoliStorageKey,
    global_kind: &WorkerGlobalKind,
    secure_context: bool,
    worker_tx: mpsc::UnboundedSender<WorkerMessage>,
) -> Option<RegisteredServiceWorkerWorkerClient> {
    let runtime = runtime?;
    if let Some(client_id) = reserved_client_id {
        if runtime.activate_reserved_worker_client(client_id, worker_tx) {
            return Some(RegisteredServiceWorkerWorkerClient { runtime, client_id });
        }
        runtime.unregister_client(client_id);
        return None;
    }
    let script_url = script_url?;
    if !matches!(script_url.scheme(), "http" | "https") {
        return None;
    }
    let client_type = match global_kind {
        WorkerGlobalKind::Dedicated { .. } => ServiceWorkerClientType::DedicatedWorker,
        WorkerGlobalKind::Shared { .. } => ServiceWorkerClientType::SharedWorker,
        WorkerGlobalKind::Service { .. } => return None,
    };
    let client_id = runtime.register_worker_client_with_storage_key(
        script_url.clone(),
        storage_key.serialized_storage_key(),
        client_type,
        secure_context,
        worker_tx,
    );
    Some(RegisteredServiceWorkerWorkerClient { runtime, client_id })
}

fn evaluate_worker_bootstrap_script(
    scope: &mut v8::PinScope<'_, '_>,
    script_source: &WorkerScriptSource,
    script_url: &str,
    script_kind: WorkerScriptKind,
    module_static_import_initiator_url: Option<url::Url>,
    module_credentials_mode: RequestCredentialsMode,
    referrer_policy: Option<String>,
    module_evaluation_tx: mpsc::UnboundedSender<WorkerModuleEvaluationCompletion>,
) -> WorkerBootstrapStart {
    match script_kind {
        WorkerScriptKind::Classic => match script_source.text_source() {
            Some(script_source) => {
                match evaluate_classic_worker_bootstrap_script(scope, script_source, script_url) {
                    Some((error, phase)) => WorkerBootstrapStart::Failed {
                        error: Box::new(error),
                        phase,
                    },
                    None => WorkerBootstrapStart::Complete,
                }
            }
            None => WorkerBootstrapStart::Failed {
                error: Box::new(worker_bootstrap_error(
                    scope,
                    script_url,
                    "Classic worker script source is not text",
                    WorkerParentErrorEventKind::Event,
                )),
                phase: WorkerErrorPhase::Bootstrap,
            },
        },
        WorkerScriptKind::Module => evaluate_module_worker_bootstrap_source(
            scope,
            script_source.module_source(),
            script_url,
            module_static_import_initiator_url,
            module_credentials_mode,
            referrer_policy,
            module_evaluation_tx,
        )
        .into(),
    }
}

fn worker_bootstrap_success<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    state: &Rc<RefCell<WorkerGlobalState>>,
) -> WorkerBootstrapSuccess {
    let service_worker_fetch_handler_type =
        if matches!(state.borrow().global_kind, WorkerGlobalKind::Service { .. }) {
            service_worker_fetch_handler_type(scope, global)
        } else {
            WorkerFetchHandlerType::NoHandler
        };
    WorkerBootstrapSuccess {
        service_worker_fetch_handler_type,
    }
}

fn worker_bootstrap_error(
    scope: &mut v8::PinScope<'_, '_>,
    script_url: &str,
    summary: &str,
    event_kind: WorkerParentErrorEventKind,
) -> WorkerBootstrapError {
    let exception =
        v8::String::new(scope, summary).map(|message| v8::Exception::syntax_error(scope, message));
    (
        V8ExceptionReport {
            summary: summary.to_owned(),
            source: Some(script_url.to_owned()),
            line: Some(1),
            column: Some(1),
            source_line: None,
            stack: None,
            callback_context: None,
            exception: None,
        },
        exception.map(|value| v8::Global::new(scope, value)),
        event_kind,
    )
}

enum WorkerBootstrapStart {
    Complete,
    Pending(Box<WorkerModulePendingBootstrap>),
    Failed {
        error: Box<WorkerBootstrapError>,
        phase: WorkerErrorPhase,
    },
}

struct WorkerBootstrapCompletionReporter {
    sender: Option<mpsc::UnboundedSender<WorkerBootstrapCompletion>>,
}

impl WorkerBootstrapCompletionReporter {
    fn new(sender: Option<mpsc::UnboundedSender<WorkerBootstrapCompletion>>) -> Self {
        Self { sender }
    }

    fn mark_success(&mut self, success: WorkerBootstrapSuccess) {
        self.send(WorkerBootstrapCompletion::success(success));
    }

    fn mark_failure(
        &mut self,
        report: &V8ExceptionReport,
        script_url: &str,
        event_kind: WorkerParentErrorEventKind,
        phase: WorkerErrorPhase,
        source: WorkerErrorSource,
    ) {
        self.send(WorkerBootstrapCompletion::failure(
            WorkerBootstrapFailure::from_exception_report(
                report, script_url, event_kind, phase, source,
            ),
        ));
    }

    fn mark_install_global_failure(&mut self, script_url: &str, message: String) {
        let failure = WorkerBootstrapFailure {
            message,
            filename: script_url.to_owned(),
            lineno: 0,
            colno: 0,
            event_kind: WorkerParentErrorEventKind::Event,
            phase: WorkerErrorPhase::Bootstrap,
            source: WorkerErrorSource::Runtime,
        };
        self.send(WorkerBootstrapCompletion::failure(failure));
    }

    fn send(&mut self, completion: WorkerBootstrapCompletion) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(completion);
        }
    }
}

impl From<WorkerModuleBootstrapStart> for WorkerBootstrapStart {
    fn from(start: WorkerModuleBootstrapStart) -> Self {
        match start {
            WorkerModuleBootstrapStart::Complete => Self::Complete,
            WorkerModuleBootstrapStart::Pending(pending) => Self::Pending(pending),
            WorkerModuleBootstrapStart::Failed(error) => Self::Failed {
                error,
                phase: WorkerErrorPhase::Bootstrap,
            },
        }
    }
}

fn evaluate_classic_worker_bootstrap_script(
    scope: &mut v8::PinScope<'_, '_>,
    script_source: &str,
    script_url: &str,
) -> Option<(
    (
        V8ExceptionReport,
        Option<v8::Global<v8::Value>>,
        WorkerParentErrorEventKind,
    ),
    WorkerErrorPhase,
)> {
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let source_str = v8::String::new(&scope, script_source).expect("v8 string allocation");
    let origin = create_script_origin(&mut scope, script_url);
    let compiled = v8::Script::compile(&scope, source_str, Some(&origin));
    match compiled {
        Some(script) => {
            let _result = script.run(&scope);
            if scope.has_caught() {
                let exception = scope.exception();
                let message = scope.message();
                let stack_trace = scope.stack_trace();
                let report = build_event_handler_exception_report(
                    &mut scope,
                    exception,
                    message,
                    stack_trace,
                );
                Some((
                    (
                        report,
                        exception.map(|value| v8::Global::new(&scope, value)),
                        WorkerParentErrorEventKind::ErrorEvent,
                    ),
                    WorkerErrorPhase::Runtime,
                ))
            } else {
                None
            }
        }
        None => {
            let exception = scope.exception();
            let message = scope.message();
            let stack_trace = scope.stack_trace();
            let report =
                build_event_handler_exception_report(&mut scope, exception, message, stack_trace);
            Some((
                (
                    report,
                    exception.map(|value| v8::Global::new(&scope, value)),
                    WorkerParentErrorEventKind::Event,
                ),
                WorkerErrorPhase::Bootstrap,
            ))
        }
    }
}

#[cfg(test)]
mod tests;
