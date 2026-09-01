use std::{ptr::NonNull, rc::Rc};

use html5ever::{
    Attribute, QualName,
    tree_builder::{ElementFlags, NodeOrText, QuirksMode},
};
use url::Url;

use crate::script_planning::{ParserPlanningReadView, ParserScriptRead};
use moli_dom::{
    NodeId,
    native::{
        Attribute as NativeAttribute, DomHost, DomMutationEffects, NativeDom, NativeNodeId, Node,
    },
};
use moli_stylesheet_blocking::{
    StylesheetBlockingReadView, StylesheetElementRead,
    document_owned_blocking_stylesheet_candidate_for_node, link_rel_includes_token,
};

use super::{HtmlTreeSinkState, stream::HtmlTreeSinkStream};
use crate::html::{ParseHandle, ParserCustomElementConstructionHandoff, ParserElementFlags};

#[allow(unused_imports)]
use super::html_chunks;

pub trait ParserMutationEffectConsumer {
    fn consume_parser_mutation_effects(&mut self, effects: DomMutationEffects);
}

#[derive(Clone, Copy)]
struct ParserMutationEffectSink {
    data: NonNull<()>,
    consume: unsafe fn(NonNull<()>, DomMutationEffects),
}

impl ParserMutationEffectSink {
    unsafe fn from_consumer_unchecked<T: ParserMutationEffectConsumer>(consumer: &mut T) -> Self {
        unsafe fn consume_impl<T: ParserMutationEffectConsumer>(
            data: NonNull<()>,
            effects: DomMutationEffects,
        ) {
            // SAFETY: ParserMutationEffectSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.consume_parser_mutation_effects(effects);
        }

        Self {
            data: NonNull::from(consumer).cast(),
            consume: consume_impl::<T>,
        }
    }

    fn consume(self, effects: DomMutationEffects) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.consume)(self.data, effects) };
    }
}

pub trait ParserDomReadConsumer {
    /// Returns a read-only snapshot of the parser's current executable
    /// Document when the embedding runtime can provide one.
    ///
    /// Most parser turns only need narrow node reads. The snapshot is reserved
    /// for end-of-document algorithms, such as Chromium-compatible unstyled
    /// XML presentation, which must inspect the complete tree before applying
    /// mutations back through the live parser mutation sink.
    fn snapshot_parser_document(&mut self) -> Option<NativeDom> {
        None
    }

    fn node_exists(&mut self, node_id: NativeNodeId) -> bool;

    fn is_connected(&mut self, node_id: NativeNodeId) -> bool;

    fn is_text_node(&mut self, node_id: NativeNodeId) -> bool;

    fn owner_document(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId>;

    fn parent_node(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId>;

    fn previous_sibling(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId>;

    fn last_child(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId>;

    fn child_handles(&mut self, node_id: NativeNodeId) -> Vec<NativeNodeId>;

    fn document_order_script_handles(
        &mut self,
        document_handle: NativeNodeId,
    ) -> Vec<NativeNodeId> {
        let mut handles = Vec::new();
        let mut stack = vec![document_handle];
        while let Some(handle) = stack.pop() {
            if self.parser_script_read(handle).is_some() {
                handles.push(handle);
            }
            let mut children = self.child_handles(handle);
            children.reverse();
            stack.extend(children);
        }
        handles
    }

    fn document_order_stylesheet_candidate_handles_before(
        &mut self,
        document_handle: NativeNodeId,
        stop_at: Option<NativeNodeId>,
    ) -> Vec<NativeNodeId> {
        let mut handles = Vec::new();
        let mut stack = vec![document_handle];
        while let Some(handle) = stack.pop() {
            if Some(handle) == stop_at {
                break;
            }
            if self.is_html_element_named(handle, "link")
                || self.is_html_element_named(handle, "style")
            {
                handles.push(handle);
            }
            let mut children = self.child_handles(handle);
            children.reverse();
            stack.extend(children);
        }
        handles
    }

    fn document_body_handle_for_document(
        &mut self,
        document_handle: NativeNodeId,
    ) -> Option<NativeNodeId>;

    fn document_base_url(&mut self, _document_handle: NativeNodeId) -> Option<Url> {
        None
    }

    fn template_contents_handle(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId>;

    fn is_html_element_named(&mut self, node_id: NativeNodeId, local_name: &str) -> bool;

    fn is_external_async_classic_candidate(&mut self, node_id: NativeNodeId) -> bool;

    fn parser_script_read(&mut self, node_id: NativeNodeId) -> Option<ParserScriptRead>;

    fn stylesheet_element(&mut self, node_id: NativeNodeId) -> Option<StylesheetElementRead>;

    fn text_content(&mut self, node_id: NativeNodeId) -> Option<String>;
}

#[derive(Clone, Copy)]
struct ParserDomReadSink {
    data: NonNull<()>,
    snapshot_parser_document: unsafe fn(NonNull<()>) -> Option<NativeDom>,
    node_exists: unsafe fn(NonNull<()>, NativeNodeId) -> bool,
    is_connected: unsafe fn(NonNull<()>, NativeNodeId) -> bool,
    is_text_node: unsafe fn(NonNull<()>, NativeNodeId) -> bool,
    owner_document: unsafe fn(NonNull<()>, NativeNodeId) -> Option<NativeNodeId>,
    parent_node: unsafe fn(NonNull<()>, NativeNodeId) -> Option<NativeNodeId>,
    previous_sibling: unsafe fn(NonNull<()>, NativeNodeId) -> Option<NativeNodeId>,
    last_child: unsafe fn(NonNull<()>, NativeNodeId) -> Option<NativeNodeId>,
    child_handles: unsafe fn(NonNull<()>, NativeNodeId) -> Vec<NativeNodeId>,
    document_order_script_handles: unsafe fn(NonNull<()>, NativeNodeId) -> Vec<NativeNodeId>,
    document_order_stylesheet_candidate_handles_before:
        unsafe fn(NonNull<()>, NativeNodeId, Option<NativeNodeId>) -> Vec<NativeNodeId>,
    document_body_handle_for_document: unsafe fn(NonNull<()>, NativeNodeId) -> Option<NativeNodeId>,
    document_base_url: unsafe fn(NonNull<()>, NativeNodeId) -> Option<Url>,
    template_contents_handle: unsafe fn(NonNull<()>, NativeNodeId) -> Option<NativeNodeId>,
    is_html_element_named: unsafe fn(NonNull<()>, NativeNodeId, &str) -> bool,
    is_external_async_classic_candidate: unsafe fn(NonNull<()>, NativeNodeId) -> bool,
    parser_script_read: unsafe fn(NonNull<()>, NativeNodeId) -> Option<ParserScriptRead>,
    stylesheet_element: unsafe fn(NonNull<()>, NativeNodeId) -> Option<StylesheetElementRead>,
    text_content: unsafe fn(NonNull<()>, NativeNodeId) -> Option<String>,
}

impl ParserDomReadSink {
    unsafe fn from_consumer_unchecked<T: ParserDomReadConsumer>(consumer: &mut T) -> Self {
        unsafe fn snapshot_parser_document_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
        ) -> Option<NativeDom> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.snapshot_parser_document()
        }

        unsafe fn node_exists_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> bool {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.node_exists(node_id)
        }
        unsafe fn is_connected_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> bool {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.is_connected(node_id)
        }
        unsafe fn is_text_node_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> bool {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.is_text_node(node_id)
        }
        unsafe fn owner_document_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> Option<NativeNodeId> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.owner_document(node_id)
        }
        unsafe fn parent_node_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> Option<NativeNodeId> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.parent_node(node_id)
        }
        unsafe fn previous_sibling_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> Option<NativeNodeId> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.previous_sibling(node_id)
        }
        unsafe fn last_child_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> Option<NativeNodeId> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.last_child(node_id)
        }
        unsafe fn child_handles_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> Vec<NativeNodeId> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.child_handles(node_id)
        }
        unsafe fn document_order_script_handles_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            document_handle: NativeNodeId,
        ) -> Vec<NativeNodeId> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.document_order_script_handles(document_handle)
        }
        unsafe fn document_order_stylesheet_candidate_handles_before_impl<
            T: ParserDomReadConsumer,
        >(
            data: NonNull<()>,
            document_handle: NativeNodeId,
            stop_at: Option<NativeNodeId>,
        ) -> Vec<NativeNodeId> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }
                .document_order_stylesheet_candidate_handles_before(document_handle, stop_at)
        }
        unsafe fn document_base_url_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            document_handle: NativeNodeId,
        ) -> Option<Url> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.document_base_url(document_handle)
        }
        unsafe fn document_body_handle_for_document_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            document_handle: NativeNodeId,
        ) -> Option<NativeNodeId> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.document_body_handle_for_document(document_handle)
        }
        unsafe fn template_contents_handle_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> Option<NativeNodeId> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.template_contents_handle(node_id)
        }
        unsafe fn is_html_element_named_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
            local_name: &str,
        ) -> bool {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.is_html_element_named(node_id, local_name)
        }
        unsafe fn is_external_async_classic_candidate_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> bool {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.is_external_async_classic_candidate(node_id)
        }
        unsafe fn parser_script_read_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> Option<ParserScriptRead> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.parser_script_read(node_id)
        }
        unsafe fn stylesheet_element_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> Option<StylesheetElementRead> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.stylesheet_element(node_id)
        }
        unsafe fn text_content_impl<T: ParserDomReadConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) -> Option<String> {
            // SAFETY: ParserDomReadSink::from_consumer_unchecked requires the pointed-to
            // consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.text_content(node_id)
        }

        Self {
            data: NonNull::from(consumer).cast(),
            snapshot_parser_document: snapshot_parser_document_impl::<T>,
            node_exists: node_exists_impl::<T>,
            is_connected: is_connected_impl::<T>,
            is_text_node: is_text_node_impl::<T>,
            owner_document: owner_document_impl::<T>,
            parent_node: parent_node_impl::<T>,
            previous_sibling: previous_sibling_impl::<T>,
            last_child: last_child_impl::<T>,
            child_handles: child_handles_impl::<T>,
            document_order_script_handles: document_order_script_handles_impl::<T>,
            document_order_stylesheet_candidate_handles_before:
                document_order_stylesheet_candidate_handles_before_impl::<T>,
            document_body_handle_for_document: document_body_handle_for_document_impl::<T>,
            document_base_url: document_base_url_impl::<T>,
            template_contents_handle: template_contents_handle_impl::<T>,
            is_html_element_named: is_html_element_named_impl::<T>,
            is_external_async_classic_candidate: is_external_async_classic_candidate_impl::<T>,
            parser_script_read: parser_script_read_impl::<T>,
            stylesheet_element: stylesheet_element_impl::<T>,
            text_content: text_content_impl::<T>,
        }
    }

    fn snapshot_parser_document(self) -> Option<NativeDom> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current parser operation.
        unsafe { (self.snapshot_parser_document)(self.data) }
    }

    fn node_exists(self, node_id: NativeNodeId) -> bool {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.node_exists)(self.data, node_id) }
    }

    fn is_connected(self, node_id: NativeNodeId) -> bool {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.is_connected)(self.data, node_id) }
    }

    fn is_text_node(self, node_id: NativeNodeId) -> bool {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.is_text_node)(self.data, node_id) }
    }

    fn owner_document(self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.owner_document)(self.data, node_id) }
    }

    fn parent_node(self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.parent_node)(self.data, node_id) }
    }

    fn previous_sibling(self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.previous_sibling)(self.data, node_id) }
    }

    fn last_child(self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.last_child)(self.data, node_id) }
    }

    fn child_handles(self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.child_handles)(self.data, node_id) }
    }

    fn document_order_script_handles(self, document_handle: NativeNodeId) -> Vec<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.document_order_script_handles)(self.data, document_handle) }
    }

    fn document_order_stylesheet_candidate_handles_before(
        self,
        document_handle: NativeNodeId,
        stop_at: Option<NativeNodeId>,
    ) -> Vec<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe {
            (self.document_order_stylesheet_candidate_handles_before)(
                self.data,
                document_handle,
                stop_at,
            )
        }
    }

    fn document_base_url(self, document_handle: NativeNodeId) -> Option<Url> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.document_base_url)(self.data, document_handle) }
    }

    fn document_body_handle_for_document(
        self,
        document_handle: NativeNodeId,
    ) -> Option<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.document_body_handle_for_document)(self.data, document_handle) }
    }

    fn template_contents_handle(self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.template_contents_handle)(self.data, node_id) }
    }

    fn is_html_element_named(self, node_id: NativeNodeId, local_name: &str) -> bool {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.is_html_element_named)(self.data, node_id, local_name) }
    }

    fn is_external_async_classic_candidate(self, node_id: NativeNodeId) -> bool {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.is_external_async_classic_candidate)(self.data, node_id) }
    }

    fn parser_script_read(self, node_id: NativeNodeId) -> Option<ParserScriptRead> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.parser_script_read)(self.data, node_id) }
    }

    fn stylesheet_element(self, node_id: NativeNodeId) -> Option<StylesheetElementRead> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.stylesheet_element)(self.data, node_id) }
    }

    fn text_content(self, node_id: NativeNodeId) -> Option<String> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.text_content)(self.data, node_id) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserDomMutation {
    AppendChild {
        parent: NativeNodeId,
        child: NativeNodeId,
    },
    InsertBefore {
        parent: NativeNodeId,
        child: NativeNodeId,
        reference_child: Option<NativeNodeId>,
    },
    RemoveChild {
        parent: NativeNodeId,
        child: NativeNodeId,
    },
}

impl ParserDomMutation {
    pub fn apply_to_dom_host(self, host: &mut DomHost) -> DomMutationEffects {
        match self {
            Self::AppendChild { parent, child } => host.append_child_effects(parent, child),
            Self::InsertBefore {
                parent,
                child,
                reference_child,
            } => host.insert_before_effects(parent, child, reference_child),
            Self::RemoveChild { parent, child } => host.remove_child_effects(parent, child),
        }
    }

    /// Applies a parser tree edit to an unexposed staging fragment.
    ///
    /// The fragment acquires normal Web mutation semantics when its roots are
    /// inserted into the live tree. Running the observer/style/slot pipeline
    /// for every tokenizer splice before then is redundant and expensive.
    pub fn apply_to_detached_dom_host(self, host: &mut DomHost) -> bool {
        match self {
            Self::AppendChild { parent, child } => {
                host.append_child_without_mutation_effects(parent, child)
            }
            Self::InsertBefore {
                parent,
                child,
                reference_child,
            } => host.insert_before_without_mutation_effects(parent, child, reference_child),
            Self::RemoveChild { parent, child } => {
                host.remove_child_without_mutation_effects(parent, child)
            }
        }
    }
}

pub trait ParserDomMutationConsumer {
    fn apply_parser_dom_mutation(&mut self, mutation: ParserDomMutation);

