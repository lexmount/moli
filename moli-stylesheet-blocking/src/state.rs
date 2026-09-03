use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use moli_dom::NodeId;
use moli_owner_queue::OwnerTaskSource;
use url::Url;

use crate::discovery::{
    DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheet,
    DocumentOwnedBlockingStylesheetDiscoveryInput, StylesheetBlockingReadView,
    collect_document_owned_blocking_stylesheets,
    collect_document_owned_blocking_stylesheets_before, document_node_precedes,
};
use crate::fetcher::{
    StylesheetFetch, StylesheetFetchNetworkResult, StylesheetFetchOptions, StylesheetFetchTerminal,
    StylesheetFetcher, StylesheetResourceKey,
};
use crate::types::{
    StylesheetBlockingEntry, StylesheetBlockingOperation, StylesheetBlockingResource,
    StylesheetBlockingStatus, StylesheetCompletion, StylesheetCompletionPayload,
    StylesheetFetchCompletion, StylesheetFetchEntry, StylesheetFetchSignature,
    StylesheetFetchStatus, StylesheetImportCompletion, StylesheetImportGraphFetchResult,
};

#[derive(Default)]
pub struct StylesheetBlockingState {
    owner_fetches: HashMap<NodeId, StylesheetFetchEntry>,
    blocking_entries: HashMap<NodeId, StylesheetBlockingEntry>,
    url_fetches: HashMap<(Url, StylesheetResourceKey), StylesheetFetch>,
    completion_source: OwnerTaskSource<StylesheetCompletion>,
    ready_network_results: VecDeque<StylesheetFetchNetworkResult>,
    // The stylesheet queue is local to document processing, not the renderer
    // resource-completion queue. Embedders can hook this to wake their outer
    // owner loop when an async stylesheet fetch becomes ready.
    completion_wake: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    /// Optional outer-owner publication boundary.
    ///
    /// A bound renderer moves the concrete completion into its stable HTML
    /// Networking source. Standalone users keep the local completion source
    /// and optional wake above; the two transports are mutually exclusive for
    /// every operation because the selected publisher is captured at spawn.
    completion_publisher: Option<Arc<dyn Fn(StylesheetCompletion) + Send + Sync + 'static>>,
}

impl StylesheetBlockingState {
    pub fn discover_from_document<F>(
        &mut self,
        fetcher: &F,
        document: &(impl StylesheetBlockingReadView + ?Sized),
    ) where
        F: StylesheetFetcher,
    {
        let blockers = collect_document_owned_blocking_stylesheets(document);
        let document_url = document
            .final_url_clone()
            .expect("parsed native dom must retain a document url");
        self.discover_from_blockers(fetcher, &document_url, blockers.iter());
    }

