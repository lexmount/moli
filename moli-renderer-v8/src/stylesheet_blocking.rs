use std::{future::Future, pin::Pin};

use moli_web_mime::{
    FetchDestination, MimeSniffingContext, computed_response_mime_type, is_css_mime,
    should_response_be_blocked_due_to_nosniff,
};
use url::Url;

use crate::network::ResourceRequestClient;
use crate::service_worker_runtime::{ServiceWorkerClientId, ServiceWorkerRequestDestination};
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
    loader: ResourceRequestClient,
    task_runner: crate::network::RendererResourceTaskRunner,
    service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
    request_resource_type: moli_fetch::RequestResourceType,
    link_preload: bool,
}

impl RendererStylesheetFetcher {
    pub(crate) fn new(
        loader: ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
        service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
    ) -> Self {
        Self {
            loader,
            task_runner,
            service_worker_context,
            request_resource_type: moli_fetch::RequestResourceType::CssStyleSheet,
            link_preload: false,
        }
    }

    pub(crate) fn for_speculative_preload(
        loader: ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
        service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
        request_resource_type: moli_fetch::RequestResourceType,
        link_preload: bool,
    ) -> Self {
        Self {
            loader,
            task_runner,
            service_worker_context,
            request_resource_type,
            link_preload,
        }
    }
}

impl StylesheetFetcher for RendererStylesheetFetcher {
    fn spawn_stylesheet_task(&self, task: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        self.task_runner.spawn(task);
    }

    fn fetch_stylesheet_resource(
        &self,
        document_url: Url,
        url: Url,
        options: StylesheetFetchOptions,
    ) -> Pin<Box<dyn Future<Output = StylesheetFetchTerminal> + Send + 'static>> {
        let loader = self.loader.clone();
        let resource_task_runner = self.task_runner.clone();
        let service_worker_context = self.service_worker_context.clone();
        let request_resource_type = self.request_resource_type;
        let link_preload = self.link_preload;
        Box::pin(async move {
            fetch_stylesheet_readiness_with_service_worker(
                loader,
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
                    &document_url,
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
    fetch_stylesheet_readiness_with_request(loader, document_url, url, options, request).await
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
    document_url: Url,
    url: Url,
    options: StylesheetFetchOptions,
    request: moli_fetch::Request,
) -> StylesheetFetchTerminal {
    match loader.fetch_text_stream(request).await {
        Ok(response) => stylesheet_terminal_from_response(
            &document_url,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StylesheetResponseProvenance {
    Network,
    ServiceWorker {
        filter: Option<AsyncSubresourceFetchResponseFilter>,
    },
}

impl StylesheetResponseProvenance {
    fn is_cors_same_origin(
        self,
        document_url: &Url,
        request_url: &Url,
        response: &crate::protocol_types::NavigationResponse,
    ) -> bool {
        match self {
            Self::Network => {
                stylesheet_response_url_chain_is_same_origin(document_url, request_url, response)
            }
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
    document_url: &Url,
    request_url: &Url,
    response: &crate::protocol_types::NavigationResponse,
) -> bool {
    moli_url::same_origin(document_url, request_url)
        && response.redirect_chain.iter().all(|redirect| {
            moli_url::same_origin(document_url, &redirect.from_url)
                && moli_url::same_origin(document_url, &redirect.to_url)
        })
        && moli_url::same_origin(document_url, &response.final_url)
}

fn stylesheet_terminal_from_response(
    document_url: &Url,
    request_url: &Url,
    options: &StylesheetFetchOptions,
    response: crate::protocol_types::NavigationResponse,
    response_provenance: StylesheetResponseProvenance,
) -> StylesheetFetchTerminal {
    let (request_mode, credentials_mode) = options.request_mode_and_credentials();
    let cors_usability =
        (request_mode == moli_fetch::RequestMode::Cors).then(|| match response_provenance {
            StylesheetResponseProvenance::ServiceWorker {
                filter:
                    Some(
                        AsyncSubresourceFetchResponseFilter::Opaque
                        | AsyncSubresourceFetchResponseFilter::OpaqueRedirect,
                    ),
            } => Err(format!(
                "failed to fetch stylesheet `{request_url}`: CORS response is opaque"
            )),
            StylesheetResponseProvenance::ServiceWorker { filter: None } => Ok(()),
            StylesheetResponseProvenance::Network => crate::network_host::validate_cors_response(
                document_url,
                &response.final_url,
                &response.headers,
                credentials_mode,
            )
            .map_err(|error| format!("failed to fetch stylesheet `{request_url}`: {error}")),
        });
    let origin_clean = cors_usability.as_ref().map_or_else(
        || response_provenance.is_cors_same_origin(document_url, request_url, &response),
        Result::is_ok,
    );
    let usability = if !(200..=299).contains(&response.status) {
        Err(format!(
            "failed to fetch stylesheet `{request_url}`: HTTP status {}",
            response.status
        ))
    } else {
        cors_usability.unwrap_or(Ok(()))
    }
    .and_then(|()| {
        let allow_non_css_mime = options.quirks_mode_mime_compatibility()
            && stylesheet_response_url_chain_is_same_origin(document_url, request_url, &response);
        validate_stylesheet_response_ref(request_url, &response, allow_non_css_mime)
    });

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

    fn stylesheet_response(
        url: &Url,
        content_type: Option<&str>,
        body: &str,
    ) -> crate::protocol_types::NavigationResponse {
        let headers = content_type
            .map(|value| vec![("Content-Type".to_owned(), value.to_owned())])
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
            &document_url,
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
            &document_url,
            &request_url,
            &options,
            cross_origin_response,
            StylesheetResponseProvenance::Network,
        );

        assert!(!cross_origin_terminal.is_ready());
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
            &document_url,
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
            &document_url,
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
                ("Content-Type".to_owned(), "text/plain".to_owned()),
                ("x-content-type-options".to_owned(), "nosniff".to_owned()),
            ],
            "body { color: red; }".to_owned(),
        );

        let terminal = stylesheet_terminal_from_response(
            &document_url,
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
                ("Content-Type".to_owned(), "text/css".to_owned()),
                (
                    "Access-Control-Allow-Origin".to_owned(),
                    "https://page.example.test".to_owned(),
                ),
            ],
            "body { color: green; }".to_owned(),
        );

        let terminal = stylesheet_terminal_from_response(
            &document_url,
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
            vec![("Content-Type".to_owned(), "text/css".to_owned())],
            "body { color: red; }".to_owned(),
        );

        let terminal = stylesheet_terminal_from_response(
            &document_url,
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
            vec![("Content-Type".to_owned(), "text/css".to_owned())],
            "body { color: red; }".to_owned(),
        );

        let terminal = stylesheet_terminal_from_response(
            &document_url,
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
            &document_url,
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
            &document_url,
            &request_url,
            &options,
            response,
            StylesheetResponseProvenance::ServiceWorker { filter: None },
        );

        assert!(terminal.is_ready());
        assert_eq!(terminal.origin_clean(), Some(true));
    }
}
