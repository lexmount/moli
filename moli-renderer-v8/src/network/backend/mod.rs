//! Browser-context scoped resource infrastructure.
//!
//! This module owns the state that is intentionally shared across navigation
//! and execution-context lifetimes. Page policy and Document/Worker lifecycle
//! state must remain outside this boundary.

mod diagnostics;
mod memory_cache;
mod runtime;

pub use diagnostics::{
    BrowserParkableImageDiagnostics, BrowserResourceDiskPoolDiagnostics,
    BrowserResourceRuntimeDiagnostics,
};
pub use memory_cache::SharedMemoryResourceCacheDiagnostics;
pub use runtime::{
    BrowserResourceRuntime, BrowserResourceRuntimeOwner, BrowserResourceRuntimeOwnerRegistrar,
    BrowserResourceRuntimeOwnerRegistration,
};
pub(crate) use runtime::{BrowserResourceRuntimeBinding, BrowserResourceRuntimeOwnerRoot};

pub(in crate::network) use memory_cache::{
    RawSubresourceCacheKey, ScriptTextCacheLookup, raw_subresource_memory_cache_expiry,
    raw_subresource_memory_cache_key, script_text_cache_key,
    script_text_request_is_memory_cacheable,
};