    pub fn discover_from_inputs<'a, F>(
        &mut self,
        fetcher: &F,
        document_url: &Url,
        inputs: impl IntoIterator<Item = &'a DocumentOwnedBlockingStylesheetDiscoveryInput>,
    ) where
        F: StylesheetFetcher,
    {
        for input in inputs {
            self.discover_input(
                fetcher,
                document_url,
                input.node_id(),
                input.signature().clone(),
            );
        }
    }

    fn discover_from_blockers<'a, F>(
        &mut self,
        fetcher: &F,
        document_url: &Url,
        blockers: impl IntoIterator<Item = &'a DocumentOwnedBlockingStylesheet>,
    ) where
        F: StylesheetFetcher,
    {
        for blocker in blockers {
            self.discover_input(
                fetcher,
                document_url,
                blocker.node_id(),
                blocker.signature().clone(),
            );
        }
    }

    fn discover_input<F>(
        &mut self,
        fetcher: &F,
        document_url: &Url,
        node_id: NodeId,
        signature: DocumentBlockingStylesheetSignature,
    ) where
        F: StylesheetFetcher,
    {
        if self
            .blocking_entries
            .get(&node_id)
            .is_some_and(|entry| entry.operation.signature() == &signature)
        {
            return;
        }

        let operation =
            StylesheetBlockingOperation::new(node_id, document_url.clone(), signature.clone());
        let resource = match &signature {
            DocumentBlockingStylesheetSignature::Link { url, options } => {
                let fetch = self.ensure_owner_fetch(
                    fetcher.clone(),
                    node_id,
                    document_url.clone(),
                    url.clone(),
                    options.clone(),
                );
                StylesheetBlockingResource::Link(fetch)
            }
            DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { urls } => {
                self.spawn_style_import_operation(fetcher.clone(), &operation, urls.clone());
                StylesheetBlockingResource::StyleImports {
                    status: StylesheetBlockingStatus::Pending,
                    completed_graph: None,
                }
            }
        };
        self.blocking_entries.insert(
            node_id,
            StylesheetBlockingEntry {
                operation,
                resource,
            },
        );
    }

    fn spawn_style_import_operation<F>(
        &self,
        fetcher: F,
        operation: &StylesheetBlockingOperation,
        urls: Vec<Url>,
    ) where
        F: StylesheetFetcher,
    {
        let sender = self.completion_source.sender();
        let completion_wake = self.completion_wake.clone();
        let completion_publisher = self.completion_publisher.clone();
        let operation = operation.clone();
        let document_url = operation.document_url().clone();
        let task_fetcher = fetcher.clone();
        fetcher.spawn_stylesheet_task(Box::pin(async move {
            let graph = task_fetcher
                .fetch_stylesheet_import_graph(document_url, urls)
                .await;
            let completion = StylesheetCompletion::style_imports(StylesheetImportCompletion {
                operation,
                graph,
            });
            if let Some(publisher) = completion_publisher {
                publisher(completion);
                return;
            }
            let sent = sender.send(completion);
            if sent.is_ok()
                && let Some(wake) = completion_wake
            {
                wake();
            }
        }));
    }

    pub fn discover_link_url<F>(
        &mut self,
        fetcher: &F,
        node_id: NodeId,
        document_url: Url,
        url: Url,
        options: StylesheetFetchOptions,
    ) -> StylesheetFetch
    where
        F: StylesheetFetcher,
    {
        let fetch = self.ensure_owner_fetch(
            fetcher.clone(),
            node_id,
            document_url.clone(),
            url.clone(),
            options.clone(),
        );
        self.rebind_blocking_link(node_id, document_url, &url, &options, &fetch);
        fetch
    }

    /// Returns the fetch already discovered for this owner, or starts the
    /// owner's current link processing when discovery has not seen it yet.
    ///
    /// Parser blocking discovery and renderer link processing are two views of
    /// the same initial owner operation. A real reprocess first invalidates the
    /// owner entry and creates a new client operation, but a compatible
    /// document-scoped physical resource remains reusable.
    pub fn adopt_or_begin_link_load<F>(
        &mut self,
        fetcher: &F,
        node_id: NodeId,
        document_url: Url,
        url: Url,
        options: StylesheetFetchOptions,
    ) -> StylesheetFetch
    where
        F: StylesheetFetcher,
    {
        let signature = StylesheetFetchSignature {
            url: url.clone(),
            options: options.clone(),
        };
        if let Some(entry) = self.owner_fetches.get(&node_id)
            && entry.signature == signature
        {
            return entry.fetch.clone();
        }
        let fetch = self.ensure_owner_fetch(
            fetcher.clone(),
            node_id,
            document_url.clone(),
            url.clone(),
            options.clone(),
        );
        self.rebind_blocking_link(node_id, document_url, &url, &options, &fetch);
        fetch
    }

    /// Starts or reuses a document-scoped stylesheet resource without binding
    /// it to a DOM owner.
    ///
    /// Speculative discovery is not a stylesheet client: it must not create a
    /// blocker or a load/error event. A later compatible owner adopts this
    /// exact resource through `adopt_or_begin_link_load`.
    pub fn preload_stylesheet<F>(
        &mut self,
        fetcher: &F,
        document_url: Url,
        url: Url,
        options: StylesheetFetchOptions,
    ) -> StylesheetFetch
    where
        F: StylesheetFetcher,
    {
        self.ensure_resource(fetcher.clone(), document_url, url, options)
    }

    fn rebind_blocking_link(
        &mut self,
        node_id: NodeId,
        document_url: Url,
        url: &Url,
        options: &StylesheetFetchOptions,
        fetch: &StylesheetFetch,
    ) {
        let Some(entry) = self.blocking_entries.get_mut(&node_id) else {
            return;
        };
        let DocumentBlockingStylesheetSignature::Link {
            url: blocked_url,
            options: blocked_options,
        } = entry.operation.signature()
        else {
            return;
        };
        let StylesheetBlockingResource::Link(current_fetch) = &entry.resource else {
            return;
        };
        if blocked_url != url || blocked_options != options || current_fetch.ptr_eq(fetch) {
            return;
        }
        let signature = DocumentBlockingStylesheetSignature::Link {
            url: url.clone(),
            options: options.clone(),
        };
        entry.operation = StylesheetBlockingOperation::new(node_id, document_url, signature);
        entry.resource = StylesheetBlockingResource::Link(fetch.clone());
    }

    pub async fn wait_for_blockers_before<F>(
        &mut self,
        fetcher: &F,
        document: &(impl StylesheetBlockingReadView + ?Sized),
        target_node_id: NodeId,
    ) where
        F: StylesheetFetcher,
    {
        self.discover_from_document(fetcher, document);
        let blockers = collect_document_owned_blocking_stylesheets_before(document, target_node_id);
        loop {
            let _ = self.drain_ready_completions();
            if !self.blocks_on_blockers(blockers.iter()) {
                return;
            }
            if !self.completion_source.wait_for_arrival().await {
                return;
            }
        }
    }

    fn ensure_owner_fetch<F>(
        &mut self,
        fetcher: F,
        node_id: NodeId,
        document_url: Url,
        url: Url,
        options: StylesheetFetchOptions,
    ) -> StylesheetFetch
    where
        F: StylesheetFetcher,
    {
        let signature = StylesheetFetchSignature {
            url: url.clone(),
            options: options.clone(),
        };
        if let Some(entry) = self.owner_fetches.get(&node_id)
            && entry.signature == signature
        {
            return entry.fetch.clone();
        }
        let fetch = self.ensure_resource(fetcher, document_url, url, options);
        self.owner_fetches.insert(
            node_id,
            StylesheetFetchEntry {
                signature,
                fetch: fetch.clone(),
            },
        );
        fetch
    }

    fn ensure_resource<F>(
        &mut self,
        fetcher: F,
        document_url: Url,
        url: Url,
        options: StylesheetFetchOptions,
    ) -> StylesheetFetch
    where
        F: StylesheetFetcher,
    {
        let resource_key = options.resource_key(url.clone());
        if let Some(url_fetch) = self
            .url_fetches
            .get(&(document_url.clone(), resource_key.clone()))
        {
            return url_fetch.clone();
        }
        let start_unix_millis = moli_time::unix_epoch_millis();
        let request_url = resource_key.request_url().clone();
        let fetch = StylesheetFetch::new(
            document_url.clone(),
            request_url.clone(),
            options.clone(),
            start_unix_millis,
        );
        let sender = self.completion_source.sender();
        let completion_wake = self.completion_wake.clone();
        let completion_publisher = self.completion_publisher.clone();
        let completion_fetch = fetch.clone();
        self.url_fetches
            .insert((document_url.clone(), resource_key), fetch.clone());
        let task_fetcher = fetcher.clone();
        fetcher.spawn_stylesheet_task(Box::pin(async move {
            let terminal = task_fetcher
                .fetch_stylesheet_resource(document_url, request_url, options)
                .await;
            let completion = StylesheetCompletion::fetch(StylesheetFetchCompletion {
                fetch: completion_fetch,
                terminal,
            });
            if let Some(publisher) = completion_publisher {
                publisher(completion);
                return;
            }
            let sent = sender.send(completion);
            if sent.is_ok()
                && let Some(wake) = completion_wake
            {
                wake();
            }
        }));
        fetch
    }

    pub fn status(&mut self, node_id: NodeId, url: &Url) -> Option<bool> {
        let _ = self.drain_ready_completions();
        self.status_without_draining(node_id, url)
    }

    pub fn status_without_draining(&self, node_id: NodeId, url: &Url) -> Option<bool> {
        let entry = self.owner_fetches.get(&node_id)?;
        if entry.signature.url != *url {
            return None;
        }
        match self.fetch_status(&entry.fetch) {
            StylesheetFetchStatus::Pending => None,
            StylesheetFetchStatus::Ready => Some(true),
            StylesheetFetchStatus::Failed => Some(false),
        }
    }

    pub fn status_for_fetch(&mut self, fetch: &StylesheetFetch) -> Option<bool> {
        let _ = self.drain_ready_completions();
        self.status_for_fetch_without_draining(fetch)
    }

    pub fn status_for_fetch_without_draining(&self, fetch: &StylesheetFetch) -> Option<bool> {
        match self.fetch_status(fetch) {
            StylesheetFetchStatus::Pending => None,
            StylesheetFetchStatus::Ready => Some(true),
            StylesheetFetchStatus::Failed => Some(false),
        }
    }

    pub fn owner_link_fetch(
        &self,
        node_id: NodeId,
        signature: &DocumentBlockingStylesheetSignature,
    ) -> Option<StylesheetFetch> {
        let DocumentBlockingStylesheetSignature::Link { .. } = signature else {
            return None;
        };
        let entry = self.owner_fetches.get(&node_id)?;
        (entry.signature
            == StylesheetFetchSignature::try_from(signature)
                .expect("link blocking signature must have a fetch signature"))
        .then(|| entry.fetch.clone())
    }

    /// Returns the signature when `fetch` is still the exact blocking
    /// `<link>` operation owned by `node_id`.
    ///
    /// Connected `<link>` load/error events outlive physical fetch completion.
    /// Embedders use the canonical owner/fetch identity to relate that posted
    /// event back to the script-blocking signature captured by the parser,
    /// without comparing URLs or inventing a second operation identifier.
    pub fn blocking_link_signature_for_fetch(
        &self,
        node_id: NodeId,
        fetch: &StylesheetFetch,
    ) -> Option<&DocumentBlockingStylesheetSignature> {
        let entry = self.blocking_entries.get(&node_id)?;
        let StylesheetBlockingResource::Link(blocking_fetch) = &entry.resource else {
            return None;
        };
        blocking_fetch
            .ptr_eq(fetch)
            .then(|| entry.operation.signature())
    }

    /// Whether `fetch` is still the exact blocking `<link>` operation owned by
    /// `node_id`.
    pub fn owns_blocking_link_fetch(&self, node_id: NodeId, fetch: &StylesheetFetch) -> bool {
        self.blocking_link_signature_for_fetch(node_id, fetch)
            .is_some()
    }

    fn fetch_status(&self, fetch: &StylesheetFetch) -> StylesheetBlockingStatus {
        fetch.status()
    }

    fn blocking_entry_status(&self, entry: &StylesheetBlockingEntry) -> StylesheetBlockingStatus {
        match &entry.resource {
            StylesheetBlockingResource::Link(fetch) => self.fetch_status(fetch),
            StylesheetBlockingResource::StyleImports { status, .. } => *status,
        }
    }

    pub fn blocking_operation(
        &self,
        node_id: NodeId,
        signature: &DocumentBlockingStylesheetSignature,
    ) -> Option<StylesheetBlockingOperation> {
        let entry = self.blocking_entries.get(&node_id)?;
        (entry.operation.signature() == signature).then(|| entry.operation.clone())
    }

    pub fn status_for_blocking_operation(
        &self,
        operation: &StylesheetBlockingOperation,
    ) -> Option<StylesheetBlockingStatus> {
        let entry = self.blocking_entries.get(&operation.node_id())?;
        if !entry.operation.ptr_eq(operation) {
            return None;
        }
        Some(self.blocking_entry_status(entry))
    }

    pub fn blocks_script(
        &self,
        document: &(impl StylesheetBlockingReadView + ?Sized),
        node_id: NodeId,
    ) -> bool {
        let blockers = collect_document_owned_blocking_stylesheets_before(document, node_id);
        self.blocks_on_blockers(blockers.iter())
            || self
                .blocking_entries
                .iter()
                .any(|(stylesheet_node_id, entry)| {
                    self.blocking_entry_status(entry) == StylesheetBlockingStatus::Pending
                        && document_node_precedes(document, *stylesheet_node_id, node_id)
                })
    }

    fn blocks_on_blockers<'a>(
        &self,
        blockers: impl IntoIterator<Item = &'a DocumentOwnedBlockingStylesheet>,
    ) -> bool {
        let pending_signatures = self.pending_signatures();
        blockers.into_iter().any(|blocker| {
            self.blocking_entries
                .get(&blocker.node_id())
                .is_some_and(|entry| {
                    entry.operation.signature() == blocker.signature()
                        && self.blocking_entry_status(entry) == StylesheetBlockingStatus::Pending
                })
                || pending_signatures.contains(blocker.signature())
        })
    }

    pub fn blocks_on_signatures<'a>(
        &self,
        signatures: impl IntoIterator<Item = &'a DocumentBlockingStylesheetSignature>,
    ) -> bool {
        let pending_signatures = self.pending_signatures();
        signatures
            .into_iter()
            .any(|signature| pending_signatures.contains(signature))
    }

    fn pending_signatures(&self) -> HashSet<DocumentBlockingStylesheetSignature> {
        self.blocking_entries
            .values()
            .filter(|entry| self.blocking_entry_status(entry) == StylesheetBlockingStatus::Pending)
            .map(|entry| entry.operation.signature().clone())
            .collect()
    }

    pub fn has_any_pending_entries(&self) -> bool {
        self.blocking_entries
            .values()
            .any(|entry| self.blocking_entry_status(entry) == StylesheetBlockingStatus::Pending)
    }

    pub fn invalidate_node(&mut self, node_id: NodeId) {
        self.owner_fetches.remove(&node_id);
        self.blocking_entries.remove(&node_id);
    }

    pub fn set_completion_wake(
        &mut self,
        completion_wake: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    ) {
        self.completion_wake = completion_wake;
    }

    /// Route future fetch completions to an outer stable owner source.
    ///
    /// Existing fetches retain the publisher captured when they started, so a
    /// Document replacement cannot silently rebind an old completion to the
    /// new owner generation.
    pub fn set_completion_publisher(
        &mut self,
        completion_publisher: Option<Arc<dyn Fn(StylesheetCompletion) + Send + Sync + 'static>>,
    ) {
        self.completion_publisher = completion_publisher;
    }

    pub fn apply_completion(
        &mut self,
        completion: StylesheetCompletion,
    ) -> Option<StylesheetFetch> {
        match completion.into_payload() {
            StylesheetCompletionPayload::Fetch(completion) => {
                let StylesheetFetchCompletion { fetch, terminal } = *completion;
                let terminal = Arc::new(terminal);
                if !fetch.finish(Arc::clone(&terminal)) || !fetch.claim_physical_observation() {
                    return None;
                }
                self.ready_network_results
                    .push_back(StylesheetFetchNetworkResult {
                        fetch: Some(fetch.clone()),
                        blocking_operation: None,
                        document_url: fetch.document_url().clone(),
                        request_url: fetch.request_url().clone(),
                        owner_node_ids: Vec::new(),
                        start_unix_millis: fetch.start_unix_millis(),
                        terminal,
                    });
                Some(fetch)
            }
            StylesheetCompletionPayload::StyleImports(StylesheetImportCompletion {
                operation,
                graph,
            }) => {
                let accepted = self
                    .blocking_entries
                    .get(&operation.node_id())
                    .is_some_and(|entry| entry.operation.ptr_eq(&operation));
                let owner_node_ids = accepted
                    .then_some(vec![operation.node_id()])
                    .unwrap_or_default();
                let retained_graph = Arc::new(graph.clone());
                let (successful, network_results) = graph.into_parts();
                for network_result in network_results {
                    let (request_url, start_unix_millis, terminal) = network_result.into_parts();
                    self.ready_network_results
                        .push_back(StylesheetFetchNetworkResult {
                            fetch: None,
                            blocking_operation: Some(operation.clone()),
                            document_url: operation.document_url().clone(),
                            request_url,
                            owner_node_ids: owner_node_ids.clone(),
                            start_unix_millis,
                            terminal: Arc::new(terminal),
                        });
                }
                if accepted
                    && let Some(entry) = self.blocking_entries.get_mut(&operation.node_id())
                    && let StylesheetBlockingResource::StyleImports {
                        status,
                        completed_graph,
                    } = &mut entry.resource
                {
                    *status = if successful {
                        StylesheetBlockingStatus::Ready
                    } else {
                        StylesheetBlockingStatus::Failed
                    };
                    *completed_graph = Some(retained_graph);
                }
                None
            }
        }
    }

    pub fn take_completed_import_graph_for_blocking_operation(
        &mut self,
        operation: &StylesheetBlockingOperation,
    ) -> Option<Arc<StylesheetImportGraphFetchResult>> {
        let entry = self.blocking_entries.get_mut(&operation.node_id())?;
        if !entry.operation.ptr_eq(operation) {
            return None;
        }
        let StylesheetBlockingResource::StyleImports {
            completed_graph, ..
        } = &mut entry.resource
        else {
            return None;
        };
        completed_graph.take()
    }

    pub fn drain_ready_completions(&mut self) -> Vec<StylesheetFetch> {
        let mut completed_fetches = Vec::new();
        while let Some(completion) = self.completion_source.pop_front() {
            completed_fetches.extend(self.apply_completion(completion));
        }
        completed_fetches
    }

    pub async fn wait_for_completion_arrival_without_timeout(&mut self) -> bool {
        self.completion_source.wait_for_arrival().await
    }

    pub fn enqueue_completion_for_testing(&self, node_id: NodeId, url: Url, successful: bool) {
        let fetch = self
            .owner_fetches
            .get(&node_id)
            .map(|entry| entry.fetch.clone())
            .expect("testing completion requires a discovered stylesheet entry");
        assert_eq!(fetch.request_url(), &url);
        let completion = StylesheetCompletion::fetch(StylesheetFetchCompletion {
            fetch,
            terminal: if successful {
                StylesheetFetchTerminal::ready(
                    moli_page_types::NavigationResponse::from_text_body(
                        Url::parse("https://example.com/style.css").expect("static url"),
                        200,
                        Vec::new(),
                        String::new(),
                    ),
                    true,
                )
            } else {
                StylesheetFetchTerminal::network_error("stylesheet load failed")
            },
        });
        if let Some(publisher) = &self.completion_publisher {
            publisher(completion);
        } else {
            let _ = self.completion_source.sender().send(completion);
        }
    }

    pub fn take_ready_network_results(&mut self) -> Vec<StylesheetFetchNetworkResult> {
        self.ready_network_results.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        future::Future,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use moli_dom::native::NativeNodeId;

    use super::*;
    use crate::discovery::StylesheetElementRead;
    use crate::fetcher::StylesheetFetcher;

    struct TestStylesheetDocument {
        url: Url,
    }

    impl StylesheetBlockingReadView for TestStylesheetDocument {
        fn stylesheet_element(&self, node_id: NativeNodeId) -> Option<StylesheetElementRead> {
            match node_id.index() {
                1 => Some(StylesheetElementRead::parser_created_html_link_for_test(
                    "print.css",
                    Some("print"),
                )),
                2 => Some(StylesheetElementRead::parser_created_html_link_for_test(
                    "screen.css",
                    None,
                )),
                _ => None,
            }
        }

        fn child_ids(&self, _node_id: NativeNodeId) -> Vec<NativeNodeId> {
            Vec::new()
        }

        fn text_content(&self, _node_id: NativeNodeId) -> Option<String> {
            None
        }

        fn final_url_clone(&self) -> Option<Url> {
            Some(self.url.clone())
        }

        fn document_base_url_clone(&self) -> Option<Url> {
            Some(self.url.clone())
        }

        fn document_node_id(&self) -> NativeNodeId {
            NativeNodeId::new(0)
        }

        fn document_is_quirks_mode(&self) -> bool {
            false
        }

        fn document_order_stylesheet_candidate_ids_before(
            &self,
            _target_node_id: Option<NodeId>,
        ) -> Vec<NativeNodeId> {
            vec![NativeNodeId::new(1), NativeNodeId::new(2)]
        }
    }

    #[derive(Clone)]
    struct ImmediateStylesheetFetcher;

    fn spawn_test_stylesheet_task(task: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        tokio::spawn(task);
    }

    impl StylesheetFetcher for ImmediateStylesheetFetcher {
        fn spawn_stylesheet_task(&self, task: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
            spawn_test_stylesheet_task(task);
        }

        fn fetch_stylesheet_resource(
            &self,
            _document_url: Url,
            url: Url,
            _options: StylesheetFetchOptions,
        ) -> Pin<Box<dyn Future<Output = StylesheetFetchTerminal> + Send + 'static>> {
            Box::pin(async move {
                StylesheetFetchTerminal::ready(
                    moli_page_types::NavigationResponse::from_text_body(
                        url,
                        200,
                        vec![("content-type".to_owned(), "text/css".to_owned())],
                        "body { color: black; }".to_owned(),
                    ),
                    true,
                )
            })
        }
    }

    #[derive(Clone)]
    struct PendingStylesheetFetcher;

    impl StylesheetFetcher for PendingStylesheetFetcher {
        fn spawn_stylesheet_task(&self, task: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
            spawn_test_stylesheet_task(task);
        }

        fn fetch_stylesheet_resource(
            &self,
            _document_url: Url,
            _url: Url,
            _options: StylesheetFetchOptions,
        ) -> Pin<Box<dyn Future<Output = StylesheetFetchTerminal> + Send + 'static>> {
            Box::pin(std::future::pending())
        }
    }

    #[derive(Clone)]
    struct GatedStylesheetFetcher {
        gate: Arc<tokio::sync::Notify>,
        started: Arc<tokio::sync::Notify>,
        fetch_count: Arc<AtomicUsize>,
    }

    impl StylesheetFetcher for GatedStylesheetFetcher {
        fn spawn_stylesheet_task(&self, task: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
            spawn_test_stylesheet_task(task);
        }

        fn fetch_stylesheet_resource(
            &self,
            _document_url: Url,
            url: Url,
            _options: StylesheetFetchOptions,
        ) -> Pin<Box<dyn Future<Output = StylesheetFetchTerminal> + Send + 'static>> {
            let gate = Arc::clone(&self.gate);
            let started = Arc::clone(&self.started);
            let fetch_count = Arc::clone(&self.fetch_count);
            Box::pin(async move {
                fetch_count.fetch_add(1, Ordering::SeqCst);
                started.notify_one();
                gate.notified().await;
                StylesheetFetchTerminal::ready(
                    moli_page_types::NavigationResponse::from_text_body(
                        url,
                        200,
                        vec![("content-type".to_owned(), "text/css".to_owned())],
                        "body { color: black; }".to_owned(),
                    ),
                    true,
                )
            })
        }
    }

    #[derive(Clone)]
    struct CountingStylesheetFetcher {
        fetch_count: Arc<AtomicUsize>,
    }

    impl StylesheetFetcher for CountingStylesheetFetcher {
        fn spawn_stylesheet_task(&self, task: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
            spawn_test_stylesheet_task(task);
        }

        fn fetch_stylesheet_resource(
            &self,
            _document_url: Url,
            url: Url,
            _options: StylesheetFetchOptions,
        ) -> Pin<Box<dyn Future<Output = StylesheetFetchTerminal> + Send + 'static>> {
            let fetch_count = Arc::clone(&self.fetch_count);
            Box::pin(async move {
                fetch_count.fetch_add(1, Ordering::SeqCst);
                StylesheetFetchTerminal::ready(
                    moli_page_types::NavigationResponse::from_text_body(
                        url,
                        200,
                        vec![("content-type".to_owned(), "text/css".to_owned())],
                        "body { color: black; }".to_owned(),
                    ),
                    true,
                )
            })
        }
    }

    #[test]
    fn stylesheet_fetch_identity_is_exact_and_clone_stable() {
        let document_url = Url::parse("https://example.com/").unwrap();
        let stylesheet_url = Url::parse("https://example.com/style.css").unwrap();
        let first = StylesheetFetch::new(
            document_url.clone(),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
            1.0,
        );
        let first_clone = first.clone();
        let distinct = StylesheetFetch::new(
            document_url,
            stylesheet_url,
            StylesheetFetchOptions::default(),
            1.0,
        );
        let mut identities = HashSet::new();

        assert!(identities.insert(first.identity()));
        assert!(!identities.insert(first_clone.identity()));
        assert!(identities.insert(distinct.identity()));
        assert_eq!(identities.len(), 2);
    }

    #[tokio::test]
    async fn stylesheet_fetch_completion_signals_configured_wake() {
        let mut state = StylesheetBlockingState::default();
        let wake_count = Arc::new(AtomicUsize::new(0));
        let wake_count_for_callback = Arc::clone(&wake_count);
        state.set_completion_wake(Some(Arc::new(move || {
            wake_count_for_callback.fetch_add(1, Ordering::SeqCst);
        })));

        let node_id = NodeId::new(1);
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/style.css").expect("static stylesheet url");

        state.discover_link_url(
            &ImmediateStylesheetFetcher,
            node_id,
            document_url,
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );

        assert_eq!(
            state.status_without_draining(node_id, &stylesheet_url),
            None
        );
        assert!(state.wait_for_completion_arrival_without_timeout().await);
        let _ = state.drain_ready_completions();

        assert_eq!(wake_count.load(Ordering::SeqCst), 1);
        assert_eq!(state.status(node_id, &stylesheet_url), Some(true));
    }

    #[tokio::test]
    async fn outer_completion_publisher_replaces_local_queue_and_wake() {
        let mut state = StylesheetBlockingState::default();
        let wake_count = Arc::new(AtomicUsize::new(0));
        let wake_count_for_callback = Arc::clone(&wake_count);
        state.set_completion_wake(Some(Arc::new(move || {
            wake_count_for_callback.fetch_add(1, Ordering::SeqCst);
        })));
        let (completion_tx, mut completion_rx) = tokio::sync::mpsc::unbounded_channel();
        state.set_completion_publisher(Some(Arc::new(move |completion| {
            completion_tx
                .send(completion)
                .expect("outer completion test receiver must stay open");
        })));

        let node_id = NodeId::new(1);
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/style.css").expect("static stylesheet url");
        state.discover_link_url(
            &ImmediateStylesheetFetcher,
            node_id,
            document_url,
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );

        let completion = completion_rx
            .recv()
            .await
            .expect("outer publisher should receive the concrete completion");
        assert!(state.completion_source.is_empty());
        assert_eq!(wake_count.load(Ordering::SeqCst), 0);
        assert_eq!(
            state.status_without_draining(node_id, &stylesheet_url),
            None
        );

        let completed_fetch = state
            .apply_completion(completion)
            .expect("first terminal must return its exact fetch");
        assert!(
            state
                .owner_fetches
                .get(&node_id)
                .is_some_and(|entry| entry.fetch.ptr_eq(&completed_fetch))
        );
        assert_eq!(state.status(node_id, &stylesheet_url), Some(true));
    }

    #[tokio::test]
    async fn in_flight_fetch_keeps_the_publisher_captured_at_start() {
        let mut state = StylesheetBlockingState::default();
        let (first_tx, mut first_rx) = tokio::sync::mpsc::unbounded_channel();
        state.set_completion_publisher(Some(Arc::new(move |completion| {
            first_tx
                .send(completion)
                .expect("first completion receiver must stay open");
        })));
        let gate = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(tokio::sync::Notify::new());
        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fetcher = GatedStylesheetFetcher {
            gate: Arc::clone(&gate),
            started,
            fetch_count,
        };
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/style.css").expect("static stylesheet url");
        state.discover_link_url(
            &fetcher,
            NodeId::new(1),
            document_url,
            stylesheet_url,
            StylesheetFetchOptions::default(),
        );

        let (replacement_tx, mut replacement_rx) = tokio::sync::mpsc::unbounded_channel();
        state.set_completion_publisher(Some(Arc::new(move |completion| {
            replacement_tx
                .send(completion)
                .expect("replacement completion receiver must stay open");
        })));
        gate.notify_one();

        let _completion = first_rx
            .recv()
            .await
            .expect("in-flight fetch must retain its original publisher");
        assert!(matches!(
            replacement_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(state.completion_source.is_empty());
    }

    #[tokio::test]
    async fn duplicate_stylesheet_url_shares_fetch_and_marks_all_nodes_ready() {
        let mut state = StylesheetBlockingState::default();
        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fetcher = CountingStylesheetFetcher {
            fetch_count: Arc::clone(&fetch_count),
        };
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/shared.css").expect("static stylesheet url");
        let first_node_id = NodeId::new(1);
        let second_node_id = NodeId::new(2);

        state.discover_link_url(
            &fetcher,
            first_node_id,
            document_url.clone(),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );
        state.discover_link_url(
            &fetcher,
            second_node_id,
            document_url,
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );

        assert!(state.wait_for_completion_arrival_without_timeout().await);
        let _ = state.drain_ready_completions();

        assert_eq!(
            fetch_count.load(Ordering::SeqCst),
            1,
            "duplicate stylesheet links should share the in-flight URL fetch"
        );
        assert_eq!(state.status(first_node_id, &stylesheet_url), Some(true));
        assert_eq!(state.status(second_node_id, &stylesheet_url), Some(true));
        let results = state.take_ready_network_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].request_url, stylesheet_url);
        assert!(
            results[0].owner_node_ids.is_empty(),
            "a shared fetch terminal is a physical observation, not a client delivery"
        );
    }

    #[tokio::test]
    async fn stale_fetch_completion_cannot_resolve_a_reprocessed_node() {
        let mut state = StylesheetBlockingState::default();
        let node_id = NodeId::new(1);
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let first_url = Url::parse("https://example.com/a.css").expect("static stylesheet url");
        let second_url = Url::parse("https://example.com/b.css").expect("static stylesheet url");
        let first_fetch = state.adopt_or_begin_link_load(
            &ImmediateStylesheetFetcher,
            node_id,
            document_url.clone(),
            first_url.clone(),
            StylesheetFetchOptions::default(),
        );
        let second_fetch = state.adopt_or_begin_link_load(
            &ImmediateStylesheetFetcher,
            node_id,
            document_url.clone(),
            second_url.clone(),
            StylesheetFetchOptions::default(),
        );
        assert!(!first_fetch.ptr_eq(&second_fetch));

        state
            .completion_source
            .sender()
            .send(StylesheetCompletion::fetch(StylesheetFetchCompletion {
                fetch: first_fetch,
                terminal: StylesheetFetchTerminal::ready(
                    moli_page_types::NavigationResponse::from_text_body(
                        first_url,
                        200,
                        vec![("content-type".to_owned(), "text/css".to_owned())],
                        "body { color: red; }".to_owned(),
                    ),
                    true,
                ),
            }))
            .expect("stale completion should enqueue");
        let _ = state.drain_ready_completions();

        assert_eq!(state.status_without_draining(node_id, &second_url), None);
        let results = state.take_ready_network_results();
        assert_eq!(results.len(), 1);
        assert!(results[0].owner_node_ids.is_empty());
    }

    #[tokio::test]
    async fn stale_style_import_completion_cannot_resolve_same_signature_aba() {
        use crate::discovery::{
            DocumentOwnedBlockingStylesheet, DocumentOwnedBlockingStylesheetCandidate,
            DocumentOwnedBlockingStylesheetDiscoveryInput,
        };

        fn input(node_id: NodeId, url: Url) -> DocumentOwnedBlockingStylesheetDiscoveryInput {
            let candidate = DocumentOwnedBlockingStylesheetCandidate::ParserCreatedStyleImport {
                node_id,
                urls: vec![url],
            };
            DocumentOwnedBlockingStylesheetDiscoveryInput::from(
                &DocumentOwnedBlockingStylesheet::from_candidate(&candidate),
            )
        }

        let mut state = StylesheetBlockingState::default();
        let node_id = NodeId::new(1);
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let first_url = Url::parse("https://example.com/a.css").expect("static stylesheet url");
        let middle_url = Url::parse("https://example.com/b.css").expect("static stylesheet url");
        let first_a_input = input(node_id, first_url.clone());
        let middle_b_input = input(node_id, middle_url);

        state.discover_from_inputs(&PendingStylesheetFetcher, &document_url, [&first_a_input]);
        let first_a = state
            .blocking_operation(node_id, first_a_input.signature())
            .expect("first A operation");
        state.discover_from_inputs(&PendingStylesheetFetcher, &document_url, [&middle_b_input]);
        state.discover_from_inputs(&PendingStylesheetFetcher, &document_url, [&first_a_input]);
        let current_a = state
            .blocking_operation(node_id, first_a_input.signature())
            .expect("current A operation");
        assert!(!first_a.ptr_eq(&current_a));

        state
            .completion_source
            .sender()
            .send(StylesheetCompletion::style_imports(
                StylesheetImportCompletion {
                    operation: first_a.clone(),
                    graph: StylesheetImportGraphFetchResult::new(
                        false,
                        vec![crate::types::StylesheetImportNetworkResult::new(
                            first_url.clone(),
                            1.0,
                            StylesheetFetchTerminal::network_error("stale A completion"),
                        )],
                    ),
                },
            ))
            .expect("stale import completion should enqueue");
        let _ = state.drain_ready_completions();

        assert_eq!(state.status_for_blocking_operation(&first_a), None);
        assert!(
            state
                .take_completed_import_graph_for_blocking_operation(&first_a)
                .is_none(),
            "a stale ABA completion must not publish retained install authority",
        );
        assert_eq!(
            state.status_for_blocking_operation(&current_a),
            Some(StylesheetBlockingStatus::Pending)
        );
        let stale_result = state
            .take_ready_network_results()
            .into_iter()
            .next()
            .expect("stale network observation");
        assert!(stale_result.owner_node_ids.is_empty());

        state
            .completion_source
            .sender()
            .send(StylesheetCompletion::style_imports(
                StylesheetImportCompletion {
                    operation: current_a.clone(),
                    graph: StylesheetImportGraphFetchResult::new(
                        false,
                        vec![crate::types::StylesheetImportNetworkResult::new(
                            first_url,
                            2.0,
                            StylesheetFetchTerminal::network_error("current A completion"),
                        )],
                    ),
                },
            ))
            .expect("current import completion should enqueue");
        let _ = state.drain_ready_completions();

        assert_eq!(
            state.status_for_blocking_operation(&current_a),
            Some(StylesheetBlockingStatus::Failed)
        );
        let retained = state
            .take_completed_import_graph_for_blocking_operation(&current_a)
            .expect("the current operation must publish one retained graph");
        assert!(!retained.successful());
        assert_eq!(retained.network_results().len(), 1);
        assert!(
            state
                .take_completed_import_graph_for_blocking_operation(&current_a)
                .is_none(),
            "retained import installation authority must be one-shot",
        );
        assert_eq!(
            state.take_ready_network_results()[0].owner_node_ids,
            vec![node_id]
        );
    }

    #[tokio::test]
    async fn completed_parser_discovery_is_adopted_by_initial_owner_processing() {
        let mut state = StylesheetBlockingState::default();
        let node_id = NodeId::new(1);
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/a.css").expect("static stylesheet url");
        let discovered_fetch = state.discover_link_url(
            &ImmediateStylesheetFetcher,
            node_id,
            document_url.clone(),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );
        state.enqueue_completion_for_testing(node_id, stylesheet_url.clone(), true);
        let _ = state.drain_ready_completions();

        let owner_fetch = state.adopt_or_begin_link_load(
            &ImmediateStylesheetFetcher,
            node_id,
            document_url,
            stylesheet_url,
            StylesheetFetchOptions::default(),
        );

        assert!(discovered_fetch.ptr_eq(&owner_fetch));
    }

    #[tokio::test]
    async fn completed_fetch_is_sticky_for_a_late_compatible_owner() {
        let mut state = StylesheetBlockingState::default();
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/shared.css").expect("static stylesheet url");
        let first_fetch = state.discover_link_url(
            &ImmediateStylesheetFetcher,
            NodeId::new(1),
            document_url.clone(),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );
        assert!(state.wait_for_completion_arrival_without_timeout().await);
        let _ = state.drain_ready_completions();

        let late_fetch = state.adopt_or_begin_link_load(
            &ImmediateStylesheetFetcher,
            NodeId::new(2),
            document_url,
            stylesheet_url,
            StylesheetFetchOptions::default(),
        );

        assert!(first_fetch.ptr_eq(&late_fetch));
        assert!(
            late_fetch
                .terminal()
                .is_some_and(|terminal| terminal.is_ready())
        );
    }

    #[tokio::test]
    async fn ownerless_pending_resource_does_not_block_and_is_adopted_by_a_late_owner() {
        let mut state = StylesheetBlockingState::default();
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/speculative.css").expect("static stylesheet url");

        let speculative = state.preload_stylesheet(
            &PendingStylesheetFetcher,
            document_url.clone(),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );
        assert!(
            !state.has_any_pending_entries(),
            "an ownerless physical resource must not enter the blocking set"
        );

        let adopted = state.adopt_or_begin_link_load(
            &PendingStylesheetFetcher,
            NodeId::new(1),
            document_url,
            stylesheet_url,
            StylesheetFetchOptions::default(),
        );

        assert!(speculative.ptr_eq(&adopted));
        assert!(
            !state.has_any_pending_entries(),
            "binding a non-blocking client must not manufacture a blocker"
        );
    }

    #[tokio::test]
    async fn invalidating_an_unbound_owner_preserves_the_ownerless_resource() {
        let mut state = StylesheetBlockingState::default();
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/speculative.css").expect("static stylesheet url");
        let node_id = NodeId::new(1);

        let speculative = state.preload_stylesheet(
            &PendingStylesheetFetcher,
            document_url.clone(),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );
        state.invalidate_node(node_id);
        let adopted = state.adopt_or_begin_link_load(
            &PendingStylesheetFetcher,
            node_id,
            document_url,
            stylesheet_url,
            StylesheetFetchOptions::default(),
        );

        assert!(
            speculative.ptr_eq(&adopted),
            "owner invalidation cannot discard a resource that was never bound to that owner"
        );
    }

    #[tokio::test]
    async fn invalidated_owner_readmission_reuses_the_in_flight_document_resource() {
        let mut state = StylesheetBlockingState::default();
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/shared.css").expect("static stylesheet url");
        let node_id = NodeId::new(1);
        let gate = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(tokio::sync::Notify::new());
        let fetch_count = Arc::new(AtomicUsize::new(0));
        let fetcher = GatedStylesheetFetcher {
            gate: Arc::clone(&gate),
            started: Arc::clone(&started),
            fetch_count: Arc::clone(&fetch_count),
        };

        let first = state.adopt_or_begin_link_load(
            &fetcher,
            node_id,
            document_url.clone(),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );
        started.notified().await;
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);

        state.invalidate_node(node_id);
        let readmitted = state.adopt_or_begin_link_load(
            &fetcher,
            node_id,
            document_url,
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );

        assert!(
            first.ptr_eq(&readmitted),
            "owner reprocessing must replace the client without duplicating a compatible physical request"
        );
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);

        gate.notify_one();
        assert!(state.wait_for_completion_arrival_without_timeout().await);
        let completed = state.drain_ready_completions();
        assert_eq!(completed.len(), 1);
        assert!(completed[0].ptr_eq(&readmitted));
        assert_eq!(state.status(node_id, &stylesheet_url), Some(true));
        assert_eq!(state.take_ready_network_results().len(), 1);
    }

    #[tokio::test]
    async fn ownerless_terminal_is_sticky_for_a_late_compatible_owner() {
        let mut state = StylesheetBlockingState::default();
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/speculative.css").expect("static stylesheet url");

        let speculative = state.preload_stylesheet(
            &ImmediateStylesheetFetcher,
            document_url.clone(),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );
        assert!(state.wait_for_completion_arrival_without_timeout().await);
        let _ = state.drain_ready_completions();
        let physical_results = state.take_ready_network_results();
        assert_eq!(physical_results.len(), 1);
        assert!(physical_results[0].owner_node_ids.is_empty());

        let adopted = state.adopt_or_begin_link_load(
            &ImmediateStylesheetFetcher,
            NodeId::new(1),
            document_url,
            stylesheet_url,
            StylesheetFetchOptions::default(),
        );

        assert!(speculative.ptr_eq(&adopted));
        assert!(
            adopted
                .terminal()
                .is_some_and(|terminal| terminal.is_ready())
        );
        assert!(
            state.take_ready_network_results().is_empty(),
            "late client attachment must not replay physical network facts"
        );
    }

    #[tokio::test]
    async fn ownerless_resources_keep_request_compatibility_boundaries() {
        let mut state = StylesheetBlockingState::default();
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://cdn.example.com/speculative.css").expect("static stylesheet url");
        let plain = StylesheetFetchOptions::default();
        let anonymous = StylesheetFetchOptions::from_link_attributes(
            Some("anonymous"),
            None,
            None,
            None,
            None,
            None,
        );

        let plain_fetch = state.preload_stylesheet(
            &PendingStylesheetFetcher,
            document_url.clone(),
            stylesheet_url.clone(),
            plain,
        );
        let anonymous_fetch = state.preload_stylesheet(
            &PendingStylesheetFetcher,
            document_url,
            stylesheet_url,
            anonymous,
        );

        assert!(!plain_fetch.ptr_eq(&anonymous_fetch));
    }

    #[tokio::test]
    async fn physical_key_keeps_mime_processing_compatibility_boundary() {
        let mut state = StylesheetBlockingState::default();
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/shared.css").expect("static stylesheet url");
        let standard = StylesheetFetchOptions::default();
        let quirks = StylesheetFetchOptions::default().with_quirks_mode_mime_compatibility(true);

        let standard_fetch = state.preload_stylesheet(
            &PendingStylesheetFetcher,
            document_url.clone(),
            stylesheet_url.clone(),
            standard,
        );
        let quirks_fetch = state.preload_stylesheet(
            &PendingStylesheetFetcher,
            document_url,
            stylesheet_url,
            quirks,
        );

        assert!(!standard_fetch.ptr_eq(&quirks_fetch));
    }

    #[tokio::test]
    async fn physical_key_ignores_fragment_nonce_and_fetch_priority() {
        let mut state = StylesheetBlockingState::default();
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let first_url =
            Url::parse("https://example.com/shared.css#first").expect("static stylesheet url");
        let second_url =
            Url::parse("https://example.com/shared.css#second").expect("static stylesheet url");
        let first_options = StylesheetFetchOptions::from_link_attributes(
            None,
            Some("no-referrer"),
            Some("sha256-shared"),
            Some("first-nonce"),
            Some("utf-8"),
            Some("low"),
        );
        let second_options = StylesheetFetchOptions::from_link_attributes(
            None,
            Some("no-referrer"),
            Some("sha256-shared"),
            Some("second-nonce"),
            Some("utf-8"),
            Some("high"),
        );

        let first = state.discover_link_url(
            &PendingStylesheetFetcher,
            NodeId::new(1),
            document_url.clone(),
            first_url,
            first_options,
        );
        let second = state.discover_link_url(
            &PendingStylesheetFetcher,
            NodeId::new(2),
            document_url,
            second_url,
            second_options,
        );

        assert!(first.ptr_eq(&second));
        assert_eq!(
            first.request_url().as_str(),
            "https://example.com/shared.css"
        );
    }

    #[tokio::test]
    async fn physical_key_keeps_request_compatibility_boundaries() {
        let mut state = StylesheetBlockingState::default();
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://cdn.example.com/shared.css").expect("static stylesheet url");
        let plain = StylesheetFetchOptions::from_link_attributes(
            None,
            Some("no-referrer"),
            Some("sha256-first"),
            None,
            Some("utf-8"),
            None,
        );
        let anonymous = StylesheetFetchOptions::from_link_attributes(
            Some("anonymous"),
            Some("no-referrer"),
            Some("sha256-first"),
            None,
            Some("utf-8"),
            None,
        );
        let different_integrity = StylesheetFetchOptions::from_link_attributes(
            None,
            Some("no-referrer"),
            Some("sha256-second"),
            None,
            Some("utf-8"),
            None,
        );

        let plain_fetch = state.discover_link_url(
            &PendingStylesheetFetcher,
            NodeId::new(1),
            document_url.clone(),
            stylesheet_url.clone(),
            plain,
        );
        let anonymous_fetch = state.discover_link_url(
            &PendingStylesheetFetcher,
            NodeId::new(2),
            document_url.clone(),
            stylesheet_url.clone(),
            anonymous,
        );
        let different_integrity_fetch = state.discover_link_url(
            &PendingStylesheetFetcher,
            NodeId::new(3),
            document_url,
            stylesheet_url,
            different_integrity,
        );

        assert!(!plain_fetch.ptr_eq(&anonymous_fetch));
        assert!(!plain_fetch.ptr_eq(&different_integrity_fetch));
    }

    #[tokio::test]
    async fn duplicate_terminal_keeps_first_terminal_and_one_physical_observation() {
        let mut state = StylesheetBlockingState::default();
        let node_id = NodeId::new(1);
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/shared.css").expect("static stylesheet url");
        let fetch = state.discover_link_url(
            &PendingStylesheetFetcher,
            node_id,
            document_url,
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );

        state.enqueue_completion_for_testing(node_id, stylesheet_url.clone(), true);
        state.enqueue_completion_for_testing(node_id, stylesheet_url, false);
        let _ = state.drain_ready_completions();

        assert!(fetch.terminal().is_some_and(|terminal| terminal.is_ready()));
        assert_eq!(state.take_ready_network_results().len(), 1);
    }

    #[tokio::test]
    async fn completed_same_url_reprocess_reuses_the_sticky_document_resource() {
        let mut state = StylesheetBlockingState::default();
        let node_id = NodeId::new(1);
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://example.com/a.css").expect("static stylesheet url");
        let first_fetch = state.adopt_or_begin_link_load(
            &ImmediateStylesheetFetcher,
            node_id,
            document_url.clone(),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );
        state.enqueue_completion_for_testing(node_id, stylesheet_url.clone(), true);
        let _ = state.drain_ready_completions();
        state.invalidate_node(node_id);

        let second_fetch = state.adopt_or_begin_link_load(
            &ImmediateStylesheetFetcher,
            node_id,
            document_url,
            stylesheet_url,
            StylesheetFetchOptions::default(),
        );

        assert!(first_fetch.ptr_eq(&second_fetch));
        assert!(
            second_fetch
                .terminal()
                .is_some_and(|terminal| terminal.is_ready())
        );
    }

    #[tokio::test]
    async fn same_url_in_different_document_fetch_contexts_is_not_shared() {
        let mut state = StylesheetBlockingState::default();
        let stylesheet_url =
            Url::parse("https://cdn.example.com/a.css").expect("static stylesheet url");
        let first_fetch = state.adopt_or_begin_link_load(
            &ImmediateStylesheetFetcher,
            NodeId::new(1),
            Url::parse("https://first.example/").expect("static document url"),
            stylesheet_url.clone(),
            StylesheetFetchOptions::default(),
        );
        let second_fetch = state.adopt_or_begin_link_load(
            &ImmediateStylesheetFetcher,
            NodeId::new(2),
            Url::parse("https://second.example/").expect("static document url"),
            stylesheet_url,
            StylesheetFetchOptions::default(),
        );

        assert!(!first_fetch.ptr_eq(&second_fetch));
    }

    #[tokio::test]
    async fn same_url_with_different_link_request_options_is_not_shared() {
        let mut state = StylesheetBlockingState::default();
        let node_id = NodeId::new(1);
        let document_url = Url::parse("https://example.com/").expect("static document url");
        let stylesheet_url =
            Url::parse("https://cdn.example.com/a.css").expect("static stylesheet url");
        let anonymous = StylesheetFetchOptions::from_link_attributes(
            Some("anonymous"),
            Some("no-referrer"),
            Some("sha256-first"),
            None,
            None,
            Some("low"),
        );
        let credentials = StylesheetFetchOptions::from_link_attributes(
            Some("use-credentials"),
            Some("origin"),
            Some("sha256-second"),
            None,
            None,
            Some("high"),
        );

        let first_fetch = state.adopt_or_begin_link_load(
            &PendingStylesheetFetcher,
            node_id,
            document_url.clone(),
            stylesheet_url.clone(),
            anonymous.clone(),
        );
        let second_fetch = state.adopt_or_begin_link_load(
            &PendingStylesheetFetcher,
            node_id,
            document_url,
            stylesheet_url,
            credentials.clone(),
        );

        assert!(!first_fetch.ptr_eq(&second_fetch));
        assert_eq!(first_fetch.options(), &anonymous);
        assert_eq!(second_fetch.options(), &credentials);
    }

    #[tokio::test]
    async fn document_wide_discovery_skips_load_only_media_stylesheets() {
        let document = TestStylesheetDocument {
            url: Url::parse("https://example.com/page.html").expect("static document url"),
        };
        let mut state = StylesheetBlockingState::default();

        state.discover_from_document(&ImmediateStylesheetFetcher, &document);
        assert!(state.wait_for_completion_arrival_without_timeout().await);
        let _ = state.drain_ready_completions();

        let results = state.take_ready_network_results();
        assert_eq!(results.len(), 1);
        assert!(results[0].request_url.as_str().ends_with("/screen.css"));
    }
}
