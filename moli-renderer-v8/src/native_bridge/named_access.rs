use crate::{document_runtime::DomHandle, util::v8_string};
use indexmap::IndexSet;
use moli_dom::native::{DomHost, Element};

use super::{
    JsContextHost, collections,
    element::live_frame_owner_content_window_for_handle,
    identity::{CollectionKind, LiveCollectionQueryKind},
    node::node_runtime_and_handle_from_object,
};

const WINDOW_NAME_ELEMENTS: &[&str] = &["img", "form", "embed", "object"];
const DOCUMENT_ALL_NAME_ELEMENTS: &[&str] = &[
    "a", "button", "embed", "form", "frame", "frameset", "iframe", "img", "input", "map", "meta",
    "object", "select", "textarea",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LegacyNamedAccessKind {
    Window,
    DocumentAll,
}

pub(crate) fn html_element_name_attribute_is_exposed(
    local_name: &str,
    kind: LegacyNamedAccessKind,
) -> bool {
    name_elements(kind).contains(&local_name)
}

pub(crate) fn dom_element_name_attribute_is_exposed(
    dom: &DomHost,
    handle: DomHandle,
    kind: LegacyNamedAccessKind,
) -> bool {
    name_elements(kind)
        .iter()
        .any(|local_name| dom.is_html_element_named(handle, local_name))
}

pub(crate) fn window_named_item_handles(
    dom: &DomHost,
    root: DomHandle,
    name: &str,
) -> Vec<DomHandle> {
    dom.element_handles_by_id_or_name_matching_in_subtree(root, name, |handle| {
        dom_element_name_attribute_is_exposed(dom, handle, LegacyNamedAccessKind::Window)
    })
}

pub(crate) fn document_all_named_item_handles(dom: &DomHost, name: &str) -> Vec<DomHandle> {
    dom.element_handles_by_id_or_name_matching(name, |handle| {
        dom_element_name_attribute_is_exposed(dom, handle, LegacyNamedAccessKind::DocumentAll)
    })
}

pub(crate) fn document_named_item_handles(dom: &DomHost, name: &str) -> Vec<DomHandle> {
    // Document named access deliberately has narrower legacy matching than
    // Window or HTMLCollection.
    // https://html.spec.whatwg.org/multipage/dom.html#dom-document-nameditem
    dom.element_handles_by_id_or_name_matching(name, |_| true)
        .into_iter()
        .filter(|handle| {
            let Some(element) = dom
                .node(*handle)
                .and_then(moli_dom::native::Node::as_element)
            else {
                return false;
            };
            document_named_element_property_names(dom, *handle, element)
                .into_iter()
                .flatten()
                .any(|candidate| candidate == name)
        })
        .collect()
}

fn document_named_element_property_names<'a>(
    dom: &DomHost,
    handle: DomHandle,
    element: &'a Element,
) -> [Option<&'a str>; 2] {
    let name = element.name_attribute().filter(|name| !name.is_empty());
    let id = element.id().filter(|id| !id.is_empty());
    if dom.is_html_element_named(handle, "object") {
        return [id, name];
    }
    if dom.is_html_element_named(handle, "img") && name.is_some() {
        return [id, name];
    }
    if ["embed", "form", "iframe"]
        .into_iter()
        .any(|local_name| dom.is_html_element_named(handle, local_name))
    {
        return [None, name];
    }
    [None, None]
}

fn document_supported_property_names(dom: &DomHost, document_handle: DomHandle) -> Vec<String> {
    if !dom
        .node(document_handle)
        .is_some_and(moli_dom::native::Node::is_document)
    {
        return Vec::new();
    }

    let mut names = IndexSet::new();
    let mut stack = dom
        .child_handles_reversed(document_handle)
        .collect::<Vec<_>>();
    while let Some(handle) = stack.pop() {
        if let Some(element) = dom
            .node(handle)
            .and_then(moli_dom::native::Node::as_element)
        {
            for name in document_named_element_property_names(dom, handle, element)
                .into_iter()
                .flatten()
            {
                names.insert(name.to_owned());
            }
        }
        stack.extend(dom.child_handles_reversed(handle));
    }
    names.into_iter().collect()
}

pub(crate) fn build_window_named_items_collection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    root: DomHandle,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    Some(collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        root,
        CollectionKind::HtmlCollection,
        LiveCollectionQueryKind::WindowNamedItems,
        Some(name.to_owned()),
        false,
    ))
}

fn build_document_named_items_collection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    Some(collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        document_handle,
        CollectionKind::HtmlCollection,
        LiveCollectionQueryKind::DocumentNamedItems,
        Some(name.to_owned()),
        false,
    ))
}

type DocumentNamedAccessContext = (*mut JsContextHost, DomHandle, String, Vec<DomHandle>);

fn is_document_legacy_unforgeable_property(name: &str) -> bool {
    // Document is [LegacyOverrideBuiltIns], so ordinary interface properties do
    // not mask supported named properties. `location` is the exception because
    // its IDL attribute is [LegacyUnforgeable] and must remain the own accessor.
    // https://html.spec.whatwg.org/multipage/dom.html#the-document-object
    name == "location"
}

