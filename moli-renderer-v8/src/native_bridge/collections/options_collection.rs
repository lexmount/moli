use super::*;
use crate::util::throw_type_error;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLOptionsCollection.remove")]
struct OptionsCollectionRemoveArgs {
    #[webidl(required)]
    index: i32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLOptionsCollection.add")]
struct OptionsCollectionAddArgs<'s> {
    #[webidl(required, converter = "raw")]
    element: v8::Local<'s, v8::Value>,
    #[webidl(index = 1, converter = "raw")]
    before: Option<v8::Local<'s, v8::Value>>,
}

fn options_collection_descriptor(
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Object>,
) -> Option<(*mut PageScriptHost, LiveCollectionDescriptor)> {
    let Ok((runtime_ptr, descriptor)) = live_collection_descriptor_from_object(scope, receiver)
    else {
        throw_type_error(scope, "Illegal invocation");
        return None;
    };
    if descriptor.collection_kind != CollectionKind::OptionsCollection {
        throw_type_error(scope, "Illegal invocation");
        return None;
    }
    Some((runtime_ptr, descriptor))
}

pub(in crate::native_bridge::collections) fn options_collection_length_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, descriptor)) = options_collection_descriptor(scope, args.this()) else {
        return;
    };
    let requested_len = match webidl::convert::<webidl::UnsignedLong>(
        scope,
        args.get(0),
        webidl::Context::member("HTMLOptionsCollection", "length"),
    ) {
        Ok(value) => value.0 as usize,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let current_len = unsafe { &*runtime_ptr }
        .dom_host()
        .select_option_elements(descriptor.root)
        .len();
    let Some(next_len) = select_options_resize_target(current_len, requested_len) else {
        return;
    };
    resize_select_options(scope, runtime_ptr, descriptor.root, next_len);
}

pub(in crate::native_bridge::collections) fn options_collection_selected_index_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, descriptor)) = options_collection_descriptor(scope, args.this()) else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let selected = runtime
        .dom_host()
        .select_selected_option_elements(descriptor.root)
        .first()
        .copied();
    let index = selected
        .and_then(|selected| {
            runtime
                .dom_host()
                .select_option_elements(descriptor.root)
                .into_iter()
                .position(|option| option == selected)
        })
        .map(|index| index as i32)
        .unwrap_or(-1);
    rv.set_int32(index);
}

pub(in crate::native_bridge::collections) fn options_collection_selected_index_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, descriptor)) = options_collection_descriptor(scope, args.this()) else {
        return;
    };
    let index = match webidl::convert::<webidl::Long>(
        scope,
        args.get(0),
        webidl::Context::member("HTMLOptionsCollection", "selectedIndex"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let handles = runtime.dom_host().select_option_elements(descriptor.root);
    let mut matched = false;
    for (position, handle) in handles.into_iter().enumerate() {
        let should_select = position as i32 == index;
        matched |= should_select;
        let _ = runtime.set_selected_state(scope, runtime_ptr, handle, should_select);
    }
    let _ = runtime.set_select_explicit_none(scope, runtime_ptr, descriptor.root, !matched);
}

pub(in crate::native_bridge::collections) fn options_collection_add_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, descriptor)) = options_collection_descriptor(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<OptionsCollectionAddArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(option) = current_or_live_delegate_node_arg_handle(scope, runtime_ptr, parsed.element)
        .or_else(|| callback_value_dom_handle(scope, parsed.element))
    else {
        throw_type_error(
            scope,
            "HTMLOptionsCollection.add requires an option or optgroup element",
        );
        return;
    };
    let Some((parent, before)) = ({
        let runtime = unsafe { &*runtime_ptr };
        if !runtime.dom_host().is_html_element_named(option, "option")
            && !runtime.dom_host().is_html_element_named(option, "optgroup")
        {
            throw_type_error(
                scope,
                "HTMLOptionsCollection.add requires an option or optgroup element",
            );
            return;
        }
        select_add_insertion_point(
            scope,
            runtime,
            descriptor.root,
            option,
            parsed.before,
            "HTMLOptionsCollection.add",
        )
    }) else {
        return;
    };
    if let Some(reference) = before {
        let _ =
            insert_before_in_reaction_scope(scope, runtime_ptr, parent, option, Some(reference));
    } else {
        let _ = append_child_in_reaction_scope(scope, runtime_ptr, parent, option);
    }
    rv.set_undefined();
}

pub(in crate::native_bridge::collections) fn options_collection_indexed_setter(
    scope: &mut v8::PinScope<'_, '_>,
    index: u32,
    value: v8::Local<'_, v8::Value>,
    args: v8::PropertyCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Ok((runtime_ptr, descriptor)) =
        live_collection_descriptor_from_object(scope, args.holder())
    else {
        return v8::Intercepted::kNo;
    };
    if descriptor.collection_kind != CollectionKind::OptionsCollection {
        return v8::Intercepted::kNo;
    }
    if set_select_indexed_option(scope, runtime_ptr, descriptor.root, index, value) {
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}

pub(in crate::native_bridge::collections) fn options_collection_remove_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, descriptor)) = options_collection_descriptor(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<OptionsCollectionRemoveArgs>(scope, &args) else {
        return;
    };
    if parsed.index >= 0
        && let Some(handle) = unsafe { &*runtime_ptr }
            .dom_host()
            .select_option_elements(descriptor.root)
            .get(parsed.index as usize)
            .copied()
        && let Some(parent) = unsafe { &*runtime_ptr }.dom_host().parent_node(handle)
    {
        let _ = remove_child_in_reaction_scope(scope, runtime_ptr, parent, handle);
    }
    rv.set_undefined();
}
