use super::*;

fn anchor_url_string_function_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    project: impl FnOnce(&url::Url) -> String,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        rv.set_empty_string();
        return;
    };
    let value = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href")
        .map(|url| project(&url))
        .unwrap_or_default();
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

fn anchor_href_getter_function<'s>(
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
    let value = resolve_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href");
    match v8_string(scope, &value) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

fn anchor_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    hyperlink_href_setter_function(scope, args, "HTMLAnchorElement", &mut rv);
}

fn area_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    hyperlink_href_setter_function(scope, args, "HTMLAreaElement", &mut rv);
}

fn base_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    hyperlink_href_setter_function(scope, args, "HTMLBaseElement", &mut rv);
}

fn link_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    hyperlink_href_setter_function(scope, args, "HTMLLinkElement", &mut rv);
}

fn hyperlink_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    owner: &'static str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), owner, "href") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "href", &value);
    rv.set_undefined();
}

pub(super) fn html_referrer_policy_getter_function<'s>(
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
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value =
        element_attribute(unsafe { &*runtime_ptr }, handle, "referrerpolicy").unwrap_or_default();
    let Some(value) = v8_string(scope, canonical_referrer_policy_value(&value)) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn html_cross_origin_getter_function<'s>(
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
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    match element_attribute(unsafe { &*runtime_ptr }, handle, "crossorigin") {
        Some(value) => {
            let Some(value) = v8_string(scope, canonical_cross_origin_value(&value)) else {
                rv.set_null();
                return;
            };
            rv.set(value.into());
        }
        None => rv.set_null(),
    }
}

fn set_html_cross_origin_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    if value.is_null() || value.is_undefined() {
        remove_reflected_attribute(scope, runtime_ptr, handle, "crossorigin");
        return;
    }
    let Some(value) = property_dom_string_value(scope, value, owner, "crossOrigin") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "crossorigin", &value);
}

fn html_cross_origin_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(descriptor) =
        CrossOriginReflection::descriptor_from_callback_data(scope, args.data())
    {
        set_html_cross_origin_for_receiver(scope, args.this(), args.get(0), descriptor.interface);
    }
    rv.set_undefined();
}

fn html_loading_getter_function<'s>(
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
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        rv.set_undefined();
        return;
    }
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "loading").unwrap_or_default();
    let Some(value) = v8_string(scope, canonical_loading_value(&value)) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn image_loading_getter_function<'s>(
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
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "loading")
        .unwrap_or_else(|| "eager".to_owned());
    let Some(value) = v8_string(scope, &value) else {
        rv.set_empty_string();
        return;
    };
    rv.set(value.into());
}

fn set_html_loading_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
) -> Option<(*mut JsContextHost, DomHandle, String)> {
    let value = property_dom_string_value(scope, value, owner, "loading")?;
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return None;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "loading", &value);
    Some((runtime_ptr, handle, value))
}

fn image_loading_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some((runtime_ptr, handle, value)) =
        set_html_loading_for_receiver(scope, args.this(), args.get(0), "HTMLImageElement")
        && !value.trim().eq_ignore_ascii_case("lazy")
    {
        queue_image_load_event_for_loading_change(scope, runtime_ptr, handle);
    }
    rv.set_undefined();
}

pub(super) fn anchor_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "type", rv);
}

pub(super) fn anchor_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "type",
        args.get(0),
        "HTMLAnchorElement",
        "type",
    );
    rv.set_undefined();
}

fn anchor_host_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| {
            url.host_str()
                .map(|host| {
                    url.port()
                        .map(|port| format!("{host}:{port}"))
                        .unwrap_or_else(|| host.to_owned())
                })
                .unwrap_or_default()
        },
        rv,
    );
}

fn hyperlink_url_setter_input<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    property: &'static str,
) -> Option<(*mut JsContextHost, DomHandle, url::Url, String)> {
    node_runtime_and_handle_from_object_or_detached(scope, args.this()).ok()?;
    let value =
        property_usv_string_value(scope, args.get(0), "HTMLHyperlinkElementUtils", property)?;
    // Conversion can change href, the document's base, or the node's owner.
    // Resolve the current node and URL only after those script side effects.
    let (runtime_ptr, handle) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this()).ok()?;
    let url = parsed_url_like_attribute(unsafe { &*runtime_ptr }, handle, "href")?;
    Some((runtime_ptr, handle, url, value))
}

fn anchor_host_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle, mut url, value)) =
        hyperlink_url_setter_input(scope, &args, "host")
    else {
        return;
    };
    if url.cannot_be_a_base() {
        return;
    }
    moli_url::components::set_host(&mut url, &value);
    // HTML updates href even when component parsing leaves the URL unchanged.
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_hostname_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| url.host_str().unwrap_or_default().to_owned(),
        rv,
    );
}

fn anchor_hostname_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle, mut url, value)) =
        hyperlink_url_setter_input(scope, &args, "hostname")
    else {
        return;
    };
    if url.cannot_be_a_base() {
        return;
    }
    moli_url::components::set_hostname(&mut url, &value);
    // HTML updates href even when component parsing leaves the URL unchanged.
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_port_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| url.port().map(|port| port.to_string()).unwrap_or_default(),
        rv,
    );
}

fn anchor_port_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle, mut url, value)) =
        hyperlink_url_setter_input(scope, &args, "port")
    else {
        return;
    };
    if !anchor_url_can_have_userinfo(&url) {
        return;
    }
    moli_url::components::set_port(&mut url, &value);
    // HTML updates href even when component parsing leaves the URL unchanged.
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_pathname_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(scope, args.this(), |url| url.path().to_owned(), rv);
}

fn anchor_pathname_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle, mut url, value)) =
        hyperlink_url_setter_input(scope, &args, "pathname")
    else {
        return;
    };
    if url.cannot_be_a_base() {
        return;
    }
    moli_url::components::set_pathname(&mut url, &value);
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_search_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| url::quirks::search(url).to_owned(),
        rv,
    );
}

