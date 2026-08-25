use super::super::{
    ChildBrowsingContextBootstrap, ChildBrowsingContextSnapshot, JsContextHost,
    child_frames::WebStorageScope,
};
use super::{
    ChildBrowsingContextFrameSnapshot, DetachedChildBrowsingContextDocumentSnapshot,
    MAX_CHILD_FRAME_TREE_DEPTH,
    frame_tree::child_browsing_context_bootstrap_url_inherits_security_origin,
};
use crate::document_runtime::DomHandle;
use url::Url;

impl JsContextHost {
    pub(crate) fn detached_child_browsing_context_document_snapshots_for_dom_snapshot(
        &mut self,
        top_frame_id: &str,
    ) -> Vec<DetachedChildBrowsingContextDocumentSnapshot> {
        let handles = self.child_browsing_context_handles_in_document_order();
        let mut snapshots = Vec::new();
        for handle in handles {
            if self
                .child_browsing_context_document_handle(handle)
                .is_some()
            {
                continue;
            }
            let owner_document = self
                .dom_host()
                .node(handle)
                .and_then(|node| node.owner_document());
            let parent_frame_id = if owner_document == Some(self.document_handle()) {
                top_frame_id.to_owned()
            } else {
                self.child_browsing_context_parent_frame_id(handle)
                    .unwrap_or_else(|| top_frame_id.to_owned())
            };
            let Some(frame_id) = self.child_browsing_context_frame_id_by_owner_node_id(handle)
            else {
                continue;
            };
            let Some(snapshot) = self.child_browsing_context_snapshot_markup(handle) else {
                continue;
            };
            let scripting_enabled = self.child_browsing_context_scripting_enabled(handle);
            let parent_scope = self
                .child_browsing_context_web_storage_scope(
                    handle,
                    &moli_url::origin_ascii_serialization(self.document_url()),
                )
                .unwrap_or_else(|| self.top_web_storage_scope());
            snapshots.push(DetachedChildBrowsingContextDocumentSnapshot {
                parent_frame_id: parent_frame_id.clone(),
                frame_id: frame_id.clone(),
                owner_node_id: handle,
                url: snapshot.url.clone(),
                markup: snapshot.markup.clone(),
                scripting_enabled,
            });
            snapshots.extend(
                self.detached_child_browsing_context_document_snapshots_in_markup(
                    &frame_id,
                    &snapshot.url,
                    &snapshot.markup,
                    scripting_enabled,
                    0,
                    parent_scope,
                ),
            );
        }
        snapshots
    }

    fn detached_child_browsing_context_document_snapshots_in_markup(
        &mut self,
        parent_frame_id: &str,
        document_url: &Url,
        markup: &str,
        scripting_enabled: bool,
        depth: usize,
        parent_scope: WebStorageScope,
    ) -> Vec<DetachedChildBrowsingContextDocumentSnapshot> {
        if depth >= MAX_CHILD_FRAME_TREE_DEPTH {
            return Vec::new();
        }

        let document = crate::parser::HtmlParser::with_scripting_enabled(scripting_enabled)
            .parse(document_url.clone(), markup.to_owned());
        let document_base_url = document
            .document()
            .map(|doc| doc.base_url().clone())
            .unwrap_or_else(|| document_url.clone());
        self.collect_detached_child_browsing_context_document_snapshots(
            &document,
            document.document_node_id(),
            &document_base_url,
            parent_frame_id,
            scripting_enabled,
            depth,
            parent_scope,
        )
    }

