use moli_fetch::{FetchCancelHandle, Request, RequestCredentialsMode, ScriptFetchRequestMetadata};
use moli_shared_worker::{
    SharedWorkerClientOwnerId, SharedWorkerCredentialsMode, SharedWorkerDescriptor,
    SharedWorkerKey, SharedWorkerSameSiteCookies, SharedWorkerScriptType,
};
use url::Url;

use crate::{
    content_security_policy::{
        ContentSecurityPolicyReportingEndpoints, ContentSecurityPolicyResourceKind,
        ensure_content_security_policy_allows_url,
    },
    context_bootstrap::{SharedStorageBucketStore, WeakIndexedDbManager},
    network::ResourceRequestClient,
    page_task_queue::{
        RendererPageSharedWorkerClientEventRealmSender, RendererWorkerHostBridgeEventSender,
    },
    protocol_types::NavigationResponse,
    referrer_policy::response_referrer_policy_from_headers,
    runtime::RendererWorkerContextRuntime,
    service_worker_runtime::{
        ServiceWorkerClientId, ServiceWorkerClientType, ServiceWorkerRequestDestination,
        ServiceWorkerRuntimeService,
    },
    types::{MessagePortId, SubresourcePolicyContext},
    worker::{WorkerNetworkPolicy, WorkerScriptKind},
};

#[derive(Clone, Debug)]
pub(crate) struct SharedWorkerScriptLoad {
    kind: SharedWorkerScriptLoadKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SharedWorkerLoadedScript {
    pub(super) script_url: String,
    pub(super) source: String,
    pub(super) response_referrer_policy: Option<String>,
    pub(super) response_policy_context: Option<SubresourcePolicyContext>,
    pub(super) response_content_security_policies: Vec<String>,
    pub(super) response_content_security_report_only_policies: Vec<String>,
    pub(super) response_content_security_reporting_endpoints:
        ContentSecurityPolicyReportingEndpoints,
}

#[derive(Clone, Debug)]
pub(super) enum SharedWorkerScriptLoadKind {
    Ready(SharedWorkerLoadedScript),
    Blob { script_url: Url },
    Fetch(Box<SharedWorkerScriptFetch>),
    Failure { message: String },
}

#[derive(Clone, Debug)]
pub(super) struct SharedWorkerScriptFetch {
    pub(super) request_client: ResourceRequestClient,
    pub(super) task_runner: crate::network::RendererResourceTaskRunner,
    pub(super) script_url: Url,
    pub(super) initiator_url: Url,
    pub(super) request_policy: SharedWorkerScriptRequestPolicy,
}

#[derive(Clone)]
pub(crate) struct SharedWorkerLaunchParams {
    pub(super) key: SharedWorkerKey,
    pub(super) script_load: SharedWorkerScriptLoad,
    pub(super) launch_context: SharedWorkerLaunchContext,
    pub(super) client_port_id: MessagePortId,
    pub(super) worker_port_id: MessagePortId,
    pub(super) client_owner_id: SharedWorkerClientOwnerId,
    pub(super) client_event_realm: RendererPageSharedWorkerClientEventRealmSender,
    pub(super) worker_host_bridge_sender: RendererWorkerHostBridgeEventSender,
    pub(super) parent_service_worker_client_id: Option<ServiceWorkerClientId>,
    pub(super) reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
}

#[derive(Clone)]
pub(crate) struct SharedWorkerLaunchContext {
    pub(super) name: String,
    pub(super) request_client: ResourceRequestClient,
    pub(super) execution_policy: SharedWorkerExecutionPolicy,
}

#[derive(Clone)]
pub(crate) struct SharedWorkerExecutionPolicy {
    pub(super) script_kind: WorkerScriptKind,
    pub(super) module_static_import_initiator_url: Url,
    pub(super) module_static_import_content_security_policies: Vec<String>,
    pub(super) network_policy: WorkerNetworkPolicy,
    pub(super) policy_context: SubresourcePolicyContext,
    pub(super) worker_context_runtime: RendererWorkerContextRuntime,
    pub(super) service_worker_runtime: Option<ServiceWorkerRuntimeService>,
    pub(super) indexed_db_manager: Option<WeakIndexedDbManager>,
    pub(super) storage_bucket_store: Option<SharedStorageBucketStore>,
    pub(super) storage_key_top_level_site: Option<String>,
    pub(super) module_credentials_mode: RequestCredentialsMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SharedWorkerScriptRequestPolicy {
    pub(super) credentials_mode: RequestCredentialsMode,
    pub(super) same_site_cookies: SharedWorkerSameSiteCookies,
    pub(super) document_referrer_policy: Option<String>,
    pub(super) network_partition_key: Option<String>,
    pub(super) document_content_security_policies: Vec<String>,
}

impl SharedWorkerLaunchParams {
    pub(crate) fn new(
        key: SharedWorkerKey,
        script_load: SharedWorkerScriptLoad,
        launch_context: SharedWorkerLaunchContext,
        client_port_id: MessagePortId,
        worker_port_id: MessagePortId,
        client_owner_id: SharedWorkerClientOwnerId,
        parent_service_worker_client_id: Option<ServiceWorkerClientId>,
        client_event_realm: RendererPageSharedWorkerClientEventRealmSender,
        worker_host_bridge_sender: RendererWorkerHostBridgeEventSender,
    ) -> Self {
        Self {
            key,
            script_load,
            launch_context,
            client_port_id,
            worker_port_id,
            client_owner_id,
            client_event_realm,
            worker_host_bridge_sender,
            parent_service_worker_client_id,
            reserved_service_worker_client_id: None,
        }
    }

