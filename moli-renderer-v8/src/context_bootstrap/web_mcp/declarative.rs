//! Native form registration following Chromium HTMLFormElement and FormMCPSchema.

use super::bindings::{check_target, document_owner, model_context_for_document};
use super::state::{FormInvocationState, RegisteredTool, ToolExecutor, ToolMetadata};
use super::tasks::queue_task;
use super::{devtools, events, execution};
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::util::context_host_ptr_from_global_bridge;
use moli_page_types::{RendererWebMcpEvent, RendererWebMcpToolId};

use crate::dom::native::{DomHost, DomMutationEffects, Node};

pub(super) mod invocation;
mod schema;

pub(crate) fn note_mutation(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    dom: &DomHost,
    effects: &DomMutationEffects,
) {
    let removed_roots = effects
        .style()
        .child_list_mutations()
        .iter()
        .flat_map(|mutation| mutation.removed_nodes().iter().copied())
        .collect::<Vec<_>>();
    let was_removed = |handle| {
        removed_roots
            .iter()
            .any(|root| dom.is_host_including_inclusive_ancestor(*root, handle))
    };
    let targets = effects
        .style()
        .attribute_mutations()
        .iter()
        .filter(|attribute| {
            attribute.namespace().is_none() && schema_attribute(attribute.local_name())
        })
        .map(|attribute| attribute.target())
        .chain(
            effects
                .style()
                .child_list_mutations()
                .iter()
                .flat_map(|mutation| {
                    mutation
                        .added_nodes()
                        .iter()
                        .chain(mutation.removed_nodes())
                        .copied()
                }),
        )
        .chain(effects.style().character_data_mutations().iter().copied())
        .map(|target| (target, true))
        .chain(
            effects
                .style()
                .child_list_mutations()
                .iter()
                .map(|mutation| (mutation.target(), false)),
        );
    for (target, discover_descendants) in targets {
        if let Some(document) = dom.owner_document_handle(target)
            && (host
                .native_bridge()
                .web_mcp
                .documents
                .get(&document)
                .is_some_and(|entry| {
                    entry
                        .tools
                        .values()
                        .any(|tool| matches!(tool.executor, ToolExecutor::Form { .. }))
                })
                || (dom.is_connected(target)
                    && ((discover_descendants && contains_tool_form(dom, target))
                        || affects_tool_form(dom, target))))
        {
            host.native_bridge_mut()
                .web_mcp
                .dirty_form_documents
                .insert(document);
        }
    }
    let invalid = host
        .native_bridge()
        .web_mcp
        .documents
        .iter()
        .flat_map(|(document, entry)| {
            entry.tools.iter().filter_map(|(name, tool)| {
                let ToolExecutor::Form { handle, .. } = tool.executor else {
                    return None;
                };
                (was_removed(handle)
                    || !form_registration_is_valid(dom, handle)
                    || dom.owner_document_handle(handle) != Some(*document))
                .then(|| (*document, name.clone(), handle))
            })
        })
        .collect::<Vec<_>>();
    let invalidated_forms = invalid
        .iter()
        .map(|(_, _, handle)| *handle)
        .collect::<std::collections::HashSet<_>>();
    for (document, name, _) in invalid {
        let (frame_id, tree, origin) = {
            let store = &mut host.native_bridge_mut().web_mcp;
            store.dirty_form_documents.insert(document);
            let entry = store.documents.get_mut(&document).expect("form document");
            entry.tools.remove(&name);
            (
                entry.frame_id.clone(),
                entry.frame_tree,
                entry.origin.clone(),
            )
        };
        devtools::emit_in_tree(
            host,
            tree,
            RendererWebMcpEvent::ToolsRemoved(vec![RendererWebMcpToolId { frame_id, name }]),
        );
        events::queue_form_removal_change(scope, host, document, &origin);
    }
    for pending in host.native_bridge_mut().web_mcp.pending.values_mut() {
        let Some(form) = &mut pending.form else {
            continue;
        };
        // A protected removal already retired that registration. Later unrelated
        // mutations must not invalidate its captured response a second time.
        if !invalidated_forms.contains(&form.handle) {
            continue;
        }
        let protected = match &form.state {
            FormInvocationState::Submitting => true,
            FormInvocationState::Responding(response) => {
                v8::Local::new(scope, response).state() == v8::PromiseState::Pending
            }
            _ => false,
        };
        if !protected {
            form.state = FormInvocationState::Invalidated;
        }
    }
}

// Native forms in connected shadow trees participate in the same registry.
// Use this traversal for initial discovery and every mutation synchronization.
fn form_handles(dom: &DomHost, root: DomHandle) -> Vec<DomHandle> {
    let mut forms = Vec::new();
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        if dom.is_html_element_named(handle, "form") {
            forms.push(handle);
        }
        stack.extend(
            dom.child_handles(handle)
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
        if let Some(shadow) = dom.shadow_root_handle(handle) {
            stack.push(shadow);
        }
    }
    forms
}

