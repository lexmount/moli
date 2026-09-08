use crate::{
    browser::{
        DocumentId, NavigationId, NavigationRequestLoadPolicy, RendererPageResidenceIdentity,
        WebContentsId,
    },
    page::SubresourceAuthCredentials,
    runtime::{
        CommittedDocumentResourceSource, ExternalRawDocumentBodyStream, NavigationEngine,
        NavigationPageStorageHandles, NavigationResourceStorageHandles,
        NavigationStreamingRawResponse, PageVmInitStage, PreparedDocumentPage,
        RendererPageReservationToken, RendererReplyBoundary, RendererReservedServiceWorkerClient,
    },
};
use moli_fetch::{
    BrowserNavigationRequestKind, FetchCancelHandle, NetworkFetchResult, RawResponse, Request,
};
use url::Url;

use super::{InheritedDocumentPolicy, WebContents, navigation_commit::DocumentNavigationIdentity};

#[cfg(test)]
mod tests;

/// Move-owned navigation work. No caller can borrow or reconfigure its engine,
/// replace its storage, or bind its response to a different Browser navigation.
pub struct AdmittedNavigationLoad {
    // Response preparation consumes admission even if renderer preparation fails.
    identity: Option<DocumentNavigationIdentity>,
    engine: NavigationEngine,
    reservation: RendererPageReservationToken,
    request_cancellation: FetchCancelHandle,
    resource_storage: NavigationResourceStorageHandles,
    page_storage: NavigationPageStorageHandles,
    initiator_url: Option<Url>,
    policy: NavigationRequestLoadPolicy,
    network_offline: bool,
    blocked_url_patterns: Vec<String>,
    pub(super) redirect_headers: Option<moli_fetch::RequestHeaders>,
    pub(super) redirect_chain: Vec<moli_fetch::RedirectInfo>,
}

/// The identity is inherited from load admission, never reconstructed from the
/// current Target, session, or pending navigation when the response completes.
pub struct PreparedNavigationResponse {
    pub(super) identity: DocumentNavigationIdentity,
    pub(super) page: PreparedDocumentPage,
}

impl PreparedNavigationResponse {
    pub fn renderer_devtools_agent_token(&self) -> crate::page::RendererDevToolsAgentToken {
        self.page.renderer_devtools_agent_token()
    }

    pub fn inspection_configuration_endpoint(
        &self,
    ) -> moli_renderer_v8::RendererPreparedDocumentInspectionEndpoint {
        self.page.inspection_configuration_endpoint()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn materialize(
        self,
        policy: Option<crate::runtime::PreparedDocumentPagePolicy>,
    ) -> anyhow::Result<crate::runtime::BuiltDocumentPage> {
        self.page.materialize(policy).await
    }
}

impl WebContents {
    pub fn start_navigation_load(
        &mut self,
        navigation: NavigationId,
        policy: NavigationRequestLoadPolicy,
        inherited: InheritedDocumentPolicy,
    ) -> Result<AdmittedNavigationLoad, String> {
        // Validate before configuring shared resources or allocating a renderer.
        let identity = self.document_navigation_identity(navigation)?;
        let (_, _, network_offline) = self.configure_navigation_resources(&inherited)?;
        let engine = self
            .navigation_engine
            .as_ref()
            .expect("configured navigation engine")
            .clone();
        let reservation = engine.reserve_page_for_creation();
        let renderer = RendererPageResidenceIdentity::from_parts(
            reservation.local_host_id(),
            reservation.page_id(),
        );
        let request_cancellation = FetchCancelHandle::new();
        if !self.navigation.admit_document_load(
            navigation,
            renderer,
            identity.preparation_cancellation.clone(),
            request_cancellation.clone(),
        ) {
            return Err("stale navigation document candidate".to_owned());
        }
        let initiator_url = self
            .main_frame
            .current_document
            .as_ref()
            .map(|document| document.page.final_url().clone())
            .filter(|url| url.host_str().is_some())
            .or_else(|| {
                self.navigation
                    .current_url()
                    .and_then(|url| Url::parse(url).ok())
                    .filter(|url| url.host_str().is_some())
            });
        let session_storage = self.session_storage.store();
        Ok(AdmittedNavigationLoad {
            identity: Some(identity),
            engine,
            reservation,
            request_cancellation,
            resource_storage: inherited
                .storage
                .resource_storage_handles(session_storage.clone())
                .into_navigation_storage(),
            page_storage: inherited
                .storage
                .page_storage_handles(session_storage.clone())
                .into_navigation_storage(),
            initiator_url,
            policy,
            network_offline,
            redirect_headers: None,
            redirect_chain: Vec::new(),
            blocked_url_patterns: self.network_request_policy.blocked_url_patterns.clone(),
        })
    }
}

impl AdmittedNavigationLoad {
    pub(super) fn identity(&self) -> &DocumentNavigationIdentity {
        self.identity
            .as_ref()
            .expect("navigation response admission already consumed")
    }

