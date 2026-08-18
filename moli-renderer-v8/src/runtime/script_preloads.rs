use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use html5ever::{
    Attribute,
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
        states::{Rawtext, Rcdata, ScriptData},
    },
};
use url::Url;

use parking_lot::Mutex;

use crate::network::ResourceRequestClient;
use crate::page_task_queue::RendererOwnerWakeSender;
use crate::parser::{PreparedScript, ScriptSource};
use crate::planning::{
    SharedScriptSourceLoad, prepared_script_with_loaded_source, script_preload_network_result,
};
use crate::runtime::RendererBrowserContextRuntime;
use crate::runtime::page_vm::{
    PageVm, ScannedImageAdmission, ScannedImageDeferral, ScannedScriptAdmission,
    ScannedScriptDeferral, ScannedStylesheetAdmission, ScannedStylesheetDeferral,
};
use crate::service_worker_runtime::ServiceWorkerClientId;
use crate::stylesheet_blocking::{
    DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheetDiscoveryInput,
    StylesheetFetchOptions, StylesheetResourceKey, link_rel_includes_token,
};
use crate::types::SharedNavigationResponseResult;

pub(super) struct AppliedPreloadedScriptSource {
    pub(super) network_result: Option<SharedNavigationResponseResult>,
}

pub(super) enum ParserBlockingPreloadDisposition {
    Missing,
    ExistingButNotReusable,
    Ready(AppliedPreloadedScriptSource),
    ReusableSourceLoad(SharedScriptSourceLoad),
}

pub(super) struct BufferedDocumentPreloadState {
    pub(super) entries: DocumentScriptPreloadStore,
    document_character_set: String,
    script_fetch_interception_enabled: bool,
    response_csp_requires_parser_admission: bool,
    owner_wake: Option<RendererOwnerWakeSender>,
    resource_task_runner: Option<crate::network::RendererResourceTaskRunner>,
    main_document_scanner: Option<Box<IncrementalHtmlPreloadScanner>>,
    pub(super) insertion_scanner: Option<Box<IncrementalHtmlPreloadScanner>>,
    meta_csp_preload_gate: MetaCspPreloadGate,
    pending_script_preloads: Vec<BufferedScriptPreloadRequest>,
    pending_stylesheet_preloads: Vec<BufferedStylesheetPreloadRequest>,
    pending_image_preloads: Vec<BufferedImagePreloadRequest>,
}

#[derive(Debug, Default)]
struct MetaCspPreloadGate {
    scanner_seen_count: usize,
    parser_processed_count: usize,
}

impl MetaCspPreloadGate {
    fn is_open(&self) -> bool {
        self.scanner_seen_count == self.parser_processed_count
    }

    fn note_scanner_seen(&mut self, count: usize) {
        self.scanner_seen_count = self.scanner_seen_count.saturating_add(count);
    }

    fn note_parser_processed(&mut self, count: usize) {
        let awaiting = self
            .scanner_seen_count
            .saturating_sub(self.parser_processed_count);
        self.parser_processed_count += count.min(awaiting);
    }

    fn has_seen_meta_csp(&self) -> bool {
        self.scanner_seen_count != 0
    }
}

pub(super) const MAX_PENDING_CSP_PRELOAD_CANDIDATES: usize = 4096;

pub(super) struct BufferedScriptPreloadEntry {
    pub(super) request: BufferedScriptPreloadRequest,
    pub(super) load: SharedScriptSourceLoad,
}

/// Document-scoped residence for classic script loads started by the HTML
/// preload scanner.
///
/// The scanner itself remains phase-one state, while its in-flight loads must
/// also be visible to nested parser re-entry in `DocumentRuntime`. Sharing only
/// this resource map keeps one physical fetch owner without moving tokenizer
/// or scanner state into the live Document.
#[derive(Clone, Default)]
pub(crate) struct DocumentScriptPreloadStore {
    entries: Arc<Mutex<HashMap<BufferedScriptPreloadKey, BufferedScriptPreloadEntry>>>,
}

impl std::fmt::Debug for DocumentScriptPreloadStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DocumentScriptPreloadStore")
            .field("entry_count", &self.len())
            .finish()
    }
}

impl DocumentScriptPreloadStore {
    pub(super) fn contains_key(&self, key: &BufferedScriptPreloadKey) -> bool {
        self.entries.lock().contains_key(key)
    }

    pub(super) fn insert(
        &mut self,
        key: BufferedScriptPreloadKey,
        entry: BufferedScriptPreloadEntry,
    ) {
        self.entries.lock().insert(key, entry);
    }

    pub(super) fn len(&self) -> usize {
        self.entries.lock().len()
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }

    #[cfg(test)]
    pub(super) fn load_for_key(
        &self,
        key: &BufferedScriptPreloadKey,
    ) -> Option<SharedScriptSourceLoad> {
        self.entries.lock().get(key).map(|entry| entry.load.clone())
    }

    fn preload_for_script(
        &self,
        script: &PreparedScript,
    ) -> Option<(SharedScriptSourceLoad, moli_fetch::RequestResourceType)> {
        if !matches!(script.source, ScriptSource::External) {
            return None;
        }
        let key = BufferedScriptPreloadKey::from_script(script)?;
        let entries = self.entries.lock();
        let entry = entries
            .get(&key)
            .filter(|entry| entry.request.matches_script(script))?;
        Some((entry.load.clone(), entry.request.resource_type_hint))
    }

    pub(crate) fn shared_preload_for_script(
        &self,
        script: &PreparedScript,
    ) -> Option<SharedScriptSourceLoad> {
        self.preload_for_script(script).map(|(load, _)| load)
    }
}

#[derive(Clone)]
pub(super) struct ServiceWorkerScriptPreloadContext {
    browser_context_runtime: RendererBrowserContextRuntime,
    client_id: ServiceWorkerClientId,
    document_url: Url,
    owner_wake: Option<RendererOwnerWakeSender>,
}

impl ServiceWorkerScriptPreloadContext {
    pub(super) fn new(
        browser_context_runtime: RendererBrowserContextRuntime,
        client_id: ServiceWorkerClientId,
        document_url: Url,
        owner_wake: Option<RendererOwnerWakeSender>,
    ) -> Self {
        Self {
            browser_context_runtime,
            client_id,
            document_url,
            owner_wake,
        }
    }
}

impl Default for BufferedDocumentPreloadState {
    fn default() -> Self {
        Self {
            entries: DocumentScriptPreloadStore::default(),
            document_character_set: "UTF-8".to_owned(),
            script_fetch_interception_enabled: false,
            response_csp_requires_parser_admission: false,
            owner_wake: None,
            resource_task_runner: None,
            main_document_scanner: None,
            insertion_scanner: None,
            meta_csp_preload_gate: MetaCspPreloadGate::default(),
            pending_script_preloads: Vec::new(),
            pending_stylesheet_preloads: Vec::new(),
            pending_image_preloads: Vec::new(),
        }
    }
}

