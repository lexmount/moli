use super::super::script_preloads::{ServiceWorkerScriptPreloadContext, admit_pending_preloads};
#[cfg(test)]
use super::parser_blocking_pending::main_parser_blocking_classic_script_item;
use super::scaffold::continue_phase_one_until_streaming_boundary_on_execution_context;
use super::*;
use crate::document_runtime::{
    response_content_security_policies_from_headers,
    response_content_security_report_only_policies_from_headers,
};
use crate::referrer_policy::response_referrer_policy_from_headers;
use moli_encoding::HtmlDocumentStreamingDecoder;
use moli_web_mime::response_headers_indicate_attachment_download;
use tokio::sync::{mpsc, oneshot};

pub(in crate::runtime) struct StreamingHtmlPageCreationResult {
    pub(in crate::runtime) response_status: u16,
    pub(in crate::runtime) response_headers: Vec<(String, String)>,
    pub(in crate::runtime) outcome: ParseTimePageVmCreationOutcome,
}

pub(in crate::runtime) enum StreamingNavigationPageCreationResult {
    Html(Box<StreamingHtmlPageCreationResult>),
    Download(RendererPendingDownloadActivation),
}

#[derive(Clone, Copy)]
enum CommittedNavigationBootstrapBoundary {
    ContinuePhaseOne,
    DocumentCommit,
}

impl ConcurrentParseTimeRuntime {
    #[cfg(test)]
    pub(in crate::runtime) async fn finish_creation_from_committed_streaming_navigation_response(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        stage: PageVmInitStage,
        started: Instant,
        response: Box<StreamingRawResponse>,
    ) -> Result<StreamingNavigationPageCreationResult> {
        Self::create_from_committed_streaming_navigation_response(
            page_id,
            local_executor,
            loader,
            env,
            runtime_hooks,
            stage,
            started,
            response,
            CommittedNavigationBootstrapBoundary::ContinuePhaseOne,
        )
        .await
    }

    pub(in crate::runtime) async fn prepare_document_from_committed_streaming_navigation_response(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        stage: PageVmInitStage,
        started: Instant,
        response: Box<StreamingRawResponse>,
    ) -> Result<StreamingNavigationPageCreationResult> {
        Self::create_from_committed_streaming_navigation_response(
            page_id,
            local_executor,
            loader,
            env,
            runtime_hooks,
            stage,
            started,
            response,
            CommittedNavigationBootstrapBoundary::DocumentCommit,
        )
        .await
    }