    fn create_parser_element_without_attributes(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId;

    fn create_parser_element_for_document_without_attributes(
        &mut self,
        document_handle: NativeNodeId,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId;

    fn add_attrs_if_missing_for_parser(
        &mut self,
        node_id: NativeNodeId,
        attrs: Vec<NativeAttribute>,
    );

    fn create_text_node(&mut self, document_handle: NativeNodeId, text: String) -> NativeNodeId;

    fn create_comment(&mut self, document_handle: NativeNodeId, text: String) -> NativeNodeId;

    fn create_processing_instruction(
        &mut self,
        document_handle: NativeNodeId,
        target: String,
        data: String,
    ) -> NativeNodeId;

    fn create_cdata_section(&mut self, document_handle: NativeNodeId, data: String)
    -> NativeNodeId;

    fn create_document_type(
        &mut self,
        document_handle: NativeNodeId,
        name: String,
        public_id: String,
        system_id: String,
    ) -> NativeNodeId;

    fn prepend_text_to_text_node(&mut self, node_id: NativeNodeId, text: String);

    fn append_text_to_text_node(&mut self, node_id: NativeNodeId, text: String);

    fn push_parse_error(&mut self, error: String);

    fn set_html_quirks_mode_for_parser(&mut self, quirks_mode: QuirksMode);

    fn mark_script_already_started_for_parser(&mut self, node_id: NativeNodeId);

    fn mark_unclosed_form_control_for_parser(&mut self, node_id: NativeNodeId);

    fn finish_parsing_script_children(&mut self, node_id: NativeNodeId);

    fn finish_parsing_link_children(&mut self, node_id: NativeNodeId);

    fn maybe_clone_an_option_into_selectedcontent(&mut self, _node_id: NativeNodeId) {}

    fn attach_declarative_shadow_for_parser(
        &mut self,
        host_id: NativeNodeId,
        template_id: NativeNodeId,
        attrs: Vec<NativeAttribute>,
    ) -> bool;

    fn associate_parser_form_owner(&mut self, target: NativeNodeId, form: NativeNodeId) -> bool;
}

#[derive(Clone, Copy)]
struct ParserDomMutationSink {
    data: NonNull<()>,
    apply: unsafe fn(NonNull<()>, ParserDomMutation),
    create_parser_element_for_document_without_attributes:
        unsafe fn(NonNull<()>, NativeNodeId, String, String, Option<String>) -> NativeNodeId,
    add_attrs_if_missing_for_parser: unsafe fn(NonNull<()>, NativeNodeId, Vec<NativeAttribute>),
    create_text_node: unsafe fn(NonNull<()>, NativeNodeId, String) -> NativeNodeId,
    create_comment: unsafe fn(NonNull<()>, NativeNodeId, String) -> NativeNodeId,
    create_processing_instruction:
        unsafe fn(NonNull<()>, NativeNodeId, String, String) -> NativeNodeId,
    create_cdata_section: unsafe fn(NonNull<()>, NativeNodeId, String) -> NativeNodeId,
    create_document_type:
        unsafe fn(NonNull<()>, NativeNodeId, String, String, String) -> NativeNodeId,
    prepend_text_to_text_node: unsafe fn(NonNull<()>, NativeNodeId, String),
    append_text_to_text_node: unsafe fn(NonNull<()>, NativeNodeId, String),
    push_parse_error: unsafe fn(NonNull<()>, String),
    set_html_quirks_mode_for_parser: unsafe fn(NonNull<()>, QuirksMode),
    mark_script_already_started_for_parser: unsafe fn(NonNull<()>, NativeNodeId),
    mark_unclosed_form_control_for_parser: unsafe fn(NonNull<()>, NativeNodeId),
    finish_parsing_script_children: unsafe fn(NonNull<()>, NativeNodeId),
    finish_parsing_link_children: unsafe fn(NonNull<()>, NativeNodeId),
    maybe_clone_an_option_into_selectedcontent: unsafe fn(NonNull<()>, NativeNodeId),
    attach_declarative_shadow_for_parser:
        unsafe fn(NonNull<()>, NativeNodeId, NativeNodeId, Vec<NativeAttribute>) -> bool,
    associate_parser_form_owner: unsafe fn(NonNull<()>, NativeNodeId, NativeNodeId) -> bool,
}

impl ParserDomMutationSink {
    unsafe fn from_consumer_unchecked<T: ParserDomMutationConsumer>(consumer: &mut T) -> Self {
        unsafe fn apply_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            mutation: ParserDomMutation,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.apply_parser_dom_mutation(mutation);
        }
        unsafe fn create_parser_element_for_document_without_attributes_impl<
            T: ParserDomMutationConsumer,
        >(
            data: NonNull<()>,
            document_handle: NativeNodeId,
            local_name: String,
            namespace: String,
            prefix: Option<String>,
        ) -> NativeNodeId {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }
                .create_parser_element_for_document_without_attributes(
                    document_handle,
                    local_name,
                    namespace,
                    prefix,
                )
        }
        unsafe fn add_attrs_if_missing_for_parser_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
            attrs: Vec<NativeAttribute>,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.add_attrs_if_missing_for_parser(node_id, attrs);
        }
        unsafe fn create_text_node_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            document_handle: NativeNodeId,
            text: String,
        ) -> NativeNodeId {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.create_text_node(document_handle, text)
        }
        unsafe fn create_comment_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            document_handle: NativeNodeId,
            text: String,
        ) -> NativeNodeId {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.create_comment(document_handle, text)
        }
        unsafe fn create_processing_instruction_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            document_handle: NativeNodeId,
            target: String,
            data_text: String,
        ) -> NativeNodeId {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.create_processing_instruction(
                document_handle,
                target,
                data_text,
            )
        }
        unsafe fn create_cdata_section_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            document_handle: NativeNodeId,
            cdata: String,
        ) -> NativeNodeId {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.create_cdata_section(document_handle, cdata)
        }
        unsafe fn create_document_type_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            document_handle: NativeNodeId,
            name: String,
            public_id: String,
            system_id: String,
        ) -> NativeNodeId {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.create_document_type(
                document_handle,
                name,
                public_id,
                system_id,
            )
        }
        unsafe fn prepend_text_to_text_node_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
            text: String,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.prepend_text_to_text_node(node_id, text);
        }
        unsafe fn append_text_to_text_node_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
            text: String,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.append_text_to_text_node(node_id, text);
        }
        unsafe fn push_parse_error_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            error: String,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.push_parse_error(error);
        }
        unsafe fn set_html_quirks_mode_for_parser_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            quirks_mode: QuirksMode,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.set_html_quirks_mode_for_parser(quirks_mode);
        }
        unsafe fn mark_script_already_started_for_parser_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.mark_script_already_started_for_parser(node_id);
        }
        unsafe fn mark_unclosed_form_control_for_parser_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.mark_unclosed_form_control_for_parser(node_id);
        }
        unsafe fn finish_parsing_script_children_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.finish_parsing_script_children(node_id);
        }
        unsafe fn finish_parsing_link_children_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.finish_parsing_link_children(node_id);
        }
        unsafe fn maybe_clone_an_option_into_selectedcontent_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            node_id: NativeNodeId,
        ) {
            // SAFETY: ParserDomMutationSink::from_consumer requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }
                .maybe_clone_an_option_into_selectedcontent(node_id);
        }
        unsafe fn attach_declarative_shadow_for_parser_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            host_id: NativeNodeId,
            template_id: NativeNodeId,
            attrs: Vec<NativeAttribute>,
        ) -> bool {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.attach_declarative_shadow_for_parser(
                host_id,
                template_id,
                attrs,
            )
        }
        unsafe fn associate_parser_form_owner_impl<T: ParserDomMutationConsumer>(
            data: NonNull<()>,
            target: NativeNodeId,
            form: NativeNodeId,
        ) -> bool {
            // SAFETY: ParserDomMutationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.associate_parser_form_owner(target, form)
        }

        Self {
            data: NonNull::from(consumer).cast(),
            apply: apply_impl::<T>,
            create_parser_element_for_document_without_attributes:
                create_parser_element_for_document_without_attributes_impl::<T>,
            add_attrs_if_missing_for_parser: add_attrs_if_missing_for_parser_impl::<T>,
            create_text_node: create_text_node_impl::<T>,
            create_comment: create_comment_impl::<T>,
            create_processing_instruction: create_processing_instruction_impl::<T>,
            create_cdata_section: create_cdata_section_impl::<T>,
            create_document_type: create_document_type_impl::<T>,
            prepend_text_to_text_node: prepend_text_to_text_node_impl::<T>,
            append_text_to_text_node: append_text_to_text_node_impl::<T>,
            push_parse_error: push_parse_error_impl::<T>,
            set_html_quirks_mode_for_parser: set_html_quirks_mode_for_parser_impl::<T>,
            mark_script_already_started_for_parser: mark_script_already_started_for_parser_impl::<T>,
            mark_unclosed_form_control_for_parser: mark_unclosed_form_control_for_parser_impl::<T>,
            finish_parsing_script_children: finish_parsing_script_children_impl::<T>,
            finish_parsing_link_children: finish_parsing_link_children_impl::<T>,
            maybe_clone_an_option_into_selectedcontent:
                maybe_clone_an_option_into_selectedcontent_impl::<T>,
            attach_declarative_shadow_for_parser: attach_declarative_shadow_for_parser_impl::<T>,
            associate_parser_form_owner: associate_parser_form_owner_impl::<T>,
        }
    }

    fn apply(self, mutation: ParserDomMutation) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.apply)(self.data, mutation) };
    }

    fn create_parser_element_for_document_without_attributes(
        self,
        document_handle: NativeNodeId,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe {
            (self.create_parser_element_for_document_without_attributes)(
                self.data,
                document_handle,
                local_name,
                namespace,
                prefix,
            )
        }
    }

    fn add_attrs_if_missing_for_parser(self, node_id: NativeNodeId, attrs: Vec<NativeAttribute>) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.add_attrs_if_missing_for_parser)(self.data, node_id, attrs) };
    }

    fn create_text_node(self, document_handle: NativeNodeId, text: String) -> NativeNodeId {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.create_text_node)(self.data, document_handle, text) }
    }

    fn create_comment(self, document_handle: NativeNodeId, text: String) -> NativeNodeId {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.create_comment)(self.data, document_handle, text) }
    }

    fn create_processing_instruction(
        self,
        document_handle: NativeNodeId,
        target: String,
        data: String,
    ) -> NativeNodeId {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.create_processing_instruction)(self.data, document_handle, target, data) }
    }

    fn create_cdata_section(self, document_handle: NativeNodeId, data: String) -> NativeNodeId {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.create_cdata_section)(self.data, document_handle, data) }
    }

    fn create_document_type(
        self,
        document_handle: NativeNodeId,
        name: String,
        public_id: String,
        system_id: String,
    ) -> NativeNodeId {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe {
            (self.create_document_type)(self.data, document_handle, name, public_id, system_id)
        }
    }

    fn prepend_text_to_text_node(self, node_id: NativeNodeId, text: String) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.prepend_text_to_text_node)(self.data, node_id, text) };
    }

    fn append_text_to_text_node(self, node_id: NativeNodeId, text: String) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.append_text_to_text_node)(self.data, node_id, text) };
    }

    fn push_parse_error(self, error: String) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.push_parse_error)(self.data, error) };
    }

    fn set_html_quirks_mode_for_parser(self, quirks_mode: QuirksMode) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.set_html_quirks_mode_for_parser)(self.data, quirks_mode) };
    }

    fn mark_script_already_started_for_parser(self, node_id: NativeNodeId) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.mark_script_already_started_for_parser)(self.data, node_id) };
    }

    fn mark_unclosed_form_control_for_parser(self, node_id: NativeNodeId) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.mark_unclosed_form_control_for_parser)(self.data, node_id) };
    }

    fn finish_parsing_script_children(self, node_id: NativeNodeId) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.finish_parsing_script_children)(self.data, node_id) };
    }

    fn finish_parsing_link_children(self, node_id: NativeNodeId) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.finish_parsing_link_children)(self.data, node_id) };
    }

    fn maybe_clone_an_option_into_selectedcontent(self, node_id: NativeNodeId) {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.maybe_clone_an_option_into_selectedcontent)(self.data, node_id) };
    }

    fn attach_declarative_shadow_for_parser(
        self,
        host_id: NativeNodeId,
        template_id: NativeNodeId,
        attrs: Vec<NativeAttribute>,
    ) -> bool {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe {
            (self.attach_declarative_shadow_for_parser)(self.data, host_id, template_id, attrs)
        }
    }

    fn associate_parser_form_owner(self, target: NativeNodeId, form: NativeNodeId) -> bool {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.associate_parser_form_owner)(self.data, target, form) }
    }
}

pub struct ParserElementCreationRequest<'a> {
    pub document_handle: NativeNodeId,
    pub intended_parent: Option<NativeNodeId>,
    pub local_name: &'a str,
    pub namespace: &'a str,
    pub prefix: Option<&'a str>,
    pub attributes: &'a [NativeAttribute],
}

pub trait ParserElementCreationConsumer {
    fn create_parser_element(
        &mut self,
        request: ParserElementCreationRequest<'_>,
    ) -> Option<NativeNodeId>;
}

#[derive(Clone, Copy)]
struct ParserElementCreationSink {
    data: NonNull<()>,
    create:
        for<'a> unsafe fn(NonNull<()>, ParserElementCreationRequest<'a>) -> Option<NativeNodeId>,
}

/// Step-scoped sink bundle for mutating the runtime-owned DOM during one
/// parser pump.
///
/// The parser crate stores only callable sinks here, never a live `DomHost`
/// pointer or owner token. TreeSink callbacks may synchronously re-enter the
/// renderer for custom-element construction, but this bundle must not be stored
/// in async work, microtasks, reactions, or scheduler queues.
pub(super) struct ParserRuntimeDomSinks {
    dom_read_sink: ParserDomReadSink,
    dom_mutation_sink: ParserDomMutationSink,
    mutation_effect_sink: ParserMutationEffectSink,
    element_creation_sink: Option<ParserElementCreationSink>,
}

impl ParserRuntimeDomSinks {
    fn new(
        dom_read_sink: ParserDomReadSink,
        dom_mutation_sink: ParserDomMutationSink,
        mutation_effect_sink: ParserMutationEffectSink,
        element_creation_sink: Option<ParserElementCreationSink>,
    ) -> Self {
        Self {
            dom_read_sink,
            dom_mutation_sink,
            mutation_effect_sink,
            element_creation_sink,
        }
    }

    /// Creates an erased bundle whose pointers are valid only while `consumer`
    /// remains exclusively borrowed by the enclosing scoped parser operation.
    pub(super) unsafe fn from_consumer<T>(consumer: &mut T) -> Self
    where
        T: ParserDomReadConsumer
            + ParserDomMutationConsumer
            + ParserMutationEffectConsumer
            + ParserElementCreationConsumer,
    {
        Self::new(
            // SAFETY: the scoped DocumentStream entrypoint keeps the one mutable
            // consumer borrow alive until its Drop guard removes this bundle.
            unsafe { ParserDomReadSink::from_consumer_unchecked(&mut *consumer) },
            // SAFETY: callbacks are invoked serially during the same scoped pump.
            unsafe { ParserDomMutationSink::from_consumer_unchecked(&mut *consumer) },
            // SAFETY: callbacks are invoked serially during the same scoped pump.
            unsafe { ParserMutationEffectSink::from_consumer_unchecked(&mut *consumer) },
            Some(
                // SAFETY: callbacks are invoked serially during the same scoped pump.
                unsafe { ParserElementCreationSink::from_consumer_unchecked(&mut *consumer) },
            ),
        )
    }

    pub(super) unsafe fn from_consumer_without_element_creation<T>(consumer: &mut T) -> Self
    where
        T: ParserDomReadConsumer + ParserDomMutationConsumer + ParserMutationEffectConsumer,
    {
        Self::new(
            // SAFETY: the scoped DocumentStream entrypoint keeps the one mutable
            // consumer borrow alive until its Drop guard removes this bundle.
            unsafe { ParserDomReadSink::from_consumer_unchecked(&mut *consumer) },
            // SAFETY: callbacks are invoked serially during the same scoped pump.
            unsafe { ParserDomMutationSink::from_consumer_unchecked(&mut *consumer) },
            // SAFETY: callbacks are invoked serially during the same scoped pump.
            unsafe { ParserMutationEffectSink::from_consumer_unchecked(&mut *consumer) },
            None,
        )
    }

    pub(super) unsafe fn from_consumers<T, E>(consumer: &mut T, element_consumer: &mut E) -> Self
    where
        T: ParserDomReadConsumer + ParserDomMutationConsumer + ParserMutationEffectConsumer,
        E: ParserElementCreationConsumer,
    {
        Self::new(
            // SAFETY: the scoped DocumentStream entrypoint keeps both disjoint
            // mutable borrows alive until its Drop guard removes this bundle.
            unsafe { ParserDomReadSink::from_consumer_unchecked(&mut *consumer) },
            // SAFETY: callbacks are invoked serially during the same scoped pump.
            unsafe { ParserDomMutationSink::from_consumer_unchecked(&mut *consumer) },
            // SAFETY: callbacks are invoked serially during the same scoped pump.
            unsafe { ParserMutationEffectSink::from_consumer_unchecked(&mut *consumer) },
            Some(
                // SAFETY: `element_consumer` is a distinct exclusive borrow.
                unsafe {
                    ParserElementCreationSink::from_consumer_unchecked(&mut *element_consumer)
                },
            ),
        )
    }

    fn mutation_effect_sink(&self) -> ParserMutationEffectSink {
        self.mutation_effect_sink
    }

    fn dom_read_sink(&self) -> ParserDomReadSink {
        self.dom_read_sink
    }

    fn dom_mutation_sink(&self) -> ParserDomMutationSink {
        self.dom_mutation_sink
    }

    fn element_creation_sink(&self) -> Option<ParserElementCreationSink> {
        self.element_creation_sink
    }
}

impl ParserElementCreationSink {
    unsafe fn from_consumer_unchecked<T: ParserElementCreationConsumer>(consumer: &mut T) -> Self {
        unsafe fn create_impl<T: ParserElementCreationConsumer>(
            data: NonNull<()>,
            request: ParserElementCreationRequest<'_>,
        ) -> Option<NativeNodeId> {
            // SAFETY: ParserElementCreationSink::from_consumer_unchecked requires the
            // pointed-to consumer to remain live and exclusive for the pump step.
            unsafe { data.cast::<T>().as_mut() }.create_parser_element(request)
        }

        Self {
            data: NonNull::from(consumer).cast(),
            create: create_impl::<T>,
        }
    }

