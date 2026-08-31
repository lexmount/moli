use std::{future::Future, pin::Pin, time::Instant};

use anyhow::{Result, anyhow, ensure};
use moli_fetch::{BrowserNavigationRequestKind, FetchCancelHandle, Request, StreamingRawResponse};
use tokio::sync::oneshot;
use tracing::debug;
use url::Url;

use crate::local_executor::{JsLocalExecutor, is_on_named_owner_execution_lane_for};
use crate::network::ResourceRequestClient;
use crate::runtime::page::PageVmNavigationResponse;
use crate::runtime::phase_one::{
    ConcurrentParseTimeRuntime, ParseTimePageVmCreationOutcome,
    StreamingNavigationPageCreationResult, response_headers_indicate_download,
};
use crate::runtime::{
    ExternalRawDocumentBodyStream, PageId, PageVmFollowNavigationTurnOutcome,
    PageVmFollowedNavigationBuildOutcome, PageVmFollowedNavigationMetadata, PageVmInitStage,
    PageVmPendingPhaseOneNavigation, PendingDocumentLifecycleTurn, RendererBrowserContextRuntime,
    RendererDocumentLifecycleTransition, RendererDocumentTerminationReason,
    RendererLifecycleStartReason, RendererPendingDownloadActivation,
    RendererPendingDownloadResponse,
};

use super::{PageVm, PageVmEnvConfig, PageVmRuntimeHooks};

