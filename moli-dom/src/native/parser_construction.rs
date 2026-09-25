use std::{cell::Cell, cell::RefCell, collections::HashSet};

use super::{DomHost, DomMutationEffects, NativeNodeId};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ConstructionState {
    #[default]
    Parsing,
    Finished,
    Aborted,
}

/// Owns child-construction obligations for one parser, independently of where
/// its nodes are subsequently moved. Creating a node through this session
/// starts its lifecycle before attributes, insertion, or custom constructors
/// can expose it. Ordinary DOM creation has no such obligation.
#[derive(Debug, Default)]
pub struct ParserConstruction {
    pending: RefCell<HashSet<NativeNodeId>>,
    state: Cell<ConstructionState>,
}

impl ParserConstruction {
    pub fn create_element(
        &self,
        host: &mut DomHost,
        document: NativeNodeId,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        assert_eq!(self.state.get(), ConstructionState::Parsing);
        let node = host.create_parser_element_without_attributes_for_document(
            document, local_name, namespace, prefix,
        );
        if host.begin_parsing_children(node) {
            assert!(self.pending.borrow_mut().insert(node));
        }
        node
    }

    pub fn is_pending(&self, node: NativeNodeId) -> bool {
        self.pending.borrow().contains(&node)
    }

    /// Close a node once, before delivering any resulting runtime effects.
    pub fn finish_children(&self, host: &mut DomHost, node: NativeNodeId) -> DomMutationEffects {
        if !self.pending.borrow_mut().remove(&node) {
            return DomMutationEffects::default();
        }
        host.finish_parsing_children_effects(node)
    }

    /// Called after the tree builder has processed normal EOF. Missing close
    /// callbacks are bugs, not implicit permission to publish unfinished text.
    pub fn finish(&self) {
        assert_ne!(self.state.get(), ConstructionState::Aborted);
        assert!(
            self.pending.borrow().is_empty(),
            "parser finished with unfinished element children: {:?}",
            self.pending.borrow(),
        );
        self.state.set(ConstructionState::Finished);
    }

    /// Cancellation deliberately leaves unfinished DOM elements unprocessed.
    /// In particular, dropping a parser must not publish a partial stylesheet.
    pub fn abort(&self) {
        self.pending.borrow_mut().clear();
        self.state.set(ConstructionState::Aborted);
    }
}

impl Drop for ParserConstruction {
    fn drop(&mut self) {
        if self.state.get() == ConstructionState::Parsing {
            self.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{DomStylesheetOwnerChangeKind, NativeDom};

    fn host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://parser-construction.test/").unwrap(),
        ))
    }

    fn style(construction: &ParserConstruction, host: &mut DomHost) -> NativeNodeId {
        construction.create_element(
            host,
            host.document_handle(),
            "style".into(),
            "http://www.w3.org/1999/xhtml".into(),
            None,
        )
    }

    #[test]
    fn parser_provenance_does_not_start_child_construction() {
        let mut host = host();
        let style = host.create_parser_element_without_attributes(
            "style".into(),
            "http://www.w3.org/1999/xhtml".into(),
            None,
        );
        assert!(host.node(style).unwrap().flags().parser_created());
        assert!(!host.is_style_element_parsing_children(style));
    }

    #[test]
    fn completion_belongs_to_the_creating_parser_after_document_adoption() {
        let mut host = host();
        let creator = ParserConstruction::default();
        let other_parser = ParserConstruction::default();
        let style = style(&creator, &mut host);
        let other_document = host.create_detached_html_document();
        assert_eq!(host.adopt_node(other_document, style), Some(style));

        let ignored = other_parser.finish_children(&mut host, style);
        assert!(ignored.stylesheet_owners().changes().is_empty());
        assert!(host.is_style_element_parsing_children(style));
        let completion = creator.finish_children(&mut host, style);
        assert!(!host.is_style_element_parsing_children(style));
        assert_eq!(completion.stylesheet_owners().changes().len(), 1);
        assert_eq!(
            completion.stylesheet_owners().changes()[0].kind(),
            &DomStylesheetOwnerChangeKind::ParsingFinished
        );
        assert!(
            creator
                .finish_children(&mut host, style)
                .stylesheet_owners()
                .changes()
                .is_empty()
        );
        creator.finish();
        other_parser.finish();
    }

    #[test]
    #[should_panic(expected = "parser finished with unfinished element children")]
    fn normal_finish_rejects_a_missing_close_callback() {
        let mut host = host();
        let construction = ParserConstruction::default();
        style(&construction, &mut host);
        construction.finish();
    }

    #[test]
    fn explicit_abort_and_drop_leave_incomplete_styles_unprocessed() {
        for explicit in [false, true] {
            let mut host = host();
            let construction = ParserConstruction::default();
            let style = style(&construction, &mut host);
            host.set_text_content(style, "body { color: red; }");
            if explicit {
                construction.abort();
                assert!(
                    construction
                        .finish_children(&mut host, style)
                        .stylesheet_owners()
                        .changes()
                        .is_empty()
                );
            }
            drop(construction);
            assert!(host.is_style_element_parsing_children(style));
        }
    }
}