    fn create_parser_element(
        self,
        request: ParserElementCreationRequest<'_>,
    ) -> Option<NativeNodeId> {
        // SAFETY: construction ties the raw pointer and callback to the same
        // consumer remains live for the current runtime-DOM sink step.
        unsafe { (self.create)(self.data, request) }
    }
}

pub(super) struct ParserMutationEffectDelivery {
    effects: DomMutationEffects,
    sink: Option<ParserMutationEffectSink>,
    runtime_dom_sinks_active: bool,
}

impl ParserMutationEffectDelivery {
    fn none() -> Self {
        Self {
            effects: DomMutationEffects::default(),
            sink: None,
            runtime_dom_sinks_active: false,
        }
    }

    pub(super) fn consume(self) {
        if !self.effects.did_change() {
            return;
        }
        if let Some(sink) = self.sink {
            sink.consume(self.effects);
            return;
        }
        assert!(
            !self.runtime_dom_sinks_active,
            "runtime DOM sink mutation requires an external mutation effect sink"
        );
        // Before bootstrap the parser still owns the initial DomHost. The DOM
        // mutation itself has already happened synchronously, and there is no
        // runtime owner yet that could observe records, reactions, style work, or
        // connected-script work. Parser mutation against the live DOM
        // owner must use the explicit sink above.
    }
}

fn template_contents_handle_in_host(host: &DomHost, node_id: NativeNodeId) -> Option<NativeNodeId> {
    host.parser_template_contents_handle(node_id)
}

fn is_html_element_named_in_host(host: &DomHost, node_id: NativeNodeId, local_name: &str) -> bool {
    host.dom().is_html_element_named(node_id, local_name)
}

fn is_external_async_classic_candidate_in_host(host: &DomHost, node_id: NativeNodeId) -> bool {
    let Some(element) = host.node(node_id).and_then(Node::as_element) else {
        return false;
    };
    if !element.is_script_element() {
        return false;
    }
    if element.script_source_attribute().is_none() || element.attribute("async").is_none() {
        return false;
    }
    if element.is_html_script() && element.attribute("nomodule").is_some() {
        return false;
    }
    let Some(script_type) = element.attribute("type") else {
        return true;
    };
    if script_type.is_empty() {
        return true;
    }
    moli_script::classify_script_kind(Some(script_type)) == moli_page_types::ScriptKind::Classic
}

fn is_modulepreload_link_candidate(
    token_local_name: &str,
    token_namespace: &str,
    token_attributes: &[NativeAttribute],
) -> bool {
    if token_namespace != "http://www.w3.org/1999/xhtml" || token_local_name != "link" {
        return false;
    }
    let mut has_modulepreload_rel = false;
    let mut has_non_empty_href = false;
    for attribute in token_attributes {
        match attribute.local_name() {
            "rel" => {
                has_modulepreload_rel |=
                    link_rel_includes_token(attribute.value(), "modulepreload");
            }
            "href" => {
                has_non_empty_href |= !attribute.value().trim().is_empty();
            }
            _ => {}
        }
    }
    has_modulepreload_rel && has_non_empty_href
}

fn is_meta_csp_candidate(
    token_local_name: &str,
    _token_namespace: &str,
    token_attributes: &[NativeAttribute],
) -> bool {
    token_local_name == "meta"
        && token_attributes.iter().any(|attribute| {
            attribute.local_name().eq_ignore_ascii_case("http-equiv")
                && attribute
                    .value()
                    .eq_ignore_ascii_case("content-security-policy")
        })
}

fn parser_script_read_in_host(host: &DomHost, node_id: NativeNodeId) -> Option<ParserScriptRead> {
    <DomHost as ParserPlanningReadView>::parser_script_read(host, node_id)
}

fn stylesheet_element_in_host(
    host: &DomHost,
    node_id: NativeNodeId,
) -> Option<StylesheetElementRead> {
    host.node(node_id)
        .and_then(StylesheetElementRead::from_node)
}

fn text_content_in_host(host: &DomHost, node_id: NativeNodeId) -> Option<String> {
    host.text_content(node_id)
}

#[cfg(test)]
struct TestMutationEffectCollector<'a> {
    host: *mut DomHost,
    effects: &'a mut DomMutationEffects,
    panic_on_mutation: bool,
}

#[cfg(test)]
impl ParserDomReadConsumer for TestMutationEffectCollector<'_> {
    fn node_exists(&mut self, node_id: NativeNodeId) -> bool {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.node(node_id).is_some()
    }

    fn is_connected(&mut self, node_id: NativeNodeId) -> bool {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.is_connected(node_id)
    }

    fn is_text_node(&mut self, node_id: NativeNodeId) -> bool {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }
            .node(node_id)
            .and_then(Node::as_text)
            .is_some()
    }

    fn owner_document(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.owner_document_handle(node_id)
    }

    fn parent_node(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }
            .node(node_id)
            .and_then(Node::parent_node)
    }

    fn previous_sibling(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }
            .node(node_id)
            .and_then(Node::prev_sibling)
    }

    fn last_child(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }
            .node(node_id)
            .and_then(Node::last_child)
    }

    fn child_handles(&mut self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.child_handles(node_id).collect()
    }

    fn document_body_handle_for_document(
        &mut self,
        document_handle: NativeNodeId,
    ) -> Option<NativeNodeId> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.document_body_handle_for_document(document_handle)
    }

    fn template_contents_handle(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        template_contents_handle_in_host(unsafe { &*self.host }, node_id)
    }

    fn is_html_element_named(&mut self, node_id: NativeNodeId, local_name: &str) -> bool {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        is_html_element_named_in_host(unsafe { &*self.host }, node_id, local_name)
    }

    fn is_external_async_classic_candidate(&mut self, node_id: NativeNodeId) -> bool {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        is_external_async_classic_candidate_in_host(unsafe { &*self.host }, node_id)
    }

    fn parser_script_read(&mut self, node_id: NativeNodeId) -> Option<ParserScriptRead> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        parser_script_read_in_host(unsafe { &*self.host }, node_id)
    }

    fn stylesheet_element(&mut self, node_id: NativeNodeId) -> Option<StylesheetElementRead> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        stylesheet_element_in_host(unsafe { &*self.host }, node_id)
    }

    fn text_content(&mut self, node_id: NativeNodeId) -> Option<String> {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        text_content_in_host(unsafe { &*self.host }, node_id)
    }
}

#[cfg(test)]
impl ParserDomMutationConsumer for TestMutationEffectCollector<'_> {
    fn apply_parser_dom_mutation(&mut self, mutation: ParserDomMutation) {
        if self.panic_on_mutation {
            self.panic_on_mutation = false;
            panic!("test parser mutation panic");
        }
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let effects = mutation.apply_to_dom_host(unsafe { &mut *self.host });
        self.effects.merge(effects);
    }

    fn create_parser_element_without_attributes(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }
            .create_parser_element_without_attributes(local_name, namespace, prefix)
    }

    fn create_parser_element_for_document_without_attributes(
        &mut self,
        document_handle: NativeNodeId,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_parser_element_without_attributes_for_document(
            document_handle,
            local_name,
            namespace,
            prefix,
        )
    }

    fn add_attrs_if_missing_for_parser(
        &mut self,
        node_id: NativeNodeId,
        attrs: Vec<NativeAttribute>,
    ) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.add_attrs_if_missing_for_parser(node_id, attrs);
    }

    fn create_text_node(&mut self, document_handle: NativeNodeId, text: String) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_text_node_for_document(document_handle, &text)
    }

    fn create_comment(&mut self, document_handle: NativeNodeId, text: String) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_comment_for_document(document_handle, &text)
    }

    fn create_processing_instruction(
        &mut self,
        document_handle: NativeNodeId,
        target: String,
        data: String,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_processing_instruction_for_document(
            document_handle,
            &target,
            &data,
        )
    }

    fn create_cdata_section(
        &mut self,
        document_handle: NativeNodeId,
        data: String,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_cdata_section_for_document(document_handle, &data)
    }

    fn create_document_type(
        &mut self,
        document_handle: NativeNodeId,
        name: String,
        public_id: String,
        system_id: String,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_document_type_for_document(
            document_handle,
            &name,
            &public_id,
            &system_id,
        )
    }

    fn prepend_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        prepend_text_to_text_node_in_host(unsafe { &mut *self.host }, node_id, text);
    }

    fn append_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        append_text_to_text_node_in_host(unsafe { &mut *self.host }, node_id, text);
    }

    fn push_parse_error(&mut self, error: String) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.push_parse_error(error);
    }

    fn set_html_quirks_mode_for_parser(&mut self, quirks_mode: QuirksMode) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.set_html_quirks_mode_for_parser(quirks_mode);
    }

    fn mark_script_already_started_for_parser(&mut self, node_id: NativeNodeId) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let _ = unsafe { &mut *self.host }.set_script_already_started(node_id, true);
    }

    fn mark_unclosed_form_control_for_parser(&mut self, node_id: NativeNodeId) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let _ = unsafe { &mut *self.host }.set_blocks_form_submission(node_id, true);
    }

    fn finish_parsing_script_children(&mut self, node_id: NativeNodeId) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let _ = unsafe { &mut *self.host }.finish_parsing_script_children(node_id);
    }

    fn finish_parsing_link_children(&mut self, node_id: NativeNodeId) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let _ = unsafe { &mut *self.host }.finish_parsing_link_children(node_id);
    }

    fn attach_declarative_shadow_for_parser(
        &mut self,
        host_id: NativeNodeId,
        template_id: NativeNodeId,
        attrs: Vec<NativeAttribute>,
    ) -> bool {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.attach_declarative_shadow_for_parser(
            host_id,
            template_id,
            &attrs,
        )
    }

    fn associate_parser_form_owner(&mut self, target: NativeNodeId, form: NativeNodeId) -> bool {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.associate_parser_form_owner(target, form)
    }
}

#[cfg(test)]
impl ParserMutationEffectConsumer for TestMutationEffectCollector<'_> {
    fn consume_parser_mutation_effects(&mut self, effects: DomMutationEffects) {
        self.effects.merge(effects);
    }
}

fn prepend_text_to_text_node_in_host(host: &mut DomHost, node_id: NativeNodeId, text: String) {
    if let Some(text_node) = host
        .node_mut(node_id)
        .and_then(|node| node.data_mut().as_text_mut())
    {
        let mut merged = text;
        merged.push_str(text_node.data());
        text_node.set_data(merged);
    }
}

fn append_text_to_text_node_in_host(host: &mut DomHost, node_id: NativeNodeId, text: String) {
    if let Some(text_node) = host
        .node_mut(node_id)
        .and_then(|node| node.data_mut().as_text_mut())
    {
        let mut merged = text_node.data().to_owned();
        merged.push_str(&text);
        text_node.set_data(merged);
    }
}

#[cfg(test)]
struct TestReadTrackingCollector<'a> {
    host: *mut DomHost,
    effects: &'a mut DomMutationEffects,
    read_calls: &'a std::cell::Cell<usize>,
    read_events: Option<&'a std::cell::RefCell<Vec<&'static str>>>,
}

#[cfg(test)]
impl TestReadTrackingCollector<'_> {
    fn record_read(&self) {
        self.read_calls.set(self.read_calls.get() + 1);
    }

    fn record_read_event(&self, event: &'static str) {
        self.record_read();
        if let Some(read_events) = self.read_events {
            read_events.borrow_mut().push(event);
        }
    }
}

#[cfg(test)]
impl ParserDomReadConsumer for TestReadTrackingCollector<'_> {
    fn node_exists(&mut self, node_id: NativeNodeId) -> bool {
        self.record_read();
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.node(node_id).is_some()
    }

    fn is_connected(&mut self, node_id: NativeNodeId) -> bool {
        self.record_read_event("connected");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.is_connected(node_id)
    }

    fn is_text_node(&mut self, node_id: NativeNodeId) -> bool {
        self.record_read();
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }
            .node(node_id)
            .and_then(Node::as_text)
            .is_some()
    }

    fn parent_node(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.record_read();
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }
            .node(node_id)
            .and_then(Node::parent_node)
    }

    fn previous_sibling(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.record_read();
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }
            .node(node_id)
            .and_then(Node::prev_sibling)
    }

    fn last_child(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.record_read();
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }
            .node(node_id)
            .and_then(Node::last_child)
    }

    fn child_handles(&mut self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        self.record_read();
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.child_handles(node_id).collect()
    }

    fn document_order_script_handles(
        &mut self,
        document_handle: NativeNodeId,
    ) -> Vec<NativeNodeId> {
        self.record_read_event("document-order-scripts");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.script_handles_in_light_subtree(document_handle)
    }

    fn document_order_stylesheet_candidate_handles_before(
        &mut self,
        document_handle: NativeNodeId,
        stop_at: Option<NativeNodeId>,
    ) -> Vec<NativeNodeId> {
        self.record_read_event("document-order-stylesheet-candidates");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }
            .stylesheet_candidate_handles_before_in_tree_scope(document_handle, stop_at)
    }

    fn owner_document(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.record_read_event("owner-document");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.owner_document_handle(node_id)
    }

    fn document_body_handle_for_document(
        &mut self,
        document_handle: NativeNodeId,
    ) -> Option<NativeNodeId> {
        self.record_read_event("document-body");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &*self.host }.document_body_handle_for_document(document_handle)
    }

    fn template_contents_handle(&mut self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        self.record_read_event("template-contents");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        template_contents_handle_in_host(unsafe { &*self.host }, node_id)
    }

    fn is_html_element_named(&mut self, node_id: NativeNodeId, local_name: &str) -> bool {
        self.record_read_event("html-element-name");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        is_html_element_named_in_host(unsafe { &*self.host }, node_id, local_name)
    }

    fn is_external_async_classic_candidate(&mut self, node_id: NativeNodeId) -> bool {
        self.record_read_event("async-classic-candidate");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        is_external_async_classic_candidate_in_host(unsafe { &*self.host }, node_id)
    }

    fn parser_script_read(&mut self, node_id: NativeNodeId) -> Option<ParserScriptRead> {
        self.record_read_event("parser-script-read");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        parser_script_read_in_host(unsafe { &*self.host }, node_id)
    }

    fn stylesheet_element(&mut self, node_id: NativeNodeId) -> Option<StylesheetElementRead> {
        self.record_read_event("stylesheet-element");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        stylesheet_element_in_host(unsafe { &*self.host }, node_id)
    }

    fn text_content(&mut self, node_id: NativeNodeId) -> Option<String> {
        self.record_read_event("text-content");
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        text_content_in_host(unsafe { &*self.host }, node_id)
    }
}

#[cfg(test)]
impl ParserDomMutationConsumer for TestReadTrackingCollector<'_> {
    fn apply_parser_dom_mutation(&mut self, mutation: ParserDomMutation) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let effects = mutation.apply_to_dom_host(unsafe { &mut *self.host });
        self.effects.merge(effects);
    }

    fn create_parser_element_without_attributes(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }
            .create_parser_element_without_attributes(local_name, namespace, prefix)
    }

    fn create_parser_element_for_document_without_attributes(
        &mut self,
        document_handle: NativeNodeId,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_parser_element_without_attributes_for_document(
            document_handle,
            local_name,
            namespace,
            prefix,
        )
    }

    fn add_attrs_if_missing_for_parser(
        &mut self,
        node_id: NativeNodeId,
        attrs: Vec<NativeAttribute>,
    ) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.add_attrs_if_missing_for_parser(node_id, attrs);
    }

    fn create_text_node(&mut self, document_handle: NativeNodeId, text: String) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_text_node_for_document(document_handle, &text)
    }

    fn create_comment(&mut self, document_handle: NativeNodeId, text: String) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_comment_for_document(document_handle, &text)
    }

    fn create_processing_instruction(
        &mut self,
        document_handle: NativeNodeId,
        target: String,
        data: String,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_processing_instruction_for_document(
            document_handle,
            &target,
            &data,
        )
    }

    fn create_cdata_section(
        &mut self,
        document_handle: NativeNodeId,
        data: String,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_cdata_section_for_document(document_handle, &data)
    }

    fn create_document_type(
        &mut self,
        document_handle: NativeNodeId,
        name: String,
        public_id: String,
        system_id: String,
    ) -> NativeNodeId {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.create_document_type_for_document(
            document_handle,
            &name,
            &public_id,
            &system_id,
        )
    }

    fn prepend_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        prepend_text_to_text_node_in_host(unsafe { &mut *self.host }, node_id, text);
    }

    fn append_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        append_text_to_text_node_in_host(unsafe { &mut *self.host }, node_id, text);
    }

    fn push_parse_error(&mut self, error: String) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.push_parse_error(error);
    }

    fn set_html_quirks_mode_for_parser(&mut self, quirks_mode: QuirksMode) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.set_html_quirks_mode_for_parser(quirks_mode);
    }

    fn mark_script_already_started_for_parser(&mut self, node_id: NativeNodeId) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let _ = unsafe { &mut *self.host }.set_script_already_started(node_id, true);
    }

    fn mark_unclosed_form_control_for_parser(&mut self, node_id: NativeNodeId) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let _ = unsafe { &mut *self.host }.set_blocks_form_submission(node_id, true);
    }

    fn finish_parsing_script_children(&mut self, node_id: NativeNodeId) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let _ = unsafe { &mut *self.host }.finish_parsing_script_children(node_id);
    }

    fn finish_parsing_link_children(&mut self, node_id: NativeNodeId) {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        let _ = unsafe { &mut *self.host }.finish_parsing_link_children(node_id);
    }

    fn attach_declarative_shadow_for_parser(
        &mut self,
        host_id: NativeNodeId,
        template_id: NativeNodeId,
        attrs: Vec<NativeAttribute>,
    ) -> bool {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.attach_declarative_shadow_for_parser(
            host_id,
            template_id,
            &attrs,
        )
    }

    fn associate_parser_form_owner(&mut self, target: NativeNodeId, form: NativeNodeId) -> bool {
        // SAFETY: tests keep the borrowed DomHost pointer alive and route the
        // parser pump through this collector for the duration of the step.
        unsafe { &mut *self.host }.associate_parser_form_owner(target, form)
    }
}

