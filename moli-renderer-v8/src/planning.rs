use std::sync::Arc;

use anyhow::{Result, anyhow};
use moli_encoding::decode_classic_script_source;
use moli_fetch::{RequestCredentialsMode, RequestMode};
use moli_web_mime::{
    FetchDestination, MimeSniffingContext, ScriptResponseMimeError, check_script_response_mime,
    computed_response_mime_type, is_webassembly_mime,
};
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::{
    network::{
        RendererResourceTaskRunner,
        context::{DocumentResourceLoader, ResourceResponseProvenance},
    },
    runtime::RendererBrowserContextRuntime,
    service_worker_runtime::ServiceWorkerClientId,
    types::{
        ScriptKind, ScriptMode, SharedNavigationResponseResult, SubresourceRequestInitiatorType,
        SubresourceResourceType,
    },
};

pub(crate) use moli_parser::{
    ParserPlanningReadView, ParserScriptRead, PrepareScriptOutcome, PreparedScript,
    ScriptFetchMetadata, ScriptSource, build_prepared_script, classify_parser_script,
};

#[derive(Clone)]
pub(crate) struct SharedScriptSourceLoad {
    inner: Arc<SharedScriptSourceLoadInner>,
}

#[derive(Debug)]
pub(crate) struct SharedScriptSourceLoadCompleter {
    load: std::sync::Weak<SharedScriptSourceLoadInner>,
    owner_wake: Option<crate::page_task_queue::RendererOwnerWakeSender>,
}

impl std::fmt::Debug for SharedScriptSourceLoad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedScriptSourceLoad")
            .finish_non_exhaustive()
    }
}

struct SharedScriptSourceLoadInner {
    state: Mutex<SharedScriptSourceLoadState>,
    notify: Notify,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for SharedScriptSourceLoadInner {
    fn drop(&mut self) {
        if let Some(task) = self.task.get_mut().take() {
            task.abort();
        }
    }
}

#[derive(Default)]
struct SharedScriptSourceLoadState {
    result: Option<PreparedScriptSourceLoadOutcome>,
    completion_wakes: Vec<Box<dyn FnOnce() + Send + 'static>>,
}

impl SharedScriptSourceLoad {
    fn pending() -> Self {
        Self {
            inner: Arc::new(SharedScriptSourceLoadInner {
                state: Mutex::new(SharedScriptSourceLoadState::default()),
                notify: Notify::new(),
                task: Mutex::new(None),
            }),
        }
    }
    pub(crate) fn spawn(
        script: PreparedScript,
        mut loader: DocumentResourceLoader,
        document_character_set: Option<String>,
        request_resource_type: Option<moli_fetch::RequestResourceType>,
        initiator: SubresourceRequestInitiatorType,
        service_worker: Option<(RendererBrowserContextRuntime, ServiceWorkerClientId)>,
        owner_wake: Option<crate::page_task_queue::RendererOwnerWakeSender>,
    ) -> Self {
        if let Some((runtime, client_id)) = service_worker {
            loader.bind_service_worker(runtime, client_id);
        }
        let task_runner = loader.task_runner();
        let completion = loader.script_source_completion();
        Self::spawn_outcome(
            async move {
                load_script_source(
                    &script,
                    &loader,
                    document_character_set.as_deref(),
                    request_resource_type,
                    initiator,
                )
                .await
            },
            task_runner,
            owner_wake,
            completion,
        )
    }

    pub(crate) fn try_outcome(&self) -> Option<PreparedScriptSourceLoadOutcome> {
        self.inner.state.lock().result.clone()
    }

    pub(crate) async fn wait_outcome(&self) -> PreparedScriptSourceLoadOutcome {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(result) = self.try_outcome() {
                return result;
            }
            notified.await;
        }
    }

    fn spawn_outcome<F>(
        task: F,
        task_runner: RendererResourceTaskRunner,
        owner_wake: Option<crate::page_task_queue::RendererOwnerWakeSender>,
        publish: impl FnOnce(SharedScriptSourceLoadCompleter, PreparedScriptSourceLoadOutcome)
        + Send
        + 'static,
    ) -> Self
    where
        F: std::future::Future<Output = PreparedScriptSourceLoadOutcome> + Send + 'static,
    {
        let load = Self::pending();
        // The producer must not keep its own consumers alive. The last
        // parser/preload consumer drops this task and its in-flight request;
        // dropping just one shared consumer leaves the others unaffected.
        let completion = SharedScriptSourceLoadCompleter {
            load: Arc::downgrade(&load.inner),
            owner_wake,
        };
        let task = task_runner.spawn_abortable(async move {
            let result = task.await;
            publish(completion, result);
        });
        *load.inner.task.lock() = Some(task);
        load
    }

    pub(crate) fn pending_with_owner_wake(
        owner_wake: Option<crate::page_task_queue::RendererOwnerWakeSender>,
    ) -> (Self, SharedScriptSourceLoadCompleter) {
        let load = Self::pending();
        (
            load.clone(),
            SharedScriptSourceLoadCompleter {
                load: Arc::downgrade(&load.inner),
                owner_wake,
            },
        )
    }

