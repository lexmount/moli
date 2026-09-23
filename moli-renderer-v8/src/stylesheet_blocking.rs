use std::{future::Future, pin::Pin};

use moli_web_mime::{
    FetchDestination, MimeSniffingContext, computed_response_mime_type, is_css_mime,
    should_response_be_blocked_due_to_nosniff,
};
use url::Url;

use crate::network::ResourceRequestClient;
use crate::service_worker_runtime::{ServiceWorkerClientId, ServiceWorkerRequestDestination};
use crate::subresource_integrity::response_matches_subresource_integrity_metadata;
use crate::types::{AsyncSubresourceFetchResponseFilter, SubresourceResourceType};

pub(crate) use moli_stylesheet_blocking::{
    DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheetDiscoveryInput,
    StylesheetBlockingOperation, StylesheetBlockingReadView, StylesheetBlockingState,
    StylesheetBlockingStatus, StylesheetCompletion, StylesheetElementRead, StylesheetFetch,
    StylesheetFetchIdentity, StylesheetFetchOptions, StylesheetFetchTerminal, StylesheetFetcher,
    StylesheetImportGraphFetchResult, StylesheetImportNetworkResult, StylesheetPhysicalOutcome,
    StylesheetResourceKey,
    collect_document_owned_blocking_stylesheet_discovery_inputs_before_in_view,
    collect_document_owned_blocking_stylesheets,
    collect_document_owned_blocking_stylesheets_before_in_view, connected_preload_like_link_url,
    document_owned_blocking_stylesheet_candidate_for_node, link_rel_includes_token,
    preload_like_link_loads_stylesheet, stylesheet_link_disposition,
    stylesheet_preload_link_request,
};

#[derive(Clone)]
pub(crate) struct ServiceWorkerStylesheetFetchContext {
    pub(crate) browser_context_runtime: crate::runtime::RendererBrowserContextRuntime,
    pub(crate) client_id: ServiceWorkerClientId,
}

#[derive(Clone)]
pub(crate) struct RendererStylesheetFetcher {
    loader: crate::network::context::DocumentResourceLoader,
    service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
    request_resource_type: moli_fetch::RequestResourceType,
    link_preload: bool,
    completion_producer: Option<crate::page_task_queue::RendererPageStylesheetTaskProducer>,
}

impl RendererStylesheetFetcher {
    pub(crate) fn with_completion_producer(
        mut self,
        producer: crate::page_task_queue::RendererPageStylesheetTaskProducer,
    ) -> Self {
        self.completion_producer = Some(producer);
        self
    }

    pub(crate) fn resource_loader(&self) -> &crate::network::context::DocumentResourceLoader {
        &self.loader
    }

    pub(crate) fn with_preload_metadata(
        mut self,
        resource_type: moli_fetch::RequestResourceType,
        link_preload: bool,
    ) -> Self {
        self.request_resource_type = resource_type;
        self.link_preload = link_preload;
        self
    }

    pub(crate) fn new(
        loader: crate::network::context::DocumentResourceLoader,
        service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
    ) -> Self {
        Self {
            loader,
            service_worker_context,
            request_resource_type: moli_fetch::RequestResourceType::CssStyleSheet,
            link_preload: false,
            completion_producer: None,
        }
    }

    pub(crate) fn for_speculative_preload(
        loader: crate::network::context::DocumentResourceLoader,
        service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
        request_resource_type: moli_fetch::RequestResourceType,
        link_preload: bool,
    ) -> Self {
        Self {
            loader,
            service_worker_context,
            request_resource_type,
            link_preload,
            completion_producer: None,
        }
    }
}

impl StylesheetFetcher for RendererStylesheetFetcher {
    fn completion_publisher(
        &self,
    ) -> Option<moli_stylesheet_blocking::StylesheetCompletionPublisher> {
        self.completion_producer.clone().map(|producer| {
            std::sync::Arc::new(move |completion| {
                let _ = producer.send_blocking_completion(completion);
            }) as moli_stylesheet_blocking::StylesheetCompletionPublisher
        })
    }

    fn resource_cache_scope(&self) -> u64 {
        self.loader.identity().value()
    }
    fn spawn_stylesheet_task(&self, task: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        self.loader.spawn_resource_task(task);
    }