#[cfg(test)]
impl ParserMutationEffectConsumer for TestReadTrackingCollector<'_> {
    fn consume_parser_mutation_effects(&mut self, effects: DomMutationEffects) {
        self.effects.merge(effects);
    }
}

#[cfg(test)]
struct TestElementCreationCollector {
    dom_host: *mut DomHost,
    created_handle: Option<NativeNodeId>,
    saw_no_token_attributes_before_append: bool,
    request_ids_and_intended_parents: Vec<(String, Option<NativeNodeId>)>,
}

#[cfg(test)]
impl ParserElementCreationConsumer for TestElementCreationCollector {
    fn create_parser_element(
        &mut self,
        request: ParserElementCreationRequest<'_>,
    ) -> Option<NativeNodeId> {
        if request.local_name != "x-sync" {
            return None;
        }
        let id = request
            .attributes
            .iter()
            .find(|attribute| attribute.local_name() == "id")
            .map(|attribute| attribute.value().to_owned())
            .unwrap_or_default();
        self.request_ids_and_intended_parents
            .push((id, request.intended_parent));
        // SAFETY: the test keeps the borrowed DomHost pointer alive and
        // exclusively routes this parser pump through the test sink.
        let host = unsafe { &mut *self.dom_host };
        let handle = host.create_parser_element_without_attributes_for_document(
            request.document_handle,
            request.local_name.to_owned(),
            request.namespace.to_owned(),
            request.prefix.map(str::to_owned),
        );
        self.saw_no_token_attributes_before_append = host.get_attribute(handle, "id").is_none();
        host.add_attrs_if_missing_for_parser(handle, request.attributes.to_vec());
        self.created_handle = Some(handle);
        Some(handle)
    }
}

pub(super) struct ParserStreamHtmlTreeSinkTarget {
    /// DomHost owned by the parser before the runtime takes over the initial document.
    /// Set to `None` after `take_parser_stream_document()`.
    owned_dom_host: Option<DomHost>,
    /// Root that html5ever mutates for this parser stream. This is normally a
    /// Document, but fragment parsing targets a detached DocumentFragment.
    parser_root_handle: Option<NativeNodeId>,
    /// Document that owns nodes created by this parser stream. This differs
    /// from `parser_root_handle` only for fragment parsing.
    parser_owner_document_handle: Option<NativeNodeId>,
    /// Parser document URL cached outside the active DomHost so runtime sink
    /// steps do not need to read the document node.
    parser_document_url: Option<Url>,
    /// Runtime-owned read/mutation/effect sinks for the current parser step.
    /// Present only while a runtime DOM sink step is active.
    runtime_dom_sinks: Option<ParserRuntimeDomSinks>,
    /// Set when html5ever resolves the next insertion point to template
    /// contents. The handle also identifies the inert owner Document to use
    /// when creating the next node.
    next_template_contents_insertion: Option<NativeNodeId>,
    open_template_element_depth: usize,
    pending_open_parser_element: Option<NativeNodeId>,
    open_parser_elements: Vec<OpenParserElement>,
    allow_declarative_shadow_roots: bool,
    pending_null_custom_element_registry_elements: Vec<NativeNodeId>,
    state: HtmlTreeSinkState,
}

struct OpenParserElement {
    node_id: NativeNodeId,
    name: Rc<QualName>,
}

#[cfg(test)]
struct ParserRuntimeDomTargetStep<'a> {
    target: &'a mut ParserStreamHtmlTreeSinkTarget,
}

#[cfg(test)]
impl Drop for ParserRuntimeDomTargetStep<'_> {
    fn drop(&mut self) {
        self.target.clear_runtime_dom_sinks_after_parse_step();
    }
}

impl ParserStreamHtmlTreeSinkTarget {
    pub(super) fn new_with_declarative_shadow_roots_and_scripting(
        final_url: Url,
        allow_declarative_shadow_roots: bool,
        scripting_enabled: bool,
    ) -> Self {
        let dom_host = DomHost::from_dom(NativeDom::new_html_with_scripting(
            final_url.clone(),
            scripting_enabled,
        ));
        let document_handle = dom_host.document_handle();
        Self {
            owned_dom_host: Some(dom_host),
            parser_root_handle: Some(document_handle),
            parser_owner_document_handle: Some(document_handle),
            parser_document_url: Some(final_url),
            runtime_dom_sinks: None,
            next_template_contents_insertion: None,
            open_template_element_depth: 0,
            pending_open_parser_element: None,
            open_parser_elements: Vec::new(),
            allow_declarative_shadow_roots,
            pending_null_custom_element_registry_elements: Vec::new(),
            state: HtmlTreeSinkState::default(),
        }
    }

    pub(super) fn new_xml(final_url: Url) -> Self {
        let dom_host = DomHost::from_dom(NativeDom::new_xml(final_url.clone()));
        let document_handle = dom_host.document_handle();
        Self {
            owned_dom_host: Some(dom_host),
            parser_root_handle: Some(document_handle),
            parser_owner_document_handle: Some(document_handle),
            parser_document_url: Some(final_url),
            runtime_dom_sinks: None,
            next_template_contents_insertion: None,
            open_template_element_depth: 0,
            pending_open_parser_element: None,
            open_parser_elements: Vec::new(),
            allow_declarative_shadow_roots: false,
            pending_null_custom_element_registry_elements: Vec::new(),
            state: HtmlTreeSinkState::default(),
        }
    }

    pub(super) fn new_live_document_root(final_url: Url, document_handle: NativeNodeId) -> Self {
        Self::new_live_document_root_with_declarative_shadow_roots(final_url, document_handle, true)
    }

    pub(super) fn new_live_xml_document_root(
        final_url: Url,
        document_handle: NativeNodeId,
    ) -> Self {
        Self::new_live_document_root_with_declarative_shadow_roots(
            final_url,
            document_handle,
            false,
        )
    }

    fn new_live_document_root_with_declarative_shadow_roots(
        final_url: Url,
        document_handle: NativeNodeId,
        allow_declarative_shadow_roots: bool,
    ) -> Self {
        Self {
            owned_dom_host: None,
            parser_root_handle: Some(document_handle),
            parser_owner_document_handle: Some(document_handle),
            parser_document_url: Some(final_url),
            runtime_dom_sinks: None,
            next_template_contents_insertion: None,
            open_template_element_depth: 0,
            pending_open_parser_element: None,
            open_parser_elements: Vec::new(),
            allow_declarative_shadow_roots,
            pending_null_custom_element_registry_elements: Vec::new(),
            state: HtmlTreeSinkState::default(),
        }
    }

    pub(super) fn new_live_fragment_root(
        final_url: Url,
        fragment_handle: NativeNodeId,
        owner_document_handle: NativeNodeId,
        runtime_dom_sinks: ParserRuntimeDomSinks,
        allow_declarative_shadow_roots: bool,
    ) -> Self {
        Self {
            owned_dom_host: None,
            parser_root_handle: Some(fragment_handle),
            parser_owner_document_handle: Some(owner_document_handle),
            parser_document_url: Some(final_url),
            runtime_dom_sinks: Some(runtime_dom_sinks),
            next_template_contents_insertion: None,
            open_template_element_depth: 0,
            pending_open_parser_element: None,
            open_parser_elements: Vec::new(),
            allow_declarative_shadow_roots,
            pending_null_custom_element_registry_elements: Vec::new(),
            state: HtmlTreeSinkState::default(),
        }
    }

    fn dom_host(&self) -> &DomHost {
        assert!(
            self.runtime_dom_sinks.is_none(),
            "parser stream must use read/mutation sinks while a runtime-DOM sink step is active"
        );
        self.owned_dom_host
            .as_ref()
            .expect("parser stream has no owned DOM backend")
    }

    fn dom_host_mut(&mut self) -> &mut DomHost {
        assert!(
            self.runtime_dom_sinks.is_none(),
            "parser stream must use read/mutation sinks while a runtime-DOM sink step is active"
        );
        self.owned_dom_host
            .as_mut()
            .expect("parser stream has no owned DOM backend")
    }