pub(super) enum LoadedFollowedLocationNavigation {
    NoDocument,
    Download(RendererPendingDownloadActivation),
    StreamingDocument {
        requested_url: Url,
        response: Box<StreamingRawResponse>,
    },
    ExternalDocument {
        requested_url: Url,
        final_url: Url,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FollowedLocationNavigationBootstrapBoundary {
    #[cfg(test)]
    ContinuePhaseOne,
    DocumentCommit,
}

#[cfg(test)]
impl LoadedFollowedLocationNavigation {
    fn has_header_for_test(&self, expected_name: &str) -> bool {
        let headers = match self {
            Self::NoDocument | Self::Download(_) => return false,
            Self::StreamingDocument { response, .. } => &response.headers,
            Self::ExternalDocument {
                response_headers, ..
            } => response_headers,
        };
        headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case(expected_name) && value == "1")
    }
}

pub(super) async fn load_followed_location_navigation(
    loader: &ResourceRequestClient,
    initiator_url: Url,
    url: Url,
    request_method: String,
    request_body: Option<Vec<u8>>,
    request_headers: Vec<(String, String)>,
    browser_navigation_kind: BrowserNavigationRequestKind,
) -> Result<LoadedFollowedLocationNavigation> {
    debug!(%url, "starting pre-commit location navigation fetch");
    if let Some(response) = about_blank_navigation_response(&url)
        .or_else(|| crate::network_host::local_url_response(&url))
    {
        if matches!(response.status, 204 | 205) {
            return Ok(LoadedFollowedLocationNavigation::NoDocument);
        }
        if response_headers_indicate_download(&response.headers) {
            let (head, body) = response.into_body();
            let body = body
                .try_into_materialized_bytes()
                .map_err(|_| anyhow!("local navigation download body was not materialized"))?;
            return Ok(LoadedFollowedLocationNavigation::Download(
                RendererPendingDownloadActivation {
                    url: head.final_url.as_str().to_owned(),
                    suggested_filename: None,
                    response: Some(RendererPendingDownloadResponse {
                        final_url: head.final_url.as_str().to_owned(),
                        status: head.status,
                        headers: head.headers,
                        body,
                    }),
                },
            ));
        }
        let (final_url, response_status, response_headers, raw_body) =
            external_raw_document_body_from_materialized_response(response)?;
        return Ok(LoadedFollowedLocationNavigation::ExternalDocument {
            requested_url: url,
            final_url,
            response_status,
            response_headers,
            raw_body,
        });
    }
    let request = build_followed_location_navigation_request(
        &initiator_url,
        &url,
        &request_method,
        request_body,
        request_headers,
        browser_navigation_kind,
    )
    .map_err(|error| anyhow!("failed to build location navigation request: {error}"))?;
    let mut response = loader
        .fetch_raw_stream_with_cancel(request, FetchCancelHandle::new())
        .await?;
    if matches!(response.status, 204 | 205) {
        while response.next_chunk().await.is_some() {}
        response.finish().await?;
        return Ok(LoadedFollowedLocationNavigation::NoDocument);
    }
    if response_headers_indicate_download(&response.headers) {
        let mut body = Vec::new();
        while let Some(chunk) = response.next_chunk().await {
            body.extend_from_slice(&chunk);
        }
        response.finish().await?;
        return Ok(LoadedFollowedLocationNavigation::Download(
            RendererPendingDownloadActivation {
                url: response.final_url.as_str().to_owned(),
                suggested_filename: None,
                response: Some(RendererPendingDownloadResponse {
                    final_url: response.final_url.as_str().to_owned(),
                    status: response.status,
                    headers: response.headers.clone(),
                    body,
                }),
            },
        ));
    }
    Ok(LoadedFollowedLocationNavigation::StreamingDocument {
        requested_url: url,
        response: Box::new(response),
    })
}

pub(super) async fn bootstrap_committed_followed_location_navigation(
    page_id: PageId,
    local_executor: JsLocalExecutor,
    loader: ResourceRequestClient,
    env: PageVmEnvConfig,
    runtime_hooks: PageVmRuntimeHooks,
    navigation_bootstrap_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
    reserved_service_worker_client_id: Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    stage: PageVmInitStage,
    loaded: LoadedFollowedLocationNavigation,
    boundary: FollowedLocationNavigationBootstrapBoundary,
) -> Result<PageVmFollowedNavigationBuildOutcome> {
    debug_assert!(
        is_on_named_owner_execution_lane_for(&local_executor),
        "committed location-navigation bootstrap must execute on the matching named owner lane"
    );
    #[cfg(test)]
    if loaded.has_header_for_test("x-moli-test-panic-after-navigation-commit") {
        panic!("injected panic after main navigation commit for testing");
    }
    #[cfg(test)]
    if loaded.has_header_for_test("x-moli-test-fail-after-navigation-commit") {
        return Err(anyhow!(
            "injected failure after main navigation commit for testing"
        ));
    }
    let env_for_bootstrap = page_vm_env_for_navigation_bootstrap(
        &env,
        navigation_bootstrap_entry,
        reserved_service_worker_client_id,
    );
    let local_executor_clone = local_executor.clone();
    let loader_for_bootstrap = loader.clone();
    let (
        requested_url,
        navigation_redirected,
        navigation_redirect_chain,
        closed_message,
        bootstrap,
    ): (
        _,
        _,
        _,
        _,
        Pin<
            Box<
                dyn Future<
                    Output = Result<(
                        PageVmFollowedNavigationBuildOutcome,
                        u16,
                        Vec<(String, String)>,
                    )>,
                >,
            >,
        >,
    ) = match loaded {
        LoadedFollowedLocationNavigation::NoDocument
        | LoadedFollowedLocationNavigation::Download(_) => {
            unreachable!("no-Document response must be returned before navigation commit")
        }
        LoadedFollowedLocationNavigation::StreamingDocument {
            requested_url,
            response,
        } => {
            let navigation_redirected = response.redirected;
            let navigation_redirect_chain = response
                .redirect_chain
                .iter()
                .cloned()
                .map(Into::into)
                .collect();
            let bootstrap = Box::pin(async move {
                let started = Instant::now();
                let result = match boundary {
                    #[cfg(test)]
                    FollowedLocationNavigationBootstrapBoundary::ContinuePhaseOne => {
                        ConcurrentParseTimeRuntime::finish_creation_from_committed_streaming_navigation_response(
                            page_id,
                            local_executor,
                            &loader_for_bootstrap,
                            &env_for_bootstrap,
                            runtime_hooks,
                            stage,
                            started,
                            response,
                        )
                        .await?
                    }
                    FollowedLocationNavigationBootstrapBoundary::DocumentCommit => {
                        ConcurrentParseTimeRuntime::prepare_document_from_committed_streaming_navigation_response(
                            page_id,
                            local_executor,
                            &loader_for_bootstrap,
                            &env_for_bootstrap,
                            runtime_hooks,
                            stage,
                            started,
                            response,
                        )
                        .await?
                    }
                };
                streaming_navigation_result_to_turn_outcome(result).await
            });
            (
                requested_url,
                navigation_redirected,
                navigation_redirect_chain,
                "committed location-navigation bootstrap local task channel closed",
                bootstrap,
            )
        }
        LoadedFollowedLocationNavigation::ExternalDocument {
            requested_url,
            final_url,
            response_status,
            response_headers,
            raw_body,
        } => {
            let bootstrap = Box::pin(async move {
                let started = Instant::now();
                let result = match boundary {
                    #[cfg(test)]
                    FollowedLocationNavigationBootstrapBoundary::ContinuePhaseOne => {
                        ConcurrentParseTimeRuntime::finish_creation_from_committed_external_raw_document_response(
                            page_id,
                            local_executor,
                            &loader_for_bootstrap,
                            &env_for_bootstrap,
                            runtime_hooks,
                            stage,
                            started,
                            final_url,
                            response_status,
                            response_headers,
                            raw_body,
                        )
                        .await?
                    }
                    FollowedLocationNavigationBootstrapBoundary::DocumentCommit => {
                        ConcurrentParseTimeRuntime::prepare_document_from_committed_external_raw_document_response(
                            page_id,
                            local_executor,
                            &loader_for_bootstrap,
                            &env_for_bootstrap,
                            runtime_hooks,
                            stage,
                            started,
                            final_url,
                            response_status,
                            response_headers,
                            raw_body,
                        )
                        .await?
                    }
                };
                streaming_navigation_result_to_turn_outcome(result).await
            });
            (
                requested_url,
                false,
                Vec::new(),
                "committed local-response navigation bootstrap local task channel closed",
                bootstrap,
            )
        }
    };
    let (mut page_vm_outcome, response_status, response_headers) =
        PageVm::run_bootstrap_future_on_fresh_local_task(
            local_executor_clone,
            closed_message,
            bootstrap,
        )
        .await?;
    attach_followed_navigation_response(
        &mut page_vm_outcome,
        PageVmNavigationResponse {
            requested_url,
            redirected: navigation_redirected,
            redirect_count: navigation_redirect_chain.len(),
            redirect_chain: navigation_redirect_chain,
            status: response_status,
            headers: response_headers,
        },
    );
    Ok(page_vm_outcome)
}

fn page_vm_env_for_navigation_bootstrap(
    env: &PageVmEnvConfig,
    navigation_bootstrap_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
    reserved_service_worker_client_id: Option<crate::service_worker_runtime::ServiceWorkerClientId>,
) -> PageVmEnvConfig {
    let mut env = env.clone();
    env.document_policy_container
        .response_content_security_policies
        .clear();
    env.document_policy_container
        .response_content_security_report_only_policies
        .clear();
    env.document_policy_container.referrer_policy = None;
    env.document_last_modified = None;
    env.document_policy_container
        .content_security_reporting_endpoints =
        crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default();
    env.document_policy_container.cross_origin_embedder_policy = Default::default();
    env.document_policy_container.cross_origin_isolated = false;
    env.document_policy_container.sandbox = Default::default();
    env.top_level_storage_key = None;
    env.navigation_bootstrap_entry = navigation_bootstrap_entry;
    env.reserved_service_worker_client_id = reserved_service_worker_client_id;
    env
}

async fn streaming_navigation_result_to_turn_outcome(
    result: StreamingNavigationPageCreationResult,
) -> Result<(
    PageVmFollowedNavigationBuildOutcome,
    u16,
    Vec<(String, String)>,
)> {
    match result {
        StreamingNavigationPageCreationResult::Download(download) => Ok((
            PageVmFollowedNavigationBuildOutcome::Download(download),
            0,
            Vec::new(),
        )),
        StreamingNavigationPageCreationResult::Html(result) => {
            let result = *result;
            let response_status = result.response_status;
            let response_headers = result.response_headers;
            match result.outcome {
                ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) => Ok((
                    PageVmFollowedNavigationBuildOutcome::PendingPhaseOne(
                        PageVmPendingPhaseOneNavigation::new(residence, Default::default()),
                    ),
                    response_status,
                    response_headers,
                )),
                ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage } => Ok((
                    PageVmFollowedNavigationBuildOutcome::TriggeredNavigation { page_vm, stage },
                    response_status,
                    response_headers,
                )),
                ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                    page_vm,
                    page_tasks,
                    stage,
                    started,
                } => Ok((
                    PageVmFollowedNavigationBuildOutcome::ContinuePostParseLifecycle {
                        page_vm,
                        page_tasks,
                        stage,
                        started,
                    },
                    response_status,
                    response_headers,
                )),
            }
        }
    }
}

