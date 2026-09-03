use super::super::JsContextHost;
use crate::{
    document_runtime::{DocumentPolicyContainer, DomHandle},
    frame_owner_model::{FrameDocumentLoadDeliveryTask, FrameLocalWindowOwnerTransition},
};
use url::Url;

/// Inputs captured from the frame creator before installing the internal
/// initial-empty Document.
///
/// This is intentionally not a `ChildBrowsingContextSnapshot`: initialization
/// has no response body, parser input, or navigation completion to restore.
#[derive(Clone, Debug)]
pub(in crate::native_bridge::context_host) struct ChildInitialEmptyDocumentInit {
    creator_base_url: Url,
    inherited_origin: String,
    policy_container: DocumentPolicyContainer,
    resource_authority: crate::network::context::DocumentResourceLoader,
}

pub(in crate::native_bridge::context_host) struct ChildInitialEmptyDocumentInitialization {
    pub(in crate::native_bridge::context_host) suppressed_load_delivery:
        FrameDocumentLoadDeliveryTask,
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn initial_child_about_blank_policy_container_from_parent(
        &self,
        handle: DomHandle,
    ) -> DocumentPolicyContainer {
        let mut policy_container = self
            .child_browsing_context_parent_handle(handle)
            .and_then(|parent| {
                self.child_browsing_contexts
                    .get(&parent)
                    .map(|entry| entry.document_policy_container_snapshot())
            })
            .unwrap_or_else(|| self.document_policy_container().clone());
        if let Some(parent_document) = self
            .dom_host()
            .node(handle)
            .and_then(crate::dom::native::Node::owner_document)
            && let Some(permissions_policy) =
                self.document_permissions_policy_for_document_handle(parent_document)
        {
            policy_container.permissions_policy = permissions_policy;
        }
        policy_container.document_referrer =
            self.document_url_for_child_context(handle).to_string();
        policy_container
    }

    pub(in crate::native_bridge::context_host) fn capture_child_initial_empty_document_init(
        &self,
        handle: DomHandle,
        policy_container: DocumentPolicyContainer,
    ) -> ChildInitialEmptyDocumentInit {
        let resource_authority = self
            .parent_document_resource_loader_for_child_context(handle)
            .expect("initial-empty child Document requires its exact parent authority")
            .clone();
        let inherited_origin = if policy_container.sandbox.forces_opaque_origin {
            "null".to_owned()
        } else {
            self.child_browsing_context_creator_network_partition_origin(handle)
        };
        ChildInitialEmptyDocumentInit {
            creator_base_url: self.document_base_url_for_child_context(handle),
            inherited_origin,
            policy_container,
            resource_authority,
        }
    }

    /// Initializes a newly created child frame with its internal empty
    /// Document. This is the local equivalent of Blink's `FrameLoader::Init`:
    /// it installs owner state and a complete empty tree without entering the
    /// navigation snapshot/parser pipeline or materializing a V8 realm.
    pub(in crate::native_bridge::context_host) fn initialize_child_frame_with_initial_empty_document(
        &mut self,
        handle: DomHandle,
        init: ChildInitialEmptyDocumentInit,
    ) -> ChildInitialEmptyDocumentInitialization {
        assert!(
            self.child_browsing_contexts.contains_key(&handle),
            "initial-empty Document initialization requires an installed child browsing context"
        );
        let preflight = self.capture_child_document_window_commit_preflight(handle);
        assert!(
            preflight.has_no_committed_document(),
            "initial-empty Document initialization requires a vacant child owner slot"
        );
        let loader_id = self.allocate_child_document_loader_id();
        let document_url = Url::parse("about:blank").expect("static about:blank must parse");
        let document_handle =
            self.create_empty_live_child_html_document(document_url.clone(), Some("text/html"));
        self.dom_host_mut()
            .set_document_fallback_base_url_for_handle(
                document_handle,
                Some(init.creator_base_url),
            );
        let document_base_url = self.document_base_url_for_handle(document_handle);
        let document_origin = init.inherited_origin.clone();
        let referrer_policy = init.policy_container.referrer_policy.clone();
        let subresource_policy_context =
            crate::types::SubresourcePolicyContext::from_document_policy(&init.policy_container);
        let owner_transition = self
            .frame_owner_store
            .initialize_child_frame_document(
                handle,
                document_handle,
                document_url,
                document_base_url,
                init.inherited_origin,
                referrer_policy,
                moli_fetch::RequestCredentialsMode::SameOrigin,
                init.policy_container,
                subresource_policy_context,
            )
            .expect("an ensured child frame must accept exactly one initial-empty Document");
        debug_assert!(matches!(
            owner_transition.local_window_owner_transition(),
            FrameLocalWindowOwnerTransition::Installed { .. }
        ));
        let current_owner = owner_transition
            .current_owner()
            .expect("initial-empty frame initialization must install an owner");
        self.register_committed_document_resource_loader(
            crate::network::context::DocumentFetchContext::new(
                crate::native_bridge::WindowDocumentOwner::Frame(current_owner),
                self.document_url_for_handle(document_handle),
                self.document_base_url_for_handle(document_handle),
                document_origin,
            ),
            crate::network::context::DocumentResourceAuthoritySource::Inherited(
                init.resource_authority,
            ),
        );
        self.child_browsing_contexts
            .get_mut(&handle)
            .expect("initial-empty owner must retain its browsing context")
            .set_current_document_loader_id(loader_id);
        let replaced_document_handle = self
            .child_browsing_context_document_handles
            .insert(handle, document_handle);
        assert!(
            replaced_document_handle.is_none(),
            "initial-empty Document initialization must not replace an adapter document handle"
        );

        self.populate_initial_empty_child_html_document(document_handle);
        self.dom_host_mut()
            .mark_subtree_connected_preserving_owner_document(document_handle);
        self.install_empty_child_classic_script_runner_for_current_document(
            handle,
            current_owner.local_window_id,
            current_owner.document_id,
        );

        // CancelParsing() in Blink leaves the internal Document complete while
        // FrameLoader::Init suppresses its finish notification. Keep the exact
        // ready load token local so the iframe connection algorithm can either
        // expose a synchronous about:blank load or discard it for a pending
        // navigation.
        let suppressed_load_delivery = self
            .frame_owner_store
            .complete_current_child_initial_empty_document(handle)
            .expect("new initial-empty Document must complete exactly once");

        ChildInitialEmptyDocumentInitialization {
            suppressed_load_delivery,
        }
    }

    pub(in crate::native_bridge::context_host) fn suppress_ready_child_initial_empty_load(
        &mut self,
        task: FrameDocumentLoadDeliveryTask,
    ) -> bool {
        self.frame_owner_store
            .suppress_current_child_initial_empty_load_delivery(task)
    }

    fn populate_initial_empty_child_html_document(&mut self, document_handle: DomHandle) {
        self.dom_host_mut()
            .set_html_quirks_mode_for_parser_document(
                document_handle,
                html5ever::tree_builder::QuirksMode::Quirks,
            );
        let html = self
            .dom_host_mut()
            .create_parser_element_without_attributes_for_document(
                document_handle,
                "html".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
        let head = self
            .dom_host_mut()
            .create_parser_element_without_attributes_for_document(
                document_handle,
                "head".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
        let body = self
            .dom_host_mut()
            .create_parser_element_without_attributes_for_document(
                document_handle,
                "body".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
        assert!(self.dom_host_mut().append_child(document_handle, html));
        assert!(self.dom_host_mut().append_child(html, head));
        assert!(self.dom_host_mut().append_child(html, body));
    }
}
