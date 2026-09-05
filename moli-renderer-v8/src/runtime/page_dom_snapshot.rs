use std::collections::HashMap;

use serde_json::{Value, json};

use crate::dom::native::DomHost;
use crate::native_bridge::DetachedChildBrowsingContextDocumentSnapshot;
use crate::runtime::page_surface::{
    RendererDomSnapshotCaptureOptions, RendererDomSnapshotCapturePayload,
};
use crate::script_vm::ScriptVm;
use moli_page_types::{
    DocumentNodeObjectSnapshot, DocumentNodeSnapshot, DocumentSnapshotNodeId,
    MAX_DOM_OUTPUT_TREE_DEPTH,
};

use super::{page_dom::live_document_node_snapshot, page_vm::PageVm};

type ContentDocumentIndices = HashMap<DocumentSnapshotNodeId, Vec<(String, usize)>>;

struct CaptureSnapshotState {
    options: RendererDomSnapshotCaptureOptions,
    strings: StringTable,
    documents: Vec<CapturedDocumentState>,
    empty_string: usize,
}

struct CapturedDocumentRoot {
    frame_id: String,
    owner: Option<CapturedContentDocumentOwner>,
    root: DocumentNodeSnapshot,
    style_source: CapturedDocumentStyleSource,
}

#[derive(Clone, Copy)]
enum CapturedDocumentStyleSource {
    Live,
    DetachedMarkup,
}

struct CapturedContentDocumentOwner {
    parent_frame_id: String,
    node_id: DocumentSnapshotNodeId,
}

struct CapturedDocumentState {
    frame_id: String,
    document_url: String,
    title: String,
    base_url: String,
    public_id: String,
    system_id: String,
    nodes: NodeTableBuilder,
    styles: Vec<Vec<usize>>,
    bounds: Vec<Vec<f64>>,
    dom_rects: Vec<Vec<f64>>,
    text: Vec<usize>,
    blended_background_colors: Vec<usize>,
    text_color_opacities: Vec<f64>,
    style_source: CapturedDocumentStyleSource,
}

impl PageVm {
    pub(crate) fn dom_snapshot_capture_payload(
        &mut self,
        top_frame_id: &str,
        options: RendererDomSnapshotCaptureOptions,
    ) -> Option<RendererDomSnapshotCapturePayload> {
        let documents = self.dom_snapshot_document_roots(top_frame_id);
        if documents.is_empty() {
            return None;
        }
        let mut state = CaptureSnapshotState::new(options, documents);
        state.push_layout_results(self.vm());
        Some(RendererDomSnapshotCapturePayload::from_protocol_payload(
            state.into_result(),
        ))
    }

    fn dom_snapshot_document_roots(&mut self, top_frame_id: &str) -> Vec<CapturedDocumentRoot> {
        let snapshots = self.document_node_snapshots_for_dom_snapshot(-1, true);
        let mut documents = captured_document_roots(snapshots, self, top_frame_id);
        let detached_documents = self
            .vm_mut()
            .detached_child_browsing_context_document_snapshots_for_dom_snapshot(top_frame_id);
        documents.extend(
            detached_documents
                .into_iter()
                .filter_map(detached_document_root),
        );
        documents
    }
}

impl CaptureSnapshotState {
    fn new(
        options: RendererDomSnapshotCaptureOptions,
        documents: Vec<CapturedDocumentRoot>,
    ) -> Self {
        let mut strings = StringTable::default();
        let empty_string = strings.intern("");
        let mut content_document_indices = ContentDocumentIndices::new();
        for (index, document) in documents.iter().enumerate() {
            let Some(owner) = &document.owner else {
                continue;
            };
            content_document_indices
                .entry(owner.node_id)
                .or_default()
                .push((owner.parent_frame_id.clone(), index));
        }
        let documents = documents
            .into_iter()
            .map(|document| {
                CapturedDocumentState::new(document, &content_document_indices, &mut strings)
            })
            .collect();
        Self {
            options,
            strings,
            documents,
            empty_string,
        }
    }

    fn push_layout_results(&mut self, vm: &ScriptVm) {
        for document in &mut self.documents {
            document.push_layout_results(
                vm,
                &self.options.computed_styles,
                &mut self.strings,
                self.empty_string,
            );
        }
    }

