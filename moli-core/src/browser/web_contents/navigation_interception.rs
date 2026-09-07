use crate::{
    browser::{
        BrowserRequestId, DocumentId, NavigationId, NavigationRequestLoadPolicy, WebContentsId,
    },
    page::SubresourceAuthCredentials,
};
use moli_fetch::{
    NetworkFetchResult, NetworkObservationJournal, RawResponse, StreamingRawResponse,
};
use url::Url;

use super::{
    AdmittedNavigationLoad, InheritedDocumentPolicy, WebContents,
    navigation_commit::DocumentNavigationIdentity,
};

/// A single Browser decision. Copying a protocol correlation cannot duplicate
/// its authority: the owning pending navigation consumes this request once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationInterceptionPermit {
    pub(super) web_contents: WebContentsId,
    pub(super) navigation: NavigationId,
    pub(super) document: DocumentId,
    pub(super) request: BrowserRequestId,
}

impl NavigationInterceptionPermit {
    pub fn navigation(self) -> NavigationId {
        self.navigation
    }

    pub fn web_contents(self) -> WebContentsId {
        self.web_contents
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_correlation_test() -> Self {
        Self {
            web_contents: WebContentsId::allocate(),
            navigation: NavigationId::allocate(),
            document: DocumentId::allocate(),
            request: BrowserRequestId::allocate(),
        }
    }
}

/// Browser-owned main-document request data while Fetch is deciding whether
/// and how to resume it. It deliberately contains no Target, session, loader,
/// protocol request id, or frontend result state.
#[derive(Debug)]
pub struct NavigationRequestInterception {
    requested_url: Url,
    method: String,
    body: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
    policy: NavigationRequestLoadPolicy,
}

impl NavigationRequestInterception {
    pub fn new(
        requested_url: Url,
        method: String,
        body: Option<Vec<u8>>,
        headers: Vec<(String, String)>,
        policy: NavigationRequestLoadPolicy,
    ) -> Self {
        Self {
            requested_url,
            method,
            body,
            headers,
            policy,
        }
    }

    fn apply_overrides(
        &mut self,
        requested_url: Option<Url>,
        method: Option<String>,
        body: Option<String>,
        headers: Option<Vec<(String, String)>>,
    ) {
        if let Some(requested_url) = requested_url {
            self.requested_url = requested_url;
        }
        if let Some(method) = method {
            self.method = method;
        }
        if let Some(body) = body {
            self.body = Some(body.into_bytes());
        }
        if let Some(headers) = headers {
            self.headers = headers;
        }
    }
}

/// A successfully claimed request-stage Browser decision.
///
/// The claim can be admitted only against the exact pending WebContents and
/// Navigation identified by `permit`; supersession makes admission fail rather
/// than falling back to a current Target/loader selection.
#[derive(Debug)]
pub struct ClaimedNavigationRequest {
    permit: NavigationInterceptionPermit,
    request: NavigationRequestInterception,
}

impl ClaimedNavigationRequest {
    pub(super) fn new(
        permit: NavigationInterceptionPermit,
        request: NavigationRequestInterception,
    ) -> Self {
        Self { permit, request }
    }

    pub fn permit(&self) -> NavigationInterceptionPermit {
        self.permit
    }

    pub fn apply_overrides(
        &mut self,
        requested_url: Option<Url>,
        method: Option<String>,
        body: Option<String>,
        headers: Option<Vec<(String, String)>>,
    ) {
        self.request
            .apply_overrides(requested_url, method, body, headers);
    }
}

/// The admitted Browser operation retains the actual request across auth
/// pauses and retries. No Target, session, loader ID or frontend configuration
/// is needed to resume it, and no Browser borrow is held while fetching.
pub struct InterceptedNavigationLoad {
    pub load: AdmittedNavigationLoad,
    pub requested_url: Url,
    pub method: String,
    body: Option<Vec<u8>>,
    pub headers: Vec<(String, String)>,
    prior_observations: NetworkObservationJournal,
}

impl std::fmt::Debug for InterceptedNavigationLoad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterceptedNavigationLoad")
            .field("renderer", &self.load.renderer_page())
            .field("requested_url", &self.requested_url)
            .finish_non_exhaustive()
    }
}

