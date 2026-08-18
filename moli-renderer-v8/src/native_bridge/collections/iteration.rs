use super::*;
use crate::util::throw_type_error;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Collection.item")]
struct CollectionItemArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLCollection.namedItem")]
struct CollectionNamedItemArgs {
    #[webidl(required)]
    name: String,
}

pub(in crate::native_bridge::collections) fn collection_value_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    index: usize,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Ok((runtime_ptr, descriptor)) = live_collection_descriptor_from_object(scope, object) {
        let handle = descriptor
            .resolve(unsafe { &*runtime_ptr })
            .get(index)
            .copied()?;
        return Some(
            wrapped_handle_value(scope, runtime_ptr, handle).unwrap_or_else(|| {
                panic!(
                    "failed to materialize live collection handle `{handle:?}` at index `{index}`"
                )
            }),
        );
    }
    if let Ok((runtime_ptr, collection_id)) = static_handle_collection_id_from_object(scope, object)
    {
        let handle = static_handle_collection_handle_at(runtime_ptr, collection_id, index)?;
        return Some(wrapped_handle_value(scope, runtime_ptr, handle).unwrap_or_else(|| {
            panic!(
                "failed to materialize handle-backed static collection handle `{handle:?}` at index `{index}`"
            )
        }));
    }

    match object.get_index(scope, index as u32) {
        Some(value) if value.is_null_or_undefined() => None,
        other => other,
    }
}

fn collection_named_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Ok((runtime_ptr, descriptor)) = live_collection_descriptor_from_object(scope, object) {
        let runtime = unsafe { &mut *runtime_ptr };
        let matches = named_item_matches(runtime, &descriptor, key);
        if descriptor.collection_kind == CollectionKind::FormControlsCollection && matches.len() > 1
        {
            let live_descriptor = LiveCollectionDescriptor {
                collection_kind: CollectionKind::RadioNodeList,
                query_kind: LiveCollectionQueryKind::FormControlsByName,
                root: descriptor.root,
                query: Some(key.to_owned()),
                include_root: false,
                tag_name_html_document: None,
                resolution_cache: Default::default(),
            };
            return Some(build_live_collection_wrapper(scope, runtime_ptr, live_descriptor).into());
        }
        let handle = matches.first().copied()?;
        return Some(
            wrapped_handle_value(scope, runtime_ptr, handle).unwrap_or_else(|| {
                panic!("failed to materialize named collection handle `{handle:?}`")
            }),
        );
    }

    let key = v8_string(scope, key)?;
    if !object.has_own_property(scope, key.into()).unwrap_or(false) {
        return None;
    }
    match object.get(scope, key.into()) {
        Some(value) if value.is_null_or_undefined() => None,
        other => other,
    }
}

pub(in crate::native_bridge::collections) fn node_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    collection_item_callback_for(scope, args, rv, is_node_list_kind);
}

pub(in crate::native_bridge::collections) fn html_collection_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    collection_item_callback_for(scope, args, rv, is_html_collection_kind);
}

fn collection_item_callback_for<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    accepts: impl FnOnce(CollectionKind) -> bool,
) {
    let object = args.this();
    let Some(kind) = collection_kind_from_object(scope, object) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if !accepts(kind) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<CollectionItemArgs>(scope, &args) else {
        return;
    };
    let Some(value) = collection_value_at(scope, object, parsed.index as usize) else {
        rv.set_null();
        return;
    };
    rv.set(value);
}

pub(in crate::native_bridge::collections) fn collection_named_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = args.this();
    let Some(kind) = collection_kind_from_object(scope, object) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if !is_html_collection_kind(kind) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(parsed) = webidl::parse_args::<CollectionNamedItemArgs>(scope, &args) else {
        return;
    };
    let Some(value) = collection_named_value(scope, object, &parsed.name) else {
        rv.set_null();
        return;
    };
    rv.set(value);
}