    pub(crate) fn register_completion_wake(&self, wake: impl FnOnce() + Send + 'static) {
        let wake = {
            let mut state = self.inner.state.lock();
            if state.result.is_some() {
                Some(Box::new(wake) as Box<dyn FnOnce() + Send + 'static>)
            } else {
                state.completion_wakes.push(Box::new(wake));
                None
            }
        };
        if let Some(wake) = wake {
            wake();
        }
    }

    fn finish(&self, result: PreparedScriptSourceLoadOutcome) {
        let completion_wakes = {
            let mut state = self.inner.state.lock();
            if state.result.is_some() {
                return;
            }
            state.result = Some(result);
            std::mem::take(&mut state.completion_wakes)
        };
        self.inner.notify.notify_waiters();
        for wake in completion_wakes {
            wake();
        }
    }

    #[cfg(test)]
    pub(crate) fn ready_ok(source: impl Into<String>) -> Self {
        let load = Self::pending();
        load.finish(PreparedScriptSourceLoadOutcome {
            source_result: Ok(source.into()),
            source_bytes: None,
            network_result: None,
        });
        load
    }

    #[cfg(test)]
    pub(crate) fn ready_err(error: impl Into<String>) -> Self {
        let load = Self::pending();
        load.finish(PreparedScriptSourceLoadOutcome {
            source_result: Err(error.into()),
            source_bytes: None,
            network_result: None,
        });
        load
    }

    #[cfg(test)]
    pub(crate) fn ready_outcome(
        source_result: std::result::Result<String, String>,
        network_result: Option<SharedNavigationResponseResult>,
    ) -> Self {
        let load = Self::pending();
        load.finish(PreparedScriptSourceLoadOutcome {
            source_result,
            source_bytes: None,
            network_result,
        });
        load
    }

    #[cfg(test)]
    pub(crate) fn spawn_for_test<F>(task: F) -> Self
    where
        F: std::future::Future<Output = std::result::Result<String, String>> + Send + 'static,
    {
        Self::spawn_outcome(
            async move {
                PreparedScriptSourceLoadOutcome {
                    source_result: task.await,
                    source_bytes: None,
                    network_result: None,
                }
            },
            RendererResourceTaskRunner::from_current_tokio().expect("test resource runtime"),
            None,
            SharedScriptSourceLoadCompleter::finish,
        )
    }
}

impl SharedScriptSourceLoadCompleter {
    pub(crate) fn finish(mut self, result: PreparedScriptSourceLoadOutcome) {
        self.complete(result);
    }

    fn complete(&mut self, result: PreparedScriptSourceLoadOutcome) {
        let Some(inner) = self.load.upgrade() else {
            return;
        };
        SharedScriptSourceLoad { inner }.finish(result);
        if let Some(owner_wake) = self.owner_wake.take() {
            owner_wake.signal_parse_time_document_script_work();
        }
    }
}

impl Drop for SharedScriptSourceLoadCompleter {
    fn drop(&mut self) {
        if self
            .load
            .upgrade()
            .is_some_and(|inner| SharedScriptSourceLoad { inner }.try_outcome().is_none())
        {
            self.complete(failed_external_script_source_load_outcome(
                "script source load closed before completion".to_owned(),
            ));
        }
    }
}

pub(crate) fn prepared_script_with_loaded_source(
    script: PreparedScript,
    source: String,
    source_bytes: Option<Vec<u8>>,
) -> PreparedScript {
    if script.kind == ScriptKind::Module
        && is_webassembly_module_script_url(&script.url)
        && let Some(bytes) = source_bytes
    {
        return script.with_loaded_binary_source(source, bytes);
    }
    script.with_loaded_source(source)
}

pub(crate) fn is_webassembly_module_script_url(url: &url::Url) -> bool {
    url.path().to_ascii_lowercase().ends_with(".wasm")
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedScriptSourceLoadOutcome {
    pub(crate) source_result: std::result::Result<String, String>,
    pub(crate) source_bytes: Option<Vec<u8>>,
    pub(crate) network_result: Option<SharedNavigationResponseResult>,
}

pub(crate) async fn load_script_source(
    script: &PreparedScript,
    loader: &DocumentResourceLoader,
    document_character_set: Option<&str>,
    request_resource_type: Option<moli_fetch::RequestResourceType>,
    initiator: SubresourceRequestInitiatorType,
) -> PreparedScriptSourceLoadOutcome {
    match &script.source {
        ScriptSource::Inline(source) | ScriptSource::Loaded(source) => {
            PreparedScriptSourceLoadOutcome {
                source_result: Ok(source.clone()),
                source_bytes: None,
                network_result: None,
            }
        }
        ScriptSource::LoadedBinary { source, bytes } => PreparedScriptSourceLoadOutcome {
            source_result: Ok(source.clone()),
            source_bytes: Some(bytes.clone()),
            network_result: None,
        },
        ScriptSource::External => {
            if let Some(outcome) = immediate_external_script_source_load_outcome(
                script,
                loader,
                document_character_set,
                initiator,
            ) {
                return outcome;
            }
            let request = external_script_request(
                script,
                &loader.fetch_context().request_origin(),
                request_resource_type,
            );
            match Box::pin(loader.fetch_resource(
                request,
                SubresourceResourceType::Script,
                initiator,
            ))
            .await
            {
                Ok((response, provenance)) => match provenance {
                    ResourceResponseProvenance::Network => {
                        external_script_source_load_outcome_from_response(
                            script,
                            &loader.fetch_context().request_origin(),
                            response,
                            document_character_set,
                        )
                    }
                    ResourceResponseProvenance::ServiceWorker { filter } => {
                        external_script_source_load_outcome_from_response_inner(
                            script,
                            response,
                            document_character_set,
                            filter,
                            None,
                        )
                    }
                },
                Err(error) => failed_external_script_source_load_outcome(format!(
                    "failed to fetch script `{}`: {error}",
                    script.url,
                )),
            }
        }
    }
}

pub(crate) fn external_script_request(
    script: &PreparedScript,
    request_origin: &moli_url::WebOrigin,
    request_resource_type: Option<moli_fetch::RequestResourceType>,
) -> moli_fetch::Request {
    moli_fetch::Request::new("GET", script.url.as_str(), None, vec![])
        .expect("prepared script url should already be parsed")
        .with_page_network_policy()
        .with_initiator_url(&script.initiator_url)
        .with_request_origin(request_origin.clone())
        .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Script)
        .with_credentials_mode(external_script_credentials_mode(
            script.kind,
            &script.fetch_metadata,
        ))
        .with_request_mode(external_script_request_mode(
            script.kind,
            &script.fetch_metadata,
        ))
        .with_resource_type(
            request_resource_type
                .unwrap_or_else(|| script_fetch_resource_type(script.kind, script.mode)),
        )
        .with_script_fetch_metadata(script_fetch_request_metadata(script))
}

pub(crate) fn immediate_external_script_source_load_outcome(
    script: &PreparedScript,
    loader: &DocumentResourceLoader,
    document_character_set: Option<&str>,
    initiator: SubresourceRequestInitiatorType,
) -> Option<PreparedScriptSourceLoadOutcome> {
    if !matches!(script.source, ScriptSource::External)
        || matches!(script.url.scheme(), "http" | "https")
    {
        return None;
    }
    if !matches!(script.url.scheme(), "data" | "blob") {
        return Some(failed_external_script_source_load_outcome(format!(
            "failed to fetch script `{}`: URL scheme `{}` is not allowed",
            script.url,
            script.url.scheme(),
        )));
    }
    let request = external_script_request(script, &loader.fetch_context().request_origin(), None);
    let Some((load, network, started)) =
        loader.prepare_resource_request(&request, SubresourceResourceType::Script, initiator)
    else {
        return Some(failed_external_script_source_load_outcome(
            "Document resource owner retired".into(),
        ));
    };
    network.observe(started);
    let outcome = match crate::network_host::local_url_response(&script.url) {
        Some(response) => {
            network.response_completed(&response);
            if script.url.scheme() == "data" {
                // Preserve the synchronous strict data-URL decoding contract.
                PreparedScriptSourceLoadOutcome {
                    source_result: decode_data_url_script_source(&script.url)
                        .map_err(|error| error.to_string()),
                    source_bytes: None,
                    network_result: None,
                }
            } else {
                external_script_source_load_outcome_from_response(
                    script,
                    &loader.fetch_context().request_origin(),
                    response.into(),
                    document_character_set,
                )
            }
        }
        None => {
            let message = if script.url.scheme() == "data" {
                decode_data_url_script_source(&script.url)
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| {
                        format!(
                            "failed to fetch script `{}`: data URL is unavailable",
                            script.url
                        )
                    })
            } else {
                format!(
                    "failed to fetch script `{}`: blob URL is unavailable",
                    script.url
                )
            };
            network.failed(&crate::network::ResourceResponseFailure::Request(
                message.clone(),
            ));
            failed_external_script_source_load_outcome(message)
        }
    };
    load.finish();
    Some(outcome)
}

