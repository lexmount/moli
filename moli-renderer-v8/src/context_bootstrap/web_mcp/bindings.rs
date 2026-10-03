//! ModelContext bindings, document ownership, and target policy checks.

use super::devtools::document_frame_id;
use super::state::DocumentTools;
use super::{api, events};
use crate::document_runtime::DomHandle;
use crate::native_bridge::{JsContextHost, WindowDocumentOwner};
use crate::util::{context_host_ptr_from_global_bridge, get_private_object, set_private_value};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use std::collections::BTreeMap;

const DOCUMENT_SLOT: &str = "__moliModelContextDocument";
const MODEL_CONTEXT_SLOT: &str = "__moliDocumentModelContext";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ModelContext, prototype = "Object")]
struct ModelContextObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = DOCUMENT_SLOT)]
    document: v8::Local<'s, v8::Object>,
    #[webapi(slot = crate::context_bootstrap::SIMPLE_EVENT_TARGET_SLOT, value = events::LISTENERS_SLOT)]
    event_target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ModelContext, enumerable)]
struct ModelContextPrototype {
    #[webapi(method = "registerTool", length = 1, callback = api::register_tool_callback)]
    register_tool: (),
    #[webapi(method = "getTools", length = 0, callback = api::get_tools_callback)]
    get_tools: (),
    #[webapi(method = "executeTool", length = 1, callback = api::execute_tool_callback)]
    execute_tool: (),
    #[webapi(accessor_property = "ontoolchange", getter = events::handler_getter, setter = events::handler_setter, data = v8::Integer::new(scope, 0))]
    on_tool_change: (),
    #[webapi(accessor_property = "ontoolactivated", getter = events::handler_getter, setter = events::handler_setter, data = v8::Integer::new(scope, 1))]
    on_tool_activated: (),
    #[webapi(accessor_property = "ontoolcancel", getter = events::handler_getter, setter = events::handler_setter, data = v8::Integer::new(scope, 2))]
    on_tool_cancel: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Document, enumerable, receiver)]
struct DocumentModelContextPrototype {
    #[webapi(accessor_property = "modelContext", getter = model_context_getter)]
    model_context: (),
}

pub(in crate::context_bootstrap) fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    if name == "ModelContext" {
        ModelContextPrototype::initialize_prototype_template(scope, prototype);
    } else if name == "Document" {
        DocumentModelContextPrototype::initialize_prototype_template(scope, prototype);
    }
}

pub(in crate::context_bootstrap) fn filter_exposure(
    scope: &mut v8::PinScope<'_, '_>,
    secure: bool,
) -> anyhow::Result<()> {
    if !secure {
        let prototype =
            crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, "Document")?;
        let key = js_string(scope, "modelContext");
        anyhow::ensure!(
            prototype.delete(scope, key.into()) == Some(true),
            "failed to remove insecure Document.modelContext"
        );
    }
    Ok(())
}

fn model_context_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(target) = model_context_for_document(scope, args.this()) {
        rv.set(target.into());
    }
}

pub(super) fn model_context_for_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(context) = get_private_object(scope, document, MODEL_CONTEXT_SLOT) {
        return Some(context);
    }
    let Ok((_, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, document)
    else {
        return None;
    };
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let prototype =
        crate::context_bootstrap::ensure_intrinsic_interface_prototype(scope, "ModelContext")
            .ok()?;
    let Ok(target) = ModelContextObject {
        prototype,
        document,
        event_target: (),
    }
    .bind(scope) else {
        return None;
    };
    crate::context_bootstrap::media_queries::install_simple_event_target_ordered_handlers(
        scope, target,
    );
    set_private_value(scope, document, MODEL_CONTEXT_SLOT, target.into());
    if let Some(owner) = document_owner(unsafe { &*host_ptr }, handle) {
        bind_document_target(scope, host_ptr, handle, target, owner);
    }
    Some(target)
}

fn bind_document_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    document: DomHandle,
    target: v8::Local<'s, v8::Object>,
    owner: WindowDocumentOwner,
) {
    let host = unsafe { &*host_ptr };
    let shared = host
        .native_bridge()
        .web_mcp
        .documents
        .get(&document)
        .filter(|entry| entry.owner == owner)
        .map(|entry| v8::Local::new(scope, &entry.target));
    if let Some(shared) = shared {
        crate::context_bootstrap::shared_event_targets::bind_shared_target(scope, target, shared);
        return;
    }
    let context = target
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let window = document_window(scope, host, document, context);
    let entry = DocumentTools {
        owner,
        target: v8::Global::new(scope, target),
        window: v8::Global::new(scope, window),
        origin: document_origin(host, document),
        frame_id: document_frame_id(host, document),
        frame_tree: document_frame_tree(host, document),
        tools: BTreeMap::new(),
    };
    unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .web_mcp
        .documents
        .insert(document, entry);
    crate::context_bootstrap::shared_event_targets::bind_shared_target(scope, target, target);
}

