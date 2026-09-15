//! Parser-owned adapters for the shared stylesheet blocking read contract.
//!
//! Shared types and discovery behavior live in `moli-stylesheet-blocking`
//! and are intentionally not re-exported from this crate.

use moli_dom::{NodeId, native::NativeNodeId};
use moli_stylesheet_blocking::{StylesheetBlockingReadView, StylesheetElementRead};
use url::Url;

use crate::ParserStreamDocumentSnapshot;

#[cfg(test)]
use moli_stylesheet_blocking::{
    collect_document_owned_blocking_stylesheet_candidates,
    collect_document_owned_blocking_stylesheet_nodes_before, connected_preload_like_link_url,
    document_owned_blocking_stylesheet_candidate_for_node, preload_like_link_loads_stylesheet,
    stylesheet_link_disposition,
};

impl StylesheetBlockingReadView for ParserStreamDocumentSnapshot {
    fn stylesheet_element(&self, node_id: NativeNodeId) -> Option<StylesheetElementRead> {
        self.node(node_id)
            .and_then(StylesheetElementRead::from_node)
    }

    fn child_ids(&self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        self.child_ids(node_id).collect()
    }

    fn text_content(&self, node_id: NativeNodeId) -> Option<String> {
        self.text_content(node_id)
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.final_url().cloned()
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        ParserStreamDocumentSnapshot::document_base_url(self)
    }

    fn document_node_id(&self) -> NativeNodeId {
        self.document_node_id()
    }

    fn document_is_quirks_mode(&self) -> bool {
        self.node(self.document_node_id())
            .and_then(moli_dom::native::Node::as_document)
            .is_some_and(|document| document.is_quirks_mode())
    }

    fn document_order_stylesheet_candidate_ids_before(
        &self,
        target_node_id: Option<NodeId>,
    ) -> Vec<NativeNodeId> {
        self.stylesheet_candidate_handles_before_in_tree_scope(
            self.document_node_id(),
            target_node_id.map(|node_id| NativeNodeId::new(node_id.index())),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        collect_document_owned_blocking_stylesheet_candidates,
        collect_document_owned_blocking_stylesheet_nodes_before, connected_preload_like_link_url,
        document_owned_blocking_stylesheet_candidate_for_node, preload_like_link_loads_stylesheet,
        stylesheet_link_disposition,
    };
    use crate::{HtmlParser, ParserPumpStep};
    use moli_dom::native::Node;

    #[test]
    fn document_owned_blocking_candidates_include_parser_created_style_imports() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            url::Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><style>@import url('/slow.css');</style><script defer>window.x = 1;</script></head></html>".to_owned(),
        );
        let script = document.script_handles()[0];

