use super::helpers::{element_option_value, select_option_handles, selected_index_for_select};
use super::*;
use crate::{
    native_bridge::{
        callback_value_dom_handle,
        element::{html_element_getter_receiver, html_element_setter_receiver},
        node::throw_incompatible_method_receiver,
    },
    webidl,
};
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

const MAX_SELECT_OPTIONS_LENGTH: usize = 100_000;

pub(in crate::native_bridge) fn select_options_resize_target(
    current_len: usize,
    requested_len: usize,
) -> Option<usize> {
    (requested_len <= current_len || requested_len <= MAX_SELECT_OPTIONS_LENGTH)
        .then_some(requested_len)
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLSelectElement.item")]
struct SelectItemArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLSelectElement.namedItem")]
struct SelectNamedItemArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLSelectElement.remove")]
struct SelectRemoveArgs {
    #[webidl(with = select_remove_index_arg)]
    index: Option<i32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLSelectElement.add")]
struct SelectAddArgs<'s> {
    #[webidl(required, converter = "raw")]
    element: v8::Local<'s, v8::Value>,
    #[webidl(index = 1, converter = "raw")]
    before: Option<v8::Local<'s, v8::Value>>,
}

fn select_getter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_getter_receiver(scope, receiver, "HTMLSelectElement", member, "select")
}

fn select_setter_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    html_element_setter_receiver(scope, receiver, "HTMLSelectElement", member, "select")
}

fn select_method_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    method: &'static str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        throw_incompatible_method_receiver(scope, "HTMLSelectElement", method);
        return None;
    };
    if !unsafe { &*runtime_ptr }
        .dom_host()
        .is_html_element_named(handle, "select")
    {
        throw_incompatible_method_receiver(scope, "HTMLSelectElement", method);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn select_boolean_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &'static str,
    property: &'static str,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_getter_receiver(scope, receiver, property) else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(element_has_attribute(
        unsafe { &*runtime_ptr },
        handle,
        attribute,
    ));
}

fn set_select_boolean_attribute_on_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    attribute: &'static str,
    property: &'static str,
    value: v8::Local<'s, v8::Value>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let (runtime_ptr, handle) = select_setter_receiver(scope, receiver, property)?;
    set_reflected_boolean_attribute(
        scope,
        runtime_ptr,
        handle,
        attribute,
        value.boolean_value(scope),
    );
    Some((runtime_ptr, handle))
}

pub(in crate::native_bridge) fn select_add_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, select_handle)) = select_method_receiver(scope, args.this(), "add")
    else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<SelectAddArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(node_handle) =
        current_or_live_delegate_node_arg_handle(scope, runtime_ptr, parsed.element)
            .or_else(|| node_or_foreign_arg_handle(scope, runtime_ptr, None, parsed.element))
    else {
        throw_type_error(
            scope,
            "HTMLSelectElement.add requires an option or optgroup element",
        );
        return;
    };
    let Some((parent, reference)) = ({
        let runtime = unsafe { &*runtime_ptr };
        if !is_select_add_candidate(runtime, node_handle) {
            throw_type_error(
                scope,
                "HTMLSelectElement.add requires an option or optgroup element",
            );
            return;
        }
        select_add_insertion_point(
            scope,
            runtime,
            select_handle,
            node_handle,
            parsed.before,
            "HTMLSelectElement.add",
        )
    }) else {
        return;
    };
    let inserted = if let Some(reference) = reference {
        insert_before_in_reaction_scope(scope, runtime_ptr, parent, node_handle, Some(reference))
    } else {
        append_child_in_reaction_scope(scope, runtime_ptr, parent, node_handle)
    };
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    rv.set_undefined();
}

fn select_remove_index_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<i32>, webidl::WebIdlError> {
    if args.length() <= index {
        return Ok(None);
    }
    webidl::argument::<webidl::Long>(
        scope,
        args,
        index,
        webidl::Context::argument("HTMLSelectElement.remove", (index + 1) as usize),
    )
    .map(|value| Some(value.0))
}