fn anchor_search_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle, mut url, value)) =
        hyperlink_url_setter_input(scope, &args, "search")
    else {
        return;
    };
    url::quirks::set_search(&mut url, &value);
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_hash_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| url::quirks::hash(url).to_owned(),
        rv,
    );
}

fn anchor_hash_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle, mut url, value)) =
        hyperlink_url_setter_input(scope, &args, "hash")
    else {
        return;
    };
    url::quirks::set_hash(&mut url, &value);
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_origin_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(scope, args.this(), moli_url::origin_ascii_serialization, rv);
}

fn anchor_protocol_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(scope, args.this(), |url| format!("{}:", url.scheme()), rv);
}

fn anchor_protocol_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle, mut url, value)) =
        hyperlink_url_setter_input(scope, &args, "protocol")
    else {
        return;
    };
    let _ = url::quirks::set_protocol(&mut url, &value);
    // HTML updates href even when component parsing leaves the URL unchanged.
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_username_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(scope, args.this(), |url| url.username().to_owned(), rv);
}

fn anchor_username_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle, mut url, value)) =
        hyperlink_url_setter_input(scope, &args, "username")
    else {
        return;
    };
    if !anchor_url_can_have_userinfo(&url) {
        return;
    }
    let _ = url::quirks::set_username(&mut url, &value);
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_password_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    anchor_url_string_function_getter(
        scope,
        args.this(),
        |url| url.password().unwrap_or("").to_owned(),
        rv,
    );
}

fn anchor_password_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle, mut url, value)) =
        hyperlink_url_setter_input(scope, &args, "password")
    else {
        return;
    };
    if !anchor_url_can_have_userinfo(&url) {
        return;
    }
    let _ = url::quirks::set_password(&mut url, &value);
    set_resolved_url_attribute(scope, runtime_ptr, handle, "href", &url);
    rv.set_undefined();
}

fn anchor_url_can_have_userinfo(url: &url::Url) -> bool {
    url.scheme() != "file" && url.host_str().is_some_and(|host| !host.is_empty())
}

fn reflected_url_attribute_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    name: &str,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        if let Some(value) = v8_string(scope, "") {
            rv.set(value.into());
        } else {
            rv.set_null();
        }
        return;
    };
    let value = resolve_url_like_attribute(unsafe { &*runtime_ptr }, handle, name);
    match v8_string(scope, &value) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

fn script_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    reflected_url_attribute_getter_function(scope, args.this(), "src", rv);
}

fn script_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = trusted_script_url_sink_string(scope, runtime_ptr, args.get(0)) else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
    rv.set_undefined();
}

fn script_dom_string_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    name: &str,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), name, rv);
}

fn script_dom_string_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    name: &str,
    property: &'static str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        name,
        args.get(0),
        "HTMLScriptElement",
        property,
    );
    rv.set_undefined();
}

fn script_type_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_dom_string_getter_function(scope, args, "type", rv);
}

fn script_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_dom_string_setter_function(scope, args, "type", "type", &mut rv);
}

fn svg_script_type_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "type",
        args.get(0),
        "SVGScriptElement",
        "type",
    );
    rv.set_undefined();
}

fn svg_fetch_priority_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    interface: &'static str,
    local_name: &'static str,
    setter: bool,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        if setter {
            throw_incompatible_setter_receiver(scope, interface, "fetchPriority");
        } else {
            throw_incompatible_getter_receiver(scope, interface, "fetchPriority");
        }
        return None;
    };
    let is_expected_element = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(|element| element.is_svg_element(local_name));
    if !is_expected_element {
        if setter {
            throw_incompatible_setter_receiver(scope, interface, "fetchPriority");
        } else {
            throw_incompatible_getter_receiver(scope, interface, "fetchPriority");
        }
        return None;
    }
    Some((runtime_ptr, handle))
}

fn svg_fetch_priority_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    interface: &'static str,
    local_name: &'static str,
) {
    let Some((runtime_ptr, handle)) =
        svg_fetch_priority_receiver(scope, args.this(), interface, local_name, false)
    else {
        return;
    };
    let raw = unsafe { &*runtime_ptr }
        .dom_host()
        .get_attribute(handle, "fetchpriority")
        .unwrap_or_default();
    let Some(value) = v8_string(scope, canonical_fetch_priority_value(&raw)) else {
        rv.set_empty_string();
        return;
    };
    rv.set(value.into());
}

fn svg_fetch_priority_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    interface: &'static str,
    local_name: &'static str,
) {
    let Some((runtime_ptr, handle)) =
        svg_fetch_priority_receiver(scope, args.this(), interface, local_name, true)
    else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, args.get(0), interface, "fetchPriority")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "fetchpriority", &value);
    rv.set_undefined();
}

fn svg_image_fetch_priority_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    svg_fetch_priority_getter_function(scope, args, rv, "SVGImageElement", "image");
}

fn svg_image_fetch_priority_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_fetch_priority_setter_function(scope, args, rv, "SVGImageElement", "image");
}

fn svg_script_fetch_priority_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    svg_fetch_priority_getter_function(scope, args, rv, "SVGScriptElement", "script");
}

fn svg_script_fetch_priority_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    svg_fetch_priority_setter_function(scope, args, rv, "SVGScriptElement", "script");
}

pub(super) fn node_nonce_getter_function<'s>(
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
    let value = runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .and_then(crate::dom::native::Element::cryptographic_nonce)
        .map(str::to_owned)
        .or_else(|| runtime.dom_host().get_attribute(handle, "nonce"))
        .unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(super) fn node_nonce_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = property_dom_string_value(scope, args.get(0), "Element", "nonce") else {
        return;
    };
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_cryptographic_nonce(handle, Some(value));
    rv.set_undefined();
}

fn script_async_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_bool(false);
        return;
    };
    let value = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(crate::dom::native::Element::script_async);
    rv.set_bool(value);
}

fn script_async_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let value = args.get(0).boolean_value(scope);
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_script_async(scope, runtime_ptr, handle, value);
    rv.set_undefined();
}