impl BufferedDocumentPreloadState {
    pub(super) fn document_script_preload_store(&self) -> DocumentScriptPreloadStore {
        self.entries.clone()
    }

    pub(super) fn bind_resource_runtime(
        &mut self,
        owner_wake: Option<RendererOwnerWakeSender>,
        resource_task_runner: Option<crate::network::RendererResourceTaskRunner>,
    ) {
        self.owner_wake = owner_wake;
        self.resource_task_runner = resource_task_runner;
    }

    fn start_preloads_for_requests(
        &mut self,
        requests: Vec<BufferedScriptPreloadRequest>,
        loader: &ResourceRequestClient,
        service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
    ) {
        if requests.is_empty() {
            return;
        }
        let resource_task_runner = self
            .resource_task_runner
            .clone()
            .expect("script preloads require the navigation resource task runner");
        for request in requests {
            let key = request.cache_key();
            if self.entries.contains_key(&key) {
                continue;
            }

            let script = request.to_preload_script();
            let document_character_set = self.document_character_set.to_owned();
            let load = if let Some(context) = service_worker_context {
                crate::planning::spawn_service_worker_aware_external_script_source_load(
                    script,
                    loader.clone(),
                    resource_task_runner.clone(),
                    Some(document_character_set),
                    Some(request.resource_type_hint),
                    context.browser_context_runtime.clone(),
                    context.client_id,
                    context.document_url.clone(),
                    context.owner_wake.clone(),
                )
            } else {
                SharedScriptSourceLoad::spawn_with_request_resource_type_and_owner_wake(
                    script,
                    loader.clone(),
                    resource_task_runner.clone(),
                    Some(document_character_set),
                    Some(request.resource_type_hint),
                    self.owner_wake.clone(),
                )
            };
            self.entries
                .insert(key, BufferedScriptPreloadEntry { request, load });
        }
    }

    fn queue_script_preloads_for_owner_admission(
        &mut self,
        requests: Vec<BufferedScriptPreloadRequest>,
    ) {
        let remaining = MAX_PENDING_CSP_PRELOAD_CANDIDATES.saturating_sub(
            self.pending_script_preloads.len()
                + self.pending_stylesheet_preloads.len()
                + self.pending_image_preloads.len(),
        );
        self.pending_script_preloads
            .extend(requests.into_iter().take(remaining));
    }

    fn queue_stylesheet_preloads_for_owner_admission(
        &mut self,
        requests: Vec<BufferedStylesheetPreloadRequest>,
    ) {
        let remaining = MAX_PENDING_CSP_PRELOAD_CANDIDATES.saturating_sub(
            self.pending_script_preloads.len()
                + self.pending_stylesheet_preloads.len()
                + self.pending_image_preloads.len(),
        );
        self.pending_stylesheet_preloads
            .extend(requests.into_iter().take(remaining));
    }

    fn queue_image_preloads_for_owner_admission(
        &mut self,
        requests: Vec<BufferedImagePreloadRequest>,
    ) {
        let remaining = MAX_PENDING_CSP_PRELOAD_CANDIDATES.saturating_sub(
            self.pending_script_preloads.len()
                + self.pending_stylesheet_preloads.len()
                + self.pending_image_preloads.len(),
        );
        self.pending_image_preloads
            .extend(requests.into_iter().take(remaining));
    }

    fn schedule_preloads_for_requests(
        &mut self,
        requests: Vec<BufferedScriptPreloadRequest>,
        loader: &ResourceRequestClient,
        service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
    ) {
        let requires_owner_admission = self.script_fetch_interception_enabled
            || self.response_csp_requires_parser_admission
            || self.meta_csp_preload_gate.has_seen_meta_csp();
        let (owner_admitted, legacy_preloads): (Vec<_>, Vec<_>) =
            requests.into_iter().partition(|request| {
                requires_owner_admission || request.kind_hint == crate::types::ScriptKind::Module
            });
        // Native module graphs share fetches through the Document's module
        // map. They cannot enter the legacy SharedScriptSourceLoad cache: a
        // parser module would have no way to join that in-flight request and
        // would issue a second fetch. PageVm bootstrap drains these descriptors
        // into the native module map as soon as the Document owner exists.
        self.queue_script_preloads_for_owner_admission(owner_admitted);
        self.start_preloads_for_requests(legacy_preloads, loader, service_worker_context);
    }

    pub(super) fn set_document_character_set(&mut self, document_character_set: &str) {
        self.document_character_set = document_character_set.to_owned();
    }

    pub(super) fn set_script_fetch_interception_enabled(&mut self, enabled: bool) {
        self.script_fetch_interception_enabled = enabled;
    }

    pub(super) fn set_response_csp_requires_parser_admission(&mut self, required: bool) {
        self.response_csp_requires_parser_admission = required;
    }

    pub(super) fn append_to_main_document_scan(
        &mut self,
        final_url: &Url,
        html: &str,
        loader: &ResourceRequestClient,
    ) {
        self.append_to_main_document_scan_with_service_worker_context(
            final_url, html, loader, None,
        );
    }

    pub(super) fn append_to_main_document_scan_with_service_worker_context(
        &mut self,
        final_url: &Url,
        html: &str,
        loader: &ResourceRequestClient,
        service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
    ) {
        if html.is_empty() {
            return;
        }
        let scanner = self
            .main_document_scanner
            .get_or_insert_with(|| Box::new(IncrementalHtmlPreloadScanner::new(final_url.clone())));
        let batch = scanner.scan_chunk(html);
        self.meta_csp_preload_gate
            .note_scanner_seen(batch.discovered_meta_csp_count);
        self.schedule_preloads_for_requests(batch.script_requests, loader, service_worker_context);
        self.queue_stylesheet_preloads_for_owner_admission(batch.stylesheet_requests);
        self.queue_image_preloads_for_owner_admission(batch.image_requests);
    }