pub(in crate::native_bridge) fn select_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(option) = runtime
        .dom_host()
        .select_option_elements(handle)
        .get(index as usize)
        .copied()
    else {
        return v8::Intercepted::kNo;
    };
    let Some(node) = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, option)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(node.into());
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn select_indexed_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, select_handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if set_select_indexed_option(scope, runtime_ptr, select_handle, index, value) {
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}

pub(in crate::native_bridge) fn select_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if unsafe { &*runtime_ptr }
        .dom_host()
        .select_option_elements(handle)
        .len()
        <= index as usize
    {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::NONE.as_u32() as i32);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn select_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if unsafe { &*runtime_ptr }
        .dom_host()
        .select_option_elements(handle)
        .len()
        <= index as usize
    {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn select_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    let keys = (0..unsafe { &*runtime_ptr }
        .dom_host()
        .select_option_elements(handle)
        .len())
        .map(|index| v8::Integer::new_from_unsigned(scope, index as u32).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

pub(in crate::native_bridge) fn select_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(option) = runtime
        .dom_host()
        .select_option_elements(handle)
        .get(index as usize)
        .copied()
    else {
        return v8::Intercepted::kNo;
    };
    let Some(value) = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, option)
    else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) =
        DataPropertyDescriptorDeclaration::new(value.into(), true, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

pub(in crate::native_bridge) fn select_indexed_definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, select_handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if descriptor.has_get() || descriptor.has_set() {
        rv.set_bool(false);
        return v8::Intercepted::kYes;
    }
    let value = if descriptor.has_value() {
        descriptor.value()
    } else {
        v8::undefined(scope).into()
    };
    if set_select_indexed_option(scope, runtime_ptr, select_handle, index, value) {
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}

pub(in crate::native_bridge) fn select_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_method_receiver(scope, args.this(), "item") else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<SelectItemArgs>(scope, &args) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(option) = runtime
        .dom_host()
        .select_option_elements(handle)
        .get(parsed.index as usize)
        .copied()
    else {
        rv.set_null();
        return;
    };
    set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, Some(option));
}

pub(in crate::native_bridge) fn select_named_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_method_receiver(scope, args.this(), "namedItem")
    else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<SelectNamedItemArgs>(scope, &args) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let option = runtime
        .dom_host()
        .select_option_elements(handle)
        .into_iter()
        .find(|option| {
            runtime
                .dom_host()
                .node(*option)
                .and_then(Node::as_element)
                .is_some_and(|element| element.matches_named_item_key(&parsed.name))
        });
    set_wrapped_node_or_null(scope, &mut rv, runtime_ptr, option);
}

pub(in crate::native_bridge) fn select_remove_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_method_receiver(scope, args.this(), "remove") else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<SelectRemoveArgs>(scope, &args) else {
        return;
    };
    let Some(index) = parsed.index else {
        if let Some(parent) = unsafe { &*runtime_ptr }.dom_host().parent_node(handle) {
            let _ = remove_child_in_reaction_scope(scope, runtime_ptr, parent, handle);
        }
        rv.set_undefined();
        return;
    };
    if index >= 0
        && let Some(option) = unsafe { &*runtime_ptr }
            .dom_host()
            .select_option_elements(handle)
            .get(index as usize)
            .copied()
        && let Some(parent) = unsafe { &*runtime_ptr }.dom_host().parent_node(option)
    {
        let _ = remove_child_in_reaction_scope(scope, runtime_ptr, parent, option);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn select_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    select_boolean_attribute_getter(scope, args.this(), "disabled", "disabled", rv);
}

pub(in crate::native_bridge) fn select_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let _ = set_select_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "disabled",
        "disabled",
        args.get(0),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn select_multiple_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    select_boolean_attribute_getter(scope, args.this(), "multiple", "multiple", rv);
}