fn failed_external_script_source_load_outcome(message: String) -> PreparedScriptSourceLoadOutcome {
    PreparedScriptSourceLoadOutcome {
        source_result: Err(message.clone()),
        source_bytes: None,
        network_result: Some(Arc::new(Err(message))),
    }
}

pub(crate) fn external_script_source_load_outcome_from_response(
    script: &PreparedScript,
    request_origin: &moli_url::WebOrigin,
    response: crate::protocol_types::NavigationResponse,
    document_character_set: Option<&str>,
) -> PreparedScriptSourceLoadOutcome {
    let request_mode = external_script_request_mode(script.kind, &script.fetch_metadata);
    let head = response.head();
    let response_filter =
        crate::network_host::network_response_filter(request_origin, &head, request_mode);
    let cors_error = if request_mode == RequestMode::Cors {
        crate::network_host::validate_cors_response_chain(
            request_origin,
            &head,
            external_script_credentials_mode(script.kind, &script.fetch_metadata),
        )
        .err()
    } else {
        None
    };
    external_script_source_load_outcome_from_response_inner(
        script,
        response,
        document_character_set,
        response_filter,
        cors_error,
    )
}

pub(crate) fn external_script_source_load_outcome_from_result(
    script: &PreparedScript,
    request_origin: &moli_url::WebOrigin,
    result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    document_character_set: Option<&str>,
) -> PreparedScriptSourceLoadOutcome {
    match result {
        Ok(response) => external_script_source_load_outcome_from_response(
            script,
            request_origin,
            response,
            document_character_set,
        ),
        Err(message) => failed_external_script_source_load_outcome(message),
    }
}

fn external_script_source_load_outcome_from_response_inner(
    script: &PreparedScript,
    response: crate::protocol_types::NavigationResponse,
    document_character_set: Option<&str>,
    response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    cors_error: Option<String>,
) -> PreparedScriptSourceLoadOutcome {
    let response_bytes = response.body_bytes().to_vec();
    let opaque_status_zero = response_filter
        == Some(crate::types::AsyncSubresourceFetchResponseFilter::Opaque)
        && response.status == 0;
    let source_result = if let Some(error) = cors_error {
        Err(format!("script request `{}` failed: {error}", script.url))
    } else if !(opaque_status_zero || (200..=299).contains(&response.status)) {
        Err(format!(
            "script request `{}` returned HTTP {}",
            script.url, response.status
        ))
    } else if !opaque_status_zero
        && let Err(error) =
            validate_external_script_response_mime(&script.url, script.kind, &response)
    {
        Err(error)
    } else if !crate::subresource_integrity::response_matches_subresource_integrity_metadata(
        &response_bytes,
        script.fetch_metadata.integrity.as_deref(),
        response_filter.is_none_or(|filter| filter.is_readable()),
    ) {
        Err(format!(
            "script request `{}` failed its integrity check",
            script.url
        ))
    } else {
        Ok(decode_external_script_source(
            script,
            &response,
            document_character_set,
        ))
    };
    PreparedScriptSourceLoadOutcome {
        source_result,
        source_bytes: Some(response_bytes),
        network_result: Some(Arc::new(Ok(response))),
    }
}

