use super::super::ChildBrowsingContextNavigationRequest;
use super::*;
use crate::dom::native::Node;
use crate::native_bridge::element::parse_url_with_document_query_encoding;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildFrameOwnerElementKind {
    Iframe,
    Frame,
    Embed,
    Object,
}

impl ChildFrameOwnerElementKind {
    fn navigation_attribute(self) -> &'static str {
        match self {
            Self::Iframe | Self::Frame | Self::Embed => "src",
            Self::Object => "data",
        }
    }

    fn always_hosts_document(self) -> bool {
        matches!(self, Self::Iframe | Self::Frame)
    }
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn child_browsing_context_initial_live_bootstrap(
        attribute_bootstrap: &ChildBrowsingContextBootstrap,
    ) -> ChildBrowsingContextBootstrap {
        match attribute_bootstrap {
            ChildBrowsingContextBootstrap::Url(_)
            | ChildBrowsingContextBootstrap::Request(_)
            | ChildBrowsingContextBootstrap::Srcdoc { .. } => {
                ChildBrowsingContextBootstrap::AboutBlank
            }
            ChildBrowsingContextBootstrap::AboutBlank => attribute_bootstrap.clone(),
        }
    }

    pub(in crate::native_bridge::context_host) fn is_child_browsing_context_host_handle(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.child_frame_owner_element_kind(handle).is_some()
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_host_is_object_element(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.child_frame_owner_element_kind(handle) == Some(ChildFrameOwnerElementKind::Object)
    }

    fn child_frame_owner_element_kind(
        &self,
        handle: DomHandle,
    ) -> Option<ChildFrameOwnerElementKind> {
        [
            ("iframe", ChildFrameOwnerElementKind::Iframe),
            ("frame", ChildFrameOwnerElementKind::Frame),
            ("embed", ChildFrameOwnerElementKind::Embed),
            ("object", ChildFrameOwnerElementKind::Object),
        ]
        .into_iter()
        .find_map(|(name, kind)| {
            self.dom_host()
                .is_html_element_named(handle, name)
                .then_some(kind)
        })
    }

    pub(crate) fn frame_owner_attribute_requires_child_refresh(
        &self,
        handle: DomHandle,
        attribute_name: &str,
    ) -> bool {
        let Some(kind) = self.child_frame_owner_element_kind(handle) else {
            return false;
        };
        let common_identity = attribute_name.eq_ignore_ascii_case("name")
            || attribute_name.eq_ignore_ascii_case("id");
        match kind {
            ChildFrameOwnerElementKind::Iframe | ChildFrameOwnerElementKind::Frame => {
                common_identity
                    || ["src", "srcdoc", "credentialless"]
                        .into_iter()
                        .any(|name| attribute_name.eq_ignore_ascii_case(name))
            }
            ChildFrameOwnerElementKind::Embed | ChildFrameOwnerElementKind::Object => {
                common_identity
                    || attribute_name.eq_ignore_ascii_case(kind.navigation_attribute())
                    || attribute_name.eq_ignore_ascii_case("type")
            }
        }
    }

    pub(crate) fn frame_owner_navigation_attribute_matches(
        &self,
        handle: DomHandle,
        attribute_name: &str,
    ) -> bool {
        self.child_frame_owner_element_kind(handle)
            .is_some_and(|kind| attribute_name.eq_ignore_ascii_case(kind.navigation_attribute()))
    }

    pub(crate) fn clear_object_fallback_for_attribute_change(
        &mut self,
        handle: DomHandle,
        attribute_name: &str,
    ) {
        if self.child_browsing_context_host_is_object_element(handle)
            && ["data", "type"]
                .into_iter()
                .any(|name| attribute_name.eq_ignore_ascii_case(name))
        {
            self.object_fallback_bootstraps.remove(&handle);
        }
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_host_is_active(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.dom_host().is_connected(handle)
            || self
                .lightweight_popup_id_for_node_owner_document(handle)
                .is_some()
    }

    pub(in crate::native_bridge::context_host) fn document_base_url_for_child_context(
        &self,
        handle: DomHandle,
    ) -> Url {
        let document = if self.dom_host().node(handle).is_some_and(Node::is_document) {
            Some(handle)
        } else {
            self.dom_host().node(handle).and_then(Node::owner_document)
        };
        document
            .map(|document| self.document_base_url_for_handle(document))
            .or_else(|| self.dom_host().document_base_url())
            .unwrap_or_else(|| self.host_document().url().clone())
    }

    pub(crate) fn document_url_for_child_context(&self, handle: DomHandle) -> Url {
        let document = if self.dom_host().node(handle).is_some_and(Node::is_document) {
            Some(handle)
        } else {
            self.dom_host().node(handle).and_then(Node::owner_document)
        };
        document
            .map(|document| self.document_url_for_handle(document))
            .or_else(|| self.dom_host().document_url().cloned())
            .unwrap_or_else(|| self.host_document().url().clone())
    }

    pub(in crate::native_bridge::context_host) fn resolve_child_browsing_context_url(
        &self,
        handle: DomHandle,
        raw: &str,
    ) -> Url {
        let base = self.document_base_url_for_child_context(handle);
        Url::options()
            .base_url(Some(&base))
            .parse(raw)
            .unwrap_or_else(|_| Url::parse("about:blank").expect("static about:blank should parse"))
    }

    fn resolve_child_frame_owner_attribute_url(&self, handle: DomHandle, raw: &str) -> Url {
        let base = self.document_base_url_for_child_context(handle);
        parse_url_with_document_query_encoding(self, handle, &base, raw)
            .unwrap_or_else(|_| Url::parse("about:blank").expect("static about:blank should parse"))
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_bootstrap_for_handle(
        &self,
        handle: DomHandle,
    ) -> Option<ChildBrowsingContextBootstrap> {
        let kind = self.child_frame_owner_element_kind(handle)?;
        if !self.child_browsing_context_host_is_active(handle) {
            return None;
        }
        let owner_dispatch_scope = self
            .owner_dispatch_scope_for_node(handle)
            .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top);
        if self
            .document_resource_loader_for_dispatch_scope(owner_dispatch_scope)
            .is_some_and(|loader| !loader.request_client().subframe_loading_enabled())
        {
            return None;
        }

        if matches!(
            kind,
            ChildFrameOwnerElementKind::Iframe | ChildFrameOwnerElementKind::Frame
        ) && let Some(srcdoc) = self.dom_host().get_attribute(handle, "srcdoc")
        {
            return Some(ChildBrowsingContextBootstrap::Srcdoc {
                base_url: self.document_base_url_for_child_context(handle),
                markup: srcdoc,
            });
        }

        let raw_url = self
            .dom_host()
            .get_attribute(handle, kind.navigation_attribute())
            .unwrap_or_default();
        let raw_url = raw_url.trim();
        if raw_url.is_empty() {
            return kind
                .always_hosts_document()
                .then_some(ChildBrowsingContextBootstrap::AboutBlank);
        }

        let url = self.resolve_child_frame_owner_attribute_url(handle, raw_url);
        if !kind.always_hosts_document()
            && (self.embedded_content_is_inside_media_element(handle)
                || !self.embedded_content_selects_nested_document(handle, raw_url, &url))
        {
            return None;
        }
        let bootstrap = if !kind.always_hosts_document() && url.scheme() == "javascript" {
            // embed/object setup fetches its attribute resource. Preserve that
            // distinction from iframe/frame navigation so Fetch rejects the
            // javascript scheme instead of executing it in the child realm.
            ChildBrowsingContextBootstrap::Request(ChildBrowsingContextNavigationRequest {
                url,
                method: "GET".to_owned(),
                body: None,
                request_headers: Vec::new(),
            })
        } else {
            ChildBrowsingContextBootstrap::Url(url)
        };
        if kind == ChildFrameOwnerElementKind::Object
            && self.object_fallback_bootstraps.get(&handle) == Some(&bootstrap)
        {
            return None;
        }
        Some(bootstrap)
    }

    fn embedded_content_is_inside_media_element(&self, handle: DomHandle) -> bool {
        let mut current = self.dom_host().parent_node(handle);
        while let Some(ancestor) = current {
            if self.dom_host().is_html_element_named(ancestor, "audio")
                || self.dom_host().is_html_element_named(ancestor, "video")
            {
                return true;
            }
            current = self.dom_host().parent_node(ancestor);
        }
        false
    }

    fn embedded_content_selects_nested_document(
        &self,
        handle: DomHandle,
        raw_url: &str,
        url: &Url,
    ) -> bool {
        let declared_type = self
            .dom_host()
            .get_attribute(handle, "type")
            .filter(|value| !value.trim().is_empty());
        let mime = match declared_type {
            Some(value) => moli_web_mime::mime_essence(value.trim()),
            None => moli_web_mime::resource_mime_essence_for_url(raw_url, url.path()),
        };
        let Some(mime) = mime else {
            // Blink also defaults an unclassified plug-in element URL to a frame.
            return true;
        };
        if moli_web_mime::is_image_mime(&mime) {
            return false;
        }
        moli_web_mime::is_html_document_mime(&mime)
            || moli_web_mime::is_dom_parser_xml_mime(&mime)
            || moli_web_mime::is_text_mime(&mime)
            || moli_web_mime::is_json_module_mime(&mime)
            || moli_web_mime::is_javascript_mime(&mime)
    }

    pub(in crate::native_bridge::context_host) fn collect_child_browsing_context_host_handles(
        &self,
        root: DomHandle,
        out: &mut Vec<DomHandle>,
    ) {
        if self.dom_host().is_connected(root) {
            out.extend(
                self.dom_host()
                    .child_browsing_context_host_candidate_handles_in_subtree_in_document_order(
                        root,
                    ),
            );
            return;
        }
        if self.is_child_browsing_context_host_handle(root) {
            out.push(root);
        }
        for child in self.dom_host().child_handles(root) {
            self.collect_child_browsing_context_host_handles(child, out);
        }
    }

    pub(in crate::native_bridge::context_host) fn next_child_browsing_context_frame_id(
        &mut self,
    ) -> String {
        let next_id = self.next_child_browsing_context_id;
        self.next_child_browsing_context_id += 1;
        format!("child-browsing-context-{next_id}")
    }
}
