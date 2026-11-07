//! Fetching and readiness classification for stylesheet import graphs.
//!
//! This module has no owner lifecycle. It produces one immutable graph result;
//! linked and inline stylesheet code decide separately where that result may be
//! installed.

use super::*;
use crate::live_stylesheet::import_url_identity;
use crate::stylesheet_blocking::{StylesheetFetchOptions, StylesheetFetcher};
use futures_util::future::{BoxFuture, join_all};
use moli_encoding::decode_text_for_legacy_web;
use moli_web_mime::{data_url_body_and_mime_type, mime_charset};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::document_runtime) enum DataStylesheetImportReadiness {
    NoImports,
    Imports(Vec<Url>),
    Failed,
}

pub(in crate::document_runtime) enum ConnectedStyleImportReadiness {
    Ready(bool),
    Pending(Vec<Url>),
}

const MAX_DATA_STYLESHEET_IMPORT_EXPANSIONS: usize = 16;
const MAX_DATA_STYLESHEET_IMPORT_URL_BYTES: usize = 16 * 1024;

#[derive(Default)]
struct ConnectedNetworkStyleImportGraph {
    pending_urls: VecDeque<Url>,
    admitted_identities: HashSet<Url>,
}

impl ConnectedNetworkStyleImportGraph {
    fn extend(&mut self, urls: impl IntoIterator<Item = Url>) {
        for url in urls {
            let identity = import_url_identity(&url);
            if self.admitted_identities.insert(identity) {
                self.pending_urls.push_back(url);
            }
        }
    }

    fn take_pending(&mut self) -> Vec<Url> {
        self.pending_urls.drain(..).collect()
    }
}

pub(crate) fn fetch_complete_stylesheet_import_graph(
    stylesheet_fetcher: crate::stylesheet_blocking::RendererStylesheetFetcher,
    document_url: Url,
    urls: Vec<Url>,
) -> impl Future<Output = crate::stylesheet_blocking::StylesheetImportGraphFetchResult> + Send {
    fetch_stylesheet_import_graph(stylesheet_fetcher, document_url, urls, None)
}

/// Continue from a root fetched with its link-specific request options. Seed
/// both URL identities so an import cycle cannot fetch that root a second time.
pub(crate) fn fetch_complete_stylesheet_import_graph_from_root(
    stylesheet_fetcher: crate::stylesheet_blocking::RendererStylesheetFetcher,
    document_url: Url,
    root: crate::stylesheet_blocking::StylesheetImportNetworkResult,
) -> impl Future<Output = crate::stylesheet_blocking::StylesheetImportGraphFetchResult> + Send {
    fetch_stylesheet_import_graph(stylesheet_fetcher, document_url, Vec::new(), Some(root))
}

fn fetch_stylesheet_import_graph(
    stylesheet_fetcher: crate::stylesheet_blocking::RendererStylesheetFetcher,
    document_url: Url,
    urls: Vec<Url>,
    root: Option<crate::stylesheet_blocking::StylesheetImportNetworkResult>,
) -> impl Future<Output = crate::stylesheet_blocking::StylesheetImportGraphFetchResult> + Send {
    // A policy-bearing graph must admit data: imports before decoding them,
    // just like network imports. Keep them in the result so CSSOM cannot later
    // reconstruct the body of a refused local import.
    let check_data_imports = stylesheet_fetcher.csp_context().is_some();
    let readiness = move |urls: Vec<Url>| {
        if check_data_imports && !urls.is_empty() {
            ConnectedStyleImportReadiness::Pending(urls)
        } else {
            connected_style_import_readiness(urls)
        }
    };
    let (mut successful, urls) = match readiness(urls) {
        ConnectedStyleImportReadiness::Ready(successful) => (successful, Vec::new()),
        ConnectedStyleImportReadiness::Pending(urls) => (true, urls),
    };
    let mut network_results = Vec::new();
    let mut import_graph = ConnectedNetworkStyleImportGraph::default();
    let mut pending_results: Vec<_> = root.into_iter().collect();
    import_graph.extend(urls);
    let mut data_expansions = 0;
    // Prepare the initial requests before yielding to the parser. Descendants
    // are prepared after their parent response, under the policy then in force.
    let initial_requests = prepare_import_requests(
        &stylesheet_fetcher,
        &document_url,
        import_graph.take_pending(),
        &mut data_expansions,
    );
    async move {
        pending_results.extend(join_all(initial_requests).await);
        while !pending_results.is_empty() {
            // Admit every completed response in this batch before discovering any
            // descendants. Redirect aliases and their final URLs share an identity
            // for both root and imported sheets.
            for result in &pending_results {
                import_graph
                    .admitted_identities
                    .insert(import_url_identity(result.request_url()));
                if let Some(response) = result.terminal().ready_response() {
                    import_graph
                        .admitted_identities
                        .insert(import_url_identity(&response.final_url));
                }
            }
            for result in std::mem::take(&mut pending_results) {
                let terminal = result.terminal();
                successful &= terminal.is_ready();
                if let Some(response) = terminal.ready_response() {
                    let nested_urls = crate::style_engine::stylesheet_top_level_import_urls(
                        response.body_text(),
                        &response.final_url,
                        false,
                    )
                    .unwrap_or_default();
                    match readiness(nested_urls) {
                        ConnectedStyleImportReadiness::Ready(nested_successful) => {
                            successful &= nested_successful;
                        }
                        ConnectedStyleImportReadiness::Pending(urls) => import_graph.extend(urls),
                    }
                }
                network_results.push(result);
            }
            pending_results = join_all(prepare_import_requests(
                &stylesheet_fetcher,
                &document_url,
                import_graph.take_pending(),
                &mut data_expansions,
            ))
            .await;
        }
        crate::stylesheet_blocking::StylesheetImportGraphFetchResult::new(
            successful,
            network_results,
        )
    }
}

