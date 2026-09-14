use super::{
    font_faces::{
        OwnerFontFaceProjection, apply_font_face_owner_projection,
        finish_font_face_owner_projections, owner_font_face_projection,
    },
    style_sheets::sync_document_style_sheets,
};
use crate::{
    document_runtime::DomHandle,
    dom::native::{DomStylesheetOwnerChange, DomStylesheetOwnerChangeKind, Node},
    native_bridge::JsContextHost,
    util::node_wrapper_from_handle,
};
use std::collections::HashMap;

#[derive(Debug)]
enum DocumentCssProjection {
    StyleSheetList {
        document: DomHandle,
    },
    FontFaceOwner {
        document: DomHandle,
        owner: DomHandle,
        projection: Option<OwnerFontFaceProjection>,
    },
}

#[derive(Default)]
struct DocumentCssProjections {
    projections: Vec<DocumentCssProjection>,
    font_face_projection_indices: HashMap<(DomHandle, DomHandle), usize>,
}

impl DocumentCssProjections {
    fn from_owner_changes(host: &JsContextHost, changes: &[DomStylesheetOwnerChange]) -> Self {
        let mut projections = Self::default();
        for change in changes {
            if !owner_change_projects_css(host, change) {
                continue;
            }
            let owner = change.owner();
            let scopes = change.tree_scopes();
            if let Some(old_scope) = scopes.old() {
                projections.push_for_scope(host, old_scope, owner, None);
            }
            let current_projection = match change.kind() {
                DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: false }
                | DomStylesheetOwnerChangeKind::Unregistered => None,
                _ => owner_font_face_projection(host, owner),
            };
            if let Some(current_scope) = scopes.current_scope() {
                projections.push_for_scope(host, current_scope, owner, current_projection.clone());
            }
            if let Some(new_scope) = scopes.new_scope() {
                projections.push_for_scope(host, new_scope, owner, current_projection.clone());
            }
        }
        projections
    }

    fn for_source_change(host: &JsContextHost, owner: DomHandle) -> Self {
        let mut projections = Self::default();
        let Some(tree_scope) = host.dom_host().root_node_handle(owner) else {
            return projections;
        };
        let projection = owner_font_face_projection(host, owner);
        projections.push_for_scope(host, tree_scope, owner, projection);
        projections
    }

    fn push_for_scope(
        &mut self,
        host: &JsContextHost,
        scope: DomHandle,
        owner: DomHandle,
        projection: Option<OwnerFontFaceProjection>,
    ) {
        if !host.dom_host().node(scope).is_some_and(Node::is_document) {
            return;
        }
        if !self.projections.iter().any(|projection| {
            matches!(
                projection,
                DocumentCssProjection::StyleSheetList { document } if *document == scope
            )
        }) {
            self.projections
                .push(DocumentCssProjection::StyleSheetList { document: scope });
        }
        if let Some(index) = self
            .font_face_projection_indices
            .get(&(scope, owner))
            .copied()
        {
            let existing = &mut self.projections[index];
            let DocumentCssProjection::FontFaceOwner {
                projection: existing_projection,
                ..
            } = existing
            else {
                unreachable!();
            };
            *existing_projection = projection;
            return;
        }
        let index = self.projections.len();
        self.projections.push(DocumentCssProjection::FontFaceOwner {
            document: scope,
            owner,
            projection,
        });
        self.font_face_projection_indices
            .insert((scope, owner), index);
    }

    fn apply(self, scope: &mut v8::PinScope<'_, '_>, host: &JsContextHost) {
        let mut font_face_documents = Vec::new();
        for projection in self.projections {
            match projection {
                DocumentCssProjection::StyleSheetList { document } => {
                    let Some(holder) = node_wrapper_from_handle(scope, document) else {
                        continue;
                    };
                    let _ = sync_document_style_sheets(scope, holder, host.dom_host(), document);
                }
                DocumentCssProjection::FontFaceOwner {
                    document,
                    owner,
                    projection,
                } => {
                    if apply_font_face_owner_projection(scope, document, owner, projection.as_ref())
                        && !font_face_documents.contains(&document)
                    {
                        font_face_documents.push(document);
                    }
                }
            }
        }
        for document in font_face_documents {
            finish_font_face_owner_projections(scope, host, document);
        }
    }
}

pub(crate) fn apply_stylesheet_owner_css_projections(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    changes: &[DomStylesheetOwnerChange],
) {
    for change in changes {
        let owner = change.owner();
        let should_detach_cached_sheet = if host.dom_host().is_html_element_named(owner, "link") {
            owner_change_detaches_cached_link_sheet(host, change)
        } else {
            owner_change_detaches_cached_inline_sheet(host, change)
        };
        if owner_change_projects_css(host, change)
            && let Some(owner_wrapper) = node_wrapper_from_handle(scope, owner)
        {
            if matches!(
                change.kind(),
                DomStylesheetOwnerChangeKind::Attribute {
                    namespace: None,
                    local_name,
                } if local_name == "media"
            ) {
                let media = host
                    .dom_host()
                    .get_attribute(owner, "media")
                    .unwrap_or_default();
                crate::native_bridge::element::sync_cached_style_sheet_media_from_owner(
                    scope,
                    owner_wrapper,
                    &media,
                );
            }
            if should_detach_cached_sheet {
                crate::native_bridge::element::detach_cached_style_sheet_for_element(
                    scope,
                    owner_wrapper,
                );
            } else if let Some(stylesheet) = host.owner_live_stylesheet(owner) {
                crate::native_bridge::element::detach_cached_style_sheet_if_live_stylesheet_changed(
                    scope,
                    owner_wrapper,
                    stylesheet.id(),
                );
            }
        }
    }
    DocumentCssProjections::from_owner_changes(host, changes).apply(scope, host);
}

