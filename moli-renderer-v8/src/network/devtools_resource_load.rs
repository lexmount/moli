use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
use moli_fetch::{FetchCancelHandle, Request};
use moli_page_types::NavigationRedirect;

use super::ResourceRequestClient;

pub enum RendererNetworkResourceLoadPreparation {
    Ready(Box<RendererPreparedNetworkResourceLoad>),
    FrameNotFound,
    CspViolation,
    UnsupportedUrlScheme,
}

pub struct RendererPreparedNetworkResourceLoad {
    request_client: ResourceRequestClient,
    request: Request,
}

impl RendererPreparedNetworkResourceLoad {
    pub(crate) fn new(request_client: ResourceRequestClient, request: Request) -> Self {
        Self {
            request_client,
            request,
        }
    }

    pub async fn execute(self) -> RendererNetworkResourceLoadOutcome {
        self.execute_with_cancel(FetchCancelHandle::new()).await
    }

    pub(crate) async fn execute_with_cancel(
        self,
        cancel_handle: FetchCancelHandle,
    ) -> RendererNetworkResourceLoadOutcome {
        let fetch_result = match self
            .request_client
            .fetch_raw_stream_with_cancel_and_network_metadata(self.request, cancel_handle)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return RendererNetworkResourceLoadOutcome::FailedBeforeResponse(format!(
                    "{error:#}"
                ));
            }
        };
        let (mut response, request_observation) = fetch_result.into_parts();
        let network_request_headers =
            request_observation.map(|observation| observation.into_headers());
        let head = response.head();
        let mut body = Vec::new();
        while let Some(chunk) = response.next_chunk().await {
            body.extend_from_slice(&chunk);
        }
        let completion_error = response
            .finish()
            .await
            .err()
            .map(|error| format!("{error:#}"));
        RendererNetworkResourceLoadOutcome::Response(Box::new(
            RendererNetworkResourceLoadResponse {
                final_url: head.final_url,
                status: head.status,
                headers: head.headers,
                body,
                completion_error,
                request_cookie_report: head.request_cookie_report,
                cookie_set_reports: head.cookie_set_reports,
                redirect_chain: head.redirect_chain.into_iter().map(Into::into).collect(),
                from_cache: head.from_cache,
                negotiated_http_version: head.negotiated_http_version,
                network_request_headers,
            },
        ))
    }
}

pub enum RendererNetworkResourceLoadOutcome {
    Response(Box<RendererNetworkResourceLoadResponse>),
    FailedBeforeResponse(String),
}

pub struct RendererNetworkResourceLoadResponse {
    pub final_url: url::Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub completion_error: Option<String>,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub cookie_set_reports: Vec<StoredCookieSetReport>,
    pub redirect_chain: Vec<NavigationRedirect>,
    pub from_cache: bool,
    pub negotiated_http_version: Option<moli_fetch::NegotiatedHttpVersion>,
    pub network_request_headers: Option<Vec<(String, String)>>,
}

impl RendererNetworkResourceLoadResponse {
    pub fn head(&self) -> moli_fetch::ResponseHead {
        moli_fetch::ResponseHead {
            final_url: self.final_url.clone(),
            status: self.status,
            headers: self.headers.clone(),
            request_cookie_report: self.request_cookie_report.clone(),
            cookie_set_reports: self.cookie_set_reports.clone(),
            redirected: !self.redirect_chain.is_empty(),
            redirect_chain: self
                .redirect_chain
                .iter()
                .cloned()
                .map(Into::into)
                .collect(),
            from_cache: self.from_cache,
            negotiated_http_version: self.negotiated_http_version,
        }
    }
}