    pub(super) fn reserve_service_worker_worker_client_for_main_script(&mut self) {
        if self.reserved_service_worker_client_id.is_some()
            || !self.script_load.can_reserve_service_worker_worker_client()
        {
            return;
        }
        let Some(service_worker_runtime) = self
            .launch_context
            .execution_policy
            .service_worker_runtime
            .as_ref()
        else {
            return;
        };
        let Ok(script_url) = Url::parse(self.key.script_url()) else {
            return;
        };
        let secure_context = self
            .launch_context
            .execution_policy
            .network_policy
            .secure_context;
        let storage_key = self.key.storage_key().serialized_storage_key();
        let client_id = if matches!(script_url.scheme(), "http" | "https") {
            Some(
                service_worker_runtime.register_reserved_worker_client_with_storage_key(
                    script_url,
                    storage_key,
                    ServiceWorkerClientType::SharedWorker,
                    secure_context,
                ),
            )
        } else if script_url.scheme() == "blob" {
            let Some(parent_client_id) = self.parent_service_worker_client_id else {
                return;
            };
            service_worker_runtime
                .register_reserved_worker_client_inheriting_controller_from_client(
                    script_url,
                    storage_key,
                    ServiceWorkerClientType::SharedWorker,
                    secure_context,
                    parent_client_id,
                )
        } else {
            None
        };
        let Some(client_id) = client_id else {
            return;
        };
        self.reserved_service_worker_client_id = Some(client_id);
    }

    pub(super) fn unregister_reserved_service_worker_client(&mut self) {
        let Some(client_id) = self.reserved_service_worker_client_id.take() else {
            return;
        };
        if let Some(service_worker_runtime) = self
            .launch_context
            .execution_policy
            .service_worker_runtime
            .as_ref()
        {
            service_worker_runtime.unregister_client(client_id);
        }
    }
}

impl SharedWorkerLaunchContext {
    pub(crate) fn new(
        name: String,
        request_client: ResourceRequestClient,
        execution_policy: SharedWorkerExecutionPolicy,
    ) -> Self {
        Self {
            name,
            request_client,
            execution_policy,
        }
    }
}

impl SharedWorkerExecutionPolicy {
    pub(crate) fn new(
        script_kind: WorkerScriptKind,
        module_static_import_initiator_url: Url,
        module_static_import_content_security_policies: Vec<String>,
        network_policy: WorkerNetworkPolicy,
        policy_context: SubresourcePolicyContext,
        worker_context_runtime: RendererWorkerContextRuntime,
        storage_key_top_level_site: Option<String>,
        module_credentials_mode: RequestCredentialsMode,
    ) -> Self {
        Self {
            script_kind,
            module_static_import_initiator_url,
            module_static_import_content_security_policies,
            network_policy,
            policy_context,
            worker_context_runtime,
            service_worker_runtime: None,
            indexed_db_manager: None,
            storage_bucket_store: None,
            storage_key_top_level_site,
            module_credentials_mode,
        }
    }