impl InterceptedNavigationLoad {
    pub fn new(
        load: AdmittedNavigationLoad,
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
        self,
        auth: Option<SubresourceAuthCredentials>,
    ) -> Result<InterceptedNavigationResponse<StreamingRawResponse>, String> {
        let response = self
            .load
            .fetch_intercepted_response(
                &self.method,
                self.requested_url.as_str(),
                self.body.clone(),
                self.headers.clone(),
                auth,
            )
            .await
            .map_err(|error| format!("failed to fetch page `{}`: {error}", self.requested_url))?;
        Ok(self.with_response(response))
    }

    pub async fn fetch_auth(
        self,
        auth: SubresourceAuthCredentials,
    ) -> Result<InterceptedNavigationResponse<RawResponse>, String> {
        let response = self
            .load
            .fetch_intercepted_auth_response(
                &self.method,
                self.requested_url.as_str(),
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
    ) -> InterceptedNavigationResponse<R> {
        let (response, observations) = response.into_parts_with_observation_journal();
        let mut prior = std::mem::take(&mut self.prior_observations);
        prior.append(observations);
        InterceptedNavigationResponse {
            work: self,
            response: NetworkFetchResult::with_observation_journal(response, prior),
        }
    }

    pub fn into_request_parts(
        self,
    ) -> (
        AdmittedNavigationLoad,
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

    pub(crate) fn into_owner_parts(
        self,
    ) -> (
        AdmittedNavigationLoad,
        Url,
        String,
        Option<Vec<u8>>,
        Vec<(String, String)>,
    ) {
        self.into_request_parts()
    }
}

#[derive(Debug)]
pub struct InterceptedNavigationResponse<R> {
    work: InterceptedNavigationLoad,
    response: NetworkFetchResult<R>,
}

impl<R> InterceptedNavigationResponse<R> {
    pub fn web_contents(&self) -> WebContentsId {
        self.identity().web_contents
    }

    pub fn response(&self) -> &R {
        self.response.response()
    }

    pub fn observation_journal(&self) -> &NetworkObservationJournal {
        self.response.observation_journal()
    }

    pub fn into_parts(self) -> (InterceptedNavigationLoad, NetworkFetchResult<R>) {
        (self.work, self.response)
    }

    pub(super) fn identity(&self) -> &DocumentNavigationIdentity {
        self.work.load.identity()
    }
}

impl InterceptedNavigationResponse<StreamingRawResponse> {
    pub async fn materialize(self) -> Result<InterceptedNavigationResponse<RawResponse>, String> {
        let (response, observations) = self.response.into_parts_with_observation_journal();
        let response = response
            .into_materialized_raw_response()
            .await
            .map_err(|error| format!("failed to read page body from stream: {error}"))?;
        Ok(InterceptedNavigationResponse {
            work: self.work,
            response: NetworkFetchResult::with_observation_journal(response, observations),
        })
    }
}

impl InterceptedNavigationResponse<RawResponse> {
    pub fn retry(self) -> InterceptedNavigationLoad {
        let (mut work, response) = self.into_parts();
        let (_, observations) = response.into_parts_with_observation_journal();
        work.prior_observations = observations;
        work
    }
}

#[derive(Debug)]
pub(super) struct PausedNavigationAuth {
    pub(super) request: BrowserRequestId,
    pub(super) response: InterceptedNavigationResponse<RawResponse>,
}

#[derive(Debug)]
pub(super) struct PausedNavigationRequest {
    pub(super) request: BrowserRequestId,
    pub(super) navigation_request: NavigationRequestInterception,
}

/// Browser-owned response-stage work for the exact pending navigation.
///
/// The DevTools projection keeps only the public request correlation and a
/// [`NavigationInterceptionPermit`]. Dropping or superseding the navigation
/// therefore drops the response stream and prepared renderer even if that
/// projection is never drained.
#[derive(Debug)]
pub(super) struct PausedNavigationResponse {
    pub(super) request: BrowserRequestId,
    pub(super) transfer: super::PausedDocumentTransfer,
}

#[derive(Debug)]
pub(super) enum PausedNavigationInterception {
    Request(Box<PausedNavigationRequest>),
    Auth(Box<PausedNavigationAuth>),
    Response(Box<PausedNavigationResponse>),
}

impl WebContents {
    pub fn pause_navigation_request(
        &mut self,
        navigation: NavigationId,
        request: NavigationRequestInterception,
    ) -> Result<NavigationInterceptionPermit, String> {
        self.navigation.pause_request(self.id, navigation, request)
    }

    pub fn take_navigation_request(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<ClaimedNavigationRequest> {
        if permit.web_contents != self.id {
            return None;
        }
        self.navigation.take_request(permit)
    }

    pub fn start_claimed_navigation_request(
        &mut self,
        request: ClaimedNavigationRequest,
        inherited: InheritedDocumentPolicy,
    ) -> Result<InterceptedNavigationLoad, String> {
        if request.permit.web_contents != self.id {
            return Err("navigation request belongs to another WebContents".to_owned());
        }
        let ClaimedNavigationRequest { permit, request } = request;
        let load = self.start_navigation_load(permit.navigation, request.policy, inherited)?;
        Ok(InterceptedNavigationLoad::new(
            load,
            request.requested_url,
            request.method,
            request.body,
            request.headers,
        ))
    }

    pub fn start_navigation_load_for_interception(
        &mut self,
        permit: NavigationInterceptionPermit,
        policy: NavigationRequestLoadPolicy,
        inherited: InheritedDocumentPolicy,
    ) -> Result<AdmittedNavigationLoad, String> {
        if permit.web_contents != self.id || !self.navigation.accepts_interception_permit(permit) {
            return Err("stale navigation document candidate".to_owned());
        }
        self.start_navigation_load(permit.navigation, policy, inherited)
    }

    pub fn pause_navigation_auth(
        &mut self,
        response: InterceptedNavigationResponse<RawResponse>,
    ) -> Result<NavigationInterceptionPermit, String> {
        if response.identity().web_contents != self.id {
            return Err("navigation auth belongs to another WebContents".to_owned());
        }
        self.navigation.pause_auth_response(response)
    }

    pub fn take_navigation_auth(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<InterceptedNavigationResponse<RawResponse>> {
        if permit.web_contents != self.id {
            return None;
        }
        self.navigation.take_auth_response(permit)
    }

    pub fn pause_navigation_response(
        &mut self,
        navigation: NavigationId,
        transfer: super::PausedDocumentTransfer,
    ) -> Result<NavigationInterceptionPermit, String> {
        self.navigation
            .pause_response(self.id, navigation, transfer)
    }

    pub fn take_navigation_response(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<super::PausedDocumentTransfer> {
        if permit.web_contents != self.id {
            return None;
        }
        self.navigation.take_response(permit)
    }

    pub fn restore_navigation_response(
        &mut self,
        permit: NavigationInterceptionPermit,
        transfer: super::PausedDocumentTransfer,
    ) -> Result<(), Box<super::PausedDocumentTransfer>> {
        if permit.web_contents != self.id {
            return Err(Box::new(transfer));
        }
        self.navigation.restore_response(permit, transfer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::web_contents::{
        DocumentBodySource, PausedDocumentTransfer, tests::BrowserFixture,
    };
    use crate::page::SubresourceAuthScheme;
    use moli_fetch::ResponseHead;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    fn request(browser: &mut BrowserFixture, url: Url) -> InterceptedNavigationLoad {
        let navigation = browser.contents.navigation.start_document_navigation();
        InterceptedNavigationLoad::new(
            browser.start(navigation).unwrap(),
            url,
            "POST".to_owned(),
            Some(vec![0, 255, 1]),
            vec![(
                "content-type".to_owned(),
                "application/octet-stream".to_owned(),
            )],
        )
    }

    fn challenge(work: InterceptedNavigationLoad) -> InterceptedNavigationResponse<RawResponse> {
        let response = RawResponse::from_head_and_body(
            ResponseHead {
                final_url: work.requested_url.clone(),
                status: 401,
                headers: vec![(
                    "WWW-Authenticate".to_owned(),
                    "Basic realm=\"test\"".to_owned(),
                )],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            b"challenge body".to_vec(),
        );
        work.with_response(NetworkFetchResult::without_request_observation(response))
    }

    fn paused_request(url: Url) -> NavigationRequestInterception {
        NavigationRequestInterception::new(
            url,
            "POST".to_owned(),
            Some(vec![0, 255, 1]),
            vec![(
                "content-type".to_owned(),
                "application/octet-stream".to_owned(),
            )],
            NavigationRequestLoadPolicy::DocumentInitiated,
        )
    }

    fn paused_response(url: Url) -> PausedDocumentTransfer {
        let response = RawResponse::from_head_and_body(
            ResponseHead {
                final_url: url.clone(),
                status: 200,
                headers: vec![("content-type".to_owned(), "text/html".to_owned())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            },
            b"response body".to_vec(),
        );
        PausedDocumentTransfer::pending(
            NavigationRequestLoadPolicy::DocumentInitiated,
            DocumentBodySource::BufferedRaw {
                requested_url: url,
                request_method: "GET".to_owned(),
                request_headers: Vec::new(),
                response,
                network_observation_journal: NetworkObservationJournal::default(),
            },
        )
    }

    #[test]
    fn request_permit_is_exact_single_use_and_excludes_other_pause_stages() {
        let mut browser = BrowserFixture::new();
        let navigation = browser.contents.navigation.start_document_navigation();
        let url = Url::parse("https://request.example/").unwrap();
        let permit = browser
            .contents
            .pause_navigation_request(navigation, paused_request(url.clone()))
            .unwrap();
        assert!(browser.contents.navigation.has_paused_request_for_test());

        let mut peer = BrowserFixture::new();
        assert!(peer.contents.take_navigation_request(permit).is_none());
        for bad in [
            NavigationInterceptionPermit {
                navigation: NavigationId::allocate(),
                ..permit
            },
            NavigationInterceptionPermit {
                document: DocumentId::allocate(),
                ..permit
            },
            NavigationInterceptionPermit {
                request: BrowserRequestId::allocate(),
                ..permit
            },
        ] {
            assert!(browser.contents.take_navigation_request(bad).is_none());
        }

        let admitted = browser.start(navigation).unwrap();
        assert!(
            browser
                .contents
                .pause_navigation_auth(challenge(InterceptedNavigationLoad::new(
                    admitted,
                    url.clone(),
                    "GET".to_owned(),
                    None,
                    Vec::new(),
                )))
                .is_err()
        );

        let claimed = browser.contents.take_navigation_request(permit).unwrap();
        assert!(browser.contents.take_navigation_request(permit).is_none());
        assert!(!browser.contents.navigation.has_paused_request_for_test());
        assert_eq!(claimed.request.requested_url, url);
    }

    #[test]
    fn auth_permit_is_exact_and_single_use_across_chained_pauses() {
        let mut browser = BrowserFixture::new();
        let url = Url::parse("https://auth.example/").unwrap();
        let work = request(&mut browser, url.clone());
        let renderer = work.load.renderer_page();
        let permit = browser
            .contents
            .pause_navigation_auth(challenge(work))
            .unwrap();
        let mut peer = BrowserFixture::new();
        assert!(peer.contents.take_navigation_auth(permit).is_none());
        for bad in [
            NavigationInterceptionPermit {
                navigation: NavigationId::allocate(),
                ..permit
            },
            NavigationInterceptionPermit {
                document: DocumentId::allocate(),
                ..permit
            },
            NavigationInterceptionPermit {
                request: BrowserRequestId::allocate(),
                ..permit
            },
        ] {
            assert!(browser.contents.take_navigation_auth(bad).is_none());
        }
        let response = browser.contents.take_navigation_auth(permit).unwrap();
        assert_eq!(response.response().body_bytes(), b"challenge body");
        assert!(browser.contents.take_navigation_auth(permit).is_none());
        let work = response.retry();
        assert_eq!(work.requested_url, url);
        assert_eq!(work.body, Some(vec![0, 255, 1]));
        assert_eq!(
            work.load.renderer_page(),
            renderer,
            "auth must not re-admit a new renderer"
        );
        let next = browser
            .contents
            .pause_navigation_auth(challenge(work))
            .unwrap();
        assert_ne!(next.request, permit.request);
        assert!(browser.contents.take_navigation_auth(permit).is_none());
        assert!(browser.contents.take_navigation_auth(next).is_some());
    }

    #[test]
    fn response_permit_is_exact_single_use_and_excludes_other_pause_stages() {
        let mut browser = BrowserFixture::new();
        let navigation = browser.contents.navigation.start_document_navigation();
        let url = Url::parse("https://response.example/").unwrap();
        let permit = browser
            .contents
            .pause_navigation_response(navigation, paused_response(url.clone()))
            .unwrap();
        assert!(
            browser
                .contents
                .navigation
                .paused_response_for_test()
                .is_some()
        );
        assert!(
            browser
                .contents
                .pause_navigation_request(navigation, paused_request(url.clone()))
                .is_err()
        );

        let admitted = browser.start(navigation).unwrap();
        assert!(
            browser
                .contents
                .pause_navigation_auth(challenge(InterceptedNavigationLoad::new(
                    admitted,
                    url,
                    "GET".to_owned(),
                    None,
                    Vec::new(),
                )))
                .is_err()
        );

        let mut peer = BrowserFixture::new();
        assert!(peer.contents.take_navigation_response(permit).is_none());
        for bad in [
            NavigationInterceptionPermit {
                navigation: NavigationId::allocate(),
                ..permit
            },
            NavigationInterceptionPermit {
                document: DocumentId::allocate(),
                ..permit
            },
            NavigationInterceptionPermit {
                request: BrowserRequestId::allocate(),
                ..permit
            },
        ] {
            assert!(browser.contents.take_navigation_response(bad).is_none());
        }
        let transfer = browser.contents.take_navigation_response(permit).unwrap();
        assert!(browser.contents.take_navigation_response(permit).is_none());
        assert!(
            browser
                .contents
                .navigation
                .paused_response_for_test()
                .is_none()
        );
        browser
            .contents
            .restore_navigation_response(permit, transfer)
            .unwrap();
        assert!(
            browser
                .contents
                .navigation
                .paused_response_for_test()
                .is_some()
        );
    }

    #[test]
    fn browser_retirement_releases_auth_before_protocol_correlation_cleanup() {
        for close in [false, true] {
            let mut browser = BrowserFixture::new();
            let work = request(&mut browser, Url::parse("https://auth.example/").unwrap());
            // The only extra strong engine lease is the actual paused Browser
            // participant. A protocol permit cannot retain it.
            let cancellation = work.load.identity().cancellation.clone();
            let preparation = work.load.identity().preparation_cancellation.clone();
            let permit = browser
                .contents
                .pause_navigation_auth(challenge(work))
                .unwrap();
            if close {
                browser
                    .contents
                    .navigation
                    .clear_document_navigation_state();
            } else {
                browser.contents.navigation.start_document_navigation();
            }
            assert!(cancellation.is_cancelled());
            assert!(preparation.is_cancelled());
            assert!(browser.contents.take_navigation_auth(permit).is_none());
            assert!(!browser.contents.navigation.has_paused_auth_for_test());
        }
    }

    #[test]
    fn late_auth_response_cannot_install_itself_in_a_winning_navigation() {
        let mut browser = BrowserFixture::new();
        let work = request(&mut browser, Url::parse("https://auth.example/").unwrap());
        let winner = browser.contents.navigation.start_document_navigation();
        assert!(
            browser
                .contents
                .pause_navigation_auth(challenge(work))
                .is_err()
        );
        assert_eq!(
            browser.contents.navigation.pending_document().unwrap().0,
            winner
        );
        assert!(!browser.contents.navigation.has_paused_auth_for_test());
    }

    #[tokio::test]
    async fn browser_retirement_cancels_buffered_auth_transport_before_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/auth", listener.local_addr().unwrap())).unwrap();
        let (seen_tx, seen_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let byte = socket.read_u8().await.unwrap();
                head.push(byte);
            }
            seen_tx.send(()).unwrap();
            // No response is released. Retirement must close the transport,
            // not merely reject its result after the server eventually replies.
            let mut remaining = Vec::new();
            socket.read_to_end(&mut remaining).await.unwrap();
            let _ = socket.shutdown().await;
        });
        let mut browser = BrowserFixture::new();
        let work = request(&mut browser, url);
        let auth = SubresourceAuthCredentials {
            target: crate::page::SubresourceAuthTarget::Server,
            username: "user".to_owned(),
            password: "pass".to_owned(),
            scheme: SubresourceAuthScheme::Digest,
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let (result, ()) = tokio::join!(work.fetch_auth(auth), async {
                seen_rx.await.unwrap();
                browser.contents.navigation.start_document_navigation();
            });
            assert!(
                result.is_err(),
                "retired auth must not complete successfully"
            );
            server.await.unwrap();
        })
        .await
        .expect("Browser retirement must cancel the buffered auth transport");
    }
}
