use super::super::{
    CollectionKind, LiveCollectionQueryKind, collections, encode_tag_name_ns_query,
    node::{
        node_is_document, node_runtime_and_handle_from_args,
        node_runtime_and_handle_from_args_or_detached,
        node_runtime_and_handle_from_object_or_detached, receiver_has_detached_state,
        require_element_method_receiver, require_parent_node_receiver, set_wrapped_node_or_null,
        throw_incompatible_method_receiver, throw_native_selector_error_for_selector,
    },
};
use super::forms::control_matches_validity_pseudo;
use crate::{
    util::{
        call_object_method, object_number_property, object_property_as_object, v8_string, v8str,
    },
    webidl,
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.querySelector")]
struct ElementQuerySelectorArgs {
    #[webidl(required)]
    selectors: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.querySelectorAll")]
struct ElementQuerySelectorAllArgs {
    #[webidl(required)]
    selectors: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.matches")]
struct ElementMatchesArgs {
    #[webidl(required)]
    selectors: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.closest")]
struct ElementClosestArgs {
    #[webidl(required)]
    selectors: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.getElementsByTagName")]
struct ElementGetElementsByTagNameArgs {
    #[webidl(required)]
    qualified_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.getElementsByTagNameNS")]
struct ElementGetElementsByTagNameNsArgs {
    #[webidl(required, nullable)]
    namespace: Option<String>,
    #[webidl(required)]
    local_name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.getElementsByClassName")]
struct ElementGetElementsByClassNameArgs {
    #[webidl(required)]
    class_names: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.getElementsByName")]
struct DocumentGetElementsByNameArgs {
    #[webidl(required)]
    element_name: String,
}

pub(in crate::native_bridge) fn node_query_selector_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        if let Ok((runtime_ptr, handle)) =
            node_runtime_and_handle_from_args_or_detached(scope, &args)
        {
            if !require_parent_node_receiver(
                scope,
                unsafe { &*runtime_ptr },
                handle,
                "querySelector",
                true,
            ) {
                return;
            }
            super::super::document::detached_query_selector_method_callback(scope, args, rv);
        } else if receiver_has_detached_state(scope, args.this()) {
            super::super::document::detached_query_selector_method_callback(scope, args, rv);
        } else {
            throw_incompatible_method_receiver(scope, "ParentNode", "querySelector");
        }
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "querySelector",
        true,
    ) {
        return;
    }
    if super::super::document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this())
        .is_some()
    {
        super::super::document::detached_query_selector_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<ElementQuerySelectorArgs>(scope, &args) else {
        return;
    };
    match unsafe { &*runtime_ptr }.query_selector(Some(handle), &parsed.selectors) {
        Ok(handle) => set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, handle),
        Err(error) => throw_native_selector_error_for_selector(scope, &parsed.selectors, &error),
    }
}

pub(in crate::native_bridge) fn node_query_selector_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        if let Ok((runtime_ptr, handle)) =
            node_runtime_and_handle_from_args_or_detached(scope, &args)
        {
            if !require_parent_node_receiver(
                scope,
                unsafe { &*runtime_ptr },
                handle,
                "querySelectorAll",
                true,
            ) {
                return;
            }
            super::super::document::detached_query_selector_all_method_callback(scope, args, rv);
        } else if receiver_has_detached_state(scope, args.this()) {
            super::super::document::detached_query_selector_all_method_callback(scope, args, rv);
        } else {
            throw_incompatible_method_receiver(scope, "ParentNode", "querySelectorAll");
        }
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "querySelectorAll",
        true,
    ) {
        return;
    }
    if super::super::document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this())
        .is_some()
    {
        super::super::document::detached_query_selector_all_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<ElementQuerySelectorAllArgs>(scope, &args) else {
        return;
    };
    match unsafe { &*runtime_ptr }.query_selector_all(Some(handle), &parsed.selectors) {
        Ok(handles) => {
            let list = collections::build_node_list_from_handles(scope, runtime_ptr, &handles);
            rv.set(list.into());
        }
        Err(error) => throw_native_selector_error_for_selector(scope, &parsed.selectors, &error),
    }
}

pub(in crate::native_bridge) fn node_matches_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(runtime_ptr) = crate::util::context_host_ptr_from_global_bridge(scope)
        && super::super::document::detached_native_handle_for_runtime(
            scope,
            runtime_ptr,
            args.this(),
        )
        .is_some()
    {
        super::super::document::detached_matches_method_callback(scope, args, rv);
        return;
    }
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        if receiver_has_detached_state(scope, args.this()) {
            super::super::document::detached_matches_method_callback(scope, args, rv);
            return;
        }
        throw_incompatible_method_receiver(scope, "Element", "matches");
        rv.set_bool(false);
        return;
    };
    if !require_element_method_receiver(scope, unsafe { &*runtime_ptr }, handle, "matches") {
        return;
    }
    if super::super::document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this())
        .is_some()
    {
        super::super::document::detached_matches_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<ElementMatchesArgs>(scope, &args) else {
        return;
    };
    if let Some(is_match) =
        control_matches_validity_pseudo(scope, runtime_ptr, handle, &parsed.selectors)
    {
        rv.set_bool(is_match);
        return;
    }
    let runtime = unsafe { &*runtime_ptr };
    match runtime.matches(handle, &parsed.selectors) {
        Ok(true) => rv.set_bool(true),
        Ok(false) if node_matches_needs_owner_document_query(scope, args.this()) => rv.set_bool(
            node_matches_owner_document_query(scope, args.this(), &parsed.selectors),
        ),
        Ok(false) => rv.set_bool(false),
        Err(error) => throw_native_selector_error_for_selector(scope, &parsed.selectors, &error),
    }
}

fn node_matches_needs_owner_document_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(owner_document) = object_property_as_object(scope, node, "ownerDocument") else {
        return false;
    };
    let current_document = scope
        .get_current_context()
        .global(scope)
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    current_document.is_none_or(|document| !owner_document.strict_equals(document.into()))
}