fn decode_external_script_source(
    script: &PreparedScript,
    response: &crate::protocol_types::NavigationResponse,
    document_character_set: Option<&str>,
) -> String {
    if script.kind == ScriptKind::Classic {
        decode_classic_script_source(
            response.body_bytes(),
            &response.headers,
            script.fetch_metadata.charset.as_deref(),
            document_character_set,
        )
    } else {
        response.body_text().to_owned()
    }
}

fn script_fetch_request_metadata(
    script: &PreparedScript,
) -> moli_fetch::ScriptFetchRequestMetadata {
    moli_fetch::ScriptFetchRequestMetadata {
        cross_origin: script.fetch_metadata.cross_origin.clone(),
        referrer_policy: script.fetch_metadata.referrer_policy.clone(),
        document_referrer_policy: None,
        charset: script.fetch_metadata.charset.clone(),
        integrity: script.fetch_metadata.integrity.clone(),
        nonce: script.fetch_metadata.nonce.clone(),
        fetch_priority: script.fetch_metadata.fetch_priority,
        scheduler_priority: None,
    }
}

fn script_fetch_resource_type(
    kind: ScriptKind,
    mode: ScriptMode,
) -> moli_fetch::RequestResourceType {
    match (kind, mode) {
        (ScriptKind::Classic, ScriptMode::Normal) => {
            moli_fetch::RequestResourceType::ParserBlockingScript
        }
        (ScriptKind::Classic, ScriptMode::Async | ScriptMode::Defer) => {
            moli_fetch::RequestResourceType::ClassicAsyncOrDeferScript
        }
        _ => moli_fetch::RequestResourceType::Script,
    }
}

pub(crate) fn external_script_credentials_mode(
    kind: ScriptKind,
    metadata: &ScriptFetchMetadata,
) -> RequestCredentialsMode {
    match kind {
        ScriptKind::Module => module_script_credentials_mode(metadata.cross_origin.as_deref()),
        ScriptKind::Classic if metadata.cross_origin.is_some() => {
            module_script_credentials_mode(metadata.cross_origin.as_deref())
        }
        ScriptKind::Classic | ScriptKind::ImportMap | ScriptKind::DataBlock => {
            RequestCredentialsMode::Include
        }
    }
}

pub(crate) fn external_script_request_mode(
    kind: ScriptKind,
    metadata: &ScriptFetchMetadata,
) -> RequestMode {
    if kind == ScriptKind::Classic && metadata.cross_origin.is_none() {
        RequestMode::NoCors
    } else {
        RequestMode::Cors
    }
}

// HTML's module-script fetch options use the CORS settings attribute credentials
// mode: absent/anonymous => same-origin, use-credentials => include. Keeping the
// same mapping as modulepreload lets both paths share the script text cache.
pub(crate) fn module_script_credentials_mode(cross_origin: Option<&str>) -> RequestCredentialsMode {
    if cross_origin == Some("use-credentials") {
        RequestCredentialsMode::Include
    } else {
        RequestCredentialsMode::SameOrigin
    }
}

pub(crate) fn decode_data_url_script_source(url: &url::Url) -> Result<String> {
    crate::worker::decode_data_url_script_source(url, "failed to decode script data URL")
        .map_err(|error| anyhow!(error))
}

pub(crate) fn validate_external_script_response_mime(
    script_url: &url::Url,
    kind: ScriptKind,
    response: &crate::protocol_types::NavigationResponse,
) -> std::result::Result<(), String> {
    if kind == ScriptKind::Module && response_has_webassembly_module_mime(response) {
        return Ok(());
    }
    let require_javascript_mime = match kind {
        ScriptKind::Classic => false,
        ScriptKind::Module => true,
        ScriptKind::ImportMap | ScriptKind::DataBlock => return Ok(()),
    };
    check_script_response_mime(
        &response.headers,
        response.body_bytes(),
        FetchDestination::Script,
        require_javascript_mime,
    )
    .map_err(|error| external_script_mime_error_message(script_url, error))
}

fn response_has_webassembly_module_mime(
    response: &crate::protocol_types::NavigationResponse,
) -> bool {
    is_webassembly_mime(&computed_response_mime_type(
        &response.headers,
        MimeSniffingContext::Script,
        response.body_bytes(),
    ))
}

