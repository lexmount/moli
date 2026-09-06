use moli_core::{
    browser::{BrowserRequestId, DocumentId, NavigationId, WebContentsId},
    page::SubresourceAuthCredentials,
};
use moli_fetch::{
    NetworkFetchResult, NetworkObservationJournal, RawResponse, StreamingRawResponse,
};
use url::Url;

use super::{AdmittedNavigationLoad, WebContents, navigation_commit::DocumentNavigationIdentity};

/// A single Browser decision. Copying a protocol correlation cannot duplicate
/// its authority: the owning pending navigation consumes this request once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NavigationInterceptionPermit {
    pub(super) web_contents: WebContentsId,
    pub(super) navigation: NavigationId,
    pub(super) document: DocumentId,
    pub(super) request: BrowserRequestId,
}

impl NavigationInterceptionPermit {
    pub(crate) fn navigation(self) -> NavigationId {
        self.navigation
    }

    pub(in crate::conn) fn web_contents(self) -> WebContentsId {
        self.web_contents
    }

    #[cfg(test)]
    pub(crate) fn for_correlation_test() -> Self {
        Self {
            web_contents: WebContentsId::allocate(),
            navigation: NavigationId::allocate(),
            document: DocumentId::allocate(),
            request: BrowserRequestId::allocate(),
        }
    }
}

/// The admitted Browser operation retains the actual request across auth
/// pauses and retries. No Target, session, loader ID or frontend configuration
/// is needed to resume it, and no Browser borrow is held while fetching.
pub(crate) struct InterceptedNavigationLoad {
    pub(in crate::conn) load: AdmittedNavigationLoad,
    pub(in crate::conn) requested_url: Url,
    pub(in crate::conn) method: String,
    body: Option<Vec<u8>>,
    pub(in crate::conn) headers: Vec<(String, String)>,
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
    pub(in crate::conn) fn new(
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

    pub(crate) async fn fetch_streaming(
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

    pub(crate) async fn fetch_auth(
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

    fn with_response<R>(
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
}

#[derive(Debug)]
pub(crate) struct InterceptedNavigationResponse<R> {
    work: InterceptedNavigationLoad,
    response: NetworkFetchResult<R>,
}

impl<R> InterceptedNavigationResponse<R> {
    pub(in crate::conn) fn web_contents(&self) -> WebContentsId {
        self.identity().web_contents
    }

    pub(crate) fn response(&self) -> &R {
        self.response.response()
    }

    pub(crate) fn observation_journal(&self) -> &NetworkObservationJournal {
        self.response.observation_journal()
    }

    pub(crate) fn into_parts(self) -> (InterceptedNavigationLoad, NetworkFetchResult<R>) {
        (self.work, self.response)
    }

    pub(super) fn identity(&self) -> &DocumentNavigationIdentity {
        self.work.load.identity()
    }
}

impl InterceptedNavigationResponse<StreamingRawResponse> {
    pub(crate) async fn materialize(
        self,
    ) -> Result<InterceptedNavigationResponse<RawResponse>, String> {
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
    pub(crate) fn retry(self) -> InterceptedNavigationLoad {
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

impl WebContents {
    pub(in crate::conn::state) fn pause_navigation_auth(
        &mut self,
        response: InterceptedNavigationResponse<RawResponse>,
    ) -> Result<NavigationInterceptionPermit, String> {
        if response.identity().web_contents != self.id {
            return Err("navigation auth belongs to another WebContents".to_owned());
        }
        self.navigation.pause_auth_response(response)
    }

    pub(in crate::conn::state) fn take_navigation_auth(
        &mut self,
        permit: NavigationInterceptionPermit,
    ) -> Option<InterceptedNavigationResponse<RawResponse>> {
        if permit.web_contents != self.id {
            return None;
        }
        self.navigation.take_auth_response(permit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::state::web_contents::tests::BrowserFixture;
    use moli_core::page::SubresourceAuthScheme;
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
            target: moli_core::page::SubresourceAuthTarget::Server,
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
