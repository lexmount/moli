use std::collections::HashMap;

use tokio::sync::oneshot;
use url::Url;

use crate::{
    browser::{
        BrowserContextId, DocumentId, NavigationId, NavigationRequestLoadPolicy,
        RendererPageResidenceIdentity, WebContentsHandle, WebContentsId,
        web_contents::{
            AdmittedDocumentMaterialization as PhysicalDocumentMaterialization,
            AdmittedInitialDocumentBuild as PhysicalInitialDocumentBuild,
            AdmittedNavigationLoad as PhysicalNavigationLoad,
            BuiltInitialDocument as PhysicalBuiltInitialDocument, ClaimedNavigationRequest,
            CommittedDocumentInfo, CommittedInitialDocument as PhysicalCommittedInitialDocument,
            DocumentNavigationDestination, InheritedDocumentPolicy,
            InitialDocumentAdmission as PhysicalInitialDocumentAdmission, InitialDocumentBuildKey,
            InitialDocumentPageBuildWaiter,
            InterceptedNavigationLoad as PhysicalInterceptedNavigationLoad,
            NavigationInterceptionPermit,
            PreparedDocumentNavigation as PhysicalPreparedDocumentNavigation,
            PreparedNavigationResponse as PhysicalPreparedNavigationResponse,
        },
    },
    page::SubresourceAuthCredentials,
    runtime::{
        BuiltDocumentPage, CommittedDocumentResourceSource, ExternalRawDocumentBodyStream,
        NavigationStreamingRawResponse, PageVmInitStage, RendererReplyBoundary,
        RendererReservedServiceWorkerClient,
    },
};
use moli_fetch::{
    NetworkFetchResult, NetworkObservationJournal, RawResponse, StreamingRawResponse,
};

use super::{BrowserContextHandle, BrowserHandle, BrowserOwnerMessage, PendingDocumentRetirement};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct NavigationWorkId(u64);

struct ContextWork<T> {
    context: BrowserContextId,
    value: T,
}

#[derive(Default)]
pub(super) struct NavigationWorkRegistry {
    next_id: u64,
    loads: HashMap<NavigationWorkId, ContextWork<PhysicalNavigationLoad>>,
    prepared_responses: HashMap<NavigationWorkId, ContextWork<PhysicalPreparedNavigationResponse>>,
    materializations: HashMap<NavigationWorkId, ContextWork<PhysicalDocumentMaterialization>>,
    prepared_documents: HashMap<NavigationWorkId, ContextWork<PhysicalPreparedDocumentNavigation>>,
    initial_builds: HashMap<NavigationWorkId, ContextWork<PhysicalInitialDocumentBuild>>,
    built_initial_documents: HashMap<NavigationWorkId, ContextWork<PhysicalBuiltInitialDocument>>,
}

impl NavigationWorkRegistry {
    fn allocate(&mut self) -> NavigationWorkId {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("Browser navigation work identity exhausted");
        NavigationWorkId(self.next_id)
    }

    fn insert_load(
        &mut self,
        context: BrowserContextId,
        load: PhysicalNavigationLoad,
    ) -> NavigationWorkId {
        let id = self.allocate();
        self.loads.insert(
            id,
            ContextWork {
                context,
                value: load,
            },
        );
        id
    }

    fn remove(&mut self, id: NavigationWorkId) {
        self.loads.remove(&id);
        self.prepared_responses.remove(&id);
        self.materializations.remove(&id);
        self.prepared_documents.remove(&id);
        self.initial_builds.remove(&id);
        if let Some(built) = self.built_initial_documents.remove(&id) {
            tokio::task::spawn_local(built.value.retire());
        }
    }

    pub(super) fn remove_context(&mut self, context: BrowserContextId) {
        self.loads.retain(|_, work| work.context != context);
        self.prepared_responses
            .retain(|_, work| work.context != context);
        self.materializations
            .retain(|_, work| work.context != context);
        self.prepared_documents
            .retain(|_, work| work.context != context);
        self.initial_builds
            .retain(|_, work| work.context != context);
        let built = self
            .built_initial_documents
            .iter()
            .filter_map(|(id, work)| (work.context == context).then_some(*id))
            .collect::<Vec<_>>();
        for id in built {
            if let Some(work) = self.built_initial_documents.remove(&id) {
                tokio::task::spawn_local(work.value.retire());
            }
        }
    }

    pub(super) fn clear(&mut self) {
        self.loads.clear();
        self.prepared_responses.clear();
        self.materializations.clear();
        self.prepared_documents.clear();
        self.initial_builds.clear();
        for (_, built) in self.built_initial_documents.drain() {
            tokio::task::spawn_local(built.value.retire());
        }
    }
}