    async fn create_from_committed_streaming_navigation_response(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        stage: PageVmInitStage,
        started: Instant,
        response: Box<StreamingRawResponse>,
        boundary: CommittedNavigationBootstrapBoundary,
    ) -> Result<StreamingNavigationPageCreationResult> {
        let response_status = response.status;
        let response_headers = response.headers.clone();
        debug_assert!(!response_headers_indicate_download(&response_headers));
        let response_final_url = response.final_url.clone();
        let mut env = env.clone();
        env.document_content_security_policies = if env.bypass_content_security_policy {
            Vec::new()
        } else {
            crate::content_security_policy::content_security_policy_headers(&response_headers)
        };

        let mut body_source = RawDocumentBodySource::fetch_response(response);
        let mut env = env.clone();
        env.response_content_security_policies = if env.bypass_content_security_policy {
            Vec::new()
        } else {
            response_content_security_policies_from_headers(&response_headers)
        };
        env.response_content_security_report_only_policies = if env.bypass_content_security_policy {
            Vec::new()
        } else {
            response_content_security_report_only_policies_from_headers(&response_headers)
        };
        env.response_referrer_policy = response_referrer_policy_from_headers(&response_headers);
        env.document_default_language =
            crate::document_language::document_default_language_from_headers(&response_headers);
        env.document_last_modified =
            crate::document_last_modified::document_last_modified_from_headers(&response_headers);
        env.content_security_reporting_endpoints = if env.bypass_content_security_policy {
            Default::default()
        } else {
            crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
                &response_headers,
                &response_final_url,
            )
        };
        env.cross_origin_embedder_policy =
            crate::cross_origin_isolation::cross_origin_embedder_policy_from_headers(
                &response_headers,
            );
        env.document_isolation_policy =
            crate::cross_origin_isolation::document_isolation_policy_from_headers(
                &response_headers,
            );
        env.cross_origin_isolated =
            crate::cross_origin_isolation::response_headers_enable_cross_origin_isolation(
                &response_final_url,
                &response_headers,
            );
        let mut state = ParseTimeDriverState::new(response_final_url);
        state
            .buffered_document_preloads
            .set_script_fetch_interception_enabled(
                env.fetch_subresource_interception_enabled
                    && env.fetch_subresource_interception_resource_type.is_none_or(
                        |resource_type| {
                            resource_type.has_same_cdp_fetch_interception_type(
                                crate::types::SubresourceResourceType::Script,
                            )
                        },
                    ),
            );
        state
            .buffered_document_preloads
            .set_response_csp_requires_parser_admission(
                !env.response_content_security_policies.is_empty(),
            );
        state.buffered_document_preloads.bind_resource_runtime(
            runtime_hooks.owner_wake(),
            runtime_hooks.resource_task_runner(),
        );
        let service_worker_preload_context =
            env.reserved_service_worker_client_id.map(|client_id| {
                ServiceWorkerScriptPreloadContext::new(
                    runtime_hooks.browser_context_runtime.clone(),
                    client_id,
                    state.final_url.clone(),
                    runtime_hooks.owner_wake(),
                )
            });
        state.service_worker_preload_context = service_worker_preload_context.clone();
        let mut decoder = HtmlDocumentStreamingDecoder::new(&response_headers);
        // Raw navigation bodies are decoded during prebootstrap scan. The decoder
        // is carried forward so split multibyte sequences are not decoded twice or
        // lost between the scan and parser handoff.
        let prebootstrap_chunks = prebootstrap_scan_ready_streaming_raw_chunks(
            &mut state,
            &mut body_source,
            &mut decoder,
            loader,
            service_worker_preload_context.as_ref(),
        );
        let bootstrap_outcome = Self::start_creation_from_streaming_html_bootstrap_with_state(
            page_id,
            local_executor,
            loader,
            &env,
            runtime_hooks,
            state,
            stage,
            started,
        )
        .await?;
        let outcome = settle_streaming_raw_response_at_boundary(
            bootstrap_outcome,
            body_source,
            prebootstrap_chunks,
            decoder,
            service_worker_preload_context,
            started,
            boundary,
        )
        .await?;
        Ok(StreamingNavigationPageCreationResult::Html(Box::new(
            StreamingHtmlPageCreationResult {
                response_status,
                response_headers,
                outcome,
            },
        )))
    }

    pub(in crate::runtime) async fn create_external_raw_document_response_at_reply_boundary(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        stage: PageVmInitStage,
        started: Instant,
        final_url: Url,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
        reply_boundary: crate::RendererReplyBoundary,
    ) -> Result<StreamingNavigationPageCreationResult> {
        if response_headers_indicate_download(&response_headers) {
            let mut body_source = RawDocumentBodySource::External(raw_body);
            let response_body = collect_streaming_raw_body(&mut body_source).await?;
            return Ok(StreamingNavigationPageCreationResult::Download(
                RendererPendingDownloadActivation {
                    url: final_url.as_str().to_owned(),
                    suggested_filename: None,
                    response: Some(RendererPendingDownloadResponse {
                        final_url: final_url.as_str().to_owned(),
                        status: response_status,
                        headers: response_headers,
                        body: response_body,
                    }),
                },
            ));
        }

        let bootstrap_boundary = if reply_boundary.waits_for_stage() {
            CommittedNavigationBootstrapBoundary::ContinuePhaseOne
        } else {
            CommittedNavigationBootstrapBoundary::DocumentCommit
        };
        Self::create_from_committed_external_raw_document_response(
            page_id,
            local_executor,
            loader,
            env,
            runtime_hooks,
            stage,
            started,
            final_url,
            response_status,
            response_headers,
            raw_body,
            bootstrap_boundary,
        )
        .await
    }

    #[cfg(test)]
    pub(in crate::runtime) async fn finish_creation_from_committed_external_raw_document_response(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        stage: PageVmInitStage,
        started: Instant,
        final_url: Url,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
    ) -> Result<StreamingNavigationPageCreationResult> {
        Self::create_from_committed_external_raw_document_response(
            page_id,
            local_executor,
            loader,
            env,
            runtime_hooks,
            stage,
            started,
            final_url,
            response_status,
            response_headers,
            raw_body,
            CommittedNavigationBootstrapBoundary::ContinuePhaseOne,
        )
        .await
    }

    pub(in crate::runtime) async fn prepare_document_from_committed_external_raw_document_response(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        stage: PageVmInitStage,
        started: Instant,
        final_url: Url,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
    ) -> Result<StreamingNavigationPageCreationResult> {
        Self::create_from_committed_external_raw_document_response(
            page_id,
            local_executor,
            loader,
            env,
            runtime_hooks,
            stage,
            started,
            final_url,
            response_status,
            response_headers,
            raw_body,
            CommittedNavigationBootstrapBoundary::DocumentCommit,
        )
        .await
    }

    async fn create_from_committed_external_raw_document_response(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        stage: PageVmInitStage,
        started: Instant,
        final_url: Url,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
        boundary: CommittedNavigationBootstrapBoundary,
    ) -> Result<StreamingNavigationPageCreationResult> {
        debug_assert!(!response_headers_indicate_download(&response_headers));
        let mut env = env.clone();
        env.document_content_security_policies = if env.bypass_content_security_policy {
            Vec::new()
        } else {
            crate::content_security_policy::content_security_policy_headers(&response_headers)
        };

        let mut body_source = RawDocumentBodySource::External(raw_body);
        let mut env = env.clone();
        env.response_content_security_policies = if env.bypass_content_security_policy {
            Vec::new()
        } else {
            response_content_security_policies_from_headers(&response_headers)
        };
        env.response_content_security_report_only_policies = if env.bypass_content_security_policy {
            Vec::new()
        } else {
            response_content_security_report_only_policies_from_headers(&response_headers)
        };
        env.response_referrer_policy = response_referrer_policy_from_headers(&response_headers);
        env.document_default_language =
            crate::document_language::document_default_language_from_headers(&response_headers);
        env.document_last_modified =
            crate::document_last_modified::document_last_modified_from_headers(&response_headers);
        env.content_security_reporting_endpoints = if env.bypass_content_security_policy {
            Default::default()
        } else {
            crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
                &response_headers,
                &final_url,
            )
        };
        env.cross_origin_embedder_policy =
            crate::cross_origin_isolation::cross_origin_embedder_policy_from_headers(
                &response_headers,
            );
        env.document_isolation_policy =
            crate::cross_origin_isolation::document_isolation_policy_from_headers(
                &response_headers,
            );
        env.cross_origin_isolated =
            crate::cross_origin_isolation::response_headers_enable_cross_origin_isolation(
                &final_url,
                &response_headers,
            );
        if response_headers_indicate_xml_document(&response_headers) {
            let body = collect_streaming_raw_body(&mut body_source).await?;
            let source = String::from_utf8_lossy(&body).into_owned();
            let content_type = moli_web_mime::response_document_content_type(&response_headers)
                .unwrap_or_else(|| "application/xml".to_owned());
            let outcome = Self::finish_creation_from_xml_bootstrap(
                page_id,
                local_executor,
                loader,
                &env,
                runtime_hooks,
                final_url,
                content_type,
                stage,
                source,
                started,
            )
            .await?;
            return Ok(StreamingNavigationPageCreationResult::Html(Box::new(
                StreamingHtmlPageCreationResult {
                    response_status,
                    response_headers,
                    outcome,
                },
            )));
        }
        let mut state = ParseTimeDriverState::new(final_url);
        state
            .buffered_document_preloads
            .set_script_fetch_interception_enabled(
                env.fetch_subresource_interception_enabled
                    && env.fetch_subresource_interception_resource_type.is_none_or(
                        |resource_type| {
                            resource_type.has_same_cdp_fetch_interception_type(
                                crate::types::SubresourceResourceType::Script,
                            )
                        },
                    ),
            );
        state
            .buffered_document_preloads
            .set_response_csp_requires_parser_admission(
                !env.response_content_security_policies.is_empty(),
            );
        state.buffered_document_preloads.bind_resource_runtime(
            runtime_hooks.owner_wake(),
            runtime_hooks.resource_task_runner(),
        );
        let service_worker_preload_context =
            env.reserved_service_worker_client_id.map(|client_id| {
                ServiceWorkerScriptPreloadContext::new(
                    runtime_hooks.browser_context_runtime.clone(),
                    client_id,
                    state.final_url.clone(),
                    runtime_hooks.owner_wake(),
                )
            });
        state.service_worker_preload_context = service_worker_preload_context.clone();
        let mut decoder = HtmlDocumentStreamingDecoder::new(&response_headers);
        // External raw bodies replay captured chunks. Pre-scan only what is already
        // buffered before bootstrap so the producer's backpressure boundary still
        // controls how far ahead the parser can get.
        let prebootstrap_chunks = prebootstrap_scan_ready_streaming_raw_chunks(
            &mut state,
            &mut body_source,
            &mut decoder,
            loader,
            service_worker_preload_context.as_ref(),
        );
        let bootstrap_outcome = Self::start_creation_from_streaming_html_bootstrap_with_state(
            page_id,
            local_executor,
            loader,
            &env,
            runtime_hooks,
            state,
            stage,
            started,
        )
        .await?;
        let outcome = settle_streaming_raw_response_at_boundary(
            bootstrap_outcome,
            body_source,
            prebootstrap_chunks,
            decoder,
            service_worker_preload_context,
            started,
            boundary,
        )
        .await?;
        Ok(StreamingNavigationPageCreationResult::Html(Box::new(
            StreamingHtmlPageCreationResult {
                response_status,
                response_headers,
                outcome,
            },
        )))
    }

    async fn start_creation_from_streaming_html_bootstrap_with_state(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        state: ParseTimeDriverState,
        stage: PageVmInitStage,
        started: Instant,
    ) -> Result<ParseTimePageVmStreamingBootstrapOutcome> {
        let (state, page_vm, triggered_navigation) =
            Self::bootstrap_page_vm_from_state_on_fresh_local_task(
                page_id,
                local_executor.clone(),
                loader.clone(),
                env.clone(),
                runtime_hooks,
                state,
                started,
                "streaming html bootstrap local task channel closed",
            )
            .await?;
        if triggered_navigation {
            return Ok(
                ParseTimePageVmStreamingBootstrapOutcome::TriggeredNavigation {
                    page_vm: Box::new(page_vm),
                    stage,
                },
            );
        }
        Ok(ParseTimePageVmStreamingBootstrapOutcome::Runtime(Box::new(
            Self::new_parser_owner(loader.clone(), stage, state, page_vm),
        )))
    }

    pub(super) fn enqueue_streaming_html_chunk(&mut self, chunk: String) {
        self.enqueue_streaming_html_chunk_with_service_worker_context(chunk, None);
    }

    pub(super) fn enqueue_streaming_html_chunk_with_service_worker_context(
        &mut self,
        chunk: String,
        service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
    ) {
        self.state
            .buffered_document_preloads
            .append_to_main_document_scan_with_service_worker_context(
                &self.state.final_url,
                &chunk,
                &self.loader,
                service_worker_context,
            );
        admit_pending_preloads(
            &mut self.page_vm,
            &mut self.state.buffered_document_preloads,
            &self.loader,
            service_worker_context,
        );
        self.state.parser_session.queue_arrived_chunk(chunk);
    }

    pub(super) fn close_streaming_html_input(&mut self) {
        self.state.input_closed = true;
    }

    pub(super) async fn continue_streaming_creation_on_execution_context(
        self,
        started: Instant,
    ) -> Result<ParseTimePageVmStreamingProgress> {
        continue_phase_one_until_streaming_boundary_on_execution_context(self, started).await
    }
}

