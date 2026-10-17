//! Directory-backed HTTP cache entries with streaming body writes.
//!
//! The cache format mirrors the important shape of Chromium's HTTP cache:
//! response metadata and response body are stored separately, and callers can
//! append body chunks as the network transfer progresses instead of retaining
//! a complete response body in memory.
//!
//! Cache writes are best-effort and deliberately do not request filesystem
//! durability. Metadata is renamed last so in-progress bodies stay unpublished
//! during normal operation, but an unexpected machine shutdown may leave
//! damaged cache files. HTTP cache contents are disposable and must not impose
//! `fsync` latency on network response completion.

mod eviction;
mod metadata;
mod path_safety;
mod policy;
mod revalidation;
mod store;
mod time;
mod vary;
mod writer;

pub use self::{
    metadata::{HttpCacheEntryMetadata, HttpCacheVaryHeader},
    policy::{
        HttpCacheResponsePolicy, cache_expires_at_unix_ms, cacheable_response_parts_policy,
        cached_response_is_fresh_immutable, cached_response_is_stale,
        request_cache_control_forbids_storage, request_cache_control_requires_validation,
        request_header_requires_validation, request_pragma_requires_validation,
        response_cache_policy, response_date, unix_now_ms,
    },
    revalidation::{merge_not_modified_headers, validation_headers_from_headers},
    store::{HttpCacheEntryInfo, HttpCacheStats, HttpCacheStore, HttpCachedEntryReader},
    vary::response_vary_header_names,
    writer::HttpCacheBodyWriter,
};

#[cfg(test)]
pub use self::store::HttpCachedEntry;

#[cfg(test)]
mod tests;