    pub fn web_contents_id(&self) -> WebContentsId {
        self.identity().web_contents
    }
    pub fn navigation_id(&self) -> NavigationId {
        self.identity().navigation
    }
    pub fn document_id(&self) -> DocumentId {
        self.identity().document
    }
    pub fn renderer_page(&self) -> RendererPageResidenceIdentity {
        RendererPageResidenceIdentity::from_parts(
            self.reservation.local_host_id(),
            self.reservation.page_id(),
        )
    }

    pub(crate) fn redirected_request(
        &mut self,
        response: moli_fetch::ResponseHead,
        mut request: moli_fetch::Request,
    ) -> moli_fetch::Request {
        if response.redirect_chain.len() > self.redirect_chain.len() {
            request = request.with_redirect_headers(self.redirect_headers.take());
        }
        for redirect in response
            .redirect_chain
            .iter()
            .skip(self.redirect_chain.len())
        {
            request.apply_redirect_status(redirect.status);
        }
        request.url = response.final_url;
        self.redirect_chain = response.redirect_chain;
        request
    }

    pub fn set_redirect_state(
        &mut self,
        headers: Option<moli_fetch::RequestHeaders>,
        chain: Vec<moli_fetch::RedirectInfo>,
    ) {
        self.redirect_headers = headers;
        self.redirect_chain = chain;
    }

    fn validate_request(
        &self,
        method: &str,
        raw_url: &str,
        headers: &moli_fetch::RequestHeaders,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.identity().is_cancelled(),
            "canceled navigation document candidate"
        );
        if self
            .blocked_url_patterns
            .iter()
            .any(|pattern| moli_fetch::url_pattern_matches(pattern, raw_url))
        {
            return Err(crate::browser::NavigationRequestBlocked.into());
        }
        if self.network_offline {
            return Err(crate::browser::NavigationNetworkError {
                kind: crate::browser::NavigationNetworkErrorKind::InternetDisconnected,
                unreachable_url: Url::parse(raw_url)?,
                request_method: method.to_owned(),
                request_headers: headers.clone(),
            }
            .into());
        }
        Ok(())
    }

    fn navigation_kind(&self) -> BrowserNavigationRequestKind {
        match self.policy {
            NavigationRequestLoadPolicy::Reload => BrowserNavigationRequestKind::Reload,
            _ => BrowserNavigationRequestKind::Navigate,
        }
    }

    fn infer_referrer(&self) -> bool {
        self.policy != NavigationRequestLoadPolicy::BrowserInitiated
    }

    pub async fn fetch_navigation(
        &mut self,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: moli_fetch::RequestHeaders,
    ) -> anyhow::Result<NavigationStreamingRawResponse> {
        self.fetch_navigation_with_auth(method, raw_url, body, request_headers, None)
            .await
    }

