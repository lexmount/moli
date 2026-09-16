use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use moli_cookie_jar::BrowserCookieFacadeContext;
use parking_lot::Mutex;

use crate::network::loads::{
    ResourceLoadDisposition, ResourceLoadKind, ResourceLoadLease, ResourceLoadRegistry,
    ResourceLoadRegistryDiagnostics,
};
use crate::network::{
    RendererResourceTaskRunner, ResourceRequestClient, navigation::DocumentFetchContextSeed,
};

use super::DocumentFetchContext;

static NEXT_DOCUMENT_RESOURCE_LOADER_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DocumentResourceLoaderIdentity(u64);

impl DocumentResourceLoaderIdentity {
    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentResourceLoaderState {
    Active,
    Detaching,
    Detached,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentResourceLoaderDiagnostics {
    pub loader_id: u64,
    pub browser_resource_runtime_id: u64,
    pub state: &'static str,
    pub document_url: String,
    pub base_url: String,
    pub origin: String,
    pub active_ordinary_load_count: usize,
    pub active_keepalive_load_count: usize,
}

struct DocumentResourceLoaderAuthority {
    id: u64,
    lifecycle: Mutex<DocumentResourceLoaderLifecycle>,
    loads: ResourceLoadRegistry,
}

struct DocumentResourceNetwork {
    reporter: crate::runtime::RendererDocumentNetworkReporter,
    observer: Arc<dyn Fn(crate::runtime::RendererNetworkObservation) + Send + Sync>,
    script_completion: Arc<ScriptSourceCompletion>,
    frame_id: Option<String>,
}

type ScriptSourceCompletion = dyn Fn(
        crate::native_bridge::WindowDocumentOwner,
        crate::planning::SharedScriptSourceLoadCompleter,
        crate::planning::PreparedScriptSourceLoadOutcome,
    ) + Send
    + Sync;

struct DocumentResourceLoaderLifecycle {
    state: DocumentResourceLoaderState,
    context: DocumentFetchContext,
}

impl Drop for DocumentResourceLoaderAuthority {
    fn drop(&mut self) {
        // Registry retirement is normally explicit at the owner-transition
        // boundary. This final guard covers construction failures and runtime
        // teardown paths that drop the authority before publishing it.
        self.loads.begin_detach();
    }
}

/// Resource loading authority for one exact committed Document.
///
/// Clones share the authority/lifecycle and are safe to retain in asynchronous
/// work. Replacing the transport backend preserves that same authority; a new
/// Document must instead call [`Self::fork_for_document`].
#[derive(Clone)]
pub struct DocumentResourceLoader {
    request_client: ResourceRequestClient,
    authority: Arc<DocumentResourceLoaderAuthority>,
    network: Option<Arc<DocumentResourceNetwork>>,
    pub(super) service_worker: Option<Arc<super::resource::ServiceWorkerResourceFetcher>>,
}

/// Exact backend source selected when a new Document commits.
///
/// Network navigations must transfer their attempt-local seed. Synthetic
/// Documents such as initial `about:blank` and `srcdoc` must instead name the
/// already-authorized creator Document explicitly. Keeping these variants
/// separate prevents a missing navigation seed from silently falling back to
/// whichever Document happens to be ambient at commit time.
#[derive(Clone)]
pub(crate) enum DocumentResourceAuthoritySource {
    Navigation(DocumentFetchContextSeed),
    Inherited(DocumentResourceLoader),
}

impl DocumentResourceLoader {
    #[cfg(test)]
    pub(crate) fn for_test(
        request_client: ResourceRequestClient,
        task_runner: RendererResourceTaskRunner,
        document_url: url::Url,
    ) -> Self {
        let context = DocumentFetchContext::new(
            crate::native_bridge::WindowDocumentOwner::for_test(1),
            document_url.clone(),
            document_url.clone(),
            moli_url::origin_ascii_serialization(&document_url),
        );
        Self::new(request_client, task_runner, context)
    }

    pub(crate) fn identity(&self) -> DocumentResourceLoaderIdentity {
        DocumentResourceLoaderIdentity(self.authority.id)
    }

    pub(crate) fn new(
        mut request_client: ResourceRequestClient,
        task_runner: RendererResourceTaskRunner,
        context: DocumentFetchContext,
    ) -> Self {
        if request_client.browser_site_context().is_none() {
            let browser_site_context = BrowserCookieFacadeContext::default()
                .with_site_for_cookies_url(context.document_url())
                .with_top_frame_origin_url(context.document_url());
            request_client = request_client.with_browser_site_context(browser_site_context);
        }
        let loads = ResourceLoadRegistry::new(task_runner);
        Self {
            request_client: request_client.with_load_context(&loads),
            network: None,
            service_worker: None,
            authority: Arc::new(DocumentResourceLoaderAuthority {
                id: NEXT_DOCUMENT_RESOURCE_LOADER_ID
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1),
                lifecycle: Mutex::new(DocumentResourceLoaderLifecycle {
                    state: DocumentResourceLoaderState::Active,
                    context,
                }),
                loads,
            }),
        }
    }

    fn from_navigation_seed(context: DocumentFetchContext, seed: DocumentFetchContextSeed) -> Self {
        let mut request_client =
            ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
                seed.browser_resource_runtime(),
                seed.page_network_policy(),
            );
        if let Some(browser_site_context) = seed.shared_browser_site_context() {
            request_client = request_client.with_shared_browser_site_context(browser_site_context);
        }
        Self::new(request_client, seed.resource_task_runner(), context)
    }

