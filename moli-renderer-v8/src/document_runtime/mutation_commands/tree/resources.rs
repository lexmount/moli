use super::super::details::DetailsInsertionPlan;
use super::{insertion_plan::TreeInsertionPlan, removal::TreeRemovalPlan};
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    native_bridge::JsContextHost,
};

fn push_unique_handle(handles: &mut Vec<DomHandle>, handle: DomHandle) {
    if !handles.contains(&handle) {
        handles.push(handle);
    }
}

fn preorder_stack(roots: &[DomHandle]) -> Vec<DomHandle> {
    roots.iter().rev().copied().collect()
}

fn element_owns_image_resource(element: &crate::dom::native::Element) -> bool {
    crate::native_bridge::element::ImageResourceElementKind::for_element(element).is_some()
}

#[derive(Default)]
pub(super) struct ImageRelevantMutationPlan {
    pub(super) pictures: Vec<DomHandle>,
    pub(super) images: Vec<DomHandle>,
}

#[derive(Default)]
pub(super) struct MediaRelevantMutationPlan {
    pub(super) media: Vec<DomHandle>,
}

#[derive(Default)]
pub(super) struct InsertionSubtreePlan {
    pub(super) node_count: usize,
    pub(super) may_have_image_resources: bool,
    pub(super) may_have_image_relevant_picture_source: bool,
    pub(super) may_have_lazy_media: bool,
    pub(super) may_have_media_sources: bool,
    pub(super) may_have_text_tracks: bool,
    pub(super) may_have_options: bool,
    pub(super) may_have_nonce: bool,
    pub(super) details: DetailsInsertionPlan,
}

impl DocumentRuntime {
    pub(super) fn queue_tree_insertion_resource_followups(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        insertion_plan: &TreeInsertionPlan<'_>,
        request_initiator_type: crate::types::SubresourceRequestInitiatorType,
    ) {
        if insertion_plan.subtree_plan.may_have_image_resources {
            self.queue_inserted_image_resource_loads(
                scope,
                host_ptr,
                insertion_plan.insertion_roots,
                request_initiator_type,
            );
        }
        self.queue_image_relevant_mutation_loads(
            scope,
            host_ptr,
            &insertion_plan.image_relevant_mutation_plan,
            request_initiator_type,
        );
        if insertion_plan.subtree_plan.may_have_lazy_media
            || insertion_plan.subtree_plan.may_have_media_sources
        {
            self.queue_inserted_media_loads(
                scope,
                host_ptr,
                insertion_plan.insertion_roots,
                &insertion_plan.media_relevant_mutation_plan,
            );
        }
        if insertion_plan.subtree_plan.may_have_text_tracks {
            self.queue_inserted_text_track_loads(scope, host_ptr, insertion_plan.insertion_roots);
        }
    }

    fn queue_inserted_image_resource_loads(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
        request_initiator_type: crate::types::SubresourceRequestInitiatorType,
    ) {
        if let [root] = roots
            && self.dom_host.first_child(*root).is_none()
        {
            if self
                .dom_host
                .node(*root)
                .and_then(Node::as_element)
                .is_some_and(element_owns_image_resource)
            {
                crate::native_bridge::element::queue_image_load_event_if_needed_with_initiator(
                    scope,
                    host_ptr,
                    *root,
                    request_initiator_type,
                );
            }
            return;
        }
        let mut stack = preorder_stack(roots);
        let mut image_elements = Vec::new();
        while let Some(handle) = stack.pop() {
            if self
                .dom_host
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(element_owns_image_resource)
            {
                image_elements.push(handle);
            }
            self.push_child_handles(&mut stack, handle);
        }
        for image in image_elements {
            crate::native_bridge::element::queue_image_load_event_if_needed_with_initiator(
                scope,
                host_ptr,
                image,
                request_initiator_type,
            );
        }
    }

