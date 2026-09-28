//! Fetching and readiness classification for stylesheet import graphs.
//!
//! This module has no owner lifecycle. It produces one immutable graph result;
//! linked and inline stylesheet code decide separately where that result may be
//! installed.

use super::*;
use crate::live_stylesheet::import_url_identity;
use crate::stylesheet_blocking::{StylesheetFetchOptions, StylesheetFetcher};
use futures_util::future::join_all;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingStylesheetImport {
    url: Url,
    options: StylesheetFetchOptions,
}

#[derive(Default)]
struct ConnectedNetworkStyleImportGraph {
    pending: VecDeque<PendingStylesheetImport>,
    admitted: HashSet<Url>,
}

impl ConnectedNetworkStyleImportGraph {
    fn extend(&mut self, urls: impl IntoIterator<Item = Url>, options: &StylesheetFetchOptions) {
        for url in urls {
            let identity = import_url_identity(&url);
            if !self.admitted.insert(identity) {
                continue;
            }
            self.pending.push_back(PendingStylesheetImport {
                url,
                options: options.clone(),
            });
        }
    }

    fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn take_pending(&mut self) -> Vec<PendingStylesheetImport> {
        self.pending.drain(..).collect()
    }
}

pub(crate) fn stylesheet_import_options(
    terminal: &crate::stylesheet_blocking::StylesheetFetchTerminal,
    document_options: &StylesheetFetchOptions,
) -> Option<StylesheetFetchOptions> {
    let response = terminal.ready_response()?;
    let inherits_document_policy = matches!(response.final_url.scheme(), "data" | "blob")
        || (terminal.from_service_worker() && terminal.service_worker_response_url().is_none());
    let policy =
        moli_fetch::response_referrer_policy_from_headers(&response.headers).or_else(|| {
            inherits_document_policy.then(|| {
                moli_fetch::effective_referrer_policy(
                    None,
                    document_options.document_referrer_policy(),
                )
                .to_owned()
            })
        });
    Some(
        StylesheetFetchOptions::for_import(response.final_url.clone(), policy)
            .with_document_referrer_policy(
                document_options
                    .document_referrer_policy()
                    .map(str::to_owned),
            ),
    )
}