pub(in crate::native_bridge) fn select_multiple_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_setter_receiver(scope, args.this(), "multiple") else {
        rv.set_undefined();
        return;
    };
    let next = args.get(0).boolean_value(scope);
    let had_multiple = element_has_attribute(unsafe { &*runtime_ptr }, handle, "multiple");
    set_reflected_boolean_attribute(scope, runtime_ptr, handle, "multiple", next);
    if had_multiple && !next {
        normalize_single_select_selectedness(scope, runtime_ptr, handle);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn select_length_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_getter_receiver(scope, args.this(), "length") else {
        rv.set_uint32(0);
        return;
    };
    rv.set_uint32(select_option_handles(unsafe { &*runtime_ptr }, handle).len() as u32);
}

pub(in crate::native_bridge) fn select_length_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_setter_receiver(scope, args.this(), "length") else {
        rv.set_undefined();
        return;
    };
    let requested_len = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        args.get(0),
        webidl::Context::member("HTMLSelectElement", "length"),
    ) {
        Ok(value) => value.0 as usize,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let current_len = select_option_handles(unsafe { &*runtime_ptr }, handle).len();
    let Some(next_len) = select_options_resize_target(current_len, requested_len) else {
        rv.set_undefined();
        return;
    };
    resize_select_options(scope, runtime_ptr, handle, next_len);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn select_options_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_getter_receiver(scope, args.this(), "options") else {
        rv.set_null();
        return;
    };
    let descriptor = LiveCollectionDescriptor {
        collection_kind: CollectionKind::OptionsCollection,
        query_kind: LiveCollectionQueryKind::Options,
        root: handle,
        query: None,
        include_root: false,
        tag_name_html_document: None,
        resolution_cache: Default::default(),
    };
    let collection = collections::build_live_collection_wrapper(scope, runtime_ptr, descriptor);
    rv.set(collection.into());
}

pub(in crate::native_bridge) fn select_selected_options_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_getter_receiver(scope, args.this(), "selectedOptions")
    else {
        rv.set_null();
        return;
    };
    let descriptor = LiveCollectionDescriptor {
        collection_kind: CollectionKind::HtmlCollection,
        query_kind: LiveCollectionQueryKind::SelectedOptions,
        root: handle,
        query: None,
        include_root: false,
        tag_name_html_document: None,
        resolution_cache: Default::default(),
    };
    let collection = collections::build_live_collection_wrapper(scope, runtime_ptr, descriptor);
    rv.set(collection.into());
}

pub(in crate::native_bridge) fn select_selected_index_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_getter_receiver(scope, args.this(), "selectedIndex")
    else {
        rv.set_int32(-1);
        return;
    };
    rv.set_int32(selected_index_for_select(unsafe { &*runtime_ptr }, handle));
}

pub(in crate::native_bridge) fn select_selected_index_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_setter_receiver(scope, args.this(), "selectedIndex")
    else {
        rv.set_undefined();
        return;
    };
    let index = match webidl::convert::<webidl::Long>(
        scope,
        args.get(0),
        webidl::Context::member("HTMLSelectElement", "selectedIndex"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let options = select_option_handles(runtime, handle);
    let mut matched = false;
    for (position, option) in options.into_iter().enumerate() {
        let should_select = position as i32 == index;
        matched |= should_select;
        let _ = runtime.set_selected_state(scope, runtime_ptr, option, should_select);
    }
    let _ = runtime.set_select_explicit_none(scope, runtime_ptr, handle, !matched);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn select_required_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    select_boolean_attribute_getter(scope, args.this(), "required", "required", rv);
}

pub(in crate::native_bridge) fn select_required_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let _ = set_select_boolean_attribute_on_receiver(
        scope,
        args.this(),
        "required",
        "required",
        args.get(0),
    );
    rv.set_undefined();
}