    pub(crate) fn with_service_worker_runtime(
        mut self,
        runtime: ServiceWorkerRuntimeService,
    ) -> Self {
        self.service_worker_runtime = Some(runtime);
        self
    }

    pub(crate) fn with_storage_bucket_store(mut self, store: SharedStorageBucketStore) -> Self {
        self.storage_bucket_store = Some(store);
        self
    }

    pub(crate) fn with_indexed_db_manager(mut self, manager: Option<WeakIndexedDbManager>) -> Self {
        self.indexed_db_manager = manager;
        self
    }
}

impl SharedWorkerScriptRequestPolicy {
    pub(crate) fn from_descriptor(
        descriptor: &SharedWorkerDescriptor,
        same_site_cookies: SharedWorkerSameSiteCookies,
        document_referrer_policy: Option<String>,
        network_partition_key: Option<String>,
        document_content_security_policies: Vec<String>,
    ) -> Self {
        Self {
            credentials_mode: shared_worker_script_credentials_mode(descriptor),
            same_site_cookies,
            document_referrer_policy,
            network_partition_key,
            document_content_security_policies,
        }
    }

    pub(crate) fn credentials_mode(&self) -> RequestCredentialsMode {
        self.credentials_mode
    }

    pub(crate) fn ensure_allows_script_url(
        &self,
        document_url: &Url,
        script_url: &Url,
    ) -> Result<(), String> {
        ensure_content_security_policy_allows_url(
            &self.document_content_security_policies,
            document_url,
            script_url,
            ContentSecurityPolicyResourceKind::WorkerConstructor,
            || {
                format!(
                    "Failed to load shared worker script `{script_url}`: blocked by Content Security Policy."
                )
            },
        )
    }
}

impl SharedWorkerLoadedScript {
    pub(super) fn new(script_url: String, source: String) -> Self {
        Self {
            script_url,
            source,
            response_referrer_policy: None,
            response_policy_context: None,
            response_content_security_policies: Vec::new(),
            response_content_security_report_only_policies: Vec::new(),
            response_content_security_reporting_endpoints:
                ContentSecurityPolicyReportingEndpoints::default(),
        }
    }

    pub(super) fn with_response_referrer_policy(
        mut self,
        response_referrer_policy: Option<String>,
    ) -> Self {
        self.response_referrer_policy = response_referrer_policy;
        self
    }

    pub(super) fn with_response_policy_context(
        mut self,
        response_policy_context: SubresourcePolicyContext,
    ) -> Self {
        self.response_policy_context = Some(response_policy_context);
        self
    }

    pub(super) fn with_response_content_security_policies(
        mut self,
        response_content_security_policies: Vec<String>,
    ) -> Self {
        self.response_content_security_policies = response_content_security_policies;
        self
    }

    pub(super) fn with_response_content_security_report_only_policies(
        mut self,
        response_content_security_report_only_policies: Vec<String>,
    ) -> Self {
        self.response_content_security_report_only_policies =
            response_content_security_report_only_policies;
        self
    }

    pub(super) fn with_response_content_security_reporting_endpoints(
        mut self,
        response_content_security_reporting_endpoints: ContentSecurityPolicyReportingEndpoints,
    ) -> Self {
        self.response_content_security_reporting_endpoints =
            response_content_security_reporting_endpoints;
        self
    }
}

impl SharedWorkerScriptLoad {
    pub(crate) fn ready(script_url: String, script_source: String) -> Self {
        Self {
            kind: SharedWorkerScriptLoadKind::Ready(SharedWorkerLoadedScript::new(
                script_url,
                script_source,
            )),
        }
    }

    pub(crate) fn blob(script_url: Url) -> Self {
        Self {
            kind: SharedWorkerScriptLoadKind::Blob { script_url },
        }
    }