    fn into_result(mut self) -> Value {
        let documents = self
            .documents
            .into_iter()
            .map(|document| document.into_value(&self.options, &mut self.strings))
            .collect::<Vec<_>>();
        json!({
            "documents": documents,
            "strings": self.strings.into_strings(),
        })
    }
}

impl CapturedDocumentState {
    fn new(
        document: CapturedDocumentRoot,
        content_document_indices: &ContentDocumentIndices,
        strings: &mut StringTable,
    ) -> Self {
        let CapturedDocumentRoot {
            frame_id,
            owner: _,
            root,
            style_source,
        } = document;
        let (public_id, system_id) = document_type_values(&root);
        let mut nodes = NodeTableBuilder::default();
        nodes.push_tree(&root, None, &frame_id, content_document_indices, strings);
        let document_url = root.document_url.clone();
        let title = document_title(&root);
        let base_url = root.base_url.clone();
        Self {
            frame_id,
            document_url,
            title,
            base_url,
            public_id,
            system_id,
            nodes,
            styles: Vec::new(),
            bounds: Vec::new(),
            dom_rects: Vec::new(),
            text: Vec::new(),
            blended_background_colors: Vec::new(),
            text_color_opacities: Vec::new(),
            style_source,
        }
    }

    fn push_layout_results(
        &mut self,
        vm: &ScriptVm,
        computed_styles: &[String],
        strings: &mut StringTable,
        empty_string: usize,
    ) {
        let bounds = lightweight_snapshot_bounds();
        let styles = match self.style_source {
            // One observation shares style preparation and the lazy sizing
            // projection across every node in this Document's capture.
            CapturedDocumentStyleSource::Live => vm
                .computed_style_property_values_for_document_snapshot(
                    self.nodes.layout_nodes.iter().map(|(_, node_id)| *node_id),
                    computed_styles,
                ),
            // These compatibility snapshots retain markup but no live style owner.
            CapturedDocumentStyleSource::DetachedMarkup => {
                vec![Vec::new(); self.nodes.layout_nodes.len()]
            }
        };
        for values in styles {
            self.styles.push(
                values
                    .into_iter()
                    .map(|value| strings.intern(value))
                    .collect(),
            );
            self.bounds.push(bounds.clone());
            self.dom_rects.push(bounds.clone());
            self.text.push(empty_string);
            self.blended_background_colors.push(empty_string);
            self.text_color_opacities.push(1.0);
        }
    }

    fn into_value(
        self,
        options: &RendererDomSnapshotCaptureOptions,
        strings: &mut StringTable,
    ) -> Value {
        let layout = self.layout_value(options);
        let nodes = self.nodes.into_value();
        let frame_id = strings.intern(&self.frame_id);
        let public_id = strings.intern(&self.public_id);
        let system_id = strings.intern(&self.system_id);
        json!({
            "documentURL": strings.intern(&self.document_url),
            "title": strings.intern(&self.title),
            "baseURL": strings.intern(&self.base_url),
            "contentLanguage": strings.intern(""),
            "encodingName": strings.intern("UTF-8"),
            "publicId": public_id,
            "systemId": system_id,
            "frameId": frame_id,
            "nodes": nodes,
            "layout": layout,
            "textBoxes": {
                "layoutIndex": [],
                "bounds": [],
                "start": [],
                "length": [],
            },
            "scrollOffsetX": 0.0,
            "scrollOffsetY": 0.0,
            "contentWidth": 0.0,
            "contentHeight": 0.0,
        })
    }