fn unavailable() -> String {
    "Browser navigation operation is unavailable".to_owned()
}

fn receive_error() -> anyhow::Error {
    anyhow::anyhow!("Browser owner stopped before completing navigation work")
}

impl BrowserHandle {
    fn discard_navigation_work(&self, id: NavigationWorkId) {
        let _ = self
            .endpoint
            .tx
            .send(BrowserOwnerMessage::Execute(Box::new(move |browser| {
                browser.navigation_work.remove(id)
            })));
    }
}

/// Move-only capability for one Browser-owned navigation load.
pub struct BrowserNavigationLoad {
    context: BrowserContextHandle,
    work: Option<NavigationWorkId>,
    web_contents: WebContentsId,
    navigation: NavigationId,
    document: DocumentId,
    renderer: RendererPageResidenceIdentity,
}

impl BrowserNavigationLoad {
    fn new(
        context: BrowserContextHandle,
        work: NavigationWorkId,
        load: &PhysicalNavigationLoad,
    ) -> Self {
        Self {
            context,
            work: Some(work),
            web_contents: load.web_contents_id(),
            navigation: load.navigation_id(),
            document: load.document_id(),
            renderer: load.renderer_page(),
        }
    }

    fn work(&self) -> Result<NavigationWorkId, String> {
        self.work.ok_or_else(unavailable)
    }

    fn take_work(&mut self) -> Result<NavigationWorkId, String> {
        self.work.take().ok_or_else(unavailable)
    }

    pub fn web_contents_id(&self) -> WebContentsId {
        self.web_contents
    }

    pub fn navigation_id(&self) -> NavigationId {
        self.navigation
    }

    pub fn document_id(&self) -> DocumentId {
        self.document
    }

    pub fn renderer_page(&self) -> RendererPageResidenceIdentity {
        self.renderer
    }

    pub fn validate_request(&self, raw_url: &str) -> anyhow::Result<()> {
        let id = self.work().map_err(anyhow::Error::msg)?;
        let raw_url = raw_url.to_owned();
        self.context
            .browser
            .execute(move |browser| {
                browser
                    .navigation_work
                    .loads
                    .get(&id)
                    .ok_or_else(unavailable)?
                    .value
                    .validate_request(&raw_url)
                    .map_err(|error| error.to_string())
            })
            .map_err(anyhow::Error::msg)?
            .map_err(anyhow::Error::msg)
    }

    pub async fn fetch_navigation(
        &mut self,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
    ) -> anyhow::Result<NavigationStreamingRawResponse> {
        let id = self.work().map_err(anyhow::Error::msg)?;
        let method = method.to_owned();
        let raw_url = raw_url.to_owned();
        let completion = self
            .context
            .browser
            .execute(move |browser| {
                let work = browser
                    .navigation_work
                    .loads
                    .remove(&id)
                    .ok_or_else(unavailable)?;
                let local_sender = browser.local_sender.clone();
                let (completion_tx, completion) = oneshot::channel();
                tokio::task::spawn_local(async move {
                    let ContextWork { context, mut value } = work;
                    let result = value
                        .fetch_navigation(&method, &raw_url, body, request_headers)
                        .await;
                    let _ = local_sender.send(Box::new(move |browser| {
                        browser
                            .navigation_work
                            .loads
                            .insert(id, ContextWork { context, value });
                        let _ = completion_tx.send(result);
                    }));
                });
                Ok::<_, String>(completion)
            })
            .map_err(anyhow::Error::msg)?
            .map_err(anyhow::Error::msg)?;
        completion.await.map_err(|_| receive_error())?
    }

    async fn fetch_intercepted_response(
        &mut self,
        method: String,
        raw_url: String,
        body: Option<Vec<u8>>,
        headers: Vec<(String, String)>,
        auth: Option<SubresourceAuthCredentials>,
    ) -> anyhow::Result<NetworkFetchResult<StreamingRawResponse>> {
        let id = self.work().map_err(anyhow::Error::msg)?;
        let completion = self
            .context
            .browser
            .execute(move |browser| {
                let work = browser
                    .navigation_work
                    .loads
                    .remove(&id)
                    .ok_or_else(unavailable)?;
                let local_sender = browser.local_sender.clone();
                let (completion_tx, completion) = oneshot::channel();
                tokio::task::spawn_local(async move {
                    let ContextWork { context, value } = work;
                    let result = value
                        .fetch_intercepted_response(&method, &raw_url, body, headers, auth)
                        .await;
                    let _ = local_sender.send(Box::new(move |browser| {
                        browser
                            .navigation_work
                            .loads
                            .insert(id, ContextWork { context, value });
                        let _ = completion_tx.send(result);
                    }));
                });
                Ok::<_, String>(completion)
            })
            .map_err(anyhow::Error::msg)?
            .map_err(anyhow::Error::msg)?;
        completion.await.map_err(|_| receive_error())?
    }

