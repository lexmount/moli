use std::{future::Future, pin::Pin};

use moli_web_mime::{
    FetchDestination, MimeSniffingContext, computed_response_mime_type, is_css_mime,
    should_response_be_blocked_due_to_nosniff,
};
use url::Url;

use crate::network::context::{DocumentResourceLoader, ResourceResponseProvenance};
use crate::service_worker_runtime::ServiceWorkerClientId;
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
    loader: DocumentResourceLoader,
    service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
    request_resource_type: moli_fetch::RequestResourceType,
    link_preload: bool,
    initiator: crate::types::SubresourceRequestInitiatorType,
}

impl RendererStylesheetFetcher {
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
        loader: DocumentResourceLoader,
        service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
    ) -> Self {
        Self {
            loader,
            service_worker_context,
            request_resource_type: moli_fetch::RequestResourceType::CssStyleSheet,
            link_preload: false,
            initiator: crate::types::SubresourceRequestInitiatorType::Parser,
        }
    }

    pub(crate) fn for_speculative_preload(
        loader: DocumentResourceLoader,
        service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
        request_resource_type: moli_fetch::RequestResourceType,
        link_preload: bool,
    ) -> Self {
        Self {
            loader,
            service_worker_context,
            request_resource_type,
            link_preload,
            initiator: crate::types::SubresourceRequestInitiatorType::Parser,
        }
    }

    pub(crate) fn for_css_imports(mut self) -> Self {
        self.initiator = crate::types::SubresourceRequestInitiatorType::Css;
        self
    }
}

impl StylesheetFetcher for RendererStylesheetFetcher {
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
        let loader = self.loader.clone();
        let service_worker_context = self.service_worker_context.clone();
        let request_resource_type = self.request_resource_type;
        let link_preload = self.link_preload;
        let initiator = self.initiator;
        Box::pin(async move {
            fetch_stylesheet_readiness_with_service_worker(
                loader,
                document_url,
                url,
                options,
                service_worker_context,
                request_resource_type,
                link_preload,
                initiator,
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

async fn fetch_stylesheet_readiness_with_service_worker(
    loader: DocumentResourceLoader,
    document_url: Url,
    url: Url,
    options: StylesheetFetchOptions,
    service_worker_context: Option<ServiceWorkerStylesheetFetchContext>,
    request_resource_type: moli_fetch::RequestResourceType,
    link_preload: bool,
    initiator: crate::types::SubresourceRequestInitiatorType,
) -> StylesheetFetchTerminal {
    let request_origin = loader.fetch_context().request_origin();
    let request = stylesheet_readiness_request(
        &document_url,
        &request_origin,
        &url,
        &options,
        request_resource_type,
        link_preload,
        None,
    );
    let mut loader = loader;
    if let Some(context) = service_worker_context {
        loader.bind_service_worker(context.browser_context_runtime, context.client_id);
    }
    match loader
        .fetch_resource(request, SubresourceResourceType::Stylesheet, initiator)
        .await
    {
        Ok((response, provenance)) => {
            stylesheet_terminal_from_response(&request_origin, &url, &options, response, provenance)
        }
        Err(error) => StylesheetFetchTerminal::network_error(format!(
            "failed to fetch stylesheet `{url}`: {error}"
        )),
    }
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

impl ResourceResponseProvenance {
    fn is_cors_same_origin(
        self,
        request_origin: &moli_url::WebOrigin,
        head: &moli_fetch::ResponseHead,
    ) -> bool {
        match self {
            Self::Network => !head.url_list().has_cross_origin_url(request_origin),
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

fn stylesheet_terminal_from_response(
    request_origin: &moli_url::WebOrigin,
    request_url: &Url,
    options: &StylesheetFetchOptions,
    response: crate::protocol_types::NavigationResponse,
    response_provenance: ResourceResponseProvenance,
) -> StylesheetFetchTerminal {
    let (request_mode, credentials_mode) = options.request_mode_and_credentials();
    let head = response.head();
    let cors_usability =
        (request_mode == moli_fetch::RequestMode::Cors).then(|| match response_provenance {
            ResourceResponseProvenance::ServiceWorker {
                filter:
                    Some(
                        AsyncSubresourceFetchResponseFilter::Opaque
                        | AsyncSubresourceFetchResponseFilter::OpaqueRedirect,
                    ),
            } => Err(format!(
                "failed to fetch stylesheet `{request_url}`: CORS response is opaque"
            )),
            ResourceResponseProvenance::ServiceWorker { .. } => Ok(()),
            ResourceResponseProvenance::Network => {
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
    let usability = if !(200..=299).contains(&response.status) {
        Err(format!(
            "failed to fetch stylesheet `{request_url}`: HTTP status {}",
            response.status
        ))
    } else {
        cors_usability.unwrap_or(Ok(()))
    }
    .and_then(|()| validate_stylesheet_response_ref(request_url, &response));

    match usability {
        Ok(()) => StylesheetFetchTerminal::ready(response, origin_clean),
        Err(reason) => StylesheetFetchTerminal::unusable_response(response, origin_clean, reason),
    }
}

pub(crate) fn validate_stylesheet_response(
    url: &Url,
    response: crate::protocol_types::NavigationResponse,
) -> Result<crate::protocol_types::NavigationResponse, String> {
    validate_stylesheet_response_ref(url, &response)?;
    Ok(response)
}

fn validate_stylesheet_response_ref(
    url: &Url,
    response: &crate::protocol_types::NavigationResponse,
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
    if !is_css_mime(&computed_mime_type) {
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
            .map(|value| vec![("Content-Type".to_owned(), value.as_bytes().to_vec())])
            .unwrap_or_default();
        crate::protocol_types::NavigationResponse::from_text_body(
            url.clone(),
            200,
            headers,
            body.to_owned(),
        )
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
            ResourceResponseProvenance::Network,
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
            ResourceResponseProvenance::Network,
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
            ResourceResponseProvenance::Network,
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
            ResourceResponseProvenance::ServiceWorker { filter: None },
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
            ResourceResponseProvenance::ServiceWorker { filter: None },
        );

        assert!(terminal.is_ready());
        assert_eq!(terminal.origin_clean(), Some(true));
    }
}