    fn fetch_stylesheet_resource(
        &self,
        document_url: Url,
        url: Url,
        options: StylesheetFetchOptions,
    ) -> Pin<Box<dyn Future<Output = StylesheetFetchTerminal> + Send + 'static>> {
        if self.loader.request_client().author_styles_disabled() {
            return Box::pin(async move {
                StylesheetFetchTerminal::network_error(format!(
                    "failed to fetch stylesheet `{url}`: net::ERR_BLOCKED_BY_CLIENT"
                ))
            });
        }
        let loader = self.loader.request_client().clone();
        let request_origin = self.loader.fetch_context().request_origin();
        let resource_task_runner = self.loader.task_runner();
        let service_worker_context = self.service_worker_context.clone();
        let request_resource_type = self.request_resource_type;
        let link_preload = self.link_preload;
        Box::pin(async move {
            fetch_stylesheet_readiness_with_service_worker(
                loader,
                request_origin,
                resource_task_runner,
                document_url,
                url,
                options,
                service_worker_context,
                request_resource_type,
                link_preload,
            )
            .await
        })
    }

    fn fetch_stylesheet_import_graph(
        &self,
        document_url: Url,
        urls: Vec<Url>,
    ) -> Pin<
        Box<
            dyn Future<Output = moli_stylesheet_blocking::StylesheetImportGraphFetchResult>
                + Send
                + 'static,
        >,
    > {
        Box::pin(
            crate::document_runtime::fetch_complete_stylesheet_import_graph(
                self.clone(),
                document_url,
                urls,
            ),
        )
    }
}

pub(crate) async fn fetch_stylesheet_readiness_with_service_worker(
    loader: ResourceRequestClient,
    request_origin: moli_url::WebOrigin,
    resource_task_runner: crate::network::RendererResourceTaskRunner,
    document_url: Url,
    url: Url,
    options: StylesheetFetchOptions,
    service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
    request_resource_type: moli_fetch::RequestResourceType,
    link_preload: bool,
) -> StylesheetFetchTerminal {
    let request = stylesheet_readiness_request(
        &document_url,
        &request_origin,
        &url,
        &options,
        request_resource_type,
        link_preload,
        None,
    );
    if let Some(context) = service_worker_context {
        match context
            .browser_context_runtime
            .fetch_service_worker_subresource_for_client_with_metadata(
                context.client_id,
                document_url.clone(),
                &request,
                &loader,
                resource_task_runner,
                ServiceWorkerRequestDestination::Style,
                SubresourceResourceType::Stylesheet,
            )
            .await
        {
            Ok(Some(response)) => {
                let response_provenance = StylesheetResponseProvenance::ServiceWorker {
                    filter: response.response_filter,
                };
                return stylesheet_terminal_from_response(
                    &request_origin,
                    &url,
                    &options,
                    *response.response,
                    response_provenance,
                );
            }
            Ok(None) => {}
            Err(error) => {
                return StylesheetFetchTerminal::network_error(format!(
                    "failed to fetch stylesheet `{url}` through service worker: {error}"
                ));
            }
        }
    }
    fetch_stylesheet_readiness_with_request(loader, request_origin, url, options, request).await
}

pub(crate) fn stylesheet_request_mode_and_credentials(
    options: &StylesheetFetchOptions,
) -> (moli_fetch::RequestMode, moli_fetch::RequestCredentialsMode) {
    options.request_mode_and_credentials()
}

pub(crate) fn apply_stylesheet_request_parameters(
    request: moli_fetch::Request,
    options: &StylesheetFetchOptions,
) -> moli_fetch::Request {
    let (request_mode, credentials_mode) = stylesheet_request_mode_and_credentials(options);
    request
        .with_request_mode(request_mode)
        .with_credentials_mode(credentials_mode)
        .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Style)
        .with_subresource_request_metadata(moli_fetch::SubresourceRequestMetadata {
            referrer_policy: options.referrer_policy().map(str::to_owned),
            document_referrer_policy: None,
            integrity: options.integrity().map(str::to_owned),
        })
}

fn stylesheet_readiness_request(
    document_url: &Url,
    request_origin: &moli_url::WebOrigin,
    url: &Url,
    options: &StylesheetFetchOptions,
    resource_type: moli_fetch::RequestResourceType,
    link_preload: bool,
    fetch_priority_hint: Option<moli_fetch::FetchPriorityHint>,
) -> moli_fetch::Request {
    let mut request = moli_fetch::Request::new("GET", url.as_str(), None, vec![])
        .expect("stylesheet url should already be parsed")
        .with_page_network_policy()
        .with_initiator_url(document_url)
        .with_request_origin(request_origin.clone())
        .with_resource_type(resource_type);
    let captured_fetch_priority =
        moli_fetch::FetchPriorityHint::from_attribute(options.fetch_priority());
    request = apply_stylesheet_request_parameters(request, options);
    if link_preload {
        request = request.with_link_preload();
    }
    if fetch_priority_hint.is_some() || captured_fetch_priority.is_some() {
        request = request.with_fetch_priority_hint(fetch_priority_hint.or(captured_fetch_priority));
    }
    request
}