    fn read_node_exists(&self, node_id: NativeNodeId) -> bool {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().node_exists(node_id)
        } else {
            self.dom_host().node(node_id).is_some()
        }
    }

    pub(super) fn snapshot_current_parser_document(&self) -> Option<NativeDom> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().snapshot_parser_document()
        } else {
            Some(self.dom_host().snapshot_document())
        }
    }

    fn read_is_connected(&self, node_id: NativeNodeId) -> bool {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().is_connected(node_id)
        } else {
            self.dom_host().is_connected(node_id)
        }
    }

    fn read_is_text_node(&self, node_id: NativeNodeId) -> bool {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().is_text_node(node_id)
        } else {
            self.dom_host()
                .node(node_id)
                .and_then(Node::as_text)
                .is_some()
        }
    }

    fn read_owner_document(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().owner_document(node_id)
        } else {
            self.dom_host().owner_document_handle(node_id)
        }
    }

    fn read_parent_node(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().parent_node(node_id)
        } else {
            self.dom_host().node(node_id).and_then(Node::parent_node)
        }
    }

    fn read_previous_sibling(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().previous_sibling(node_id)
        } else {
            self.dom_host().node(node_id).and_then(Node::prev_sibling)
        }
    }

    fn read_last_child(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().last_child(node_id)
        } else {
            self.dom_host().node(node_id).and_then(Node::last_child)
        }
    }

    fn read_child_handles(&self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().child_handles(node_id)
        } else {
            self.dom_host().child_handles(node_id).collect()
        }
    }

    fn read_document_base_url(&self) -> Option<Url> {
        let document_handle = self.parser_owner_document_node_id();
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().document_base_url(document_handle)
        } else {
            self.dom_host()
                .document_base_url_for_handle(document_handle)
        }
    }

    fn read_document_body_handle(&self) -> Option<NativeNodeId> {
        let document_handle = self.parser_owner_document_node_id();
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_read_sink()
                .document_body_handle_for_document(document_handle)
        } else {
            self.dom_host()
                .document_body_handle_for_document(document_handle)
        }
    }

    fn read_template_contents_handle(&self, node_id: NativeNodeId) -> Option<NativeNodeId> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().template_contents_handle(node_id)
        } else {
            template_contents_handle_in_host(self.dom_host(), node_id)
        }
    }

    fn read_is_html_element_named(&self, node_id: NativeNodeId, local_name: &str) -> bool {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_read_sink()
                .is_html_element_named(node_id, local_name)
        } else {
            is_html_element_named_in_host(self.dom_host(), node_id, local_name)
        }
    }

    fn read_is_external_async_classic_candidate(&self, node_id: NativeNodeId) -> bool {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_read_sink()
                .is_external_async_classic_candidate(node_id)
        } else {
            is_external_async_classic_candidate_in_host(self.dom_host(), node_id)
        }
    }

    fn read_parser_script(&self, node_id: NativeNodeId) -> Option<ParserScriptRead> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().parser_script_read(node_id)
        } else {
            parser_script_read_in_host(self.dom_host(), node_id)
        }
    }

    fn read_stylesheet_element(&self, node_id: NativeNodeId) -> Option<StylesheetElementRead> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().stylesheet_element(node_id)
        } else {
            stylesheet_element_in_host(self.dom_host(), node_id)
        }
    }

    fn read_text_content(&self, node_id: NativeNodeId) -> Option<String> {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_read_sink().text_content(node_id)
        } else {
            text_content_in_host(self.dom_host(), node_id)
        }
    }

    pub(super) fn parser_root_node_id(&self) -> NativeNodeId {
        self.parser_root_handle
            .expect("parser stream should record its parser root handle")
    }

    pub(super) fn parser_owner_document_node_id(&self) -> NativeNodeId {
        self.parser_owner_document_handle
            .expect("parser stream should record its owner Document handle")
    }

    fn parses_whole_document(&self) -> bool {
        self.parser_root_node_id() == self.parser_owner_document_node_id()
    }

    fn intended_parent_for_next_parser_element(&self) -> Option<NativeNodeId> {
        self.open_parser_elements
            .last()
            .map(|element| element.node_id)
            .or_else(|| Some(self.parser_root_node_id()))
    }

    fn owner_document_for_next_parser_node(&self) -> NativeNodeId {
        self.next_template_contents_insertion
            .and_then(|handle| self.read_owner_document(handle))
            .or_else(|| {
                self.intended_parent_for_next_parser_element()
                    .and_then(|handle| self.read_owner_document(handle))
            })
            .unwrap_or_else(|| self.parser_owner_document_node_id())
    }

    fn parser_document_url(&self) -> Option<&Url> {
        self.parser_document_url.as_ref()
    }

    fn collect_script_handles_from_parser_root(
        &self,
        include_template_contents: bool,
    ) -> Vec<NativeNodeId> {
        if !include_template_contents && let Some(owner) = &self.runtime_dom_sinks {
            return owner
                .dom_read_sink()
                .document_order_script_handles(self.parser_root_node_id());
        }
        let mut handles = Vec::new();
        self.collect_script_handles_from_node(
            self.parser_root_node_id(),
            include_template_contents,
            &mut handles,
        );
        handles
    }

    fn collect_script_handles_from_node(
        &self,
        current: NativeNodeId,
        include_template_contents: bool,
        handles: &mut Vec<NativeNodeId>,
    ) {
        if !self.read_node_exists(current) {
            return;
        }
        if self.read_parser_script(current).is_some() {
            handles.push(current);
        }
        if include_template_contents
            && let Some(template_contents) = self.read_template_contents_handle(current)
        {
            self.collect_script_handles_from_node(template_contents, true, handles);
        }
        for child in self.read_child_handles(current) {
            self.collect_script_handles_from_node(child, include_template_contents, handles);
        }
    }

    fn owned_dom_host_for_readback(&self) -> &DomHost {
        assert!(
            self.runtime_dom_sinks.is_none(),
            "cannot snapshot parser stream while a runtime-DOM sink step is active"
        );
        self.owned_dom_host
            .as_ref()
            .expect("owned parser stream DOM should exist for snapshot/readback")
    }

    pub(super) fn snapshot_parser_stream_document(&self) -> NativeDom {
        self.owned_dom_host_for_readback().snapshot_document()
    }

    pub(super) fn snapshot_parser_stream_dom_host(&self) -> DomHost {
        self.owned_dom_host_for_readback().clone()
    }

    pub(super) fn take_parser_stream_null_custom_element_registry_elements(
        &mut self,
    ) -> Vec<NativeNodeId> {
        std::mem::take(&mut self.pending_null_custom_element_registry_elements)
    }

    pub(super) fn take_parser_stream_document(&mut self) -> DomHost {
        assert!(
            self.runtime_dom_sinks.is_none(),
            "cannot take owned DomHost while a runtime-DOM sink step is active"
        );
        debug_assert!(
            self.parser_root_handle.is_some()
                && self.parser_owner_document_handle.is_some()
                && self.parser_document_url.is_some(),
            "parser stream should cache document handle/url before runtime takes the DOM"
        );
        self.owned_dom_host
            .take()
            .expect("parser stream bootstrap DOM should exist before runtime takeover")
    }

    fn mutation_effect_delivery(
        &self,
        effects: DomMutationEffects,
    ) -> ParserMutationEffectDelivery {
        ParserMutationEffectDelivery {
            effects,
            sink: self
                .runtime_dom_sinks
                .as_ref()
                .map(ParserRuntimeDomSinks::mutation_effect_sink),
            runtime_dom_sinks_active: self.runtime_dom_sinks.is_some(),
        }
    }

    fn apply_parser_dom_mutation(&mut self, mutation: ParserDomMutation) -> DomMutationEffects {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_mutation_sink().apply(mutation);
            DomMutationEffects::default()
        } else {
            mutation.apply_to_dom_host(self.dom_host_mut())
        }
    }

    fn record_parser_dom_mutation(
        &mut self,
        mutation: ParserDomMutation,
    ) -> ParserMutationEffectDelivery {
        let effects = self.apply_parser_dom_mutation(mutation);
        self.mutation_effect_delivery(effects)
    }

    pub(super) fn append_existing_node(&mut self, parent: NativeNodeId, child: NativeNodeId) {
        self.record_parser_dom_mutation(ParserDomMutation::AppendChild { parent, child })
            .consume();
    }

    pub(super) fn remove_existing_node(&mut self, parent: NativeNodeId, child: NativeNodeId) {
        self.record_parser_dom_mutation(ParserDomMutation::RemoveChild { parent, child })
            .consume();
    }

    fn note_parser_element_appended(&mut self, handle: &ParseHandle) {
        let Some(node_id) = handle.dom_node_id() else {
            return;
        };
        if handle.element_name.is_some()
            && self.pending_open_parser_element == Some(node_id)
            && self.read_node_exists(node_id)
        {
            self.open_parser_elements.push(OpenParserElement {
                node_id,
                name: handle
                    .element_name
                    .clone()
                    .expect("element handle should carry its qualified name"),
            });
            self.pending_open_parser_element = None;
            if self.state.parser_meta_csp_candidates.remove(&node_id)
                && self.read_is_connected(node_id)
                && self.read_owner_document(node_id) == Some(self.parser_owner_document_node_id())
            {
                self.state
                    .discovered_parser_meta_csp_candidates
                    .push_back(node_id);
            }
            self.note_blocking_stylesheet_pause_if_needed(node_id);
            let is_html_link = handle.element_name.as_ref().is_some_and(|name| {
                name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                    && name.local.as_ref().eq_ignore_ascii_case("link")
            });
            if is_html_link {
                // `<link>` is a void element, so html5ever does not necessarily
                // call TreeSink::pop(). Blink still runs
                // HTMLLinkElement::FinishParsingChildren at this token
                // boundary; consume the link-specific parser state here.
                self.note_parser_element_popped(node_id);
            }
        }
    }

    fn note_blocking_stylesheet_pause_if_needed(&mut self, node_id: NativeNodeId) {
        if let Some(pending) = self.state.pending_blocking_stylesheet_pause {
            debug_assert_eq!(
                pending, node_id,
                "parser cannot discover another blocking stylesheet before yielding the pending pause"
            );
            return;
        }
        if !self.capture_parser_blocking_stylesheet(node_id) {
            return;
        }
        if self.state.finishing_tree_builder {
            // EOF can close an unterminated body <style>. Its operation still
            // delays load, but there is no remaining tokenizer input to pause.
            return;
        }
        let Some(body) = self.read_document_body_handle() else {
            return;
        };
        let mut ancestor = self.read_parent_node(node_id);
        let mut is_in_body = false;
        while let Some(current) = ancestor {
            if current == body {
                is_in_body = true;
                break;
            }
            ancestor = self.read_parent_node(current);
        }
        if !is_in_body {
            return;
        }
        self.state.pending_blocking_stylesheet_pause = Some(node_id);
    }

    fn capture_parser_blocking_stylesheet(&mut self, node_id: NativeNodeId) -> bool {
        // Parser-created stylesheets only block the parser when they belong to
        // its connected Document tree.  Nodes in HTMLTemplateElement.content
        // are owned by an inert template Document and must never start the
        // main Document's resource lifecycle.
        if !self.read_is_connected(node_id)
            || self.read_owner_document(node_id) != Some(self.parser_owner_document_node_id())
        {
            return false;
        }
        let Some(candidate) = document_owned_blocking_stylesheet_candidate_for_node(
            self,
            NodeId::new(node_id.index()),
        ) else {
            return false;
        };
        if self
            .state
            .captured_blocking_stylesheet_nodes
            .insert(node_id)
        {
            let input =
                moli_stylesheet_blocking::DocumentOwnedBlockingStylesheetDiscoveryInput::from(
                    &candidate,
                );
            self.state
                .captured_blocking_stylesheet_signatures
                .insert(input.signature().clone());
            self.state
                .discovered_blocking_stylesheet_inputs
                .push_back(input);
        }
        true
    }

    fn note_parser_element_popped(&mut self, node_id: NativeNodeId) {
        if let Some(index) = self
            .open_parser_elements
            .iter()
            .rposition(|candidate| candidate.node_id == node_id)
        {
            self.open_parser_elements.remove(index);
        }
        if self.read_is_html_element_named(node_id, "link") {
            let _ = self.capture_parser_blocking_stylesheet(node_id);
            self.finish_parsing_link_children_for_dom_host(node_id);
        }
    }

    pub(super) fn note_foreign_end_tag_processed(
        &mut self,
        local_name: &str,
    ) -> Option<NativeNodeId> {
        let mut first = true;
        let mut matching_index = None;
        for (index, element) in self.open_parser_elements.iter().enumerate().rev() {
            let is_html = element.name.ns.as_ref() == "http://www.w3.org/1999/xhtml";
            if !first && is_html {
                return None;
            }
            if element.name.local.as_ref().eq_ignore_ascii_case(local_name) {
                matching_index = Some(index);
                break;
            }
            first = false;
        }
        let matching_index = matching_index?;
        let matching_node = self.open_parser_elements[matching_index].node_id;
        let matching_svg_script = local_name.eq_ignore_ascii_case("script")
            && self.read_parser_script(matching_node).is_some();
        let closed = self.open_parser_elements[matching_index..]
            .iter()
            .rev()
            .map(|element| element.node_id)
            .collect::<Vec<_>>();
        for node_id in closed {
            if matching_svg_script && node_id == matching_node {
                self.note_node_closed(node_id, true);
            } else {
                self.note_node_popped_without_script_handoff(node_id);
            }
        }
        matching_svg_script.then_some(matching_node)
    }

    pub(super) fn note_self_closing_foreign_element_processed(
        &mut self,
        local_name: &str,
    ) -> Option<NativeNodeId> {
        let element = self.open_parser_elements.last()?;
        if element.name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
            || !element.name.local.as_ref().eq_ignore_ascii_case(local_name)
        {
            return None;
        }
        let node_id = element.node_id;
        let svg_script = self.read_parser_script(node_id).is_some();
        self.note_node_closed(node_id, svg_script);
        svg_script.then_some(node_id)
    }

    pub(super) fn create_parser_element_without_attributes(
        &mut self,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        let document_handle = self.parser_owner_document_node_id();
        self.create_parser_element_without_attributes_for_document(
            document_handle,
            local_name,
            namespace,
            prefix,
        )
    }

    fn create_parser_element_without_attributes_for_document(
        &mut self,
        document_handle: NativeNodeId,
        local_name: String,
        namespace: String,
        prefix: Option<String>,
    ) -> NativeNodeId {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .create_parser_element_for_document_without_attributes(
                    document_handle,
                    local_name,
                    namespace,
                    prefix,
                )
        } else {
            self.dom_host_mut()
                .create_parser_element_without_attributes_for_document(
                    document_handle,
                    local_name,
                    namespace,
                    prefix,
                )
        }
    }

    pub(super) fn add_attrs_if_missing_for_parser(
        &mut self,
        node_id: NativeNodeId,
        attrs: Vec<NativeAttribute>,
    ) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .add_attrs_if_missing_for_parser(node_id, attrs);
        } else {
            self.dom_host_mut()
                .add_attrs_if_missing_for_parser(node_id, attrs);
        }
    }

    pub(super) fn create_text_node_for_document(
        &mut self,
        document_handle: NativeNodeId,
        text: String,
    ) -> NativeNodeId {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .create_text_node(document_handle, text)
        } else {
            self.dom_host_mut()
                .create_text_node_for_document(document_handle, &text)
        }
    }

    pub(super) fn create_comment_node(&mut self, text: String) -> NativeNodeId {
        let document_handle = self.owner_document_for_next_parser_node();
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .create_comment(document_handle, text)
        } else {
            self.dom_host_mut()
                .create_comment_for_document(document_handle, &text)
        }
    }

    pub(super) fn create_processing_instruction_node(
        &mut self,
        target: String,
        data: String,
    ) -> NativeNodeId {
        let document_handle = self.owner_document_for_next_parser_node();
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .create_processing_instruction(document_handle, target, data)
        } else {
            self.dom_host_mut()
                .create_processing_instruction_for_document(document_handle, &target, &data)
        }
    }

    pub(super) fn create_cdata_section_node(&mut self, data: String) -> NativeNodeId {
        let document_handle = self.owner_document_for_next_parser_node();
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .create_cdata_section(document_handle, data)
        } else {
            self.dom_host_mut()
                .create_cdata_section_for_document(document_handle, &data)
        }
    }

    pub(super) fn create_document_type_node(
        &mut self,
        name: String,
        public_id: String,
        system_id: String,
    ) -> NativeNodeId {
        let document_handle = self.parser_owner_document_node_id();
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_mutation_sink().create_document_type(
                document_handle,
                name,
                public_id,
                system_id,
            )
        } else {
            self.dom_host_mut().create_document_type_for_document(
                document_handle,
                &name,
                &public_id,
                &system_id,
            )
        }
    }

    fn prepend_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .prepend_text_to_text_node(node_id, text);
        } else {
            prepend_text_to_text_node_in_host(self.dom_host_mut(), node_id, text);
        }
    }

    fn append_text_to_text_node(&mut self, node_id: NativeNodeId, text: String) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .append_text_to_text_node(node_id, text);
        } else {
            append_text_to_text_node_in_host(self.dom_host_mut(), node_id, text);
        }
    }

    fn push_parse_error_to_dom_host(&mut self, error: String) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner.dom_mutation_sink().push_parse_error(error);
        } else {
            self.dom_host_mut().push_parse_error(error);
        }
    }

    fn set_html_quirks_mode_for_dom_host(&mut self, quirks_mode: QuirksMode) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .set_html_quirks_mode_for_parser(quirks_mode);
        } else {
            self.dom_host_mut()
                .set_html_quirks_mode_for_parser(quirks_mode);
        }
    }

    fn mark_script_already_started_for_dom_host(&mut self, node_id: NativeNodeId) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .mark_script_already_started_for_parser(node_id);
        } else {
            let _ = self
                .dom_host_mut()
                .set_script_already_started(node_id, true);
        }
    }

    fn mark_unclosed_form_control_for_dom_host(&mut self, node_id: NativeNodeId) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .mark_unclosed_form_control_for_parser(node_id);
        } else {
            let _ = self
                .dom_host_mut()
                .set_blocks_form_submission(node_id, true);
        }
    }

    fn finish_parsing_script_children_for_dom_host(&mut self, node_id: NativeNodeId) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .finish_parsing_script_children(node_id);
        } else {
            let _ = self.dom_host_mut().finish_parsing_script_children(node_id);
        }
    }

    fn finish_parsing_link_children_for_dom_host(&mut self, node_id: NativeNodeId) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .finish_parsing_link_children(node_id);
        } else {
            let _ = self.dom_host_mut().finish_parsing_link_children(node_id);
        }
    }

    pub(super) fn maybe_clone_an_option_into_selectedcontent(&mut self, node_id: NativeNodeId) {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .maybe_clone_an_option_into_selectedcontent(node_id);
        }
    }

    fn attach_declarative_shadow_for_dom_host(
        &mut self,
        host_id: NativeNodeId,
        template_id: NativeNodeId,
        attrs: Vec<NativeAttribute>,
    ) -> bool {
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_mutation_sink()
                .attach_declarative_shadow_for_parser(host_id, template_id, attrs)
        } else {
            self.dom_host_mut()
                .attach_declarative_shadow_for_parser(host_id, template_id, &attrs)
        }
    }

    pub(super) fn drain_ready_parser_scripts(&mut self) -> Vec<NativeNodeId> {
        self.state.ready_parser_scripts.drain(..).collect()
    }

    pub(super) fn drain_discovered_async_prefetch_candidates(&mut self) -> Vec<NativeNodeId> {
        self.state
            .discovered_async_prefetch_candidates
            .drain(..)
            .collect()
    }

    pub(super) fn drain_discovered_modulepreload_link_candidates(&mut self) -> Vec<NativeNodeId> {
        self.state
            .discovered_modulepreload_link_candidates
            .drain(..)
            .collect()
    }

    pub(super) fn drain_discovered_parser_meta_csp_candidates(&mut self) -> Vec<NativeNodeId> {
        self.state
            .discovered_parser_meta_csp_candidates
            .drain(..)
            .collect()
    }

    pub(super) fn note_defined_autonomous_custom_element(&mut self, local_name: &str) {
        self.state
            .defined_autonomous_custom_elements
            .insert(local_name.to_ascii_lowercase());
    }

    pub(super) fn drain_pending_custom_element_construction_handoffs(
        &mut self,
    ) -> Vec<ParserCustomElementConstructionHandoff> {
        self.state
            .pending_custom_element_construction_handoffs
            .drain(..)
            .collect()
    }

    pub(super) fn has_pending_custom_element_construction_handoff(&self) -> bool {
        !self
            .state
            .pending_custom_element_construction_handoffs
            .is_empty()
    }

    pub(super) fn front_pending_custom_element_construction_handoff(
        &self,
    ) -> Option<&ParserCustomElementConstructionHandoff> {
        self.state
            .pending_custom_element_construction_handoffs
            .front()
    }

    pub(super) fn pop_pending_custom_element_construction_handoff(
        &mut self,
    ) -> Option<ParserCustomElementConstructionHandoff> {
        self.state
            .pending_custom_element_construction_handoffs
            .pop_front()
    }

    pub(super) fn front_pending_blocking_stylesheet_pause(&self) -> Option<NativeNodeId> {
        self.state.pending_blocking_stylesheet_pause
    }

    pub(super) fn pop_pending_blocking_stylesheet_pause(&mut self) -> Option<NativeNodeId> {
        self.state.pending_blocking_stylesheet_pause.take()
    }

    pub(super) fn begin_tree_builder_finish(&mut self, unclosed_form_controls: &[NativeNodeId]) {
        self.state.finishing_tree_builder = true;
        for &node_id in unclosed_form_controls {
            self.mark_unclosed_form_control_for_dom_host(node_id);
        }
    }

    pub(super) fn drain_discovered_blocking_stylesheet_inputs(
        &mut self,
    ) -> Vec<moli_stylesheet_blocking::DocumentOwnedBlockingStylesheetDiscoveryInput> {
        self.state
            .discovered_blocking_stylesheet_inputs
            .drain(..)
            .collect()
    }

    pub(super) fn captured_blocking_stylesheet_signatures(
        &self,
    ) -> std::collections::HashSet<moli_stylesheet_blocking::DocumentBlockingStylesheetSignature>
    {
        self.state.captured_blocking_stylesheet_signatures.clone()
    }

    fn parser_custom_element_handoff_for_created_element(
        &self,
        node_id: NativeNodeId,
        token_local_name: &str,
        token_namespace: &str,
        token_prefix: Option<&str>,
        token_attributes: &[NativeAttribute],
    ) -> Option<ParserCustomElementConstructionHandoff> {
        let (local_name, namespace, prefix, attributes) = if self.runtime_dom_sinks.is_some() {
            (
                token_local_name,
                token_namespace,
                token_prefix.map(str::to_owned),
                token_attributes.to_vec(),
            )
        } else {
            let node = self.dom_host().node(node_id)?;
            let element = node.as_element()?;
            (
                element.local_name(),
                element.namespace(),
                element.prefix().map(str::to_owned),
                element.attributes().to_vec(),
            )
        };
        if namespace != "http://www.w3.org/1999/xhtml" {
            return None;
        }
        if !self
            .state
            .defined_autonomous_custom_elements
            .contains(local_name)
        {
            return None;
        }
        Some(ParserCustomElementConstructionHandoff {
            placeholder: node_id,
            local_name: local_name.to_owned(),
            namespace: namespace.to_owned(),
            prefix,
            attributes,
            owner_document: self.read_owner_document(node_id)?,
            parent_at_creation: self.read_parent_node(node_id),
        })
    }

    fn append_text(
        &mut self,
        parent_id: NativeNodeId,
        text: String,
    ) -> ParserMutationEffectDelivery {
        self.insert_text_node(parent_id, None, text)
    }

    fn insert_text_node(
        &mut self,
        parent_id: NativeNodeId,
        reference_child: Option<NativeNodeId>,
        text: String,
    ) -> ParserMutationEffectDelivery {
        if text.is_empty() || !self.read_node_exists(parent_id) {
            return ParserMutationEffectDelivery::none();
        }

        if let Some(reference_child) = reference_child {
            if self.read_is_text_node(reference_child) {
                self.prepend_text_to_text_node(reference_child, text);
                return ParserMutationEffectDelivery::none();
            }

            if let Some(previous) = self.read_previous_sibling(reference_child)
                && self.read_is_text_node(previous)
            {
                self.append_text_to_text_node(previous, text);
                return ParserMutationEffectDelivery::none();
            }
        } else if let Some(last_child) = self.read_last_child(parent_id)
            && self.read_is_text_node(last_child)
        {
            self.append_text_to_text_node(last_child, text);
            return ParserMutationEffectDelivery::none();
        }

        let document_handle = self
            .read_owner_document(parent_id)
            .unwrap_or_else(|| self.parser_owner_document_node_id());
        let text_node = self.create_text_node_for_document(document_handle, text);
        if let Some(reference_child) = reference_child {
            let Some(parent) = self.read_parent_node(reference_child) else {
                return ParserMutationEffectDelivery::none();
            };
            self.record_parser_dom_mutation(ParserDomMutation::InsertBefore {
                parent,
                child: text_node,
                reference_child: Some(reference_child),
            })
        } else {
            self.record_parser_dom_mutation(ParserDomMutation::AppendChild {
                parent: parent_id,
                child: text_node,
            })
        }
    }

    fn is_external_async_classic_candidate(&self, node_id: NativeNodeId) -> bool {
        self.read_is_external_async_classic_candidate(node_id)
    }

    pub(super) fn push_parse_error(&mut self, error: String) {
        if self.parses_whole_document() {
            self.push_parse_error_to_dom_host(error);
        }
    }

    pub(super) fn document_handle(&self) -> ParseHandle {
        ParseHandle::new(self.parser_root_node_id(), None)
    }

    pub(super) fn set_html_quirks_mode(&mut self, quirks_mode: QuirksMode) {
        self.state.html_quirks_mode = quirks_mode;
        if self.parses_whole_document() {
            self.set_html_quirks_mode_for_dom_host(quirks_mode);
        }
    }

    pub(super) fn set_current_position(&mut self, line_number: u64, column_number: u64) {
        self.state.current_position = crate::ParserSourcePosition {
            line: line_number,
            column: column_number,
        };
    }

    pub(super) fn script_start_line(&self, node_id: NativeNodeId) -> Option<u64> {
        self.script_start_position(node_id)
            .map(|(line, _column)| line)
    }

    pub(super) fn script_start_position(&self, node_id: NativeNodeId) -> Option<(u64, u64)> {
        self.state
            .script_start_positions
            .get(&node_id)
            .map(|position| (position.line, position.column))
    }

    pub(super) fn create_element(
        &mut self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
        script_nonceable: Option<bool>,
    ) -> ParseHandle {
        let parser_flags = ParserElementFlags::from_html5ever(&flags);
        let template_contents_insertion = self.take_template_contents_insertion_hint();
        let insertion_is_template_contents =
            self.open_template_element_depth > 0 || template_contents_insertion.is_some();
        let intended_parent = self.intended_parent_for_next_parser_element();
        let document_handle = template_contents_insertion
            .and_then(|handle| self.read_owner_document(handle))
            .or_else(|| intended_parent.and_then(|handle| self.read_owner_document(handle)))
            .unwrap_or_else(|| self.parser_owner_document_node_id());
        let element_name = Rc::new(name.clone());
        let local_name = name.local.to_string();
        let namespace = name.ns.to_string();
        let is_script = local_name == "script"
            && matches!(
                namespace.as_str(),
                "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
            );
        let prefix = name.prefix.as_ref().map(|prefix| prefix.to_string());
        let mut has_null_custom_element_registry_attribute = false;
        let attributes: Vec<NativeAttribute> = attrs
            .into_iter()
            .map(|attribute| {
                if attribute.name.ns.to_string().is_empty()
                    && attribute
                        .name
                        .local
                        .as_ref()
                        .eq_ignore_ascii_case("customelementregistry")
                {
                    has_null_custom_element_registry_attribute = true;
                }
                NativeAttribute::new(
                    attribute.name.local.to_string(),
                    attribute.name.ns.to_string(),
                    attribute
                        .name
                        .prefix
                        .as_ref()
                        .map(|prefix| prefix.to_string()),
                    attribute.value.to_string(),
                )
            })
            .collect();

        if flags.template {
            self.open_template_element_depth = self.open_template_element_depth.saturating_add(1);
        }

        if !insertion_is_template_contents
            && let Some(sink) = self
                .runtime_dom_sinks
                .as_ref()
                .and_then(ParserRuntimeDomSinks::element_creation_sink)
        {
            let request = ParserElementCreationRequest {
                document_handle,
                intended_parent,
                local_name: &local_name,
                namespace: &namespace,
                prefix: prefix.as_deref(),
                attributes: &attributes,
            };
            if let Some(node_id) = sink.create_parser_element(request)
                && self.read_node_exists(node_id)
            {
                return self.finish_created_element(
                    node_id,
                    is_script,
                    element_name,
                    &local_name,
                    &namespace,
                    prefix.as_deref(),
                    &attributes,
                    has_null_custom_element_registry_attribute,
                    parser_flags,
                    script_nonceable,
                );
            }
        }

        let token_local_name = local_name.clone();
        let token_namespace = namespace.clone();
        let token_prefix = prefix.clone();
        let token_attributes = attributes.clone();
        let node_id = self.create_parser_element_without_attributes_for_document(
            document_handle,
            local_name,
            namespace,
            prefix,
        );
        self.add_attrs_if_missing_for_parser(node_id, attributes);
        self.finish_created_element(
            node_id,
            is_script,
            element_name,
            &token_local_name,
            &token_namespace,
            token_prefix.as_deref(),
            &token_attributes,
            has_null_custom_element_registry_attribute,
            parser_flags,
            script_nonceable,
        )
    }

    fn finish_created_element(
        &mut self,
        node_id: NativeNodeId,
        is_script: bool,
        element_name: Rc<QualName>,
        token_local_name: &str,
        token_namespace: &str,
        token_prefix: Option<&str>,
        token_attributes: &[NativeAttribute],
        has_null_custom_element_registry_attribute: bool,
        parser_flags: ParserElementFlags,
        script_nonceable: Option<bool>,
    ) -> ParseHandle {
        if has_null_custom_element_registry_attribute {
            self.pending_null_custom_element_registry_elements
                .push(node_id);
        }
        if is_script {
            if script_nonceable == Some(false) {
                self.state.non_nonceable_parser_scripts.insert(node_id);
            }
            self.state
                .script_start_positions
                .insert(node_id, self.state.current_position);
        }
        if self.is_external_async_classic_candidate(node_id) {
            self.state
                .discovered_async_prefetch_candidates
                .push_back(node_id);
        }
        if is_modulepreload_link_candidate(token_local_name, token_namespace, token_attributes) {
            self.state
                .discovered_modulepreload_link_candidates
                .push_back(node_id);
        }
        if is_meta_csp_candidate(token_local_name, token_namespace, token_attributes) {
            self.state.parser_meta_csp_candidates.insert(node_id);
        }
        self.pending_open_parser_element = Some(node_id);
        if let Some(handoff) = self.parser_custom_element_handoff_for_created_element(
            node_id,
            token_local_name,
            token_namespace,
            token_prefix,
            token_attributes,
        ) {
            self.state
                .pending_custom_element_construction_handoffs
                .push_back(handoff);
        }
        ParseHandle::new_element(node_id, element_name, parser_flags)
    }

    pub(super) fn create_comment(&mut self, text: String) -> ParseHandle {
        ParseHandle::new(self.create_comment_node(text), None)
    }

    pub(super) fn create_processing_instruction(
        &mut self,
        target: String,
        data: String,
    ) -> ParseHandle {
        ParseHandle::new(self.create_processing_instruction_node(target, data), None)
    }

    pub(super) fn create_cdata_section(&mut self, data: String) -> ParseHandle {
        ParseHandle::new(self.create_cdata_section_node(data), None)
    }

    pub(super) fn append(
        &mut self,
        parent_id: NativeNodeId,
        child: NodeOrText<ParseHandle>,
    ) -> ParserMutationEffectDelivery {
        self.clear_template_contents_insertion_hint();
        match child {
            NodeOrText::AppendNode(handle) => {
                let child_id = handle.node_id();
                let delivery = self.record_parser_dom_mutation(ParserDomMutation::AppendChild {
                    parent: parent_id,
                    child: child_id,
                });
                self.note_parser_element_appended(&handle);
                delivery
            }
            NodeOrText::AppendText(text) => self.append_text(parent_id, text.to_string()),
        }
    }

    pub(super) fn append_before_sibling(
        &mut self,
        sibling_id: NativeNodeId,
        child: NodeOrText<ParseHandle>,
    ) -> ParserMutationEffectDelivery {
        self.clear_template_contents_insertion_hint();
        let Some(parent_id) = self.read_parent_node(sibling_id) else {
            return ParserMutationEffectDelivery::none();
        };

        match child {
            NodeOrText::AppendNode(handle) => {
                let child_id = handle.node_id();
                let delivery = self.record_parser_dom_mutation(ParserDomMutation::InsertBefore {
                    parent: parent_id,
                    child: child_id,
                    reference_child: Some(sibling_id),
                });
                self.note_parser_element_appended(&handle);
                delivery
            }
            NodeOrText::AppendText(text) => {
                self.insert_text_node(parent_id, Some(sibling_id), text.to_string())
            }
        }
    }

    pub(super) fn append_based_on_parent_node(
        &mut self,
        element_id: NativeNodeId,
        prev_element_id: NativeNodeId,
        child: NodeOrText<ParseHandle>,
    ) -> ParserMutationEffectDelivery {
        self.clear_template_contents_insertion_hint();
        if self.read_parent_node(element_id).is_some() {
            self.append_before_sibling(element_id, child)
        } else {
            self.append(prev_element_id, child)
        }
    }

    pub(super) fn append_doctype(
        &mut self,
        name: String,
        public_id: String,
        system_id: String,
    ) -> ParserMutationEffectDelivery {
        let document_handle = self.parser_root_node_id();
        let doctype = self.create_document_type_node(name, public_id, system_id);
        self.record_parser_dom_mutation(ParserDomMutation::AppendChild {
            parent: document_handle,
            child: doctype,
        })
    }

    pub(super) fn template_contents_handle(
        &mut self,
        node_id: NativeNodeId,
    ) -> Option<ParseHandle> {
        let handle = self
            .read_template_contents_handle(node_id)
            .map(|handle| ParseHandle::new(handle, None));
        if let Some(handle) = handle.as_ref() {
            self.next_template_contents_insertion = Some(handle.node_id());
        }
        handle
    }

    pub(super) fn template_contents_parent_handle(
        &self,
        node_id: NativeNodeId,
    ) -> Option<ParseHandle> {
        self.read_template_contents_handle(node_id)
            .map(|handle| ParseHandle::new(handle, None))
    }

    fn take_template_contents_insertion_hint(&mut self) -> Option<NativeNodeId> {
        self.next_template_contents_insertion.take()
    }

    fn clear_template_contents_insertion_hint(&mut self) {
        self.next_template_contents_insertion = None;
    }

    pub(super) fn attach_declarative_shadow(
        &mut self,
        host_id: NativeNodeId,
        template_id: NativeNodeId,
        attrs: &[Attribute],
    ) -> bool {
        if !self.allow_declarative_shadow_roots {
            return false;
        }
        let attrs = attrs
            .iter()
            .map(|attribute| {
                NativeAttribute::new(
                    attribute.name.local.to_string(),
                    attribute.name.ns.to_string(),
                    attribute
                        .name
                        .prefix
                        .as_ref()
                        .map(|prefix| prefix.to_string()),
                    attribute.value.to_string(),
                )
            })
            .collect::<Vec<_>>();
        self.attach_declarative_shadow_for_dom_host(host_id, template_id, attrs)
    }

    pub(super) fn add_attrs_if_missing(&mut self, node_id: NativeNodeId, attrs: Vec<Attribute>) {
        let attrs = attrs
            .into_iter()
            .map(|attribute| {
                NativeAttribute::new(
                    attribute.name.local.to_string(),
                    attribute.name.ns.to_string(),
                    attribute.name.prefix.map(|prefix| prefix.to_string()),
                    attribute.value.to_string(),
                )
            })
            .collect();
        self.add_attrs_if_missing_for_parser(node_id, attrs);
    }

    pub(super) fn associate_with_form(&mut self, target: NativeNodeId, form: NativeNodeId) {
        if let Some(owner) = &self.runtime_dom_sinks {
            let _ = owner
                .dom_mutation_sink()
                .associate_parser_form_owner(target, form);
        } else {
            let _ = self
                .dom_host_mut()
                .associate_parser_form_owner(target, form);
        }
    }

    pub(super) fn remove_from_parent(
        &mut self,
        node_id: NativeNodeId,
    ) -> ParserMutationEffectDelivery {
        let Some(parent) = self.read_parent_node(node_id) else {
            return ParserMutationEffectDelivery::none();
        };
        self.record_parser_dom_mutation(ParserDomMutation::RemoveChild {
            parent,
            child: node_id,
        })
    }

    pub(super) fn reparent_children(
        &mut self,
        node_id: NativeNodeId,
        new_parent_id: NativeNodeId,
    ) -> ParserMutationEffectDelivery {
        let child_ids = self.read_child_handles(node_id);
        let mut effects = DomMutationEffects::default();
        for child_id in child_ids {
            effects.merge(
                self.apply_parser_dom_mutation(ParserDomMutation::AppendChild {
                    parent: new_parent_id,
                    child: child_id,
                }),
            );
        }
        self.mutation_effect_delivery(effects)
    }

    pub(super) fn mark_script_already_started(&mut self, node_id: NativeNodeId) {
        self.mark_script_already_started_for_dom_host(node_id);
    }

    pub(super) fn note_node_closed(&mut self, node_id: NativeNodeId, is_script_element: bool) {
        if is_script_element {
            self.finish_parsing_script_children_for_dom_host(node_id);
            self.state.ready_parser_scripts.push_back(node_id);
        }
        self.note_node_popped_without_script_handoff(node_id);
    }

    fn note_node_popped_without_script_handoff(&mut self, node_id: NativeNodeId) {
        if self.read_is_html_element_named(node_id, "template") {
            self.open_template_element_depth = self.open_template_element_depth.saturating_sub(1);
        }
        if self.read_is_html_element_named(node_id, "style") {
            self.note_blocking_stylesheet_pause_if_needed(node_id);
        }
        self.note_parser_element_popped(node_id);
    }

    pub(super) fn restore_parser_stream_dom_host(&mut self, dom_host: DomHost) {
        assert!(
            self.owned_dom_host.is_none() && self.runtime_dom_sinks.is_none(),
            "owned parser stream DOM should only be restored after take_parser_stream_document()"
        );
        self.owned_dom_host = Some(dom_host);
    }

    pub(super) fn enter_runtime_dom_sinks_parse_step(&mut self, sinks: ParserRuntimeDomSinks) {
        assert!(
            self.owned_dom_host.is_none() && self.runtime_dom_sinks.is_none(),
            "parser target should not already hold an owned DOM backend or sink bundle when entering a runtime-DOM sink step"
        );
        self.runtime_dom_sinks = Some(sinks);
    }

    #[cfg(test)]
    fn with_runtime_dom_consumer_for_test<T, R>(
        &mut self,
        consumer: &mut T,
        op: impl FnOnce(&mut Self) -> R,
    ) -> R
    where
        T: ParserDomReadConsumer + ParserDomMutationConsumer + ParserMutationEffectConsumer,
    {
        // SAFETY: `consumer` remains exclusively borrowed until the target-step
        // guard clears every erased callback, including during unwinding.
        let sinks =
            unsafe { ParserRuntimeDomSinks::from_consumer_without_element_creation(consumer) };
        self.enter_runtime_dom_sinks_parse_step(sinks);
        let step = ParserRuntimeDomTargetStep { target: self };
        op(&mut *step.target)
    }

    /// Clears the runtime DOM sinks after a parse pump step.
    pub(super) fn clear_runtime_dom_sinks_after_parse_step(&mut self) {
        assert!(
            self.runtime_dom_sinks.is_some(),
            "parser target should hold runtime DOM sinks during a runtime-DOM sink step"
        );
        self.runtime_dom_sinks = None;
    }

    pub(super) fn replace_parser_stream_document(&mut self, document: NativeDom) {
        self.parser_root_handle = Some(document.document_node_id());
        self.parser_owner_document_handle = Some(document.document_node_id());
        self.parser_document_url = document.final_url().cloned();
        self.owned_dom_host = Some(DomHost::from_dom(document));
    }

    pub(super) fn finish_dom_host(self) -> DomHost {
        assert!(
            self.runtime_dom_sinks.is_none(),
            "cannot finish parser stream while a runtime-DOM sink step is active"
        );
        self.owned_dom_host
            .expect("owned parser stream DOM should exist when finishing the parser stream")
    }

    pub(super) fn finish_document(self, _html: String) -> NativeDom {
        self.finish_dom_host().into_dom()
    }
}