fn node_matches_owner_document_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    selector: &str,
) -> bool {
    let Some(document) = object_property_as_object(scope, node, "ownerDocument") else {
        return false;
    };
    let Some(selector) = v8_string(scope, selector) else {
        return false;
    };
    let Some(list) = call_object_method(scope, document, "querySelectorAll", &[selector.into()])
    else {
        return false;
    };
    let Ok(list) = v8::Local::<v8::Object>::try_from(list) else {
        return false;
    };
    let Some(length) = object_number_property(scope, list, "length") else {
        return false;
    };
    for index in 0..length as u32 {
        let Some(candidate) = list.get_index(scope, index) else {
            continue;
        };
        let Ok(candidate) = v8::Local::<v8::Object>::try_from(candidate) else {
            continue;
        };
        if candidate.strict_equals(node.into())
            || call_object_method(scope, candidate, "isSameNode", &[node.into()])
                .is_some_and(|value| value.boolean_value(scope))
            || call_object_method(scope, node, "isSameNode", &[candidate.into()])
                .is_some_and(|value| value.boolean_value(scope))
        {
            return true;
        }
    }
    false
}

pub(in crate::native_bridge) fn node_closest_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "closest");
        rv.set_null();
        return;
    };
    if !require_element_method_receiver(scope, unsafe { &*runtime_ptr }, handle, "closest") {
        return;
    }
    let receiver_is_detached =
        super::super::document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this())
            .is_some();
    let Some(parsed) = webidl::parse_args::<ElementClosestArgs>(scope, &args) else {
        return;
    };
    match unsafe { &*runtime_ptr }.closest(handle, &parsed.selectors) {
        Ok(Some(handle)) if receiver_is_detached => {
            match super::super::document::detached_native_object_for_handle(
                scope,
                runtime_ptr,
                handle,
            ) {
                Some(node) => rv.set(node.into()),
                None => rv.set_null(),
            }
        }
        Ok(handle) => set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, handle),
        Err(error) => throw_native_selector_error_for_selector(scope, &parsed.selectors, &error),
    }
}

