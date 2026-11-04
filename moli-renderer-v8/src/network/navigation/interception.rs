use anyhow::{Result, bail};
use moli_fetch::{
    NetworkFetchResult, RawResponse, RedirectInfo, Request, Response, ResponseHead,
    StreamingRawResponse,
};
use url::Url;

use super::NavigationResourceLoader;

/// Request execution shared by top-level, child and auxiliary navigations.
/// The navigation owner decides when a hop may start and where its result can
/// commit. This object owns transport routing, request overrides and redirects;
/// it deliberately has no DevTools session or frame-lifecycle state.
#[derive(Debug)]
pub struct NavigationInterception {
    loader: NavigationResourceLoader,
    request: Request,
    navigation_url: Url,
    manual_redirects: bool,
}

struct CancelOnDrop(Option<NavigationResourceLoader>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(loader) = self.0.take() {
            loader.cancel();
        }
    }
}

impl NavigationInterception {
    pub fn new(loader: NavigationResourceLoader, request: Request) -> Self {
        Self {
            navigation_url: request.url.clone(),
            loader,
            request,
            manual_redirects: false,
        }
    }

    pub fn resource_loader(&self) -> &NavigationResourceLoader {
        &self.loader
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    pub fn request_mut(&mut self) -> &mut Request {
        &mut self.request
    }

    /// Return each HTTP redirect to the navigation owner for its next-hop decision.
    pub fn set_manual_redirects(&mut self, enabled: bool) {
        self.manual_redirects = enabled;
    }

    /// Overrides the current transport hop without changing the document's URL.
    pub fn apply_request_override(
        &mut self,
        url: Url,
        method: String,
        headers: moli_fetch::RequestHeaders,
        body: Option<bytes::Bytes>,
        redirect_headers: Option<moli_fetch::RequestHeaders>,
    ) {
        self.request.url = url;
        self.request.method = method;
        self.request.request_headers = headers;
        self.request.body = body;
        self.request.set_redirect_headers(redirect_headers);
    }

    pub fn navigation_url(&self, response: &ResponseHead) -> Url {
        if !self.manual_redirects && response.redirected {
            response.final_url.clone()
        } else {
            self.navigation_url.clone()
        }
    }

    fn transport_request(&self) -> Result<Request> {
        moli_url_policy::route_navigation_url(
            &self.request.url,
            moli_url_policy::LocalFileNavigationAccess::Denied,
        )?;
        Ok(if self.manual_redirects {
            self.request.clone().with_single_hop()
        } else {
            self.request.clone()
        })
    }

    async fn execute<T>(
        loader: &NavigationResourceLoader,
        operation: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        let mut guard = CancelOnDrop(Some(loader.clone()));
        let result = operation.await;
        guard.0.take();
        result
    }

    pub async fn fetch_text(&self) -> Result<NetworkFetchResult<Response>> {
        Self::execute(
            &self.loader,
            self.loader
                .fetch_with_network_metadata(self.transport_request()?),
        )
        .await
    }

    pub async fn fetch_raw(&self) -> Result<RawResponse> {
        Self::execute(
            &self.loader,
            self.loader.fetch_raw(self.transport_request()?),
        )
        .await
    }

    pub async fn fetch_raw_stream(&self) -> Result<NetworkFetchResult<StreamingRawResponse>> {
        Self::execute(
            &self.loader,
            self.loader
                .fetch_raw_stream_with_network_metadata(self.transport_request()?),
        )
        .await
    }

    /// Prepares an owned transfer future while keeping redirect state with its owner.
    pub fn fetch_buffered(
        &self,
    ) -> impl std::future::Future<Output = Result<NetworkFetchResult<RawResponse>>> + Send + 'static
    {
        let loader = self.loader.clone();
        let request = self.transport_request();
        async move { Self::execute(&loader, loader.fetch_raw_with_network_metadata(request?)).await }
    }

    /// Returns the next hop to its owner before any transport is started.
    /// Synthetic and network responses use exactly the same redirect rules.
    pub fn follow_redirect(&mut self, head: &ResponseHead) -> Result<Option<RedirectInfo>> {
        if !self.manual_redirects {
            return Ok(None);
        }
        let Some(next_url) = crate::network_host::next_redirect_url(
            &head.final_url,
            head.status,
            &head.headers,
            self.request.redirect_count(),
        )
        .map_err(anyhow::Error::msg)?
        else {
            return Ok(None);
        };
        if !matches!(next_url.scheme(), "http" | "https") {
            bail!("document redirect to unsupported URL {next_url}");
        }
        let redirect = RedirectInfo {
            source: moli_fetch::RedirectSource::Network,
            from_url: head.final_url.clone(),
            to_url: next_url.clone(),
            status: head.status,
            headers: head.headers.clone(),
            network_extra_info_available: false,
            request_extra_info: None,
            response_extra_info: None,
            redirect_has_extra_info: false,
            request_cookie_report: head.request_cookie_report.clone(),
            cookie_set_reports: head.cookie_set_reports.clone(),
            from_cache: head.from_cache,
            negotiated_http_version: head.negotiated_http_version,
        };
        self.request
            .update_referrer_for_redirect(&head.final_url, &head.headers);
        self.request.follow_redirect(redirect.clone())?;
        self.loader.restart_after_response()?;
        self.navigation_url = next_url;
        Ok(Some(redirect))
    }
}

#[cfg(test)]
mod tests;
