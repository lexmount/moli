use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use moli_fetch::{BrowserRequestMetadata, Request, RequestCredentialsMode, RequestMode};
use parking_lot::Mutex;
use tokio::sync::Notify;
use url::Url;

use crate::{
    protocol_types::NavigationResponse,
    subresource_integrity::integrity_metadata_allows_preload_consumption,
    types::AsyncSubresourceFetchResponseFilter,
};

#[derive(Clone, Debug)]
pub(crate) struct DocumentPreloadResponse {
    pub(crate) response: NavigationResponse,
    pub(crate) from_service_worker: bool,
    pub(crate) response_filter: Option<AsyncSubresourceFetchResponseFilter>,
}

#[derive(Debug)]
pub(crate) struct ConsumedPreloadError(pub(crate) String);

impl std::fmt::Display for ConsumedPreloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConsumedPreloadError {}

type PreloadResult = Result<DocumentPreloadResponse, String>;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct PreloadKey {
    url: Url,
    destination: &'static str,
    mode: RequestMode,
    credentials: RequestCredentialsMode,
}

impl PreloadKey {
    fn from_request(request: &Request) -> Option<Self> {
        if request.method != "GET"
            || request.body.is_some()
            || !matches!(
                request.request_mode,
                RequestMode::NoCors | RequestMode::Cors | RequestMode::SameOrigin
            )
            || !matches!(request.url.scheme(), "http" | "https")
        {
            return None;
        }
        let destination = match request.browser_request_metadata()? {
            BrowserRequestMetadata::Script => "script",
            BrowserRequestMetadata::Style | BrowserRequestMetadata::StyleModule => "style",
            BrowserRequestMetadata::Image => "image",
            BrowserRequestMetadata::Font => "font",
            BrowserRequestMetadata::TextTrack => "track",
            BrowserRequestMetadata::Fetch | BrowserRequestMetadata::Xhr => "",
            _ => return None,
        };
        Some(Self {
            url: request.url.clone(),
            destination,
            mode: request.request_mode,
            credentials: request.credentials_mode,
        })
    }
}

struct PreloadEntry {
    integrity: String,
    result: Mutex<Option<PreloadResult>>,
    available: Notify,
}

impl PreloadEntry {
    fn complete(&self, result: PreloadResult) {
        let mut current = self.result.lock();
        if current.is_some() {
            return;
        }
        *current = Some(result);
        drop(current);
        self.available.notify_waiters();
    }
}

#[derive(Default)]
struct PreloadMap {
    retired: bool,
    entries: HashMap<PreloadKey, Arc<PreloadEntry>>,
    // Includes consumed entries, so retiring the Document also wakes consumers
    // waiting for a response that has not yet reached the loader.
    pending: Vec<Weak<PreloadEntry>>,
}

#[derive(Clone, Default)]
pub(crate) struct DocumentPreloads(Arc<Mutex<PreloadMap>>);

impl DocumentPreloads {
    pub(crate) fn register(&self, request: &Request) -> Option<DocumentPreloadProducer> {
        let key = PreloadKey::from_request(request)?;
        let mut map = self.0.lock();
        if map.retired {
            return None;
        }
        let entry = Arc::new(PreloadEntry {
            integrity: request
                .subresource_request_metadata()
                .and_then(|metadata| metadata.integrity.as_deref())
                .unwrap_or_default()
                .to_owned(),
            result: Mutex::new(None),
            available: Notify::new(),
        });
        map.pending.retain(|entry| entry.strong_count() != 0);
        map.pending.push(Arc::downgrade(&entry));
        map.entries.insert(key, entry.clone());
        Some(DocumentPreloadProducer { entry })
    }