    pub(super) fn insertion_subtree_plan(&self, roots: &[DomHandle]) -> InsertionSubtreePlan {
        let mut plan = InsertionSubtreePlan::default();
        let mut stack = preorder_stack(roots);
        while let Some(handle) = stack.pop() {
            plan.node_count += 1;
            if let Some(element) = self.dom_host.node(handle).and_then(Node::as_element) {
                plan.details.observe_element(handle, element);
                match element.local_name() {
                    "img" => {
                        plan.may_have_image_resources = true;
                        plan.may_have_image_relevant_picture_source = true;
                    }
                    "embed" | "object" => {
                        plan.may_have_image_resources = true;
                    }
                    "source" => {
                        plan.may_have_image_relevant_picture_source = true;
                        plan.may_have_media_sources = true;
                    }
                    "audio" | "video" => {
                        plan.may_have_lazy_media = true;
                    }
                    "track" => {
                        plan.may_have_text_tracks = true;
                    }
                    "option" => {
                        plan.may_have_options = true;
                    }
                    _ => {}
                }
                if element.attribute("nonce").is_some() {
                    plan.may_have_nonce = true;
                }
            }
            self.push_child_handles(&mut stack, handle);
        }
        plan
    }

    pub(super) fn image_relevant_mutation_plan_before_insert(
        &self,
        new_parent: DomHandle,
        roots: &[DomHandle],
    ) -> ImageRelevantMutationPlan {
        let mut plan = ImageRelevantMutationPlan::default();
        let new_parent_is_picture = self.dom_host.is_html_element_named(new_parent, "picture");
        for &root in roots {
            let root_is_img = self.dom_host.is_html_element_named(root, "img");
            let root_is_source = self.dom_host.is_html_element_named(root, "source");
            if !root_is_img && !root_is_source {
                continue;
            }
            if new_parent_is_picture {
                push_unique_handle(&mut plan.pictures, new_parent);
            }
            let old_parent = self.dom_host.parent_node(root);
            if let Some(old_parent) = old_parent
                && self.dom_host.is_html_element_named(old_parent, "picture")
            {
                push_unique_handle(&mut plan.pictures, old_parent);
            }
            if root_is_img
                && (new_parent_is_picture
                    || old_parent.is_some_and(|parent| {
                        self.dom_host.is_html_element_named(parent, "picture")
                    }))
            {
                push_unique_handle(&mut plan.images, root);
            }
        }
        plan
    }

    pub(super) fn media_relevant_mutation_plan_before_insert(
        &self,
        new_parent: DomHandle,
        roots: &[DomHandle],
    ) -> MediaRelevantMutationPlan {
        let mut plan = MediaRelevantMutationPlan::default();
        for &root in roots {
            if !self.dom_host.is_html_element_named(root, "source") {
                continue;
            }
            if self.dom_host.is_html_element_named(new_parent, "audio")
                || self.dom_host.is_html_element_named(new_parent, "video")
            {
                push_unique_handle(&mut plan.media, new_parent);
            }
            if let Some(old_parent) = self.dom_host.parent_node(root)
                && (self.dom_host.is_html_element_named(old_parent, "audio")
                    || self.dom_host.is_html_element_named(old_parent, "video"))
            {
                push_unique_handle(&mut plan.media, old_parent);
            }
        }
        plan
    }

    fn queue_image_relevant_mutation_loads(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        plan: &ImageRelevantMutationPlan,
        request_initiator_type: crate::types::SubresourceRequestInitiatorType,
    ) {
        let mut images = plan.images.clone();
        for &picture in &plan.pictures {
            for child in self.dom_host.child_handles(picture) {
                if self.dom_host.is_html_element_named(child, "img") {
                    push_unique_handle(&mut images, child);
                }
            }
        }
        let runtime = unsafe { &mut *host_ptr };
        for image in images {
            crate::native_bridge::element::reset_image_load_dispatch(runtime, image);
            crate::native_bridge::element::queue_image_load_event_if_needed_with_initiator(
                scope,
                host_ptr,
                image,
                request_initiator_type,
            );
        }
    }

