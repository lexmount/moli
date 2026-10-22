use anyhow::Result;

use super::ScriptVm;
use crate::document_runtime::DomHandle;
use crate::dom::native::{Attribute, NativeDom, Node, NodeType};
use crate::native_bridge::element::{
    focus_live_element_for_inspector, mutate_live_element_attribute_for_inspector,
};
use crate::native_bridge::{
    JsContextHost, node_runtime_and_handle_from_object, validate_element_name,
};
use crate::parser::{HtmlParser, XmlParser};
use crate::runtime::{
    RendererDomAttributeMutation, RendererDomAttributeMutationOutcome, RendererDomFocusOutcome,
};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum DomInspectorEdit {
    MoveTo {
        node: DomHandle,
        target: DomHandle,
        insert_before: Option<DomHandle>,
    },
    SetAttributesAsText {
        node: DomHandle,
        text: String,
        name: Option<String>,
    },
    SetNodeName {
        node: DomHandle,
        name: String,
    },
    SetNodeValue {
        node: DomHandle,
        value: String,
    },
    SetOuterHtml {
        node: DomHandle,
        outer_html: String,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum DomInspectorEditOutcome {
    Applied { result_node: Option<DomHandle> },
    NodeNotFound,
    NodeNotElement,
    NodeValueUnsupported,
    MoveIntoSelfOrDescendant,
    AnchorNotChildOfTarget,
    DetachedNode,
    InvalidName { name: String },
    CouldNotParseAttributes,
    MutationFailed,
}

fn first_element_in_subtree(document: &NativeDom, root: DomHandle) -> Option<DomHandle> {
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        if document.node(handle).is_some_and(Node::is_element) {
            return Some(handle);
        }
        stack.extend(document.child_ids_reversed(handle));
    }
    None
}

fn parsed_inspector_attributes(
    runtime: &JsContextHost,
    handle: DomHandle,
    text: &str,
) -> Option<Vec<Attribute>> {
    let dom_host = runtime.dom_host();
    let element = dom_host.node(handle)?.as_element()?;
    let document_handle = dom_host.owner_document_handle(handle)?;
    let document_url = dom_host
        .document_url_for_handle(document_handle)
        .cloned()
        .unwrap_or_else(|| url::Url::parse("about:blank").expect("valid fallback URL"));
    let wrapper_name = if element.namespace() == "http://www.w3.org/2000/svg" {
        "svg"
    } else if element.namespace() == "http://www.w3.org/1998/Math/MathML" {
        "math"
    } else {
        "span"
    };
    let markup = format!("<{wrapper_name} {text}></{wrapper_name}>");
    let is_html_document = dom_host
        .node_document_is_html_document(handle)
        .unwrap_or(false);

    let parsed = if is_html_document {
        HtmlParser::with_scripting_enabled(runtime.document_scripting_enabled(document_handle))
            .parse_fragment_without_declarative_shadow_roots(
                document_url,
                "http://www.w3.org/1999/xhtml",
                "body",
                markup,
            )
    } else {
        let parsed = XmlParser.parse(document_url, markup);
        if !parsed.parse_errors().is_empty() {
            return None;
        }
        parsed
    };
    let parsed_element = if is_html_document {
        let source_root = parsed.body_node_id().unwrap_or_else(|| {
            let document_root = parsed.document_node_id();
            let document_children = parsed.child_ids(document_root).collect::<Vec<_>>();
            if document_children.len() == 1
                && parsed
                    .node(document_children[0])
                    .is_some_and(|node| node.is_html_element_named("html"))
            {
                document_children[0]
            } else {
                document_root
            }
        });
        parsed
            .child_ids(source_root)
            .find_map(|child| first_element_in_subtree(&parsed, child))?
    } else {
        first_element_in_subtree(&parsed, parsed.document_node_id())?
    };
    parsed
        .node(parsed_element)?
        .as_element()
        .map(|element| element.attributes().to_vec())
}

fn clone_element_attributes(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    target: DomHandle,
    attributes: &[Attribute],
) -> bool {
    for attribute in attributes {
        let qualified_name = attribute.name();
        let namespace = (!attribute.namespace().is_empty()).then_some(attribute.namespace());
        // The mutation API reports whether state changed, so a same-value write
        // is false even when the requested attribute is already present.
        let _did_change = unsafe { &mut *host_ptr }
            .set_attribute_ns_appending_to_current_reaction_queue(
                scope,
                host_ptr,
                target,
                namespace,
                attribute.prefix(),
                attribute.local_name(),
                &qualified_name,
                attribute.value(),
            );
        let attribute_was_cloned = unsafe { &*host_ptr }
            .dom_host()
            .node(target)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.attributes().iter().any(|candidate| {
                    candidate.namespace() == attribute.namespace()
                        && candidate.prefix() == attribute.prefix()
                        && candidate.local_name() == attribute.local_name()
                        && candidate.value() == attribute.value()
                })
            });
        if !attribute_was_cloned {
            return false;
        }
    }
    true
}