    fn layout_value(&self, options: &RendererDomSnapshotCaptureOptions) -> Value {
        let node_index = self
            .nodes
            .layout_nodes
            .iter()
            .map(|(node_table_index, _)| *node_table_index)
            .collect::<Vec<_>>();
        let mut layout = serde_json::Map::new();
        layout.insert("nodeIndex".to_owned(), json!(node_index));
        layout.insert("styles".to_owned(), json!(self.styles));
        layout.insert("bounds".to_owned(), json!(self.bounds));
        layout.insert("text".to_owned(), json!(self.text));
        let stacking_contexts = if self.nodes.layout_nodes.is_empty() {
            Vec::new()
        } else {
            vec![0]
        };
        layout.insert(
            "stackingContexts".to_owned(),
            json!({
                "index": stacking_contexts,
            }),
        );
        if options.include_paint_order {
            layout.insert(
                "paintOrders".to_owned(),
                json!((0..self.nodes.layout_nodes.len()).collect::<Vec<_>>()),
            );
        }
        if options.include_dom_rects {
            layout.insert("offsetRects".to_owned(), json!(self.dom_rects));
            layout.insert("scrollRects".to_owned(), json!(self.dom_rects));
            layout.insert("clientRects".to_owned(), json!(self.dom_rects));
        }
        if options.include_blended_background_colors {
            layout.insert(
                "blendedBackgroundColors".to_owned(),
                json!(self.blended_background_colors),
            );
        }
        if options.include_text_color_opacities {
            layout.insert(
                "textColorOpacities".to_owned(),
                json!(self.text_color_opacities),
            );
        }
        Value::Object(layout)
    }
}

fn captured_document_roots(
    snapshots: Vec<DocumentNodeObjectSnapshot>,
    page_vm: &PageVm,
    top_frame_id: &str,
) -> Vec<CapturedDocumentRoot> {
    snapshots
        .into_iter()
        .map(|snapshot| CapturedDocumentRoot {
            frame_id: snapshot.frame_id.unwrap_or_else(|| top_frame_id.to_owned()),
            owner: snapshot
                .owner_node_id
                .map(|node_id| CapturedContentDocumentOwner {
                    parent_frame_id: dom_snapshot_parent_frame_id_for_owner_node(
                        page_vm,
                        top_frame_id,
                        node_id,
                    ),
                    node_id,
                }),
            root: snapshot.snapshot,
            style_source: CapturedDocumentStyleSource::Live,
        })
        .collect()
}

fn dom_snapshot_parent_frame_id_for_owner_node(
    page_vm: &PageVm,
    top_frame_id: &str,
    node_id: DocumentSnapshotNodeId,
) -> String {
    let dom_host = page_vm.vm().document_runtime.dom_host();
    let owner_document = dom_host
        .node(node_id)
        .and_then(|node| node.owner_document());
    if owner_document == Some(dom_host.document_handle()) {
        return top_frame_id.to_owned();
    }
    page_vm
        .vm()
        .child_browsing_context_parent_frame_id(node_id)
        .unwrap_or_else(|| top_frame_id.to_owned())
}

fn detached_document_root(
    snapshot: DetachedChildBrowsingContextDocumentSnapshot,
) -> Option<CapturedDocumentRoot> {
    let DetachedChildBrowsingContextDocumentSnapshot {
        parent_frame_id,
        frame_id,
        owner_node_id,
        url,
        markup,
        scripting_enabled,
    } = snapshot;
    let dom_host = DomHost::from_dom(
        crate::parser::HtmlParser::with_scripting_enabled(scripting_enabled).parse(url, markup),
    );
    let root =
        live_document_node_snapshot(&dom_host, dom_host.dom().document_node_id(), -1, None, true)?;
    Some(CapturedDocumentRoot {
        frame_id,
        owner: Some(CapturedContentDocumentOwner {
            parent_frame_id,
            node_id: owner_node_id,
        }),
        root,
        style_source: CapturedDocumentStyleSource::DetachedMarkup,
    })
}

#[derive(Default)]
struct StringTable {
    strings: Vec<String>,
    indices: HashMap<String, usize>,
}

impl StringTable {
    fn intern(&mut self, value: impl AsRef<str>) -> usize {
        let value = value.as_ref();
        if let Some(index) = self.indices.get(value).copied() {
            return index;
        }
        let index = self.strings.len();
        self.strings.push(value.to_owned());
        self.indices.insert(value.to_owned(), index);
        index
    }

    fn into_strings(self) -> Vec<String> {
        self.strings
    }
}

#[derive(Default)]
struct NodeTableBuilder {
    parent_index: Vec<i64>,
    node_type: Vec<u8>,
    node_name: Vec<usize>,
    node_value: Vec<usize>,
    backend_node_id: Vec<u32>,
    has_unbound_backend_node_id: bool,
    attributes: Vec<Vec<usize>>,
    content_document_index_node_index: Vec<usize>,
    content_document_index_value: Vec<usize>,
    shadow_root_type_index: Vec<usize>,
    shadow_root_type_value: Vec<usize>,
    pseudo_type_index: Vec<usize>,
    pseudo_type_value: Vec<usize>,
    layout_nodes: Vec<(usize, DocumentSnapshotNodeId)>,
}