pub(in crate::native_bridge) fn select_value_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_getter_receiver(scope, args.this(), "value") else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let value = runtime
        .dom_host()
        .select_selected_option_elements(handle)
        .first()
        .copied()
        .and_then(|option| element_option_value(runtime, option))
        .unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn select_value_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_setter_receiver(scope, args.this(), "value") else {
        rv.set_undefined();
        return;
    };
    let Some(next_value) =
        form_dom_string_property_value(scope, args.get(0), "HTMLSelectElement", "value", false)
    else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }.set_select_value(handle, &next_value);
    rv.set_undefined();
}

fn is_select_add_candidate(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime.dom_host().is_html_element_named(handle, "option")
        || runtime.dom_host().is_html_element_named(handle, "optgroup")
}

pub(in crate::native_bridge) fn select_add_insertion_point<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &JsContextHost,
    select_handle: DomHandle,
    added_handle: DomHandle,
    before: Option<v8::Local<'s, v8::Value>>,
    prefix: &'static str,
) -> Option<(DomHandle, Option<DomHandle>)> {
    let Some(raw) = before else {
        return Some((select_handle, None));
    };
    if raw.is_null_or_undefined() {
        return Some((select_handle, None));
    }
    if let Some(reference) = callback_value_dom_handle(scope, raw) {
        if reference == added_handle {
            return None;
        }
        if runtime
            .dom_host()
            .is_html_element_named(reference, "optgroup")
        {
            return Some((
                runtime
                    .dom_host()
                    .parent_node(reference)
                    .unwrap_or(select_handle),
                Some(reference),
            ));
        }
        return Some((
            runtime
                .dom_host()
                .parent_node(reference)
                .unwrap_or(select_handle),
            Some(reference),
        ));
    }
    let index =
        match webidl::convert::<webidl::Long>(scope, raw, webidl::Context::argument(prefix, 2)) {
            Ok(value) => value.0,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
    if index < 0 {
        return Some((select_handle, None));
    }
    let Some(reference) = runtime
        .dom_host()
        .select_option_elements(select_handle)
        .get(index as usize)
        .copied()
    else {
        return Some((select_handle, None));
    };
    Some((
        runtime
            .dom_host()
            .parent_node(reference)
            .unwrap_or(select_handle),
        Some(reference),
    ))
}

pub(in crate::native_bridge) fn resize_select_options(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    select_handle: DomHandle,
    next_len: usize,
) {
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let mut options = unsafe { &*runtime_ptr }
            .dom_host()
            .select_option_elements(select_handle);
        while options.len() > next_len {
            let Some(option) = options.pop() else {
                break;
            };
            if let Some(parent) = unsafe { &*runtime_ptr }.dom_host().parent_node(option) {
                let _ = remove_child_to_current_reaction_queue(scope, runtime_ptr, parent, option);
            } else {
                break;
            }
        }
        let current_len = options.len();
        for _ in current_len..next_len {
            let option = unsafe { &mut *runtime_ptr }.create_element("option");
            let _ =
                append_child_to_current_reaction_queue(scope, runtime_ptr, select_handle, option);
        }
    });
}

pub(in crate::native_bridge) fn set_select_indexed_option(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    select_handle: DomHandle,
    index: u32,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    let runtime = unsafe { &mut *runtime_ptr };
    if value.is_null() {
        if let Some(option) = runtime
            .dom_host()
            .select_option_elements(select_handle)
            .get(index as usize)
            .copied()
            && let Some(parent) = runtime.dom_host().parent_node(option)
        {
            let _ = remove_child_in_reaction_scope(scope, runtime_ptr, parent, option);
        }
        return true;
    }
    let Some(next_option) = current_or_live_delegate_node_arg_handle(scope, runtime_ptr, value)
        .or_else(|| callback_value_dom_handle(scope, value))
    else {
        return false;
    };
    if !runtime
        .dom_host()
        .is_html_element_named(next_option, "option")
    {
        return false;
    }
    let index = index as usize;
    let options = runtime.dom_host().select_option_elements(select_handle);
    if let Some(current) = options.get(index).copied() {
        let parent = runtime
            .dom_host()
            .parent_node(current)
            .unwrap_or(select_handle);
        custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            let _ = insert_before_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                next_option,
                Some(current),
            );
            let _ = remove_child_to_current_reaction_queue(scope, runtime_ptr, parent, current);
        });
        return true;
    }
    if select_options_resize_target(options.len(), index.saturating_add(1)).is_none() {
        return true;
    }
    if index > options.len() {
        resize_select_options(scope, runtime_ptr, select_handle, index);
    }
    let _ = append_child_in_reaction_scope(scope, runtime_ptr, select_handle, next_option);
    true
}