fn response_headers_indicate_xml_document(headers: &[(String, String)]) -> bool {
    moli_web_mime::response_document_content_type(headers)
        .is_some_and(|mime| moli_web_mime::is_dom_parser_xml_mime(&mime))
}

const EXTERNAL_RAW_DOCUMENT_BODY_BUFFERED_CHUNKS: usize = 8;

pub struct ExternalRawDocumentBodyStream {
    body_chunks: mpsc::Receiver<Vec<u8>>,
    completion: Option<oneshot::Receiver<Result<()>>>,
}

impl ExternalRawDocumentBodyStream {
    /// Creates a bounded external body stream.
    ///
    /// The producer may be a small fixed number of chunks ahead of the parser. This is
    /// the intended backpressure boundary for replaying captured/spooled raw
    /// bodies without relying on cooperative task yields.
    pub fn channel(completion: oneshot::Receiver<Result<()>>) -> (mpsc::Sender<Vec<u8>>, Self) {
        let (body_tx, body_chunks) = mpsc::channel(EXTERNAL_RAW_DOCUMENT_BODY_BUFFERED_CHUNKS);
        (body_tx, Self::new(body_chunks, completion))
    }

    pub fn new(
        body_chunks: mpsc::Receiver<Vec<u8>>,
        completion: oneshot::Receiver<Result<()>>,
    ) -> Self {
        Self {
            body_chunks,
            completion: Some(completion),
        }
    }

