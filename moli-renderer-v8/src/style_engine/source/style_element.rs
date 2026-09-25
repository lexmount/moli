use super::{
    DomHandle, DomHost, DomStylesheetOwnerChange, DomStylesheetOwnerChangeKind, MoliStyleEngine,
    owner_document_for_source_owner, stylesheet_source_base_url,
};
use crate::dom::native::Node;

impl MoliStyleEngine {
    /// Adopt style elements built before this style engine observed their DOM
    /// lifecycle. Existing sources, including CSSOM edits, remain authoritative.
    pub(crate) fn initialize_style_element_sources_with_host(
        &mut self,
        host: &DomHost,
        document: DomHandle,
    ) {
        for (_, owners) in host
            .stylesheet_candidate_tree_scope_snapshots_for_document(document)
            .iter()
        {
            for &owner in owners.iter() {
                if host.is_inline_style_sheet_owner(owner)
                    && self.owner_style_sheet_processing_source(owner).is_none()
                {
                    self.process_style_element_with_host(host, owner);
                }
            }
        }
    }

    pub(super) fn apply_style_element_change_with_host(
        &mut self,
        host: &DomHost,
        change: &DomStylesheetOwnerChange,
    ) {
        let owner = change.owner();
        match change.kind() {
            DomStylesheetOwnerChangeKind::Registered
            | DomStylesheetOwnerChangeKind::Contents
            | DomStylesheetOwnerChangeKind::ParsingFinished => {
                if style_element_has_document_root(host, owner) {
                    self.process_style_element_with_host(host, owner);
                }
            }
            DomStylesheetOwnerChangeKind::OwnerDocumentChanged
            | DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: true } => {
                self.process_style_element_with_host(host, owner);
            }
            DomStylesheetOwnerChangeKind::Unregistered
            | DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: false } => {
                self.remove_owner_style_sheet_source(owner);
                let documents = self.linked_stylesheet_owner_documents(owner);
                self.remove_linked_stylesheet_owner_from_documents(host, owner, documents);
            }
            DomStylesheetOwnerChangeKind::Attribute {
                namespace: None,
                local_name,
            } if local_name == "type" => {
                // Type changes eligibility, not the prepared source or CSSOM identity.
                self.invalidate_owner_stylesheet_set_for_owner_with_host(host, owner);
            }
            DomStylesheetOwnerChangeKind::Attribute { .. } => {}
        }
    }

    /// The only production path that prepares an owner source from DOM text.
    /// Parsing state is enforced here, before a source can become visible to
    /// stylesheet installation, CSSOM reads, or rendering synchronization.
    fn process_style_element_with_host(&mut self, host: &DomHost, owner: DomHandle) {
        if host.is_style_element_parsing_children(owner) {
            return;
        }
        let Some(document) = owner_document_for_source_owner(host, owner) else {
            return;
        };
        let css_text = host.text_content(owner).unwrap_or_default();
        let parser_base = stylesheet_source_base_url(host, owner);
        let previous_linked_documents = self.linked_stylesheet_owner_documents(owner);
        self.remove_linked_stylesheet_owner_from_documents(host, owner, previous_linked_documents);
        let previous_documents = self.move_owner_style_sheet_source_to_document(document, owner);
        self.world_for_document(document)
            .owner_style_sheet_sources
            .borrow_mut()
            .replace_processed_source(owner, css_text, parser_base);
        self.invalidate_owner_stylesheet_set_for_owner_with_host(host, owner);
        for previous_document in previous_documents {
            self.mark_document_stylesheet_set_dirty(previous_document);
        }
    }
}

fn style_element_has_document_root(host: &DomHost, owner: DomHandle) -> bool {
    if host.is_connected(owner) {
        return true;
    }
    // Documents without a browsing context have no native connected flag,
    // but styles inserted into them still participate in their CSSOM.
    let mut current = owner;
    loop {
        let Some(root) = host.root_node_handle(current) else {
            return false;
        };
        if host.node(root).is_some_and(Node::is_document) {
            return true;
        }
        let Some(shadow_host) = host.shadow_root_host(root) else {
            return false;
        };
        current = shadow_host;
    }
}