fn script_text_getter_function<'s>(
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
    let value = node_direct_text_content(unsafe { &*runtime_ptr }, handle).unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

fn script_source_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    sink: TrustedScriptElementSink,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = html_element_setter_receiver(
        scope,
        args.this(),
        "HTMLScriptElement",
        sink.api_name(),
        "script",
    ) else {
        return;
    };
    let Some(text) = trusted_script_element_sink_string(scope, runtime_ptr, args.get(0), sink)
    else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }
        .dom_host_mut()
        .set_script_text_internal_slot(handle, &text);
    if sink == TrustedScriptElementSink::InnerText {
        let _ = set_inner_text_in_reaction_scope(scope, runtime_ptr, handle, &text);
    } else {
        let _ = set_text_content_in_reaction_scope(scope, runtime_ptr, handle, &text);
    }
    rv.set_undefined();
}

fn script_text_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_source_setter_function(scope, args, TrustedScriptElementSink::Text, rv);
}

fn script_inner_text_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if html_element_getter_receiver(
        scope,
        args.this(),
        "HTMLScriptElement",
        "innerText",
        "script",
    )
    .is_none()
    {
        return;
    }
    node_inner_text_getter_function(scope, args, rv);
}

fn script_inner_text_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_source_setter_function(scope, args, TrustedScriptElementSink::InnerText, rv);
}

fn script_text_content_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if html_element_getter_receiver(
        scope,
        args.this(),
        "HTMLScriptElement",
        "textContent",
        "script",
    )
    .is_none()
    {
        return;
    }
    node_text_content_getter_function(scope, args, rv);
}

fn script_text_content_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_source_setter_function(scope, args, TrustedScriptElementSink::TextContent, rv);
}

fn script_boolean_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    name: &str,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    boolean_attribute_property_getter_from_object_or_detached(scope, args.this(), name, rv);
}

fn script_boolean_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    name: &str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        name,
        args.get(0).boolean_value(scope),
    );
    rv.set_undefined();
}

fn script_defer_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_boolean_getter_function(scope, args, "defer", rv);
}

fn script_defer_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_boolean_setter_function(scope, args, "defer", &mut rv);
}

fn script_no_module_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_boolean_getter_function(scope, args, "nomodule", rv);
}

fn script_no_module_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_boolean_setter_function(scope, args, "nomodule", &mut rv);
}

fn script_integrity_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_dom_string_getter_function(scope, args, "integrity", rv);
}

fn script_integrity_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_dom_string_setter_function(scope, args, "integrity", "integrity", &mut rv);
}

fn script_event_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_dom_string_getter_function(scope, args, "event", rv);
}

fn script_event_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_dom_string_setter_function(scope, args, "event", "event", &mut rv);
}

fn script_html_for_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    script_dom_string_getter_function(scope, args, "for", rv);
}

fn script_html_for_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    script_dom_string_setter_function(scope, args, "for", "htmlFor", &mut rv);
}

fn image_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    reflected_url_attribute_getter_function(scope, args.this(), "src", rv);
}

fn image_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), "HTMLImageElement", "src")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
    rv.set_undefined();
}

fn generic_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    reflected_url_attribute_getter_function(scope, args.this(), "src", rv);
}

fn source_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    generic_src_setter_function(scope, args, "HTMLSourceElement", &mut rv);
}

fn embed_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    generic_src_setter_function(scope, args, "HTMLEmbedElement", &mut rv);
}

fn frame_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    generic_src_setter_function(scope, args, "HTMLFrameElement", &mut rv);
}

fn generic_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    owner: &'static str,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), owner, "src") else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
    rv.set_undefined();
}

fn image_srcset_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "srcset", rv);
}

fn image_srcset_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "srcset",
        args.get(0),
        "HTMLImageElement",
        "srcset",
    );
    rv.set_undefined();
}

fn source_srcset_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    attribute_property_getter_from_object_or_detached(scope, args.this(), "srcset", rv);
}

fn source_srcset_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_dom_string_attribute_property_on_object(
        scope,
        args.this(),
        "srcset",
        args.get(0),
        "HTMLSourceElement",
        "srcset",
    );
    rv.set_undefined();
}

fn iframe_src_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    reflected_url_attribute_getter_function(scope, args.this(), "src", rv);
}

fn iframe_src_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    let Some(value) = property_usv_string_value(scope, args.get(0), "HTMLIFrameElement", "src")
    else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if iframe_uses_detached_content_cache(runtime, handle)
        || !runtime.dom_host().is_connected(handle)
    {
        set_reflected_attribute(scope, runtime_ptr, handle, "src", &value);
        clear_detached_iframe_cached_context(scope, receiver);
    } else {
        update_iframe_snapshot_navigation(scope, runtime_ptr, handle, &value);
    }
    rv.set_undefined();
}

fn iframe_srcdoc_getter_function<'s>(
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
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, "srcdoc").unwrap_or_default();
    match v8_string(scope, &value) {
        Some(value) => rv.set(value.into()),
        None => rv.set_null(),
    }
}

fn iframe_srcdoc_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, args.get(0), "HTMLIFrameElement", "srcdoc")
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, "srcdoc", &value);
    let runtime = unsafe { &*runtime_ptr };
    if iframe_uses_detached_content_cache(runtime, handle)
        || !runtime.dom_host().is_connected(handle)
    {
        clear_detached_iframe_cached_context(scope, receiver);
    }
    rv.set_undefined();
}

fn frame_owner_content_document_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    match frame_owner_content_document(scope, args.this()) {
        Some(document) => rv.set(document.into()),
        None => rv.set_null(),
    }
}

pub(super) fn frame_owner_get_svg_document_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let document = frame_owner_content_document(scope, args.this()).filter(|document| {
        node_runtime_and_handle_from_object_or_detached(scope, *document).is_ok_and(
            |(runtime_ptr, handle)| {
                unsafe { &*runtime_ptr }
                    .dom_host()
                    .document_content_type_for_handle(handle)
                    == Some("image/svg+xml")
            },
        )
    });
    match document {
        Some(document) => rv.set(document.into()),
        None => rv.set_null(),
    }
}