pub(in crate::runtime) fn attach_navigation_response_to_page_vm(
    page_vm: &mut PageVm,
    response: PageVmNavigationResponse,
) {
    page_vm.navigation_response = Some(response);
}

fn attach_followed_navigation_response(
    page_vm_outcome: &mut PageVmFollowedNavigationBuildOutcome,
    response: PageVmNavigationResponse,
) {
    match page_vm_outcome {
        PageVmFollowedNavigationBuildOutcome::ContinuePostParseLifecycle { page_vm, .. }
        | PageVmFollowedNavigationBuildOutcome::TriggeredNavigation { page_vm, .. } => {
            attach_navigation_response_to_page_vm(page_vm, response);
        }
        PageVmFollowedNavigationBuildOutcome::PendingPhaseOne(pending) => {
            pending.metadata.committed_navigation_response = Some(response);
        }
        PageVmFollowedNavigationBuildOutcome::Download(_) => {}
    }
}

fn external_raw_document_body_from_materialized_response(
    response: moli_fetch::Response,
) -> Result<(
    Url,
    u16,
    Vec<(String, String)>,
    ExternalRawDocumentBodyStream,
)> {
    let (head, body) = response.into_body();
    let body = body
        .try_into_materialized_bytes()
        .map_err(|_| anyhow!("local navigation response body was not materialized"))?;
    let (completion_tx, completion_rx) = oneshot::channel();
    let (body_tx, body_stream) = ExternalRawDocumentBodyStream::channel(completion_rx);
    let _ = body_tx.try_send(body);
    drop(body_tx);
    let _ = completion_tx.send(Ok(()));
    Ok((head.final_url, head.status, head.headers, body_stream))
}

fn build_followed_location_navigation_request(
    initiator_url: &Url,
    url: &Url,
    request_method: &str,
    request_body: Option<Vec<u8>>,
    request_headers: Vec<(String, String)>,
    browser_navigation_kind: BrowserNavigationRequestKind,
) -> Result<Request> {
    Request::new_bytes(request_method, url.as_str(), request_body, request_headers).map(|request| {
        request
            .with_top_level_navigation_cookie_context()
            .with_browser_navigation_kind(browser_navigation_kind)
            .with_initiator_url(initiator_url)
    })
}