    async fn fetch_intercepted_auth_response(
        &mut self,
        method: String,
        raw_url: String,
        body: Option<Vec<u8>>,
        headers: Vec<(String, String)>,
        auth: SubresourceAuthCredentials,
    ) -> anyhow::Result<NetworkFetchResult<RawResponse>> {
        let id = self.work().map_err(anyhow::Error::msg)?;
        let completion = self
            .context
            .browser
            .execute(move |browser| {
                let work = browser
                    .navigation_work
                    .loads
                    .remove(&id)
                    .ok_or_else(unavailable)?;
                let local_sender = browser.local_sender.clone();
                let (completion_tx, completion) = oneshot::channel();
                tokio::task::spawn_local(async move {
                    let ContextWork { context, value } = work;
                    let result = value
                        .fetch_intercepted_auth_response(&method, &raw_url, body, headers, auth)
                        .await;
                    let _ = local_sender.send(Box::new(move |browser| {
                        browser
                            .navigation_work
                            .loads
                            .insert(id, ContextWork { context, value });
                        let _ = completion_tx.send(result);
                    }));
                });
                Ok::<_, String>(completion)
            })
            .map_err(anyhow::Error::msg)?
            .map_err(anyhow::Error::msg)?;
        completion.await.map_err(|_| receive_error())?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_document_response_async(
        &mut self,
        requested_url: Url,
        final_url: Url,
        redirected: bool,
        redirect_count: usize,
        response_status: u16,
        response_headers: Vec<(String, String)>,
        raw_body: ExternalRawDocumentBodyStream,
        stage: PageVmInitStage,
        reply_boundary: RendererReplyBoundary,
        resource_source: CommittedDocumentResourceSource,
        reserved_service_worker_client: Option<RendererReservedServiceWorkerClient>,
    ) -> anyhow::Result<BrowserPreparedNavigationResponse> {
        let id = self.take_work().map_err(anyhow::Error::msg)?;
        let context_handle = self.context.clone();
        let completion = self
            .context
            .browser
            .execute(move |browser| {
                let work = browser
                    .navigation_work
                    .loads
                    .remove(&id)
                    .ok_or_else(unavailable)?;
                let local_sender = browser.local_sender.clone();
                let (completion_tx, completion) = oneshot::channel();
                tokio::task::spawn_local(async move {
                    let ContextWork { context, mut value } = work;
                    let result = value
                        .prepare_document_response_async(
                            requested_url,
                            final_url,
                            redirected,
                            redirect_count,
                            response_status,
                            response_headers,
                            raw_body,
                            stage,
                            reply_boundary,
                            resource_source,
                            reserved_service_worker_client,
                        )
                        .await;
                    let _ = local_sender.send(Box::new(move |browser| {
                        let result = match result {
                            Ok(prepared) => {
                                let renderer_token = prepared.renderer_devtools_agent_token();
                                let inspection = prepared.inspection_configuration_endpoint();
                                browser.navigation_work.prepared_responses.insert(
                                    id,
                                    ContextWork {
                                        context,
                                        value: prepared,
                                    },
                                );
                                Ok(BrowserPreparedNavigationResponse {
                                    context: context_handle,
                                    work: Some(id),
                                    renderer_token,
                                    inspection,
                                })
                            }
                            Err(error) => Err(error.to_string()),
                        };
                        let _ = completion_tx.send(result);
                    }));
                });
                Ok::<_, String>(completion)
            })
            .map_err(anyhow::Error::msg)?
            .map_err(anyhow::Error::msg)?;
        completion
            .await
            .map_err(|_| receive_error())?
            .map_err(anyhow::Error::msg)
    }
}

impl Drop for BrowserNavigationLoad {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            self.context.browser.discard_navigation_work(work);
        }
    }
}

/// Browser-owned intercepted request whose renderer reservation stays on the
/// Browser owner sequence while network transport is awaited.
pub struct BrowserInterceptedNavigationLoad {
    pub load: BrowserNavigationLoad,
    pub requested_url: Url,
    pub method: String,
    body: Option<Vec<u8>>,
    pub headers: Vec<(String, String)>,
    prior_observations: NetworkObservationJournal,
}

impl std::fmt::Debug for BrowserInterceptedNavigationLoad {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserInterceptedNavigationLoad")
            .field("renderer", &self.load.renderer_page())
            .field("requested_url", &self.requested_url)
            .finish_non_exhaustive()
    }
}