fn frame_owner_content_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let (runtime_ptr, handle) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver).ok()?;
    if iframe_is_inside_its_own_child_context_document(scope, runtime_ptr, handle)
        || iframe_has_inactive_child_context(unsafe { &*runtime_ptr }, handle)
        || iframe_is_in_own_child_document(unsafe { &*runtime_ptr }, handle)
    {
        return None;
    }
    let runtime = unsafe { &*runtime_ptr };
    if iframe_uses_detached_content_cache(runtime, handle)
        || !runtime.dom_host().is_connected(handle)
    {
        return if iframe_uses_detached_content_cache(runtime, handle)
            || disconnected_iframe_can_materialize_detached_content(runtime, handle)
        {
            detached_iframe_content_document(scope, receiver)
        } else {
            None
        };
    }
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.refresh_child_browsing_context(scope, handle);
    if !runtime.child_browsing_context_is_same_origin_with_top(handle) {
        return None;
    }
    if let Some(window) = runtime.child_browsing_context_window_wrapper(scope, handle) {
        runtime.set_cached_detached_iframe_content_window(scope, handle, window);
        if let Some(document) = window
            .get(scope, v8str(scope, "document").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            runtime.set_cached_detached_iframe_content_document(scope, handle, document);
            return Some(document);
        }
    }
    let document = runtime.child_browsing_context_document_wrapper(scope, handle)?;
    runtime.set_cached_detached_iframe_content_document(scope, handle, document);
    Some(document)
}

fn frame_owner_content_window_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return;
    };
    if iframe_is_inside_its_own_child_context_document(scope, runtime_ptr, handle) {
        rv.set_null();
        return;
    }
    if iframe_has_inactive_child_context(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    if iframe_is_in_own_child_document(unsafe { &*runtime_ptr }, handle) {
        rv.set_null();
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    if iframe_uses_detached_content_cache(runtime, handle)
        || !runtime.dom_host().is_connected(handle)
    {
        if iframe_uses_detached_content_cache(runtime, handle)
            || disconnected_iframe_can_materialize_detached_content(runtime, handle)
        {
            match detached_iframe_content_window(scope, receiver) {
                Some(window) => rv.set(window.into()),
                None => rv.set_null(),
            }
        } else {
            rv.set_null();
        }
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    runtime.refresh_child_browsing_context(scope, handle);
    let exposes_same_origin_wrapper =
        runtime.child_browsing_context_is_same_origin_with_top(handle);
    let window = runtime.child_browsing_context_window_proxy_for_top(scope, handle);
    if window.is_some() {
        runtime.mark_child_browsing_context_window_wrapper_exposed_to_top(handle);
    }
    if exposes_same_origin_wrapper && window.is_some() {
        runtime.request_child_frame_realm_materialization(handle);
    }
    match window {
        Some(window) => {
            if runtime.child_browsing_context_is_same_origin_with_top(handle) {
                runtime.set_cached_detached_iframe_content_window(scope, handle, window);
            }
            rv.set(window.into());
        }
        None => rv.set_null(),
    }
}

pub(super) fn object_content_document_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if html_element_getter_receiver(
        scope,
        args.this(),
        "HTMLObjectElement",
        "contentDocument",
        "object",
    )
    .is_none()
    {
        return;
    }
    frame_owner_content_document_getter_function(scope, args, rv);
}

pub(super) fn object_content_window_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if html_element_getter_receiver(
        scope,
        args.this(),
        "HTMLObjectElement",
        "contentWindow",
        "object",
    )
    .is_none()
    {
        return;
    }
    frame_owner_content_window_getter_function(scope, args, rv);
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLScriptElement, enumerable)]
pub(super) struct HtmlScriptElementPrototypeDeclaration {
    #[webapi(
        accessor_property = "innerText",
        getter = script_inner_text_getter_function,
        setter = script_inner_text_setter_function
    )]
    inner_text: (),
    #[webapi(
        accessor_property = "textContent",
        getter = script_text_content_getter_function,
        setter = script_text_content_setter_function
    )]
    text_content: (),
    #[webapi(
        accessor_property = "crossOrigin",
        getter = html_cross_origin_getter_function,
        setter = html_cross_origin_setter_function,
        setter_data = CrossOriginReflection::Script
    )]
    cross_origin: (),
    #[webapi(
        accessor_property,
        getter = script_src_getter_function,
        setter = script_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property = "fetchPriority",
        getter = html_fetch_priority_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::ScriptFetchPriority
    )]
    fetch_priority: (),
    #[webapi(
        accessor_property,
        getter = html_charset_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ScriptCharset
    )]
    charset: (),
    #[webapi(
        accessor_property = "type",
        getter = script_type_getter_function,
        setter = script_type_setter_function
    )]
    r#type: (),
    #[webapi(
        accessor_property = "async",
        getter = script_async_getter_function,
        setter = script_async_setter_function
    )]
    r#async: (),
    #[webapi(
        accessor_property,
        getter = script_text_getter_function,
        setter = script_text_setter_function
    )]
    text: (),
    #[webapi(
        accessor_property,
        getter = script_defer_getter_function,
        setter = script_defer_setter_function
    )]
    defer: (),
    #[webapi(
        accessor_property,
        getter = script_no_module_getter_function,
        setter = script_no_module_setter_function
    )]
    no_module: (),
    #[webapi(
        accessor_property,
        getter = script_integrity_getter_function,
        setter = script_integrity_setter_function
    )]
    integrity: (),
    #[webapi(
        accessor_property,
        getter = script_event_getter_function,
        setter = script_event_setter_function
    )]
    event: (),
    #[webapi(
        accessor_property,
        getter = script_html_for_getter_function,
        setter = script_html_for_setter_function
    )]
    html_for: (),
    #[webapi(
        accessor_property,
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ScriptReferrerPolicy
    )]
    referrer_policy: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGScriptElement, enumerable)]
