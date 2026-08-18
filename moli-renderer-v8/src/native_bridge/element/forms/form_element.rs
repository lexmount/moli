use super::*;
use crate::custom_elements::is_form_associated_custom_element_handle;
use crate::native_bridge::element::{html_element_getter_receiver, html_element_setter_receiver};
use crate::util::throw_type_error;
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

pub(in crate::native_bridge) fn form_action_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = element_attribute(runtime, handle, "action")
        .filter(|value| !value.is_empty())
        .map(|_| resolve_url_like_attribute(runtime, handle, "action"))
        .unwrap_or_else(|| form_owner_document_url(runtime, handle));
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_null();
    }
}

pub(in crate::native_bridge) fn form_action_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_form_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "action",
        args.get(0),
        "HTMLFormElement",
        "action",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_accept_charset_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "accept-charset", rv);
}

pub(in crate::native_bridge) fn form_accept_charset_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_form_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "accept-charset",
        args.get(0),
        "HTMLFormElement",
        "acceptCharset",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_autocomplete_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_empty_string();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "autocomplete")
        .filter(|value| value.eq_ignore_ascii_case("off"))
        .map(|_| "off")
        .unwrap_or("on");
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn form_autocomplete_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_form_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "autocomplete",
        args.get(0),
        "HTMLFormElement",
        "autocomplete",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_enctype_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_empty_string();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "enctype")
        .map(|value| normalized_form_enctype(&value))
        .unwrap_or("application/x-www-form-urlencoded");
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn form_enctype_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_form_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "enctype",
        args.get(0),
        "HTMLFormElement",
        "enctype",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_encoding_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    form_enctype_getter_function(scope, args, rv);
}

pub(in crate::native_bridge) fn form_encoding_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_form_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "enctype",
        args.get(0),
        "HTMLFormElement",
        "encoding",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_method_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_empty_string();
        return;
    };
    let method = element_attribute(unsafe { &*runtime_ptr }, handle, "method")
        .map(|value| normalized_form_method(&value).to_owned())
        .unwrap_or_else(|| "get".to_owned());
    if let Some(value) = v8_string(scope, &method) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn form_method_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let Some(method) =
        form_dom_string_property_value(scope, args.get(0), "HTMLFormElement", "method", false)
    else {
        rv.set_undefined();
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "method", &method);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_no_validate_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), "novalidate", rv);
}

pub(in crate::native_bridge) fn form_no_validate_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    if unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some()
    {
        set_reflected_boolean_attribute(
            scope,
            runtime_ptr,
            handle,
            "novalidate",
            args.get(0).boolean_value(scope),
        );
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_elements_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let descriptor = LiveCollectionDescriptor {
        collection_kind: CollectionKind::FormControlsCollection,
        query_kind: LiveCollectionQueryKind::FormControls,
        root: handle,
        query: None,
        include_root: false,
        tag_name_html_document: None,
        resolution_cache: Default::default(),
    };
    let collection = collections::build_live_collection_wrapper(scope, runtime_ptr, descriptor);
    rv.set(collection.into());
}

pub(crate) fn form_control_elements(
    runtime: &JsContextHost,
    form_handle: DomHandle,
) -> Vec<DomHandle> {
    if runtime
        .dom_host()
        .is_html_element_named(form_handle, "fieldset")
    {
        return collect_form_control_elements_from(runtime, form_handle, false, None, false);
    }

    if !runtime
        .dom_host()
        .is_html_element_named(form_handle, "form")
    {
        return Vec::new();
    }

    if !runtime.dom_host().is_connected(form_handle) {
        return collect_form_control_elements_from(
            runtime,
            form_handle,
            false,
            Some(form_handle),
            false,
        );
    }

    let document_handle = runtime
        .dom_host()
        .owner_document_handle(form_handle)
        .unwrap_or_else(|| runtime.dom_host().document_handle());
    collect_form_control_elements_from(runtime, document_handle, false, Some(form_handle), false)
}

pub(crate) fn form_data_control_elements(
    runtime: &JsContextHost,
    form_handle: DomHandle,
) -> Vec<DomHandle> {
    if !runtime
        .dom_host()
        .is_html_element_named(form_handle, "form")
    {
        return Vec::new();
    }

    if !runtime.dom_host().is_connected(form_handle) {
        return collect_form_control_elements_from(
            runtime,
            form_handle,
            false,
            Some(form_handle),
            true,
        );
    }

    let document_handle = runtime
        .dom_host()
        .owner_document_handle(form_handle)
        .unwrap_or_else(|| runtime.dom_host().document_handle());
    collect_form_control_elements_from(runtime, document_handle, false, Some(form_handle), true)
}