impl NodeTableBuilder {
    fn push_tree(
        &mut self,
        snapshot: &DocumentNodeSnapshot,
        parent_index: Option<usize>,
        frame_id: &str,
        content_document_indices: &ContentDocumentIndices,
        strings: &mut StringTable,
    ) {
        self.push_tree_with_budget(
            snapshot,
            parent_index,
            frame_id,
            content_document_indices,
            strings,
            MAX_DOM_OUTPUT_TREE_DEPTH,
        );
    }

    fn push_tree_with_budget(
        &mut self,
        snapshot: &DocumentNodeSnapshot,
        parent_index: Option<usize>,
        frame_id: &str,
        content_document_indices: &ContentDocumentIndices,
        strings: &mut StringTable,
        remaining_tree_depth: usize,
    ) {
        let mut stack = vec![(snapshot, parent_index, remaining_tree_depth)];
        while let Some((snapshot, parent_index, remaining_tree_depth)) = stack.pop() {
            let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
                continue;
            };
            let index = self.node_type.len();
            self.parent_index
                .push(parent_index.map(|index| index as i64).unwrap_or(-1));
            self.node_type.push(snapshot.node_type);
            self.node_name.push(strings.intern(&snapshot.node_name));
            self.node_value.push(strings.intern(&snapshot.node_value));
            if let Some(backend_node_id) = snapshot.backend_node_id {
                self.backend_node_id.push(backend_node_id);
            } else {
                self.has_unbound_backend_node_id = true;
            }
            self.attributes.push(
                snapshot
                    .attributes
                    .iter()
                    .flat_map(|attribute| {
                        [
                            strings.intern(&attribute.local_name),
                            strings.intern(&attribute.value),
                        ]
                    })
                    .collect(),
            );

            if let Some(content_document_index) = content_document_index_for_node(
                content_document_indices,
                frame_id,
                snapshot.node_id,
            ) {
                self.content_document_index_node_index.push(index);
                self.content_document_index_value
                    .push(content_document_index);
            }

            if let Some(shadow_root_type) = snapshot.shadow_root_type.as_deref() {
                self.shadow_root_type_index.push(index);
                self.shadow_root_type_value
                    .push(strings.intern(shadow_root_type));
            }
            if let Some(pseudo_type) = snapshot.pseudo_type.as_deref() {
                self.pseudo_type_index.push(index);
                self.pseudo_type_value.push(strings.intern(pseudo_type));
            }

            if snapshot.has_geometry {
                self.layout_nodes.push((index, snapshot.node_id));
            }

            for child in snapshot.children.iter().rev() {
                stack.push((child, Some(index), next_tree_depth));
            }
            for shadow_root in snapshot.shadow_roots.iter().rev() {
                stack.push((shadow_root, Some(index), next_tree_depth));
            }
            for pseudo_element in snapshot.pseudo_elements.iter().rev() {
                stack.push((pseudo_element, Some(index), next_tree_depth));
            }
        }
    }

    fn into_value(self) -> Value {
        let mut nodes = serde_json::Map::new();
        nodes.insert("parentIndex".to_owned(), json!(self.parent_index));
        nodes.insert("nodeType".to_owned(), json!(self.node_type));
        nodes.insert("nodeName".to_owned(), json!(self.node_name));
        nodes.insert("nodeValue".to_owned(), json!(self.node_value));
        if !self.has_unbound_backend_node_id && self.backend_node_id.len() == self.node_type.len() {
            nodes.insert("backendNodeId".to_owned(), json!(self.backend_node_id));
        }
        nodes.insert("attributes".to_owned(), json!(self.attributes));
        if !self.content_document_index_node_index.is_empty() {
            nodes.insert(
                "contentDocumentIndex".to_owned(),
                json!({
                    "index": self.content_document_index_node_index,
                    "value": self.content_document_index_value,
                }),
            );
        }
        if !self.shadow_root_type_index.is_empty() {
            nodes.insert(
                "shadowRootType".to_owned(),
                json!({
                    "index": self.shadow_root_type_index,
                    "value": self.shadow_root_type_value,
                }),
            );
        }
        if !self.pseudo_type_index.is_empty() {
            nodes.insert(
                "pseudoType".to_owned(),
                json!({
                    "index": self.pseudo_type_index,
                    "value": self.pseudo_type_value,
                }),
            );
        }
        Value::Object(nodes)
    }
}