fn external_script_mime_error_message(
    script_url: &url::Url,
    error: ScriptResponseMimeError,
) -> String {
    match error {
        ScriptResponseMimeError::Nosniff => {
            format!("script request `{script_url}` blocked by X-Content-Type-Options nosniff")
        }
        ScriptResponseMimeError::Unsupported(mime_type) => {
            format!(
                "script request `{script_url}` returned unsupported script MIME type `{mime_type}`"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shared_script_source_cannot_overtake_its_queued_network_receipts() {
        use crate::frame_owner_model::{
            DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId,
        };
        use crate::network::{ResourceRequestClient, context::DocumentFetchContext};
        use crate::runtime::{
            RendererBrowserContextRuntimeId, RendererDocumentLifecycleJournalHandle,
            RendererNetworkReporter, RendererOwnerLocalHostId,
        };

        let client = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut loader = DocumentResourceLoader::new(
            (*client).clone(),
            RendererResourceTaskRunner::from_current_tokio().unwrap(),
            DocumentFetchContext::new(
                crate::native_bridge::WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                    FrameSchedulerLaneId(1),
                    LocalWindowId(1),
                    DocumentId(1),
                )),
                url.clone(),
                url,
                "https://example.test",
            ),
        );
        let mut queue = crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        loader.bind_network(
            RendererNetworkReporter::new(RendererBrowserContextRuntimeId::new_for_testing(1))
                .for_document(
                    RendererOwnerLocalHostId::new_for_testing(1),
                    RendererDocumentLifecycleJournalHandle::new_initial(
                        crate::PageId::new_for_testing(1),
                    )
                    .identity(),
                ),
            queue.sender(),
            None,
        );
        let load = SharedScriptSourceLoad::spawn(
            prepared_external_script(
                "data:text/javascript,globalThis.ready=true",
                ScriptKind::Classic,
            ),
            loader,
            None,
            None,
            SubresourceRequestInitiatorType::Parser,
            None,
            None,
        );
        // Let the actual producer finish while the Page has consumed no tasks.
        // A previously admitted parser turn can run at exactly this boundary.
        let producer = load.inner.task.lock().take().unwrap();
        producer.await.unwrap();
        assert!(queue.has_ready_completion());
        assert!(
            load.try_outcome().is_none(),
            "transport completion cannot expose script source ahead of its Page receipts"
        );
        let mut stages = Vec::new();
        loop {
            assert!(load.try_outcome().is_none());
            match queue.pop_next_page_terminal().expect("source completion follows its receipts") {
                crate::page_resource_completion::RendererPageResourceTerminal::AsyncSubresource { event } => {
                    let crate::types::AsyncSubresourceFetchEvent::NativeNetwork(observation) = *event else {
                        panic!("script loading must only queue its own native receipts")
                    };
                    stages.push(observation.item().clone());
                }
                crate::page_resource_completion::RendererPageResourceTerminal::SharedScriptSource { completion, outcome, .. } => {
                    assert_eq!(stages.len(), 3, "start, response, and terminal precede source readiness");
                    let crate::runtime::RendererNetworkOutputItem::Resource(terminal) = &stages[2] else {
                        panic!("expected resource receipt")
                    };
                    assert!(matches!(terminal.as_ref(), crate::types::ScriptNetworkOutputItem::SubresourceBodyFinished(_)));
                    completion.finish(*outcome);
                    break;
                }
                terminal => panic!("unexpected source task: {terminal:?}"),
            }
        }
        assert_eq!(
            load.try_outcome().unwrap().source_result.unwrap(),
            "globalThis.ready=true"
        );
        assert!(!queue.has_ready_completion());
    }

    #[tokio::test]
    async fn shared_source_load_last_consumer_cancels_its_producer() {
        struct Dropped(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                let _ = self.0.take().unwrap().send(());
            }
        }
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, mut dropped_rx) = tokio::sync::oneshot::channel();
        let load = SharedScriptSourceLoad::spawn_for_test(async move {
            let _dropped = Dropped(Some(dropped_tx));
            started_tx.send(()).unwrap();
            std::future::pending().await
        });
        let retained = load.clone();
        started_rx.await.unwrap();
        drop(load);
        assert_eq!(
            dropped_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        );
        drop(retained);
        tokio::time::timeout(std::time::Duration::from_secs(3), dropped_rx)
            .await
            .expect("the final consumer must cancel the actual resource task")
            .unwrap();
    }

    #[tokio::test]
    async fn shared_source_load_surviving_consumer_receives_its_completion() {
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let load = SharedScriptSourceLoad::spawn_for_test(async move {
            finish_rx.await.unwrap();
            Ok("retained-script".to_owned())
        });
        let retained = load.clone();
        drop(load);
        finish_tx
            .send(())
            .expect("a remaining consumer must retain the producer");
        assert_eq!(
            retained.wait_outcome().await.source_result.unwrap(),
            "retained-script"
        );
    }

    #[tokio::test]
    async fn shared_source_load_completion_uses_parse_time_owner_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(71)),
        );
        let load = SharedScriptSourceLoad::spawn_outcome(
            async {
                PreparedScriptSourceLoadOutcome {
                    source_result: Ok("window.ready = true;".to_owned()),
                    source_bytes: None,
                    network_result: None,
                }
            },
            RendererResourceTaskRunner::from_current_tokio()
                .expect("Tokio test should expose its resource task runner"),
            Some(owner_wake),
            SharedScriptSourceLoadCompleter::finish,
        );

        let _ = load.wait_outcome().await;
        let wake = wake_rx
            .recv()
            .await
            .expect("completed parser source load should wake its Page owner");
        assert_eq!(
            wake.source_for_test(),
            crate::page_task_queue::RendererOwnerWakeSource::ParseTimeDocumentScriptWork
        );
    }

    #[tokio::test]
    async fn shared_source_load_completion_wakes_registered_and_late_owners() {
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let load = SharedScriptSourceLoad::spawn_for_test(async move {
            finish_rx.await.expect("source completion signal");
            Ok("window.ready = true;".to_owned())
        });
        let (first_wake_tx, first_wake_rx) = tokio::sync::oneshot::channel();
        load.register_completion_wake(move || {
            let _ = first_wake_tx.send(());
        });

        finish_tx.send(()).expect("finish source load");
        tokio::time::timeout(std::time::Duration::from_secs(1), first_wake_rx)
            .await
            .expect("registered source owner should wake")
            .expect("registered source wake sender");

        let (late_wake_tx, late_wake_rx) = tokio::sync::oneshot::channel();
        load.register_completion_wake(move || {
            let _ = late_wake_tx.send(());
        });
        late_wake_rx
            .await
            .expect("late source owner should wake immediately");
    }

    fn prepared_external_classic_with_mode(mode: ScriptMode) -> PreparedScript {
        let url = url::Url::parse("https://example.test/app.js").unwrap();
        PreparedScript {
            position: 0,
            node_id: crate::dom::NodeId::new(1),
            kind: ScriptKind::Classic,
            mode,
            source_kind: crate::types::ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: url.clone(),
            base_url: url.clone(),
            initiator_url: url,
            host_script_handle: None,
        }
    }

    fn script_response(
        url: &url::Url,
        headers: Vec<(&str, &str)>,
    ) -> crate::protocol_types::NavigationResponse {
        crate::protocol_types::NavigationResponse::from_text_body(
            url.clone(),
            200,
            headers
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.as_bytes().to_vec()))
                .collect(),
            "console.log('ok')".to_owned(),
        )
    }

    fn script_response_with_body_bytes(
        url: &url::Url,
        headers: Vec<(&str, &str)>,
        body_bytes: Vec<u8>,
    ) -> crate::protocol_types::NavigationResponse {
        script_response_with_status_and_body_bytes(url, 200, headers, body_bytes)
    }

    fn script_response_with_status_and_body_bytes(
        url: &url::Url,
        status: u16,
        headers: Vec<(&str, &str)>,
        body_bytes: Vec<u8>,
    ) -> crate::protocol_types::NavigationResponse {
        let mut head = script_response(url, headers).head();
        head.status = status;
        let body = String::from_utf8_lossy(&body_bytes).into_owned();
        crate::protocol_types::NavigationResponse::from_head_and_body(head, body, body_bytes)
    }

    fn prepared_external_script(url: &str, kind: ScriptKind) -> PreparedScript {
        let url = url::Url::parse(url).expect("test script url");
        prepared_external_script_with_initiator(url.clone(), kind, url)
    }

    fn prepared_external_script_with_initiator(
        url: url::Url,
        kind: ScriptKind,
        initiator_url: url::Url,
    ) -> PreparedScript {
        PreparedScript {
            position: 0,
            node_id: crate::dom::NodeId::new(1),
            kind,
            mode: match kind {
                ScriptKind::Module => crate::types::ScriptMode::ModuleDefer,
                _ => crate::types::ScriptMode::Normal,
            },
            source_kind: crate::types::ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: url.clone(),
            base_url: url.clone(),
            initiator_url,
            host_script_handle: None,
        }
    }

    #[test]
    fn shared_script_source_load_completer_drop_finishes_with_error() {
        let (load, completer) = SharedScriptSourceLoad::pending_with_owner_wake(None);

        drop(completer);

        let outcome = load
            .try_outcome()
            .expect("dropped completer should finish the pending source load");
        let error = outcome
            .source_result
            .expect_err("dropped completer should fail source load");
        assert_eq!(error, "script source load closed before completion");
        let network_result = outcome
            .network_result
            .expect("dropped completer should record failed network result");
        assert_eq!(
            network_result
                .as_ref()
                .as_ref()
                .expect_err("network failure"),
            &error
        );
    }

    #[test]
    fn local_external_script_sources_are_terminal_without_an_async_fetch() {
        let data = prepared_external_script(
            "data:text/javascript,globalThis.localSource%3Dtrue",
            ScriptKind::Classic,
        );
        let client =
            crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
                .unwrap();
        let loader = DocumentResourceLoader::for_test(
            client.handle(),
            RendererResourceTaskRunner::for_test(),
            data.initiator_url.clone(),
        );
        let data_outcome = immediate_external_script_source_load_outcome(
            &data,
            &loader,
            Some("utf-8"),
            SubresourceRequestInitiatorType::Parser,
        )
        .expect("data URL source should be synchronously terminal");
        assert_eq!(
            data_outcome.source_result.expect("decoded data URL source"),
            "globalThis.localSource=true"
        );

        let unsupported = prepared_external_script("custom:script", ScriptKind::Classic);
        assert!(
            immediate_external_script_source_load_outcome(
                &unsupported,
                &loader,
                None,
                SubresourceRequestInitiatorType::Parser
            )
            .expect("unsupported scheme should be synchronously terminal")
            .source_result
            .is_err()
        );

        let network = prepared_external_script("https://example.test/app.js", ScriptKind::Classic);
        assert!(
            immediate_external_script_source_load_outcome(
                &network,
                &loader,
                None,
                SubresourceRequestInitiatorType::Parser
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn unsupported_external_script_scheme_fails_before_network_fetch() -> anyhow::Result<()> {
        let loader =
            crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())?;
        let script = prepared_external_script("unknown://example/", ScriptKind::Classic);

        let outcome = load_script_source(
            &script,
            &DocumentResourceLoader::for_test(
                loader.handle(),
                RendererResourceTaskRunner::for_test(),
                script.initiator_url.clone(),
            ),
            Some("UTF-8"),
            None,
            crate::types::SubresourceRequestInitiatorType::Parser,
        )
        .await;

        let error = outcome
            .source_result
            .expect_err("unsupported external script scheme should fail");
        assert_eq!(
            error,
            "failed to fetch script `unknown://example/`: URL scheme `unknown` is not allowed"
        );
        assert!(
            outcome.source_bytes.is_none(),
            "unsupported scheme should not synthesize script body bytes"
        );
        let network_result = outcome
            .network_result
            .expect("unsupported scheme should be recorded as a failed subresource load");
        assert_eq!(
            network_result
                .as_ref()
                .as_ref()
                .expect_err("network failure"),
            &error
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn top_level_external_module_source_load_reuses_script_text_cache_across_same_site_pages()
    -> anyhow::Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request_head(&mut stream).await.unwrap();
            let body = "export default function cachedTopLevelModule() {}";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();

            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                    .await
                    .is_err(),
                "second top-level module source load should use the loader script text cache"
            );
        });

        let loader =
            crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())?;
        let module_url = url::Url::parse(&format!("http://{addr}/cached-module.mjs"))?;
        let first_page = url::Url::parse(&format!("http://{addr}/first-page.html"))?;
        let second_page = url::Url::parse(&format!("http://{addr}/second-page.html"))?;
        let first_script = prepared_external_script_with_initiator(
            module_url.clone(),
            ScriptKind::Module,
            first_page,
        );
        let second_script =
            prepared_external_script_with_initiator(module_url, ScriptKind::Module, second_page);

        let first = load_script_source(
            &first_script,
            &DocumentResourceLoader::for_test(
                loader.handle(),
                RendererResourceTaskRunner::for_test(),
                first_script.initiator_url.clone(),
            ),
            Some("UTF-8"),
            None,
            crate::types::SubresourceRequestInitiatorType::Parser,
        )
        .await;
        let second = load_script_source(
            &second_script,
            &DocumentResourceLoader::for_test(
                loader.handle(),
                RendererResourceTaskRunner::for_test(),
                second_script.initiator_url.clone(),
            ),
            Some("UTF-8"),
            None,
            crate::types::SubresourceRequestInitiatorType::Parser,
        )
        .await;

        first.source_result.map_err(anyhow::Error::msg)?;
        second.source_result.map_err(anyhow::Error::msg)?;
        let first_network = first
            .network_result
            .expect("first module source load should record a network result");
        let second_network = second
            .network_result
            .expect("second module source load should record a network result");
        assert!(
            !first_network
                .as_ref()
                .as_ref()
                .expect("first fetch should succeed")
                .from_cache,
            "first module source load should come from network"
        );
        assert!(
            second_network
                .as_ref()
                .as_ref()
                .expect("second fetch should succeed")
                .from_cache,
            "second module source load should preserve memory-cache provenance"
        );

        server.await?;
        Ok(())
    }

    #[test]
    fn external_module_script_uses_same_origin_credentials_by_default() {
        let metadata = ScriptFetchMetadata::default();

        assert_eq!(
            external_script_credentials_mode(ScriptKind::Module, &metadata),
            RequestCredentialsMode::SameOrigin,
            "module scripts must match modulepreload's default credentials mode"
        );
    }

    #[test]
    fn external_module_script_uses_same_origin_credentials_for_anonymous_crossorigin() {
        let metadata = ScriptFetchMetadata {
            cross_origin: Some("anonymous".to_owned()),
            ..ScriptFetchMetadata::default()
        };

        assert_eq!(
            external_script_credentials_mode(ScriptKind::Module, &metadata),
            RequestCredentialsMode::SameOrigin
        );
    }

    #[test]
    fn external_module_script_uses_include_credentials_for_use_credentials() {
        let metadata = ScriptFetchMetadata {
            cross_origin: Some("use-credentials".to_owned()),
            ..ScriptFetchMetadata::default()
        };

        assert_eq!(
            external_script_credentials_mode(ScriptKind::Module, &metadata),
            RequestCredentialsMode::Include
        );
    }

    #[test]
    fn classic_external_script_keeps_include_credentials() {
        let metadata = ScriptFetchMetadata::default();

        assert_eq!(
            external_script_credentials_mode(ScriptKind::Classic, &metadata),
            RequestCredentialsMode::Include
        );
    }

    #[test]
    fn external_script_request_sets_browser_fetch_mode() {
        let classic =
            prepared_external_script("https://example.test/classic.js", ScriptKind::Classic);
        assert_eq!(
            external_script_request(
                &classic,
                &moli_url::WebOrigin::from_url(&classic.initiator_url),
                None
            )
            .request_mode,
            RequestMode::NoCors
        );

        let mut classic_crossorigin =
            prepared_external_script("https://example.test/cors-classic.js", ScriptKind::Classic);
        classic_crossorigin.fetch_metadata.cross_origin = Some("anonymous".to_owned());
        assert_eq!(
            external_script_request(
                &classic_crossorigin,
                &moli_url::WebOrigin::from_url(&classic_crossorigin.initiator_url),
                None
            )
            .request_mode,
            RequestMode::Cors
        );

        let module =
            prepared_external_script("https://example.test/module.mjs", ScriptKind::Module);
        assert_eq!(
            external_script_request(
                &module,
                &moli_url::WebOrigin::from_url(&module.initiator_url),
                None
            )
            .request_mode,
            RequestMode::Cors
        );
    }

    #[test]
    fn classic_script_modes_map_to_chromium_resource_priorities() {
        let normal = prepared_external_classic_with_mode(ScriptMode::Normal);
        assert_eq!(
            script_fetch_resource_type(normal.kind, normal.mode),
            moli_fetch::RequestResourceType::ParserBlockingScript
        );

        for mode in [ScriptMode::Async, ScriptMode::Defer] {
            let script = prepared_external_classic_with_mode(mode);
            assert_eq!(
                script_fetch_resource_type(script.kind, script.mode),
                moli_fetch::RequestResourceType::ClassicAsyncOrDeferScript,
                "Chromium lowers classic {mode:?} script fetches"
            );
        }
    }

    #[test]
    fn module_and_runtime_ordered_scripts_use_default_script_priority() {
        for mode in [
            ScriptMode::InOrder,
            ScriptMode::ImportMapInOrder,
            ScriptMode::ModuleInOrder,
            ScriptMode::ModuleDefer,
        ] {
            let script = prepared_external_classic_with_mode(mode);
            assert_eq!(
                script_fetch_resource_type(script.kind, script.mode),
                moli_fetch::RequestResourceType::Script,
                "{mode:?} should keep Chromium's default script priority"
            );
        }
    }

    #[test]
    fn classic_script_mime_blocks_nosniff_and_script_like_blocked_types() {
        let url = url::Url::parse("https://example.test/app.js").unwrap();

        assert!(
            validate_external_script_response_mime(
                &url,
                ScriptKind::Classic,
                &script_response(
                    &url,
                    vec![
                        ("Content-Type", "text/html"),
                        ("X-Content-Type-Options", "nosniff"),
                    ],
                ),
            )
            .expect_err("nosniff text/html classic script should be blocked")
            .contains("X-Content-Type-Options nosniff")
        );
        assert!(
            validate_external_script_response_mime(
                &url,
                ScriptKind::Classic,
                &script_response(&url, vec![("Content-Type", "image/png")]),
            )
            .expect_err("image classic script should be blocked")
            .contains("unsupported script MIME type `image/png`")
        );
        assert!(
            validate_external_script_response_mime(
                &url,
                ScriptKind::Classic,
                &script_response(&url, vec![("Content-Type", "text/html")]),
            )
            .is_ok(),
            "classic scripts keep the non-nosniff text/html compatibility path"
        );
    }

    #[test]
    fn module_script_mime_requires_javascript() {
        let url = url::Url::parse("https://example.test/app.mjs").unwrap();

        assert!(
            validate_external_script_response_mime(
                &url,
                ScriptKind::Module,
                &script_response(&url, vec![("Content-Type", "text/html")]),
            )
            .expect_err("module script should require JavaScript MIME")
            .contains("unsupported script MIME type `text/html`")
        );
        assert!(
            validate_external_script_response_mime(
                &url,
                ScriptKind::Module,
                &script_response(&url, vec![("Content-Type", "Text/JavaScript")]),
            )
            .is_ok()
        );
    }

    #[test]
    fn external_script_decode_applies_document_charset_only_to_classic_scripts() {
        let script_source = r#"globalThis.encodingProbe = "目次";"#;
        let bytes = encoding_rs::SHIFT_JIS.encode(script_source).0.into_owned();
        let classic =
            prepared_external_script("https://example.test/classic.js", ScriptKind::Classic);
        let module =
            prepared_external_script("https://example.test/module.mjs", ScriptKind::Module);
        let classic_response = script_response_with_body_bytes(
            &classic.url,
            vec![("Content-Type", "application/javascript")],
            bytes.clone(),
        );
        let module_response = script_response_with_body_bytes(
            &module.url,
            vec![("Content-Type", "application/javascript")],
            bytes.clone(),
        );

        assert_eq!(
            decode_external_script_source(&classic, &classic_response, Some("shift_jis")),
            script_source
        );
        assert_eq!(
            decode_external_script_source(&module, &module_response, Some("shift_jis")),
            String::from_utf8_lossy(&bytes)
        );
    }

    #[test]
    fn external_script_source_load_enforces_integrity() {
        use base64::Engine as _;

        let body = b"export const loaded = true;";
        let digest = base64::engine::general_purpose::STANDARD
            .encode(moli_crypto::DigestAlgorithm::Sha384.digest_bytes(body));
        let mut script =
            prepared_external_script("https://example.test/module.mjs", ScriptKind::Module);
        script.fetch_metadata.integrity = Some(format!("sha384-{digest}"));

        let matching = external_script_source_load_outcome_from_response(
            &script,
            &moli_url::WebOrigin::from_url(&script.initiator_url),
            script_response_with_body_bytes(
                &script.url,
                vec![("Content-Type", "text/javascript")],
                body.to_vec(),
            ),
            None,
        );
        assert_eq!(
            matching.source_result.expect("matching integrity"),
            String::from_utf8_lossy(body)
        );

        script.fetch_metadata.integrity = Some("sha384-foobar".to_owned());
        let mismatch = external_script_source_load_outcome_from_response(
            &script,
            &moli_url::WebOrigin::from_url(&script.initiator_url),
            script_response_with_body_bytes(
                &script.url,
                vec![("Content-Type", "text/javascript")],
                body.to_vec(),
            ),
            None,
        );
        assert!(
            mismatch
                .source_result
                .expect_err("mismatching integrity")
                .contains("failed its integrity check")
        );
    }

    #[test]
    fn service_worker_opaque_status_zero_external_script_response_uses_internal_body() {
        let script =
            prepared_external_script("https://example.test/opaque.js", ScriptKind::Classic);
        let source = "__opaqueScriptCallback('OK');";
        let response = script_response_with_status_and_body_bytes(
            &script.url,
            0,
            Vec::new(),
            source.as_bytes().to_vec(),
        );

        let outcome = external_script_source_load_outcome_from_response_inner(
            &script,
            response,
            None,
            Some(crate::types::AsyncSubresourceFetchResponseFilter::Opaque),
            None,
        );

        assert_eq!(outcome.source_result.expect("opaque script source"), source);
        assert_eq!(
            outcome.source_bytes.expect("opaque script source bytes"),
            source.as_bytes()
        );
    }

    #[test]
    fn ordinary_status_zero_external_script_response_remains_http_failure() {
        let script =
            prepared_external_script("https://example.test/opaque.js", ScriptKind::Classic);
        let response = script_response_with_status_and_body_bytes(
            &script.url,
            0,
            Vec::new(),
            b"ignored".to_vec(),
        );

        let outcome = external_script_source_load_outcome_from_response(
            &script,
            &moli_url::WebOrigin::from_url(&script.initiator_url),
            response,
            None,
        );

        assert!(
            outcome
                .source_result
                .expect_err("ordinary status zero should fail")
                .contains("returned HTTP 0")
        );
    }

    #[test]
    fn module_script_mime_allows_webassembly() {
        let url = url::Url::parse("https://example.test/app.wasm").unwrap();

        assert!(
            validate_external_script_response_mime(
                &url,
                ScriptKind::Module,
                &script_response(&url, vec![("Content-Type", "application/wasm")]),
            )
            .is_ok()
        );
    }
}