    pub fn from_bytes(body: Vec<u8>) -> Self {
        let (completion_tx, completion_rx) = oneshot::channel();
        let (body_tx, body_stream) = Self::channel(completion_rx);
        let _ = body_tx.try_send(body);
        drop(body_tx);
        let _ = completion_tx.send(Ok(()));
        body_stream
    }

    fn body_chunk_stream_is_exhausted(&self) -> bool {
        self.body_chunks.is_closed() && self.body_chunks.is_empty()
    }
}

pub(super) enum RawDocumentBodySource {
    FetchResponse(Box<StreamingRawResponse>),
    External(ExternalRawDocumentBodyStream),
}

impl RawDocumentBodySource {
    fn fetch_response(response: Box<StreamingRawResponse>) -> Self {
        Self::FetchResponse(response)
    }

    pub(super) async fn next_chunk(&mut self) -> Option<Vec<u8>> {
        match self {
            Self::FetchResponse(response) => response.next_chunk().await,
            Self::External(source) => source.body_chunks.recv().await,
        }
    }

    pub(super) fn try_next_chunk(&mut self) -> Option<Vec<u8>> {
        // Nonblocking polls are used only to coalesce work that is already ready.
        // They must not wait, or the streaming backpressure boundary would move
        // from the response source into the parser loop.
        match self {
            Self::FetchResponse(response) => response.try_next_chunk(),
            Self::External(source) => source.body_chunks.try_recv().ok(),
        }
    }

    fn body_chunk_stream_is_exhausted(&self) -> bool {
        match self {
            Self::FetchResponse(response) => response.body_chunk_stream_is_exhausted(),
            Self::External(source) => source.body_chunk_stream_is_exhausted(),
        }
    }

    pub(super) async fn finish(&mut self) -> Result<()> {
        match self {
            Self::FetchResponse(response) => response.finish().await,
            Self::External(source) => source
                .completion
                .take()
                .expect("external raw document body completion should only be awaited once")
                .await
                .map_err(|_| anyhow!("external raw document body completion channel closed"))?,
        }
    }
}

fn enqueue_prebootstrap_html_chunk(runtime: &mut ConcurrentParseTimeRuntime, chunk: String) {
    runtime.state.parser_session.queue_arrived_chunk(chunk);
}

fn scan_prebootstrap_html_chunk_into_state(
    state: &mut ParseTimeDriverState,
    loader: &ResourceRequestClient,
    chunk: &str,
    service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
) {
    state
        .buffered_document_preloads
        .append_to_main_document_prebootstrap_scan_with_service_worker_context(
            &state.final_url,
            chunk,
            loader,
            service_worker_context,
        );
}

pub(super) fn enqueue_streaming_raw_chunk(
    runtime: &mut ConcurrentParseTimeRuntime,
    decoder: &mut HtmlDocumentStreamingDecoder,
    chunk: Vec<u8>,
    service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
) {
    let text_chunks = decoder.push(&chunk);
    sync_document_character_set_from_decoder(runtime, decoder);
    for text_chunk in text_chunks {
        runtime.enqueue_streaming_html_chunk_with_service_worker_context(
            text_chunk,
            service_worker_context,
        );
    }
}

fn scan_prebootstrap_raw_chunk_into_state(
    state: &mut ParseTimeDriverState,
    decoder: &mut HtmlDocumentStreamingDecoder,
    loader: &ResourceRequestClient,
    chunk: Vec<u8>,
    service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
) -> Vec<String> {
    let mut text_chunks = Vec::new();
    let decoded_chunks = decoder.push(&chunk);
    sync_state_document_character_set_from_decoder(state, decoder);
    for text_chunk in decoded_chunks {
        scan_prebootstrap_html_chunk_into_state(state, loader, &text_chunk, service_worker_context);
        text_chunks.push(text_chunk);
    }
    text_chunks
}

fn preload_scan_ready_streaming_raw_chunks(
    runtime: &mut ConcurrentParseTimeRuntime,
    response: &mut RawDocumentBodySource,
    decoder: &mut HtmlDocumentStreamingDecoder,
    service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
) {
    // Chromium keeps parser-visible tree construction paused while allowing the
    // preload scanner to see input after a parser-blocking script. This helper
    // only decodes ready bytes, queues parser input, and feeds discovery; it
    // must not pump the tokenizer or execute script.
    while let Some(chunk) = response.try_next_chunk() {
        enqueue_streaming_raw_chunk(runtime, decoder, chunk, service_worker_context);
    }
}