fn document_runtime_and_handle(
    scope: &mut v8::PinScope<'_, '_>,
    holder: v8::Local<'_, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let (runtime_ptr, document_handle) = node_runtime_and_handle_from_object(scope, holder).ok()?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(document_handle)
        .is_some_and(moli_dom::native::Node::is_document)
        .then_some((runtime_ptr, document_handle))
}

fn document_named_access_context_for_name(
    scope: &mut v8::PinScope<'_, '_>,
    name: String,
    holder: v8::Local<'_, v8::Object>,
) -> Option<DocumentNamedAccessContext> {
    if name.is_empty() || is_document_legacy_unforgeable_property(&name) {
        return None;
    }
    let (runtime_ptr, document_handle) = document_runtime_and_handle(scope, holder)?;
    let runtime = unsafe { &*runtime_ptr };
    let handles = document_named_item_handles(runtime.dom_host(), &name);
    (!handles.is_empty()).then_some((runtime_ptr, document_handle, name, handles))
}

fn document_named_access_context(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
    holder: v8::Local<'_, v8::Object>,
) -> Option<DocumentNamedAccessContext> {
    let key = v8::Local::<v8::String>::try_from(key).ok()?;
    document_named_access_context_for_name(scope, key.to_rust_string_lossy(scope), holder)
}

fn document_named_access_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: DocumentNamedAccessContext,
) -> Option<v8::Local<'s, v8::Value>> {
    let (runtime_ptr, document_handle, name, handles) = context;
    match handles.as_slice() {
        [handle] => {
            if unsafe { &*runtime_ptr }
                .dom_host()
                .is_html_element_named(*handle, "iframe")
                && let Some(window) =
                    live_frame_owner_content_window_for_handle(scope, runtime_ptr, *handle)
            {
                return Some(window.into());
            }
            unsafe { &mut *runtime_ptr }
                .native_bridge_mut()
                .wrap_handle(scope, runtime_ptr, *handle)
                .map(Into::into)
        }
        _ => build_document_named_items_collection(scope, runtime_ptr, document_handle, &name)
            .map(Into::into),
    }
}

fn document_named_property_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(context) = document_named_access_context(scope, key, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = document_named_access_value(scope, context) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn document_named_property_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if document_named_access_context(scope, key, args.holder()).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
    v8::Intercepted::kYes
}

fn document_indexed_property_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(context) =
        document_named_access_context_for_name(scope, index.to_string(), args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = document_named_access_value(scope, context) else {
        return v8::Intercepted::kNo;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn document_indexed_property_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if document_named_access_context_for_name(scope, index.to_string(), args.holder()).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
    v8::Intercepted::kYes
}

fn document_indexed_property_enumerator(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Some((runtime_ptr, document_handle)) = document_runtime_and_handle(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys =
        document_supported_property_names(unsafe { &*runtime_ptr }.dom_host(), document_handle)
            .into_iter()
            .filter_map(|name| collections::array_index_property_name(&name))
            .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
            .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn document_named_property_enumerator(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Some((runtime_ptr, document_handle)) = document_runtime_and_handle(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys =
        document_supported_property_names(unsafe { &*runtime_ptr }.dom_host(), document_handle)
            .into_iter()
            .filter(|name| collections::array_index_property_name(name).is_none())
            .filter_map(|name| v8_string(scope, &name).map(Into::into))
            .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

pub(in crate::native_bridge) fn install_document_named_property_handler(
    template: v8::Local<'_, v8::ObjectTemplate>,
) {
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(document_indexed_property_getter)
            .query(document_indexed_property_query)
            .enumerator(document_indexed_property_enumerator),
    );
    template.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(document_named_property_getter)
            .query(document_named_property_query)
            .enumerator(document_named_property_enumerator)
            .flags(v8::PropertyHandlerFlags::ONLY_INTERCEPT_STRINGS),
    );
}

fn name_elements(kind: LegacyNamedAccessKind) -> &'static [&'static str] {
    match kind {
        LegacyNamedAccessKind::Window => WINDOW_NAME_ELEMENTS,
        LegacyNamedAccessKind::DocumentAll => DOCUMENT_ALL_NAME_ELEMENTS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_name_attribute_candidates_are_scoped_by_consumer() {
        for local_name in ["img", "form", "embed", "object"] {
            assert!(html_element_name_attribute_is_exposed(
                local_name,
                LegacyNamedAccessKind::Window
            ));
        }
        for local_name in ["a", "button", "iframe", "input", "meta", "applet", "div"] {
            assert!(!html_element_name_attribute_is_exposed(
                local_name,
                LegacyNamedAccessKind::Window
            ));
        }
        for local_name in [
            "a", "button", "embed", "form", "frame", "frameset", "iframe", "img", "input", "map",
            "meta", "object", "select", "textarea",
        ] {
            assert!(html_element_name_attribute_is_exposed(
                local_name,
                LegacyNamedAccessKind::DocumentAll
            ));
        }
        for local_name in ["applet", "div", "svg"] {
            assert!(!html_element_name_attribute_is_exposed(
                local_name,
                LegacyNamedAccessKind::DocumentAll
            ));
        }
    }
}