    pub(crate) fn consume(&self, request: &Request) -> Option<DocumentPreloadConsumer> {
        if request.priority_hints.link_preload {
            return None;
        }
        // Fetch/XHR author headers make the request ineligible for preload
        // consumption. DOM-initiated requests do not set the unsafe-request flag.
        if matches!(
            request.browser_request_metadata(),
            Some(BrowserRequestMetadata::Fetch | BrowserRequestMetadata::Xhr)
        ) && !request.request_headers.is_empty()
        {
            return None;
        }
        let key = PreloadKey::from_request(request)?;
        let mut map = self.0.lock();
        if map.retired {
            return None;
        }
        let entry = map.entries.get(&key)?;
        if !integrity_metadata_allows_preload_consumption(
            &entry.integrity,
            request
                .subresource_request_metadata()
                .and_then(|metadata| metadata.integrity.as_deref()),
        ) {
            return None;
        }
        Some(DocumentPreloadConsumer {
            entry: map.entries.remove(&key)?,
        })
    }

    pub(crate) fn retire(&self) {
        let pending = {
            let mut map = self.0.lock();
            map.retired = true;
            map.entries.clear();
            std::mem::take(&mut map.pending)
        };
        for entry in pending.into_iter().filter_map(|entry| entry.upgrade()) {
            entry.complete(Err("preload Document retired".to_owned()));
        }
    }
}

pub(crate) struct DocumentPreloadProducer {
    entry: Arc<PreloadEntry>,
}

impl DocumentPreloadProducer {
    pub(crate) fn complete(self, result: PreloadResult) {
        self.entry.complete(result);
    }
}

impl Drop for DocumentPreloadProducer {
    fn drop(&mut self) {
        self.entry
            .complete(Err("preload response was cancelled".to_owned()));
    }
}

pub(crate) struct DocumentPreloadConsumer {
    entry: Arc<PreloadEntry>,
}

impl DocumentPreloadConsumer {
    pub(crate) fn try_response(&self) -> Option<PreloadResult> {
        self.entry.result.lock().clone().map(|result| {
            result.map(|mut preload| {
                preload.response.preload_state = moli_fetch::ResponsePreloadState::Consumed {
                    filter: preload.response_filter.clone(),
                    from_service_worker: preload.from_service_worker,
                };
                preload
            })
        })
    }

    pub(crate) async fn into_completion(
        self,
        internal_id: u64,
        request: Request,
    ) -> crate::types::AsyncSubresourceFetchCompletion {
        use crate::types::{AsyncSubresourceFetchCompletion, AsyncSubresourceFetchResult};
        let (result, response_filter, from_service_worker) = match self.response().await {
            Ok(preload) => (
                AsyncSubresourceFetchResult::Response(preload.response),
                preload.response_filter,
                preload.from_service_worker,
            ),
            Err(error) => (
                AsyncSubresourceFetchResult::PreloadFailure(error),
                None,
                false,
            ),
        };
        AsyncSubresourceFetchCompletion {
            internal_id,
            request_url: request.url,
            request_method: request.method,
            request_headers: request.request_headers,
            request_body: request
                .body
                .as_ref()
                .map(|body| String::from_utf8_lossy(body).into_owned()),
            response_status_text: None,
            skip_fetch_security_validation: from_service_worker,
            response_filter,
            network_error_text: None,
            result,
        }
    }

