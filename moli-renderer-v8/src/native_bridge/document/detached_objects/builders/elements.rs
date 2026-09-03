use super::*;
use crate::util::{
    call_global_bridge_method, context_host_ptr_from_global_bridge, get_private_value,
    set_private_value, v8str,
};
use crate::{
    context_bootstrap::ensure_intrinsic_interface_prototype,
    custom_elements,
    dom::native::CustomElementState,
    native_bridge::document::detached_install::{
        install_detached_anchor_instance_properties,
        install_detached_form_associated_instance_properties,
        install_detached_form_control_instance_properties,
        install_detached_form_instance_properties, install_detached_iframe_instance_properties,
        install_detached_image_instance_properties, install_detached_label_instance_properties,
        install_detached_option_instance_properties, install_detached_select_instance_properties,
        install_detached_text_replacement_instance_properties,
    },
};
use moli_webapi_declare::WebApiObject;

const DETACHED_ELEMENT_BRIDGE_PROTOTYPE_SLOT: &str = "__moliDetachedElementBridgePrototype";

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct GenericHtmlElementProxyHandlerDeclaration {
    #[webapi(method, length = 3, callback = generic_html_element_proxy_get_callback)]
    get: (),
    #[webapi(method, length = 2, callback = generic_html_element_proxy_has_callback)]
    has: (),
    #[webapi(method, length = 4, callback = generic_html_element_proxy_set_callback)]
    set: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct SelectHtmlElementProxyHandlerDeclaration {
    #[webapi(method, length = 3, callback = select_html_element_proxy_get_callback)]
    get: (),
    #[webapi(method, length = 2, callback = select_html_element_proxy_has_callback)]
    has: (),
    #[webapi(method, length = 4, callback = select_html_element_proxy_set_callback)]
    set: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct DetachedElementObjectDeclaration<'scope, 'tag> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,

    #[webapi(to_string_tag)]
    to_string_tag: Option<&'tag str>,
}

fn element_interface_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    constructor_name: Option<&str>,
    html_like: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(constructor_name) = constructor_name
        && let Some(prototype) =
            owner_document_constructor_prototype(scope, owner_document, constructor_name)
                .or_else(|| ensure_intrinsic_interface_prototype(scope, constructor_name).ok())
    {
        return Some(prototype);
    }
    if html_like {
        owner_document_constructor_prototype(scope, owner_document, "HTMLElement")
            .or_else(|| ensure_intrinsic_interface_prototype(scope, "HTMLElement").ok())
            .or_else(|| ensure_intrinsic_interface_prototype(scope, "Element").ok())
    } else {
        owner_document_constructor_prototype(scope, owner_document, "Element")
            .or_else(|| ensure_intrinsic_interface_prototype(scope, "Element").ok())
    }
}

fn owner_document_constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    constructor_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let default_view = owner_document.get(scope, v8str(scope, "defaultView").into())?;
    if default_view.is_null_or_undefined() {
        return None;
    }
    let default_view = v8::Local::<v8::Object>::try_from(default_view).ok()?;
    let context = default_view.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    ensure_intrinsic_interface_prototype(scope, constructor_name).ok()
}

pub(in crate::native_bridge::document) fn generic_html_element_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let handler = GenericHtmlElementProxyHandlerDeclaration::default()
        .bind(scope)
        .ok()?;
    let proxy = v8::Proxy::new(scope, target, handler)?;
    let proxy: v8::Local<'s, v8::Value> = proxy.into();
    v8::Local::<v8::Object>::try_from(proxy).ok()
}

pub(in crate::native_bridge::document) fn select_html_element_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let handler = SelectHtmlElementProxyHandlerDeclaration {
        get: (),
        has: (),
        set: (),
    }
    .bind(scope)
    .ok()?;
    let proxy = v8::Proxy::new(scope, target, handler)?;
    let proxy: v8::Local<'s, v8::Value> = proxy.into();
    v8::Local::<v8::Object>::try_from(proxy).ok()
}

fn generic_html_element_proxy_hidden_key(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    value
        .to_string(scope)
        .map(|value| {
            matches!(
                value.to_rust_string_lossy(scope).as_str(),
                "href" | "name" | "src"
            )
        })
        .unwrap_or(false)
}

