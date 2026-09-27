//! Fetch results owned by one Document's ordinary preload map.
//!
//! The link event and its eventual consumer observe the same terminal result.
//! This is separate from the browser's byte cache: a rejected preload remains
//! consumable as a failure, without changing the cached transport response.

use std::{collections::HashMap, fmt, sync::Arc};

use moli_fetch::{BrowserRequestMetadata, Request, RequestCredentialsMode, RequestMode};
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::{
    protocol_types::NavigationResponse,
    types::{AsyncSubresourceFetchResponseFilter, SubresourceResourceType},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PreloadDestination {
    Script,
    Image,
    Font,
    Track,
    Fetch,
}

impl PreloadDestination {
    pub(crate) fn for_resource_type(resource_type: SubresourceResourceType) -> Option<Self> {
        match resource_type {
            SubresourceResourceType::Script => Some(Self::Script),
            SubresourceResourceType::Image => Some(Self::Image),
            SubresourceResourceType::Font => Some(Self::Font),
            SubresourceResourceType::TextTrack => Some(Self::Track),
            SubresourceResourceType::Fetch => Some(Self::Fetch),
            _ => None,
        }
    }

    pub(crate) fn browser_request_metadata(self) -> BrowserRequestMetadata {
        match self {
            Self::Script => BrowserRequestMetadata::Script,
            Self::Image => BrowserRequestMetadata::Image,
            Self::Font => BrowserRequestMetadata::Font,
            Self::Track => BrowserRequestMetadata::TextTrack,
            Self::Fetch => BrowserRequestMetadata::Fetch,
        }
    }

    fn for_request(request: &Request) -> Option<Self> {
        match request.browser_request_metadata()? {
            BrowserRequestMetadata::Script => Some(Self::Script),
            BrowserRequestMetadata::Image => Some(Self::Image),
            BrowserRequestMetadata::Font => Some(Self::Font),
            BrowserRequestMetadata::TextTrack => Some(Self::Track),
            BrowserRequestMetadata::Fetch => Some(Self::Fetch),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PreloadResponseProvenance {
    Network,
    ServiceWorker {
        filter: Option<AsyncSubresourceFetchResponseFilter>,
        status_text: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum PreloadFailure {
    Cors(String),
    OpaqueCorsResponse,
    HttpStatus(u16),
    Integrity,
}

impl fmt::Display for PreloadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cors(message) => f.write_str(message),
            Self::OpaqueCorsResponse => f.write_str("CORS preload received an opaque response"),
            Self::HttpStatus(status) => write!(f, "preload returned HTTP {status}"),
            Self::Integrity => f.write_str("preload failed its integrity check"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreloadResponse {
    pub(crate) response: NavigationResponse,
    pub(crate) filter: Option<AsyncSubresourceFetchResponseFilter>,
    pub(crate) provenance: PreloadResponseProvenance,
}

#[derive(Debug)]
pub(crate) enum PreloadFetchTerminal {
    Ready(PreloadResponse),
    Rejected {
        response: PreloadResponse,
        reason: PreloadFailure,
    },
    NetworkError(String),
}

impl PreloadFetchTerminal {
    pub(crate) fn successful(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    pub(crate) fn response(&self) -> Option<&PreloadResponse> {
        match self {
            Self::Ready(response) | Self::Rejected { response, .. } => Some(response),
            Self::NetworkError(_) => None,
        }
    }

    pub(crate) fn consumer_response(&self) -> Result<&PreloadResponse, String> {
        match self {
            Self::Ready(response) => Ok(response),
            Self::Rejected { response, .. }
                if response.filter.is_some_and(|filter| !filter.is_readable()) =>
            {
                // Fetch may expose this message to script. The diagnostic
                // reason can contain an opaque response's internal status.
                Err("preload response is not readable".to_owned())
            }
            Self::Rejected { reason, .. } => Err(reason.to_string()),
            Self::NetworkError(message) => Err(message.clone()),
        }
    }

    pub(crate) fn observation(&self) -> Result<NavigationResponse, String> {
        match self {
            // Fetch exposes CORS rejection as a network error. Keep its raw
            // response in the terminal for diagnostics, but do not expose its
            // status or body size through Resource Timing/network results.
            Self::Rejected {
                reason: reason @ (PreloadFailure::Cors(_) | PreloadFailure::OpaqueCorsResponse),
                ..
            } => Err(reason.to_string()),
            Self::Ready(response) | Self::Rejected { response, .. } => {
                Ok(response.response.clone())
            }
            Self::NetworkError(message) => Err(message.clone()),
        }
    }
}

pub(crate) fn finalize_preload_response(
    request: &Request,
    response: NavigationResponse,
    provenance: PreloadResponseProvenance,
) -> PreloadFetchTerminal {
    let filter = match &provenance {
        PreloadResponseProvenance::Network => request.request_origin().and_then(|origin| {
            crate::network_host::network_response_filter(
                origin,
                &response.head(),
                request.request_mode,
            )
        }),
        PreloadResponseProvenance::ServiceWorker { filter, .. } => *filter,
    };
    let readable = filter.is_none_or(|filter| filter.is_readable());
    let usability =
        if request.priority_hints.link_preload && request.request_mode == RequestMode::Cors {
            match &provenance {
                PreloadResponseProvenance::Network => {
                    request.request_origin().map_or(Ok(()), |origin| {
                        crate::network_host::validate_cors_response_chain(
                            origin,
                            &response.head(),
                            request.credentials_mode,
                        )
                        .map_err(PreloadFailure::Cors)
                    })
                }
                PreloadResponseProvenance::ServiceWorker { .. } if !readable => {
                    Err(PreloadFailure::OpaqueCorsResponse)
                }
                PreloadResponseProvenance::ServiceWorker { .. } => Ok(()),
            }
        } else {
            Ok(())
        }
        .and_then(|()| {
            // Preserve opaque SW load-event behavior; its internal status is not
            // the status of the filtered response delivered by respondWith().
            if matches!(&provenance, PreloadResponseProvenance::ServiceWorker { .. }) && !readable
                || (200..=299).contains(&response.status)
            {
                Ok(())
            } else {
                Err(PreloadFailure::HttpStatus(response.status))
            }
        })
        .and_then(|()| {
            let integrity = request
                .priority_hints
                .link_preload
                .then(|| request_integrity(request))
                .flatten();
            if crate::subresource_integrity::response_matches_subresource_integrity_metadata(
                response.body_bytes(),
                integrity,
                readable,
            ) {
                Ok(())
            } else {
                Err(PreloadFailure::Integrity)
            }
        });
    let response = PreloadResponse {
        response,
        filter,
        provenance,
    };
    match usability {
        Ok(()) => PreloadFetchTerminal::Ready(response),
        Err(reason) => PreloadFetchTerminal::Rejected { response, reason },
    }
}

fn request_integrity(request: &Request) -> Option<&str> {
    request
        .subresource_request_metadata()
        .and_then(|metadata| metadata.integrity.as_deref())
}

fn parsed_integrity(request: &Request) -> Vec<(usize, String)> {
    let mut hashes = crate::subresource_integrity::integrity_metadata_hashes(
        request_integrity(request).unwrap_or_default(),
    )
    .map(|hash| (hash.algorithm.output_len_bytes(), hash.digest.to_owned()))
    .collect::<Vec<_>>();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct PreloadKey {
    url: url::Url,
    destination: PreloadDestination,
    mode: RequestMode,
    credentials: RequestCredentialsMode,
}

impl PreloadKey {
    fn for_request(request: &Request) -> Option<Self> {
        if request.method != "GET" || request.body.is_some() {
            return None;
        }
        Some(Self {
            url: request.url.clone(),
            destination: PreloadDestination::for_request(request)?,
            mode: request.request_mode,
            credentials: request.credentials_mode,
        })
    }
}

struct PreloadEntry {
    integrity: Vec<(usize, String)>,
    response: watch::Receiver<Option<Arc<PreloadFetchTerminal>>>,
}

#[derive(Default)]
struct PreloadStoreState {
    retired: bool,
    entries: HashMap<PreloadKey, PreloadEntry>,
}

#[derive(Clone, Default)]
pub(crate) struct DocumentPreloads {
    state: Arc<Mutex<PreloadStoreState>>,
}

impl DocumentPreloads {
    pub(crate) fn begin(&self, request: &Request) -> Option<PreloadCompletion> {
        if !request.priority_hints.link_preload {
            return None;
        }
        let key = PreloadKey::for_request(request)?;
        let mut state = self.state.lock();
        if state.retired {
            return None;
        }
        let (sender, response) = watch::channel(None);
        state.entries.insert(
            key,
            PreloadEntry {
                integrity: parsed_integrity(request),
                response,
            },
        );
        Some(PreloadCompletion {
            sender: Some(sender),
        })
    }

    pub(crate) fn consume(&self, request: &Request) -> Option<PreloadedResource> {
        if request.priority_hints.link_preload {
            return None;
        }
        let key = PreloadKey::for_request(request)?;
        let mut state = self.state.lock();
        let entry = state.entries.get(&key)?;
        let integrity = parsed_integrity(request);
        if !integrity.is_empty() && integrity != entry.integrity {
            return None;
        }
        let entry = state.entries.remove(&key)?;
        Some(PreloadedResource {
            response: entry.response,
        })
    }

    pub(crate) fn retire(&self) {
        let mut state = self.state.lock();
        state.retired = true;
        state.entries.clear();
    }
}

pub(crate) struct PreloadCompletion {
    sender: Option<watch::Sender<Option<Arc<PreloadFetchTerminal>>>>,
}

impl PreloadCompletion {
    pub(crate) fn finish(mut self, terminal: Arc<PreloadFetchTerminal>) {
        if let Some(sender) = self.sender.take() {
            sender.send_replace(Some(terminal));
        }
    }
}

impl Drop for PreloadCompletion {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            sender.send_replace(Some(Arc::new(PreloadFetchTerminal::NetworkError(
                "preload was canceled".to_owned(),
            ))));
        }
    }
}

pub(crate) struct PreloadedResource {
    response: watch::Receiver<Option<Arc<PreloadFetchTerminal>>>,
}

impl PreloadedResource {
    pub(crate) async fn wait(mut self) -> Arc<PreloadFetchTerminal> {
        loop {
            if let Some(response) = self.response.borrow().clone() {
                return response;
            }
            if self.response.changed().await.is_err() {
                return Arc::new(PreloadFetchTerminal::NetworkError(
                    "preload producer closed without a result".to_owned(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_fetch::SubresourceRequestMetadata;

    fn request(integrity: Option<&str>) -> Request {
        let url = url::Url::parse("https://example.test/resource").unwrap();
        Request::new("GET", url.as_str(), None, vec![])
            .unwrap()
            .with_request_origin(moli_url::WebOrigin::from_url(&url))
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(RequestCredentialsMode::Include)
            .with_browser_request_metadata(BrowserRequestMetadata::Script)
            .with_subresource_request_metadata(SubresourceRequestMetadata {
                integrity: integrity.map(str::to_owned),
                ..Default::default()
            })
    }

    fn response(url: &url::Url) -> NavigationResponse {
        NavigationResponse::from_text_body(url.clone(), 200, vec![], "body".to_owned())
    }

    #[tokio::test]
    async fn preload_consumers_inherit_pending_integrity_failure_once() {
        let preloads = DocumentPreloads::default();
        let preload_request = request(Some("sha256-AAAA")).with_link_preload();
        let completion = preloads.begin(&preload_request).unwrap();
        let consumer = preloads.consume(&request(None)).unwrap();
        assert!(preloads.consume(&request(None)).is_none());
        let terminal = Arc::new(finalize_preload_response(
            &preload_request,
            response(&preload_request.url),
            PreloadResponseProvenance::Network,
        ));
        assert!(matches!(
            terminal.as_ref(),
            PreloadFetchTerminal::Rejected {
                reason: PreloadFailure::Integrity,
                ..
            }
        ));
        assert_eq!(terminal.observation().unwrap().body_bytes(), b"body");
        completion.finish(terminal);
        assert!(consumer.wait().await.consumer_response().is_err());
    }

    #[test]
    fn preload_reuse_matches_destination_mode_credentials_and_parsed_integrity() {
        let store = DocumentPreloads::default();
        let original = request(Some("sha256-AAAA?ignored=yes sha384-BBBB")).with_link_preload();
        let _completion = store.begin(&original).unwrap();
        for different in [
            request(None).with_browser_request_metadata(BrowserRequestMetadata::Fetch),
            request(None).with_request_mode(RequestMode::Cors),
            request(None).with_credentials_mode(RequestCredentialsMode::SameOrigin),
            request(Some("sha256-CCCC")),
        ] {
            assert!(store.consume(&different).is_none());
        }
        assert!(
            store
                .consume(&request(Some("sha384-BBBB sha256-AAAA")))
                .is_some()
        );
        for destination in [
            PreloadDestination::Script,
            PreloadDestination::Image,
            PreloadDestination::Font,
            PreloadDestination::Track,
            PreloadDestination::Fetch,
        ] {
            let consumer =
                request(None).with_browser_request_metadata(destination.browser_request_metadata());
            let _completion = store.begin(&consumer.clone().with_link_preload()).unwrap();
            assert!(store.consume(&consumer).is_some());
        }
    }

    #[tokio::test]
    async fn preload_producer_cancellation_and_document_retirement_do_not_hang_consumers() {
        let first_document = DocumentPreloads::default();
        let next_document = DocumentPreloads::default();
        let preload_request = request(None).with_link_preload();
        let completion = first_document.begin(&preload_request).unwrap();
        assert!(next_document.consume(&request(None)).is_none());
        let consumer = first_document.consume(&request(None)).unwrap();
        drop(completion);
        assert!(matches!(
            consumer.wait().await.as_ref(),
            PreloadFetchTerminal::NetworkError(_)
        ));
        let _completion = first_document.begin(&preload_request).unwrap();
        first_document.retire();
        assert!(first_document.consume(&request(None)).is_none());
        assert!(first_document.begin(&preload_request).is_none());
    }

    #[tokio::test]
    async fn old_preload_completion_cannot_replace_a_new_entry_for_the_same_url() {
        let store = DocumentPreloads::default();
        let request = request(None);
        let older = store.begin(&request.clone().with_link_preload()).unwrap();
        let newer = store.begin(&request.clone().with_link_preload()).unwrap();
        older.finish(Arc::new(PreloadFetchTerminal::NetworkError(
            "old failure".to_owned(),
        )));
        newer.finish(Arc::new(finalize_preload_response(
            &request,
            response(&request.url),
            PreloadResponseProvenance::Network,
        )));
        assert!(store.consume(&request).unwrap().wait().await.successful());
    }

    #[test]
    fn opaque_preload_failure_keeps_its_status_out_of_consumer_errors() {
        let request = request(None).with_link_preload();
        let mut response = response(&url::Url::parse("https://other.test/resource").unwrap());
        response.status = 403;
        let terminal =
            finalize_preload_response(&request, response, PreloadResponseProvenance::Network);
        assert!(matches!(
            &terminal,
            PreloadFetchTerminal::Rejected {
                reason: PreloadFailure::HttpStatus(403),
                ..
            }
        ));
        let error = terminal.consumer_response().unwrap_err();
        assert!(
            !error.contains("403"),
            "opaque status leaked through {error}"
        );
    }

    #[test]
    fn preload_finalization_preserves_response_provenance_and_failure_reasons() {
        let request = request(Some("sha256-AAAA"))
            .with_request_mode(RequestMode::Cors)
            .with_link_preload();
        let cross = url::Url::parse("https://other.test/resource").unwrap();
        let blocked = finalize_preload_response(
            &request,
            response(&cross),
            PreloadResponseProvenance::Network,
        );
        assert!(blocked.response().is_some());
        assert!(blocked.observation().is_err());
        assert!(matches!(
            blocked,
            PreloadFetchTerminal::Rejected {
                reason: PreloadFailure::Cors(_),
                ..
            }
        ));
        assert!(matches!(
            finalize_preload_response(
                &request,
                response(&cross),
                PreloadResponseProvenance::ServiceWorker {
                    filter: None,
                    status_text: None
                }
            ),
            PreloadFetchTerminal::Rejected {
                reason: PreloadFailure::Integrity,
                ..
            }
        ));
        assert!(matches!(
            finalize_preload_response(
                &request,
                response(&request.url),
                PreloadResponseProvenance::ServiceWorker {
                    filter: Some(AsyncSubresourceFetchResponseFilter::Opaque),
                    status_text: None
                }
            ),
            PreloadFetchTerminal::Rejected {
                reason: PreloadFailure::OpaqueCorsResponse,
                ..
            }
        ));
    }
}
