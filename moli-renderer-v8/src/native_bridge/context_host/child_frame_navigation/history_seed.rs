use super::super::{ChildBrowsingContextBootstrap, JsContextHost, NavigationHistoryEntrySeed};
use moli_page_types::{
    apply_child_browsing_context_navigation_to_entry_seed as apply_child_navigation_to_seed,
    child_browsing_context_single_entry_seed as page_child_browsing_context_single_entry_seed,
    replace_child_browsing_context_navigation_in_entry_seed as replace_child_navigation_in_seed,
};
use url::Url;

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn child_browsing_context_bootstrap_url(
        bootstrap: &ChildBrowsingContextBootstrap,
    ) -> Option<Url> {
        match bootstrap {
            ChildBrowsingContextBootstrap::AboutBlank => Url::parse("about:blank").ok(),
            ChildBrowsingContextBootstrap::Url(url) => Some(url.clone()),
            ChildBrowsingContextBootstrap::Request(request) => Some(request.url.clone()),
            ChildBrowsingContextBootstrap::Srcdoc { base_url, .. } => Some(base_url.clone()),
        }
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_navigation_entry_url(
        bootstrap: &ChildBrowsingContextBootstrap,
    ) -> Option<Url> {
        match bootstrap {
            ChildBrowsingContextBootstrap::Srcdoc { .. } => Url::parse("about:srcdoc").ok(),
            ChildBrowsingContextBootstrap::Url(url) if url.scheme() == "javascript" => {
                Url::parse("about:blank").ok()
            }
            ChildBrowsingContextBootstrap::Request(request)
                if request.url.scheme() == "javascript" =>
            {
                Url::parse("about:blank").ok()
            }
            _ => Self::child_browsing_context_bootstrap_url(bootstrap),
        }
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_single_entry_seed(
        bootstrap: &ChildBrowsingContextBootstrap,
    ) -> NavigationHistoryEntrySeed {
        let url = Self::child_browsing_context_navigation_entry_url(bootstrap);
        page_child_browsing_context_single_entry_seed(url.as_ref())
    }

    pub(in crate::native_bridge::context_host) fn apply_child_browsing_context_navigation_to_entry_seed(
        seed: &mut NavigationHistoryEntrySeed,
        url: &Url,
    ) {
        apply_child_navigation_to_seed(seed, url, None, None);
    }

    pub(in crate::native_bridge::context_host) fn replace_child_browsing_context_navigation_in_entry_seed(
        seed: &mut NavigationHistoryEntrySeed,
        url: &Url,
        history_state_json: Option<String>,
        navigation_state_json: Option<String>,
    ) {
        replace_child_navigation_in_seed(seed, url, history_state_json, navigation_state_json);
    }
}
