use super::super::{ChildBrowsingContextBootstrap, JsContextHost};
use super::ChildDocumentNavigationInitiator;
use crate::document_runtime::DomHandle;
use url::{Position, Url};

impl JsContextHost {
    pub(in crate::native_bridge::context_host::child_documents) fn child_document_navigation_would_recurse(
        &self,
        handle: DomHandle,
        bootstrap: &ChildBrowsingContextBootstrap,
        initiator: ChildDocumentNavigationInitiator,
    ) -> bool {
        if matches!(
            initiator,
            ChildDocumentNavigationInitiator::HistoryTraversal
        ) {
            return false;
        }
        let Some(url) = Self::child_browsing_context_bootstrap_url(bootstrap) else {
            return false;
        };
        let method = match bootstrap {
            ChildBrowsingContextBootstrap::Request(request) => request.method.as_str(),
            _ => "GET",
        };
        let mut ancestors = Vec::new();
        let mut child = handle;
        for _ in 0..=self.child_browsing_contexts.len() {
            ancestors.push(self.document_url_for_child_context(child));
            let Some(parent) = self.child_browsing_context_parent_handle(child) else {
                break;
            };
            child = parent;
        }
        child_url_repeats_ancestors(&url, method, &ancestors)
    }
}

fn child_url_repeats_ancestors(url: &Url, method: &str, ancestors: &[Url]) -> bool {
    if url.scheme() == "about" || method == "POST" {
        return false;
    }
    // Like Chromium's IsSelfReferentialURL, allow one self-reference. A second
    // matching ancestor would let automatically created frames load forever.
    ancestors
        .iter()
        .filter(|ancestor| ancestor[..Position::AfterQuery] == url[..Position::AfterQuery])
        .take(2)
        .count()
        == 2
}

#[cfg(test)]
mod tests {
    use super::child_url_repeats_ancestors;
    use url::Url;

    #[test]
    fn repeated_ancestor_url_admission_preserves_url_and_request_boundaries() {
        for (target, method, ancestors, blocked) in [
            (
                "https://frame.test/a#target",
                "GET",
                vec!["https://frame.test/a#parent"],
                false,
            ),
            (
                "https://frame.test/a#target",
                "GET",
                vec!["https://frame.test/a#parent", "https://frame.test/a"],
                true,
            ),
            (
                "https://frame.test/a?q=1",
                "GET",
                vec!["https://frame.test/a?q=2", "https://frame.test/a?q=1"],
                false,
            ),
            (
                "https://frame.test/a",
                "GET",
                vec!["https://other.test/a", "https://frame.test/a"],
                false,
            ),
            (
                "https://frame.test/a",
                "POST",
                vec!["https://frame.test/a", "https://frame.test/a"],
                false,
            ),
            (
                "about:blank",
                "GET",
                vec!["about:blank", "about:blank"],
                false,
            ),
            (
                "about:srcdoc",
                "GET",
                vec!["about:srcdoc", "about:srcdoc"],
                false,
            ),
        ] {
            let target = Url::parse(target).unwrap();
            let ancestors = ancestors
                .into_iter()
                .map(|url| Url::parse(url).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                child_url_repeats_ancestors(&target, method, &ancestors),
                blocked,
                "{target}/{method}"
            );
        }
    }
}