    fn collect_detached_child_browsing_context_document_snapshots(
        &mut self,
        document: &crate::dom::native::NativeDom,
        root: DomHandle,
        document_base_url: &Url,
        parent_frame_id: &str,
        document_scripting_enabled: bool,
        depth: usize,
        parent_scope: WebStorageScope,
    ) -> Vec<DetachedChildBrowsingContextDocumentSnapshot> {
        let mut snapshots = Vec::new();
        let mut ordinal = 0usize;
        let mut pending = document.child_ids(root).collect::<Vec<_>>();
        pending.reverse();
        while let Some(child) = pending.pop() {
            if self.is_detached_child_browsing_context_host_handle(document, child) {
                ordinal += 1;
                let frame_id = format!("{parent_frame_id}/child-{ordinal}");
                let bootstrap = Self::detached_child_browsing_context_bootstrap(
                    document,
                    child,
                    document_base_url,
                );
                let Some(snapshot) = self
                    .detached_child_browsing_context_snapshot_markup(&bootstrap, document_base_url)
                else {
                    continue;
                };
                let security_origin_inherited =
                    child_browsing_context_bootstrap_url_inherits_security_origin(
                        &bootstrap,
                        &snapshot.url,
                    );
                let parent_scope = self.detached_child_frame_web_storage_scope(
                    document,
                    child,
                    &snapshot.url,
                    security_origin_inherited,
                    parent_scope.clone(),
                );
                let scripting_enabled = Self::detached_child_document_scripting_enabled(
                    document,
                    child,
                    document_scripting_enabled,
                    &snapshot,
                );
                snapshots.push(DetachedChildBrowsingContextDocumentSnapshot {
                    parent_frame_id: parent_frame_id.to_owned(),
                    frame_id: frame_id.clone(),
                    owner_node_id: child,
                    url: snapshot.url.clone(),
                    markup: snapshot.markup.clone(),
                    scripting_enabled,
                });
                snapshots.extend(
                    self.detached_child_browsing_context_document_snapshots_in_markup(
                        &frame_id,
                        &snapshot.url,
                        &snapshot.markup,
                        scripting_enabled,
                        depth + 1,
                        parent_scope,
                    ),
                );
                continue;
            }
            let child_ids = document.child_ids(child).collect::<Vec<_>>();
            pending.extend(child_ids.into_iter().rev());
        }
        snapshots
    }

    pub(in crate::native_bridge::context_host::child_frame_snapshots) fn detached_child_browsing_context_frame_tree_snapshot(
        &mut self,
        parent_frame_id: &str,
        document_url: &Url,
        markup: &str,
        scripting_enabled: bool,
        depth: usize,
        parent_scope: WebStorageScope,
    ) -> Vec<ChildBrowsingContextFrameSnapshot> {
        if depth >= MAX_CHILD_FRAME_TREE_DEPTH {
            return Vec::new();
        }

        let document = crate::parser::HtmlParser::with_scripting_enabled(scripting_enabled)
            .parse(document_url.clone(), markup.to_owned());
        let document_base_url = document
            .document()
            .map(|doc| doc.base_url().clone())
            .unwrap_or_else(|| document_url.clone());
        self.collect_detached_child_frame_tree_snapshots(
            &document,
            document.document_node_id(),
            &document_base_url,
            parent_frame_id,
            scripting_enabled,
            depth,
            parent_scope,
        )
    }

    fn collect_detached_child_frame_tree_snapshots(
        &mut self,
        document: &crate::dom::native::NativeDom,
        root: DomHandle,
        document_base_url: &Url,
        parent_frame_id: &str,
        document_scripting_enabled: bool,
        depth: usize,
        parent_scope: WebStorageScope,
    ) -> Vec<ChildBrowsingContextFrameSnapshot> {
        let mut snapshots = Vec::new();
        let mut pending = document.child_ids(root).collect::<Vec<_>>();
        pending.reverse();
        while let Some(child) = pending.pop() {
            if self.is_detached_child_browsing_context_host_handle(document, child) {
                snapshots.push(self.detached_child_frame_snapshot_for_handle(
                    document,
                    child,
                    document_base_url,
                    parent_frame_id,
                    snapshots.len() + 1,
                    document_scripting_enabled,
                    depth,
                    parent_scope.clone(),
                ));
                continue;
            }
            let child_ids = document.child_ids(child).collect::<Vec<_>>();
            pending.extend(child_ids.into_iter().rev());
        }
        snapshots
    }

    fn detached_child_frame_snapshot_for_handle(
        &mut self,
        document: &crate::dom::native::NativeDom,
        handle: DomHandle,
        document_base_url: &Url,
        parent_frame_id: &str,
        ordinal: usize,
        document_scripting_enabled: bool,
        depth: usize,
        parent_scope: WebStorageScope,
    ) -> ChildBrowsingContextFrameSnapshot {
        let name = document
            .get_attribute(handle, "name")
            .filter(|value| !value.is_empty());
        let owner_element_id = document
            .get_attribute(handle, "id")
            .filter(|value| !value.is_empty());
        let bootstrap =
            Self::detached_child_browsing_context_bootstrap(document, handle, document_base_url);
        let url = Self::child_browsing_context_navigation_entry_url(&bootstrap)
            .unwrap_or_else(|| Url::parse("about:blank").expect("static about:blank should parse"));
        let security_origin_inherited =
            child_browsing_context_bootstrap_url_inherits_security_origin(&bootstrap, &url);
        let security_origin_opaque =
            document
                .get_attribute(handle, "sandbox")
                .is_some_and(|sandbox| {
                    super::super::child_frames::sandbox_attribute_forces_opaque_origin(&sandbox)
                })
                || (!security_origin_inherited && url.scheme() == "data");
        let storage_scope = self.detached_child_frame_web_storage_scope(
            document,
            handle,
            &url,
            security_origin_inherited,
            parent_scope,
        );
        let storage_key = storage_scope.storage_key().serialized_storage_key();
        let frame_id = format!("{parent_frame_id}/child-{ordinal}");
        let child_frames = self
            .detached_child_browsing_context_snapshot_markup(&bootstrap, document_base_url)
            .map(|snapshot| {
                let scripting_enabled = Self::detached_child_document_scripting_enabled(
                    document,
                    handle,
                    document_scripting_enabled,
                    &snapshot,
                );
                self.detached_child_browsing_context_frame_tree_snapshot(
                    &frame_id,
                    &snapshot.url,
                    &snapshot.markup,
                    scripting_enabled,
                    depth + 1,
                    storage_scope.clone(),
                )
            })
            .unwrap_or_default();
        ChildBrowsingContextFrameSnapshot {
            loader_id: format!("LID-DETACHED-{frame_id}"),
            frame_id,
            name,
            owner_element_id,
            url: url.as_str().to_owned(),
            storage_key,
            security_origin_inherited,
            security_origin_opaque,
            child_frames,
        }
    }