impl BrowserInterceptedNavigationLoad {
    pub fn new(
        load: BrowserNavigationLoad,
        requested_url: Url,
        method: String,
        body: Option<Vec<u8>>,
        headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            load,
            requested_url,
            method,
            body,
            headers,
            prior_observations: NetworkObservationJournal::default(),
        }
    }

    pub async fn fetch_streaming(
        mut self,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<BrowserInterceptedNavigationResponse<StreamingRawResponse>, String> {
        let response = self
            .load
            .fetch_intercepted_response(
                self.method.clone(),
                self.requested_url.to_string(),
                self.body.clone(),
                self.headers.clone(),
                auth,
            )
            .await
            .map_err(|error| format!("failed to fetch page `{}`: {error}", self.requested_url))?;
        Ok(self.with_response(response))
    }

    pub async fn fetch_auth(
        mut self,
        auth: SubresourceAuthCredentials,
    ) -> Result<BrowserInterceptedNavigationResponse<RawResponse>, String> {
        let response = self
            .load
            .fetch_intercepted_auth_response(
                self.method.clone(),
                self.requested_url.to_string(),
                self.body.clone(),
                self.headers.clone(),
                auth,
            )
            .await
            .map_err(|error| format!("failed to fetch page `{}`: {error}", self.requested_url))?;
        Ok(self.with_response(response))
    }

    pub fn with_response<R>(
        mut self,
        response: NetworkFetchResult<R>,
    ) -> BrowserInterceptedNavigationResponse<R> {
        let (response, observations) = response.into_parts_with_observation_journal();
        let mut prior = std::mem::take(&mut self.prior_observations);
        prior.append(observations);
        BrowserInterceptedNavigationResponse {
            work: self,
            response: NetworkFetchResult::with_observation_journal(response, prior),
        }
    }

    pub fn into_request_parts(
        self,
    ) -> (
        BrowserNavigationLoad,
        Url,
        String,
        Option<Vec<u8>>,
        Vec<(String, String)>,
    ) {
        (
            self.load,
            self.requested_url,
            self.method,
            self.body,
            self.headers,
        )
    }
}

#[derive(Debug)]
pub struct BrowserInterceptedNavigationResponse<R> {
    work: BrowserInterceptedNavigationLoad,
    response: NetworkFetchResult<R>,
}

impl<R> BrowserInterceptedNavigationResponse<R> {
    pub fn web_contents(&self) -> WebContentsId {
        self.work.load.web_contents_id()
    }

    pub fn response(&self) -> &R {
        self.response.response()
    }

    pub fn observation_journal(&self) -> &NetworkObservationJournal {
        self.response.observation_journal()
    }

    pub fn into_parts(self) -> (BrowserInterceptedNavigationLoad, NetworkFetchResult<R>) {
        (self.work, self.response)
    }
}

impl BrowserInterceptedNavigationResponse<StreamingRawResponse> {
    pub async fn materialize(
        self,
    ) -> Result<BrowserInterceptedNavigationResponse<RawResponse>, String> {
        let (response, observations) = self.response.into_parts_with_observation_journal();
        let response = response
            .into_materialized_raw_response()
            .await
            .map_err(|error| format!("failed to read page body from stream: {error}"))?;
        Ok(BrowserInterceptedNavigationResponse {
            work: self.work,
            response: NetworkFetchResult::with_observation_journal(response, observations),
        })
    }
}

impl BrowserInterceptedNavigationResponse<RawResponse> {
    pub fn retry(self) -> BrowserInterceptedNavigationLoad {
        let (mut work, response) = self.into_parts();
        let (_, observations) = response.into_parts_with_observation_journal();
        work.prior_observations = observations;
        work
    }
}

pub enum BrowserInitialDocumentAdmission {
    Present,
    Join(InitialDocumentPageBuildWaiter),
    Build(Box<BrowserInitialDocumentBuild>),
}

pub struct BrowserInitialDocumentBuild {
    context: BrowserContextHandle,
    work: Option<NavigationWorkId>,
    key: InitialDocumentBuildKey,
    inspection: moli_renderer_v8::RendererPreparedDocumentInspectionEndpoint,
}

impl BrowserInitialDocumentBuild {
    pub fn key(&self) -> InitialDocumentBuildKey {
        self.key
    }

    pub fn inspection_endpoint(
        &self,
    ) -> moli_renderer_v8::RendererPreparedDocumentInspectionEndpoint {
        self.inspection.clone()
    }

    pub fn start_preparation(&mut self) -> anyhow::Result<()> {
        let work = self.work.ok_or_else(|| anyhow::anyhow!(unavailable()))?;
        self.context
            .browser
            .execute(move |browser| {
                browser
                    .navigation_work
                    .initial_builds
                    .get_mut(&work)
                    .ok_or_else(unavailable)?
                    .value
                    .start_preparation()
                    .map_err(|error| error.to_string())
            })
            .map_err(anyhow::Error::msg)?
            .map_err(anyhow::Error::msg)
    }