pub(super) fn document_owner(
    host: &JsContextHost,
    document: DomHandle,
) -> Option<WindowDocumentOwner> {
    if document == host.document_handle() {
        return host
            .current_main_document_task_owner()
            .map(WindowDocumentOwner::Frame);
    }
    if let Some(popup) = host.lightweight_popup_id_for_document_handle(document) {
        return host
            .current_lightweight_popup_document_owner(popup)
            .map(WindowDocumentOwner::LightweightPopup);
    }
    let handle = host.child_browsing_context_host_for_document_handle(document)?;
    if !host.child_browsing_context_host_is_active(handle) {
        return None;
    }
    host.current_child_document_task_owner(handle)
        .map(WindowDocumentOwner::Frame)
}

pub(super) fn document_frame_tree(host: &JsContextHost, document: DomHandle) -> Option<u64> {
    let mut current = document;
    loop {
        if let Some(popup) = host.lightweight_popup_id_for_document_handle(current) {
            return Some(popup);
        }
        let handle = host.child_browsing_context_host_for_document_handle(current)?;
        current = host.dom_host().node(handle)?.owner_document()?;
    }
}

fn document_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    document: DomHandle,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Object> {
    if let Some(popup) = host.lightweight_popup_id_for_document_handle(document)
        && let Some(window) = host.lightweight_popup_window(scope, popup)
    {
        return window;
    }
    host.child_browsing_context_host_for_document_handle(document)
        .and_then(|handle| host.existing_child_browsing_context_window_wrapper(scope, handle))
        .unwrap_or_else(|| context.global(scope))
}

pub(super) fn document_origin(host: &JsContextHost, document: DomHandle) -> url::Origin {
    if document == host.document_handle() && host.document_sandbox_policy().forces_opaque_origin
        || host
            .child_browsing_context_host_for_document_handle(document)
            .is_some_and(|child| host.child_browsing_context_has_opaque_origin(child))
    {
        return url::Origin::new_opaque();
    }
    host.lightweight_popup_id_for_document_handle(document)
        .and_then(|popup| host.lightweight_popup_origin(popup))
        .or_else(|| {
            host.child_browsing_context_host_for_document_handle(document)
                .and_then(|child| host.child_browsing_context_target_origin(child))
        })
        .and_then(|origin| url::Url::parse(&origin).ok())
        .map_or_else(
            || host.document_url_for_handle(document).origin(),
            |url| url.origin(),
        )
}

pub(super) fn document_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let document = get_private_object(scope, target, DOCUMENT_SLOT)?;
    crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, document)
        .ok()
        .map(|(_, handle)| handle)
}

pub(super) fn check_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Result<(*mut JsContextHost, DomHandle), v8::Local<'s, v8::Value>> {
    if !web_api_interfaces::ModelContext::is_instance(scope, target) {
        return Err(v8::Exception::type_error(
            scope,
            js_string(scope, "Illegal invocation"),
        ));
    }
    let active = (|| {
        let host_ptr = context_host_ptr_from_global_bridge(scope)?;
        let document = document_for_target(scope, target)?;
        let owner = document_owner(unsafe { &*host_ptr }, document)?;
        Some((host_ptr, document, owner))
    })();
    let Some((host_ptr, document, owner)) = active else {
        return Err(dom_error(
            scope,
            "InvalidStateError",
            "The document is not active.",
        ));
    };
    if !unsafe { &*host_ptr }
        .document_permissions_policy_for_document_handle(document)
        .is_some_and(|policy| policy.tools_enabled())
    {
        return Err(dom_error(
            scope,
            "NotAllowedError",
            "Access to the feature tools is disallowed by permissions policy.",
        ));
    }
    // document.open keeps its Document wrapper while retiring its previous
    // generation. A cached ModelContext must acquire fresh registration state.
    bind_document_target(scope, host_ptr, document, target, owner);
    Ok((host_ptr, document))
}

pub(super) fn dom_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    crate::native_bridge::abort::dom_exception_value(scope, message, name)
}

pub(super) fn js_string<'s>(
    scope: &v8::PinScope<'s, '_, ()>,
    value: &str,
) -> v8::Local<'s, v8::String> {
    v8::String::new(scope, value).expect("WebMCP string")
}
