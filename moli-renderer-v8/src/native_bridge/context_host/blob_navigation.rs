use super::{JsContextHost, OwnerDispatchScope};
use crate::network_host::{BlobUrlFetchEnvironment, CapturedBlobResource};
use url::Url;

impl JsContextHost {
    /// Child navigations use the initiating Document's complete storage key.
    /// Keep a denied resolution so a later load cannot retry it without authority.
    pub(in crate::native_bridge::context_host) fn capture_child_blob_resource(
        &mut self,
        url: &Url,
        initiator: Option<OwnerDispatchScope>,
    ) -> Option<CapturedBlobResource> {
        let resource = CapturedBlobResource::capture(url)?;
        let environment = initiator
            .and_then(|source| BlobUrlFetchEnvironment::for_child_navigation(self, source));
        Some(resource.for_fetch_environment(environment.as_ref()))
    }

    /// Chromium's popup rule compares sites rather than complete storage keys.
    /// Top-level Blob navigation remains allowed across storage keys.
    pub(crate) fn blob_navigation_requires_noopener(
        &mut self,
        source: OwnerDispatchScope,
        url: &Url,
    ) -> bool {
        url.scheme() == "blob"
            && self
                .storage_context_for_dispatch_scope(source)
                .is_none_or(|context| {
                    moli_storage_key::site_for_url(url) != context.storage_key().top_level_site()
                })
    }
}