    pub(super) fn append_to_main_document_prebootstrap_scan_with_service_worker_context(
        &mut self,
        final_url: &Url,
        html: &str,
        loader: &ResourceRequestClient,
        service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
    ) {
        if html.is_empty() {
            return;
        }
        let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
        let scanner = self
            .main_document_scanner
            .get_or_insert_with(|| Box::new(IncrementalHtmlPreloadScanner::new(final_url.clone())));
        let batch = scanner.scan_chunk(html);
        self.meta_csp_preload_gate
            .note_scanner_seen(batch.discovered_meta_csp_count);
        let discovered_count = batch.script_requests.len();
        // Bootstrap-time scanning runs before the parser can execute inline code.
        // Keep it narrow to requests that can affect DCL: parser-blocking classic,
        // defer classic, and modules. Async classic scripts should keep their
        // normal background progress instead of competing with critical startup.
        let requests = batch
            .script_requests
            .into_iter()
            .filter(prebootstrap_preload_request_is_dcl_relevant)
            .collect::<Vec<_>>();
        let request_count = requests.len();
        self.schedule_preloads_for_requests(requests, loader, service_worker_context);
        self.queue_stylesheet_preloads_for_owner_admission(batch.stylesheet_requests);
        self.queue_image_preloads_for_owner_admission(batch.image_requests);
        if let Some(started) = timing_started
            && (request_count > 0 || started.elapsed().as_millis() > 1)
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                chunk_bytes = html.len(),
                discovered_count,
                request_count,
                entry_count = self.entries.len(),
                elapsed_ms = started.elapsed().as_millis(),
                elapsed_us = started.elapsed().as_micros(),
                stage = "main_document_prebootstrap_preload_scan_done",
            );
        }
    }

    pub(super) fn catch_up_main_document_scan_if_absent(
        &mut self,
        final_url: &Url,
        html: &str,
        loader: &ResourceRequestClient,
    ) {
        if self.main_document_scanner.is_some() || html.is_empty() {
            return;
        }
        self.append_to_main_document_scan(final_url, html, loader);
    }

    pub(super) fn append_to_insertion_scan_with_service_worker_context(
        &mut self,
        final_url: &Url,
        html: &str,
        loader: &ResourceRequestClient,
        service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
    ) {
        if html.is_empty() {
            return;
        }
        let scanner = self.insertion_scanner.get_or_insert_with(|| {
            Box::new(IncrementalHtmlPreloadScanner::new_conservative(
                final_url.clone(),
            ))
        });
        let batch = scanner.scan_chunk(html);
        self.meta_csp_preload_gate
            .note_scanner_seen(batch.discovered_meta_csp_count);
        self.schedule_preloads_for_requests(batch.script_requests, loader, service_worker_context);
    }

    pub(super) fn reset_insertion_scan(&mut self) {
        self.insertion_scanner = None;
    }

    pub(super) fn take_pending_stylesheet_preloads(
        &mut self,
    ) -> Vec<BufferedStylesheetPreloadRequest> {
        if !self.meta_csp_preload_gate.is_open() {
            return Vec::new();
        }
        std::mem::take(&mut self.pending_stylesheet_preloads)
    }

    fn take_pending_image_preloads(&mut self) -> Vec<BufferedImagePreloadRequest> {
        if !self.meta_csp_preload_gate.is_open() {
            return Vec::new();
        }
        std::mem::take(&mut self.pending_image_preloads)
    }

    fn take_pending_script_preloads(&mut self) -> Vec<BufferedScriptPreloadRequest> {
        if !self.meta_csp_preload_gate.is_open() {
            return Vec::new();
        }
        std::mem::take(&mut self.pending_script_preloads)
    }

    pub(super) fn note_parser_processed_meta_csp(&mut self, count: usize) {
        self.meta_csp_preload_gate.note_parser_processed(count);
    }

    pub(super) fn claim_pending_script_preload_for_parser(&mut self, script: &PreparedScript) {
        self.pending_script_preloads
            .retain(|request| !request.matches_script(script));
    }

    pub(super) fn claim_pending_stylesheet_preloads_for_parser(
        &mut self,
        inputs: &[DocumentOwnedBlockingStylesheetDiscoveryInput],
    ) {
        let claimed_resources = inputs
            .iter()
            .filter_map(|input| match input.signature() {
                DocumentBlockingStylesheetSignature::Link { url, options } => {
                    Some(options.resource_key(url.clone()))
                }
                DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { .. } => None,
            })
            .collect::<HashSet<_>>();
        if claimed_resources.is_empty() {
            return;
        }
        self.pending_stylesheet_preloads.retain(|request| {
            !claimed_resources.contains(&request.options.resource_key(request.url.clone()))
        });
    }

    #[cfg(test)]
    pub(super) fn meta_csp_counts_for_test(&self) -> (usize, usize) {
        (
            self.meta_csp_preload_gate.scanner_seen_count,
            self.meta_csp_preload_gate.parser_processed_count,
        )
    }

    #[cfg(test)]
    pub(super) fn pending_preload_counts_for_test(&self) -> (usize, usize) {
        (
            self.pending_script_preloads.len(),
            self.pending_stylesheet_preloads.len(),
        )
    }

    #[cfg(test)]
    pub(super) fn take_pending_script_preloads_for_test(
        &mut self,
    ) -> Vec<BufferedScriptPreloadRequest> {
        self.take_pending_script_preloads()
    }

    pub(super) fn shared_preload_for_script(
        &self,
        script: &PreparedScript,
    ) -> Option<SharedScriptSourceLoad> {
        self.entries.shared_preload_for_script(script)
    }

    pub(super) fn parser_blocking_preload_disposition_for_script(
        &mut self,
        script: &mut PreparedScript,
    ) -> ParserBlockingPreloadDisposition {
        if !matches!(script.source, ScriptSource::External) {
            return ParserBlockingPreloadDisposition::Missing;
        }

        let Some((load, resource_type_hint)) = self.entries.preload_for_script(script) else {
            return ParserBlockingPreloadDisposition::Missing;
        };
        if parser_blocking_consumer_should_refetch_pending_late_preload(
            script,
            resource_type_hint,
            &load,
        ) {
            return ParserBlockingPreloadDisposition::ExistingButNotReusable;
        }

        let Some(outcome) = load.try_outcome() else {
            return ParserBlockingPreloadDisposition::ReusableSourceLoad(load);
        };

        match outcome.source_result {
            Ok(source) => {
                *script = prepared_script_with_loaded_source(
                    script.clone(),
                    source,
                    outcome.source_bytes,
                );
                ParserBlockingPreloadDisposition::Ready(AppliedPreloadedScriptSource {
                    network_result: script_preload_network_result(outcome.network_result),
                })
            }
            // A completed fetch failure is still the terminal result for this
            // script request. Keep it attached to the PendingScript so the
            // parser owner can dispatch `error`; retrying or dropping it would
            // create a second source-of-truth for the same request.
            Err(_) => ParserBlockingPreloadDisposition::ReusableSourceLoad(load),
        }
    }

    pub(super) async fn apply_preloaded_source_to_script_if_available(
        &mut self,
        script: &mut PreparedScript,
        wait_for_pending: bool,
    ) -> Option<AppliedPreloadedScriptSource> {
        if !matches!(script.source, ScriptSource::External) {
            return None;
        }

        let (load, resource_type_hint) = self.entries.preload_for_script(script)?;
        let probe_started = (wait_for_pending && moli_trace::defer_wait_probe_enabled())
            .then(std::time::Instant::now);
        if probe_started.is_some() {
            tracing::info!(
                target: "moli_defer_wait_probe",
                url = %script.url,
                position = script.position,
                mode = ?script.mode,
                kind = ?script.kind,
                stage = "script_preload_wait_start",
            );
        }
        let outcome = if wait_for_pending {
            if parser_blocking_consumer_should_refetch_pending_late_preload(
                script,
                resource_type_hint,
                &load,
            ) {
                if probe_started.is_some() {
                    tracing::info!(
                        target: "moli_defer_wait_probe",
                        url = %script.url,
                        position = script.position,
                        mode = ?script.mode,
                        kind = ?script.kind,
                        stage = "script_preload_wait_skipped_for_late_parser_consumer",
                    );
                }
                None
            } else {
                Some(load.wait_outcome().await)
            }
        } else {
            load.try_outcome()
        };
        if let (Some(started), Some(outcome)) = (probe_started, outcome.as_ref()) {
            tracing::info!(
                target: "moli_defer_wait_probe",
                url = %script.url,
                position = script.position,
                mode = ?script.mode,
                kind = ?script.kind,
                ok = outcome.source_result.is_ok(),
                elapsed_ms = started.elapsed().as_millis(),
                stage = "script_preload_wait_done",
            );
        }
        match outcome {
            Some(outcome) => match outcome.source_result {
                Ok(source) => {
                    *script = prepared_script_with_loaded_source(
                        script.clone(),
                        source,
                        outcome.source_bytes,
                    );
                    Some(AppliedPreloadedScriptSource {
                        network_result: script_preload_network_result(outcome.network_result),
                    })
                }
                Err(_) => None,
            },
            None => None,
        }
    }
}