    pub async fn materialize(mut self) -> anyhow::Result<BrowserBuiltInitialDocument> {
        let id = self
            .work
            .take()
            .ok_or_else(|| anyhow::anyhow!(unavailable()))?;
        let context_handle = self.context.clone();
        let completion = self
            .context
            .browser
            .execute(move |browser| {
                let work = browser
                    .navigation_work
                    .initial_builds
                    .remove(&id)
                    .ok_or_else(unavailable)?;
                let local_sender = browser.local_sender.clone();
                let (completion_tx, completion) = oneshot::channel();
                tokio::task::spawn_local(async move {
                    let ContextWork { context, value } = work;
                    let result = value.materialize().await;
                    let _ = local_sender.send(Box::new(move |browser| {
                        let result = match result {
                            Ok(built) => {
                                let key = built.key();
                                browser.navigation_work.built_initial_documents.insert(
                                    id,
                                    ContextWork {
                                        context,
                                        value: built,
                                    },
                                );
                                Ok(BrowserBuiltInitialDocument {
                                    context: context_handle,
                                    work: Some(id),
                                    key,
                                })
                            }
                            Err(error) => Err(error.to_string()),
                        };
                        let _ = completion_tx.send(result);
                    }));
                });
                Ok::<_, String>(completion)
            })
            .map_err(anyhow::Error::msg)?
            .map_err(anyhow::Error::msg)?;
        completion
            .await
            .map_err(|_| receive_error())?
            .map_err(anyhow::Error::msg)
    }
}

impl Drop for BrowserInitialDocumentBuild {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            self.context.browser.discard_navigation_work(work);
        }
    }
}

pub struct BrowserBuiltInitialDocument {
    context: BrowserContextHandle,
    work: Option<NavigationWorkId>,
    key: InitialDocumentBuildKey,
}

impl BrowserBuiltInitialDocument {
    pub fn key(&self) -> InitialDocumentBuildKey {
        self.key
    }

    pub async fn retire(mut self) {
        let Some(work) = self.work.take() else {
            return;
        };
        let completion = self.context.browser.execute(move |browser| {
            let built = browser
                .navigation_work
                .built_initial_documents
                .remove(&work)
                .map(|work| work.value);
            let (completion_tx, completion) = oneshot::channel();
            tokio::task::spawn_local(async move {
                if let Some(built) = built {
                    built.retire().await;
                }
                let _ = completion_tx.send(());
            });
            completion
        });
        if let Ok(completion) = completion {
            let _ = completion.await;
        }
    }
}

impl Drop for BrowserBuiltInitialDocument {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            self.context.browser.discard_navigation_work(work);
        }
    }
}

pub struct BrowserCommittedInitialDocument {
    pub key: InitialDocumentBuildKey,
    pub lifecycle: crate::browser::web_contents::CommittedDocumentLifecycle,
    pub diagnostics: crate::page::RendererPageCreationDiagnostics,
    pub inspection_endpoint: moli_renderer_v8::RendererInspectionEndpoint,
}

/// Exact prepared renderer reservation retained by the Browser owner.
pub struct BrowserPreparedNavigationResponse {
    context: BrowserContextHandle,
    work: Option<NavigationWorkId>,
    renderer_token: crate::page::RendererDevToolsAgentToken,
    inspection: moli_renderer_v8::RendererPreparedDocumentInspectionEndpoint,
}

impl BrowserPreparedNavigationResponse {
    pub fn renderer_devtools_agent_token(&self) -> crate::page::RendererDevToolsAgentToken {
        self.renderer_token
    }

    pub fn inspection_configuration_endpoint(
        &self,
    ) -> moli_renderer_v8::RendererPreparedDocumentInspectionEndpoint {
        self.inspection.clone()
    }

    fn take_work(&mut self) -> Result<NavigationWorkId, String> {
        self.work.take().ok_or_else(unavailable)
    }
}

impl Drop for BrowserPreparedNavigationResponse {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            self.context.browser.discard_navigation_work(work);
        }
    }
}

/// Materialization admitted against an exact prepared navigation response.
pub struct BrowserDocumentMaterialization {
    context: BrowserContextHandle,
    work: Option<NavigationWorkId>,
}