pub(super) fn new_parser_stream_html_tree_sink_target(
    final_url: Url,
    scripting_enabled: bool,
) -> ParserStreamHtmlTreeSinkTarget {
    ParserStreamHtmlTreeSinkTarget::new_with_declarative_shadow_roots_and_scripting(
        final_url,
        true,
        scripting_enabled,
    )
}

pub(super) fn new_parser_stream_html_tree_sink_stream(
    final_url: Url,
    scripting_enabled: bool,
) -> HtmlTreeSinkStream {
    HtmlTreeSinkStream::from_target_with_scripting(
        new_parser_stream_html_tree_sink_target(final_url, scripting_enabled),
        scripting_enabled,
    )
}

pub(super) fn new_live_document_root_html_tree_sink_stream(
    final_url: Url,
    document_handle: NativeNodeId,
    scripting_enabled: bool,
) -> HtmlTreeSinkStream {
    HtmlTreeSinkStream::from_target_with_scripting(
        ParserStreamHtmlTreeSinkTarget::new_live_document_root(final_url, document_handle),
        scripting_enabled,
    )
}

pub(super) fn new_live_fragment_root_html_tree_sink_stream(
    final_url: Url,
    fragment_handle: NativeNodeId,
    owner_document_handle: NativeNodeId,
    context_handle: NativeNodeId,
    context_namespace: &str,
    context_local_name: &str,
    runtime_dom_sinks: ParserRuntimeDomSinks,
    allow_declarative_shadow_roots: bool,
    scripting_enabled: bool,
) -> HtmlTreeSinkStream {
    let mut stream = HtmlTreeSinkStream::from_fragment_target(
        ParserStreamHtmlTreeSinkTarget::new_live_fragment_root(
            final_url,
            fragment_handle,
            owner_document_handle,
            runtime_dom_sinks,
            allow_declarative_shadow_roots,
        ),
        context_handle,
        context_namespace,
        context_local_name,
        scripting_enabled,
    );
    stream.clear_runtime_dom_sinks_after_parse_step();
    stream
}