fn parser_blocking_consumer_should_refetch_pending_late_preload(
    script: &PreparedScript,
    resource_type_hint: moli_fetch::RequestResourceType,
    load: &SharedScriptSourceLoad,
) -> bool {
    // A late speculative preload is a medium-priority guess made by the preload
    // scanner after the document has already reached images. Chromium does not
    // let that pending speculative request block the synchronous parser path: if
    // the parser reaches the same classic script before the preload completes,
    // the parser-discovered request is treated as non-speculative and receives
    // parser-blocking priority. Returning true here lets Moli fall back to
    // its normal blocking-script fetch path. Already completed late preloads are
    // still reused, so this only changes the pending case.
    matches!(
        (
            script.kind,
            script.mode,
            resource_type_hint,
            load.try_outcome().is_none()
        ),
        (
            crate::types::ScriptKind::Classic,
            crate::types::ScriptMode::Normal,
            moli_fetch::RequestResourceType::LatePreloadScript,
            true
        )
    )
}

#[cfg(test)]
pub(super) fn collect_preloadable_external_script_urls_from_html(
    final_url: &Url,
    html: &str,
) -> Vec<Url> {
    collect_preloadable_external_script_requests_from_html(final_url, html)
        .into_iter()
        .map(|request| request.url)
        .collect()
}

#[cfg(test)]
pub(super) fn collect_preloadable_external_script_requests_from_html(
    final_url: &Url,
    html: &str,
) -> Vec<BufferedScriptPreloadRequest> {
    if html.is_empty() {
        return Vec::new();
    }

    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url.clone());
    let mut requests = scanner.scan_script_chunk(html);
    requests.extend(scanner.finish_script_scan());
    requests
}

#[cfg(test)]
pub(super) fn collect_preloadable_stylesheet_requests_from_html(
    final_url: &Url,
    html: &str,
) -> Vec<BufferedStylesheetPreloadRequest> {
    if html.is_empty() {
        return Vec::new();
    }

    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url.clone());
    let mut requests = scanner.scan_chunk(html).stylesheet_requests;
    requests.extend(scanner.finish_scan().stylesheet_requests);
    requests
}

fn buffered_script_kind_from_type(script_type: Option<&str>) -> crate::types::ScriptKind {
    moli_script::classify_script_kind(script_type)
}

fn html_attr_value(attrs: &[Attribute], name: &str) -> Option<String> {
    attrs
        .iter()
        .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(name))
        .map(|attr| attr.value.to_string())
}

fn html_attr_present(attrs: &[Attribute], name: &str) -> bool {
    attrs
        .iter()
        .any(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(name))
}