impl BrowserDocumentMaterialization {
    pub async fn materialize(
        mut self,
    ) -> anyhow::Result<BuiltDocumentPage<BrowserPreparedDocumentNavigation>> {
        let id = self
            .work
            .take()
            .ok_or_else(|| anyhow::anyhow!(unavailable()))?;
        let context_handle = self.context.clone();
        let completion = self
            .context
            .browser
            .execute(move |browser| {
                let work = browser
                    .navigation_work
                    .materializations
                    .remove(&id)
                    .ok_or_else(unavailable)?;
                let local_sender = browser.local_sender.clone();
                let (completion_tx, completion) = oneshot::channel();
                tokio::task::spawn_local(async move {
                    let ContextWork { context, value } = work;
                    let result = value.materialize().await;
                    let _ = local_sender.send(Box::new(move |browser| {
                        let result = match result {
                            Ok(BuiltDocumentPage {
                                page,
                                page_creation_diagnostics,
                                page_creation_artifacts,
                                pending_download,
                            }) => {
                                let navigation = page.navigation();
                                let web_contents = page.web_contents_id();
                                let renderer = page.renderer_residence();
                                browser.navigation_work.prepared_documents.insert(
                                    id,
                                    ContextWork {
                                        context,
                                        value: page,
                                    },
                                );
                                Ok(BuiltDocumentPage {
                                    page: BrowserPreparedDocumentNavigation {
                                        context: context_handle,
                                        work: Some(id),
                                        navigation,
                                        web_contents,
                                        renderer,
                                    },
                                    page_creation_diagnostics,
                                    page_creation_artifacts,
                                    pending_download,
                                })
                            }
                            Err(error) => Err(error.to_string()),
                        };
                        let _ = completion_tx.send(result);
                    }));
                });
                Ok::<_, String>(completion)
            })
            .map_err(anyhow::Error::msg)?
            .map_err(anyhow::Error::msg)?;
        completion
            .await
            .map_err(|_| receive_error())?
            .map_err(anyhow::Error::msg)
    }
}

impl Drop for BrowserDocumentMaterialization {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            self.context.browser.discard_navigation_work(work);
        }
    }
}

/// Move-only capability for a materialized navigation awaiting Browser commit.
pub struct BrowserPreparedDocumentNavigation {
    context: BrowserContextHandle,
    work: Option<NavigationWorkId>,
    navigation: NavigationId,
    web_contents: WebContentsId,
    renderer: RendererPageResidenceIdentity,
}

impl BrowserPreparedDocumentNavigation {
    pub fn navigation(&self) -> NavigationId {
        self.navigation
    }

    pub fn web_contents_id(&self) -> WebContentsId {
        self.web_contents
    }

    pub fn renderer_residence(&self) -> RendererPageResidenceIdentity {
        self.renderer
    }

    fn take_work(&mut self) -> Result<NavigationWorkId, String> {
        self.work.take().ok_or_else(unavailable)
    }
}

impl Drop for BrowserPreparedDocumentNavigation {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            self.context.browser.discard_navigation_work(work);
        }
    }
}

/// Sendable outcome of a completed Browser document navigation.
pub struct BrowserDocumentNavigationCommit {
    pub web_contents: WebContentsId,
    pub frame_slot: crate::browser::MainFrameSlotId,
    pub navigation: NavigationId,
    pub document: DocumentId,
    pub previous_document: Option<DocumentId>,
    pub previous_renderer: Option<RendererPageResidenceIdentity>,
    pub inspection_endpoint: moli_renderer_v8::RendererInspectionEndpoint,
    pub lifecycle: crate::browser::web_contents::CommittedDocumentLifecycle,
    pub info: CommittedDocumentInfo,
    pub retirement: PendingDocumentRetirement,
    pub post_response_continuation:
        Option<crate::page::RendererPageCommandPostResponseContinuation>,
}

impl BrowserContextHandle {
    pub fn start_initial_document(
        &self,
        handle: WebContentsHandle,
        inherited: InheritedDocumentPolicy,
    ) -> Result<BrowserInitialDocumentAdmission, String> {
        let context_handle = self.clone();
        self.browser.execute(move |browser| {
            match browser
                .context_mut(context_handle.id)?
                .start_initial_document(handle, inherited)?
            {
                PhysicalInitialDocumentAdmission::Present => {
                    Ok(BrowserInitialDocumentAdmission::Present)
                }
                PhysicalInitialDocumentAdmission::Join(waiter) => {
                    Ok(BrowserInitialDocumentAdmission::Join(waiter))
                }
                PhysicalInitialDocumentAdmission::Build(build) => {
                    let key = build.key();
                    let inspection = build.inspection_endpoint();
                    let work = browser.navigation_work.allocate();
                    browser.navigation_work.initial_builds.insert(
                        work,
                        ContextWork {
                            context: context_handle.id,
                            value: *build,
                        },
                    );
                    Ok(BrowserInitialDocumentAdmission::Build(Box::new(
                        BrowserInitialDocumentBuild {
                            context: context_handle,
                            work: Some(work),
                            key,
                            inspection,
                        },
                    )))
                }
            }
        })?
    }