pub(crate) fn autofill_related_form_control_elements(
    runtime: &JsContextHost,
    anchor: DomHandle,
) -> Vec<DomHandle> {
    if !is_form_control_handle(runtime, anchor, false) {
        return Vec::new();
    }
    if let Some(form) = form_associated_form_owner(runtime, anchor) {
        return form_control_elements(runtime, form);
    }
    let Some(document) = runtime.dom_host().owner_document_handle(anchor) else {
        return Vec::new();
    };
    collect_form_control_elements_from(runtime, document, false, None, false)
        .into_iter()
        .filter(|candidate| form_associated_form_owner(runtime, *candidate).is_none())
        .collect()
}

fn collect_form_control_elements_from(
    runtime: &JsContextHost,
    root: DomHandle,
    include_root: bool,
    form_handle: Option<DomHandle>,
    include_image_inputs: bool,
) -> Vec<DomHandle> {
    let mut out = Vec::new();
    let mut stack = Vec::new();
    if include_root {
        stack.push(root);
    } else {
        push_shadow_including_children(runtime, root, &mut stack);
    }
    while let Some(handle) = stack.pop() {
        if is_form_control_handle(runtime, handle, include_image_inputs)
            && form_handle
                .is_none_or(|owner| form_associated_form_owner(runtime, handle) == Some(owner))
        {
            out.push(handle);
        }
        push_shadow_including_children(runtime, handle, &mut stack);
    }
    out
}

fn push_shadow_including_children(
    runtime: &JsContextHost,
    handle: DomHandle,
    stack: &mut Vec<DomHandle>,
) {
    let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
    if runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some()
        && let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
    {
        let mut shadow_children = runtime
            .dom_host()
            .child_handles(shadow_root)
            .collect::<Vec<_>>();
        shadow_children.append(&mut children);
        children = shadow_children;
    }
    stack.extend(children.into_iter().rev());
}

fn is_form_control_handle(
    runtime: &JsContextHost,
    handle: DomHandle,
    include_image_inputs: bool,
) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .is_some_and(|element| {
            if is_form_associated_custom_element_handle(runtime, handle) {
                return true;
            }
            if element.namespace() != "http://www.w3.org/1999/xhtml" {
                return false;
            }
            match element.local_name() {
                "input" => include_image_inputs || element.input_type() != "image",
                "button" | "fieldset" | "object" | "output" | "select" | "textarea" => true,
                _ => false,
            }
        })
}

pub(in crate::native_bridge) fn fieldset_elements_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_element_getter_receiver(
        scope,
        args.this(),
        "HTMLFieldSetElement",
        "elements",
        "fieldset",
    ) else {
        rv.set_null();
        return;
    };
    let descriptor = LiveCollectionDescriptor {
        collection_kind: CollectionKind::HtmlCollection,
        query_kind: LiveCollectionQueryKind::FormControls,
        root: handle,
        query: None,
        include_root: false,
        tag_name_html_document: None,
        resolution_cache: Default::default(),
    };
    let collection = collections::build_live_collection_wrapper(scope, runtime_ptr, descriptor);
    rv.set(collection.into());
}

pub(in crate::native_bridge) fn form_length_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_uint32(0);
        return;
    };
    let len = form_control_elements(unsafe { &*runtime_ptr }, handle).len() as u32;
    rv.set_uint32(len);
}

pub(in crate::native_bridge) fn form_name_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "name", rv);
}

pub(in crate::native_bridge) fn form_name_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_form_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "name",
        args.get(0),
        "HTMLFormElement",
        "name",
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_target_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, args.this(), "HTMLFormElement", "target", "form")
    else {
        rv.set_empty_string();
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "target").unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn form_owner_document_url(runtime: &JsContextHost, handle: DomHandle) -> String {
    runtime
        .dom_host()
        .owner_document_handle(handle)
        .or_else(|| runtime.dom_host().root_node_handle(handle))
        .and_then(|document_handle| runtime.dom_host().node(document_handle))
        .and_then(Node::as_document)
        .map(|document| document.url().to_string())
        .unwrap_or_else(|| runtime.host_document().url().to_string())
}