pub(crate) async fn fetch_complete_stylesheet_import_graph(
    stylesheet_fetcher: crate::stylesheet_blocking::RendererStylesheetFetcher,
    document_url: Url,
    urls: Vec<Url>,
    options: StylesheetFetchOptions,
) -> crate::stylesheet_blocking::StylesheetImportGraphFetchResult {
    let urls = match connected_style_import_readiness(urls) {
        ConnectedStyleImportReadiness::Ready(successful) => {
            return crate::stylesheet_blocking::StylesheetImportGraphFetchResult::new(
                successful,
                Vec::new(),
            );
        }
        ConnectedStyleImportReadiness::Pending(urls) => urls,
    };
    let document_options =
        stylesheet_fetcher.prepare_stylesheet_fetch_options(StylesheetFetchOptions::default());
    let mut successful = true;
    let mut network_results = Vec::new();
    let mut import_graph = ConnectedNetworkStyleImportGraph::default();
    if let Some(parent) = options.referrer_url() {
        import_graph.admitted.insert(import_url_identity(parent));
    }
    import_graph.extend(urls, &options);
    while !import_graph.is_empty() {
        let mut network_requests = Vec::new();
        for request in import_graph.take_pending() {
            if request.url.scheme() != "data" {
                network_requests.push(request);
                continue;
            }
            match data_stylesheet_import_readiness(&request.url) {
                DataStylesheetImportReadiness::NoImports => {}
                DataStylesheetImportReadiness::Failed => successful = false,
                DataStylesheetImportReadiness::Imports(urls) => {
                    let options = StylesheetFetchOptions::for_import(
                        request.url,
                        document_options
                            .document_referrer_policy()
                            .map(str::to_owned),
                    );
                    import_graph.extend(urls, &options);
                }
            }
        }
        let pending_fetches = join_all(network_requests.into_iter().map(|request| {
            let stylesheet_fetcher = stylesheet_fetcher.clone();
            let fetch_document_url = document_url.clone();
            let start_unix_millis = moli_time::unix_epoch_millis();
            async move {
                let terminal = stylesheet_fetcher
                    .fetch_stylesheet_resource(
                        fetch_document_url,
                        request.url.clone(),
                        request.options,
                    )
                    .await;
                (request.url, start_unix_millis, terminal)
            }
        }))
        .await;
        for (url, start_unix_millis, terminal) in pending_fetches {
            successful &= terminal.is_ready();
            let import_options = stylesheet_import_options(&terminal, &document_options);
            if let Some(response) = terminal.ready_response() {
                let nested_urls = crate::style_engine::stylesheet_top_level_import_urls(
                    response.body_text(),
                    &response.final_url,
                    false,
                )
                .unwrap_or_default();
                match connected_style_import_readiness(nested_urls) {
                    ConnectedStyleImportReadiness::Ready(nested_successful) => {
                        successful &= nested_successful;
                    }
                    ConnectedStyleImportReadiness::Pending(nested_urls) => {
                        import_graph.extend(
                            nested_urls,
                            import_options
                                .as_ref()
                                .expect("ready stylesheet retains import referrer state"),
                        );
                    }
                }
            }
            network_results.push(
                crate::stylesheet_blocking::StylesheetImportNetworkResult::new(
                    url,
                    start_unix_millis,
                    terminal,
                )
                .with_import_options(import_options),
            );
        }
    }
    crate::stylesheet_blocking::StylesheetImportGraphFetchResult::new(successful, network_results)
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
    mut urls: Vec<Url>,
) -> ConnectedStyleImportReadiness {
    let mut roots = HashSet::new();
    urls.retain(|url| roots.insert(import_url_identity(url)));
    let mut has_network_imports = false;
    let mut stack = urls.iter().cloned().collect::<VecDeque<_>>();
    let mut seen = HashSet::new();
    let mut data_expansions = 0;
    while let Some(url) = stack.pop_front() {
        if !seen.insert(import_url_identity(&url)) {
            continue;
        }
        if url.scheme() != "data" {
            has_network_imports = true;
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
    if has_network_imports {
        // Preserve local parents so fetching can retain each sheet's referrer
        // state instead of treating its descendants as direct imports.
        ConnectedStyleImportReadiness::Pending(urls)
    } else {
        ConnectedStyleImportReadiness::Ready(true)
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
    fn connected_style_import_readiness_preserves_local_parent_sheets() {
        let local =
            Url::parse("data:text/css,@import 'https://example.test/imported.css';").unwrap();
        let direct = Url::parse("https://example.test/direct.css").unwrap();
        let ConnectedStyleImportReadiness::Pending(pending) =
            connected_style_import_readiness(vec![local.clone(), direct.clone()])
        else {
            panic!("local sheets with external imports must remain pending");
        };

        assert_eq!(pending, vec![local, direct]);
    }

    #[test]
    fn network_style_import_graph_deduplicates_fragments_before_fetch() {
        let first = Url::parse("https://example.test/shared.css#first").unwrap();
        let duplicate = Url::parse("https://example.test/shared.css#second").unwrap();
        let second = Url::parse("https://example.test/second.css").unwrap();
        let mut graph = ConnectedNetworkStyleImportGraph::default();

        graph.extend(
            [first.clone(), duplicate, second.clone()],
            &StylesheetFetchOptions::default(),
        );
        assert_eq!(
            graph
                .take_pending()
                .into_iter()
                .map(|request| request.url)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        assert!(graph.is_empty());
    }

    #[test]
    fn network_style_import_graph_leaves_admission_to_resource_scheduler() {
        let urls = (0..1_100)
            .map(|index| Url::parse(&format!("https://example.test/import-{index}.css")).unwrap());
        let mut graph = ConnectedNetworkStyleImportGraph::default();

        graph.extend(urls, &StylesheetFetchOptions::default());

        assert_eq!(graph.take_pending().len(), 1_100);
        assert!(graph.is_empty());
    }
}