fn create_renamed_element(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    owner_document: DomHandle,
    name: &str,
    is_html_document: bool,
) -> Option<DomHandle> {
    if is_html_document {
        let normalized_name = name.to_ascii_lowercase();
        let wrapper =
            crate::custom_elements::create_element_for_document_local_name_is_and_registry(
                scope,
                host_ptr,
                owner_document,
                &normalized_name,
                None,
                None,
                None,
            )?;
        return node_runtime_and_handle_from_object(scope, wrapper)
            .ok()
            .map(|(_, handle)| handle);
    }

    let runtime = unsafe { &mut *host_ptr };
    let namespace = runtime
        .dom_host()
        .document_content_type_for_handle(owner_document)
        .is_some_and(|content_type| content_type.eq_ignore_ascii_case("application/xhtml+xml"))
        .then_some("http://www.w3.org/1999/xhtml");
    let handle = runtime.create_element_ns(namespace, name)?;
    runtime.initialize_new_native_node_owner_document(owner_document, handle)
}

fn move_node_for_inspector(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    node: DomHandle,
    target: DomHandle,
    insert_before: Option<DomHandle>,
) -> DomInspectorEditOutcome {
    let runtime = unsafe { &*host_ptr };
    if runtime.dom_host().node(node).is_none() {
        return DomInspectorEditOutcome::NodeNotFound;
    }
    if !runtime
        .dom_host()
        .node(target)
        .is_some_and(Node::is_element)
    {
        return if runtime.dom_host().node(target).is_some() {
            DomInspectorEditOutcome::NodeNotElement
        } else {
            DomInspectorEditOutcome::NodeNotFound
        };
    }
    if runtime.dom_host().dom().contains(node, target) {
        return DomInspectorEditOutcome::MoveIntoSelfOrDescendant;
    }
    if let Some(anchor) = insert_before
        && runtime.dom_host().parent_node(anchor) != Some(target)
    {
        return if runtime.dom_host().node(anchor).is_some() {
            DomInspectorEditOutcome::AnchorNotChildOfTarget
        } else {
            DomInspectorEditOutcome::NodeNotFound
        };
    }

    crate::custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        let _ = unsafe { &mut *host_ptr }.insert_before_appending_to_current_reaction_queue(
            scope,
            host_ptr,
            target,
            node,
            insert_before,
        );
    });
    if unsafe { &*host_ptr }.dom_host().parent_node(node) != Some(target) {
        return DomInspectorEditOutcome::MutationFailed;
    }
    DomInspectorEditOutcome::Applied {
        result_node: Some(node),
    }
}

