//! Native form registration following Chromium HTMLFormElement and FormMCPSchema.

use super::bindings::{check_target, document_owner, model_context_for_document};
use super::state::{FormInvocationState, RegisteredTool, ToolExecutor, ToolMetadata};
use super::tasks::queue_task;
use super::{events, execution, registry};
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::util::context_host_ptr_from_global_bridge;

use crate::dom::native::{DomHost, DomMutationEffects, Node};
use std::collections::{HashMap, HashSet};

pub(super) mod invocation;
mod schema;

pub(crate) fn note_mutation(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    dom: &DomHost,
    effects: &DomMutationEffects,
) {
    let mut candidates = HashMap::new();
    let mut affected = HashMap::new();
    let mut mark = |target, kind| {
        collect_affected_forms(dom, &mut candidates, &mut affected, target, kind);
    };
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
    for attribute in effects.style().attribute_mutations() {
        if attribute.namespace().is_some() || !schema_attribute(attribute.local_name()) {
            continue;
        }
        let target = attribute.target();
        let scope = if matches!(attribute.local_name(), "id" | "form" | "for") {
            FormMutationScope::Associations
        } else if dom.is_html_element_named(target, "fieldset") {
            FormMutationScope::Subtree
        } else {
            FormMutationScope::Target
        };
        mark(target, scope);
    }
    for mutation in effects.style().child_list_mutations() {
        let scope = if dom.is_html_element_named(mutation.target(), "fieldset")
            && mutation
                .added_nodes()
                .iter()
                .chain(mutation.removed_nodes())
                .any(|node| dom.is_html_element_named(*node, "legend"))
        {
            FormMutationScope::Subtree
        } else {
            FormMutationScope::Target
        };
        mark(mutation.target(), scope);
        for node in mutation.added_nodes() {
            mark(*node, FormMutationScope::Subtree);
        }
        for node in mutation.removed_nodes() {
            // Adoption has already changed the removed subtree's owner. Its
            // former container still identifies the document whose external
            // control associations must be refreshed.
            if dom.owner_document_handle(*node) != dom.owner_document_handle(mutation.target()) {
                mark(mutation.target(), FormMutationScope::Associations);
            }
            mark(*node, FormMutationScope::RemovedSubtree);
        }
    }
    for target in effects.style().character_data_mutations() {
        mark(*target, FormMutationScope::Target);
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
    for (document, _, handle) in &invalid {
        affected.entry(*document).or_default().insert(*handle);
    }
    for (document, forms) in affected {
        if !forms.is_empty() {
            host.native_bridge_mut()
                .web_mcp
                .dirty_forms
                .entry(document)
                .or_default()
                .extend(forms);
        }
    }
    for (document, name, _) in invalid {
        let _ = registry::take_tool(host, document, name);
        let origin = host.native_bridge().web_mcp.documents[&document]
            .origin
            .clone();
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

// The native index includes closed shadow trees. Candidate order matters only
// when synchronize_document sorts the affected forms before registration.
fn form_handles(dom: &DomHost, root: DomHandle) -> Vec<DomHandle> {
    dom.html_elements_by_local_name_in_shadow_including_subtree(root, "form")
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

enum FormMutationScope {
    Target,
    Subtree,
    RemovedSubtree,
    Associations,
}

fn collect_affected_forms(
    dom: &DomHost,
    candidates: &mut HashMap<DomHandle, Vec<DomHandle>>,
    affected: &mut HashMap<DomHandle, HashSet<DomHandle>>,
    target: DomHandle,
    scope: FormMutationScope,
) {
    if !dom.is_connected(target) && !matches!(scope, FormMutationScope::RemovedSubtree) {
        return;
    }
    let Some(document) = dom.owner_document_handle(target) else {
        return;
    };
    // Discovery is independent of successful registration: name-conflicting
    // candidates still need to retry when their controls change.
    let candidates = candidates.entry(document).or_insert_with(|| {
        form_handles(dom, document)
            .into_iter()
            .filter(|form| form_registration_is_valid(dom, *form))
            .collect()
    });
    if candidates.is_empty() {
        return;
    }
    let affected = affected.entry(document).or_default();
    let mut all = matches!(scope, FormMutationScope::Associations);
    let mut mark_owner = |handle| {
        if dom.is_html_element_named(handle, "form") && form_registration_is_valid(dom, handle) {
            affected.insert(handle);
        }
        if let Some(owner) = dom.form_control_owner(handle)
            && form_registration_is_valid(dom, owner)
        {
            affected.insert(owner);
        }
    };
    let mut ancestor = Some(target);
    while let Some(handle) = ancestor {
        mark_owner(handle);
        all |= dom.is_html_element_named(handle, "label");
        ancestor = dom.parent_node(handle);
    }
    if matches!(
        scope,
        FormMutationScope::Subtree | FormMutationScope::RemovedSubtree
    ) {
        let mut stack = vec![target];
        while let Some(handle) = stack.pop() {
            mark_owner(handle);
            if let Some(element) = dom.node(handle).and_then(Node::as_element) {
                // ID/label association changes can affect both the old and new
                // owners. A removed external control has already lost its old
                // owner, so conservatively update candidates in that document.
                all |= element.has_attribute("id")
                    || element.is_html_label()
                    || (matches!(scope, FormMutationScope::RemovedSubtree)
                        && element.namespace() == "http://www.w3.org/1999/xhtml"
                        && matches!(
                            element.local_name(),
                            "button"
                                | "fieldset"
                                | "input"
                                | "object"
                                | "output"
                                | "select"
                                | "textarea"
                        ));
            }
            if all {
                break;
            }
            stack.extend(dom.child_handles(handle));
            if let Some(shadow) = dom.shadow_root_handle(handle) {
                stack.push(shadow);
            }
        }
    }
    if all {
        affected.extend(candidates.iter().copied());
    }
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
    let initial_forms = if needs_initial_scan {
        form_handles(host.dom_host(), document)
            .into_iter()
            .filter(|form| form_registration_is_valid(host.dom_host(), *form))
            .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let store = &mut host.native_bridge_mut().web_mcp;
    if needs_initial_scan {
        // The scheduler discards tasks owned by the previous document. Its
        // queued marker must not prevent the replacement document's scan.
        store.form_registration_task_queued = false;
        store.initialized_form_owner = owner;
        if !initial_forms.is_empty() {
            store
                .dirty_forms
                .entry(document)
                .or_default()
                .extend(initial_forms);
        }
    }
    !store.form_registration_task_queued && !store.dirty_forms.is_empty()
}

/// Queue the task after `prepare_registration_task` returned true.
pub(crate) fn queue_registration_task(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
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
        std::mem::take(&mut store.dirty_forms)
            .into_iter()
            .collect::<Vec<_>>()
    };
    documents.sort_unstable_by_key(|(document, _)| document.index());
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
    for (document, forms) in documents {
        synchronize_document(scope, host_ptr, document, forms);
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
    forms: impl IntoIterator<Item = DomHandle>,
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
    for handle in forms {
        if !form_registration_is_valid(dom, handle)
            || dom.owner_document_handle(handle) != Some(document)
        {
            continue;
        }
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
    affected_forms: HashSet<DomHandle>,
) {
    let mut forms = affected_forms.iter().copied().collect::<Vec<_>>();
    forms.sort_unstable_by(|left, right| {
        unsafe { &*host_ptr }
            .dom_host()
            .compare_handles_in_shadow_including_tree_order(*left, *right)
    });
    let definitions = form_tools(scope, unsafe { &*host_ptr }, document, forms);
    let definitions_by_handle = definitions
        .iter()
        .map(|definition| (definition.handle, definition))
        .collect::<HashMap<_, _>>();
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
                if !affected_forms.contains(&handle) {
                    return None;
                }
                let same = definitions_by_handle
                    .get(&handle)
                    .is_some_and(|definition| {
                        &definition.name == name
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
        let _ = registry::take_tool(host, document, name);
        changes += 1;
    }
    for definition in definitions {
        let host = unsafe { &mut *host_ptr };
        if host.native_bridge().web_mcp.documents[&document]
            .tools
            .contains_key(&definition.name)
        {
            continue;
        }
        let backend_node_id = host.renderer_backend_node_id_for_live_handle(definition.handle);
        let tool = RegisteredTool {
            metadata: definition.metadata,
            stack_trace: None,
            exposed_to: Vec::new(),
            executor: ToolExecutor::Form {
                handle: definition.handle,
                autosubmit: definition.autosubmit,
                backend_node_id,
            },
        };
        registry::insert_tool(host, document, definition.name, tool);
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