    fn queue_inserted_media_loads(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
        mutation_plan: &MediaRelevantMutationPlan,
    ) {
        let mut stack = preorder_stack(roots);
        let mut media = Vec::new();
        while let Some(handle) = stack.pop() {
            if self
                .dom_host
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| {
                    element.is_html_element("audio") || element.is_html_element("video")
                })
            {
                push_unique_handle(&mut media, handle);
            } else if self.dom_host.is_html_element_named(handle, "source")
                && let Some(parent) = self.dom_host.parent_node(handle)
                && (self.dom_host.is_html_element_named(parent, "audio")
                    || self.dom_host.is_html_element_named(parent, "video"))
            {
                push_unique_handle(&mut media, parent);
            }
            self.push_child_handles(&mut stack, handle);
        }
        for &handle in &mutation_plan.media {
            push_unique_handle(&mut media, handle);
        }
        for handle in media {
            if mutation_plan.media.contains(&handle) {
                crate::native_bridge::element::queue_media_load_if_source_or_loading_change(
                    scope, host_ptr, handle, "src",
                );
            } else {
                crate::native_bridge::element::queue_media_load_if_needed(scope, host_ptr, handle);
            }
        }
    }

    pub(super) fn queue_tree_removal_resource_followups(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        removal_plan: &TreeRemovalPlan,
    ) {
        let mut handles = Vec::new();
        self.collect_subtree_handles_preorder(removal_plan.root, &mut handles);
        for handle in handles {
            if self
                .dom_host
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(element_owns_image_resource)
            {
                crate::native_bridge::element::queue_image_load_event_if_needed(
                    scope, host_ptr, handle,
                );
            } else if self.dom_host.is_html_element_named(handle, "track") {
                let followup =
                    unsafe { &mut *host_ptr }.cancel_pending_text_track_load_sequence(handle, true);
                crate::native_bridge::element::queue_media_canplay_after_text_tracks(
                    scope, host_ptr, followup,
                );
            }
        }

        if self
            .dom_host
            .is_html_element_named(removal_plan.root, "source")
            && (self
                .dom_host
                .is_html_element_named(removal_plan.parent, "audio")
                || self
                    .dom_host
                    .is_html_element_named(removal_plan.parent, "video"))
        {
            crate::native_bridge::element::queue_media_load_if_source_or_loading_change(
                scope,
                host_ptr,
                removal_plan.parent,
                "src",
            );
        }

        if !self
            .dom_host
            .is_html_element_named(removal_plan.root, "source")
            || !self
                .dom_host
                .is_html_element_named(removal_plan.parent, "picture")
        {
            return;
        }
        let image = self
            .dom_host
            .child_handles(removal_plan.parent)
            .into_iter()
            .find(|child| self.dom_host.is_html_element_named(*child, "img"));
        if let Some(image) = image {
            crate::native_bridge::element::reset_image_load_dispatch(
                unsafe { &mut *host_ptr },
                image,
            );
            crate::native_bridge::element::queue_image_load_event_if_needed(scope, host_ptr, image);
        }
    }

    fn queue_inserted_text_track_loads(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        roots: &[DomHandle],
    ) {
        if let [root] = roots
            && self.dom_host.first_child(*root).is_none()
        {
            if self.dom_host.is_html_element_named(*root, "track") {
                crate::native_bridge::element::queue_default_text_track_mode_if_needed(
                    scope, host_ptr, *root,
                );
                crate::native_bridge::element::queue_text_track_load_if_needed(
                    scope, host_ptr, *root,
                );
            }
            return;
        }
        let mut stack = preorder_stack(roots);
        let mut tracks = Vec::new();
        while let Some(handle) = stack.pop() {
            if self.dom_host.is_html_element_named(handle, "track") {
                tracks.push(handle);
            }
            self.push_child_handles(&mut stack, handle);
        }
        for track in tracks {
            crate::native_bridge::element::queue_default_text_track_mode_if_needed(
                scope, host_ptr, track,
            );
            crate::native_bridge::element::queue_text_track_load_if_needed(scope, host_ptr, track);
        }
    }

    pub(super) fn push_child_handles(&self, stack: &mut Vec<DomHandle>, handle: DomHandle) {
        let children = self.dom_host.child_handles(handle).collect::<Vec<_>>();
        stack.extend(children.into_iter().rev());
    }
}