#[derive(Default)]
struct BufferedPreloadScannerRequests {
    script_requests: Vec<BufferedScriptPreloadRequest>,
    stylesheet_requests: Vec<BufferedStylesheetPreloadRequest>,
    image_requests: Vec<BufferedImagePreloadRequest>,
    seen: HashSet<BufferedPreloadKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum BufferedPreloadKey {
    Script(BufferedScriptPreloadKey),
    Stylesheet(BufferedStylesheetPreloadKey),
    Image(Url),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BufferedStylesheetPreloadKey {
    resource: StylesheetResourceKey,
    media: Option<String>,
    nonce: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BufferedScriptPreloadKey {
    url: Url,
    destination: BufferedScriptPreloadDestination,
    cross_origin: Option<String>,
    referrer_policy: Option<String>,
    charset: Option<String>,
    integrity: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BufferedScriptPreloadDestination {
    ClassicScript,
    ModuleScript,
}

impl BufferedScriptPreloadKey {
    pub(super) fn new(
        url: Url,
        kind: crate::types::ScriptKind,
        fetch_metadata: &crate::planning::ScriptFetchMetadata,
    ) -> Option<Self> {
        Some(Self {
            url,
            destination: BufferedScriptPreloadDestination::from_kind(kind)?,
            cross_origin: fetch_metadata.cross_origin.clone(),
            referrer_policy: fetch_metadata.referrer_policy.clone(),
            charset: fetch_metadata.charset.clone(),
            integrity: fetch_metadata.integrity.clone(),
        })
    }

    pub(crate) fn from_script(script: &PreparedScript) -> Option<Self> {
        Self::new(script.url.clone(), script.kind, &script.fetch_metadata)
    }
}

impl BufferedScriptPreloadDestination {
    fn from_kind(kind: crate::types::ScriptKind) -> Option<Self> {
        match kind {
            crate::types::ScriptKind::Classic => Some(Self::ClassicScript),
            crate::types::ScriptKind::Module => Some(Self::ModuleScript),
            crate::types::ScriptKind::ImportMap | crate::types::ScriptKind::DataBlock => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BufferedScriptPreloadRequest {
    pub(crate) url: Url,
    pub(super) initiator_url: Url,
    pub(super) kind_hint: crate::types::ScriptKind,
    pub(super) mode_hint: crate::types::ScriptMode,
    pub(super) resource_type_hint: moli_fetch::RequestResourceType,
    pub(super) fetch_metadata: crate::planning::ScriptFetchMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BufferedStylesheetPreloadRequest {
    pub(super) url: Url,
    pub(super) media: Option<String>,
    pub(super) options: StylesheetFetchOptions,
    pub(super) request_resource_type: moli_fetch::RequestResourceType,
    pub(super) link_preload: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BufferedImagePreloadRequest {
    pub(super) url: Url,
    pub(super) fetch_priority: Option<moli_fetch::FetchPriorityHint>,
}

impl BufferedStylesheetPreloadRequest {
    fn scanner_key(&self) -> BufferedStylesheetPreloadKey {
        BufferedStylesheetPreloadKey {
            resource: self.options.resource_key(self.url.clone()),
            media: self
                .media
                .as_deref()
                .map(str::trim)
                .filter(|media| !media.is_empty())
                .map(str::to_owned),
            nonce: self.options.nonce().map(str::to_owned),
        }
    }
}

pub(super) fn admit_stylesheet_preloads(
    page_vm: &mut PageVm,
    requests: Vec<BufferedStylesheetPreloadRequest>,
) {
    let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
    let discovered_count = requests.len();
    let mut admitted_count = 0_usize;
    let mut fetch_interception_count = 0_usize;
    let mut media_mismatch_count = 0_usize;
    let mut content_security_policy_count = 0_usize;
    for request in requests {
        match page_vm.admit_scanned_stylesheet_preload(
            request.url,
            request.media.as_deref(),
            request.options,
            request.request_resource_type,
            request.link_preload,
        ) {
            ScannedStylesheetAdmission::Admitted => admitted_count += 1,
            ScannedStylesheetAdmission::DeferredToParser(
                ScannedStylesheetDeferral::FetchInterception,
            ) => fetch_interception_count += 1,
            ScannedStylesheetAdmission::DeferredToParser(
                ScannedStylesheetDeferral::MediaMismatch,
            ) => media_mismatch_count += 1,
            ScannedStylesheetAdmission::DeferredToParser(
                ScannedStylesheetDeferral::ContentSecurityPolicy,
            ) => content_security_policy_count += 1,
        }
    }
    if let Some(started) = timing_started
        && discovered_count > 0
    {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            discovered_count,
            admitted_count,
            fetch_interception_count,
            media_mismatch_count,
            content_security_policy_count,
            elapsed_us = started.elapsed().as_micros(),
            stage = "scanned_stylesheet_preload_admission_done",
        );
    }
}

fn admit_image_preloads(page_vm: &mut PageVm, requests: Vec<BufferedImagePreloadRequest>) {
    let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
    let discovered_count = requests.len();
    let mut admitted_count = 0_usize;
    let mut disabled_count = 0_usize;
    let mut fetch_interception_count = 0_usize;
    let mut content_security_policy_count = 0_usize;
    let mut service_worker_count = 0_usize;
    for request in requests {
        match page_vm.admit_scanned_image_preload(request.url, request.fetch_priority) {
            ScannedImageAdmission::Admitted => admitted_count += 1,
            ScannedImageAdmission::DeferredToParser(ScannedImageDeferral::Disabled) => {
                disabled_count += 1
            }
            ScannedImageAdmission::DeferredToParser(ScannedImageDeferral::FetchInterception) => {
                fetch_interception_count += 1
            }
            ScannedImageAdmission::DeferredToParser(
                ScannedImageDeferral::ContentSecurityPolicy,
            ) => content_security_policy_count += 1,
            ScannedImageAdmission::DeferredToParser(ScannedImageDeferral::ServiceWorker) => {
                service_worker_count += 1
            }
        }
    }
    if let Some(started) = timing_started
        && discovered_count > 0
    {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            discovered_count,
            admitted_count,
            disabled_count,
            fetch_interception_count,
            content_security_policy_count,
            service_worker_count,
            elapsed_us = started.elapsed().as_micros(),
            stage = "scanned_image_preload_admission_done",
        );
    }
}

fn admit_script_preloads(
    page_vm: &mut PageVm,
    state: &mut BufferedDocumentPreloadState,
    requests: Vec<BufferedScriptPreloadRequest>,
    loader: &ResourceRequestClient,
    service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
) {
    let timing_started = moli_trace::cdp_nav_timing_enabled().then(std::time::Instant::now);
    let discovered_count = requests.len();
    let mut admitted = Vec::with_capacity(discovered_count);
    let mut script_execution_disabled_count = 0_usize;
    let mut fetch_interception_count = 0_usize;
    let mut content_security_policy_count = 0_usize;
    for request in requests {
        match page_vm.admit_scanned_script_preload(&request.url, &request.fetch_metadata) {
            ScannedScriptAdmission::Admitted => admitted.push(request),
            ScannedScriptAdmission::DeferredToParser(
                ScannedScriptDeferral::ScriptExecutionDisabled,
            ) => script_execution_disabled_count += 1,
            ScannedScriptAdmission::DeferredToParser(ScannedScriptDeferral::FetchInterception) => {
                fetch_interception_count += 1
            }
            ScannedScriptAdmission::DeferredToParser(
                ScannedScriptDeferral::ContentSecurityPolicy,
            ) => content_security_policy_count += 1,
        }
    }
    let admitted_count = admitted.len();
    let (native_modules, legacy_preloads): (Vec<_>, Vec<_>) = admitted
        .into_iter()
        .partition(|request| request.kind_hint == crate::types::ScriptKind::Module);
    state.start_preloads_for_requests(legacy_preloads, loader, service_worker_context);
    for request in native_modules {
        let request_url = request.url.clone();
        if let Err(error) = page_vm
            .vm_mut()
            .register_native_modulepreload_for_owner(request.into_native_module_preload())
        {
            page_vm.vm_mut().record_runtime_warning(format_args!(
                "preload-scanned module script `{request_url}` failed before fetch scheduling: {error}"
            ));
        }
    }
    if let Some(started) = timing_started
        && discovered_count > 0
    {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            discovered_count,
            admitted_count,
            script_execution_disabled_count,
            fetch_interception_count,
            content_security_policy_count,
            elapsed_us = started.elapsed().as_micros(),
            stage = "scanned_script_preload_admission_done",
        );
    }
}

pub(super) fn admit_pending_preloads(
    page_vm: &mut PageVm,
    state: &mut BufferedDocumentPreloadState,
    loader: &ResourceRequestClient,
    service_worker_context: Option<&ServiceWorkerScriptPreloadContext>,
) {
    let script_preloads = state.take_pending_script_preloads();
    let stylesheet_preloads = state.take_pending_stylesheet_preloads();
    let image_preloads = state.take_pending_image_preloads();
    admit_script_preloads(
        page_vm,
        state,
        script_preloads,
        loader,
        service_worker_context,
    );
    admit_stylesheet_preloads(page_vm, stylesheet_preloads);
    admit_image_preloads(page_vm, image_preloads);
}

impl BufferedScriptPreloadRequest {
    pub(crate) fn cache_key(&self) -> BufferedScriptPreloadKey {
        BufferedScriptPreloadKey::new(self.url.clone(), self.kind_hint, &self.fetch_metadata)
            .expect("buffered preload requests are only built for classic/module scripts")
    }

    pub(crate) fn to_preload_script(&self) -> PreparedScript {
        PreparedScript {
            position: 0,
            node_id: crate::dom::NodeId::new(0),
            kind: self.kind_hint,
            mode: self.mode_hint,
            source_kind: crate::types::ScriptSourceKind::External,
            fetch_metadata: self.fetch_metadata.clone(),
            source: ScriptSource::External,
            url: self.url.clone(),
            base_url: self.url.clone(),
            initiator_url: self.initiator_url.clone(),
            host_script_handle: None,
        }
    }

    fn into_native_module_preload(self) -> crate::module_runtime::NativeModuleSingleFetchRequest {
        debug_assert_eq!(self.kind_hint, crate::types::ScriptKind::Module);
        let module_key = crate::module_runtime::ModuleMapKey::java_script(self.url.clone());
        let fetch_metadata =
            crate::module_runtime::ModuleFetchMetadata::from_top_level_script_fetch_metadata(
                &self.fetch_metadata,
            );
        crate::module_runtime::NativeModuleSingleFetchRequest::new(
            self.url.clone(),
            self.url,
            self.initiator_url,
            module_key,
            fetch_metadata,
        )
    }

    pub(crate) fn matches_script(&self, script: &PreparedScript) -> bool {
        script.source_kind == crate::types::ScriptSourceKind::External
            && self.url == script.url
            && self.kind_hint == script.kind
            && BufferedScriptPreloadKey::from_script(script)
                .is_some_and(|script_key| self.cache_key() == script_key)
    }

    pub(crate) fn is_parser_blocking_classic(&self) -> bool {
        self.kind_hint == crate::types::ScriptKind::Classic
            && self.mode_hint == crate::types::ScriptMode::Normal
    }

    pub(crate) fn resource_type_hint(&self) -> moli_fetch::RequestResourceType {
        self.resource_type_hint
    }

    pub(crate) fn fetch_metadata(&self) -> &crate::planning::ScriptFetchMetadata {
        &self.fetch_metadata
    }

    #[cfg(test)]
    pub(super) fn request_metadata_for_testing(
        &self,
    ) -> (
        Option<&str>,
        Option<&str>,
        Option<&str>,
        Option<&str>,
        Option<&str>,
        Option<&str>,
    ) {
        (
            self.fetch_metadata.cross_origin.as_deref(),
            self.fetch_metadata.referrer_policy.as_deref(),
            self.fetch_metadata.charset.as_deref(),
            self.fetch_metadata.integrity.as_deref(),
            self.fetch_metadata.nonce.as_deref(),
            self.fetch_metadata
                .fetch_priority
                .as_ref()
                .map(|priority| priority.as_ref()),
        )
    }
}

fn buffered_script_mode_hint_from_attrs(
    kind: crate::types::ScriptKind,
    async_attribute_present: bool,
    defer_attribute_present: bool,
) -> crate::types::ScriptMode {
    match kind {
        crate::types::ScriptKind::Module => crate::types::ScriptMode::ModuleDefer,
        crate::types::ScriptKind::Classic => {
            if async_attribute_present {
                crate::types::ScriptMode::Async
            } else if defer_attribute_present {
                crate::types::ScriptMode::Defer
            } else {
                crate::types::ScriptMode::Normal
            }
        }
        crate::types::ScriptKind::ImportMap | crate::types::ScriptKind::DataBlock => {
            crate::types::ScriptMode::Normal
        }
    }
}

pub(super) fn prebootstrap_preload_request_is_dcl_relevant(
    request: &BufferedScriptPreloadRequest,
) -> bool {
    // Async classic scripts are observable before DCL when ready, but they do not
    // block DCL. Prebootstrap read-ahead is intentionally reserved for work that
    // can otherwise hold parser completion or defer/module execution.
    matches!(request.kind_hint, crate::types::ScriptKind::Module)
        || matches!(
            request.mode_hint,
            crate::types::ScriptMode::Normal | crate::types::ScriptMode::Defer
        )
}

pub(super) struct IncrementalHtmlPreloadScanner {
    input: BufferQueue,
    tokenizer: Tokenizer<HtmlPreloadScannerSink>,
}

pub(crate) struct IncrementalBufferedScriptPreloadScanner {
    input: BufferQueue,
    tokenizer: Tokenizer<HtmlPreloadScannerSink>,
}

#[derive(Default)]
pub(super) struct BufferedPreloadScanBatch {
    pub(super) script_requests: Vec<BufferedScriptPreloadRequest>,
    pub(super) stylesheet_requests: Vec<BufferedStylesheetPreloadRequest>,
    pub(super) image_requests: Vec<BufferedImagePreloadRequest>,
    pub(super) discovered_meta_csp_count: usize,
}

#[derive(Clone, Copy)]
enum MetaCspScannerMode {
    CollectDescriptors,
    StopAfterMeta,
}

impl IncrementalHtmlPreloadScanner {
    pub(super) fn new(initiator_url: Url) -> Self {
        Self::with_meta_csp_mode(initiator_url, MetaCspScannerMode::CollectDescriptors)
    }

    pub(super) fn new_conservative(initiator_url: Url) -> Self {
        Self::with_meta_csp_mode(initiator_url, MetaCspScannerMode::StopAfterMeta)
    }

    fn with_meta_csp_mode(initiator_url: Url, meta_csp_mode: MetaCspScannerMode) -> Self {
        Self {
            input: BufferQueue::default(),
            tokenizer: Tokenizer::new(
                HtmlPreloadScannerSink::new(initiator_url, meta_csp_mode),
                TokenizerOpts::default(),
            ),
        }
    }

    #[cfg(test)]
    pub(super) fn scan_script_chunk(&mut self, html: &str) -> Vec<BufferedScriptPreloadRequest> {
        self.scan_chunk(html).script_requests
    }

    pub(super) fn scan_chunk(&mut self, html: &str) -> BufferedPreloadScanBatch {
        if html.is_empty() {
            return BufferedPreloadScanBatch::default();
        }
        self.input.push_back(StrTendril::from(html));
        let _ = self.tokenizer.feed(&self.input);
        self.tokenizer.sink.take_new_requests()
    }

    #[cfg(test)]
    pub(super) fn finish_script_scan(&mut self) -> Vec<BufferedScriptPreloadRequest> {
        self.finish_scan().script_requests
    }

    #[cfg(test)]
    pub(super) fn finish_scan(&mut self) -> BufferedPreloadScanBatch {
        self.tokenizer.end();
        self.tokenizer.sink.take_new_requests()
    }
}

impl std::fmt::Debug for IncrementalBufferedScriptPreloadScanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncrementalBufferedScriptPreloadScanner")
            .finish_non_exhaustive()
    }
}

impl IncrementalBufferedScriptPreloadScanner {
    pub(crate) fn new(initiator_url: Url) -> Self {
        Self {
            input: BufferQueue::default(),
            tokenizer: Tokenizer::new(
                HtmlPreloadScannerSink::new(initiator_url, MetaCspScannerMode::StopAfterMeta),
                TokenizerOpts::default(),
            ),
        }
    }

    pub(crate) fn push_html(&mut self, html: &str) -> Vec<BufferedScriptPreloadRequest> {
        if html.is_empty() {
            return Vec::new();
        }
        self.input.push_back(StrTendril::from(html));
        let _ = self.tokenizer.feed(&self.input);
        self.tokenizer.sink.take_new_requests().script_requests
    }
}

struct HtmlPreloadScannerSink {
    final_url: Url,
    base_url: RefCell<Url>,
    has_valid_base: Cell<bool>,
    requests: RefCell<BufferedPreloadScannerRequests>,
    image_fetched: Cell<bool>,
    meta_csp_mode: MetaCspScannerMode,
    seen_meta_csp_count: Cell<usize>,
    reported_meta_csp_count: Cell<usize>,
    template_depth: Cell<usize>,
    picture_depth: Cell<usize>,
}

impl HtmlPreloadScannerSink {
    fn new(final_url: Url, meta_csp_mode: MetaCspScannerMode) -> Self {
        Self {
            base_url: RefCell::new(final_url.clone()),
            final_url,
            has_valid_base: Cell::new(false),
            requests: RefCell::new(BufferedPreloadScannerRequests::default()),
            image_fetched: Cell::new(false),
            meta_csp_mode,
            seen_meta_csp_count: Cell::new(0),
            reported_meta_csp_count: Cell::new(0),
            template_depth: Cell::new(0),
            picture_depth: Cell::new(0),
        }
    }

    fn take_new_requests(&self) -> BufferedPreloadScanBatch {
        let mut requests = self.requests.borrow_mut();
        let seen_meta_csp_count = self.seen_meta_csp_count.get();
        let discovered_meta_csp_count =
            seen_meta_csp_count.saturating_sub(self.reported_meta_csp_count.get());
        self.reported_meta_csp_count.set(seen_meta_csp_count);
        BufferedPreloadScanBatch {
            script_requests: std::mem::take(&mut requests.script_requests),
            stylesheet_requests: std::mem::take(&mut requests.stylesheet_requests),
            image_requests: std::mem::take(&mut requests.image_requests),
            discovered_meta_csp_count,
        }
    }

    fn maybe_collect_script_preload(&self, tag: &Tag) {
        if matches!(self.meta_csp_mode, MetaCspScannerMode::StopAfterMeta)
            && self.seen_meta_csp_count.get() != 0
        {
            return;
        }
        let Some(src) = html_attr_value(&tag.attrs, "src") else {
            return;
        };
        if src.trim().is_empty() {
            return;
        }

        let kind_hint =
            buffered_script_kind_from_type(html_attr_value(&tag.attrs, "type").as_deref());
        if html_attr_present(&tag.attrs, "nomodule")
            && matches!(kind_hint, crate::types::ScriptKind::Classic)
        {
            return;
        }
        match kind_hint {
            crate::types::ScriptKind::Classic | crate::types::ScriptKind::Module => {}
            crate::types::ScriptKind::ImportMap | crate::types::ScriptKind::DataBlock => {
                return;
            }
        }

        let Ok(url) = self.base_url.borrow().join(&src) else {
            return;
        };
        if url.scheme() == "data" {
            return;
        }
        let mut requests = self.requests.borrow_mut();
        let fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
            html_attr_value(&tag.attrs, "crossorigin").as_deref(),
            html_attr_value(&tag.attrs, "referrerpolicy").as_deref(),
            html_attr_value(&tag.attrs, "charset").as_deref(),
            html_attr_value(&tag.attrs, "integrity").as_deref(),
            html_attr_value(&tag.attrs, "nonce").as_deref(),
            html_attr_value(&tag.attrs, "fetchpriority").as_deref(),
        );
        let Some(key) = BufferedScriptPreloadKey::new(url.clone(), kind_hint, &fetch_metadata)
        else {
            return;
        };
        if requests.seen.insert(BufferedPreloadKey::Script(key)) {
            let mode_hint = buffered_script_mode_hint_from_attrs(
                kind_hint,
                html_attr_present(&tag.attrs, "async"),
                html_attr_present(&tag.attrs, "defer"),
            );
            requests.script_requests.push(BufferedScriptPreloadRequest {
                url,
                initiator_url: self.final_url.clone(),
                kind_hint,
                mode_hint,
                resource_type_hint: buffered_script_preload_resource_type(
                    kind_hint,
                    mode_hint,
                    self.image_fetched.get(),
                ),
                fetch_metadata,
            });
        }
    }

    fn maybe_update_base_url(&self, tag: &Tag) {
        if self.has_valid_base.get() {
            return;
        }
        let Some(href) = html_attr_value(&tag.attrs, "href") else {
            return;
        };
        let href = href.trim();
        if href.is_empty() {
            return;
        }
        let Ok(base_url) = self.base_url.borrow().join(href) else {
            return;
        };
        *self.base_url.borrow_mut() = base_url;
        self.has_valid_base.set(true);
    }

    fn maybe_collect_stylesheet_preload(&self, tag: &Tag) {
        if (matches!(self.meta_csp_mode, MetaCspScannerMode::StopAfterMeta)
            && self.seen_meta_csp_count.get() != 0)
            || html_attr_present(&tag.attrs, "disabled")
        {
            return;
        }
        let Some(rel) = html_attr_value(&tag.attrs, "rel") else {
            return;
        };
        let is_stylesheet = link_rel_includes_token(&rel, "stylesheet");
        let is_style_preload = link_rel_includes_token(&rel, "preload")
            && html_attr_value(&tag.attrs, "as")
                .as_deref()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("style"));
        if !is_stylesheet && !is_style_preload {
            return;
        }
        if !moli_web_mime::is_stylesheet_type_attribute(
            html_attr_value(&tag.attrs, "type").as_deref(),
        ) {
            return;
        }
        let Some(href) = html_attr_value(&tag.attrs, "href") else {
            return;
        };
        let href = href.trim();
        if href.is_empty() {
            return;
        }
        let Ok(url) = self.base_url.borrow().join(href) else {
            return;
        };
        if url.scheme() == "data" {
            return;
        }

        let request = BufferedStylesheetPreloadRequest {
            url,
            media: html_attr_value(&tag.attrs, "media"),
            options: StylesheetFetchOptions::from_link_attributes(
                html_attr_value(&tag.attrs, "crossorigin").as_deref(),
                html_attr_value(&tag.attrs, "referrerpolicy").as_deref(),
                html_attr_value(&tag.attrs, "integrity").as_deref(),
                html_attr_value(&tag.attrs, "nonce").as_deref(),
                html_attr_value(&tag.attrs, "charset").as_deref(),
                html_attr_value(&tag.attrs, "fetchpriority").as_deref(),
            ),
            request_resource_type: if self.image_fetched.get() {
                moli_fetch::RequestResourceType::LatePreloadCssStyleSheet
            } else {
                moli_fetch::RequestResourceType::CssStyleSheet
            },
            link_preload: is_style_preload,
        };
        let key = request.scanner_key();
        let mut requests = self.requests.borrow_mut();
        if requests.seen.insert(BufferedPreloadKey::Stylesheet(key)) {
            requests.stylesheet_requests.push(request);
        }
    }

    fn maybe_note_image_fetch(&self, tag: &Tag) {
        if tag.name.as_ref() == "img"
            && (html_attr_value(&tag.attrs, "src")
                .as_deref()
                .is_some_and(|src| !src.trim().is_empty())
                || html_attr_value(&tag.attrs, "srcset")
                    .as_deref()
                    .is_some_and(|srcset| !srcset.trim().is_empty()))
        {
            self.image_fetched.set(true);
        }
    }

    fn maybe_collect_image_preload(&self, tag: &Tag) {
        self.maybe_note_image_fetch(tag);
        if (matches!(self.meta_csp_mode, MetaCspScannerMode::StopAfterMeta)
            && self.seen_meta_csp_count.get() != 0)
            || self.picture_depth.get() != 0
            || html_attr_present(&tag.attrs, "crossorigin")
            || html_attr_value(&tag.attrs, "loading")
                .as_deref()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("lazy"))
            || html_attr_value(&tag.attrs, "srcset")
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        {
            return;
        }
        let Some(src) = html_attr_value(&tag.attrs, "src") else {
            return;
        };
        let src = src.trim();
        if src.is_empty() {
            return;
        }
        let Ok(url) = self.base_url.borrow().join(src) else {
            return;
        };
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        let mut requests = self.requests.borrow_mut();
        if requests.seen.insert(BufferedPreloadKey::Image(url.clone())) {
            requests.image_requests.push(BufferedImagePreloadRequest {
                url,
                fetch_priority: moli_fetch::FetchPriorityHint::from_attribute(
                    html_attr_value(&tag.attrs, "fetchpriority").as_deref(),
                ),
            });
        }
    }

    fn maybe_note_meta_csp(&self, tag: &Tag) {
        if tag.name.as_ref() == "meta"
            && html_attr_value(&tag.attrs, "http-equiv")
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("content-security-policy"))
        {
            self.seen_meta_csp_count
                .set(self.seen_meta_csp_count.get().saturating_add(1));
        }
    }
}