    pub(in crate::browser) async fn fetch_navigation_with_auth(
        &mut self,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        request_headers: moli_fetch::RequestHeaders,
        auth: Option<SubresourceAuthCredentials>,
    ) -> anyhow::Result<NavigationStreamingRawResponse> {
        let request = self.intercepted_request(method, raw_url, body, request_headers, auth)?;
        self.engine
            .fetch_navigation_request_with_storage_async(
                self.resource_storage.clone(),
                request,
                self.request_cancellation.clone(),
            )
            .await
    }

    fn intercepted_request(
        &self,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        headers: moli_fetch::RequestHeaders,
        auth: Option<SubresourceAuthCredentials>,
    ) -> anyhow::Result<Request> {
        self.validate_request(method, raw_url, &headers)?;
        let mut request = Request::new_browser_bytes(
            method,
            raw_url,
            body,
            headers,
            self.initiator_url
                .as_ref()
                .map_or(moli_url::WebOrigin::Opaque, moli_url::WebOrigin::from_url),
        )?
        .with_top_level_navigation_cookie_context()
        .with_page_network_policy()
        .with_browser_navigation_kind(self.navigation_kind())
        .with_redirect_headers(self.redirect_headers.clone())
        .with_redirect_chain(self.redirect_chain.clone());
        if !self.infer_referrer() {
            request = request.without_inferred_referrer();
        }
        if let Some(initiator) = &self.initiator_url {
            request = request.with_initiator_url(initiator);
        }
        request.set_auth(auth.map(Into::into));
        Ok(request)
    }

    pub async fn fetch_intercepted_auth_response(
        &self,
        method: &str,
        raw_url: &str,
        body: Option<Vec<u8>>,
        headers: moli_fetch::RequestHeaders,
        auth: SubresourceAuthCredentials,
    ) -> anyhow::Result<NetworkFetchResult<RawResponse>> {
        let request = self.intercepted_request(method, raw_url, body, headers, Some(auth))?;
        // Digest's intermediate 401 responses still require buffered transport.
        let response = self
            .engine
            .resource_request_client()
            .expect("admitted resource runtime")
            .fetch_raw_with_cancel_and_network_metadata(request, self.request_cancellation.clone())
            .await?;
        anyhow::ensure!(
            !self.identity().is_cancelled(),
            "canceled navigation document candidate"
        );
        Ok(response)
    }

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
    ) -> anyhow::Result<PreparedNavigationResponse> {
        let identity = self
            .identity
            .take()
            .ok_or_else(|| anyhow::anyhow!("navigation response admission already consumed"))?;
        anyhow::ensure!(
            !identity.is_cancelled(),
            "canceled navigation document candidate"
        );
        let page = self
            .engine
            .prepare_document_response_async(
                self.reservation,
                self.page_storage.clone(),
                requested_url,
                final_url,
                self.initiator_url.clone(),
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
            .await?;
        anyhow::ensure!(
            !identity.is_cancelled(),
            "canceled navigation document candidate"
        );
        Ok(PreparedNavigationResponse { identity, page })
    }

    /// Direct Page-builder fixtures have no Browser transaction to commit.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_fixture(
        engine: NavigationEngine,
        resource_storage: NavigationResourceStorageHandles,
        page_storage: NavigationPageStorageHandles,
        initiator_url: Option<Url>,
        policy: NavigationRequestLoadPolicy,
        network_offline: bool,
        blocked_url_patterns: Vec<String>,
    ) -> Self {
        let reservation = engine.reserve_page_for_creation();
        Self {
            identity: Some(DocumentNavigationIdentity {
                web_contents: WebContentsId::allocate(),
                frame_slot: crate::browser::MainFrameSlotId::allocate(),
                navigation: NavigationId::allocate(),
                document: DocumentId::allocate(),
                cancellation: FetchCancelHandle::new(),
                preparation_cancellation: FetchCancelHandle::new(),
            }),
            engine,
            reservation,
            request_cancellation: FetchCancelHandle::new(),
            resource_storage,
            page_storage,
            initiator_url,
            policy,
            network_offline,
            blocked_url_patterns,
            redirect_headers: None,
            redirect_chain: Vec::new(),
        }
    }
}