fn set_attributes_as_text_for_inspector(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    node: DomHandle,
    text: &str,
    name: Option<&str>,
) -> DomInspectorEditOutcome {
    let runtime = unsafe { &*host_ptr };
    let Some(element) = runtime.dom_host().node(node).and_then(Node::as_element) else {
        return if runtime.dom_host().node(node).is_some() {
            DomInspectorEditOutcome::NodeNotElement
        } else {
            DomInspectorEditOutcome::NodeNotFound
        };
    };
    let is_html_element_in_html_document = element.namespace() == "http://www.w3.org/1999/xhtml"
        && runtime
            .dom_host()
            .node_document_is_html_document(node)
            .unwrap_or(false);
    let adjusted_name = name.map(|name| {
        if is_html_element_in_html_document {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        }
    });
    let Some(attributes) = parsed_inspector_attributes(runtime, node, text) else {
        return DomInspectorEditOutcome::CouldNotParseAttributes;
    };

    let mut found_original_attribute = false;
    for attribute in attributes {
        let attribute_name = if is_html_element_in_html_document {
            attribute.name().to_ascii_lowercase()
        } else {
            attribute.name()
        };
        found_original_attribute |= adjusted_name.as_deref() == Some(attribute_name.as_str());
        let outcome = mutate_live_element_attribute_for_inspector(
            scope,
            host_ptr,
            node,
            RendererDomAttributeMutation::Set {
                name: attribute_name,
                value: attribute.value().to_owned(),
            },
        );
        match outcome {
            RendererDomAttributeMutationOutcome::Applied { .. } => {}
            RendererDomAttributeMutationOutcome::InvalidName { name } => {
                return DomInspectorEditOutcome::InvalidName { name };
            }
            RendererDomAttributeMutationOutcome::NodeNotFound => {
                return DomInspectorEditOutcome::NodeNotFound;
            }
            RendererDomAttributeMutationOutcome::NodeNotElement => {
                return DomInspectorEditOutcome::NodeNotElement;
            }
        }
    }

    if let Some(name) = adjusted_name
        && !found_original_attribute
        && !name.trim().is_empty()
    {
        let outcome = mutate_live_element_attribute_for_inspector(
            scope,
            host_ptr,
            node,
            RendererDomAttributeMutation::Remove { name },
        );
        if !matches!(outcome, RendererDomAttributeMutationOutcome::Applied { .. }) {
            return DomInspectorEditOutcome::MutationFailed;
        }
    }

    DomInspectorEditOutcome::Applied { result_node: None }
}

fn set_node_name_for_inspector(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    node: DomHandle,
    name: &str,
) -> DomInspectorEditOutcome {
    if !validate_element_name(name) {
        return DomInspectorEditOutcome::InvalidName {
            name: name.to_owned(),
        };
    }
    let runtime = unsafe { &*host_ptr };
    let Some(current) = runtime.dom_host().node(node) else {
        return DomInspectorEditOutcome::NodeNotFound;
    };
    let node_type = current.node_type();
    if !matches!(
        node_type,
        NodeType::Element | NodeType::ProcessingInstruction
    ) {
        return DomInspectorEditOutcome::NodeNotElement;
    }
    let Some(parent) = runtime.dom_host().parent_node(node) else {
        return DomInspectorEditOutcome::DetachedNode;
    };
    let Some(owner_document) = runtime.dom_host().owner_document_handle(node) else {
        return DomInspectorEditOutcome::MutationFailed;
    };
    let next_sibling = current.next_sibling();
    let children = runtime.dom_host().child_handles(node).collect::<Vec<_>>();
    let attributes = current
        .as_element()
        .map(|element| element.attributes().to_vec())
        .unwrap_or_default();
    let processing_instruction_data = current
        .as_processing_instruction()
        .map(|instruction| instruction.data().to_owned());
    let is_html_document = runtime
        .dom_host()
        .node_document_is_html_document(node)
        .unwrap_or(false);

    let mut result = None;
    crate::custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
        let new_node = match node_type {
            NodeType::Element => {
                create_renamed_element(scope, host_ptr, owner_document, name, is_html_document)
            }
            NodeType::ProcessingInstruction => Some(
                unsafe { &mut *host_ptr }.create_processing_instruction_for_document(
                    owner_document,
                    name,
                    processing_instruction_data.as_deref().unwrap_or_default(),
                ),
            ),
            _ => None,
        };
        let Some(new_node) = new_node else {
            return;
        };
        if node_type == NodeType::Element
            && !clone_element_attributes(scope, host_ptr, new_node, &attributes)
        {
            return;
        }
        for child in &children {
            let _ = unsafe { &mut *host_ptr }.insert_before_appending_to_current_reaction_queue(
                scope, host_ptr, new_node, *child, None,
            );
        }
        let inserted = unsafe { &mut *host_ptr }.insert_before_appending_to_current_reaction_queue(
            scope,
            host_ptr,
            parent,
            new_node,
            next_sibling,
        );
        let removed = unsafe { &mut *host_ptr }
            .remove_child_appending_to_current_reaction_queue(scope, host_ptr, parent, node);
        if inserted && removed {
            result = Some(new_node);
        }
    });

    result
        .map(|result_node| DomInspectorEditOutcome::Applied {
            result_node: Some(result_node),
        })
        .unwrap_or(DomInspectorEditOutcome::MutationFailed)
}

