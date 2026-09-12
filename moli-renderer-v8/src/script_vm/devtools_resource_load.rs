use moli_fetch::{
    Request, RequestCacheMode, RequestCredentialsMode, RequestMode, RequestRedirectMode,
};
use url::Url;

use super::ScriptVm;
use crate::{
    native_bridge::OwnerDispatchScope,
    network::{RendererNetworkResourceLoadPreparation, RendererPreparedNetworkResourceLoad},
};

impl ScriptVm {
    pub(crate) fn prepare_devtools_network_resource_load(
        &self,
        frame_id: &str,
        url: Url,
        disable_cache: bool,
        include_credentials: bool,
    ) -> RendererNetworkResourceLoadPreparation {
        let host = self._context_host.borrow();
        let (owner, document_url, network_partition_key, resource_loader) =
            if self.root_frame_id() == Some(frame_id) {
                let Some(resource_loader) = host.current_main_document_resource_loader() else {
                    return RendererNetworkResourceLoadPreparation::FrameNotFound;
                };
                (
                    OwnerDispatchScope::Top,
                    self.document_runtime.document_url().clone(),
                    None,
                    resource_loader,
                )
            } else {
                let Some(handle) = host.child_browsing_context_handle_by_frame_id(frame_id) else {
                    return RendererNetworkResourceLoadPreparation::FrameNotFound;
                };
                let owner = OwnerDispatchScope::Child(handle);
                let Some(document_url) = host.child_browsing_context_current_url(handle) else {
                    return RendererNetworkResourceLoadPreparation::FrameNotFound;
                };
                let Some(resource_loader) = host.document_resource_loader_for_dispatch_scope(owner)
                else {
                    return RendererNetworkResourceLoadPreparation::FrameNotFound;
                };
                (
                    owner,
                    document_url,
                    host.child_browsing_context_network_partition_key(handle),
                    resource_loader,
                )
            };

        if !host.document_connect_csp_allows_for_owner(owner, &document_url, &url) {
            return RendererNetworkResourceLoadPreparation::CspViolation;
        }
        if moli_url_policy::route_fetch_url(&url).is_err() {
            return RendererNetworkResourceLoadPreparation::UnsupportedUrlScheme;
        }

        let cache_mode = if disable_cache {
            RequestCacheMode::Bypass
        } else {
            RequestCacheMode::Default
        };
        let credentials_mode = if include_credentials {
            RequestCredentialsMode::Include
        } else {
            RequestCredentialsMode::SameOrigin
        };
        let request = Request::new("GET", url.as_str(), None, Vec::new())
            .expect("a parsed DevTools resource URL should remain valid")
            .with_initiator_url(&document_url)
            .with_request_origin(moli_url::WebOrigin::from_url(&document_url))
            .without_inferred_referrer()
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(credentials_mode)
            .with_redirect_mode(RequestRedirectMode::Follow)
            .with_cache_mode(cache_mode)
            .with_network_partition_key(network_partition_key)
            .with_subframe_context(!matches!(owner, OwnerDispatchScope::Top));

        RendererNetworkResourceLoadPreparation::Ready(Box::new(
            RendererPreparedNetworkResourceLoad::new(
                resource_loader.frozen_request_client(),
                request,
            ),
        ))
    }
}