pub(super) struct SvgScriptElementPrototypeDeclaration {
    #[webapi(
        accessor_property = "fetchPriority",
        getter = svg_script_fetch_priority_getter_function,
        setter = svg_script_fetch_priority_setter_function
    )]
    fetch_priority: (),
    #[webapi(
        accessor_property = "type",
        getter = script_type_getter_function,
        setter = svg_script_type_setter_function
    )]
    r#type: (),
    #[webapi(
        accessor_property = "async",
        getter = script_async_getter_function,
        setter = script_async_setter_function
    )]
    r#async: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGImageElement, enumerable)]
pub(super) struct SvgImageElementPrototypeDeclaration {
    #[webapi(
        accessor_property = "fetchPriority",
        getter = svg_image_fetch_priority_getter_function,
        setter = svg_image_fetch_priority_setter_function
    )]
    fetch_priority: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLAnchorElement, enumerable)]
pub(super) struct HtmlAnchorElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = anchor_href_getter_function,
        setter = anchor_href_setter_function
    )]
    href: (),
    #[webapi(
        accessor_property,
        getter = anchor_host_getter_function,
        setter = anchor_host_setter_function
    )]
    host: (),
    #[webapi(
        accessor_property,
        getter = anchor_hostname_getter_function,
        setter = anchor_hostname_setter_function
    )]
    hostname: (),
    #[webapi(
        accessor_property,
        getter = anchor_port_getter_function,
        setter = anchor_port_setter_function
    )]
    port: (),
    #[webapi(
        accessor_property,
        getter = anchor_pathname_getter_function,
        setter = anchor_pathname_setter_function
    )]
    pathname: (),
    #[webapi(
        accessor_property,
        getter = anchor_search_getter_function,
        setter = anchor_search_setter_function
    )]
    search: (),
    #[webapi(
        accessor_property,
        getter = anchor_hash_getter_function,
        setter = anchor_hash_setter_function
    )]
    hash: (),
    #[webapi(accessor_property, getter = anchor_origin_getter_function)]
    origin: (),
    #[webapi(
        accessor_property,
        getter = anchor_protocol_getter_function,
        setter = anchor_protocol_setter_function
    )]
    protocol: (),
    #[webapi(
        accessor_property,
        getter = anchor_username_getter_function,
        setter = anchor_username_setter_function
    )]
    username: (),
    #[webapi(
        accessor_property,
        getter = anchor_password_getter_function,
        setter = anchor_password_setter_function
    )]
    password: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLAreaElement, enumerable)]
pub(super) struct HtmlAreaElementUrlPrototypeDeclaration {
    #[webapi(method = "toString", length = 0, callback = area_to_string_callback)]
    to_string: (),
    #[webapi(
        accessor_property,
        getter = anchor_href_getter_function,
        setter = area_href_setter_function
    )]
    href: (),
    #[webapi(
        accessor_property,
        getter = anchor_host_getter_function,
        setter = anchor_host_setter_function
    )]
    host: (),
    #[webapi(
        accessor_property,
        getter = anchor_hostname_getter_function,
        setter = anchor_hostname_setter_function
    )]
    hostname: (),
    #[webapi(
        accessor_property,
        getter = anchor_port_getter_function,
        setter = anchor_port_setter_function
    )]
    port: (),
    #[webapi(
        accessor_property,
        getter = anchor_pathname_getter_function,
        setter = anchor_pathname_setter_function
    )]
    pathname: (),
    #[webapi(
        accessor_property,
        getter = anchor_search_getter_function,
        setter = anchor_search_setter_function
    )]
    search: (),
    #[webapi(
        accessor_property,
        getter = anchor_hash_getter_function,
        setter = anchor_hash_setter_function
    )]
    hash: (),
    #[webapi(accessor_property, getter = anchor_origin_getter_function)]
    origin: (),
    #[webapi(
        accessor_property,
        getter = anchor_protocol_getter_function,
        setter = anchor_protocol_setter_function
    )]
    protocol: (),
    #[webapi(
        accessor_property,
        getter = anchor_username_getter_function,
        setter = anchor_username_setter_function
    )]
    username: (),
    #[webapi(
        accessor_property,
        getter = anchor_password_getter_function,
        setter = anchor_password_setter_function
    )]
    password: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLImageElement, enumerable)]
pub(super) struct HtmlImageElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = image_width_getter_function,
        setter = image_width_setter_function
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = image_height_getter_function,
        setter = image_height_setter_function
    )]
    height: (),
    #[webapi(accessor_property = "naturalWidth", getter = image_natural_width_getter_function)]
    natural_width: (),
    #[webapi(accessor_property = "naturalHeight", getter = image_natural_height_getter_function)]
    natural_height: (),
    #[webapi(accessor_property, getter = image_x_getter_function)]
    x: (),
    #[webapi(accessor_property, getter = image_y_getter_function)]
    y: (),
    #[webapi(
        accessor_property = "isMap",
        getter = image_is_map_getter_function,
        setter = image_is_map_setter_function
    )]
    is_map: (),
    #[webapi(accessor_property, getter = image_complete_getter_function)]
    complete: (),
    #[webapi(accessor_property = "currentSrc", getter = image_current_src_getter_function)]
    current_src: (),
    #[webapi(
        accessor_property,
        getter = html_sizes_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ImageSizes
    )]
    sizes: (),
    #[webapi(
        accessor_property = "crossOrigin",
        getter = html_cross_origin_getter_function,
        setter = html_cross_origin_setter_function,
        setter_data = CrossOriginReflection::Image
    )]
    cross_origin: (),
    #[webapi(
        accessor_property,
        getter = html_alt_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ImageAlt
    )]
    alt: (),
    #[webapi(
        accessor_property,
        getter = image_src_getter_function,
        setter = image_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = image_srcset_getter_function,
        setter = image_srcset_setter_function
    )]
    srcset: (),
    #[webapi(
        accessor_property = "fetchPriority",
        getter = html_fetch_priority_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::ImageFetchPriority
    )]
    fetch_priority: (),
    #[webapi(
        accessor_property = "useMap",
        getter = html_use_map_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ImageUseMap
    )]
    use_map: (),
    #[webapi(
        accessor_property,
        getter = image_loading_getter_function,
        setter = image_loading_setter_function
    )]
    loading: (),
    #[webapi(
        accessor_property = "referrerPolicy",
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::ImageReferrerPolicy
    )]
    referrer_policy: (),
    #[webapi(
        accessor_property = "longDesc",
        getter = html_long_desc_getter_function,
        setter = image_long_desc_setter_function
    )]
    long_desc: (),
    #[webapi(
        accessor_property,
        getter = html_lowsrc_getter_function,
        setter = image_lowsrc_setter_function
    )]
    lowsrc: (),
    #[webapi(
        accessor_property,
        getter = html_decoding_getter_function,
        setter = image_decoding_setter_function
    )]
    decoding: (),
    #[webapi(
        accessor_property,
        getter = html_border_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::ImageBorder
    )]
    border: (),
    #[webapi(
        accessor_property,
        getter = html_hspace_getter_function,
        setter = unsigned_long_reflection_setter_function,
        setter_data = UnsignedLongReflection::ImageHspace
    )]
    hspace: (),
    #[webapi(
        accessor_property,
        getter = html_vspace_getter_function,
        setter = unsigned_long_reflection_setter_function,
        setter_data = UnsignedLongReflection::ImageVspace
    )]
    vspace: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLSourceElement, enumerable)]