    pub fn commit_initial_document(
        &self,
        mut built: BrowserBuiltInitialDocument,
    ) -> Result<BrowserCommittedInitialDocument, Box<BrowserBuiltInitialDocument>> {
        if built.context.id != self.id {
            return Err(Box::new(built));
        }
        let Some(work) = built.work.take() else {
            return Err(Box::new(built));
        };
        let key = built.key;
        let context_handle = self.clone();
        let fallback_context = self.clone();
        match self.browser.execute(move |browser| {
            let Some(stored) = browser
                .navigation_work
                .built_initial_documents
                .remove(&work)
            else {
                return Err(Box::new(BrowserBuiltInitialDocument {
                    context: context_handle,
                    work: None,
                    key,
                }));
            };
            if stored.context != context_handle.id {
                browser
                    .navigation_work
                    .built_initial_documents
                    .insert(work, stored);
                return Err(Box::new(BrowserBuiltInitialDocument {
                    context: context_handle,
                    work: Some(work),
                    key,
                }));
            }
            let committed = match browser.context_mut(context_handle.id) {
                Ok(context) => context.commit_initial_document(stored.value),
                Err(_) => Err(Box::new(stored.value)),
            };
            match committed {
                Ok(PhysicalCommittedInitialDocument {
                    key,
                    lifecycle,
                    diagnostics,
                    inspection_endpoint,
                }) => Ok(BrowserCommittedInitialDocument {
                    key,
                    lifecycle,
                    diagnostics,
                    inspection_endpoint,
                }),
                Err(stale) => {
                    browser.navigation_work.built_initial_documents.insert(
                        work,
                        ContextWork {
                            context: context_handle.id,
                            value: *stale,
                        },
                    );
                    Err(Box::new(BrowserBuiltInitialDocument {
                        context: context_handle,
                        work: Some(work),
                        key,
                    }))
                }
            }
        }) {
            Ok(result) => result,
            Err(_) => Err(Box::new(BrowserBuiltInitialDocument {
                context: fallback_context,
                work: Some(work),
                key,
            })),
        }
    }

    pub fn start_navigation_load(
        &self,
        handle: WebContentsHandle,
        navigation: NavigationId,
        policy: NavigationRequestLoadPolicy,
        inherited: InheritedDocumentPolicy,
    ) -> Result<BrowserNavigationLoad, String> {
        let context_handle = self.clone();
        self.browser.execute(move |browser| {
            let load = browser
                .context_mut(context_handle.id)?
                .start_navigation_load(handle, navigation, policy, inherited)?;
            let work = browser.navigation_work.insert_load(context_handle.id, load);
            let load = &browser
                .navigation_work
                .loads
                .get(&work)
                .expect("inserted navigation load")
                .value;
            Ok(BrowserNavigationLoad::new(context_handle, work, load))
        })?
    }

    pub fn start_claimed_navigation_request(
        &self,
        request: ClaimedNavigationRequest,
        inherited: InheritedDocumentPolicy,
    ) -> Result<BrowserInterceptedNavigationLoad, String> {
        let context_handle = self.clone();
        self.browser.execute(move |browser| {
            let work = browser
                .context_mut(context_handle.id)?
                .start_claimed_navigation_request(request, inherited)?;
            let (load, requested_url, method, body, headers) = work.into_owner_parts();
            let id = browser.navigation_work.insert_load(context_handle.id, load);
            let stored = &browser
                .navigation_work
                .loads
                .get(&id)
                .expect("inserted intercepted navigation load")
                .value;
            Ok(BrowserInterceptedNavigationLoad::new(
                BrowserNavigationLoad::new(context_handle, id, stored),
                requested_url,
                method,
                body,
                headers,
            ))
        })?
    }

    pub fn start_navigation_load_for_interception(
        &self,
        permit: NavigationInterceptionPermit,
        policy: NavigationRequestLoadPolicy,
        inherited: InheritedDocumentPolicy,
    ) -> Result<BrowserNavigationLoad, String> {
        let context_handle = self.clone();
        self.browser.execute(move |browser| {
            let load = browser
                .context_mut(context_handle.id)?
                .start_navigation_load_for_interception(permit, policy, inherited)?;
            let id = browser.navigation_work.insert_load(context_handle.id, load);
            let stored = &browser
                .navigation_work
                .loads
                .get(&id)
                .expect("inserted intercepted navigation load")
                .value;
            Ok(BrowserNavigationLoad::new(context_handle, id, stored))
        })?
    }

