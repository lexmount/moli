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
        host: &mut crate::native_bridge::JsContextHost,
        parser_document: Option<&Url>,
    ) -> Self {
        let Some(initiator) = crate::script_continuation::resource_initiator(scope) else {
            return parser_document.map(Self::from_document).unwrap_or_default();
        };
        // An explicit unknown/redacted source must remain unknown; it cannot
        // acquire the parser fallback merely because its URL is missing.
        if let Some(crate::script_continuation::ResourceInitiatorOrigin::Window(origin)) =
            initiator.origin
            && Some(origin) == host.window_origin_key_for_current_realm(scope)
        {
            return Self(initiator.source_url.map(String::from).unwrap_or_default());
        }
        Self::default()
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