    pub(crate) fn for_committed_document(
        context: DocumentFetchContext,
        source: DocumentResourceAuthoritySource,
    ) -> Self {
        match source {
            DocumentResourceAuthoritySource::Navigation(seed) => {
                assert_eq!(
                    seed.final_url(),
                    context.document_url(),
                    "committed Document context must match its navigation seed final URL"
                );
                Self::from_navigation_seed(context, seed)
            }
            DocumentResourceAuthoritySource::Inherited(loader) => loader.fork_for_document(context),
        }
    }

    pub(crate) fn fork_for_document(&self, context: DocumentFetchContext) -> Self {
        Self::new(self.request_client.clone(), self.task_runner(), context)
    }

    pub(crate) fn transfer_existing_loads_to(&self, replacement: &Self) -> usize {
        assert_eq!(
            self.state(),
            DocumentResourceLoaderState::Active,
            "only the active source Document can transfer existing loads"
        );
        assert_eq!(
            replacement.state(),
            DocumentResourceLoaderState::Active,
            "existing loads require an active replacement Document"
        );
        self.authority
            .loads
            .transfer_existing_loads_to(&replacement.authority.loads)
    }

    pub(crate) fn with_replacement_transport(&self, transport: ResourceRequestClient) -> Self {
        let mut request_client = self.request_client.clone();
        request_client.replace_browser_resource_runtime(transport.browser_resource_runtime());
        Self {
            request_client,
            authority: Arc::clone(&self.authority),
            network: self.network.clone(),
            service_worker: self.service_worker.clone(),
        }
    }

    pub(crate) fn begin_detach(&self) -> bool {
        let mut lifecycle = self.authority.lifecycle.lock();
        if lifecycle.state != DocumentResourceLoaderState::Active {
            return false;
        }
        lifecycle.state = DocumentResourceLoaderState::Detaching;
        drop(lifecycle);
        self.authority.loads.begin_detach();
        true
    }

    pub(crate) fn finish_detach(&self) {
        let mut lifecycle = self.authority.lifecycle.lock();
        if matches!(
            lifecycle.state,
            DocumentResourceLoaderState::Active | DocumentResourceLoaderState::Detaching
        ) {
            lifecycle.state = DocumentResourceLoaderState::Detached;
        }
    }

    pub(crate) fn accepts_ordinary_loads(&self) -> bool {
        self.state() == DocumentResourceLoaderState::Active
    }

    pub fn loader_id_for_diagnostics(&self) -> u64 {
        self.authority.id
    }