    pub(crate) fn fetch(
        request_client: ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
        script_url: Url,
        initiator_url: Url,
        request_policy: SharedWorkerScriptRequestPolicy,
    ) -> Self {
        Self {
            kind: SharedWorkerScriptLoadKind::Fetch(Box::new(SharedWorkerScriptFetch {
                request_client,
                task_runner,
                script_url,
                initiator_url,
                request_policy,
            })),
        }
    }

    pub(crate) fn failure(message: impl Into<String>) -> Self {
        Self {
            kind: SharedWorkerScriptLoadKind::Failure {
                message: message.into(),
            },
        }
    }

    pub(super) fn into_kind(self) -> SharedWorkerScriptLoadKind {
        self.kind
    }

    fn can_reserve_service_worker_worker_client(&self) -> bool {
        matches!(
            self.kind,
            SharedWorkerScriptLoadKind::Blob { .. } | SharedWorkerScriptLoadKind::Fetch(_)
        )
    }
}

pub(super) async fn fetch_shared_worker_script_source_async(
    request_client: &ResourceRequestClient,
    resource_task_runner: crate::network::RendererResourceTaskRunner,
    script_url: &Url,
    initiator_url: &Url,
    request_policy: SharedWorkerScriptRequestPolicy,
    service_worker_runtime: Option<ServiceWorkerRuntimeService>,
    reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
    cancel_handle: FetchCancelHandle,
) -> Result<SharedWorkerLoadedScript, String> {
    if cancel_handle.is_cancelled() {
        return Err("SharedWorker script load canceled.".to_owned());
    }
    let mut request_url = script_url.clone();
    request_url.set_fragment(None);
    let request = shared_worker_script_request(&request_url, initiator_url, request_policy)?;
    if let (Some(service_worker_runtime), Some(client_id)) = (
        service_worker_runtime.as_ref(),
        reserved_service_worker_client_id,
    ) && let Some(response) = service_worker_runtime
        .fetch_main_resource_for_worker_client(
            client_id,
            &request,
            request_client,
            resource_task_runner,
            ServiceWorkerRequestDestination::SharedWorker,
            cancel_handle.clone(),
        )
        .await?
    {
        return loaded_shared_worker_script_from_navigation_response(
            response,
            initiator_url,
            script_url,
        );
    }
    if cancel_handle.is_cancelled() {
        return Err("SharedWorker script load canceled.".to_owned());
    }
    let response = request_client
        .fetch_text_stream_with_cancel(request, cancel_handle)
        .await
        .map_err(|error| format!("Failed to load shared worker script `{request_url}`: {error}"))?;
    loaded_shared_worker_script_from_fetch_response(response, initiator_url, script_url)
}

fn loaded_shared_worker_script_from_fetch_response(
    response: moli_fetch::Response,
    initiator_url: &Url,
    script_url: &Url,
) -> Result<SharedWorkerLoadedScript, String> {
    crate::worker::ensure_worker_script_redirect_chain_same_origin(
        initiator_url,
        &response.redirect_chain,
        &response.final_url,
    )
    .map_err(|message| format!("Failed to load shared worker script `{script_url}`: {message}"))?;
    moli_fetch::ensure_http_status_success(response.final_url.as_str(), response.status, false)
        .map_err(|error| error.to_string())?;
    crate::worker::ensure_worker_script_mime_acceptable(
        &response.final_url,
        &response.headers,
        response.body_bytes(),
    )?;
    let response_referrer_policy = response_referrer_policy(&response.headers);
    let response_policy_context =
        worker_policy_context_from_response(&response.final_url, &response.headers);
    let response_content_security_policies =
        crate::content_security_policy::content_security_policy_headers(&response.headers);
    let response_content_security_report_only_policies =
        crate::content_security_policy::content_security_policy_report_only_headers(
            &response.headers,
        );
    let response_content_security_reporting_endpoints =
        crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
            &response.headers,
            &response.final_url,
        );
    let (head, body) = response.into_text_parts();
    let mut final_url = head.final_url;
    final_url.set_fragment(script_url.fragment());
    Ok(SharedWorkerLoadedScript::new(final_url.to_string(), body)
        .with_response_referrer_policy(response_referrer_policy)
        .with_response_policy_context(response_policy_context)
        .with_response_content_security_policies(response_content_security_policies)
        .with_response_content_security_report_only_policies(
            response_content_security_report_only_policies,
        )
        .with_response_content_security_reporting_endpoints(
            response_content_security_reporting_endpoints,
        ))
}

fn loaded_shared_worker_script_from_navigation_response(
    response: NavigationResponse,
    initiator_url: &Url,
    script_url: &Url,
) -> Result<SharedWorkerLoadedScript, String> {
    let (head, body, body_bytes) = response.into_parts();
    crate::worker::ensure_worker_script_redirect_chain_same_origin(
        initiator_url,
        &head.redirect_chain,
        &head.final_url,
    )
    .map_err(|message| {
        format!(
            "Failed to load shared worker script `{}`: {message}",
            head.final_url
        )
    })?;
    moli_fetch::ensure_http_status_success(head.final_url.as_str(), head.status, false)
        .map_err(|error| error.to_string())?;
    crate::worker::ensure_worker_script_mime_acceptable(
        &head.final_url,
        &head.headers,
        &body_bytes,
    )?;
    let response_referrer_policy = response_referrer_policy(&head.headers);
    let response_policy_context =
        worker_policy_context_from_response(&head.final_url, &head.headers);
    let response_content_security_policies =
        crate::content_security_policy::content_security_policy_headers(&head.headers);
    let response_content_security_report_only_policies =
        crate::content_security_policy::content_security_policy_report_only_headers(&head.headers);
    let response_content_security_reporting_endpoints =
        crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
            &head.headers,
            &head.final_url,
        );
    let mut final_url = head.final_url;
    final_url.set_fragment(script_url.fragment());
    Ok(SharedWorkerLoadedScript::new(final_url.to_string(), body)
        .with_response_referrer_policy(response_referrer_policy)
        .with_response_policy_context(response_policy_context)
        .with_response_content_security_policies(response_content_security_policies)
        .with_response_content_security_report_only_policies(
            response_content_security_report_only_policies,
        )
        .with_response_content_security_reporting_endpoints(
            response_content_security_reporting_endpoints,
        ))
}

