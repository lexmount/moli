//! Renderer projections of an immutable stylesheet import graph result.
//!
//! Graph acquisition does not know about DOM owners. This module is the one
//! boundary that decorates a terminal graph for network observation or turns
//! it into the inputs consumed by the live CSSOM installer.

use super::*;

pub(in crate::document_runtime) async fn fetch_observed_stylesheet_import_graph(
    stylesheet_fetcher: crate::stylesheet_blocking::RendererStylesheetFetcher,
    document_url: Url,
    urls: Vec<Url>,
    source_owners: Vec<DomHandle>,
) -> (
    Arc<crate::stylesheet_blocking::StylesheetImportGraphFetchResult>,
    Vec<ConnectedLoadNetworkResult>,
) {
    let graph = Arc::new(
        super::import_graph::fetch_complete_stylesheet_import_graph(
            stylesheet_fetcher,
            document_url.clone(),
            urls,
        )
        .await,
    );
    let network_results = graph
        .network_results()
        .iter()
        .map(|result| {
            let request_url = result.request_url().clone();
            let start_unix_millis = result.start_unix_millis();
            let terminal = result.terminal();
            let origin_clean = terminal.origin_clean().unwrap_or(false);
            let result = terminal.physical().as_result();
            ConnectedLoadNetworkResult {
                stylesheet_fetch: None,
                blocking_operation: None,
                source_operation: None,
                import_roots: Vec::new(),
                document_url: document_url.clone(),
                request_url,
                source_owners: source_owners.clone(),
                resource_type: SubresourceResourceType::Stylesheet,
                start_unix_millis: Some(start_unix_millis),
                origin_clean,
                result,
            }
        })
        .collect();
    (graph, network_results)
}

pub(crate) fn live_stylesheet_import_responses(
    graph: &crate::stylesheet_blocking::StylesheetImportGraphFetchResult,
) -> Vec<crate::live_stylesheet::LiveStylesheetImportResponse> {
    graph
        .network_results()
        .iter()
        .map(|result| {
            let terminal = result.terminal();
            let ready_response = terminal.ready_response();
            crate::live_stylesheet::LiveStylesheetImportResponse {
                request_url: result.request_url().clone(),
                response_url: match terminal.physical() {
                    crate::stylesheet_blocking::StylesheetPhysicalOutcome::Response(response) => {
                        response.final_url.clone()
                    }
                    crate::stylesheet_blocking::StylesheetPhysicalOutcome::NetworkError(_) => {
                        result.request_url().clone()
                    }
                },
                css_text: ready_response
                    .map(|response| response.body_text().to_owned())
                    .unwrap_or_default(),
                successful: ready_response.is_some(),
                origin_clean: terminal.origin_clean().unwrap_or(false),
            }
        })
        .collect()
}
