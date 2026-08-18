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
}

/// Inputs that exist before the initial committed Document owner is known.
///
/// This is deliberately not a resource authority: it cannot register loads,
/// carry lifecycle state, or escape as a `DocumentResourceLoader`. The
/// bootstrap path consumes it only after constructing the exact Document
/// owner, at which point [`Self::commit`] creates an already-active authority.
#[derive(Clone)]
pub(crate) struct DocumentResourceLoaderBootstrap {
    request_client: ResourceRequestClient,
    task_runner: RendererResourceTaskRunner,
}

impl DocumentResourceLoaderBootstrap {
    pub(crate) fn new(
        request_client: ResourceRequestClient,
        task_runner: RendererResourceTaskRunner,
    ) -> Self {
        Self {
            request_client,
            task_runner,
        }
    }

    pub(crate) fn commit(self, context: DocumentFetchContext) -> DocumentResourceLoader {
        DocumentResourceLoader::new(self.request_client, self.task_runner, context)
    }
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
            request_client,
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
        let mut request_client =
            ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
                transport.browser_resource_runtime(),
                self.request_client.page_network_policy(),
            );
        if let Some(browser_site_context) = self.request_client.shared_browser_site_context() {
            request_client = request_client.with_shared_browser_site_context(browser_site_context);
        }
        Self {
            request_client,
            authority: Arc::clone(&self.authority),
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

    #[cfg(test)]
    pub(crate) fn owner(&self) -> crate::native_bridge::WindowDocumentOwner {
        self.authority.lifecycle.lock().context.owner()
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