fn shared_worker_script_request(
    request_url: &Url,
    initiator_url: &Url,
    request_policy: SharedWorkerScriptRequestPolicy,
) -> Result<Request, String> {
    let request = Request::new("GET", request_url.as_str(), None, vec![])
        .map_err(|error| error.to_string())
        .map(|request| {
            request
                .with_credentials_mode(request_policy.credentials_mode)
                .with_page_network_policy()
                .with_initiator_url(initiator_url)
                .with_network_partition_key(request_policy.network_partition_key.clone())
                .with_script_fetch_metadata(ScriptFetchRequestMetadata {
                    document_referrer_policy: request_policy.document_referrer_policy.clone(),
                    ..ScriptFetchRequestMetadata::default()
                })
        })?;
    Ok(match request_policy.same_site_cookies {
        SharedWorkerSameSiteCookies::All => request,
        SharedWorkerSameSiteCookies::None => request.with_cross_site_cookie_context(),
    })
}

pub(super) fn shared_worker_script_credentials_mode(
    descriptor: &SharedWorkerDescriptor,
) -> RequestCredentialsMode {
    match descriptor.script_type() {
        SharedWorkerScriptType::Classic => RequestCredentialsMode::SameOrigin,
        SharedWorkerScriptType::Module => match descriptor.credentials_mode() {
            SharedWorkerCredentialsMode::Omit => RequestCredentialsMode::Omit,
            SharedWorkerCredentialsMode::SameOrigin => RequestCredentialsMode::SameOrigin,
            SharedWorkerCredentialsMode::Include => RequestCredentialsMode::Include,
        },
    }
}