fn prepare_import_requests(
    fetcher: &crate::stylesheet_blocking::RendererStylesheetFetcher,
    document_url: &Url,
    urls: Vec<Url>,
    data_expansions: &mut usize,
) -> Vec<BoxFuture<'static, crate::stylesheet_blocking::StylesheetImportNetworkResult>> {
    urls.into_iter()
        .map(|url| -> BoxFuture<'static, _> {
            let started = moli_time::unix_epoch_millis();
            let terminal: BoxFuture<'static, _> = if url.scheme() == "data" {
                *data_expansions += 1;
                Box::pin(std::future::ready(fetch_data_stylesheet_import(
                    fetcher,
                    &url,
                    *data_expansions,
                )))
            } else {
                fetcher.fetch_stylesheet_resource(
                    document_url.clone(),
                    url.clone(),
                    StylesheetFetchOptions::default(),
                )
            };
            Box::pin(async move {
                crate::stylesheet_blocking::StylesheetImportNetworkResult::new(
                    url,
                    started,
                    terminal.await,
                )
            })
        })
        .collect()
}

fn fetch_data_stylesheet_import(
    fetcher: &crate::stylesheet_blocking::RendererStylesheetFetcher,
    url: &Url,
    expansion: usize,
) -> crate::stylesheet_blocking::StylesheetFetchTerminal {
    use crate::stylesheet_blocking::StylesheetFetchTerminal;
    if let Some(context) = fetcher.csp_context()
        && let Err(error) = context.for_request(None)(url, 0)
    {
        return StylesheetFetchTerminal::network_error(error.to_string());
    }
    if expansion > MAX_DATA_STYLESHEET_IMPORT_EXPANSIONS
        || url.as_str().len() > MAX_DATA_STYLESHEET_IMPORT_URL_BYTES
    {
        return StylesheetFetchTerminal::network_error("data stylesheet import expansion limit");
    }
    let Some((body, mime_type)) = data_url_body_and_mime_type(url.as_str()) else {
        return StylesheetFetchTerminal::network_error("invalid data stylesheet import");
    };
    let css_text = decode_text_for_legacy_web(&body, mime_charset(&mime_type).as_deref());
    StylesheetFetchTerminal::ready(
        crate::protocol_types::NavigationResponse::from_text_body(
            url.clone(),
            200,
            vec![("Content-Type".to_owned(), b"text/css".to_vec())],
            css_text,
        ),
        true,
    )
}