fn owner_change_detaches_cached_inline_sheet(
    host: &JsContextHost,
    change: &DomStylesheetOwnerChange,
) -> bool {
    owner_is_inline_style_element(host, change.owner())
        && (matches!(
            change.kind(),
            DomStylesheetOwnerChangeKind::Unregistered
                | DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: false }
        ) || host.owner_style_sheet_source(change.owner()).is_none())
}

fn owner_change_detaches_cached_link_sheet(
    host: &JsContextHost,
    change: &DomStylesheetOwnerChange,
) -> bool {
    if matches!(
        change.kind(),
        DomStylesheetOwnerChangeKind::Attribute {
            namespace: None,
            local_name,
        } if local_name == "media"
    ) {
        return false;
    }
    if matches!(
        change.kind(),
        DomStylesheetOwnerChangeKind::Attribute {
            namespace: None,
            local_name,
        } if local_name == "title"
    ) {
        let Some(element) = host
            .dom_host()
            .node(change.owner())
            .and_then(Node::as_element)
        else {
            return true;
        };
        // A valid title update changes sheet-set metadata without replacing the
        // associated CSSStyleSheet. Detach only when an alternate becomes invalid.
        return !crate::style_engine::link_rel_qualifies_as_stylesheet(
            element.attribute("rel"),
            element.attribute("title"),
        );
    }
    if matches!(
        change.kind(),
        DomStylesheetOwnerChangeKind::Attribute { .. }
    ) {
        // The source lifecycle retains an installed sheet while its successor
        // loads. Preserve the corresponding JS object until an actual install
        // changes its identity, or the owner stops being a stylesheet.
        return host
            .linked_stylesheet_source_for_owner(change.owner())
            .is_none();
    }
    true
}

pub(crate) fn apply_stylesheet_source_css_projection(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    owner: DomHandle,
) {
    if host.dom_host().is_html_element_named(owner, "link")
        && let Some(expected_id) = host
            .linked_stylesheet_source_for_owner(owner)
            .and_then(|source| source.live_stylesheet_id())
        && let Some(owner_wrapper) = node_wrapper_from_handle(scope, owner)
    {
        crate::native_bridge::element::detach_cached_style_sheet_if_live_stylesheet_changed(
            scope,
            owner_wrapper,
            expected_id,
        );
    }
    DocumentCssProjections::for_source_change(host, owner).apply(scope, host);
}

fn owner_change_projects_css(host: &JsContextHost, change: &DomStylesheetOwnerChange) -> bool {
    match change.kind() {
        DomStylesheetOwnerChangeKind::Attribute {
            namespace,
            local_name,
        } => owner_attribute_projects_css(
            owner_is_inline_style_element(host, change.owner()),
            host.dom_host()
                .is_html_element_named(change.owner(), "link"),
            namespace.as_deref(),
            local_name,
        ),
        DomStylesheetOwnerChangeKind::Registered
        | DomStylesheetOwnerChangeKind::Unregistered
        | DomStylesheetOwnerChangeKind::Contents
        | DomStylesheetOwnerChangeKind::OwnerDocumentChanged
        | DomStylesheetOwnerChangeKind::TreeConnectionChanged { .. } => true,
    }
}

fn owner_is_inline_style_element(host: &JsContextHost, owner: DomHandle) -> bool {
    host.dom_host().is_inline_style_sheet_owner(owner)
}

fn owner_attribute_projects_css(
    is_style: bool,
    is_link: bool,
    namespace: Option<&str>,
    local_name: &str,
) -> bool {
    if namespace.is_some() {
        return false;
    }
    if is_style {
        return matches!(local_name, "type" | "media");
    }
    is_link
        && (matches!(local_name, "title" | "media")
            || crate::document_runtime::attribute_reprocesses_connected_stylesheet(local_name))
}

#[cfg(test)]
mod tests {
    use super::owner_attribute_projects_css;

    #[test]
    fn irrelevant_owner_attributes_do_not_create_css_projections() {
        for local_name in ["class", "id", "data-state", "title"] {
            assert!(!owner_attribute_projects_css(true, false, None, local_name));
        }
        for local_name in ["class", "id", "data-state"] {
            assert!(!owner_attribute_projects_css(false, true, None, local_name));
        }
        assert!(!owner_attribute_projects_css(
            false,
            true,
            Some("urn:test"),
            "href"
        ));
    }

    #[test]
    fn source_identity_attributes_create_only_relevant_projections() {
        for local_name in ["type", "media"] {
            assert!(owner_attribute_projects_css(true, false, None, local_name));
        }
        assert!(!owner_attribute_projects_css(true, false, None, "disabled"));
        for local_name in [
            "href", "rel", "title", "media", "as", "type", "disabled", "sizes",
        ] {
            assert!(owner_attribute_projects_css(false, true, None, local_name));
        }
    }
}