pub(super) struct HtmlSourceElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = generic_src_getter_function,
        setter = source_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = html_sizes_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::SourceSizes
    )]
    sizes: (),
    #[webapi(
        accessor_property,
        getter = source_width_getter_function,
        setter = source_width_setter_function
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = source_height_getter_function,
        setter = source_height_setter_function
    )]
    height: (),
    #[webapi(
        accessor_property,
        getter = source_srcset_getter_function,
        setter = source_srcset_setter_function
    )]
    srcset: (),
    #[webapi(
        accessor_property,
        getter = html_media_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::SourceMedia
    )]
    media: (),
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::SourceType
    )]
    r#type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLEmbedElement, enumerable)]
pub(super) struct HtmlEmbedElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = generic_src_getter_function,
        setter = embed_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::EmbedWidth
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = html_height_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::EmbedHeight
    )]
    height: (),
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::EmbedType
    )]
    r#type: (),
    #[webapi(
        method = "getSVGDocument",
        length = 0,
        callback = frame_owner_get_svg_document_function,
        receiver = web_api_interfaces::HTMLEmbedElement::is_instance
    )]
    get_svg_document: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLFrameElement, enumerable)]
pub(super) struct HtmlFrameElementLegacyPrototypeDeclaration {
    #[webapi(
        accessor_property = "noResize",
        getter = html_no_resize_getter_function,
        setter = html_no_resize_setter_function
    )]
    no_resize: (),
    #[webapi(
        accessor_property,
        getter = generic_src_getter_function,
        setter = frame_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = html_scrolling_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::FrameScrolling
    )]
    scrolling: (),
    #[webapi(
        accessor_property = "frameBorder",
        getter = html_frame_border_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::FrameFrameBorder
    )]
    frame_border: (),
    #[webapi(
        accessor_property = "longDesc",
        getter = html_long_desc_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::FrameLongDesc
    )]
    long_desc: (),
    #[webapi(
        accessor_property = "marginHeight",
        getter = html_margin_height_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::FrameMarginHeight
    )]
    margin_height: (),
    #[webapi(
        accessor_property = "marginWidth",
        getter = html_margin_width_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::FrameMarginWidth
    )]
    margin_width: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLIFrameElement, enumerable)]
pub(super) struct HtmlIFrameElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = iframe_src_getter_function,
        setter = iframe_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = iframe_srcdoc_getter_function,
        setter = iframe_srcdoc_setter_function
    )]
    srcdoc: (),
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::IframeAllow
    )]
    allow: (),
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::IframeCsp
    )]
    csp: (),
    #[webapi(
        accessor_property,
        getter = html_loading_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeLoading
    )]
    loading: (),
    #[webapi(
        accessor_property,
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeReferrerPolicy
    )]
    referrer_policy: (),
    #[webapi(
        accessor_property,
        getter = node_sandbox_getter_function,
        setter = node_sandbox_setter_function
    )]
    sandbox: (),
    #[webapi(
        accessor_property = "allowFullscreen",
        getter = node_allow_fullscreen_getter_function,
        setter = node_allow_fullscreen_setter_function
    )]
    allow_fullscreen: (),
    #[webapi(
        accessor_property,
        getter = node_credentialless_getter_function,
        setter = node_credentialless_setter_function
    )]
    credentialless: (),
    #[webapi(
        accessor_property,
        getter = html_scrolling_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeScrolling
    )]
    scrolling: (),
    #[webapi(
        accessor_property,
        getter = html_width_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeWidth
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = html_height_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeHeight
    )]
    height: (),
    #[webapi(
        accessor_property = "frameBorder",
        getter = html_frame_border_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::IframeFrameBorder
    )]
    frame_border: (),
    #[webapi(
        accessor_property = "longDesc",
        getter = html_long_desc_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::IframeLongDesc
    )]
    long_desc: (),
    #[webapi(
        accessor_property = "marginHeight",
        getter = html_margin_height_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::IframeMarginHeight
    )]
    margin_height: (),
    #[webapi(
        accessor_property = "marginWidth",
        getter = html_margin_width_getter_function,
        setter = null_to_empty_dom_string_reflection_setter_function,
        setter_data = NullToEmptyDomStringReflection::IframeMarginWidth
    )]
    margin_width: (),
    #[webapi(
        accessor_property,
        getter = frame_owner_content_document_getter_function,
        receiver = web_api_interfaces::HTMLIFrameElement::is_instance
    )]
    content_document: (),
    #[webapi(
        accessor_property,
        getter = frame_owner_content_window_getter_function,
        receiver = web_api_interfaces::HTMLIFrameElement::is_instance
    )]
    content_window: (),
    #[webapi(
        method = "getSVGDocument",
        length = 0,
        callback = frame_owner_get_svg_document_function,
        receiver = web_api_interfaces::HTMLIFrameElement::is_instance
    )]
    get_svg_document: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLBaseElement, enumerable)]