fn about_blank_navigation_response(url: &Url) -> Option<moli_fetch::Response> {
    if url.scheme() != "about" || url.path() != "blank" {
        return None;
    }
    Some(moli_fetch::Response::from_head_and_lossy_body_bytes(
        moli_fetch::ResponseHead {
            final_url: url.clone(),
            status: 200,
            headers: vec![(
                "Content-Type".to_owned(),
                "text/html; charset=utf-8".to_owned(),
            )],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        Vec::new(),
    ))
}

fn mark_followed_navigation_document_commit(
    outcome: &mut PageVmFollowedNavigationBuildOutcome,
    handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
) -> Result<()> {
    match outcome {
        PageVmFollowedNavigationBuildOutcome::ContinuePostParseLifecycle { page_vm, .. }
        | PageVmFollowedNavigationBuildOutcome::TriggeredNavigation { page_vm, .. } => {
            page_vm.prepare_replacement_document_commit(handoff)
        }
        PageVmFollowedNavigationBuildOutcome::PendingPhaseOne(pending) => pending
            .page_vm_mut()
            .prepare_replacement_document_commit(handoff),
        PageVmFollowedNavigationBuildOutcome::Download(_) => Ok(()),
    }
}

pub(in crate::runtime) enum PageVmDocumentCommitPreparation {
    Uncommitted(Box<PageVmFollowNavigationTurnOutcome>),
    Prepared(Box<PageVmPreparedFollowedNavigationCommit>),
}

pub(in crate::runtime) struct PageVmPreparedFollowedNavigationCommit {
    initiator_url: Url,
    navigation_handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    loaded: LoadedFollowedLocationNavigation,
    navigation_bootstrap_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
    reserved_service_worker_client_id: Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    service_worker_client_navigate: Option<crate::types::ServiceWorkerClientNavigateContinuation>,
    stage: PageVmInitStage,
}

struct PageVmCommittedNavigationBootstrapPayload {
    page_id: PageId,
    local_executor: JsLocalExecutor,
    request_client: ResourceRequestClient,
    env: PageVmEnvConfig,
    runtime_hooks: PageVmRuntimeHooks,
    navigation_bootstrap_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
    loaded: LoadedFollowedLocationNavigation,
    stage: PageVmInitStage,
}

/// Owns all continuation state after the source Document has committed away
/// and until a replacement `PageVm` has been produced.
///
/// Failure metadata intentionally remains outside the consumed bootstrap
/// payload. If the local task is cancelled while awaiting the replacement,
/// the checked-out committed entry can still reject the ServiceWorker follow
/// and retire the Page without consulting the dead source `ScriptVm`.
pub(in crate::runtime) struct PageVmCommittedNavigationBootstrap {
    payload: Option<PageVmCommittedNavigationBootstrapPayload>,
    browser_context_runtime: RendererBrowserContextRuntime,
    initiator_url: Option<Url>,
    navigation_handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    reserved_service_worker_client_id: Option<crate::service_worker_runtime::ServiceWorkerClientId>,
    service_worker_client_navigate: Option<crate::types::ServiceWorkerClientNavigateContinuation>,
}

impl PageVmCommittedNavigationBootstrap {
    pub(in crate::runtime) async fn bootstrap(
        &mut self,
    ) -> Result<PageVmFollowedNavigationBuildOutcome> {
        let payload = self.payload.take().ok_or_else(|| {
            anyhow!("committed location-navigation bootstrap was already consumed")
        })?;
        bootstrap_committed_followed_location_navigation(
            payload.page_id,
            payload.local_executor,
            payload.request_client,
            payload.env,
            payload.runtime_hooks,
            payload.navigation_bootstrap_entry,
            self.reserved_service_worker_client_id,
            payload.stage,
            payload.loaded,
            FollowedLocationNavigationBootstrapBoundary::DocumentCommit,
        )
        .await
    }

    pub(in crate::runtime) fn finalize_build_outcome(
        &mut self,
        mut outcome: PageVmFollowedNavigationBuildOutcome,
    ) -> Result<PageVmFollowedNavigationBuildOutcome> {
        mark_followed_navigation_document_commit(&mut outcome, self.navigation_handoff)?;
        match &mut outcome {
            PageVmFollowedNavigationBuildOutcome::ContinuePostParseLifecycle {
                page_vm, ..
            }
            | PageVmFollowedNavigationBuildOutcome::TriggeredNavigation { page_vm, .. } => {
                if let Some(continuation) = self.service_worker_client_navigate.take() {
                    page_vm
                        .vm_mut()
                        .complete_pending_service_worker_client_navigate_after_follow(continuation);
                }
                self.reserved_service_worker_client_id = None;
                self.initiator_url = None;
            }
            PageVmFollowedNavigationBuildOutcome::PendingPhaseOne(pending) => {
                pending.metadata.service_worker_client_navigate =
                    self.service_worker_client_navigate.take();
                pending.metadata.abort_reserved_service_worker_client_id =
                    self.reserved_service_worker_client_id.take();
                pending.metadata.abort_navigation_initiator_url = self.initiator_url.take();
            }
            PageVmFollowedNavigationBuildOutcome::Download(download) => {
                return Err(anyhow!(
                    "committed location-navigation bootstrap produced a download for {} without a replacement PageVm",
                    download.url
                ));
            }
        }
        Ok(outcome)
    }

    pub(in crate::runtime) fn reject(&mut self, failure: &str) {
        let mut metadata = PageVmFollowedNavigationMetadata {
            service_worker_client_navigate: self.service_worker_client_navigate.take(),
            abort_reserved_service_worker_client_id: self.reserved_service_worker_client_id.take(),
            ..PageVmFollowedNavigationMetadata::default()
        };
        metadata.reject(
            None,
            &self.browser_context_runtime,
            format!("Cannot navigate to URL: {failure}"),
        );
        self.initiator_url = None;
    }
}

impl PageVm {
    #[cfg(test)]
    pub(super) async fn follow_pending_location_navigation_one_turn_async(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        stage: PageVmInitStage,
    ) -> Result<PageVmFollowNavigationTurnOutcome> {
        match self
            .prepare_pending_location_navigation_one_turn_async(
                pending_document_lifecycle_turn,
                stage,
            )
            .await?
        {
            PageVmDocumentCommitPreparation::Uncommitted(outcome) => Ok(*outcome),
            PageVmDocumentCommitPreparation::Prepared(prepared) => {
                self.finish_prepared_followed_location_navigation_for_test(
                    pending_document_lifecycle_turn,
                    *prepared,
                )
                .await
            }
        }
    }

    pub(in crate::runtime) async fn prepare_pending_location_navigation_document_commit_one_turn_async(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        stage: PageVmInitStage,
    ) -> Result<PageVmDocumentCommitPreparation> {
        self.prepare_pending_location_navigation_one_turn_async(
            pending_document_lifecycle_turn,
            stage,
        )
        .await
    }

    async fn prepare_pending_location_navigation_one_turn_async(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        stage: PageVmInitStage,
    ) -> Result<PageVmDocumentCommitPreparation> {
        let Some(pending) = self.vm_mut().take_pending_location_navigation_with_seed() else {
            return Ok(PageVmDocumentCommitPreparation::Uncommitted(Box::new(
                PageVmFollowNavigationTurnOutcome::Completed,
            )));
        };
        let navigation_kind = pending.kind();
        let initiator_url = self.vm().document_runtime.document_url().clone();
        let navigation_handoff = pending.handoff;
        let url = pending.url.clone();
        let request_method = pending.request_method.clone();
        let request_body = pending.request_body.clone();
        let request_headers = pending.request_headers.clone();
        let browser_navigation_kind = pending.browser_navigation_kind;
        let reserved_service_worker_client_id = pending
            .reserved_service_worker_client
            .map(|reserved| reserved.release());
        let service_worker_client_navigate = pending.service_worker_client_navigate;
        tracing::debug!(stage = ?stage, %url, "following pending location navigation asynchronously");

        if navigation_kind == crate::native_bridge::PendingLocationNavigationKind::JavascriptUrl {
            if let Some(client_id) = reserved_service_worker_client_id {
                self.vm_mut()
                    .unregister_reserved_service_worker_client_after_navigation_abort(client_id);
            }
            let replacement_lifecycle_snapshot =
                self.document_replacement_lifecycle_action_snapshot();
            let source_document = self.document_lifecycle.identity();
            let outcome =
                self.follow_taken_javascript_location_navigation(initiator_url, url, stage);
            let reconciliation = self
                .reconcile_javascript_navigation_lifecycle_after_owner_action(
                    replacement_lifecycle_snapshot,
                    pending_document_lifecycle_turn,
                    source_document,
                )
                .await;
            let outcome = match (outcome, reconciliation) {
                (Ok(PageVmFollowNavigationTurnOutcome::Completed), Ok(reconciliation)) => {
                    Ok(reconciliation.into_follow_outcome_after_completed_javascript_url())
                }
                (Ok(outcome), Ok(_)) => Ok(outcome),
                (Err(navigation_error), Ok(_)) => Err(navigation_error),
                (Ok(_), Err(reconciliation_error)) => Err(reconciliation_error),
                (Err(navigation_error), Err(reconciliation_error)) => Err(anyhow!(
                    "javascript: navigation failed ({navigation_error:#}) and its Document replacement lifecycle reconciliation also failed ({reconciliation_error:#})"
                )),
            };
            if let Some(continuation) = service_worker_client_navigate {
                match &outcome {
                    Ok(PageVmFollowNavigationTurnOutcome::Download(_)) => self
                        .vm_mut()
                        .reject_pending_service_worker_client_navigate_after_follow(
                            continuation,
                            "Cannot navigate to URL.".to_owned(),
                        ),
                    Ok(PageVmFollowNavigationTurnOutcome::Completed)
                    | Ok(PageVmFollowNavigationTurnOutcome::PostParseLifecycle { .. })
                    | Ok(PageVmFollowNavigationTurnOutcome::TriggeredNavigation { .. }) => self
                        .vm_mut()
                        .complete_pending_service_worker_client_navigate_after_follow(continuation),
                    Err(error) => self
                        .vm_mut()
                        .reject_pending_service_worker_client_navigate_after_follow(
                            continuation,
                            format!("Cannot navigate to URL: {error}"),
                        ),
                }
            }
            return outcome
                .map(|outcome| PageVmDocumentCommitPreparation::Uncommitted(Box::new(outcome)));
        }

        let loaded = match load_followed_location_navigation(
            &self.request_client,
            initiator_url.clone(),
            url,
            request_method,
            request_body,
            request_headers,
            browser_navigation_kind,
        )
        .await
        {
            Ok(loaded) => loaded,
            Err(error) => {
                self.reject_failed_followed_location_navigation(
                    &initiator_url,
                    reserved_service_worker_client_id,
                    service_worker_client_navigate,
                    &error,
                );
                return Err(error);
            }
        };
        let loaded = match loaded {
            LoadedFollowedLocationNavigation::NoDocument => {
                self.abort_followed_navigation_without_document(
                    &initiator_url,
                    reserved_service_worker_client_id,
                    service_worker_client_navigate,
                );
                return Ok(PageVmDocumentCommitPreparation::Uncommitted(Box::new(
                    PageVmFollowNavigationTurnOutcome::Completed,
                )));
            }
            LoadedFollowedLocationNavigation::Download(download) => {
                self.abort_followed_navigation_without_document(
                    &initiator_url,
                    reserved_service_worker_client_id,
                    service_worker_client_navigate,
                );
                return Ok(PageVmDocumentCommitPreparation::Uncommitted(Box::new(
                    PageVmFollowNavigationTurnOutcome::Download(download),
                )));
            }
            loaded @ (LoadedFollowedLocationNavigation::StreamingDocument { .. }
            | LoadedFollowedLocationNavigation::ExternalDocument { .. }) => loaded,
        };

        let termination = self.document_lifecycle.request_termination(
            self.document_lifecycle.identity(),
            RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
        );
        debug_assert!(
            matches!(
                termination,
                RendererDocumentLifecycleTransition::Recorded(_)
                    | RendererDocumentLifecycleTransition::Deferred
                    | RendererDocumentLifecycleTransition::Duplicate
            ),
            "cross-document navigation should terminate the active renderer document: {termination:?}"
        );
        *pending_document_lifecycle_turn = None;

        Ok(PageVmDocumentCommitPreparation::Prepared(Box::new(
            PageVmPreparedFollowedNavigationCommit {
                initiator_url,
                navigation_handoff,
                loaded,
                navigation_bootstrap_entry: pending.entry_seed,
                reserved_service_worker_client_id,
                service_worker_client_navigate,
                stage,
            },
        )))
    }

    #[cfg(test)]
    async fn finish_prepared_followed_location_navigation_for_test(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        prepared: PageVmPreparedFollowedNavigationCommit,
    ) -> Result<PageVmFollowNavigationTurnOutcome> {
        let PageVmPreparedFollowedNavigationCommit {
            initiator_url,
            loaded,
            navigation_bootstrap_entry,
            reserved_service_worker_client_id,
            service_worker_client_navigate,
            stage,
            ..
        } = prepared;
        let outcome = match self
            .bootstrap_followed_location_navigation(
                loaded,
                navigation_bootstrap_entry,
                reserved_service_worker_client_id,
                stage,
                FollowedLocationNavigationBootstrapBoundary::ContinuePhaseOne,
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                self.reject_failed_followed_location_navigation(
                    &initiator_url,
                    reserved_service_worker_client_id,
                    service_worker_client_navigate,
                    &error,
                );
                return Err(error);
            }
        };
        Ok(match outcome {
            PageVmFollowedNavigationBuildOutcome::ContinuePostParseLifecycle {
                page_vm,
                page_tasks,
                stage,
                started,
            } => {
                *self = page_vm;
                if let Some(continuation) = service_worker_client_navigate {
                    self.vm_mut()
                        .complete_pending_service_worker_client_navigate_after_follow(continuation);
                }
                let lifecycle = self
                    .begin_post_parse_lifecycle_on_named_owner_lane(
                        pending_document_lifecycle_turn,
                        page_tasks,
                        stage,
                        started,
                    )
                    .await?;
                PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                    target_stage: stage,
                    outcome: lifecycle,
                }
            }
            PageVmFollowedNavigationBuildOutcome::Download(download) => {
                if let Some(client_id) = reserved_service_worker_client_id {
                    self.vm_mut()
                        .unregister_reserved_service_worker_client_after_navigation_abort(
                            client_id,
                        );
                }
                self.vm_mut()
                    .restore_top_level_location_runtime_state(&initiator_url);
                if let Some(continuation) = service_worker_client_navigate {
                    self.vm_mut()
                        .reject_pending_service_worker_client_navigate_after_follow(
                            continuation,
                            "Cannot navigate to URL.".to_owned(),
                        );
                }
                PageVmFollowNavigationTurnOutcome::Download(download)
            }
            PageVmFollowedNavigationBuildOutcome::PendingPhaseOne(mut pending) => {
                pending.metadata.service_worker_client_navigate = service_worker_client_navigate;
                pending.metadata.abort_reserved_service_worker_client_id =
                    reserved_service_worker_client_id;
                pending.metadata.abort_navigation_initiator_url = Some(initiator_url);
                let error = anyhow!(
                    "standalone PageVm navigation cannot retain an owner-owned pending phase-one residence"
                );
                let browser_context_runtime = pending
                    .page_vm()
                    .runtime_hooks
                    .browser_context_runtime
                    .clone();
                pending.metadata.reject(
                    None,
                    &browser_context_runtime,
                    format!("Cannot navigate to URL: {error}"),
                );
                pending.page_vm_mut().close_for_context_teardown();
                return Err(error);
            }
            PageVmFollowedNavigationBuildOutcome::TriggeredNavigation { page_vm, stage } => {
                *self = page_vm;
                if let Some(continuation) = service_worker_client_navigate {
                    self.vm_mut()
                        .complete_pending_service_worker_client_navigate_after_follow(continuation);
                }
                PageVmFollowNavigationTurnOutcome::TriggeredNavigation { stage }
            }
        })
    }

    pub(in crate::runtime) fn commit_prepared_followed_location_navigation(
        &mut self,
        prepared: PageVmPreparedFollowedNavigationCommit,
    ) -> Result<PageVmCommittedNavigationBootstrap> {
        let PageVmPreparedFollowedNavigationCommit {
            initiator_url,
            navigation_handoff,
            loaded,
            navigation_bootstrap_entry,
            reserved_service_worker_client_id,
            service_worker_client_navigate,
            stage,
        } = prepared;
        let env = self.followed_location_navigation_env();
        let runtime_hooks = self.runtime_hooks.clone().for_cross_document_commit();
        let browser_context_runtime = runtime_hooks.browser_context_runtime.clone();
        let local_executor = self.local_executor.clone();
        let request_client = self.request_client.clone();
        let page_id = self.page_id;
        let commit_result = (|| {
            ensure!(
                runtime_hooks.has_renderer_page_script_environment(),
                "owner-managed navigation commit requires a renderer Page script environment"
            );
            self.commit_main_window_proxy_navigation()
        })();
        if let Err(error) = commit_result {
            self.reject_failed_followed_location_navigation(
                &initiator_url,
                reserved_service_worker_client_id,
                service_worker_client_navigate,
                &error,
            );
            return Err(error);
        }
        Ok(PageVmCommittedNavigationBootstrap {
            payload: Some(PageVmCommittedNavigationBootstrapPayload {
                page_id,
                local_executor,
                request_client,
                env,
                runtime_hooks,
                navigation_bootstrap_entry,
                loaded,
                stage,
            }),
            browser_context_runtime,
            initiator_url: Some(initiator_url),
            navigation_handoff,
            reserved_service_worker_client_id,
            service_worker_client_navigate,
        })
    }

    fn prepare_replacement_document_commit(
        &mut self,
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    ) -> Result<()> {
        ensure!(
            self.replacement_document_commit_handoff.is_none(),
            "replacement PageVm already owns a pending Document commit identity"
        );
        self.replacement_document_commit_handoff = Some(handoff);
        Ok(())
    }

    fn abort_followed_navigation_without_document(
        &mut self,
        initiator_url: &Url,
        reserved_service_worker_client_id: Option<
            crate::service_worker_runtime::ServiceWorkerClientId,
        >,
        service_worker_client_navigate: Option<
            crate::types::ServiceWorkerClientNavigateContinuation,
        >,
    ) {
        if let Some(client_id) = reserved_service_worker_client_id {
            self.vm_mut()
                .unregister_reserved_service_worker_client_after_navigation_abort(client_id);
        }
        self.vm_mut()
            .restore_top_level_location_runtime_state(initiator_url);
        if let Some(continuation) = service_worker_client_navigate {
            self.vm_mut()
                .reject_pending_service_worker_client_navigate_after_follow(
                    continuation,
                    "Cannot navigate to URL.".to_owned(),
                );
        }
    }

    fn reject_failed_followed_location_navigation(
        &mut self,
        initiator_url: &Url,
        reserved_service_worker_client_id: Option<
            crate::service_worker_runtime::ServiceWorkerClientId,
        >,
        service_worker_client_navigate: Option<
            crate::types::ServiceWorkerClientNavigateContinuation,
        >,
        error: &anyhow::Error,
    ) {
        let navigation_committed = !self.has_live_script_vm();
        if !navigation_committed {
            self.vm_mut()
                .restore_top_level_location_runtime_state(initiator_url);
        }
        if let Some(client_id) = reserved_service_worker_client_id {
            if navigation_committed {
                self.runtime_hooks
                    .browser_context_runtime
                    .unregister_service_worker_client(client_id);
            } else {
                self.vm_mut()
                    .unregister_reserved_service_worker_client_after_navigation_abort(client_id);
            }
        }
        let Some(continuation) = service_worker_client_navigate else {
            return;
        };
        let message = format!("Cannot navigate to URL: {error}");
        if navigation_committed {
            self.runtime_hooks
                .browser_context_runtime
                .service_worker_runtime()
                .enqueue_client_navigate_completed(
                    crate::types::ServiceWorkerClientNavigateCompletion {
                        request_id: continuation.request_id,
                        source_version_id: continuation.source_version_id,
                        source_run: continuation.source_run,
                        result: Err(
                            crate::service_worker_runtime::ServiceWorkerClientNavigateError::type_error(
                                message,
                            ),
                        ),
                    },
                );
        } else {
            self.vm_mut()
                .reject_pending_service_worker_client_navigate_after_follow(continuation, message);
        }
    }

    pub(in crate::runtime) fn replacement_document_commit_handoff(
        &self,
    ) -> Option<crate::page_task_queue::RendererTopLevelNavigationHandoff> {
        self.replacement_document_commit_handoff
    }

    pub(in crate::runtime) fn settle_replacement_document_commit(
        &mut self,
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    ) -> Result<()> {
        ensure!(
            self.replacement_document_commit_handoff == Some(handoff),
            "replacement Document commit identity changed before publication"
        );
        self.replacement_document_commit_handoff = None;
        Ok(())
    }

    pub(super) fn follow_pending_javascript_location_navigation_if_present(
        &mut self,
        stage: PageVmInitStage,
    ) -> Result<PageVmFollowNavigationTurnOutcome> {
        let Some(pending) = self.vm_mut().take_pending_location_navigation_of_kind(
            crate::native_bridge::PendingLocationNavigationKind::JavascriptUrl,
        ) else {
            return Ok(PageVmFollowNavigationTurnOutcome::Completed);
        };
        let initiator_url = self.vm().document_runtime.document_url().clone();
        let service_worker_client_navigate = pending.service_worker_client_navigate;
        let outcome =
            self.follow_taken_javascript_location_navigation(initiator_url, pending.url, stage);
        if let Some(continuation) = service_worker_client_navigate {
            match &outcome {
                Ok(PageVmFollowNavigationTurnOutcome::Download(_)) => self
                    .vm_mut()
                    .reject_pending_service_worker_client_navigate_after_follow(
                        continuation,
                        "Cannot navigate to URL.".to_owned(),
                    ),
                Ok(PageVmFollowNavigationTurnOutcome::Completed)
                | Ok(PageVmFollowNavigationTurnOutcome::PostParseLifecycle { .. })
                | Ok(PageVmFollowNavigationTurnOutcome::TriggeredNavigation { .. }) => self
                    .vm_mut()
                    .complete_pending_service_worker_client_navigate_after_follow(continuation),
                Err(error) => self
                    .vm_mut()
                    .reject_pending_service_worker_client_navigate_after_follow(
                        continuation,
                        format!("Cannot navigate to URL: {error}"),
                    ),
            }
        }
        outcome
    }

    fn follow_taken_javascript_location_navigation(
        &mut self,
        initiator_url: Url,
        url: Url,
        stage: PageVmInitStage,
    ) -> Result<PageVmFollowNavigationTurnOutcome> {
        let source = crate::javascript_url::decoded_source(&url);
        tracing::debug!(
            stage = ?stage,
            %url,
            source_len = source.len(),
            "executing pending javascript location navigation"
        );
        self.vm_mut()
            .restore_top_level_location_runtime_state(&initiator_url);
        if self.vm().script_execution_disabled() {
            return Ok(PageVmFollowNavigationTurnOutcome::Completed);
        }
        let replacement_html = self
            .vm_mut()
            .eval_javascript_url_runtime_turn(&source, &initiator_url)?;
        // Chromium suppresses a string completion when the script synchronously
        // starts a normal navigation or submits a form. That newer navigation
        // owns the target Document and must win over javascript: replacement.
        if self.vm().has_pending_location_navigation() {
            return Ok(PageVmFollowNavigationTurnOutcome::TriggeredNavigation { stage });
        }
        if let Some(replacement_html) = replacement_html {
            self.document_lifecycle.set_next_document_open_start_reason(
                RendererLifecycleStartReason::JavascriptDocumentReplacement,
            );
            let replacement_html = serde_json::to_string(&replacement_html)?;
            let execution = self.vm_mut().exec_runtime_turn(
                &format!("document.open(); document.write({replacement_html}); document.close();"),
                Some(&url),
            );
            self.document_lifecycle.set_next_document_open_start_reason(
                RendererLifecycleStartReason::ExplicitDocumentOpen,
            );
            execution?;
        }
        if self.vm().has_pending_location_navigation() {
            Ok(PageVmFollowNavigationTurnOutcome::TriggeredNavigation { stage })
        } else {
            Ok(PageVmFollowNavigationTurnOutcome::Completed)
        }
    }

    fn followed_location_navigation_env(&self) -> PageVmEnvConfig {
        PageVmEnvConfig {
            main_document_commit: None,
            web_storage: self.vm().web_storage_handles(),
            document_start_scripts: self.document_start_scripts.clone(),
            runtime_bindings: self.runtime_bindings.clone(),
            runtime_inspector_session_restore_snapshots: self
                .runtime_inspector_session_restore_snapshots(),
            runtime_isolated_worlds: self.runtime_isolated_worlds.clone(),
            permission_overrides: self.permission_overrides.clone(),
            extra_http_headers: self.extra_http_headers.clone(),
            navigator_identity: self.navigator_identity.clone(),
            document_policy_container: crate::document_runtime::DocumentPolicyContainer {
                document_content_security_policies: self.vm().document_content_security_policies(),
                ..Default::default()
            },
            document_default_language: None,
            document_last_modified: None,
            locale_override: self.locale_override.clone(),
            timezone_override: self.timezone_override.clone(),
            script_execution_disabled: self.script_execution_disabled(),
            bypass_content_security_policy: self.bypass_content_security_policy,
            cpu_throttling_rate: self.cpu_throttling_rate,
            emulated_media: self.emulated_media.clone(),
            idle_override: self.idle_override,
            viewport_surface: self.viewport_surface,
            network_offline: self.network_offline,
            blocked_url_patterns: self.blocked_url_patterns.clone(),
            indexed_db_manager: self.indexed_db_manager.clone(),
            storage_bucket_store: self.storage_bucket_store.clone(),
            fetch_subresource_interception_enabled: self.fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type: self
                .fetch_subresource_interception_resource_type,
            layout_policy: self.layout_policy,
            wpt_extensions_enabled: self.wpt_extensions_enabled,
            root_frame_id: self.vm().root_frame_id().map(str::to_owned),
            top_level_storage_key: None,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
        }
    }

    #[cfg(test)]
    async fn bootstrap_followed_location_navigation(
        &mut self,
        loaded: LoadedFollowedLocationNavigation,
        navigation_bootstrap_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
        reserved_service_worker_client_id: Option<
            crate::service_worker_runtime::ServiceWorkerClientId,
        >,
        stage: PageVmInitStage,
        boundary: FollowedLocationNavigationBootstrapBoundary,
    ) -> Result<PageVmFollowedNavigationBuildOutcome> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "async followed location-navigation rebuild must execute on the matching named owner lane"
        );
        debug_assert!(matches!(
            &loaded,
            LoadedFollowedLocationNavigation::StreamingDocument { .. }
                | LoadedFollowedLocationNavigation::ExternalDocument { .. }
        ));
        let env = self.followed_location_navigation_env();
        let runtime_hooks = self.runtime_hooks.clone().for_cross_document_commit();
        if runtime_hooks.has_renderer_page_script_environment() {
            self.commit_main_window_proxy_navigation()?;
        }
        bootstrap_committed_followed_location_navigation(
            self.page_id,
            self.local_executor.clone(),
            self.request_client.clone(),
            env,
            runtime_hooks,
            navigation_bootstrap_entry,
            reserved_service_worker_client_id,
            stage,
            loaded,
            boundary,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{about_blank_navigation_response, build_followed_location_navigation_request};
    use moli_fetch::{BrowserNavigationRequestKind, outgoing_request_headers};
    use url::Url;

    #[test]
    fn followed_location_navigation_request_uses_document_url_as_initiator() {
        let initiator_url =
            Url::parse("https://example.com/path/index.html?from=challenge").unwrap();
        let target_url = Url::parse("https://example.com/path/index.html?from=challenge").unwrap();

        let request = build_followed_location_navigation_request(
            &initiator_url,
            &target_url,
            "GET",
            None,
            Vec::new(),
            BrowserNavigationRequestKind::Navigate,
        )
        .unwrap();
        let headers = outgoing_request_headers(&Default::default(), &request, None);

        assert_eq!(
            request.cookie_context.initiator_url.as_ref(),
            Some(&initiator_url)
        );
        assert_eq!(
            moli_fetch::Request::get(target_url.as_str())
                .unwrap()
                .cookie_context
                .initiator_url,
            None
        );
        assert_eq!(
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("referer"))
                .map(|(_, value)| value.as_str()),
            Some(initiator_url.as_str())
        );
        assert_eq!(
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("sec-fetch-site"))
                .map(|(_, value)| value.as_str()),
            Some("same-origin")
        );
    }

    #[test]
    fn followed_location_reload_preserves_request_kind_and_browser_headers() {
        let initiator_url = Url::parse("https://example.com/current").unwrap();
        let target_url = initiator_url.clone();

        let request = build_followed_location_navigation_request(
            &initiator_url,
            &target_url,
            "GET",
            None,
            Vec::new(),
            BrowserNavigationRequestKind::Reload,
        )
        .unwrap();
        let headers = outgoing_request_headers(&Default::default(), &request, None);

        assert_eq!(
            request.browser_navigation_kind(),
            BrowserNavigationRequestKind::Reload
        );
        assert_eq!(
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
                .map(|(_, value)| value.as_str()),
            Some("max-age=0")
        );
    }

    #[test]
    fn followed_location_navigation_preserves_post_request() {
        let initiator_url = Url::parse("https://example.com/form").unwrap();
        let target_url = Url::parse("https://example.com/submit").unwrap();
        let body = b"answer=42".to_vec();

        let request = build_followed_location_navigation_request(
            &initiator_url,
            &target_url,
            "POST",
            Some(body.clone()),
            vec![(
                "Content-Type".to_owned(),
                "application/x-www-form-urlencoded".to_owned(),
            )],
            BrowserNavigationRequestKind::Navigate,
        )
        .unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.body, Some(body));
        assert_eq!(
            request.request_headers,
            vec![(
                "Content-Type".to_owned(),
                "application/x-www-form-urlencoded".to_owned(),
            )]
        );
    }

    #[test]
    fn about_blank_followed_location_navigation_uses_synthetic_response() {
        let url = Url::parse("about:blank#fragment").unwrap();
        let response = about_blank_navigation_response(&url).unwrap();

        assert_eq!(response.head().final_url.as_str(), "about:blank#fragment");
        assert_eq!(response.head().status, 200);
        assert_eq!(
            response
                .head()
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.as_str()),
            Some("text/html; charset=utf-8")
        );
    }
}
