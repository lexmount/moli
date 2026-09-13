//! Share only pending, compatible image transfers within one Document.
//!
//! The registry holds weak futures, not responses or transport ownership. Each
//! consumer polls a clone with its own cancellation handle; dropping the last
//! clone cancels the transfer. Completed bodies remain the bounded memory
//! cache's responsibility, and failed loads are never retained for reuse.

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, anyhow};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared, WeakShared},
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, NetworkFetchResult, RawResponse, Request,
    RequestCredentialsMode, RequestMode, RequestRedirectMode, RequestResourceType,
    cookie_header_for_request,
};
use parking_lot::Mutex;

use super::{RawSubresourceCacheKey, ResourceRequestClient, raw_subresource_memory_cache_key};

const MAX_PENDING_IMAGES: usize = 256;
type ImageFetch = BoxFuture<'static, Result<NetworkFetchResult<RawResponse>, SharedImageError>>;

#[derive(Default)]
pub(super) struct PendingImageLoads {
    entries: Mutex<HashMap<ImageFetchKey, PendingImageEntry>>,
}

struct PendingImageEntry {
    future: WeakShared<ImageFetch>,
    allow_late_join: Arc<AtomicBool>,
}

#[derive(Eq, Hash, PartialEq)]
struct ImageFetchKey {
    resource: RawSubresourceCacheKey,
    policy_revision: u64,
    mode: RequestMode,
    infer_referrer: bool,
    referrer_policy: Option<String>,
    document_referrer_policy: Option<String>,
    integrity: Option<String>,
    cookie_header: Option<String>,
}

impl ImageFetchKey {
    fn new(client: &ResourceRequestClient, request: &Request) -> Option<Self> {
        if request.resource_type != RequestResourceType::Image
            || request.browser_request_metadata() != Some(BrowserRequestMetadata::Image)
            || !matches!(request.url.scheme(), "http" | "https")
            || !request.cache_mode().allows_memory_cache_lookup()
            || request.redirect_mode != RequestRedirectMode::Follow
        {
            return None;
        }
        let resource = raw_subresource_memory_cache_key(request)?;
        let metadata = request.subresource_request_metadata();
        let cookie_header = if request.credentials_mode == RequestCredentialsMode::Omit {
            None
        } else {
            // A cookie change during a pending load must not reuse the old
            // credentialed request. Failure to query cookies disables sharing.
            cookie_header_for_request(
                &client.resource_runtime.cookie_store(),
                &request.url,
                request.cookie_context.clone(),
            )
            .ok()?
        };
        Some(Self {
            resource,
            policy_revision: client.page_network_policy.revision(),
            mode: request.request_mode,
            infer_referrer: request.infers_referrer_from_initiator(),
            referrer_policy: metadata.and_then(|m| m.referrer_policy.clone()),
            document_referrer_policy: metadata.and_then(|m| m.document_referrer_policy.clone()),
            integrity: metadata.and_then(|m| m.integrity.clone()),
            cookie_header,
        })
    }
}

impl PendingImageLoads {
    fn join_or_start(
        &self,
        key: ImageFetchKey,
        fetch: impl FnOnce(Arc<AtomicBool>) -> ImageFetch,
    ) -> Shared<ImageFetch> {
        let mut entries = self.entries.lock();
        entries.retain(|_, entry| {
            entry.allow_late_join.load(Ordering::Acquire)
                && entry
                    .future
                    .upgrade()
                    .is_some_and(|load| load.peek().is_none())
        });
        if let Some(load) = entries.get(&key).and_then(|entry| entry.future.upgrade()) {
            return load;
        }
        let allow_late_join = Arc::new(AtomicBool::new(true));
        let load = fetch(allow_late_join.clone()).shared();
        if entries.len() < MAX_PENDING_IMAGES {
            entries.insert(
                key,
                PendingImageEntry {
                    future: load.downgrade().expect("new image future is pending"),
                    allow_late_join,
                },
            );
        }
        load
    }
}

/// Preserve the original error display and cause chain for every consumer.
#[derive(Clone, Debug)]
struct SharedImageError(Arc<anyhow::Error>);

impl fmt::Display for SharedImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Error for SharedImageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

struct ImageTransport(FetchCancelHandle);

impl Drop for ImageTransport {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl ResourceRequestClient {
    pub(crate) async fn fetch_image_with_cancel_and_network_metadata(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NetworkFetchResult<RawResponse>> {
        let request = self.apply_network_policy(request)?;
        self.fetch_image_after_policy(request, cancel_handle).await
    }

    pub(super) async fn fetch_image_after_policy(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
    ) -> Result<NetworkFetchResult<RawResponse>> {
        if cancel_handle.is_cancelled() {
            return Err(anyhow!("image request cancelled"));
        }
        let Some(key) = ImageFetchKey::new(self, &request) else {
            return self.materialize_image(request, cancel_handle, None).await;
        };
        let load = self.pending_images.join_or_start(key, |allow_late_join| {
            let client = self.clone();
            async move {
                let transport = ImageTransport(FetchCancelHandle::new());
                client
                    .materialize_image(request, transport.0.clone(), Some(&allow_late_join))
                    .await
                    .map_err(|error| SharedImageError(Arc::new(error)))
            }
            .boxed()
        });
        tokio::select! {
            biased;
            result = load => result.map_err(anyhow::Error::new),
            _ = cancel_handle.cancelled() => Err(anyhow!("image request cancelled")),
        }
    }

    async fn materialize_image(
        &self,
        request: Request,
        cancel_handle: FetchCancelHandle,
        allow_late_join: Option<&AtomicBool>,
    ) -> Result<NetworkFetchResult<RawResponse>> {
        if request.auth_requires_buffered_transport() || !request.follow_redirects {
            return self
                .resource_runtime
                .client()
                .fetch_with_cancel_and_network_metadata(request, cancel_handle)
                .await
                .map(|observed| {
                    observed.map_response(moli_fetch::Response::into_materialized_raw_response)
                });
        }
        let observed = self
            .fetch_raw_stream_with_cancel_after_policy_and_network_metadata(request, cancel_handle)
            .await?;
        let (response, journal) = observed.into_parts_with_observation_journal();
        // Once headers arrive, do not admit later consumers to a no-store
        // response. Use the existing conservative policy (also excluding
        // private responses), without removing consumers already waiting.
        if let Some(allow_late_join) = allow_late_join
            && !moli_http_cache::response_cache_policy(&response.headers).store
        {
            allow_late_join.store(false, Ordering::Release);
        }
        Ok(NetworkFetchResult::with_observation_journal(
            response.into_materialized_raw_response().await?,
            journal,
        ))
    }
}

#[cfg(test)]
mod tests;