impl ParserPlanningReadView for ParserStreamHtmlTreeSinkTarget {
    fn parser_script_read(&self, node_id: NativeNodeId) -> Option<ParserScriptRead> {
        let mut script = self.read_parser_script(node_id)?;
        if self.state.non_nonceable_parser_scripts.contains(&node_id) {
            script.fetch_metadata.nonce = None;
        }
        Some(script)
    }

    fn script_handles(&self) -> Vec<NativeNodeId> {
        self.collect_script_handles_from_parser_root(true)
    }

    fn is_connected(&self, node_id: NativeNodeId) -> bool {
        self.read_is_connected(node_id)
    }

    fn document_order_script_handles(&self) -> Vec<NativeNodeId> {
        self.collect_script_handles_from_parser_root(false)
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.parser_document_url().cloned()
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        self.read_document_base_url()
            .or_else(|| self.parser_document_url().cloned())
    }

    fn script_start_line(&self, node_id: NativeNodeId) -> Option<u64> {
        self.script_start_line(node_id)
    }
}

impl StylesheetBlockingReadView for ParserStreamHtmlTreeSinkTarget {
    fn stylesheet_element(&self, node_id: NativeNodeId) -> Option<StylesheetElementRead> {
        self.read_stylesheet_element(node_id)
    }

    fn child_ids(&self, node_id: NativeNodeId) -> Vec<NativeNodeId> {
        self.read_child_handles(node_id)
    }

    fn text_content(&self, node_id: NativeNodeId) -> Option<String> {
        self.read_text_content(node_id)
    }

    fn final_url_clone(&self) -> Option<Url> {
        self.parser_document_url().cloned()
    }

    fn document_base_url_clone(&self) -> Option<Url> {
        self.read_document_base_url()
            .or_else(|| self.parser_document_url().cloned())
    }

    fn document_node_id(&self) -> NativeNodeId {
        self.parser_owner_document_node_id()
    }

    fn document_is_quirks_mode(&self) -> bool {
        matches!(self.state.html_quirks_mode, QuirksMode::Quirks)
    }

    fn document_order_stylesheet_candidate_ids_before(
        &self,
        target_node_id: Option<NodeId>,
    ) -> Vec<NativeNodeId> {
        let stop_at = target_node_id.map(|node_id| NativeNodeId::new(node_id.index()));
        if let Some(owner) = &self.runtime_dom_sinks {
            owner
                .dom_read_sink()
                .document_order_stylesheet_candidate_handles_before(
                    self.parser_root_node_id(),
                    stop_at,
                )
        } else {
            self.dom_host()
                .stylesheet_candidate_handles_before_in_tree_scope(
                    self.parser_root_node_id(),
                    stop_at,
                )
        }
    }
}

#[test]
fn parser_stream_html_tree_sink_target_builds_dom_and_records_parser_state() {
    let html = "<!doctype html><html><head><template id=t><span>inner</span></template><script>window.inline = true;</script><script async src=\"/async.js\"></script></head><body><div>hello</div></body></html>";
    let url = Url::parse("https://example.test/").expect("test url");

    let mut live =
        crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url.clone());
    for chunk in html_chunks(html) {
        live.feed(chunk);
    }
    let parser = crate::HtmlParser::SCRIPTING_ENABLED;
    let expected = parser.parse(url, html.to_owned());
    let actual = live.snapshot_parser_stream_document();

    assert_eq!(actual.parse_errors(), expected.parse_errors());
    assert_eq!(actual.final_url(), expected.final_url());
    assert_eq!(actual.document_order_script_handles().len(), 2);
    assert_eq!(
        actual.document_order_script_handles().len(),
        expected.document_order_script_handles().len()
    );
    assert_eq!(
        actual.text_content(actual.document_body_handle().expect("body")),
        expected.text_content(expected.document_body_handle().expect("body"))
    );

    let ready = live.drain_ready_parser_scripts();
    assert_eq!(
        ready.len(),
        2,
        "both closed script elements should be recorded as ready"
    );

    let async_candidates = live.drain_discovered_async_prefetch_candidates();
    assert_eq!(
        async_candidates.len(),
        1,
        "external async classic should be discovered early"
    );

    let template = actual
        .elements_by_tag_name(actual.document_node_id(), "template", false)
        .into_iter()
        .next()
        .expect("template element");
    let template_contents = actual
        .node(template)
        .and_then(Node::as_element)
        .and_then(|element| element.template_contents());
    assert!(
        template_contents.is_some(),
        "template contents should be preserved"
    );
}

#[test]
fn parser_stream_marks_only_eof_unclosed_form_controls_as_submission_blocking() {
    let cases = [
        (
            "select",
            "<!doctype html><form><select><option>secret<element attribute></element>",
            true,
        ),
        (
            "select",
            "<!doctype html><form><select><option>safe</option></select></form>",
            false,
        ),
        (
            "textarea",
            "<!doctype html><form><textarea>secret<element attribute></element>",
            true,
        ),
        (
            "textarea",
            "<!doctype html><form><textarea>safe</textarea></form>",
            false,
        ),
    ];

    for (tag_name, html, expected) in cases {
        let url = Url::parse("https://dangling-markup.test/").expect("test url");
        let mut stream =
            crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
        stream.feed(html);
        let document = stream.finish();
        let control = document
            .elements_by_tag_name(document.document_node_id(), tag_name, false)
            .into_iter()
            .next()
            .expect("form control should be parsed");

        assert_eq!(
            document
                .node(control)
                .and_then(Node::as_element)
                .is_some_and(|element| element.blocks_form_submission()),
            expected,
            "{tag_name} EOF state for {html:?}"
        );
    }
}

#[test]
fn parser_stream_async_prefetch_uses_shared_script_type_classification() {
    let html = concat!(
        "<!doctype html><html><head>",
        "<script async src=\"/classic.js\"></script>",
        "<script async type=\"   \" src=\"/data-block.js\"></script>",
        "<script async type=\" text/javascript \" src=\"/typed-classic.js\"></script>",
        "</head></html>",
    );
    let url = Url::parse("https://example.test/").expect("test url");

    let mut live = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    for chunk in html_chunks(html) {
        live.feed(chunk);
    }

    let actual = live.snapshot_parser_stream_document();
    let candidate_srcs: Vec<_> = live
        .drain_discovered_async_prefetch_candidates()
        .into_iter()
        .map(|node_id| {
            actual
                .node(node_id)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("src"))
                .expect("async prefetch candidate should be a script with src")
                .to_owned()
        })
        .collect();

    assert_eq!(
        candidate_srcs,
        vec!["/classic.js".to_owned(), "/typed-classic.js".to_owned()]
    );
}

#[test]
fn parser_stream_discovers_modulepreload_link_candidates() {
    let html = concat!(
        "<!doctype html><html><head>",
        "<link rel='modulepreload' href='/entry.mjs'>",
        "<link rel='dns-prefetch MODULEPRELOAD' href='/caps.mjs'>",
        "<link rel='modulepreload' href='   '>",
        "<link rel='preload' href='/classic.js'>",
        "</head></html>",
    );
    let url = Url::parse("https://example.test/").expect("test url");

    let mut live = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    let mut modulepreload_candidates = Vec::new();
    for chunk in html_chunks(html) {
        let outcome = live.pump_parser_step(chunk);
        assert!(
            outcome.discovered_async_prefetch_scripts.is_empty(),
            "modulepreload links should not be reported as async script prefetches"
        );
        modulepreload_candidates.extend(outcome.discovered_modulepreload_link_candidates);
    }

    let actual = live.snapshot_parser_stream_document();
    let candidate_hrefs: Vec<_> = modulepreload_candidates
        .into_iter()
        .map(|node_id| {
            actual
                .node(node_id)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("href"))
                .expect("modulepreload candidate should be a link with href")
                .to_owned()
        })
        .collect();

    assert_eq!(
        candidate_hrefs,
        vec!["/entry.mjs".to_owned(), "/caps.mjs".to_owned()]
    );
}

#[test]
fn parser_stream_html_tree_sink_target_matches_parser_for_parser_created_flag() {
    let html = "<!doctype html><html><body><script src=\"/app.js\"></script><style>@import url('/a.css');</style></body></html>";
    let url = Url::parse("https://example.test/").expect("test url");
    let mut live =
        crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url.clone());
    for chunk in html_chunks(html) {
        live.feed(chunk);
    }
    let parser = crate::HtmlParser::SCRIPTING_ENABLED;
    let expected = parser.parse(url, html.to_owned());
    let actual = live.snapshot_parser_stream_document();

    let actual_flags = actual.script_handles().into_iter().all(|handle| {
        actual
            .node(handle)
            .is_some_and(|node| node.flags().parser_created())
    });
    let expected_flags = expected.script_handles().into_iter().all(|handle| {
        expected
            .node(handle)
            .is_some_and(|node| node.flags().parser_created())
    });

    assert_eq!(actual_flags, expected_flags);
}

#[test]
fn parser_stream_runtime_dom_sinks_pump_preserves_runtime_mutation_effects() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut stream = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    let mut dom_host = stream.take_parser_stream_dom_host();
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();

    {
        let mut collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        let _ = stream.pump_parser_step_with_runtime_dom_consumer_without_element_creation(
            "<!doctype html><html><body><div id='a'></div><script src='/app.js'></script></body></html>",
            &mut collector,
        );
    }
    stream.restore_parser_stream_dom_host(dom_host);

    assert!(
        effects.did_change(),
        "external runtime DOM sink pump sink should retain runtime-visible mutation effects"
    );
    assert!(
        !effects.tree().connected_roots().is_empty(),
        "connected subtree signals must survive for the runtime mutation owner"
    );
    assert!(
        !effects.style().child_list_mutations().is_empty(),
        "style child-list signals must survive for parser/runtime owner alignment"
    );
    assert!(
        !effects.scripts().prepare_triggers().is_empty(),
        "script prepare triggers should be retained and then gated by renderer policy"
    );
}

#[test]
fn parser_stream_runtime_dom_consumer_is_cleared_when_pump_unwinds() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut stream = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    let mut dom_host = stream.take_parser_stream_dom_host();
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();
    let mut collector = TestMutationEffectCollector {
        host: ptr,
        effects: &mut effects,
        panic_on_mutation: true,
    };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = stream.pump_parser_step_with_runtime_dom_consumer_without_element_creation(
            "<!doctype html><html><body><p>panic</p></body></html>",
            &mut collector,
        );
    }));
    assert!(
        result.is_err(),
        "test consumer should abort the parser pump"
    );

    stream.restore_parser_stream_dom_host(dom_host);
    let _ = stream.snapshot_parser_stream_document();
}

#[test]
#[should_panic(expected = "cannot snapshot parser stream while a runtime-DOM sink step is active")]
fn parser_stream_runtime_dom_sinks_rejects_snapshot_readback() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut dom_host = DomHost::from_dom(NativeDom::new_html(url.clone()));
    let document = dom_host.document_handle();
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();

    let mut collector = TestMutationEffectCollector {
        host: ptr,
        effects: &mut effects,
        panic_on_mutation: false,
    };
    let mut target = ParserStreamHtmlTreeSinkTarget::new_live_document_root(url, document);
    target.with_runtime_dom_consumer_for_test(&mut collector, |target| {
        let _ = target.snapshot_parser_stream_document();
    });
}

#[test]
fn parser_stream_runtime_dom_sinks_routes_parser_state_writes_through_sink() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut dom_host = DomHost::from_dom(NativeDom::new_html(url.clone()));
    let document = dom_host.document_handle();
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();
    let script;

    {
        let mut collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        let mut target = ParserStreamHtmlTreeSinkTarget::new_live_document_root(url, document);
        script = target.with_runtime_dom_consumer_for_test(&mut collector, |target| {
            target.push_parse_error("state write through sink".to_owned());
            target.set_html_quirks_mode(QuirksMode::Quirks);
            let script = target.create_parser_element_without_attributes(
                "script".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
            target.mark_script_already_started(script);
            script
        });
    }

    assert_eq!(
        dom_host.dom().parse_errors(),
        &["state write through sink".to_owned()]
    );
    assert_eq!(
        format!(
            "{:?}",
            dom_host.dom().document().expect("document").quirks_mode()
        ),
        "Quirks"
    );
    assert!(
        dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| element.script_already_started()),
        "script already-started flag should be written through the live parser sink"
    );
}

#[test]
fn parser_stream_runtime_dom_sinks_routes_declarative_shadow_attach_through_sink() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut stream = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    let mut dom_host = stream.take_parser_stream_dom_host();
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();

    {
        let mut collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        let _ = stream.pump_parser_step_with_runtime_dom_consumer_without_element_creation(
            concat!(
                "<!doctype html><html><body>",
                "<div id='host'>",
                "<template shadowrootmode='open'><span>shadow text</span></template>",
                "<p id='light'>light text</p>",
                "</div></body></html>"
            ),
            &mut collector,
        );
    }

    let host = dom_host
        .element_handle_by_id("host")
        .expect("declarative shadow host");
    let shadow_root = dom_host
        .shadow_root_handle(host)
        .expect("shadow root should be attached through parser sink");
    assert_eq!(
        dom_host.text_content(shadow_root).as_deref(),
        Some("shadow text")
    );
    assert!(
        dom_host
            .elements_by_tag_name(host, "template", false)
            .is_empty(),
        "parser attach should remove the declarative template from light DOM"
    );
    assert!(
        dom_host.element_handle_by_id("light").is_some(),
        "light DOM siblings after the declarative template should remain"
    );
}