fn generic_html_element_proxy_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let key = args
        .get(1)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if generic_html_element_proxy_hidden_key_name(&key) {
        rv.set_undefined();
        return;
    }
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_undefined();
        return;
    };
    if let Ok(receiver) = v8::Local::<v8::Object>::try_from(args.get(2))
        && let Some(helper) = generic_html_element_proxy_bridge_getter(&key)
        && let Some(value) = call_global_bridge_method(scope, helper, &[receiver.into()])
    {
        rv.set(value);
        return;
    }
    if let Some(method) = detached_element_proxy_bridge_method(scope, &key) {
        rv.set(method);
        return;
    }
    let receiver = v8::Local::<v8::Object>::try_from(args.get(2)).unwrap_or(target);
    match target.get_with_receiver(scope, args.get(1), receiver) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn generic_html_element_proxy_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if generic_html_element_proxy_hidden_key(scope, args.get(1)) {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    }
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    rv.set(v8::Boolean::new(scope, target.has(scope, args.get(1)).unwrap_or(false)).into());
}

fn generic_html_element_proxy_hidden_key_name(name: &str) -> bool {
    matches!(name, "href" | "name" | "src")
}

fn generic_html_element_proxy_bridge_getter(name: &str) -> Option<&'static str> {
    Some(match name {
        "nodeType" => "__detachedNodeType",
        "nodeName" => "__detachedNodeName",
        "parentNode" => "__detachedParentNode",
        "parentElement" => "__detachedParentElement",
        "ownerDocument" => "__detachedOwnerDocument",
        "childNodes" => "__detachedChildNodes",
        "firstChild" => "__detachedFirstChild",
        "lastChild" => "__detachedLastChild",
        "previousSibling" => "__detachedPreviousSibling",
        "nextSibling" => "__detachedNextSibling",
        "children" => "__detachedChildren",
        "firstElementChild" => "__detachedFirstElementChild",
        "lastElementChild" => "__detachedLastElementChild",
        "previousElementSibling" => "__detachedPreviousElementSibling",
        "nextElementSibling" => "__detachedNextElementSibling",
        "childElementCount" => "__detachedChildElementCount",
        "textContent" => "__detachedTextContent",
        "nodeValue" => "__detachedNodeValue",
        "namespaceURI" => "__detachedElementNamespaceURI",
        "prefix" => "__detachedElementPrefix",
        "localName" => "__detachedElementLocalName",
        "tagName" => "__detachedElementTagName",
        _ => return None,
    })
}

fn detached_element_proxy_bridge_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let bridge = bridge_prototype_object(scope, "__detachedElementPrototype")?;
    let key = v8_string(scope, name)?;
    if !bridge.has_own_property(scope, key.into()).unwrap_or(false) {
        return None;
    }
    let value = bridge.get(scope, key.into())?;
    value.is_function().then_some(value)
}

fn generic_html_element_proxy_set_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if generic_html_element_proxy_hidden_key(scope, args.get(1)) {
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    }
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let receiver = v8::Local::<v8::Object>::try_from(args.get(3)).unwrap_or(target);
    rv.set(
        v8::Boolean::new(
            scope,
            target
                .set_with_receiver(scope, args.get(1), args.get(2), receiver)
                .unwrap_or(false),
        )
        .into(),
    );
}

fn select_html_element_proxy_index_key(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Value>,
) -> Option<u32> {
    let key = key.to_string(scope)?;
    array_index_property_name(&key.to_rust_string_lossy(scope))
}

fn select_html_element_indexed_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Value>> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let select_handle = detached_native_handle_for_runtime(scope, runtime_ptr, target)?;
    let option = unsafe { &*runtime_ptr }
        .dom_host()
        .select_option_elements(select_handle)
        .get(index as usize)
        .copied()?;
    detached_native_object_for_handle(scope, runtime_ptr, option).map(Into::into)
}

fn select_html_element_proxy_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_undefined();
        return;
    };
    if let Some(index) = select_html_element_proxy_index_key(scope, args.get(1))
        && let Some(value) = select_html_element_indexed_value(scope, target, index)
    {
        rv.set(value);
        return;
    }
    if args
        .get(1)
        .to_string(scope)
        .is_some_and(|key| key.to_rust_string_lossy(scope) == "remove")
        && let Some(function) =
            v8::Function::builder(crate::native_bridge::element::select_remove_callback)
                .length(0)
                .build(scope)
    {
        function.set_name(v8str(scope, "remove"));
        rv.set(function.into());
        return;
    }
    if let Some(key) = args.get(1).to_string(scope)
        && let Some(method) =
            detached_element_proxy_bridge_method(scope, &key.to_rust_string_lossy(scope))
    {
        rv.set(method);
        return;
    }
    let receiver = v8::Local::<v8::Object>::try_from(args.get(2)).unwrap_or(target);
    match target.get_with_receiver(scope, args.get(1), receiver) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn select_html_element_proxy_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    if let Some(index) = select_html_element_proxy_index_key(scope, args.get(1))
        && select_html_element_indexed_value(scope, target, index).is_some()
    {
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    }
    rv.set(v8::Boolean::new(scope, target.has(scope, args.get(1)).unwrap_or(false)).into());
}