fn form_registration_is_valid(dom: &DomHost, handle: DomHandle) -> bool {
    dom.is_connected(handle)
        && dom
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.has_attribute("toolname") && element.has_attribute("tooldescription")
            })
}

fn contains_tool_form(dom: &DomHost, root: DomHandle) -> bool {
    dom.html_elements_by_local_name_in_shadow_including_subtree(root, "form")
        .into_iter()
        .any(|handle| {
            dom.node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| {
                    element.has_attribute("toolname") && element.has_attribute("tooldescription")
                })
        })
}

fn affects_tool_form(dom: &DomHost, target: DomHandle) -> bool {
    let Some(document) = dom.owner_document_handle(target) else {
        return false;
    };
    let forms = dom.html_elements_by_local_name_in_shadow_including_subtree(document, "form");
    if forms.is_empty() {
        return false;
    }
    let owner = dom.form_control_owner(target);
    // A name collision can keep a valid form out of the tool registry. Its
    // descendants and associated controls still need to update its definition.
    // Inspect indexed forms, never the growing mutation container's subtree.
    forms.into_iter().any(|form| {
        form_registration_is_valid(dom, form)
            && (form == target || dom.is_ancestor(form, target) || owner == Some(form))
    })
}

fn schema_attribute(name: &str) -> bool {
    matches!(
        name,
        "toolname"
            | "tooldescription"
            | "tooltitle"
            | "toolautosubmit"
            | "name"
            | "type"
            | "form"
            | "id"
            | "disabled"
            | "readonly"
            | "required"
            | "min"
            | "max"
            | "step"
            | "pattern"
            | "multiple"
            | "value"
            | "toolparamdescription"
            | "aria-description"
            | "for"
    )
}

// Keep discovery outside V8: pages without declarative tools must not acquire
// another realm entry while completing unrelated parser or resource work.
pub(crate) fn prepare_registration_task(host: &mut JsContextHost) -> bool {
    let document = host.document_handle();
    let owner = document_owner(host, document);
    let needs_initial_scan = host.native_bridge().web_mcp.initialized_form_owner != owner;
    let has_initial_forms = needs_initial_scan && contains_tool_form(host.dom_host(), document);
    let store = &mut host.native_bridge_mut().web_mcp;
    if needs_initial_scan {
        // The scheduler discards tasks owned by the previous document. Its
        // queued marker must not prevent the replacement document's scan.
        store.form_registration_task_queued = false;
        store.initialized_form_owner = owner;
        if has_initial_forms {
            store.dirty_form_documents.insert(document);
        }
    }
    !store.form_registration_task_queued && !store.dirty_form_documents.is_empty()
}

pub(crate) fn queue_registration_task(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    if !prepare_registration_task(unsafe { &mut *host_ptr }) {
        return;
    }
    unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .form_registration_task_queued = true;
    let callback =
        v8::Function::new(scope, registration_callback).expect("WebMCP form registration task");
    queue_task(scope, host_ptr, callback);
}