#[test]
fn parser_stream_runtime_dom_sinks_routes_tree_adjacency_reads_through_sink() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut dom_host = DomHost::from_dom(NativeDom::new_html(url.clone()));
    let document = dom_host.document_handle();
    let body = dom_host.create_parser_element_without_attributes(
        "body".to_owned(),
        "http://www.w3.org/1999/xhtml".to_owned(),
        None,
    );
    let _ = dom_host.append_child(document, body);
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();
    let read_calls = std::cell::Cell::new(0);

    {
        let mut collector = TestReadTrackingCollector {
            host: ptr,
            effects: &mut effects,
            read_calls: &read_calls,
            read_events: None,
        };
        let mut target = ParserStreamHtmlTreeSinkTarget::new_live_document_root(url, document);
        target.with_runtime_dom_consumer_for_test(&mut collector, |target| {
            target
                .append(body, NodeOrText::AppendText("hello".into()))
                .consume();
            target
                .append(body, NodeOrText::AppendText(" world".into()))
                .consume();
        });
    }

    assert!(
        read_calls.get() > 0,
        "live parser tree-adjacency reads should route through ParserDomReadSink"
    );
    assert_eq!(
        dom_host.text_content(body).as_deref(),
        Some("hello world"),
        "text merge should still preserve parser-visible DOM output"
    );
}

#[test]
fn parser_stream_runtime_dom_sinks_routes_script_planning_reads_through_sink() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut stream = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    let mut dom_host = stream.take_parser_stream_dom_host();
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();
    let read_calls = std::cell::Cell::new(0);
    let read_events = std::cell::RefCell::new(Vec::new());
    let outcome;

    {
        let mut collector = TestReadTrackingCollector {
            host: ptr,
            effects: &mut effects,
            read_calls: &read_calls,
            read_events: Some(&read_events),
        };
        outcome = stream.pump_parser_step_with_runtime_dom_consumer_without_element_creation(
            concat!(
                "<!doctype html><html><head>",
                "<template id='t'><span>inside</span></template>",
                "<script async src='/async.js'></script>",
                "</head><body></body></html>"
            ),
            &mut collector,
        );
    }
    stream.restore_parser_stream_dom_host(dom_host);

    let events = read_events.borrow();
    for expected in [
        "template-contents",
        "html-element-name",
        "async-classic-candidate",
        "parser-script-read",
    ] {
        assert!(
            events.contains(&expected),
            "live parser planning read should route {expected} through ParserDomReadSink; got {events:?}"
        );
    }
    assert!(
        !events.contains(&"document-order-scripts"),
        "parser-boundary preparation must use its stable parser position instead of rescanning document-order scripts; got {events:?}"
    );
    assert!(
        !outcome.discovered_async_prefetch_scripts.is_empty(),
        "async script should still be prepared while planning reads route through the sink"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == "parser-script-read")
            .count(),
        1,
        "one parser boundary must classify and prepare an async script exactly once, even when the same payload is emitted for prefetch and handoff; got {events:?}"
    );
    assert!(
        read_calls.get() >= events.len(),
        "specific read events should also count as general read sink calls"
    );
}

#[test]
fn parser_stream_runtime_dom_sinks_routes_stylesheet_blocking_reads_through_sink() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut stream = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    let mut dom_host = stream.take_parser_stream_dom_host();
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();
    let read_calls = std::cell::Cell::new(0);
    let read_events = std::cell::RefCell::new(Vec::new());

    {
        let mut collector = TestReadTrackingCollector {
            host: ptr,
            effects: &mut effects,
            read_calls: &read_calls,
            read_events: Some(&read_events),
        };
        let _ = stream.pump_parser_step_with_runtime_dom_consumer_without_element_creation(
            concat!(
                "<!doctype html><html><head>",
                "<style>@import url('/blocking.css');</style>",
                "<script src='/app.js'></script>",
                "</head><body></body></html>"
            ),
            &mut collector,
        );
    }
    stream.restore_parser_stream_dom_host(dom_host);

    let events = read_events.borrow();
    for expected in ["stylesheet-element", "text-content"] {
        assert!(
            events.contains(&expected),
            "live parser stylesheet-blocking read should route {expected} through ParserDomReadSink; got {events:?}"
        );
    }
    assert!(
        !events.contains(&"document-order-stylesheet-candidates"),
        "token-time stylesheet capture must read the exact owner instead of rescanning the candidate registry; got {events:?}"
    );
}

#[test]
fn parser_stream_live_document_root_writes_detached_document() {
    let main_url = Url::parse("https://parent.example.test/").expect("test url");
    let child_url = Url::parse("https://parent.example.test/child.html").expect("test url");
    let mut dom_host = DomHost::from_dom(NativeDom::new_html(main_url));
    let child_document = dom_host.create_detached_html_document_with_url(child_url.clone());
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();
    let mut stream = crate::DocumentStream::new_scripting_enabled_live_document_root_for_testing(
        child_url,
        child_document,
    );

    {
        let mut collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        let _ = stream.pump_parser_step_with_runtime_dom_consumer_without_element_creation(
            "<!doctype html><html><body><p id='child'>child text</p></body></html>",
            &mut collector,
        );
    }

    assert!(
        effects.did_change(),
        "live document root parse should still emit runtime mutation effects"
    );
    assert!(
        dom_host
            .elements_by_tag_name(dom_host.document_handle(), "p", false)
            .is_empty(),
        "live child document root parse must not write into the main document root"
    );
    let child_paragraphs = dom_host.elements_by_tag_name(child_document, "p", false);
    assert_eq!(
        child_paragraphs.len(),
        1,
        "parser should write elements under the live child document root"
    );
    assert_eq!(
        dom_host.text_content(child_paragraphs[0]).as_deref(),
        Some("child text")
    );
}

#[test]
fn parser_stream_live_fragment_root_writes_fragment() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut dom_host = DomHost::from_dom(NativeDom::new_html(url.clone()));
    let document = dom_host.document_handle();
    let fragment = dom_host.create_document_fragment();
    let context = dom_host.create_parser_element_without_attributes(
        "div".to_owned(),
        "http://www.w3.org/1999/xhtml".to_owned(),
        None,
    );
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();

    {
        let mut collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        let mut stream = crate::DocumentStream::new_live_fragment_root_for_testing(
            url,
            fragment,
            document,
            context,
            "http://www.w3.org/1999/xhtml",
            "div",
            &mut collector,
            false,
            true,
        );
        let _ = stream.pump_parser_step_with_runtime_dom_consumer_without_element_creation(
            "<span id='frag'>fragment text</span>",
            &mut collector,
        );
    }

    assert!(
        effects.did_change(),
        "live fragment root parse should emit runtime mutation effects"
    );
    let fragment_children = dom_host.child_handles(fragment).collect::<Vec<_>>();
    assert_eq!(
        fragment_children.len(),
        1,
        "fragment parser should append its synthetic html root under the live fragment root"
    );
    assert!(
        dom_host
            .node(fragment_children[0])
            .is_some_and(|node| node.is_html_element_named("html"))
    );
    let root_children = dom_host
        .child_handles(fragment_children[0])
        .collect::<Vec<_>>();
    assert_eq!(
        root_children.len(),
        1,
        "fragment parser should append parsed roots under the synthetic html root"
    );
    assert!(
        dom_host
            .node(root_children[0])
            .is_some_and(|node| node.is_html_element_named("span"))
    );
    assert_eq!(
        dom_host.text_content(root_children[0]).as_deref(),
        Some("fragment text")
    );
    assert_eq!(
        dom_host.owner_document_handle(root_children[0]),
        Some(document),
        "live fragment nodes must be owned by the target Document, not by the fragment root"
    );
    assert!(
        dom_host
            .elements_by_tag_name(dom_host.document_handle(), "span", false)
            .is_empty(),
        "live fragment root parse must not append roots under the document"
    );
}

#[test]
fn live_fragment_parser_preserves_noscript_when_scripting_is_disabled() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut dom_host = DomHost::from_dom(NativeDom::new_html_with_scripting(url.clone(), false));
    let document = dom_host.document_handle();
    let html = dom_host.create_element("html");
    let body = dom_host.create_element("body");
    assert!(dom_host.append_child(document, html));
    assert!(dom_host.append_child(html, body));
    let existing_noscript = dom_host.create_element("noscript");
    let existing_text = dom_host.create_text_node("<em>existing&</em>");
    assert!(dom_host.append_child(existing_noscript, existing_text));
    assert!(dom_host.append_child(body, existing_noscript));
    let fragment = dom_host.create_document_fragment_for_document(document);
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();

    {
        let mut collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        crate::HtmlParser::with_scripting_enabled(false).parse_fragment_into_live_dom(
            url,
            fragment,
            document,
            body,
            "http://www.w3.org/1999/xhtml",
            "body",
            "<noscript>&lt;em&gt;fallback&amp;&lt;/em&gt;</noscript><span>tail</span>",
            &mut collector,
            false,
        );
    }

    assert_eq!(
        dom_host
            .elements_by_tag_name(fragment, "noscript", false)
            .len(),
        1,
        "a scripting-disabled live fragment must preserve its noscript root"
    );
    assert_eq!(
        dom_host.elements_by_tag_name(fragment, "span", false).len(),
        1
    );
}

#[test]
fn live_fragment_parser_uses_template_contents_owner_document() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut dom_host = DomHost::from_dom(NativeDom::new_html(url.clone()));
    let document = dom_host.document_handle();
    let fragment = dom_host.create_document_fragment_for_document(document);
    let context = dom_host.create_parser_element_without_attributes_for_document(
        document,
        "div".to_owned(),
        "http://www.w3.org/1999/xhtml".to_owned(),
        None,
    );
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();

    let signals = {
        let mut collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        crate::HtmlParser::with_scripting_enabled(true).parse_fragment_into_live_dom(
            url,
            fragment,
            document,
            context,
            "http://www.w3.org/1999/xhtml",
            "div",
            "<template><span>inside</span></template>",
            &mut collector,
            false,
        )
    };

    assert!(signals.parser_created_null_registry_elements.is_empty());
    let template = dom_host
        .elements_by_tag_name(fragment, "template", false)
        .into_iter()
        .next()
        .expect("parsed template");
    let contents = dom_host
        .parser_template_contents_handle(template)
        .expect("template contents");
    let contents_document = dom_host
        .owner_document_handle(contents)
        .expect("template contents owner Document");
    let span = dom_host
        .child_handles(contents)
        .next()
        .expect("template contents child");

    assert_eq!(dom_host.owner_document_handle(template), Some(document));
    assert_ne!(contents_document, document);
    assert_eq!(
        dom_host.owner_document_handle(span),
        Some(contents_document)
    );
}

#[test]
fn parser_stream_element_creation_sink_can_own_token_attributes() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut stream = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    let mut dom_host = stream.take_parser_stream_dom_host();
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();
    let mut element_creation = TestElementCreationCollector {
        dom_host: ptr,
        created_handle: None,
        saw_no_token_attributes_before_append: false,
        request_ids_and_intended_parents: Vec::new(),
    };

    {
        let mut mutation_collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        let _ = stream.pump_parser_step_with_runtime_dom_consumers(
            "<!doctype html><html><body><x-sync id='owned'></x-sync><p id='default'></p></body></html>",
            &mut mutation_collector,
            &mut element_creation,
        );
    }

    let created = element_creation
        .created_handle
        .expect("element sink should create x-sync");
    assert!(
        element_creation.saw_no_token_attributes_before_append,
        "creation sink should observe the constructor-before-token-attributes slot"
    );
    assert_eq!(
        dom_host.get_attribute(created, "id").as_deref(),
        Some("owned")
    );
    assert!(
        dom_host
            .node(created)
            .is_some_and(|node| node.flags().parser_created())
    );
    stream.restore_parser_stream_dom_host(dom_host);
}

#[test]
fn parser_stream_element_creation_request_reports_intended_parent() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut stream = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    let mut dom_host = stream.take_parser_stream_dom_host();
    let document = dom_host.document_handle();
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();
    let mut element_creation = TestElementCreationCollector {
        dom_host: ptr,
        created_handle: None,
        saw_no_token_attributes_before_append: false,
        request_ids_and_intended_parents: Vec::new(),
    };

    {
        let mut mutation_collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        let _ = stream.pump_parser_step_with_runtime_dom_consumers(
            "<!doctype html><html><body><svg><g/><x-sync id='inside-svg'></x-sync></svg><x-sync id='after-svg'></x-sync><x-sync id='outer'><x-sync id='inner'></x-sync></x-sync></body></html>",
            &mut mutation_collector,
            &mut element_creation,
        );
    }

    let inside_svg_parent = element_creation
        .request_ids_and_intended_parents
        .iter()
        .find_map(|(id, parent)| (id == "inside-svg").then_some(*parent))
        .expect("inside-svg request should be recorded");
    let after_svg_parent = element_creation
        .request_ids_and_intended_parents
        .iter()
        .find_map(|(id, parent)| (id == "after-svg").then_some(*parent))
        .expect("after-svg request should be recorded");
    let outer_parent = element_creation
        .request_ids_and_intended_parents
        .iter()
        .find_map(|(id, parent)| (id == "outer").then_some(*parent))
        .expect("outer x-sync request should be recorded");
    let inner_parent = element_creation
        .request_ids_and_intended_parents
        .iter()
        .find_map(|(id, parent)| (id == "inner").then_some(*parent))
        .expect("inner x-sync request should be recorded");
    let outer = dom_host
        .element_handle_by_id("outer")
        .expect("outer x-sync should be created");
    let inside_svg = dom_host
        .element_handle_by_id("inside-svg")
        .expect("inside-svg should be created");
    let body = dom_host
        .document_body_handle()
        .expect("parsed document should have a body");

    assert_eq!(
        inside_svg_parent,
        dom_host.parent_node(inside_svg),
        "self-closing foreign elements must not remain as the mirrored parser parent"
    );
    assert_eq!(
        after_svg_parent,
        Some(body),
        "closing an SVG subtree must restore the mirrored HTML parser parent"
    );

    assert_ne!(
        outer_parent,
        Some(document),
        "outer custom element should be created under the current body/html context, not a bare document fallback"
    );
    assert_eq!(
        inner_parent,
        Some(outer),
        "nested custom element lookup should use the open parser parent"
    );
    stream.restore_parser_stream_dom_host(dom_host);
}

#[test]
fn parser_stream_element_creation_sink_uses_live_document_root_handle() {
    let main_url = Url::parse("https://parent.example.test/").expect("test url");
    let child_url = Url::parse("https://parent.example.test/child.html").expect("test url");
    let mut dom_host = DomHost::from_dom(NativeDom::new_html(main_url));
    let child_document = dom_host.create_detached_html_document_with_url(child_url.clone());
    let ptr = &mut dom_host as *mut DomHost;
    let mut effects = DomMutationEffects::default();
    let mut stream = crate::DocumentStream::new_scripting_enabled_live_document_root_for_testing(
        child_url,
        child_document,
    );
    let mut element_creation = TestElementCreationCollector {
        dom_host: ptr,
        created_handle: None,
        saw_no_token_attributes_before_append: false,
        request_ids_and_intended_parents: Vec::new(),
    };

    {
        let mut mutation_collector = TestMutationEffectCollector {
            host: ptr,
            effects: &mut effects,
            panic_on_mutation: false,
        };
        let _ = stream.pump_parser_step_with_runtime_dom_consumers(
            "<!doctype html><html><body><x-sync id='child-owned'></x-sync></body></html>",
            &mut mutation_collector,
            &mut element_creation,
        );
    }

    let created = element_creation
        .created_handle
        .expect("element sink should create x-sync in the child document");
    assert_eq!(
        dom_host.owner_document_handle(created),
        Some(child_document),
        "parser element creation request should carry the live document root, not the main document"
    );
    assert_eq!(
        dom_host.get_attribute(created, "id").as_deref(),
        Some("child-owned")
    );
}

#[test]
fn parser_stream_owned_bootstrap_builds_dom_without_mutation_owner() {
    let url = Url::parse("https://example.test/").expect("test url");
    let mut stream = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);

    stream.feed(
        "<!doctype html><html><body><div id='a'></div><script src='/app.js'></script></body></html>",
    );
    let document = stream.snapshot_parser_stream_document();

    assert!(
        document
            .elements_by_tag_name(document.document_node_id(), "div", false)
            .into_iter()
            .any(|handle| document
                .node(handle)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("id"))
                == Some("a")),
        "bootstrap-owned parser mutations should build the owned DOM synchronously"
    );
}

#[test]
fn parser_stream_does_not_prefetch_whitespace_type_async_script() {
    let html = "<!doctype html><html><head><script async src=\"/data.js\" type=\"   \"></script></head><body></body></html>";
    let url = Url::parse("https://example.test/").expect("test url");
    let mut live = crate::DocumentStream::new_scripting_enabled_parser_stream_for_testing(url);
    for chunk in html_chunks(html) {
        live.feed(chunk);
    }

    let async_candidates = live.drain_discovered_async_prefetch_candidates();
    assert!(
        async_candidates.is_empty(),
        "whitespace-only script type is a data block, not an async classic candidate"
    );
}