pub(super) fn load_shared_worker_blob_script_source(
    script_url: &Url,
) -> Result<SharedWorkerLoadedScript, String> {
    let mut resource_url = script_url.clone();
    resource_url.set_fragment(None);
    crate::blob::object_url_body_and_type(resource_url.as_str())
        .map(|(body, _)| SharedWorkerLoadedScript::new(script_url.to_string(), body))
        .ok_or_else(|| {
            format!("Failed to load shared worker script `{script_url}`: blob URL is unavailable.")
        })
}

fn response_referrer_policy(headers: &[(String, String)]) -> Option<String> {
    response_referrer_policy_from_headers(headers)
}

fn worker_policy_context_from_response(
    final_url: &Url,
    headers: &[(String, String)],
) -> SubresourcePolicyContext {
    SubresourcePolicyContext {
        cross_origin_embedder_policy:
            crate::cross_origin_isolation::cross_origin_embedder_policy_from_headers(headers),
        document_isolation_policy:
            crate::cross_origin_isolation::document_isolation_policy_from_headers(headers),
        cross_origin_isolated:
            crate::cross_origin_isolation::response_headers_enable_cross_origin_isolation(
                final_url, headers,
            ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(
        script_type: SharedWorkerScriptType,
        credentials_mode: SharedWorkerCredentialsMode,
    ) -> SharedWorkerDescriptor {
        SharedWorkerDescriptor::new(
            script_type,
            credentials_mode,
            false,
            moli_shared_worker::SharedWorkerCreationContextType::Secure,
        )
    }

    #[test]
    fn shared_worker_fetch_credentials_mode_tracks_module_options() {
        assert_eq!(
            shared_worker_script_credentials_mode(&descriptor(
                SharedWorkerScriptType::Module,
                SharedWorkerCredentialsMode::Omit
            )),
            RequestCredentialsMode::Omit
        );
        assert_eq!(
            shared_worker_script_credentials_mode(&descriptor(
                SharedWorkerScriptType::Module,
                SharedWorkerCredentialsMode::SameOrigin
            )),
            RequestCredentialsMode::SameOrigin
        );
        assert_eq!(
            shared_worker_script_credentials_mode(&descriptor(
                SharedWorkerScriptType::Module,
                SharedWorkerCredentialsMode::Include
            )),
            RequestCredentialsMode::Include
        );
    }

    #[test]
    fn shared_worker_script_request_applies_module_credentials_option() {
        let request_url = Url::parse("https://app.test/worker.js").unwrap();
        let initiator = Url::parse("https://app.test/page.html").unwrap();

        for (credentials_mode, expected) in [
            (
                SharedWorkerCredentialsMode::Omit,
                RequestCredentialsMode::Omit,
            ),
            (
                SharedWorkerCredentialsMode::SameOrigin,
                RequestCredentialsMode::SameOrigin,
            ),
            (
                SharedWorkerCredentialsMode::Include,
                RequestCredentialsMode::Include,
            ),
        ] {
            let request = shared_worker_script_request(
                &request_url,
                &initiator,
                SharedWorkerScriptRequestPolicy::from_descriptor(
                    &descriptor(SharedWorkerScriptType::Module, credentials_mode),
                    SharedWorkerSameSiteCookies::All,
                    None,
                    None,
                    Vec::new(),
                ),
            )
            .unwrap();
            assert_eq!(request.credentials_mode, expected);
            assert!(request.uses_page_network_policy());
        }
    }

    #[test]
    fn shared_worker_script_request_same_site_none_forces_cross_site_cookie_context() {
        let request_url = Url::parse("https://app.test/worker.js").unwrap();
        let initiator = Url::parse("https://app.test/page.html").unwrap();
        let request = shared_worker_script_request(
            &request_url,
            &initiator,
            SharedWorkerScriptRequestPolicy::from_descriptor(
                &descriptor(
                    SharedWorkerScriptType::Classic,
                    SharedWorkerCredentialsMode::SameOrigin,
                ),
                SharedWorkerSameSiteCookies::None,
                None,
                None,
                Vec::new(),
            ),
        )
        .unwrap();

        assert!(request.cookie_context.site_context.is_cross_site());
    }

    #[test]
    fn classic_shared_worker_fetch_ignores_credentials_option_for_script_request() {
        assert_eq!(
            shared_worker_script_credentials_mode(&descriptor(
                SharedWorkerScriptType::Classic,
                SharedWorkerCredentialsMode::Omit
            )),
            RequestCredentialsMode::SameOrigin
        );
        assert_eq!(
            shared_worker_script_credentials_mode(&descriptor(
                SharedWorkerScriptType::Classic,
                SharedWorkerCredentialsMode::Include
            )),
            RequestCredentialsMode::SameOrigin
        );
    }

    #[test]
    fn shared_worker_response_policy_context_uses_response_headers() {
        let headers = vec![
            (
                "Cross-Origin-Embedder-Policy".to_owned(),
                "require-corp".to_owned(),
            ),
            (
                "Document-Isolation-Policy".to_owned(),
                "isolate-and-credentialless".to_owned(),
            ),
        ];

        let final_url = Url::parse("https://worker.test/shared.js").unwrap();
        let policy_context = worker_policy_context_from_response(&final_url, &headers);

        assert_eq!(
            policy_context.cross_origin_embedder_policy,
            crate::cross_origin_isolation::CrossOriginEmbedderPolicy::RequireCorp
        );
        assert_eq!(
            policy_context.document_isolation_policy,
            crate::cross_origin_isolation::DocumentIsolationPolicy::IsolateAndCredentialless
        );
        assert!(policy_context.cross_origin_isolated);
    }

    #[test]
    fn shared_worker_script_request_carries_document_referrer_policy() {
        let request_url = Url::parse("https://cdn.test/worker.js").unwrap();
        let initiator = Url::parse("https://app.test/page.html").unwrap();
        let request = shared_worker_script_request(
            &request_url,
            &initiator,
            SharedWorkerScriptRequestPolicy::from_descriptor(
                &descriptor(
                    SharedWorkerScriptType::Classic,
                    SharedWorkerCredentialsMode::SameOrigin,
                ),
                SharedWorkerSameSiteCookies::All,
                Some("no-referrer".to_owned()),
                None,
                Vec::new(),
            ),
        )
        .unwrap();

        assert_eq!(
            request
                .subresource_request_metadata()
                .and_then(|metadata| metadata.document_referrer_policy.as_deref()),
            Some("no-referrer")
        );
    }

    #[test]
    fn shared_worker_script_request_carries_network_partition_key() {
        let request_url = Url::parse("https://cdn.test/worker.js").unwrap();
        let initiator = Url::parse("https://app.test/page.html").unwrap();
        let request = shared_worker_script_request(
            &request_url,
            &initiator,
            SharedWorkerScriptRequestPolicy::from_descriptor(
                &descriptor(
                    SharedWorkerScriptType::Classic,
                    SharedWorkerCredentialsMode::SameOrigin,
                ),
                SharedWorkerSameSiteCookies::All,
                None,
                Some("credentialless-key".to_owned()),
                Vec::new(),
            ),
        )
        .unwrap();

        assert_eq!(request.network_partition_key(), Some("credentialless-key"));
    }

    #[test]
    fn shared_worker_response_referrer_policy_combines_header_instances() {
        let headers = vec![
            ("Referrer-Policy".to_owned(), "no-referrer".to_owned()),
            ("referrer-policy".to_owned(), "future-policy".to_owned()),
        ];

        assert_eq!(
            response_referrer_policy(&headers),
            Some("no-referrer".to_owned())
        );
    }

    #[test]
    fn shared_worker_response_referrer_policy_uses_last_valid_token() {
        let headers = vec![(
            "Referrer-Policy".to_owned(),
            "not-yet-standardized, no-referrer".to_owned(),
        )];

        assert_eq!(
            response_referrer_policy(&headers),
            Some("no-referrer".to_owned())
        );
    }

    #[test]
    fn shared_worker_response_referrer_policy_ignores_invalid_later_header() {
        let headers = vec![
            ("Referrer-Policy".to_owned(), "no-referrer".to_owned()),
            (
                "Referrer-Policy".to_owned(),
                "not-yet-standardized".to_owned(),
            ),
        ];

        assert_eq!(
            response_referrer_policy(&headers),
            Some("no-referrer".to_owned())
        );
    }
}
