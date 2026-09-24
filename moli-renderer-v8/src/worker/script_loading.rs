use moli_fetch::RedirectInfo;
use url::Url;

impl crate::runtime::RendererDedicatedWorkerHost {
    pub(crate) fn start_main_script_request(
        &self,
        script_url: &Url,
        initiator_url: &Url,
    ) -> Option<std::sync::Arc<crate::network::ResourceTransfer>> {
        crate::network::ResourceTransfer::start_main_script(
            self.network(),
            super::WorkerNetworkObserver::Dedicated(self.network_observer()),
            script_url,
            initiator_url,
        )
    }
}

impl crate::network::ResourceTransfer {
    pub(crate) fn start_script(
        source: &crate::runtime::RendererWorkerNetworkReporter,
        observer: super::WorkerNetworkObserver,
        script_url: &Url,
        initiator_url: &Url,
    ) -> Option<std::sync::Arc<Self>> {
        Self::for_worker(source, observer, |request| {
            super::global_scope::worker_request_started(
                request,
                initiator_url,
                script_url,
                "GET",
                &moli_fetch::RequestHeaders::default(),
                &None,
                moli_page_types::SubresourceResourceType::Script,
            )
        })
    }

    pub(crate) fn materialize_script_response<T>(
        &self,
        response: moli_fetch::Response,
        materialize: impl FnOnce(moli_fetch::Response) -> Result<T, String>,
    ) -> Result<T, String> {
        let body = moli_page_types::SubresourceResponseBody::from_fetch_response(&response);
        let head = crate::network::ResourceResponseHead {
            status_text: None,
            head: response.head(),
            network_request_headers: response
                .network_request_extra_info()
                .map(|info| info.headers.clone()),
        };
        self.finish_script_response(head, body, materialize(response))
    }

    pub(crate) fn start_main_script(
        source: &crate::runtime::RendererWorkerNetworkReporter,
        observer: super::WorkerNetworkObserver,
        script_url: &Url,
        initiator_url: &Url,
    ) -> Option<std::sync::Arc<Self>> {
        let mut url = script_url.clone();
        url.set_fragment(None);
        crate::network::ResourceTransfer::for_worker(source, observer, |request| {
            moli_page_types::SubresourceRequestStarted::new(
                request.handle(),
                None,
                initiator_url.clone(),
                url,
                "GET".into(),
                moli_fetch::RequestHeaders::default(),
                None,
                moli_page_types::SubresourceResourceType::Script,
                moli_page_types::SubresourceRequestInitiatorType::Other,
                None,
            )
            .with_worker_main_script()
        })
    }

    pub(crate) fn main_script_response<T>(
        &self,
        response: &crate::protocol_types::NavigationResponse,
        result: Result<T, String>,
    ) -> Result<T, String> {
        let body = moli_page_types::SubresourceResponseBody::from_navigation_response(response);
        let head = crate::network::ResourceResponseHead {
            status_text: None,
            head: response.head(),
            network_request_headers: response.network_request_headers().map(<[_]>::to_vec),
        };
        self.finish_script_response(head, body, result)
    }

    fn finish_script_response<T>(
        &self,
        head: crate::network::ResourceResponseHead,
        body: moli_page_types::SubresourceResponseBody,
        result: Result<T, String>,
    ) -> Result<T, String> {
        match &result {
            Ok(_) => self.body_completed(head, body),
            Err(message) => self.failed(&crate::network::ResourceResponseFailure::PartialBody {
                message: message.clone(),
                response: std::sync::Arc::new(head),
                body,
            }),
        }
        result
    }
}

pub(crate) fn ensure_worker_script_redirect_chain_same_origin(
    initiator_url: &Url,
    redirect_chain: &[RedirectInfo],
    final_url: &Url,
) -> Result<(), String> {
    if !matches!(initiator_url.scheme(), "http" | "https") {
        return Ok(());
    }
    for redirect in redirect_chain {
        if !moli_url::same_origin(initiator_url, &redirect.from_url) {
            return Err(format!(
                "cross-origin redirect from `{}` is not allowed.",
                redirect.from_url
            ));
        }
        if !moli_url::same_origin(initiator_url, &redirect.to_url) {
            return Err(format!(
                "cross-origin redirect to `{}` is not allowed.",
                redirect.to_url
            ));
        }
    }
    if !moli_url::same_origin(initiator_url, final_url) {
        return Err(format!(
            "cross-origin redirect to `{final_url}` is not allowed."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    fn redirect(from_url: &Url, to_url: &Url) -> RedirectInfo {
        RedirectInfo {
            source: moli_fetch::RedirectSource::Network,
            from_url: from_url.clone(),
            to_url: to_url.clone(),
            status: 302,
            headers: Vec::new(),
            network_extra_info_available: true,
            request_extra_info: None,
            response_extra_info: None,
            redirect_has_extra_info: true,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        }
    }

    #[test]
    fn worker_script_redirect_chain_rejects_cross_origin_final_url() {
        let initiator = url("https://app.test/page.html");
        let same_origin = url("https://app.test/worker.js");
        let cross_origin = url("https://evil.test/worker.js");

        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &[], &same_origin).is_ok()
        );
        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &[], &cross_origin)
                .is_err()
        );
    }

    #[test]
    fn worker_script_redirect_chain_rejects_cross_origin_intermediate_hop() {
        let initiator = url("https://app.test/page.html");
        let same_origin_redirect = url("https://app.test/redirect");
        let cross_origin_redirect = url("https://evil.test/redirect");
        let same_origin_final = url("https://app.test/worker.js");
        let chain = vec![
            redirect(&same_origin_redirect, &cross_origin_redirect),
            redirect(&cross_origin_redirect, &same_origin_final),
        ];

        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &chain, &same_origin_final)
                .is_err()
        );
    }

    #[test]
    fn worker_script_redirect_chain_accepts_same_origin_hops() {
        let initiator = url("https://app.test/page.html");
        let same_origin_redirect = url("https://app.test/redirect");
        let same_origin_middle = url("https://app.test/middle");
        let same_origin_final = url("https://app.test/worker.js");
        let chain = vec![
            redirect(&same_origin_redirect, &same_origin_middle),
            redirect(&same_origin_middle, &same_origin_final),
        ];

        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &chain, &same_origin_final)
                .is_ok()
        );
    }

    #[test]
    fn worker_script_redirect_chain_skips_opaque_initiators() {
        let initiator = url("data:text/html,hello");
        let cross_origin = url("https://evil.test/worker.js");

        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &[], &cross_origin).is_ok()
        );
    }
}