pub(in crate::native_bridge) fn form_target_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, args.this(), "HTMLFormElement", "target", "form")
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) =
        form_dom_string_property_value(scope, args.get(0), "HTMLFormElement", "target", false)
    else {
        rv.set_undefined();
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "target", &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_named_getter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if is_array_index_property_name(&key)
        || object_has_expando_named_property(scope, args.holder(), &key)
    {
        return v8::Intercepted::kNo;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let matches = form_named_item_matches(runtime, handle, &key);
    if matches.len() > 1 {
        let descriptor = LiveCollectionDescriptor {
            collection_kind: CollectionKind::RadioNodeList,
            query_kind: LiveCollectionQueryKind::FormControlsByName,
            root: handle,
            query: Some(key),
            include_root: false,
            tag_name_html_document: None,
            resolution_cache: Default::default(),
        };
        let list = collections::build_live_collection_wrapper(scope, runtime_ptr, descriptor);
        rv.set(list.into());
        return v8::Intercepted::kYes;
    }
    let Some(match_handle) = matches.first().copied() else {
        let Some(past_handle) = runtime.form_past_named_item(handle, &key) else {
            return v8::Intercepted::kNo;
        };
        let Some(node) = runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, past_handle)
        else {
            return v8::Intercepted::kNo;
        };
        rv.set(node.into());
        return v8::Intercepted::kYes;
    };
    runtime.remember_form_past_named_item(handle, key, match_handle);
    let Some(node) = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, match_handle)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(node.into());
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn form_named_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if is_array_index_property_name(&key)
        || object_has_expando_named_property(scope, args.holder(), &key)
    {
        return v8::Intercepted::kNo;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let matches = form_named_item_matches(runtime, handle, &key);
    let value = if matches.len() > 1 {
        let descriptor = LiveCollectionDescriptor {
            collection_kind: CollectionKind::RadioNodeList,
            query_kind: LiveCollectionQueryKind::FormControlsByName,
            root: handle,
            query: Some(key),
            include_root: false,
            tag_name_html_document: None,
            resolution_cache: Default::default(),
        };
        let list = collections::build_live_collection_wrapper(scope, runtime_ptr, descriptor);
        list.into()
    } else if let Some(match_handle) = matches.first().copied() {
        runtime.remember_form_past_named_item(handle, key, match_handle);
        let Some(node) = runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, match_handle)
        else {
            return v8::Intercepted::kNo;
        };
        node.into()
    } else {
        let Some(past_handle) = runtime.form_past_named_item(handle, &key) else {
            return v8::Intercepted::kNo;
        };
        let Some(node) = runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, past_handle)
        else {
            return v8::Intercepted::kNo;
        };
        node.into()
    };
    let Ok(descriptor) = DataPropertyDescriptorDeclaration::new(value, false, false).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn form_named_deleter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if is_array_index_property_name(&key)
        || object_has_expando_named_property(scope, args.holder(), &key)
    {
        return v8::Intercepted::kNo;
    }
    if !form_has_named_item_or_past_name(unsafe { &*runtime_ptr }, handle, &key) {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn form_named_definer(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    _desc: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if is_array_index_property_name(&key)
        || object_has_expando_named_property(scope, args.holder(), &key)
    {
        return v8::Intercepted::kNo;
    }
    if !form_has_named_item_or_past_name(unsafe { &*runtime_ptr }, handle, &key) {
        return v8::Intercepted::kNo;
    }
    throw_type_error(scope, "Cannot redefine an HTMLFormElement named property.");
    v8::Intercepted::kYes
}

fn object_has_expando_named_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> bool {
    if form_native_property_can_be_overridden(key) {
        return false;
    }
    let Some(names) = object.get_own_property_names(
        scope,
        v8::GetPropertyNamesArgs {
            mode: v8::KeyCollectionMode::OwnOnly,
            property_filter: v8::PropertyFilter::ALL_PROPERTIES | v8::PropertyFilter::SKIP_SYMBOLS,
            index_filter: v8::IndexFilter::IncludeIndices,
            key_conversion: v8::KeyConversionMode::KeepNumbers,
        },
    ) else {
        return false;
    };
    for index in 0..names.length() {
        let Some(name) = names
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
        else {
            continue;
        };
        if name == key {
            return true;
        }
    }
    false
}

fn form_native_property_can_be_overridden(key: &str) -> bool {
    matches!(
        key,
        "addEventListener"
            | "removeEventListener"
            | "dispatchEvent"
            | "nodeType"
            | "nodeName"
            | "ownerDocument"
            | "namespaceURI"
            | "prefix"
            | "localName"
            | "title"
            | "lang"
            | "dir"
            | "acceptCharset"
            | "action"
            | "autocomplete"
            | "enctype"
            | "encoding"
            | "method"
            | "name"
            | "noValidate"
            | "target"
            | "elements"
            | "length"
            | "submit"
            | "reset"
            | "requestSubmit"
            | "checkValidity"
            | "reportValidity"
    )
}

fn form_has_named_item_or_past_name(
    runtime: &JsContextHost,
    form_handle: DomHandle,
    key: &str,
) -> bool {
    !form_named_item_matches(runtime, form_handle, key).is_empty()
        || runtime.form_past_named_item(form_handle, key).is_some()
}

pub(in crate::native_bridge) fn form_indexed_getter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, form_handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(control_handle) = form_control_elements(runtime, form_handle)
        .get(index as usize)
        .copied()
    else {
        return v8::Intercepted::kNo;
    };
    let Some(control) = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, control_handle)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(control.into());
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn form_indexed_query(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, form_handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if form_control_elements(unsafe { &*runtime_ptr }, form_handle).len() <= index as usize {
        return v8::Intercepted::kNo;
    }
    // HTMLFormElement indexed supported properties report configurable
    // descriptors for Web compat, but legacy platform indexed properties are
    // read-only and reject deletion. V8 fast lookup/delete paths can consult
    // query attributes before descriptor/deleter callbacks, so keep the
    // internal attributes strict while `form_indexed_descriptor` exposes the
    // Web-facing descriptor shape (`configurable: true`, `writable: false`).
    rv.set_int32(
        (v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE).as_u32() as i32,
    );
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn form_indexed_setter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    _value: v8::Local<'_, v8::Value>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, form_handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if form_control_elements(unsafe { &*runtime_ptr }, form_handle).len() <= index as usize {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn form_indexed_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, form_handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(control_handle) = form_control_elements(runtime, form_handle)
        .get(index as usize)
        .copied()
    else {
        return v8::Intercepted::kNo;
    };
    let Some(control) = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, control_handle)
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) =
        DataPropertyDescriptorDeclaration::new(control.into(), false, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn form_indexed_deleter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, form_handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if form_control_elements(unsafe { &*runtime_ptr }, form_handle).len() <= index as usize {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn form_indexed_enumerator(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Ok((runtime_ptr, form_handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys = (0..form_control_elements(unsafe { &*runtime_ptr }, form_handle).len())
        .map(|index| v8::Integer::new_from_unsigned(scope, index as u32).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

pub(in crate::native_bridge) fn form_indexed_definer(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    _desc: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, form_handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if form_control_elements(unsafe { &*runtime_ptr }, form_handle).len() <= index as usize {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn form_named_query(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return v8::Intercepted::kNo;
    };
    let key = key.to_rust_string_lossy(scope);
    if is_array_index_property_name(&key)
        || object_has_expando_named_property(scope, args.holder(), &key)
    {
        return v8::Intercepted::kNo;
    }
    if !form_has_named_item_or_past_name(unsafe { &*runtime_ptr }, handle, &key) {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(
        (v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY).as_u32() as i32,
    );
    v8::Intercepted::kYes
}

fn form_named_item_matches(
    runtime: &JsContextHost,
    form_handle: DomHandle,
    key: &str,
) -> Vec<DomHandle> {
    let controls = form_control_elements(runtime, form_handle)
        .into_iter()
        .filter(|handle| {
            runtime
                .dom_host()
                .node(*handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.matches_named_item_key(key))
        })
        .collect::<Vec<_>>();
    if controls.is_empty() {
        form_named_image_matches(runtime, form_handle, key)
    } else {
        controls
    }
}

fn form_named_image_matches(
    runtime: &JsContextHost,
    form_handle: DomHandle,
    key: &str,
) -> Vec<DomHandle> {
    if !runtime
        .dom_host()
        .is_html_element_named(form_handle, "form")
    {
        return Vec::new();
    }
    runtime
        .dom_host()
        .elements_by_tag_name(form_handle, "img", false)
        .into_iter()
        .filter(|handle| nearest_form_ancestor(runtime, *handle) == Some(form_handle))
        .filter(|handle| {
            runtime
                .dom_host()
                .node(*handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.matches_named_item_key(key))
        })
        .collect()
}

fn nearest_form_ancestor(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    let mut current = runtime.dom_host().parent_node(handle);
    while let Some(parent) = current {
        if runtime.dom_host().is_html_element_named(parent, "form") {
            return Some(parent);
        }
        current = runtime.dom_host().parent_node(parent);
    }
    None
}

fn is_array_index_property_name(value: &str) -> bool {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return false;
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    value
        .parse::<u64>()
        .is_ok_and(|index| index < u64::from(u32::MAX))
}