pub(super) struct HtmlBaseElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = anchor_href_getter_function,
        setter = base_href_setter_function
    )]
    href: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLLinkElement, enumerable)]
pub(super) struct HtmlLinkElementUrlPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = link_disabled_getter_function,
        setter = link_disabled_setter_function
    )]
    disabled: (),
    #[webapi(
        accessor_property = "crossOrigin",
        getter = html_cross_origin_getter_function,
        setter = html_cross_origin_setter_function,
        setter_data = CrossOriginReflection::Link
    )]
    cross_origin: (),
    #[webapi(
        accessor_property,
        getter = anchor_href_getter_function,
        setter = link_href_setter_function
    )]
    href: (),
    #[webapi(
        accessor_property = "fetchPriority",
        getter = html_fetch_priority_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::LinkFetchPriority
    )]
    fetch_priority: (),
    #[webapi(
        accessor_property = "referrerPolicy",
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkReferrerPolicy
    )]
    referrer_policy: (),
    #[webapi(
        accessor_property,
        getter = html_as_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkAs
    )]
    r#as: (),
    #[webapi(
        accessor_property,
        getter = html_hreflang_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkHreflang
    )]
    hreflang: (),
    #[webapi(
        accessor_property,
        getter = html_charset_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkCharset
    )]
    charset: (),
    #[webapi(
        accessor_property,
        getter = html_media_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::LinkMedia
    )]
    media: (),
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::LinkIntegrity
    )]
    integrity: (),
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::LinkRev
    )]
    rev: (),
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::LinkType
    )]
    r#type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLStyleElement, enumerable, receiver)]
pub(super) struct HtmlStyleElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::StyleType
    )]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = html_media_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::StyleMedia
    )]
    media: (),
    #[webapi(
        accessor_property,
        getter = style_blocking_getter_function,
        setter = style_blocking_setter_function
    )]
    blocking: (),
    #[webapi(
        accessor_property,
        getter = style_disabled_getter_function,
        setter = style_disabled_setter_function
    )]
    disabled: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGStyleElement, enumerable)]
pub(super) struct SvgStyleElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = svg_style_dom_string_getter_function,
        setter = svg_style_dom_string_setter_function,
        data = callback_data_index_value(scope, 0)
    )]
    media: (),
    #[webapi(
        accessor_property,
        getter = svg_style_dom_string_getter_function,
        setter = svg_style_dom_string_setter_function,
        data = callback_data_index_value(scope, 1)
    )]
    title: (),
    #[webapi(
        accessor_property,
        getter = svg_style_dom_string_getter_function,
        setter = svg_style_dom_string_setter_function,
        data = callback_data_index_value(scope, 2)
    )]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = svg_style_disabled_getter_function,
        setter = svg_style_disabled_setter_function
    )]
    disabled: (),
}

const SVG_STYLE_DOM_STRING_ATTRIBUTES: &[&str] = &["media", "title", "type"];

fn is_svg_style_element(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::as_element)
        .is_some_and(|element| {
            element.namespace() == "http://www.w3.org/2000/svg" && element.local_name() == "style"
        })
}

fn svg_style_element_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_getter_receiver(scope, "SVGStyleElement", member);
        return None;
    };
    if !is_svg_style_element(unsafe { &*runtime_ptr }, handle) {
        throw_incompatible_getter_receiver(scope, "SVGStyleElement", member);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn svg_style_element_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_setter_receiver(scope, "SVGStyleElement", member);
        return None;
    };
    if !is_svg_style_element(unsafe { &*runtime_ptr }, handle) {
        throw_incompatible_setter_receiver(scope, "SVGStyleElement", member);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn svg_style_dom_string_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        SVG_STYLE_DOM_STRING_ATTRIBUTES,
        "SVGStyleElement DOMString attributes",
    ) else {
        return;
    };
    let Some((runtime_ptr, handle)) =
        svg_style_element_getter_receiver(scope, args.this(), attribute)
    else {
        return;
    };
    let value = element_attribute(unsafe { &*runtime_ptr }, handle, attribute).unwrap_or_default();
    set_element_string_return_value(scope, &mut rv, &value);
}

fn svg_style_dom_string_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        SVG_STYLE_DOM_STRING_ATTRIBUTES,
        "SVGStyleElement DOMString attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    let Some((runtime_ptr, handle)) =
        svg_style_element_setter_receiver(scope, args.this(), attribute)
    else {
        return;
    };
    let Some(value) = property_dom_string_value(scope, args.get(0), "SVGStyleElement", attribute)
    else {
        return;
    };
    set_reflected_attribute(scope, runtime_ptr, handle, attribute, &value);
    rv.set_undefined();
}

fn svg_style_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if svg_style_element_getter_receiver(scope, args.this(), "disabled").is_none() {
        return;
    }
    style_disabled_getter_function(scope, args, rv);
}

fn svg_style_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if svg_style_element_setter_receiver(scope, args.this(), "disabled").is_none() {
        return;
    }
    style_disabled_setter_function(scope, args, rv);
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLDetailsElement, enumerable)]
pub(super) struct HtmlDetailsElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = details_open_getter_function,
        setter = details_open_setter_function
    )]
    open: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLDialogElement, enumerable)]
pub(super) struct HtmlDialogElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = dialog_open_getter_function,
        setter = dialog_open_setter_function
    )]
    open: (),
    #[webapi(
        accessor_property,
        getter = dialog_return_value_getter_function,
        setter = dialog_return_value_setter_function
    )]
    return_value: (),
    #[webapi(method, length = 0, callback = dialog_show_callback)]
    show: (),
    #[webapi(method, length = 0, callback = dialog_show_modal_callback)]
    show_modal: (),
    #[webapi(method, length = 1, callback = dialog_close_callback)]
    close: (),
    #[webapi(method, length = 1, callback = dialog_request_close_callback)]
    request_close: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLMetaElement, enumerable)]