pub(in crate::native_bridge) fn normalize_single_select_selectedness(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    select_handle: DomHandle,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    if element_has_attribute(runtime, select_handle, "multiple") {
        return;
    }
    let options = runtime.dom_host().select_option_elements(select_handle);
    let selected = options.iter().rev().copied().find(|option| {
        runtime
            .dom_host()
            .node(*option)
            .and_then(Node::as_element)
            .is_some_and(Element::selected)
    });
    if let Some(selected) = selected {
        for option in options {
            let _ = runtime.set_selected_state(scope, runtime_ptr, option, option == selected);
        }
        let _ = runtime.set_select_explicit_none(scope, runtime_ptr, select_handle, false);
    }
}

pub(in crate::native_bridge) fn select_size_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_getter_receiver(scope, args.this(), "size") else {
        rv.set_int32(0);
        return;
    };
    let size = element_attribute(unsafe { &*runtime_ptr }, handle, "size")
        .map(|value| parse_non_negative_integer_prefix(&value))
        .unwrap_or(0);
    rv.set_int32(size);
}

fn parse_non_negative_integer_prefix(value: &str) -> i32 {
    let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let value = value.strip_prefix('+').unwrap_or(value);
    let end = value
        .bytes()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(value.len());
    let digits = &value[..end];
    if digits.is_empty() {
        return 0;
    }
    digits.parse::<i32>().unwrap_or(0)
}

pub(in crate::native_bridge) fn select_size_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = select_setter_receiver(scope, args.this(), "size") else {
        rv.set_undefined();
        return;
    };
    let Some(value) = args.get(0).number_value(scope) else {
        rv.set_undefined();
        return;
    };
    let size = webidl_long_from_number(value).max(0);
    set_reflected_attribute(scope, runtime_ptr, handle, "size", &size.to_string());
    rv.set_undefined();
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SELECT_OPTIONS_LENGTH, parse_non_negative_integer_prefix, select_options_resize_target,
    };

    #[test]
    fn options_length_gate_rejects_only_over_limit_expansion() {
        assert_eq!(
            select_options_resize_target(3, MAX_SELECT_OPTIONS_LENGTH),
            Some(MAX_SELECT_OPTIONS_LENGTH)
        );
        assert_eq!(
            select_options_resize_target(3, MAX_SELECT_OPTIONS_LENGTH + 1),
            None
        );
        assert_eq!(
            select_options_resize_target(
                MAX_SELECT_OPTIONS_LENGTH + 2,
                MAX_SELECT_OPTIONS_LENGTH + 1
            ),
            Some(MAX_SELECT_OPTIONS_LENGTH + 1)
        );
    }

    #[test]
    fn select_size_integer_prefix_parser_preserves_html_whitespace_and_plus_rules() {
        assert_eq!(parse_non_negative_integer_prefix(" \t+12px"), 12);
        assert_eq!(parse_non_negative_integer_prefix("12像素"), 12);
        assert_eq!(parse_non_negative_integer_prefix("0"), 0);
        assert_eq!(parse_non_negative_integer_prefix("-1"), 0);
        assert_eq!(parse_non_negative_integer_prefix("\u{a0}12"), 0);
        assert_eq!(parse_non_negative_integer_prefix("2147483647"), i32::MAX);
        assert_eq!(parse_non_negative_integer_prefix("2147483648"), 0);
    }
}