fn run_dom_inspector_edit(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    edit: DomInspectorEdit,
) -> DomInspectorEditOutcome {
    match edit {
        DomInspectorEdit::MoveTo {
            node,
            target,
            insert_before,
        } => move_node_for_inspector(scope, host_ptr, node, target, insert_before),
        DomInspectorEdit::SetAttributesAsText { node, text, name } => {
            set_attributes_as_text_for_inspector(scope, host_ptr, node, &text, name.as_deref())
        }
        DomInspectorEdit::SetNodeName { node, name } => {
            set_node_name_for_inspector(scope, host_ptr, node, &name)
        }
        DomInspectorEdit::SetNodeValue { node, value } => {
            let runtime = unsafe { &mut *host_ptr };
            let Some(node_type) = runtime.dom_host().node(node).map(Node::node_type) else {
                return DomInspectorEditOutcome::NodeNotFound;
            };
            if !matches!(node_type, NodeType::Text | NodeType::ProcessingInstruction) {
                return DomInspectorEditOutcome::NodeValueUnsupported;
            }
            // A same-value character-data write is a successful CDP edit even
            // though the mutation API correctly reports no state change.
            let _did_change = runtime.set_text_content(scope, host_ptr, node, &value);
            if runtime.dom_host().node(node).and_then(Node::node_value) == Some(value.as_str()) {
                DomInspectorEditOutcome::Applied { result_node: None }
            } else {
                DomInspectorEditOutcome::MutationFailed
            }
        }
        DomInspectorEdit::SetOuterHtml { node, outer_html } => {
            let outer_html = crate::dom::native::DomStringValue::from(outer_html);
            let runtime = unsafe { &*host_ptr };
            if runtime.dom_host().node(node).is_none() {
                return DomInspectorEditOutcome::NodeNotFound;
            }
            if runtime.dom_host().parent_node(node).is_none() {
                return DomInspectorEditOutcome::DetachedNode;
            }
            if runtime.dom_host().owner_document_handle(node).is_none() {
                return DomInspectorEditOutcome::MutationFailed;
            }
            let changed = crate::custom_elements::with_custom_element_reaction_scope(
                scope,
                host_ptr,
                |scope| {
                    unsafe { &mut *host_ptr }.set_outer_html(scope, host_ptr, node, &outer_html)
                },
            );
            if matches!(changed, Ok(true)) {
                DomInspectorEditOutcome::Applied { result_node: None }
            } else {
                DomInspectorEditOutcome::MutationFailed
            }
        }
    }
}