fn content_document_index_for_node(
    content_document_indices: &ContentDocumentIndices,
    frame_id: &str,
    node_id: DocumentSnapshotNodeId,
) -> Option<usize> {
    content_document_indices
        .get(&node_id)?
        .iter()
        .find_map(|(parent_frame_id, index)| (parent_frame_id == frame_id).then_some(*index))
}

fn lightweight_snapshot_bounds() -> Vec<f64> {
    vec![0.0, 0.0, 1.0, 1.0]
}

fn document_type_values(root: &DocumentNodeSnapshot) -> (String, String) {
    find_document_type(root)
        .map(|doctype| {
            (
                doctype.public_id.clone().unwrap_or_default(),
                doctype.system_id.clone().unwrap_or_default(),
            )
        })
        .unwrap_or_default()
}

fn find_document_type(snapshot: &DocumentNodeSnapshot) -> Option<&DocumentNodeSnapshot> {
    let mut stack = vec![snapshot];
    while let Some(snapshot) = stack.pop() {
        if snapshot.document_type_name.is_some() {
            return Some(snapshot);
        }
        stack.extend(snapshot.shadow_roots.iter().rev());
        stack.extend(snapshot.children.iter().rev());
    }
    None
}

fn document_title(root: &DocumentNodeSnapshot) -> String {
    find_title_node(root).map(text_content).unwrap_or_default()
}

fn find_title_node(snapshot: &DocumentNodeSnapshot) -> Option<&DocumentNodeSnapshot> {
    let mut stack = vec![snapshot];
    while let Some(snapshot) = stack.pop() {
        if snapshot.is_element && snapshot.local_name.eq_ignore_ascii_case("title") {
            return Some(snapshot);
        }
        stack.extend(snapshot.shadow_roots.iter().rev());
        stack.extend(snapshot.children.iter().rev());
    }
    None
}