    pub(in crate::native_bridge::context_host::child_frame_snapshots) fn detached_child_document_scripting_enabled(
        snapshot_document: &crate::dom::native::NativeDom,
        synthetic_owner: DomHandle,
        parent_scripting_enabled: bool,
        snapshot: &ChildBrowsingContextSnapshot,
    ) -> bool {
        // Live child Documents do not reach this helper: their snapshot path uses the
        // sandbox policy captured in `ChildBrowsingContextEntry` at commit time. This
        // owner belongs to immutable navigation markup that was reparsed only to
        // discover a synthetic nested frame, so its sandbox attribute is the policy
        // input for that synthetic Document rather than a mutable live owner attribute.
        parent_scripting_enabled
            && super::super::child_frames::document_sandbox_policy_from_attribute(
                snapshot_document
                    .get_attribute(synthetic_owner, "sandbox")
                    .as_deref(),
            )
            .allows_scripts
            && snapshot.policy_container.sandbox.allows_scripts
    }

    pub(in crate::native_bridge::context_host::child_frame_snapshots) fn is_detached_child_browsing_context_host_handle(
        &self,
        document: &crate::dom::native::NativeDom,
        handle: DomHandle,
    ) -> bool {
        document.is_html_element_named(handle, "iframe")
            || document.is_html_element_named(handle, "frame")
    }

    pub(in crate::native_bridge::context_host::child_frame_snapshots) fn detached_child_browsing_context_bootstrap(
        document: &crate::dom::native::NativeDom,
        handle: DomHandle,
        document_base_url: &Url,
    ) -> ChildBrowsingContextBootstrap {
        if let Some(srcdoc) = document.get_attribute(handle, "srcdoc") {
            return ChildBrowsingContextBootstrap::Srcdoc {
                base_url: document_base_url.clone(),
                markup: srcdoc,
            };
        }

        let src = document.get_attribute(handle, "src").unwrap_or_default();
        if !src.is_empty() {
            return ChildBrowsingContextBootstrap::Url(
                Url::options()
                    .base_url(Some(document_base_url))
                    .parse(&src)
                    .unwrap_or_else(|_| {
                        Url::parse("about:blank").expect("static about:blank should parse")
                    }),
            );
        }

        ChildBrowsingContextBootstrap::AboutBlank
    }

    pub(in crate::native_bridge::context_host::child_frame_snapshots) fn detached_child_browsing_context_snapshot_markup(
        &mut self,
        bootstrap: &ChildBrowsingContextBootstrap,
        document_base_url: &Url,
    ) -> Option<ChildBrowsingContextSnapshot> {
        match bootstrap {
            ChildBrowsingContextBootstrap::AboutBlank => Some(
                ChildBrowsingContextSnapshot::about_blank(document_base_url.clone()),
            ),
            ChildBrowsingContextBootstrap::Srcdoc { base_url, markup } => {
                Some(ChildBrowsingContextSnapshot::srcdoc(
                    base_url.clone(),
                    markup.clone(),
                    self.document_character_set().to_owned(),
                ))
            }
            ChildBrowsingContextBootstrap::Url(url) => self
                .materialize_local_child_snapshot_for_url(url)
                .map(|snapshot| self.apply_page_csp_bypass_to_child_snapshot(snapshot)),
            ChildBrowsingContextBootstrap::Request(_) => None,
        }
    }
}