pub(in crate::native_bridge) fn node_get_elements_by_tag_name_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        if receiver_has_detached_state(scope, args.this()) {
            super::super::document::detached_get_elements_by_tag_name_method_callback(
                scope, args, rv,
            );
        } else {
            throw_incompatible_method_receiver(scope, "Element", "getElementsByTagName");
        }
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "getElementsByTagName",
        true,
    ) {
        return;
    };
    if super::super::document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this())
        .is_some()
    {
        super::super::document::detached_get_elements_by_tag_name_method_callback(scope, args, rv);
        return;
    }
    let Some(parsed) = webidl::parse_args::<ElementGetElementsByTagNameArgs>(scope, &args) else {
        return;
    };
    let include_root = node_is_document(unsafe { &*runtime_ptr }, handle);
    let collection = collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::HtmlCollection,
        LiveCollectionQueryKind::TagName,
        Some(parsed.qualified_name),
        include_root,
    );
    rv.set(collection.into());
}

pub(in crate::native_bridge) fn node_get_elements_by_tag_name_ns_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        if receiver_has_detached_state(scope, args.this()) {
            super::super::document::detached_get_elements_by_tag_name_ns_method_callback(
                scope, args, rv,
            );
        } else {
            throw_incompatible_method_receiver(scope, "Element", "getElementsByTagNameNS");
        }
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "getElementsByTagNameNS",
        true,
    ) {
        return;
    };
    if super::super::document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this())
        .is_some()
    {
        super::super::document::detached_get_elements_by_tag_name_ns_method_callback(
            scope, args, rv,
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ElementGetElementsByTagNameNsArgs>(scope, &args) else {
        return;
    };
    let include_root = node_is_document(unsafe { &*runtime_ptr }, handle);
    let collection = collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::HtmlCollection,
        LiveCollectionQueryKind::TagNameNs,
        Some(encode_tag_name_ns_query(
            parsed.namespace.as_deref(),
            &parsed.local_name,
        )),
        include_root,
    );
    rv.set(collection.into());
}

pub(in crate::native_bridge) fn node_get_elements_by_class_name_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        if receiver_has_detached_state(scope, args.this()) {
            super::super::document::detached_get_elements_by_class_name_method_callback(
                scope, args, rv,
            );
        } else {
            throw_incompatible_method_receiver(scope, "Element", "getElementsByClassName");
        }
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "getElementsByClassName",
        true,
    ) {
        return;
    };
    if super::super::document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this())
        .is_some()
    {
        super::super::document::detached_get_elements_by_class_name_method_callback(
            scope, args, rv,
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ElementGetElementsByClassNameArgs>(scope, &args) else {
        return;
    };
    let include_root = node_is_document(unsafe { &*runtime_ptr }, handle);
    let collection = collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::HtmlCollection,
        LiveCollectionQueryKind::ClassName,
        Some(parsed.class_names),
        include_root,
    );
    rv.set(collection.into());
}

pub(in crate::native_bridge) fn node_get_elements_by_name_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        if receiver_has_detached_state(scope, args.this()) {
            super::super::document::detached_get_elements_by_name_method_callback(scope, args, rv);
        } else {
            throw_incompatible_method_receiver(scope, "Element", "getElementsByName");
        }
        return;
    };
    if !require_parent_node_receiver(
        scope,
        unsafe { &*runtime_ptr },
        handle,
        "getElementsByName",
        true,
    ) {
        return;
    };
    let Some(parsed) = webidl::parse_args::<DocumentGetElementsByNameArgs>(scope, &args) else {
        return;
    };
    let include_root = node_is_document(unsafe { &*runtime_ptr }, handle);
    let collection = collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::NodeList,
        LiveCollectionQueryKind::Name,
        Some(parsed.element_name),
        include_root,
    );
    rv.set(collection.into());
}
