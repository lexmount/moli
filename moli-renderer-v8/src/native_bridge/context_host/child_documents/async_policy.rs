use super::super::fixture_support::child_upstream_fixture_text;
use super::super::{ChildBrowsingContextBootstrap, JsContextHost};
use crate::document_runtime::DomHandle;
use url::Url;

impl JsContextHost {
    pub(in crate::native_bridge::context_host::child_documents) fn child_document_url_requires_async_load(
        &self,
        url: &Url,
    ) -> bool {
        matches!(url.scheme(), "http" | "https") && child_upstream_fixture_text(url).is_none()
    }

    pub(in crate::native_bridge::context_host::child_documents) fn child_document_bootstrap_requires_async_load(
        &self,
        bootstrap: &ChildBrowsingContextBootstrap,
    ) -> bool {
        match bootstrap {
            ChildBrowsingContextBootstrap::Url(url)
                if self.child_document_url_requires_async_load(url) =>
            {
                true
            }
            ChildBrowsingContextBootstrap::Request(_) => true,
            _ => false,
        }
    }

    pub(crate) fn child_browsing_context_attribute_bootstrap_requires_async_load(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| {
                self.child_document_bootstrap_requires_async_load(entry.attribute_bootstrap())
            })
    }
}