    pub fn pause_navigation_auth(
        &self,
        response: BrowserInterceptedNavigationResponse<RawResponse>,
    ) -> Result<NavigationInterceptionPermit, String> {
        let (work, response) = response.into_parts();
        let BrowserInterceptedNavigationLoad {
            mut load,
            requested_url,
            method,
            body,
            headers,
            prior_observations: _,
        } = work;
        if load.context.id != self.id {
            return Err("navigation auth belongs to another BrowserContext".to_owned());
        }
        let id = load.take_work()?;
        let context = self.id;
        self.browser.execute(move |browser| {
            let load = browser
                .navigation_work
                .loads
                .remove(&id)
                .ok_or_else(unavailable)?;
            if load.context != context {
                return Err("navigation auth belongs to another BrowserContext".to_owned());
            }
            let work = PhysicalInterceptedNavigationLoad::new(
                load.value,
                requested_url,
                method,
                body,
                headers,
            );
            browser
                .context_mut(context)?
                .pause_navigation_auth(work.with_response(response))
        })?
    }

    pub fn take_navigation_auth(
        &self,
        permit: NavigationInterceptionPermit,
    ) -> Option<BrowserInterceptedNavigationResponse<RawResponse>> {
        let context_handle = self.clone();
        self.browser
            .execute(move |browser| {
                let response = browser
                    .context_mut(context_handle.id)
                    .ok()?
                    .take_navigation_auth(permit)?;
                let (work, response) = response.into_parts();
                let (load, requested_url, method, body, headers) = work.into_owner_parts();
                let id = browser.navigation_work.insert_load(context_handle.id, load);
                let stored = &browser
                    .navigation_work
                    .loads
                    .get(&id)
                    .expect("restored intercepted navigation load")
                    .value;
                Some(BrowserInterceptedNavigationResponse {
                    work: BrowserInterceptedNavigationLoad::new(
                        BrowserNavigationLoad::new(context_handle, id, stored),
                        requested_url,
                        method,
                        body,
                        headers,
                    ),
                    response,
                })
            })
            .ok()
            .flatten()
    }

    pub fn start_document_materialization(
        &self,
        handle: WebContentsHandle,
        navigation: NavigationId,
        mut page: BrowserPreparedNavigationResponse,
        destination: DocumentNavigationDestination,
        inherited: InheritedDocumentPolicy,
    ) -> Result<BrowserDocumentMaterialization, String> {
        if page.context.id != self.id {
            return Err("navigation response belongs to another BrowserContext".to_owned());
        }
        let work = page.take_work()?;
        let context_handle = self.clone();
        self.browser.execute(move |browser| {
            let prepared = browser
                .navigation_work
                .prepared_responses
                .remove(&work)
                .ok_or_else(unavailable)?;
            if prepared.context != context_handle.id {
                return Err("navigation response belongs to another BrowserContext".to_owned());
            }
            let materialization = browser
                .context_mut(context_handle.id)?
                .start_document_materialization(
                    handle,
                    navigation,
                    prepared.value,
                    destination,
                    inherited,
                )?;
            browser.navigation_work.materializations.insert(
                work,
                ContextWork {
                    context: context_handle.id,
                    value: materialization,
                },
            );
            Ok(BrowserDocumentMaterialization {
                context: context_handle,
                work: Some(work),
            })
        })?
    }

    pub fn commit_document_navigation(
        &self,
        mut prepared: BrowserPreparedDocumentNavigation,
    ) -> Result<BrowserDocumentNavigationCommit, String> {
        if prepared.context.id != self.id {
            return Err("navigation document belongs to another BrowserContext".to_owned());
        }
        let work = prepared.take_work()?;
        let context = self.id;
        self.browser.execute(move |browser| {
            let prepared = browser
                .navigation_work
                .prepared_documents
                .remove(&work)
                .ok_or_else(unavailable)?;
            if prepared.context != context {
                return Err("navigation document belongs to another BrowserContext".to_owned());
            }
            let commit = browser
                .context_mut(context)?
                .commit_document_navigation(prepared.value)?;
            let (completion_tx, completion) = oneshot::channel();
            tokio::task::spawn_local(async move {
                commit.retirement.close().await;
                let _ = completion_tx.send(());
            });
            Ok(BrowserDocumentNavigationCommit {
                web_contents: commit.web_contents,
                frame_slot: commit.frame_slot,
                navigation: commit.navigation,
                document: commit.document,
                previous_document: commit.previous_document,
                previous_renderer: commit.previous_renderer,
                inspection_endpoint: commit.inspection_endpoint,
                lifecycle: commit.lifecycle,
                info: commit.info,
                retirement: PendingDocumentRetirement { completion },
                post_response_continuation: commit.post_response_continuation,
            })
        })?
    }
}