impl ScriptVm {
    /// Completes one DevTools live-DOM command that entered a Window realm.
    ///
    /// The native mutation/focus helpers are command bodies, not task
    /// boundaries. They may synchronously run custom-element or focus
    /// callbacks and may enqueue MutationObserver delivery. The enclosing
    /// browser command therefore owns one explicit Page-agent checkpoint after
    /// the body is settled, followed by child-record synchronization. This is
    /// deliberately narrower than generic runtime-work draining.
    pub(super) fn finish_devtools_live_dom_command<T>(&mut self, body: Result<T>) -> Result<T> {
        let checkpoint = self.perform_owner_lane_task_microtask_checkpoints();
        match body {
            Ok(value) => {
                checkpoint?;
                self.sync_child_browsing_context_records();
                Ok(value)
            }
            Err(error) => {
                if let Err(checkpoint_error) = checkpoint {
                    tracing::warn!(
                        %checkpoint_error,
                        "DOM-inspector command body and command-end checkpoint both failed"
                    );
                }
                Err(error)
            }
        }
    }

    pub(super) fn child_execution_context_id_for_live_dom_handle(
        &self,
        handle: DomHandle,
    ) -> Option<Option<i64>> {
        let runtime = self._context_host.borrow();
        runtime
            .dom_host()
            .owner_document_handle(handle)
            .and_then(|document| runtime.child_browsing_context_host_for_document_handle(document))
            .map(|child| runtime.child_default_execution_context_id(child))
    }

    pub(crate) fn edit_document_node(
        &mut self,
        edit: DomInspectorEdit,
    ) -> Result<DomInspectorEditOutcome> {
        let handle = match &edit {
            DomInspectorEdit::MoveTo { node, .. }
            | DomInspectorEdit::SetAttributesAsText { node, .. }
            | DomInspectorEdit::SetNodeName { node, .. }
            | DomInspectorEdit::SetNodeValue { node, .. }
            | DomInspectorEdit::SetOuterHtml { node, .. } => *node,
        };
        let body = match self.child_execution_context_id_for_live_dom_handle(handle) {
            Some(Some(execution_context_id)) => self.with_child_frame_realm_context_scope(
                execution_context_id,
                move |scope, runtime_ptr| Ok(run_dom_inspector_edit(scope, runtime_ptr, edit)),
            ),
            Some(None) => Ok(DomInspectorEditOutcome::NodeNotFound),
            None => self.with_default_context_scope(move |scope, runtime_ptr| {
                Ok(run_dom_inspector_edit(scope, runtime_ptr, edit))
            }),
        };
        self.finish_devtools_live_dom_command(body)
    }

    pub(crate) fn mutate_document_node_attribute(
        &mut self,
        handle: DomHandle,
        mutation: RendererDomAttributeMutation,
    ) -> Result<RendererDomAttributeMutationOutcome> {
        let body = self.with_default_context_scope(move |scope, runtime_ptr| {
            Ok(mutate_live_element_attribute_for_inspector(
                scope,
                runtime_ptr,
                handle,
                mutation,
            ))
        });
        self.finish_devtools_live_dom_command(body)
    }

    pub(crate) fn focus_document_node(
        &mut self,
        handle: DomHandle,
    ) -> Result<RendererDomFocusOutcome> {
        let child_execution_context_id =
            self.child_execution_context_id_for_live_dom_handle(handle);

        let body = match child_execution_context_id {
            Some(Some(execution_context_id)) => self.with_child_frame_realm_context_scope(
                execution_context_id,
                move |scope, runtime_ptr| {
                    Ok(focus_live_element_for_inspector(scope, runtime_ptr, handle))
                },
            ),
            Some(None) => Ok(RendererDomFocusOutcome::NodeNotFound),
            None => self.with_default_context_scope(move |scope, runtime_ptr| {
                Ok(focus_live_element_for_inspector(scope, runtime_ptr, handle))
            }),
        };
        self.finish_devtools_live_dom_command(body)
    }
}
