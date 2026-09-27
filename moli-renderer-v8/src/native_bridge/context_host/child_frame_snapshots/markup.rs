use super::super::{ChildBrowsingContextBootstrap, ChildBrowsingContextSnapshot, JsContextHost};
use crate::document_runtime::DomHandle;

impl JsContextHost {
    pub(crate) fn child_browsing_context_bootstrap(
        &self,
        handle: DomHandle,
    ) -> Option<ChildBrowsingContextBootstrap> {
        self.child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.live_bootstrap())
    }

    pub(crate) fn child_browsing_context_snapshot_markup(
        &mut self,
        handle: DomHandle,
    ) -> Option<ChildBrowsingContextSnapshot> {
        if let Some(snapshot) = self
            .child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.cached_snapshot())
        {
            return Some(snapshot);
        }
        let bootstrap = self.child_browsing_context_bootstrap(handle)?;
        match bootstrap {
            ChildBrowsingContextBootstrap::AboutBlank => {
                Some(ChildBrowsingContextSnapshot::about_blank(
                    self.document_base_url_for_child_context(handle),
                ))
            }
            ChildBrowsingContextBootstrap::Srcdoc { base_url, markup } => {
                Some(ChildBrowsingContextSnapshot::srcdoc(base_url, markup))
            }
            ChildBrowsingContextBootstrap::Url(url) => {
                if self.child_document_load_is_pending(handle) {
                    return None;
                }
                let snapshot =
                    self.materialize_local_child_snapshot_for_navigation_url(handle, &url)?;
                if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
                    entry.cache_snapshot_for_current_url_bootstrap(&url, &snapshot);
                }
                Some(snapshot)
            }
            ChildBrowsingContextBootstrap::Request(_) => None,
        }
    }
}