fn buffered_script_preload_resource_type(
    kind: crate::types::ScriptKind,
    mode: crate::types::ScriptMode,
    image_fetched: bool,
) -> moli_fetch::RequestResourceType {
    match (kind, mode) {
        (crate::types::ScriptKind::Classic, crate::types::ScriptMode::Normal) if image_fetched => {
            moli_fetch::RequestResourceType::LatePreloadScript
        }
        (crate::types::ScriptKind::Classic, crate::types::ScriptMode::Normal) => {
            moli_fetch::RequestResourceType::ParserBlockingScript
        }
        (
            crate::types::ScriptKind::Classic,
            crate::types::ScriptMode::Async | crate::types::ScriptMode::Defer,
        ) => moli_fetch::RequestResourceType::ClassicAsyncOrDeferScript,
        _ => moli_fetch::RequestResourceType::Script,
    }
}

impl TokenSink for HtmlPreloadScannerSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        let Token::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };
        if tag.name.as_ref() == "template" {
            match tag.kind {
                TagKind::StartTag => {
                    self.template_depth
                        .set(self.template_depth.get().saturating_add(1));
                }
                TagKind::EndTag => {
                    self.template_depth
                        .set(self.template_depth.get().saturating_sub(1));
                }
            }
            return TokenSinkResult::Continue;
        }
        if self.template_depth.get() > 0 {
            return if tag.kind == TagKind::StartTag {
                tokenizer_state_for_preload_scanner_tag(&tag)
            } else {
                TokenSinkResult::Continue
            };
        }
        if tag.name.as_ref() == "picture" {
            match tag.kind {
                TagKind::StartTag => {
                    self.picture_depth
                        .set(self.picture_depth.get().saturating_add(1));
                }
                TagKind::EndTag => {
                    self.picture_depth
                        .set(self.picture_depth.get().saturating_sub(1));
                }
            }
            return TokenSinkResult::Continue;
        }
        if tag.kind != TagKind::StartTag {
            return TokenSinkResult::Continue;
        }

        self.maybe_note_meta_csp(&tag);
        match tag.name.as_ref() {
            "base" => {
                self.maybe_update_base_url(&tag);
                TokenSinkResult::Continue
            }
            "script" => {
                self.maybe_collect_script_preload(&tag);
                TokenSinkResult::RawData(ScriptData)
            }
            "link" => {
                self.maybe_collect_stylesheet_preload(&tag);
                TokenSinkResult::Continue
            }
            "img" => {
                self.maybe_collect_image_preload(&tag);
                TokenSinkResult::Continue
            }
            _ => tokenizer_state_for_preload_scanner_tag(&tag),
        }
    }
}

fn tokenizer_state_for_preload_scanner_tag(tag: &Tag) -> TokenSinkResult<()> {
    match tag.name.as_ref() {
        "script" => TokenSinkResult::RawData(ScriptData),
        "noscript" | "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
            TokenSinkResult::RawData(Rawtext)
        }
        "title" | "textarea" => TokenSinkResult::RawData(Rcdata),
        "plaintext" => TokenSinkResult::Plaintext,
        _ => TokenSinkResult::Continue,
    }
}