fn prebootstrap_scan_ready_streaming_raw_chunks(
    state: &mut ParseTimeDriverState,
    response: &mut RawDocumentBodySource,
    decoder: &mut HtmlDocumentStreamingDecoder,
    loader: &ResourceRequestClient,
    service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
) -> Vec<String> {
    // Keep decoded text chunks paired with the decoder state so prebootstrap scan
    // does not change parser-visible bytes, even around split UTF-8 boundaries.
    let mut text_chunks = Vec::new();
    while let Some(chunk) = response.try_next_chunk() {
        text_chunks.extend(scan_prebootstrap_raw_chunk_into_state(
            state,
            decoder,
            loader,
            chunk,
            service_worker_context,
        ));
    }
    text_chunks
}

async fn collect_streaming_raw_body(response: &mut RawDocumentBodySource) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.next_chunk().await {
        body.extend_from_slice(&chunk);
    }
    response.finish().await?;
    Ok(body)
}

async fn settle_streaming_raw_response_at_boundary(
    bootstrap_outcome: ParseTimePageVmStreamingBootstrapOutcome,
    response: RawDocumentBodySource,
    prebootstrap_chunks: Vec<String>,
    decoder: HtmlDocumentStreamingDecoder,
    service_worker_context: Option<ServiceWorkerScriptPreloadContext>,
    started: Instant,
    boundary: CommittedNavigationBootstrapBoundary,
) -> Result<ParseTimePageVmCreationOutcome> {
    match boundary {
        CommittedNavigationBootstrapBoundary::ContinuePhaseOne => {
            drive_streaming_raw_response_through_phase_one(
                bootstrap_outcome,
                response,
                prebootstrap_chunks,
                decoder,
                service_worker_context,
                started,
            )
            .await
        }
        CommittedNavigationBootstrapBoundary::DocumentCommit => {
            park_streaming_raw_response_at_document_commit(
                bootstrap_outcome,
                response,
                prebootstrap_chunks,
                decoder,
                service_worker_context,
                started,
            )
        }
    }
}

fn park_streaming_raw_response_at_document_commit(
    bootstrap_outcome: ParseTimePageVmStreamingBootstrapOutcome,
    response: RawDocumentBodySource,
    prebootstrap_chunks: Vec<String>,
    decoder: HtmlDocumentStreamingDecoder,
    service_worker_context: Option<ServiceWorkerScriptPreloadContext>,
    started: Instant,
) -> Result<ParseTimePageVmCreationOutcome> {
    let mut runtime = match bootstrap_outcome {
        ParseTimePageVmStreamingBootstrapOutcome::TriggeredNavigation { page_vm, stage } => {
            return Ok(ParseTimePageVmCreationOutcome::TriggeredNavigation {
                page_vm: *page_vm,
                stage,
            });
        }
        ParseTimePageVmStreamingBootstrapOutcome::Runtime(runtime) => *runtime,
    };
    sync_document_character_set_from_decoder(&mut runtime, &decoder);
    let has_prebootstrap_input = !prebootstrap_chunks.is_empty();
    for chunk in prebootstrap_chunks {
        enqueue_prebootstrap_html_chunk(&mut runtime, chunk);
    }
    if has_prebootstrap_input
        && !runtime
            .page_vm()
            .vm()
            .document_runtime
            .request_main_parser_continuation_if_active()
    {
        return Err(anyhow!(
            "buffered prebootstrap input lost its active parser continuation"
        ));
    }
    let continuation = PendingStreamingPhaseOneContinuation::bridge(
        runtime,
        response,
        decoder,
        service_worker_context,
        started,
    )?;
    Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(
        PendingPhaseOneResidence::open_streaming(Box::new(continuation)),
    ))
}

async fn drive_streaming_raw_response_through_phase_one(
    bootstrap_outcome: ParseTimePageVmStreamingBootstrapOutcome,
    mut response: RawDocumentBodySource,
    prebootstrap_chunks: Vec<String>,
    mut decoder: HtmlDocumentStreamingDecoder,
    service_worker_context: Option<ServiceWorkerScriptPreloadContext>,
    started: Instant,
) -> Result<ParseTimePageVmCreationOutcome> {
    let mut runtime = match bootstrap_outcome {
        ParseTimePageVmStreamingBootstrapOutcome::TriggeredNavigation { page_vm, stage } => {
            return Ok(ParseTimePageVmCreationOutcome::TriggeredNavigation {
                page_vm: *page_vm,
                stage,
            });
        }
        ParseTimePageVmStreamingBootstrapOutcome::Runtime(runtime) => *runtime,
    };
    sync_document_character_set_from_decoder(&mut runtime, &decoder);
    // Prebootstrap scanning deliberately does not mutate parser input. Feed
    // the saved text only after the PageVm exists, then coalesce bytes that
    // are already buffered without waiting for the network producer.
    for chunk in prebootstrap_chunks {
        enqueue_prebootstrap_html_chunk(&mut runtime, chunk);
    }
    preload_scan_ready_streaming_raw_chunks(
        &mut runtime,
        &mut response,
        &mut decoder,
        service_worker_context.as_ref(),
    );
    let input_finished = response.body_chunk_stream_is_exhausted();
    if input_finished {
        response.finish().await?;
        if let Some(tail) = decoder.finish() {
            runtime.enqueue_streaming_html_chunk(tail);
        }
        sync_document_character_set_from_decoder(&mut runtime, &decoder);
        runtime.close_streaming_html_input();
    }
    match runtime
        .continue_streaming_creation_on_execution_context(started)
        .await?
    {
        ParseTimePageVmStreamingProgress::NeedMoreInput(next_runtime) if !input_finished => {
            let continuation = PendingStreamingPhaseOneContinuation::bridge(
                *next_runtime,
                response,
                decoder,
                service_worker_context,
                started,
            )?;
            Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(
                PendingPhaseOneResidence::open_streaming(Box::new(continuation)),
            ))
        }
        ParseTimePageVmStreamingProgress::PendingPageTask(next_runtime) if !input_finished => {
            let continuation = PendingStreamingPhaseOneContinuation::bridge(
                *next_runtime,
                response,
                decoder,
                service_worker_context,
                started,
            )?;
            Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(
                PendingPhaseOneResidence::open_streaming(Box::new(continuation)),
            ))
        }
        ParseTimePageVmStreamingProgress::NeedMoreInput(next_runtime)
            if next_runtime.has_pending_parser_blocking_source_load() =>
        {
            Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(
                PendingPhaseOneResidence::parser_blocking_source_load(next_runtime, started),
            ))
        }
        ParseTimePageVmStreamingProgress::PendingPageTask(next_runtime) => {
            Ok(ParseTimePageVmCreationOutcome::PendingPhaseOne(
                PendingPhaseOneResidence::closed_input_page_work(next_runtime, started),
            ))
        }
        ParseTimePageVmStreamingProgress::NeedMoreInput(_) => Err(anyhow!(
            "closed streaming html input should not stall waiting for more input"
        )),
        ParseTimePageVmStreamingProgress::TriggeredNavigation { page_vm, stage } => {
            Ok(ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, stage })
        }
        ParseTimePageVmStreamingProgress::ContinuePhaseTwo {
            page_vm,
            page_tasks,
            stage,
            started,
        } => Ok(ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
            page_vm,
            page_tasks,
            stage,
            started,
        }),
    }
}