pub(super) struct HtmlMetaElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = meta_content_getter_function,
        setter = meta_content_setter_function
    )]
    content: (),
    #[webapi(
        accessor_property = "httpEquiv",
        getter = meta_http_equiv_getter_function,
        setter = meta_http_equiv_setter_function
    )]
    http_equiv: (),
    #[webapi(
        accessor_property,
        getter = dom_string_reflection_getter_function,
        setter = dom_string_reflection_setter_function,
        data = DomStringReflection::MetaScheme
    )]
    scheme: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLMetaElement, enumerable)]
pub(super) struct HtmlMetaElementMediaPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_media_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::MetaMedia
    )]
    media: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLAnchorElement, enumerable)]
pub(super) struct HtmlAnchorElementTargetPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = anchor_target_getter_function,
        setter = anchor_target_setter_function
    )]
    target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLAreaElement, enumerable)]
pub(super) struct HtmlAreaElementTargetPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = area_target_getter_function,
        setter = area_target_setter_function
    )]
    target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLAreaElement, enumerable)]
pub(super) struct HtmlAreaElementReferrerPolicyPrototypeDeclaration {
    #[webapi(
        accessor_property = "referrerPolicy",
        getter = html_referrer_policy_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaReferrerPolicy
    )]
    referrer_policy: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLAreaElement, enumerable)]
pub(super) struct HtmlAreaElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_alt_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaAlt
    )]
    alt: (),
    #[webapi(
        accessor_property,
        getter = html_coords_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaCoords
    )]
    coords: (),
    #[webapi(
        accessor_property,
        getter = html_download_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaDownload
    )]
    download: (),
    #[webapi(
        accessor_property,
        getter = html_hreflang_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaHreflang
    )]
    hreflang: (),
    #[webapi(
        accessor_property,
        getter = html_shape_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaShape
    )]
    shape: (),
    #[webapi(
        accessor_property,
        getter = html_ping_getter_function,
        setter = usv_string_reflection_setter_function,
        setter_data = UsvStringReflection::AreaPing
    )]
    ping: (),
    #[webapi(
        accessor_property,
        getter = html_type_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::AreaType
    )]
    r#type: (),
    #[webapi(
        accessor_property = "noHref",
        getter = html_no_href_getter_function,
        setter = area_no_href_setter_function
    )]
    no_href: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLBaseElement, enumerable)]
pub(super) struct HtmlBaseElementTargetPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = base_target_getter_function,
        setter = base_target_setter_function
    )]
    target: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLLinkElement, enumerable)]
pub(super) struct HtmlLinkElementTargetPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = link_target_getter_function,
        setter = link_target_setter_function
    )]
    target: (),
}

pub(super) fn install_html_rel_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: ElementReflectionInterface,
) {
    let getter = v8::FunctionTemplate::builder(html_rel_getter_function)
        .length(0)
        .build(scope);
    getter.set_class_name(v8str(scope, "get rel"));
    let setter_data = interface
        .to_v8_template_value(scope)
        .expect("Element reflection interface must convert to V8 template data");
    let setter = v8::FunctionTemplate::builder(html_rel_setter_function)
        .data(setter_data)
        .length(1)
        .build(scope);
    setter.set_class_name(v8str(scope, "set rel"));
    prototype.set_accessor_property(
        v8str(scope, "rel").into(),
        Some(getter),
        Some(setter),
        v8::PropertyAttribute::NONE,
    );

    let getter = v8::FunctionTemplate::builder(html_rel_list_getter_function)
        .length(0)
        .build(scope);
    getter.set_class_name(v8str(scope, "get relList"));
    let setter_data = interface
        .to_v8_template_value(scope)
        .expect("Element reflection interface must convert to V8 template data");
    let setter = v8::FunctionTemplate::builder(html_rel_list_setter_function)
        .data(setter_data)
        .length(1)
        .build(scope);
    setter.set_class_name(v8str(scope, "set relList"));
    prototype.set_accessor_property(
        v8str(scope, "relList").into(),
        Some(getter),
        Some(setter),
        v8::PropertyAttribute::NONE,
    );
}

pub(super) fn install_html_name_template_binding<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: ElementReflectionInterface,
) {
    let getter = v8::FunctionTemplate::builder(html_name_getter_function)
        .length(0)
        .build(scope);
    getter.set_class_name(v8str(scope, "get name"));
    let setter_data = interface
        .to_v8_template_value(scope)
        .expect("Element reflection interface must convert to V8 template data");
    let setter = v8::FunctionTemplate::builder(html_name_setter_function)
        .data(setter_data)
        .length(1)
        .build(scope);
    setter.set_class_name(v8str(scope, "set name"));
    prototype.set_accessor_property(
        v8str(scope, "name").into(),
        Some(getter),
        Some(setter),
        v8::PropertyAttribute::NONE,
    );
}

pub(super) const HTML_NAME_REFLECTION_INTERFACES: &[ElementReflectionInterface] = &[
    ElementReflectionInterface::HtmlAnchorElement,
    ElementReflectionInterface::HtmlButtonElement,
    ElementReflectionInterface::HtmlDetailsElement,
    ElementReflectionInterface::HtmlEmbedElement,
    ElementReflectionInterface::HtmlFieldSetElement,
    ElementReflectionInterface::HtmlFrameElement,
    ElementReflectionInterface::HtmlIFrameElement,
    ElementReflectionInterface::HtmlImageElement,
    ElementReflectionInterface::HtmlInputElement,
    ElementReflectionInterface::HtmlMapElement,
    ElementReflectionInterface::HtmlMetaElement,
    ElementReflectionInterface::HtmlObjectElement,
    ElementReflectionInterface::HtmlOutputElement,
    ElementReflectionInterface::HtmlParamElement,
    ElementReflectionInterface::HtmlSelectElement,
    ElementReflectionInterface::HtmlTextAreaElement,
];
