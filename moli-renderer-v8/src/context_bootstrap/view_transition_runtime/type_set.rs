use super::*;
use crate::web_api_interfaces;
use crate::{
    util::{get_private_value, global_constructor_prototype, set_private_value},
    webidl,
    webidl_iterator::{
        SetlikeWebIdlIteratorKind, SetlikeWebIdlIteratorMethod, call_setlike_webidl_for_each,
        new_setlike_webidl_iterator,
    },
};

const VIEW_TRANSITION_TYPE_SET_BACKING_SLOT: &str = "__lmViewTransitionTypeSetBacking";

pub(super) fn new_view_transition_type_set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    initial_types: &[String],
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, "ViewTransitionTypeSet")?;
    let set = v8::Set::new(scope);
    for value in initial_types {
        let value = v8_string(scope, value)?;
        set.add(scope, value.into())?;
    }
    let object = v8::Object::new(scope);
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    web_api_interfaces::ViewTransitionTypeSet::DESCRIPTOR
        .initialize(scope, object)
        .ok()?;
    set_private_value(
        scope,
        object,
        VIEW_TRANSITION_TYPE_SET_BACKING_SLOT,
        set.into(),
    );
    Some(object)
}

fn require_view_transition_type_set_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<v8::Local<'s, v8::Set>> {
    let branded =
        moli_webapi_declare::implements_interface(scope, receiver, "ViewTransitionTypeSet");
    let backing = get_private_value(scope, receiver, VIEW_TRANSITION_TYPE_SET_BACKING_SLOT)
        .and_then(|value| v8::Local::<v8::Set>::try_from(value).ok());
    if branded && backing.is_some() {
        return backing;
    }
    throw_type_error(
        scope,
        &format!("Failed to execute '{member}' on 'ViewTransitionTypeSet': Illegal invocation."),
    );
    None
}

pub(super) fn view_transition_type_set_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(set) = require_view_transition_type_set_receiver(scope, args.this(), "size getter")
    else {
        return;
    };
    rv.set_uint32(set.size() as u32);
}

pub(super) fn view_transition_type_set_add_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(set) = require_view_transition_type_set_receiver(scope, args.this(), "add") else {
        return;
    };
    let Some(value) = view_transition_type_set_string_argument(scope, &args, "add") else {
        return;
    };
    let _ = set.add(scope, value.into());
    rv.set(args.this().into());
}

pub(super) fn view_transition_type_set_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(set) = require_view_transition_type_set_receiver(scope, args.this(), "clear") else {
        return;
    };
    set.clear();
    rv.set_undefined();
}

pub(super) fn view_transition_type_set_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(set) = require_view_transition_type_set_receiver(scope, args.this(), "delete") else {
        return;
    };
    let Some(value) = view_transition_type_set_string_argument(scope, &args, "delete") else {
        return;
    };
    rv.set_bool(set.delete(scope, value.into()).unwrap_or(false));
}

pub(super) fn view_transition_type_set_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(set) = require_view_transition_type_set_receiver(scope, args.this(), "has") else {
        return;
    };
    let Some(value) = view_transition_type_set_string_argument(scope, &args, "has") else {
        return;
    };
    rv.set_bool(set.has(scope, value.into()).unwrap_or(false));
}

pub(super) fn view_transition_type_set_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_view_transition_type_set_receiver(scope, args.this(), "entries")
    else {
        return;
    };
    set_view_transition_type_set_iterator(
        scope,
        backing,
        SetlikeWebIdlIteratorMethod::Entries,
        &mut rv,
    );
}

pub(super) fn view_transition_type_set_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_view_transition_type_set_receiver(scope, args.this(), "forEach")
    else {
        return;
    };
    if let Some(result) = call_setlike_webidl_for_each(
        scope,
        backing,
        args.this(),
        args.get(0),
        args.get(1),
        "ViewTransitionTypeSet forEach",
    ) {
        rv.set(result);
    }
}

pub(super) fn view_transition_type_set_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_view_transition_type_set_receiver(scope, args.this(), "keys")
    else {
        return;
    };
    set_view_transition_type_set_iterator(
        scope,
        backing,
        SetlikeWebIdlIteratorMethod::Keys,
        &mut rv,
    );
}

pub(super) fn view_transition_type_set_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(backing) = require_view_transition_type_set_receiver(scope, args.this(), "values")
    else {
        return;
    };
    set_view_transition_type_set_iterator(
        scope,
        backing,
        SetlikeWebIdlIteratorMethod::Values,
        &mut rv,
    );
}

fn set_view_transition_type_set_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Set>,
    method: SetlikeWebIdlIteratorMethod,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(iterator) = new_setlike_webidl_iterator(
        scope,
        backing,
        SetlikeWebIdlIteratorKind::ViewTransitionTypeSet,
        method,
    ) {
        rv.set(iterator.into());
    } else {
        rv.set_undefined();
    }
}

fn view_transition_type_set_string_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    member: &'static str,
) -> Option<v8::Local<'s, v8::String>> {
    match webidl::argument::<webidl::DomString>(
        scope,
        args,
        0,
        webidl::Context::argument(member, 1),
    ) {
        Ok(value) => v8_string(scope, &value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}