fn select_html_element_proxy_set_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    if let Some(index) = select_html_element_proxy_index_key(scope, args.get(1))
        && let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(select_handle) = detached_native_handle_for_runtime(scope, runtime_ptr, target)
        && crate::native_bridge::element::set_select_indexed_option(
            scope,
            runtime_ptr,
            select_handle,
            index,
            args.get(2),
        )
    {
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    }
    rv.set(
        v8::Boolean::new(
            scope,
            target.set(scope, args.get(1), args.get(2)).unwrap_or(false),
        )
        .into(),
    );
}

pub(in crate::native_bridge::document) fn mirror_detached_private_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
    target: v8::Local<'s, v8::Object>,
) {
    for slot in [
        DETACHED_STATE_SLOT,
        DETACHED_LIVE_DELEGATE_SLOT,
        DETACHED_NATIVE_HANDLE_SLOT,
    ] {
        if let Some(value) = get_private_value(scope, source, slot) {
            set_private_value(scope, target, slot, value);
        }
    }
}

pub(in crate::native_bridge::document) fn copy_detached_element_bridge_members<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) {
    let Some(parent) = target.get_prototype(scope) else {
        return;
    };
    if v8::Local::<v8::Object>::try_from(parent).is_err() {
        return;
    }
    if get_private_value(scope, target, DETACHED_ELEMENT_BRIDGE_PROTOTYPE_SLOT).is_some() {
        return;
    }

    set_private_value(
        scope,
        target,
        DETACHED_ELEMENT_BRIDGE_PROTOTYPE_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}

pub(crate) fn preserve_detached_element_bridge_for_custom_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    element: v8::Local<'s, v8::Object>,
    custom_prototype: v8::Local<'s, v8::Object>,
) {
    if detached_native_element_runtime_and_handle(scope, element).is_some() {
        copy_detached_element_bridge_members(scope, custom_prototype);
    }
}

pub(in crate::native_bridge::document) fn remove_detached_element_instance_selector_matching_methods<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) {
    for name in ["matches", "webkitMatchesSelector"] {
        if let Some(key) = v8_string(scope, name) {
            let _ = target.delete(scope, key.into());
        }
    }
}