pub(in crate::document_runtime) fn data_stylesheet_import_readiness(
    stylesheet_url: &Url,
) -> DataStylesheetImportReadiness {
    if stylesheet_url.scheme() != "data" {
        return DataStylesheetImportReadiness::NoImports;
    }
    if stylesheet_url.as_str().len() > MAX_DATA_STYLESHEET_IMPORT_URL_BYTES {
        return DataStylesheetImportReadiness::Failed;
    }
    let Some((body, mime_type)) = data_url_body_and_mime_type(stylesheet_url.as_str()) else {
        return DataStylesheetImportReadiness::Failed;
    };
    // Chromium treats a data: URL selected by a stylesheet request as CSS even
    // when its media type is omitted or is not text/css. HTTP response MIME
    // enforcement belongs to the network stylesheet response validator and
    // must not be reused for this local-scheme path.
    let css_text = decode_text_for_legacy_web(&body, mime_charset(&mime_type).as_deref());
    let Ok(urls) =
        crate::style_engine::stylesheet_top_level_import_urls(&css_text, stylesheet_url, true)
    else {
        return DataStylesheetImportReadiness::Failed;
    };
    if urls.is_empty() {
        DataStylesheetImportReadiness::NoImports
    } else {
        DataStylesheetImportReadiness::Imports(urls)
    }
}

pub(in crate::document_runtime) fn connected_style_import_readiness(
    urls: Vec<Url>,
) -> ConnectedStyleImportReadiness {
    let mut pending = Vec::new();
    let mut stack = VecDeque::from(urls);
    let mut seen = HashSet::new();
    let mut data_expansions = 0;
    while let Some(url) = stack.pop_front() {
        if !seen.insert(import_url_identity(&url)) {
            continue;
        }
        if url.scheme() != "data" {
            pending.push(url);
            continue;
        }
        if url.as_str().len() > MAX_DATA_STYLESHEET_IMPORT_URL_BYTES {
            return ConnectedStyleImportReadiness::Ready(false);
        }
        data_expansions += 1;
        if data_expansions > MAX_DATA_STYLESHEET_IMPORT_EXPANSIONS {
            return ConnectedStyleImportReadiness::Ready(false);
        }
        match data_stylesheet_import_readiness(&url) {
            DataStylesheetImportReadiness::NoImports => {}
            DataStylesheetImportReadiness::Failed => {
                return ConnectedStyleImportReadiness::Ready(false);
            }
            DataStylesheetImportReadiness::Imports(imports) => {
                for import in imports.into_iter().rev() {
                    stack.push_front(import);
                }
            }
        }
    }
    if pending.is_empty() {
        ConnectedStyleImportReadiness::Ready(true)
    } else {
        ConnectedStyleImportReadiness::Pending(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_stylesheet_import_readiness_tracks_top_level_imports() {
        let stylesheet_url =
            Url::parse("data:/,@import url('https://example.test/imported.css');").unwrap();

        assert_eq!(
            data_stylesheet_import_readiness(&stylesheet_url),
            DataStylesheetImportReadiness::Imports(vec![
                Url::parse("https://example.test/imported.css").unwrap()
            ])
        );
    }

    #[test]
    fn connected_style_import_readiness_preserves_external_import_order() {
        let first = Url::parse("https://example.test/first.css").unwrap();
        let second = Url::parse("https://example.test/second.css").unwrap();

        let ConnectedStyleImportReadiness::Pending(pending) =
            connected_style_import_readiness(vec![first.clone(), second.clone()])
        else {
            panic!("external imports must remain pending");
        };

        assert_eq!(pending, vec![first, second]);
    }

    #[test]
    fn connected_style_import_readiness_deduplicates_url_fragments() {
        let first = Url::parse("https://example.test/shared.css#first").unwrap();
        let duplicate = Url::parse("https://example.test/shared.css#second").unwrap();

        let ConnectedStyleImportReadiness::Pending(pending) =
            connected_style_import_readiness(vec![first.clone(), duplicate])
        else {
            panic!("external import must remain pending");
        };

        assert_eq!(pending, vec![first]);
    }

    #[test]
    fn network_style_import_graph_deduplicates_fragments_before_fetch() {
        let first = Url::parse("https://example.test/shared.css#first").unwrap();
        let duplicate = Url::parse("https://example.test/shared.css#second").unwrap();
        let second = Url::parse("https://example.test/second.css").unwrap();
        let mut graph = ConnectedNetworkStyleImportGraph::default();

        graph.extend([first.clone(), duplicate, second.clone()]);
        assert_eq!(graph.take_pending(), vec![first, second]);
        assert!(graph.take_pending().is_empty());
    }

    #[test]
    fn network_style_import_graph_leaves_admission_to_resource_scheduler() {
        let urls = (0..1_100)
            .map(|index| Url::parse(&format!("https://example.test/import-{index}.css")).unwrap());
        let mut graph = ConnectedNetworkStyleImportGraph::default();

        graph.extend(urls);

        assert_eq!(graph.take_pending().len(), 1_100);
        assert!(graph.take_pending().is_empty());
    }
}