fn text_content(snapshot: &DocumentNodeSnapshot) -> String {
    let mut value = String::new();
    let mut stack = vec![snapshot];
    while let Some(snapshot) = stack.pop() {
        if snapshot.node_type == 3 {
            value.push_str(&snapshot.node_value);
        }
        stack.extend(snapshot.shadow_roots.iter().rev());
        stack.extend(snapshot.children.iter().rev());
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_page_types::DocumentNodeAssociatedSnapshot;
    use url::Url;

    fn snapshot_contains_element_id(snapshot: &DocumentNodeSnapshot, id: &str) -> bool {
        snapshot
            .attributes
            .iter()
            .any(|attribute| attribute.local_name == "id" && attribute.value == id)
            || snapshot
                .children
                .iter()
                .any(|child| snapshot_contains_element_id(child, id))
            || snapshot
                .shadow_roots
                .iter()
                .any(|root| snapshot_contains_element_id(root, id))
    }

    fn detached_noscript_document_root(scripting_enabled: bool) -> CapturedDocumentRoot {
        detached_document_root(DetachedChildBrowsingContextDocumentSnapshot {
            parent_frame_id: "parent".to_owned(),
            frame_id: "child".to_owned(),
            owner_node_id: crate::dom::native::NativeNodeId::new(7),
            url: Url::parse("https://detached-dom-snapshot.test/").expect("test URL"),
            markup: "<noscript><span id='fallback'></span></noscript>".to_owned(),
            scripting_enabled,
        })
        .expect("detached document root")
    }

    fn element_snapshot(index: usize, child: Option<DocumentNodeSnapshot>) -> DocumentNodeSnapshot {
        let children = child.into_iter().collect::<Vec<_>>();
        DocumentNodeSnapshot {
            node_id: DocumentSnapshotNodeId::new(index),
            parent_id: None,
            inspector_identity: None,
            inspector_parent_identity: None,
            frontend_node_id: None,
            parent_frontend_node_id: None,
            backend_node_id: Some(moli_page_types::RENDERER_BACKEND_NODE_ID_START + index as u32),
            frame_id: None,
            node_type: 1,
            node_name: "DIV".to_owned(),
            local_name: "div".to_owned(),
            node_value: String::new(),
            child_count: children.len(),
            document_url: "about:blank".to_owned(),
            base_url: "about:blank".to_owned(),
            namespace_uri: Some("http://www.w3.org/1999/xhtml".to_owned()),
            attributes: Vec::new(),
            document_type_name: None,
            public_id: None,
            system_id: None,
            is_element: true,
            has_geometry: false,
            shadow_root_type: None,
            shadow_roots: Vec::new(),
            pseudo_type: None,
            pseudo_elements: Vec::new(),
            associated: None,
            children,
        }
    }

    fn deep_snapshot(depth: usize) -> DocumentNodeSnapshot {
        let mut snapshot = element_snapshot(depth, None);
        for index in (0..depth).rev() {
            snapshot = element_snapshot(index, Some(snapshot));
        }
        snapshot
    }

    #[test]
    fn node_table_uses_renderer_backend_id_binding() {
        let snapshot = element_snapshot(7, None);
        let mut strings = StringTable::default();
        let mut builder = NodeTableBuilder::default();
        let content_document_indices = ContentDocumentIndices::new();

        builder.push_tree(
            &snapshot,
            None,
            "TID-test",
            &content_document_indices,
            &mut strings,
        );

        assert_eq!(
            builder.backend_node_id,
            vec![snapshot.backend_node_id.unwrap()]
        );
    }

    #[test]
    fn detached_dom_snapshot_reuses_document_scripting_policy() {
        let enabled = detached_noscript_document_root(true);
        assert!(
            !snapshot_contains_element_id(&enabled.root, "fallback"),
            "scripting-enabled noscript contents must remain text in DOMSnapshot fallback parsing"
        );

        let disabled = detached_noscript_document_root(false);
        assert!(
            snapshot_contains_element_id(&disabled.root, "fallback"),
            "scripting-disabled noscript contents must remain markup in DOMSnapshot fallback parsing"
        );
    }

    #[test]
    fn node_table_excludes_template_content_association() {
        let mut snapshot = element_snapshot(7, None);
        snapshot.node_name = "TEMPLATE".to_owned();
        snapshot.local_name = "template".to_owned();
        let mut content = element_snapshot(8, None);
        content.node_type = 11;
        content.node_name = "#document-fragment".to_owned();
        content.local_name.clear();
        content.is_element = false;
        snapshot.associated = Some(Box::new(DocumentNodeAssociatedSnapshot::TemplateContent(
            content,
        )));

        let mut strings = StringTable::default();
        let mut builder = NodeTableBuilder::default();
        let content_document_indices = ContentDocumentIndices::new();
        builder.push_tree(
            &snapshot,
            None,
            "TID-test",
            &content_document_indices,
            &mut strings,
        );

        let nodes = builder.into_value();
        assert_eq!(
            nodes["nodeType"],
            json!([1]),
            "Chromium DOMSnapshot does not add template content to its node table"
        );
    }

    #[test]
    fn node_table_omits_backend_id_column_for_unbound_snapshot_node() {
        let mut snapshot = element_snapshot(7, None);
        snapshot.backend_node_id = None;
        let mut strings = StringTable::default();
        let mut builder = NodeTableBuilder::default();
        let content_document_indices = ContentDocumentIndices::new();

        builder.push_tree(
            &snapshot,
            None,
            "TID-test",
            &content_document_indices,
            &mut strings,
        );

        let nodes = builder.into_value();
        assert!(
            nodes.get("backendNodeId").is_none(),
            "unbound one-shot snapshot nodes must not forge a backendNodeId column"
        );
    }

    #[test]
    fn node_table_truncates_deep_snapshot_at_tree_recursion_limit() {
        let snapshot = deep_snapshot(MAX_DOM_OUTPUT_TREE_DEPTH + 8);
        let mut strings = StringTable::default();
        let mut builder = NodeTableBuilder::default();
        let content_document_indices = ContentDocumentIndices::new();

        builder.push_tree(
            &snapshot,
            None,
            "TID-test",
            &content_document_indices,
            &mut strings,
        );

        assert_eq!(builder.node_type.len(), MAX_DOM_OUTPUT_TREE_DEPTH);
    }
}