async fn fetch_stylesheet_readiness_with_request(
    loader: ResourceRequestClient,
    request_origin: moli_url::WebOrigin,
    url: Url,
    options: StylesheetFetchOptions,
    request: moli_fetch::Request,
) -> StylesheetFetchTerminal {
    match loader.fetch_text_stream(request).await {
        Ok(response) => stylesheet_terminal_from_response(
            &request_origin,
            &url,
            &options,
            crate::protocol_types::NavigationResponse::from(response),
            StylesheetResponseProvenance::Network,
        ),
        Err(error) => StylesheetFetchTerminal::network_error(format!(
            "failed to fetch stylesheet `{url}`: {error}"
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StylesheetResponseProvenance {
    Network,
    ServiceWorker {
        filter: Option<AsyncSubresourceFetchResponseFilter>,
    },
}

impl StylesheetResponseProvenance {
    fn is_cors_same_origin(
        &self,
        request_origin: &moli_url::WebOrigin,
        head: &moli_fetch::ResponseHead,
    ) -> bool {
        match self {
            // Fetch classifies data responses as basic despite their opaque URL
            // origin, while retaining taint from cross-origin HTTP redirects.
            Self::Network => crate::network_host::network_response_filter(
                request_origin,
                head,
                moli_fetch::RequestMode::NoCors,
                moli_fetch::RequestRedirectMode::Follow,
            )
            .is_none(),
            Self::ServiceWorker { filter } => !matches!(
                filter,
                Some(
                    AsyncSubresourceFetchResponseFilter::Opaque
                        | AsyncSubresourceFetchResponseFilter::OpaqueRedirect
                )
            ),
        }
    }
}

fn stylesheet_response_url_chain_is_same_origin(
    request_origin: &moli_url::WebOrigin,
    request_url: &Url,
    head: &moli_fetch::ResponseHead,
) -> bool {
    request_origin.same_origin(&moli_url::WebOrigin::from_url(request_url))
        && !head.url_list().has_cross_origin_url(request_origin)
}

fn stylesheet_terminal_from_response(
    request_origin: &moli_url::WebOrigin,
    request_url: &Url,
    options: &StylesheetFetchOptions,
    response: crate::protocol_types::NavigationResponse,
    response_provenance: StylesheetResponseProvenance,
) -> StylesheetFetchTerminal {
    let (request_mode, credentials_mode) = options.request_mode_and_credentials();
    let head = response.head();
    let cors_usability =
        (request_mode == moli_fetch::RequestMode::Cors).then(|| match &response_provenance {
            StylesheetResponseProvenance::ServiceWorker {
                filter:
                    Some(
                        AsyncSubresourceFetchResponseFilter::Opaque
                        | AsyncSubresourceFetchResponseFilter::OpaqueRedirect,
                    ),
            } => Err(format!(
                "failed to fetch stylesheet `{request_url}`: CORS response is opaque"
            )),
            StylesheetResponseProvenance::ServiceWorker { .. } => Ok(()),
            StylesheetResponseProvenance::Network => {
                crate::network_host::validate_cors_response_chain(
                    request_origin,
                    &head,
                    credentials_mode,
                )
                .map_err(|error| format!("failed to fetch stylesheet `{request_url}`: {error}"))
            }
        });
    let origin_clean = cors_usability.as_ref().map_or_else(
        || response_provenance.is_cors_same_origin(request_origin, &head),
        Result::is_ok,
    );
    let fetch_usability = if !(200..=299).contains(&response.status) {
        Err(format!(
            "failed to fetch stylesheet `{request_url}`: HTTP status {}",
            response.status
        ))
    } else {
        cors_usability.unwrap_or(Ok(()))
    };
    if let Err(reason) = fetch_usability {
        return StylesheetFetchTerminal::unusable_response(response, origin_clean, reason);
    }
    if !response_matches_subresource_integrity_metadata(
        response.body_bytes(),
        options.integrity(),
        origin_clean,
    ) {
        return StylesheetFetchTerminal::integrity_failure(
            response,
            origin_clean,
            format!(
                "failed to fetch stylesheet `{request_url}`: subresource integrity check failed"
            ),
        );
    }
    let allow_non_css_mime = options.quirks_mode_mime_compatibility()
        && stylesheet_response_url_chain_is_same_origin(request_origin, request_url, &head);
    let usability = validate_stylesheet_response_ref(request_url, &response, allow_non_css_mime);

    match usability {
        Ok(()) => StylesheetFetchTerminal::ready(response, origin_clean),
        Err(reason) => StylesheetFetchTerminal::unusable_response(response, origin_clean, reason),
    }
}

pub(crate) fn validate_stylesheet_response(
    url: &Url,
    response: crate::protocol_types::NavigationResponse,
) -> Result<crate::protocol_types::NavigationResponse, String> {
    validate_stylesheet_response_ref(url, &response, false)?;
    Ok(response)
}

fn validate_stylesheet_response_ref(
    url: &Url,
    response: &crate::protocol_types::NavigationResponse,
    allow_non_css_mime: bool,
) -> Result<(), String> {
    if should_response_be_blocked_due_to_nosniff(&response.headers, FetchDestination::Style) {
        return Err(format!(
            "failed to fetch stylesheet `{url}`: blocked by X-Content-Type-Options nosniff"
        ));
    }
    let computed_mime_type = computed_response_mime_type(
        &response.headers,
        MimeSniffingContext::Style,
        response.body_bytes(),
    );
    if !allow_non_css_mime && !is_css_mime(&computed_mime_type) {
        return Err(format!(
            "failed to fetch stylesheet `{url}`: unsupported stylesheet MIME type `{computed_mime_type}`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use moli_crypto::DigestAlgorithm;

    fn sha384_integrity(body: &[u8]) -> String {
        format!(
            "sha384-{}",
            STANDARD.encode(DigestAlgorithm::Sha384.digest_bytes(body))
        )
    }

    fn integrity_options(cross_origin: Option<&str>, integrity: &str) -> StylesheetFetchOptions {
        StylesheetFetchOptions::from_link_attributes(
            cross_origin,
            None,
            Some(integrity),
            None,
            None,
            None,
        )
    }

    fn stylesheet_response(
        url: &Url,
        content_type: Option<&str>,
        body: &str,
    ) -> crate::protocol_types::NavigationResponse {
        let headers = content_type
            .map(|value| vec![("Content-Type".to_owned(), value.as_bytes().to_vec())])
            .unwrap_or_default();
        crate::protocol_types::NavigationResponse::from_text_body(
            url.clone(),
            200,
            headers,
            body.to_owned(),
        )
    }

    fn stylesheet_redirect(from_url: &Url, to_url: &Url) -> crate::types::NavigationRedirect {
        crate::types::NavigationRedirect {
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
    fn stylesheet_integrity_checks_raw_bytes_and_strongest_hash() {
        let document_url = Url::parse("https://example.test/page").unwrap();
        let stylesheet_url = document_url.join("/app.css").unwrap();
        let raw_body = b"/*\xff*/ body { color: green; }";
        let text = String::from_utf8_lossy(raw_body).into_owned();
        let response = stylesheet_response(&stylesheet_url, Some("text/css"), &text);
        let response = crate::protocol_types::NavigationResponse::from_head_and_body(
            response.head(),
            text.clone(),
            raw_body.to_vec(),
        );
        let matching = sha384_integrity(raw_body);
        let wrong = sha384_integrity(b"wrong");
        let weaker = format!(
            "sha256-{}",
            STANDARD.encode(DigestAlgorithm::Sha256.digest_bytes(raw_body))
        );

        for (integrity, expected) in [
            (matching.clone(), true),
            (wrong.clone(), false),
            (sha384_integrity(text.as_bytes()), false),
            (format!("{weaker} {wrong}"), false),
            (format!("{wrong} {matching}"), true),
        ] {
            let terminal = stylesheet_terminal_from_response(
                &moli_url::WebOrigin::from_url(&document_url),
                &stylesheet_url,
                &integrity_options(None, &integrity),
                response.clone(),
                StylesheetResponseProvenance::Network,
            );
            assert_eq!(terminal.is_ready(), expected, "{integrity}");
            assert_eq!(terminal.failed_integrity(), !expected, "{integrity}");
            assert_eq!(terminal.ready_response().is_some(), expected);
            assert_eq!(
                terminal.physical().as_result().unwrap().body_bytes(),
                raw_body,
                "SRI rejection must retain the physical response for network bookkeeping"
            );
        }
    }

    #[test]
    fn stylesheet_integrity_requires_cors_for_cross_origin_network_responses() {
        let document_url = Url::parse("https://page.example.test/").unwrap();
        let stylesheet_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let body = "body { color: green; }";
        let integrity = sha384_integrity(body.as_bytes());

        for (cross_origin, credentials_allowed, expected) in [
            (None, false, false),
            (Some("anonymous"), false, true),
            (Some("use-credentials"), true, true),
            (Some("use-credentials"), false, false),
        ] {
            let mut response = stylesheet_response(&stylesheet_url, Some("text/css"), body);
            response.headers.push((
                "Access-Control-Allow-Origin".to_owned(),
                b"https://page.example.test".to_vec(),
            ));
            if credentials_allowed {
                response.headers.push((
                    "Access-Control-Allow-Credentials".to_owned(),
                    b"true".to_vec(),
                ));
            }
            let terminal = stylesheet_terminal_from_response(
                &moli_url::WebOrigin::from_url(&document_url),
                &stylesheet_url,
                &integrity_options(cross_origin, &integrity),
                response,
                StylesheetResponseProvenance::Network,
            );
            assert_eq!(
                terminal.is_ready(),
                expected,
                "crossorigin={cross_origin:?}, ACAC={credentials_allowed}"
            );
        }
    }

    #[test]
    fn stylesheet_integrity_uses_committed_request_origin() {
        let stylesheet_url = Url::parse("https://page.example.test/app.css").unwrap();
        let body = "body { color: green; }";
        let integrity = sha384_integrity(body.as_bytes());

        for (cross_origin, allow_origin, expected) in [
            (None, None, false),
            (Some("anonymous"), Some("https://page.example.test"), false),
            (Some("anonymous"), Some("null"), true),
        ] {
            let mut response = stylesheet_response(&stylesheet_url, Some("text/css"), body);
            if let Some(allow_origin) = allow_origin {
                response.headers.push((
                    "Access-Control-Allow-Origin".to_owned(),
                    allow_origin.as_bytes().to_vec(),
                ));
            }
            let terminal = stylesheet_terminal_from_response(
                &moli_url::WebOrigin::Opaque,
                &stylesheet_url,
                &integrity_options(cross_origin, &integrity),
                response,
                StylesheetResponseProvenance::Network,
            );
            assert_eq!(
                terminal.is_ready(),
                expected,
                "{cross_origin:?}, {allow_origin:?}"
            );
            assert_eq!(terminal.origin_clean(), Some(expected));
            assert_eq!(terminal.failed_integrity(), cross_origin.is_none());
        }
    }

    #[test]
    fn stylesheet_integrity_checks_service_worker_response_filter() {
        let document_url = Url::parse("https://page.example.test/").unwrap();
        let stylesheet_url = document_url.join("/app.css").unwrap();
        let response_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let body = "body { color: green; }";
        let integrity = sha384_integrity(body.as_bytes());

        for (filter, expected) in [
            (None, true),
            (Some(AsyncSubresourceFetchResponseFilter::Basic), true),
            (
                Some(AsyncSubresourceFetchResponseFilter::Cors(vec![])),
                true,
            ),
            (Some(AsyncSubresourceFetchResponseFilter::Opaque), false),
            (
                Some(AsyncSubresourceFetchResponseFilter::OpaqueRedirect),
                false,
            ),
        ] {
            let terminal = stylesheet_terminal_from_response(
                &moli_url::WebOrigin::from_url(&document_url),
                &stylesheet_url,
                &integrity_options(None, &integrity),
                stylesheet_response(&response_url, Some("text/css"), body),
                StylesheetResponseProvenance::ServiceWorker {
                    filter: filter.clone(),
                },
            );
            assert_eq!(terminal.is_ready(), expected, "{filter:?}");
            assert_eq!(terminal.ready_response().is_some(), expected);
        }
    }

    #[test]
    fn stylesheet_integrity_accepts_readable_data_responses() {
        let document_url = Url::parse("https://page.example.test/").unwrap();
        let body = "body { color: green; }";
        let stylesheet_url = Url::parse(&format!(
            "data:text/css;base64,{}",
            STANDARD.encode(body.as_bytes())
        ))
        .unwrap();
        let matching = sha384_integrity(body.as_bytes());
        let wrong = sha384_integrity(b"wrong");

        for cross_origin in [None, Some("anonymous"), Some("use-credentials")] {
            for (integrity, expected) in [(&matching, true), (&wrong, false)] {
                let terminal = stylesheet_terminal_from_response(
                    &moli_url::WebOrigin::from_url(&document_url),
                    &stylesheet_url,
                    &integrity_options(cross_origin, integrity),
                    stylesheet_response(&stylesheet_url, Some("text/css"), body),
                    StylesheetResponseProvenance::Network,
                );
                assert_eq!(
                    terminal.is_ready(),
                    expected,
                    "{cross_origin:?}: {integrity}"
                );
                assert_eq!(terminal.origin_clean(), Some(true));
            }
        }
    }

    #[test]
    fn stylesheet_integrity_rejects_no_cors_redirect_taint() {
        let document_url = Url::parse("https://page.example.test/").unwrap();
        let stylesheet_url = document_url.join("/app.css").unwrap();
        let cross_origin_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let body = "body { color: green; }";
        let integrity = sha384_integrity(body.as_bytes());

        for (request_url, redirects, expected) in [
            (&stylesheet_url, vec![], true),
            (
                &stylesheet_url,
                vec![
                    stylesheet_redirect(&stylesheet_url, &cross_origin_url),
                    stylesheet_redirect(&cross_origin_url, &stylesheet_url),
                ],
                false,
            ),
            (
                &cross_origin_url,
                vec![stylesheet_redirect(&cross_origin_url, &stylesheet_url)],
                false,
            ),
        ] {
            let mut response = stylesheet_response(&stylesheet_url, Some("text/css"), body);
            response.redirected = !redirects.is_empty();
            response.redirect_chain = redirects;
            let terminal = stylesheet_terminal_from_response(
                &moli_url::WebOrigin::from_url(&document_url),
                request_url,
                &integrity_options(None, &integrity),
                response,
                StylesheetResponseProvenance::Network,
            );
            assert_eq!(terminal.is_ready(), expected, "{request_url}");
        }
    }

    #[test]
    fn validates_stylesheet_response_rejects_explicit_non_css_mime() {
        let url = Url::parse("https://example.com/app.css").unwrap();
        let response = stylesheet_response(&url, Some("text/html"), "body { color: red; }");

        assert!(
            validate_stylesheet_response(&url, response)
                .expect_err("text/html stylesheet should be rejected")
                .contains("unsupported stylesheet MIME type `text/html`")
        );
    }

    #[test]
    fn validates_stylesheet_response_allows_missing_content_type_in_style_context() {
        let url = Url::parse("https://example.com/app.css").unwrap();
        let response = stylesheet_response(&url, None, "body { color: red; }");

        let response = validate_stylesheet_response(&url, response)
            .expect("missing Content-Type stylesheet should be accepted");
        assert_eq!(response.body_text(), "body { color: red; }");
    }

    #[test]
    fn quirks_mode_mime_compatibility_requires_same_origin_final_url() {
        let document_url = Url::parse("https://page.example.test/document").unwrap();
        let request_url = Url::parse("https://page.example.test/app.css").unwrap();
        let options = StylesheetFetchOptions::default().with_quirks_mode_mime_compatibility(true);
        let same_origin_response =
            stylesheet_response(&request_url, Some("text/plain"), "body { color: green; }");

        let same_origin_terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &request_url,
            &options,
            same_origin_response,
            StylesheetResponseProvenance::Network,
        );

        assert!(same_origin_terminal.is_ready());

        let cross_origin_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let cross_origin_response = stylesheet_response(
            &cross_origin_url,
            Some("text/plain"),
            "body { color: red; }",
        );
        let cross_origin_terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &request_url,
            &options,
            cross_origin_response,
            StylesheetResponseProvenance::Network,
        );

        assert!(!cross_origin_terminal.is_ready());
    }

    #[test]
    fn quirks_mode_mime_compatibility_uses_committed_request_origin() {
        let request_url = Url::parse("https://page.example.test/app.css").unwrap();
        let options = StylesheetFetchOptions::default().with_quirks_mode_mime_compatibility(true);
        let inherited_origin = moli_url::WebOrigin::from_serialized("https://page.example.test");

        for (request_origin, expected_ready) in [
            (inherited_origin, true),
            (moli_url::WebOrigin::Opaque, false),
        ] {
            let response =
                stylesheet_response(&request_url, Some("text/plain"), "body { color: green; }");
            let terminal = stylesheet_terminal_from_response(
                &request_origin,
                &request_url,
                &options,
                response,
                StylesheetResponseProvenance::Network,
            );

            assert_eq!(terminal.is_ready(), expected_ready);
            assert_eq!(terminal.origin_clean(), Some(expected_ready));
        }
    }

    #[test]
    fn quirks_mode_mime_compatibility_rejects_cross_origin_redirect_taint() {
        let document_url = Url::parse("https://page.example.test/document").unwrap();
        let same_origin_url = Url::parse("https://page.example.test/app.css").unwrap();
        let cross_origin_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let options = StylesheetFetchOptions::default().with_quirks_mode_mime_compatibility(true);

        let mut cross_to_same_response =
            stylesheet_response(&same_origin_url, Some("text/plain"), "body { color: red; }");
        cross_to_same_response.redirected = true;
        cross_to_same_response.redirect_chain =
            vec![stylesheet_redirect(&cross_origin_url, &same_origin_url)];

        let cross_to_same_terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &cross_origin_url,
            &options,
            cross_to_same_response,
            StylesheetResponseProvenance::Network,
        );

        assert!(!cross_to_same_terminal.is_ready());
        assert_eq!(cross_to_same_terminal.origin_clean(), Some(false));

        let mut through_cross_response =
            stylesheet_response(&same_origin_url, Some("text/plain"), "body { color: red; }");
        through_cross_response.redirected = true;
        through_cross_response.redirect_chain = vec![
            stylesheet_redirect(&same_origin_url, &cross_origin_url),
            stylesheet_redirect(&cross_origin_url, &same_origin_url),
        ];

        let through_cross_terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &same_origin_url,
            &options,
            through_cross_response,
            StylesheetResponseProvenance::Network,
        );

        assert!(!through_cross_terminal.is_ready());
        assert_eq!(through_cross_terminal.origin_clean(), Some(false));
    }

    #[test]
    fn quirks_mode_mime_compatibility_does_not_bypass_nosniff() {
        let document_url = Url::parse("https://page.example.test/document").unwrap();
        let stylesheet_url = Url::parse("https://page.example.test/app.css").unwrap();
        let options = StylesheetFetchOptions::default().with_quirks_mode_mime_compatibility(true);
        let response = crate::protocol_types::NavigationResponse::from_text_body(
            stylesheet_url.clone(),
            200,
            vec![
                ("Content-Type".to_owned(), b"text/plain".to_vec()),
                ("x-content-type-options".to_owned(), b"nosniff".to_vec()),
            ],
            "body { color: red; }".to_owned(),
        );

        let terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &stylesheet_url,
            &options,
            response,
            StylesheetResponseProvenance::Network,
        );

        assert!(!terminal.is_ready());
    }

    #[test]
    fn linked_stylesheet_request_uses_captured_processing_attributes() {
        let document_url = Url::parse("https://example.com/page").unwrap();
        let stylesheet_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let options = StylesheetFetchOptions::from_link_attributes(
            Some("anonymous"),
            Some("no-referrer"),
            Some("sha384-integrity"),
            Some("nonce-value"),
            Some("utf-8"),
            Some("high"),
        );

        let request = stylesheet_readiness_request(
            &document_url,
            &moli_url::WebOrigin::from_url(&document_url),
            &stylesheet_url,
            &options,
            moli_fetch::RequestResourceType::CssStyleSheet,
            false,
            None,
        );
        let metadata = request
            .subresource_request_metadata()
            .expect("captured link metadata");

        assert_eq!(
            request.browser_request_metadata(),
            Some(moli_fetch::BrowserRequestMetadata::Style)
        );
        assert_eq!(request.request_mode, moli_fetch::RequestMode::Cors);
        assert_eq!(
            request.credentials_mode,
            moli_fetch::RequestCredentialsMode::SameOrigin
        );
        assert_eq!(metadata.referrer_policy.as_deref(), Some("no-referrer"));
        assert_eq!(metadata.integrity.as_deref(), Some("sha384-integrity"));
        assert_eq!(
            request.priority_hints.fetch_priority,
            Some(moli_fetch::FetchPriorityHint::High)
        );
        assert_eq!(options.nonce(), Some("nonce-value"));
        assert_eq!(options.charset(), Some("utf-8"));
        assert!(
            !request.allows_credentials_for_url(&stylesheet_url),
            "anonymous CORS stylesheet requests must not include cross-origin credentials"
        );
    }

    #[test]
    fn linked_stylesheet_without_crossorigin_uses_no_cors_request_parameters() {
        let document_url = Url::parse("https://example.com/page").unwrap();
        let stylesheet_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let request = stylesheet_readiness_request(
            &document_url,
            &moli_url::WebOrigin::from_url(&document_url),
            &stylesheet_url,
            &StylesheetFetchOptions::default(),
            moli_fetch::RequestResourceType::CssStyleSheet,
            false,
            None,
        );

        assert_eq!(request.request_mode, moli_fetch::RequestMode::NoCors);
        assert_eq!(
            request.credentials_mode,
            moli_fetch::RequestCredentialsMode::Include
        );
        assert_eq!(
            request.browser_request_metadata(),
            Some(moli_fetch::BrowserRequestMetadata::Style)
        );
    }

    #[test]
    fn anonymous_cors_stylesheet_response_is_ready_and_origin_clean() {
        let document_url = Url::parse("https://page.example.test/").unwrap();
        let stylesheet_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let options = StylesheetFetchOptions::from_link_attributes(
            Some("anonymous"),
            None,
            None,
            None,
            None,
            None,
        );
        let response = crate::protocol_types::NavigationResponse::from_text_body(
            stylesheet_url.clone(),
            200,
            vec![
                ("Content-Type".to_owned(), b"text/css".to_vec()),
                (
                    "Access-Control-Allow-Origin".to_owned(),
                    b"https://page.example.test".to_vec(),
                ),
            ],
            "body { color: green; }".to_owned(),
        );

        let terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &stylesheet_url,
            &options,
            response,
            StylesheetResponseProvenance::Network,
        );

        assert!(terminal.is_ready());
        assert_eq!(terminal.origin_clean(), Some(true));
        assert!(terminal.ready_response().is_some());
    }

    #[test]
    fn cors_rejection_keeps_the_physical_stylesheet_response() {
        let document_url = Url::parse("https://page.example.test/").unwrap();
        let stylesheet_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let options = StylesheetFetchOptions::from_link_attributes(
            Some("anonymous"),
            None,
            None,
            None,
            None,
            None,
        );
        let response = crate::protocol_types::NavigationResponse::from_text_body(
            stylesheet_url.clone(),
            200,
            vec![("Content-Type".to_owned(), b"text/css".to_vec())],
            "body { color: red; }".to_owned(),
        );

        let terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &stylesheet_url,
            &options,
            response,
            StylesheetResponseProvenance::Network,
        );

        assert!(!terminal.is_ready());
        assert_eq!(terminal.origin_clean(), Some(false));
        let physical = terminal
            .physical()
            .as_result()
            .expect("CORS rejection must retain its physical response");
        assert_eq!(physical.status, 200);
        assert_eq!(physical.body_text(), "body { color: red; }");
    }

    #[test]
    fn http_failure_keeps_response_but_is_not_stylesheet_ready() {
        let document_url = Url::parse("https://example.test/").unwrap();
        let stylesheet_url = Url::parse("https://example.test/missing.css").unwrap();
        let response = crate::protocol_types::NavigationResponse::from_text_body(
            stylesheet_url.clone(),
            404,
            vec![("Content-Type".to_owned(), b"text/css".to_vec())],
            "body { color: red; }".to_owned(),
        );

        let terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &stylesheet_url,
            &StylesheetFetchOptions::default(),
            response,
            StylesheetResponseProvenance::Network,
        );

        assert!(!terminal.is_ready());
        assert_eq!(
            terminal.origin_clean(),
            Some(true),
            "same-origin physical responses stay origin-clean even when HTTP status makes the stylesheet unusable"
        );
        assert_eq!(
            terminal
                .physical()
                .as_result()
                .expect("HTTP failure still has a response")
                .status,
            404
        );
    }

    #[test]
    fn service_worker_basic_no_cors_response_stays_origin_clean_after_cross_origin_redirect() {
        let document_url = Url::parse("https://page.example.test/").unwrap();
        let request_url = Url::parse("https://page.example.test/app.css").unwrap();
        let response_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let response =
            stylesheet_response(&response_url, Some("text/css"), "body { color: green; }");

        let terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &request_url,
            &StylesheetFetchOptions::default(),
            response,
            StylesheetResponseProvenance::ServiceWorker { filter: None },
        );

        assert!(terminal.is_ready());
        assert_eq!(
            terminal.origin_clean(),
            Some(true),
            "a basic service-worker response is CORS-same-origin independently of its response URL"
        );
    }

    #[test]
    fn service_worker_basic_cors_response_does_not_require_network_acao() {
        let document_url = Url::parse("https://page.example.test/").unwrap();
        let request_url = Url::parse("https://cdn.example.test/app.css").unwrap();
        let options = StylesheetFetchOptions::from_link_attributes(
            Some("anonymous"),
            None,
            None,
            None,
            None,
            None,
        );
        let response =
            stylesheet_response(&request_url, Some("text/css"), "body { color: green; }");

        let terminal = stylesheet_terminal_from_response(
            &moli_url::WebOrigin::from_url(&document_url),
            &request_url,
            &options,
            response,
            StylesheetResponseProvenance::ServiceWorker { filter: None },
        );

        assert!(terminal.is_ready());
        assert_eq!(terminal.origin_clean(), Some(true));
    }
}