    pub(crate) fn shares_authority_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.authority, &other.authority)
    }

    pub(crate) fn request_client(&self) -> &ResourceRequestClient {
        &self.request_client
    }

    pub(crate) fn fetch_context(&self) -> DocumentFetchContext {
        self.authority.lifecycle.lock().context.clone()
    }

    pub(crate) fn owner(&self) -> crate::native_bridge::WindowDocumentOwner {
        self.authority.lifecycle.lock().context.owner()
    }

    pub(super) fn document_url(&self) -> url::Url {
        self.authority
            .lifecycle
            .lock()
            .context
            .document_url()
            .clone()
    }

    pub(crate) fn task_runner(&self) -> RendererResourceTaskRunner {
        self.authority.loads.task_runner()
    }

    pub(crate) fn spawn_resource_task(
        &self,
        task: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        self.task_runner().spawn(task);
    }

    pub(crate) fn frozen_request_client(&self) -> ResourceRequestClient {
        self.request_client.frozen_request_client()
    }

    pub(crate) fn bind_network(
        &mut self,
        reporter: crate::runtime::RendererDocumentNetworkReporter,
        completion: crate::page_task_queue::RendererResourceCompletionSender,
        frame_id: Option<String>,
    ) {
        self.network = Some(Arc::new(DocumentResourceNetwork {
            reporter,
            observer: completion.network_observer(),
            script_completion: Arc::new(move |owner, result, outcome| {
                let _ = completion.send_shared_script_source(owner, result, outcome);
            }),
            frame_id,
        }));
    }

    pub(crate) fn script_source_completion(
        &self,
    ) -> impl FnOnce(
        crate::planning::SharedScriptSourceLoadCompleter,
        crate::planning::PreparedScriptSourceLoadOutcome,
    ) + Send
    + 'static {
        #[cfg(not(test))]
        assert!(
            self.network.is_some(),
            "Document resource output must be bound before loading scripts"
        );
        let sender = self
            .network
            .as_ref()
            .map(|network| network.script_completion.clone());
        let owner = self.owner();
        move |completion, outcome| {
            if let Some(sender) = sender {
                sender(owner, completion, outcome);
            } else {
                #[cfg(test)]
                completion.finish(outcome);
                #[cfg(not(test))]
                unreachable!("Document script load must retain its completion route");
            }
        }
    }

    pub(crate) fn prepare_resource_request(
        &self,
        request: &moli_fetch::Request,
        resource_type: crate::types::SubresourceResourceType,
        initiator: crate::types::SubresourceRequestInitiatorType,
    ) -> Option<(
        ResourceLoadLease,
        Arc<super::super::ResourceTransfer>,
        crate::runtime::RendererNetworkObservation,
    )> {
        let load = self.register_load(
            resource_type.into(),
            ResourceLoadDisposition::Ordinary,
            None,
        )?;
        let binding = self.network.as_deref();
        #[cfg(not(test))]
        let binding = Some(
            binding
                .expect("committed Document resource authority must have its native output route"),
        );
        #[cfg(test)]
        let test_request = binding
            .is_none()
            .then(crate::runtime::RendererNetworkRequest::unobserved_for_test);
        let request_network = binding.and_then(|binding| binding.reporter.start_request());
        #[cfg(test)]
        let request_network = request_network.or(test_request);
        let request_network = request_network?;
        let observer = binding.map(|binding| binding.observer.clone());
        let (network, started) = super::super::ResourceTransfer::start(
            request_network,
            move |event| {
                if let Some(observer) = &observer {
                    observer(event);
                }
            },
            |network| self.resource_request_started(network, request, resource_type, initiator),
        );
        Some((load, network, started))
    }

    pub(crate) fn resource_request_started(
        &self,
        network: &crate::runtime::RendererNetworkRequest,
        request: &moli_fetch::Request,
        resource_type: crate::types::SubresourceResourceType,
        initiator: crate::types::SubresourceRequestInitiatorType,
    ) -> crate::types::SubresourceRequestStarted {
        crate::types::SubresourceRequestStarted::new(
            network.handle(),
            self.network
                .as_ref()
                .and_then(|binding| binding.frame_id.clone()),
            self.document_url(),
            request.url.clone(),
            request.method.clone(),
            request.request_headers.clone(),
            None,
            resource_type,
            initiator,
            None,
        )
        .with_request_body_bytes(request.body.clone())
    }

    pub(crate) fn register_load(
        &self,
        kind: ResourceLoadKind,
        disposition: ResourceLoadDisposition,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) -> Option<ResourceLoadLease> {
        if self.state() != DocumentResourceLoaderState::Active {
            return None;
        }
        self.authority.loads.register(
            kind,
            disposition,
            self.request_client.frozen_request_client(),
            cancel_handle,
        )
    }

    /// Registers a keepalive whose completion is network-only by
    /// construction.
    ///
    /// A CSP report may be derived from a detached keepalive redirect. In that
    /// case the source Document authority is intentionally no longer in the
    /// live owner registry, but its captured request policy remains the only
    /// valid authority. The new report therefore transfers directly to the
    /// browser runtime instead of consulting the replacement Document.
    pub(crate) fn register_network_only_keepalive_load(
        &self,
        kind: ResourceLoadKind,
        request_client: ResourceRequestClient,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) -> Option<ResourceLoadLease> {
        if let Some(load) = self.authority.loads.register(
            kind,
            ResourceLoadDisposition::Keepalive,
            request_client.clone(),
            cancel_handle.clone(),
        ) {
            return Some(load);
        }
        matches!(
            self.state(),
            DocumentResourceLoaderState::Detaching | DocumentResourceLoaderState::Detached
        )
        .then(|| {
            self.authority
                .loads
                .register_detached_keepalive(kind, request_client, cancel_handle)
        })
    }

    pub(crate) fn load_diagnostics(&self) -> ResourceLoadRegistryDiagnostics {
        self.authority.loads.diagnostics()
    }

    pub fn state(&self) -> DocumentResourceLoaderState {
        self.authority.lifecycle.lock().state
    }

    pub fn diagnostics(&self) -> DocumentResourceLoaderDiagnostics {
        let lifecycle = self.authority.lifecycle.lock();
        let loads = self.load_diagnostics();
        DocumentResourceLoaderDiagnostics {
            loader_id: self.authority.id,
            browser_resource_runtime_id: self
                .request_client
                .resource_runtime_diagnostics()
                .runtime_id,
            state: state_name(lifecycle.state),
            document_url: lifecycle.context.document_url().to_string(),
            base_url: lifecycle.context.base_url().to_string(),
            origin: lifecycle.context.origin().to_owned(),
            active_ordinary_load_count: loads.active_ordinary_load_count,
            active_keepalive_load_count: loads.active_keepalive_load_count,
        }
    }
}

impl std::fmt::Debug for DocumentResourceLoader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DocumentResourceLoader")
            .field("diagnostics", &self.diagnostics())
            .finish()
    }
}

fn state_name(state: DocumentResourceLoaderState) -> &'static str {
    match state {
        DocumentResourceLoaderState::Active => "active",
        DocumentResourceLoaderState::Detaching => "detaching",
        DocumentResourceLoaderState::Detached => "detached",
    }
}