    pub(crate) async fn response(self) -> PreloadResult {
        loop {
            let notified = self.entry.available.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(result) = self.try_response() {
                return result;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_fetch::{ResponseCacheState, SubresourceRequestMetadata};

    fn request() -> Request {
        Request::new("GET", "https://example.test/asset", None, vec![])
            .unwrap()
            .with_browser_request_metadata(BrowserRequestMetadata::Script)
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(RequestCredentialsMode::Include)
    }

    fn response() -> DocumentPreloadResponse {
        DocumentPreloadResponse {
            response: NavigationResponse::from_text_body(
                request().url,
                200,
                vec![],
                "body".to_owned(),
            ),
            response_filter: Some(AsyncSubresourceFetchResponseFilter::Opaque),
            from_service_worker: true,
        }
    }

    #[tokio::test]
    async fn pending_preloads_are_consumed_once_and_retain_response_provenance() {
        let preloads = DocumentPreloads::default();
        let request = request();
        let producer = preloads.register(&request).unwrap();
        let consumer = preloads.consume(&request).unwrap();
        assert!(preloads.consume(&request).is_none());
        let pending = tokio::spawn(consumer.response());
        tokio::task::yield_now().await;
        assert!(!pending.is_finished());
        producer.complete(Ok(response()));
        let actual = pending.await.unwrap().unwrap();
        assert_eq!(actual.response.body_bytes(), b"body");
        assert_eq!(actual.response.cache_state, ResponseCacheState::None);
        assert!(actual.response.preload_state.is_consumed());
        assert!(actual.response.preload_state.from_service_worker());
        assert_eq!(
            actual.response.preload_state.response_filter(),
            Some(&AsyncSubresourceFetchResponseFilter::Opaque)
        );
    }

    #[tokio::test]
    async fn integrity_failure_is_reused_and_incompatible_metadata_does_not_remove_it() {
        let preloads = DocumentPreloads::default();
        let producer_request =
            request().with_subresource_request_metadata(SubresourceRequestMetadata {
                integrity: Some("sha384-AAAA".to_owned()),
                ..Default::default()
            });
        let producer = preloads.register(&producer_request).unwrap();
        producer.complete(Err("integrity mismatch".to_owned()));
        let other = request().with_subresource_request_metadata(SubresourceRequestMetadata {
            integrity: Some("sha384-BBBB".to_owned()),
            ..Default::default()
        });
        assert!(preloads.consume(&other).is_none());
        assert_eq!(
            preloads
                .consume(&request())
                .unwrap()
                .response()
                .await
                .unwrap_err(),
            "integrity mismatch"
        );
        assert!(preloads.consume(&request()).is_none());
    }

    #[tokio::test]
    async fn incompatible_fetch_keys_and_author_headers_leave_preload_available() {
        let preloads = DocumentPreloads::default();
        let producer = preloads.register(&request()).unwrap();
        let mut post = request();
        post.method = "POST".to_owned();
        for other in [
            post,
            request().with_request_mode(RequestMode::Navigate),
            request().with_request_mode(RequestMode::Cors),
            request().with_credentials_mode(RequestCredentialsMode::Omit),
            request().with_browser_request_metadata(BrowserRequestMetadata::Image),
        ] {
            assert!(preloads.consume(&other).is_none());
        }
        let mut producer_request = request();
        producer_request.priority_hints.link_preload = true;
        assert!(preloads.consume(&producer_request).is_none());
        drop(producer);
        assert!(
            preloads
                .consume(&request())
                .unwrap()
                .response()
                .await
                .is_err()
        );

        let fetch = request().with_browser_request_metadata(BrowserRequestMetadata::Fetch);
        let _producer = preloads.register(&fetch).unwrap();
        let mut with_header = fetch.clone();
        with_header.request_headers = moli_fetch::RequestHeaders::from_byte_strings(&[(
            "X-Test".to_owned(),
            "yes".to_owned(),
        )])
        .unwrap();
        assert!(preloads.consume(&with_header).is_none());
        assert!(preloads.consume(&fetch).is_some());
    }

    #[tokio::test]
    async fn retirement_wakes_reserved_consumers_and_blocks_late_publication() {
        let preloads = DocumentPreloads::default();
        let producer = preloads.register(&request()).unwrap();
        let pending = tokio::spawn(preloads.consume(&request()).unwrap().response());
        preloads.retire();
        producer.complete(Ok(response()));
        assert_eq!(
            pending.await.unwrap().unwrap_err(),
            "preload Document retired"
        );
        assert!(preloads.register(&request()).is_none());
        assert!(preloads.consume(&request()).is_none());
    }
}