        let candidates = collect_document_owned_blocking_stylesheet_candidates(&document);
        let blockers = collect_document_owned_blocking_stylesheet_nodes_before(
            &document,
            moli_dom::NodeId::new(script.index()),
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            document_owned_blocking_stylesheet_candidate_for_node(
                &document,
                candidates[0].node_id(),
            ),
            Some(candidates[0].clone()),
        );
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0], candidates[0].node_id());
    }

    #[test]
    fn parser_created_link_blocker_is_captured_before_link_processing_state_is_consumed() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let mut stream = parser.start_document(url::Url::parse("https://example.com/").unwrap());
        stream.append_to_end(
            "<!doctype html><html><head><link rel=stylesheet href='/slow.css'><script>window.x = 1;</script></head></html>".to_owned(),
        );
        let mut captured_blockers = Vec::new();
        while stream.has_pending_input() {
            let outcome = stream.pump_next_parser_step(0);
            captured_blockers.extend(outcome.discovered_blocking_stylesheet_inputs);
            if matches!(outcome.result, ParserPumpStep::InputDrained) && !stream.has_pending_input()
            {
                break;
            }
        }
        let document = stream.finish();
        let script = document.script_handles()[0];

        let blockers = collect_document_owned_blocking_stylesheet_nodes_before(
            &document,
            moli_dom::NodeId::new(script.index()),
        );

        assert_eq!(captured_blockers.len(), 1);
        assert!(
            blockers.is_empty(),
            "the completed link must not remain eligible for a later parser-blocking rescan"
        );
        assert!(
            document
                .node(moli_dom::native::NativeNodeId::new(
                    captured_blockers[0].node_id().index(),
                ))
                .is_some_and(|node| node.flags().parser_created())
        );
    }

    #[test]
    fn stylesheet_link_resolves_against_the_processed_document_base_url() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            url::Url::parse("https://example.com/page/index.html").unwrap(),
            "<!doctype html><html><head><base href=\"/assets/\"><link rel=\"stylesheet\" href=\"app.css\"></head></html>"
                .to_owned(),
        );
        let head = document.document_head_handle().expect("head handle");
        let link = document
            .child_nodes(head)
            .expect("head children")
            .into_iter()
            .find(|handle| {
                document
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("link"))
            })
            .expect("link handle");

        let disposition =
            stylesheet_link_disposition(&document, moli_dom::NodeId::new(link.index()))
                .expect("stylesheet disposition");
        assert_eq!(
            disposition.url().as_str(),
            "https://example.com/assets/app.css"
        );
        assert!(!disposition.options().quirks_mode_mime_compatibility());
    }

    #[test]
    fn stylesheet_link_captures_quirks_mode_mime_compatibility() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            url::Url::parse("https://example.com/page.html").unwrap(),
            "<html><head><link rel=stylesheet href=app.css></head></html>".to_owned(),
        );
        let link = document
            .document_head_handle()
            .and_then(|head| document.child_nodes(head))
            .and_then(|children| {
                children.into_iter().find(|handle| {
                    document
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("link"))
                })
            })
            .expect("stylesheet link");

        let disposition =
            stylesheet_link_disposition(&document, moli_dom::NodeId::new(link.index()))
                .expect("stylesheet disposition");

        assert!(disposition.options().quirks_mode_mime_compatibility());
    }

    #[test]
    fn preload_like_link_stylesheet_filter_ignores_script_modulepreloads() {
        assert!(preload_like_link_loads_stylesheet(
            "preload",
            Some("style"),
            "/app.css"
        ));
        assert!(!preload_like_link_loads_stylesheet(
            "preload",
            Some("script"),
            "/entry.js"
        ));
        assert!(!preload_like_link_loads_stylesheet(
            "modulepreload",
            Some("script"),
            "/entry.js"
        ));
        assert!(preload_like_link_loads_stylesheet(
            "modulepreload",
            Some("style"),
            "/theme.css"
        ));
        assert!(preload_like_link_loads_stylesheet(
            "modulepreload",
            None,
            "/theme.css?hash=1"
        ));
        assert!(!preload_like_link_loads_stylesheet(
            "modulepreload",
            None,
            "/entry.js"
        ));
    }

    #[test]
    fn connected_preload_like_link_url_accepts_prefetch() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            url::Url::parse("https://example.com/path/page.html").unwrap(),
            "<!doctype html><html><head><link rel=\"prefetch\" href=\"../next.html\"></head><body></body></html>"
                .to_owned(),
        );
        let head = document.document_head_handle().expect("head handle");
        let link = document
            .child_nodes(head)
            .expect("head children")
            .into_iter()
            .find(|handle| {
                document
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("link"))
            })
            .expect("link handle");

        assert_eq!(
            connected_preload_like_link_url(&document, moli_dom::NodeId::new(link.index())),
            Some(url::Url::parse("https://example.com/next.html").unwrap())
        );
    }

    #[test]
    fn connected_preload_like_link_url_accepts_compression_dictionary() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            url::Url::parse("https://example.com/path/page.html").unwrap(),
            "<!doctype html><html><head><link rel=\"compression-dictionary\" href=\"../dict.bin\"></head><body></body></html>"
                .to_owned(),
        );
        let head = document.document_head_handle().expect("head handle");
        let link = document
            .child_nodes(head)
            .expect("head children")
            .into_iter()
            .find(|handle| {
                document
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("link"))
            })
            .expect("link handle");

        assert_eq!(
            connected_preload_like_link_url(&document, moli_dom::NodeId::new(link.index())),
            Some(url::Url::parse("https://example.com/dict.bin").unwrap())
        );
    }
}