fn registration_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let mut documents = {
        let store = &mut unsafe { &mut *host_ptr }.native_bridge_mut().web_mcp;
        store.form_registration_task_queued = false;
        std::mem::take(&mut store.dirty_form_documents)
            .into_iter()
            .collect::<Vec<_>>()
    };
    documents.sort_unstable_by_key(|document| document.index());
    // An observed invalidation cannot be undone by the final DOM snapshot.
    let invalidated = unsafe { &*host_ptr }
        .native_bridge()
        .web_mcp
        .pending
        .iter()
        .filter(|(_, pending)| {
            pending
                .form
                .as_ref()
                .is_some_and(|form| matches!(form.state, FormInvocationState::Invalidated))
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for id in invalidated {
        execution::finish_error(
            scope,
            host_ptr,
            id,
            "Tool definition changed during execution",
        );
    }
    for document in documents {
        synchronize_document(scope, host_ptr, document);
    }
}

struct FormTool {
    handle: DomHandle,
    name: String,
    metadata: ToolMetadata,
    autosubmit: bool,
}

fn form_tools(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    document: DomHandle,
) -> Vec<FormTool> {
    if document_owner(host, document).is_none()
        || !host.document_scripting_enabled(document)
        || !host
            .document_permissions_policy_for_document_handle(document)
            .is_some_and(|policy| policy.tools_enabled())
    {
        return Vec::new();
    }
    let secure_url = host
        .child_browsing_context_host_for_document_handle(document)
        .and_then(|child| host.child_browsing_context_secure_context_url(child))
        .unwrap_or_else(|| host.document_url_for_handle(document));
    if !moli_url::is_potentially_trustworthy_url(&secure_url) {
        return Vec::new();
    }
    let dom = host.dom_host();
    let mut result = Vec::new();
    for handle in form_handles(dom, document) {
        let element = dom.node(handle).and_then(Node::as_element).expect("form");
        let (Some(name), Some(description)) = (
            element.attribute("toolname"),
            element.attribute("tooldescription"),
        ) else {
            continue;
        };
        if !moli_webmcp::is_valid_tool_name(name) {
            continue;
        }
        result.push(FormTool {
            handle,
            name: name.into(),
            metadata: ToolMetadata {
                description: description.into(),
                title: element.attribute("tooltitle").unwrap_or_default().into(),
                input_schema: Some(schema::input_schema(host, handle, |pattern| {
                    crate::native_bridge::element::v8_pattern_is_usable(scope, pattern).is_some()
                })),
                annotations: None,
            },
            autosubmit: element.has_attribute("toolautosubmit"),
        });
    }
    result
}

fn synchronize_document(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    document: DomHandle,
) {
    let definitions = form_tools(scope, unsafe { &*host_ptr }, document);
    if definitions.is_empty()
        && !unsafe { &*host_ptr }
            .native_bridge()
            .web_mcp
            .documents
            .contains_key(&document)
    {
        return;
    }
    if document_owner(unsafe { &*host_ptr }, document).is_none() {
        return;
    }
    let context = if document == unsafe { &*host_ptr }.document_handle() {
        Some(scope.get_current_context())
    } else {
        let host = unsafe { &mut *host_ptr };
        host.child_browsing_context_host_for_document_handle(document)
            .and_then(|child| host.child_browsing_context_window_wrapper(scope, child))
            .and_then(|window| window.get_creation_context(scope))
    };
    let Some(context) = context else { return };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(document_wrapper) =
        crate::native_bridge::wrapped_handle_value(scope, host_ptr, document)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(target) = model_context_for_document(scope, document_wrapper) else {
        return;
    };
    if check_target(scope, target).is_err() {
        return;
    }
    let removed_names = {
        let entry = &unsafe { &*host_ptr }.native_bridge().web_mcp.documents[&document];
        entry
            .tools
            .iter()
            .filter_map(|(name, tool)| {
                let ToolExecutor::Form {
                    handle, autosubmit, ..
                } = tool.executor
                else {
                    return None;
                };
                let same = definitions.iter().any(|definition| {
                    definition.handle == handle
                        && &definition.name == name
                        && definition.metadata == tool.metadata
                        && definition.autosubmit == autosubmit
                });
                (!same).then(|| name.clone())
            })
            .collect::<Vec<_>>()
    };
    let mut changes = 0;
    for name in removed_names {
        let pending = unsafe { &*host_ptr }
            .native_bridge()
            .web_mcp
            .pending
            .iter()
            .filter(|(_, pending)| {
                pending.document == document
                    && pending.name == name
                    && pending.form.as_ref().is_some_and(|form| {
                        matches!(
                            form.state,
                            FormInvocationState::Filling
                                | FormInvocationState::Ready
                                | FormInvocationState::Invalidated
                        )
                    })
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in pending {
            execution::finish_error(
                scope,
                host_ptr,
                id,
                "Tool definition changed during execution",
            );
        }
        let host = unsafe { &mut *host_ptr };
        let entry = host
            .native_bridge_mut()
            .web_mcp
            .documents
            .get_mut(&document)
            .expect("form document");
        entry.tools.remove(&name);
        let frame_id = entry.frame_id.clone();
        devtools::emit_in_tree(
            host,
            host.native_bridge().web_mcp.documents[&document].frame_tree,
            RendererWebMcpEvent::ToolsRemoved(vec![RendererWebMcpToolId { frame_id, name }]),
        );
        changes += 1;
    }
    for definition in definitions {
        let host = unsafe { &mut *host_ptr };
        let backend_node_id = host.renderer_backend_node_id_for_live_handle(definition.handle);
        let store = &mut host.native_bridge_mut().web_mcp;
        let entry = store.documents.get_mut(&document).expect("form document");
        if entry.tools.contains_key(&definition.name) {
            continue;
        }
        store.next_registration = store
            .next_registration
            .checked_add(1)
            .expect("WebMCP registration space exhausted");
        let tool = RegisteredTool {
            registration: store.next_registration,
            metadata: definition.metadata,
            stack_trace: None,
            exposed_to: Vec::new(),
            executor: ToolExecutor::Form {
                handle: definition.handle,
                autosubmit: definition.autosubmit,
                backend_node_id,
            },
            abort: None,
            registration_resolver: None,
        };
        let snapshot = devtools::protocol_tool(entry, &definition.name, &tool);
        entry.tools.insert(definition.name, tool);
        let tree = entry.frame_tree;
        devtools::emit_in_tree(host, tree, RendererWebMcpEvent::ToolsAdded(vec![snapshot]));
        changes += 1;
    }
    if changes > 0 {
        let origin = unsafe { &*host_ptr }.native_bridge().web_mcp.documents[&document]
            .origin
            .clone();
        for _ in 0..changes {
            events::queue_tool_change(scope, host_ptr, document, &origin, &[]);
        }
    }
}