pub(super) fn sync_document_character_set_from_decoder(
    runtime: &mut ConcurrentParseTimeRuntime,
    decoder: &HtmlDocumentStreamingDecoder,
) {
    let encoding = decoder.document_encoding_name();
    runtime.state.document_character_set = encoding.to_owned();
    runtime
        .state
        .buffered_document_preloads
        .set_document_character_set(encoding);
    runtime.page_vm.set_document_character_set(encoding);
}

fn sync_state_document_character_set_from_decoder(
    state: &mut ParseTimeDriverState,
    decoder: &HtmlDocumentStreamingDecoder,
) {
    let encoding = decoder.document_encoding_name();
    state.document_character_set = encoding.to_owned();
    state
        .buffered_document_preloads
        .set_document_character_set(encoding);
}

pub(in crate::runtime) fn response_headers_indicate_download(headers: &[(String, String)]) -> bool {
    response_headers_indicate_attachment_download(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ScriptSource;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn external_raw_document_body_source_collects_chunks_and_completion() {
        let (completion_tx, completion_rx) = oneshot::channel();
        let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
        let producer = tokio::spawn(async move {
            body_tx.send(b"hel".to_vec()).await.unwrap();
            body_tx.send(b"lo".to_vec()).await.unwrap();
            drop(body_tx);
            completion_tx.send(Ok(())).unwrap();
        });

        let mut source = RawDocumentBodySource::External(raw_body);
        let body = collect_streaming_raw_body(&mut source)
            .await
            .expect("external body source should collect");
        producer.await.expect("producer should finish");

        assert_eq!(body, b"hello");
    }

    fn default_test_page_vm_env_config() -> PageVmEnvConfig {
        PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
            root_frame_id: None,
            main_document_commit: None,
            top_level_storage_key: None,
            document_start_scripts: vec![],
            runtime_bindings: vec![],
            runtime_inspector_session_restore_snapshots: vec![],
            runtime_isolated_worlds: vec![],
            permission_overrides: vec![],
            extra_http_headers: vec![],
            document_content_security_policies: Vec::new(),
            response_content_security_policies: Vec::new(),
            response_content_security_report_only_policies: Vec::new(),
            response_referrer_policy: None,
            content_security_reporting_endpoints:
                crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
            cross_origin_embedder_policy: Default::default(),
            document_isolation_policy: Default::default(),
            cross_origin_isolated: false,
            document_default_language: None,
            document_last_modified: None,
            locale_override: None,
            timezone_override: None,
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            cpu_throttling_rate: 1.0,
            emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
            idle_override: None,
            viewport_surface: None,
            network_offline: false,
            blocked_url_patterns: Vec::new(),
            indexed_db_manager: None,
            storage_bucket_store: None,
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
            layout_policy: moli_page_types::LayoutPolicy::default(),
            wpt_extensions_enabled: false,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
        }
    }

    fn prepared_external_classic_for_streaming_test(url: &str) -> PreparedScript {
        let url = Url::parse(url).expect("test script url");
        PreparedScript {
            position: 0,
            node_id: NodeId::new(1),
            kind: crate::types::ScriptKind::Classic,
            mode: crate::types::ScriptMode::Normal,
            source_kind: crate::types::ScriptSourceKind::External,
            fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            initiator_url: url.clone(),
            base_url: url.clone(),
            url,
            host_script_handle: None,
        }
    }

    fn classic_preload_key_for_streaming_test(url: &str) -> BufferedScriptPreloadKey {
        BufferedScriptPreloadKey::new(
            Url::parse(url).expect("test preload url"),
            crate::types::ScriptKind::Classic,
            &crate::planning::ScriptFetchMetadata::default(),
        )
        .expect("classic scripts are preloadable")
    }

    fn native_dom_has_element_id(dom: &moli_dom::native::NativeDom, id: &str) -> bool {
        dom.nodes()
            .iter()
            .filter_map(moli_dom::native::Node::as_element)
            .any(|element| element.attribute("id") == Some(id))
    }

    async fn read_http_request_head(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
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
                return Ok(());
            }
        }
    }

    async fn spawn_single_script_server(body: &'static str) -> (Url, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test script server should bind");
        let addr = listener
            .local_addr()
            .expect("test script server should expose address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("test script server should accept one request");
            read_http_request_head(&mut stream)
                .await
                .expect("test script server should read request");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("test script server should write response");
        });
        (
            Url::parse(&format!("http://{addr}/later.js")).expect("test script url"),
            server,
        )
    }

    struct TestConcurrentParseTimeRuntime {
        runtime: ConcurrentParseTimeRuntime,
        _js_runtime_owner: crate::runtime::JsRuntimeOwner,
        _loader_owner: crate::network::ResourceRequestClientOwner,
    }

    impl std::ops::Deref for TestConcurrentParseTimeRuntime {
        type Target = ConcurrentParseTimeRuntime;

        fn deref(&self) -> &Self::Target {
            &self.runtime
        }
    }

    impl std::ops::DerefMut for TestConcurrentParseTimeRuntime {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.runtime
        }
    }

    fn streaming_runtime_with_pending_parser_blocking_source_load(
        load: crate::planning::SharedScriptSourceLoad,
    ) -> TestConcurrentParseTimeRuntime {
        let js_runtime_owner = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let loader = loader_owner.handle();
        let mut state = ParseTimeDriverState::new(final_url);
        let parser_dom_host = state
            .parser_session
            .stream_handle()
            .borrow_mut()
            .take_parser_stream_dom_host();
        let local_executor = JsLocalExecutor::new();
        let runtime_hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
        state.buffered_document_preloads.bind_resource_runtime(
            runtime_hooks.owner_wake(),
            runtime_hooks.resource_task_runner(),
        );
        let page_vm = PageVm::new(
            PageId::new_for_testing(1),
            local_executor,
            &loader,
            &default_test_page_vm_env_config(),
            runtime_hooks,
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");
        let script =
            prepared_external_classic_for_streaming_test("https://example.test/blocking.js");
        let parser_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser test owner");
        state.pending_parsing_blocking_script =
            PendingParsingBlockingClassicScriptRunner::from_parser_blocking_script(
                main_parser_blocking_classic_script_item(
                    parser_document_owner,
                    crate::parser_script::payload::ParserPreparedClassicScript::new(
                        crate::parser_script::payload::ParserClassicScriptMetadata::new(
                            script.node_id,
                            1,
                        ),
                        script,
                    ),
                    HashSet::new(),
                    Some(PendingParserBlockingSourceLoad::ReusablePreload(load)),
                ),
            );
        TestConcurrentParseTimeRuntime {
            runtime: ConcurrentParseTimeRuntime::new_parser_owner(
                loader,
                PageVmInitStage::Load,
                state,
                page_vm,
            ),
            _js_runtime_owner: js_runtime_owner,
            _loader_owner: loader_owner,
        }
    }

    #[tokio::test]
    async fn parser_deferred_source_terminal_remains_retained_until_queue_observation() {
        let blocking_load =
            crate::planning::SharedScriptSourceLoad::spawn_for_test(std::future::pending());
        let mut runtime = streaming_runtime_with_pending_parser_blocking_source_load(blocking_load);
        let (later_ready_tx, later_ready_rx) = oneshot::channel();
        let later_load = crate::planning::SharedScriptSourceLoad::spawn_for_test(async move {
            later_ready_rx
                .await
                .expect("test should release the parser-deferred source");
            Ok("window.later = true;".to_owned())
        });
        let later_load_observer = later_load.clone();
        runtime
            .page_vm
            .page_task_queue
            .extend_post_parse_work([PostParsePageOwnedWork::document_script_work_with_blocking_signatures(
                crate::document_script_scheduler::PageOwnedDocumentScriptWork::script_waiting_for_source(
                    crate::document_script_scheduler::DocumentScriptExecutionLane::ClassicDefer,
                prepared_external_classic_for_streaming_test("https://example.test/later.js"),
                later_load,
                ),
                HashSet::new(),
            )]);

        assert!(
            runtime
                .page_vm
                .page_task_queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_waiting_for_source_load),
            "parser-deferred work should remain source-blocked before its terminal arrives"
        );
        later_ready_tx
            .send(())
            .expect("parser-deferred source receiver should still be alive");
        let outcome = later_load_observer.wait_outcome().await;
        assert!(outcome.source_result.is_ok());
        assert!(
            runtime
                .page_vm
                .page_task_queue
                .complete_ready_source_loads()
        );
        assert!(
            !runtime
                .page_vm
                .page_task_queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_waiting_for_source_load),
            "the retained terminal must be observable when the parser owner later inspects its queue"
        );
    }

    #[tokio::test]
    async fn parser_blocking_source_loaded_prescans_ready_chunks_without_parser_progress() {
        let load = crate::planning::SharedScriptSourceLoad::ready_ok("window.blocking = true;");
        let mut runtime = streaming_runtime_with_pending_parser_blocking_source_load(load);
        let (completion_tx, completion_rx) = oneshot::channel();
        let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
        body_tx
            .try_send(
                br#"<script defer src="/later.js"></script><body><div id="after-blocking">after</div>"#
                    .to_vec(),
            )
            .expect("ready body chunk should fit test channel");
        drop(body_tx);
        completion_tx.send(Ok(())).unwrap();
        let mut source = RawDocumentBodySource::External(raw_body);
        let headers = vec![(
            "Content-Type".to_owned(),
            "text/html; charset=utf-8".to_owned(),
        )];
        let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

        preload_scan_ready_streaming_raw_chunks(&mut runtime, &mut source, &mut decoder, None);

        assert!(
            runtime
                .state
                .buffered_document_preloads
                .entries
                .contains_key(&classic_preload_key_for_streaming_test(
                    "https://example.test/later.js"
                )),
            "ready chunks pre-scanned at source completion should feed the main-document preload scanner"
        );
        assert_eq!(
            runtime
                .state
                .parser_session
                .queued_chunk_count_for_testing(),
            1,
            "pre-scan should queue parser input instead of running the parser"
        );
        assert!(
            !native_dom_has_element_id(
                &runtime.page_vm.vm().snapshot_live_document(),
                "after-blocking"
            ),
            "pre-scanning ready chunks must not advance parser-visible DOM past the blocking script"
        );
    }

    #[tokio::test]
    async fn post_bootstrap_ready_chunk_preload_uses_service_worker_owner_wake() {
        let script_body = "window.later = true;";
        let (script_url, server) = spawn_single_script_server(script_body).await;
        let load = crate::planning::SharedScriptSourceLoad::ready_ok("window.blocking = true;");
        let mut runtime = streaming_runtime_with_pending_parser_blocking_source_load(load);
        let browser_context_owner = crate::runtime::RendererBrowserContextRuntime::new();
        let browser_context_runtime = browser_context_owner.handle();
        let document_url = Url::parse("https://example.test/").expect("test document url");
        let completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let client_id = browser_context_runtime.register_service_worker_client(
            document_url.clone(),
            moli_storage_key::MoliStorageKey::first_party_from_url(&document_url, None)
                .serialized_storage_key(),
            crate::service_worker_runtime::ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(1)),
            completion_queue.sender(),
        );
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let wake_page_id = PageId::new_for_testing(77);
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(wake_page_id),
        );
        let service_worker_context = ServiceWorkerScriptPreloadContext::new(
            browser_context_runtime,
            client_id,
            document_url,
            Some(owner_wake),
        );
        let (completion_tx, completion_rx) = oneshot::channel();
        let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
        body_tx
            .try_send(
                format!(r#"<script defer src="{script_url}"></script><body>after"#).into_bytes(),
            )
            .expect("ready body chunk should fit test channel");
        drop(body_tx);
        completion_tx.send(Ok(())).unwrap();
        let mut source = RawDocumentBodySource::External(raw_body);
        let headers = vec![(
            "Content-Type".to_owned(),
            "text/html; charset=utf-8".to_owned(),
        )];
        let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

        preload_scan_ready_streaming_raw_chunks(
            &mut runtime,
            &mut source,
            &mut decoder,
            Some(&service_worker_context),
        );

        let preload = runtime
            .state
            .buffered_document_preloads
            .entries
            .load_for_key(&classic_preload_key_for_streaming_test(script_url.as_str()))
            .expect("ready chunk should create script preload");
        let outcome =
            tokio::time::timeout(std::time::Duration::from_secs(2), preload.wait_outcome())
                .await
                .expect("script preload should finish");
        assert_eq!(
            outcome
                .source_result
                .expect("script preload should load source"),
            script_body
        );
        let wake = tokio::time::timeout(std::time::Duration::from_secs(1), wake_rx.recv())
            .await
            .expect("service-worker-aware preload should signal owner wake")
            .expect("owner wake channel should remain open");
        assert_eq!(wake.page_id(), wake_page_id);
        assert!(matches!(
            wake,
            crate::page_task_queue::RendererOwnerWake::Page {
                source:
                    crate::page_task_queue::RendererOwnerWakeSource::ParseTimeDocumentScriptWork,
                ..
            }
        ));
        server.await.expect("test script server should finish");
        drop(completion_queue);
    }

    #[test]
    fn ready_raw_chunks_can_be_scanned_before_bootstrap_without_parser_input() {
        let (completion_tx, completion_rx) = oneshot::channel();
        let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
        body_tx.try_send(b"<html><body>".to_vec()).unwrap();
        body_tx.try_send(b"ready</body></html>".to_vec()).unwrap();
        drop(body_tx);
        completion_tx.send(Ok(())).unwrap();

        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("test loader should construct");
        let mut source = RawDocumentBodySource::External(raw_body);
        let mut state =
            ParseTimeDriverState::new(Url::parse("https://example.test/").expect("test url"));
        let headers = vec![(
            "Content-Type".to_owned(),
            "text/html; charset=utf-8".to_owned(),
        )];
        let mut decoder = HtmlDocumentStreamingDecoder::new(&headers);

        let chunks = prebootstrap_scan_ready_streaming_raw_chunks(
            &mut state,
            &mut source,
            &mut decoder,
            &loader,
            None,
        );

        assert_eq!(
            chunks,
            vec!["<html><body>".to_owned(), "ready</body></html>".to_owned()]
        );
        assert_eq!(state.parser_session.queued_chunk_count_for_testing(), 0);
    }

    #[test]
    fn streaming_navigation_download_detection_uses_content_disposition_type() {
        assert!(response_headers_indicate_download(&[(
            "Content-Disposition".to_owned(),
            "attachment; filename=report.html".to_owned(),
        )]));

        assert!(!response_headers_indicate_download(&[(
            "Content-Disposition".to_owned(),
            "inline; filename=attachment.html".to_owned(),
        )]));
    }
}