pub(in crate::native_bridge::document) fn build_detached_element_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner_document: v8::Local<'s, v8::Object>,
    qualified_name: &str,
    namespace_uri: Option<String>,
    document_kind: &str,
    split_qualified_name: bool,
    is_name: Option<&str>,
    registry_association: Option<custom_elements::CustomElementRegistryAssociation>,
) -> Option<v8::Local<'s, v8::Object>> {
    let qualified = qualified_name.to_owned();
    let (prefix, local_name_raw) = if split_qualified_name {
        qualified_name_parts(&qualified)
    } else {
        (None, qualified.clone())
    };
    let namespace_uri = normalize_namespace(namespace_uri);
    let html_like = document_kind == "html" && namespace_uri.as_deref() == Some(XHTML_NS);
    let local_name = if html_like {
        local_name_raw.to_ascii_lowercase()
    } else {
        local_name_raw
    };
    let html_interface_like = (html_like || namespace_uri.as_deref() == Some(XHTML_NS))
        && html_element_constructor_name(&local_name).is_some();
    let svg_interface_like = namespace_uri.as_deref() == Some(SVG_NS)
        && svg_element_constructor_name(&local_name).is_some();
    let node_name = if html_like {
        qualified.to_ascii_uppercase()
    } else {
        qualified.clone()
    };
    let to_string_tag = if html_interface_like {
        Some(html_element_to_string_tag(&local_name))
    } else if namespace_uri.as_deref() == Some(SVG_NS) {
        Some(svg_element_to_string_tag(&local_name))
    } else {
        Some("Element")
    };
    // For known element types, use a type-specific bridge prototype that
    // inherits from the constructor prototype (for instanceof) while carrying
    // detached DOM methods.  Fall back to the generic element prototype.
    let prototype_name = if html_interface_like {
        html_element_constructor_name(&local_name)
    } else if svg_interface_like {
        svg_element_constructor_name(&local_name)
    } else {
        None
    };
    let mut object = if let Some(proto) =
        element_interface_prototype(scope, owner_document, prototype_name, html_interface_like)
    {
        DetachedElementObjectDeclaration::new(proto, to_string_tag)
            .bind(scope)
            .ok()?
    } else {
        new_detached_object_with_prototype(scope, "__detachedElementPrototype", to_string_tag)?
    };
    let state = new_detached_state_object(scope, "element", 1, &node_name)?;
    let _ = state.set(
        scope,
        v8str(scope, "ownerDocument").into(),
        owner_document.into(),
    );
    let _ = state.set(
        scope,
        v8str(scope, "localName").into(),
        v8_string(scope, &local_name)?.into(),
    );
    let namespace_value = match namespace_uri.as_deref() {
        Some(value) => v8_string(scope, value)?.into(),
        None => v8::null(scope).into(),
    };
    let _ = state.set(scope, v8str(scope, "namespaceURI").into(), namespace_value);
    let prefix_value = match prefix.as_deref() {
        Some(value) => v8_string(scope, value)?.into(),
        None => v8::null(scope).into(),
    };
    let _ = state.set(scope, v8str(scope, "prefix").into(), prefix_value);
    let _ = state.set(
        scope,
        v8str(scope, "qualifiedName").into(),
        v8_string(scope, &qualified)?.into(),
    );
    let _ = state.set(
        scope,
        v8str(scope, "documentKind").into(),
        v8_string(scope, document_kind)?.into(),
    );
    let attributes = new_map_object(scope);
    if let Some(is_name) = is_name {
        detached_map_set(scope, attributes, "is", is_name);
    }
    let _ = state.set(scope, v8str(scope, "attributes").into(), attributes.into());
    let namespace_attributes = new_map_object(scope);
    let _ = state.set(
        scope,
        v8str(scope, "namespaceAttributes").into(),
        namespace_attributes.into(),
    );
    define_detached_state(scope, object, state);
    let mut native_handle = None;
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(owner_document_handle) = detached_native_handle(scope, owner_document)
    {
        let handle = if split_qualified_name {
            unsafe { &mut *runtime_ptr }.create_element_ns(namespace_uri.as_deref(), &qualified)?
        } else {
            unsafe { &mut *runtime_ptr }
                .dom_host_mut()
                .create_element_with_parts(namespace_uri.as_deref(), prefix.as_deref(), &local_name)
        };
        let _ = initialize_new_detached_native_node_owner_document(
            runtime_ptr,
            owner_document_handle,
            handle,
        );
        if let Some(is_name) = is_name {
            let runtime = unsafe { &mut *runtime_ptr };
            let _ = runtime.dom_host_mut().set_attribute(handle, "is", is_name);
            if runtime
                .dom_host_mut()
                .set_custom_element_state(handle, CustomElementState::Undefined)
            {
                runtime.note_style_subtree_context_change(handle);
            }
        }
        let effective_registry_association = registry_association.unwrap_or_else(|| {
            unsafe { &*runtime_ptr }
                .effective_custom_element_registry_association(owner_document_handle)
        });
        unsafe { &mut *runtime_ptr }
            .set_custom_element_registry_association(handle, effective_registry_association);
        define_detached_native_handle(scope, object, handle);
        native_handle = Some((runtime_ptr, handle));
    }
    install_detached_element_instance_properties(scope, object);
    copy_detached_element_bridge_members(scope, object);
    remove_detached_element_instance_selector_matching_methods(scope, object);
    if html_interface_like && matches!(local_name.as_str(), "a" | "area") {
        install_detached_anchor_instance_properties(scope, object);
    }
    if html_like && local_name == "a" {
        install_detached_text_replacement_instance_properties(scope, object);
    }
    if html_like && local_name == "option" {
        install_detached_option_instance_properties(scope, object);
    }
    if html_like && local_name == "label" {
        install_detached_label_instance_properties(scope, object);
    }
    if html_like && local_name == "img" {
        install_detached_image_instance_properties(scope, object);
    }
    if html_like
        && matches!(
            local_name.as_str(),
            "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"
        )
    {
        install_detached_form_associated_instance_properties(scope, object);
    }
    if html_like && local_name == "iframe" {
        install_detached_iframe_instance_properties(scope, object);
    }
    if html_like && local_name == "form" {
        install_detached_form_instance_properties(scope, object);
    }
    if html_like
        && matches!(
            local_name.as_str(),
            "button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea"
        )
    {
        install_detached_form_control_instance_properties(scope, object);
    }
    if html_like && local_name == "select" {
        install_detached_select_instance_properties(scope, object);
        if let Some(proxy) = select_html_element_proxy(scope, object) {
            mirror_detached_private_slots(scope, object, proxy);
            if let Some((_, handle)) = native_handle {
                define_detached_native_handle(scope, proxy, handle);
            }
            object = proxy;
        }
    }

    // Run custom-element upgrade only after the detached wrapper has its own
    // DOM surface installed. The HTML constructor returns this object from the
    // construction stack, so constructor code must not observe a half-built
    // element wrapper.
    if let Some((runtime_ptr, handle)) = native_handle {
        let _ = custom_elements::upgrade_element_with_wrapper_if_defined(
            scope,
            runtime_ptr,
            object,
            handle,
        );
    }

    Some(object)
}
