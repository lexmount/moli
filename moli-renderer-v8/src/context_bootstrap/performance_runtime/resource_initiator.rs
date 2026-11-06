use moli_fetch::{ResponseCacheState, ResponseHead};
use url::Url;

use super::ResourcePerformanceEntry;

/// Request-owned attribution, independent of the script or Document active
/// when an asynchronous response is published. It carries no realm authority.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResourceTimingInitiator(String);

impl ResourceTimingInitiator {
    pub(crate) fn capture_window(
        scope: &mut v8::PinScope<'_, '_>,
        host: &mut crate::native_bridge::PageScriptHost,
        parser_document: Option<&Url>,
    ) -> Self {
        let context = scope.get_current_context();
        let Some(identity) = host.window_execution_context_identity_for_v8_context(scope, context)
        else {
            return Self::default();
        };
        if !host.window_execution_context_identity_is_current(identity) {
            return Self::default();
        }
        let owner = identity.dispatch_scope();
        let Some(loader) = host.document_resource_loader_for_dispatch_scope(owner) else {
            return Self::default();
        };
        let Some(environment) = host.subresource_request_environment(&loader, owner) else {
            return Self::default();
        };
        let origin = environment.request_origin;
        if origin.is_opaque() {
            return Self::default();
        }
        // An explicit unknown or redacted primitive anchor cannot acquire the
        // parser fallback merely because its source URL is empty.
        if crate::resource_initiator::anchor(scope).is_some() {
            return Self(crate::resource_initiator::current_url(scope, &origin));
        }
        parser_document
            .filter(|url| origin.same_origin_url(url))
            .map(Self::from_document)
            .unwrap_or_default()
    }

    pub(crate) fn from_document(url: &Url) -> Self {
        Self(url.as_str().to_owned())
    }

    pub(crate) fn from_captured_url(url: String) -> Self {
        Self(url)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn apply(
        &self,
        entry: &mut ResourcePerformanceEntry,
        head: &ResponseHead,
        readable: bool,
    ) {
        // TAO exposes network timings, not the initiating source. Cache and
        // service-worker responses likewise do not expose a network dependency.
        if readable
            && head.cache_state == ResponseCacheState::None
            && !head.preload_state.from_service_worker()
        {
            entry.initiator_url.clone_from(&self.0);
        }
    }

    pub(crate) fn apply_child_document(
        &self,
        entry: &mut ResourcePerformanceEntry,
        network: &crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
        expose_document_response: bool,
    ) {
        if expose_document_response
            && network.resource_timing_body_visible
            && network.cache_state == ResponseCacheState::None
            && !network.preload_state.from_service_worker()
        {
            entry.initiator_url.clone_from(&self.0);
        }
    }
}
