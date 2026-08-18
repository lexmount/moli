use super::*;
use crate::native_bridge::element::element_attribute_for_object;
use crate::util::throw_type_error;
use moli_webapi_declare::WebApiFunctionTemplate;

pub(in crate::native_bridge) const STATIC_COLLECTION_LENGTH_SLOT: &str =
    "__lmStaticCollectionLength";
const STATIC_COLLECTION_LENGTH_INTERNAL_FIELD: usize = 2;
pub(in crate::native_bridge::collections) const STATIC_HANDLE_COLLECTION_ID_INTERNAL_FIELD: usize =
    3;
const STATIC_HANDLE_NODE_LIST_EAGER_LIMIT: usize = 1_000;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLOptionsCollection", enumerable)]
struct OptionsCollectionPrototypeDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),

    #[webapi(
        accessor_property,
        getter = options_collection_length_getter,
        setter = options_collection_length_setter
    )]
    length: (),

    #[webapi(
        accessor_property,
        getter = options_collection_selected_index_getter,
        setter = options_collection_selected_index_setter
    )]
    selected_index: (),

    #[webapi(method, length = 1, callback = options_collection_add_callback)]
    add: (),

    #[webapi(method, length = 1, callback = options_collection_remove_callback)]
    remove: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "RadioNodeList", enumerable)]
struct RadioNodeListPrototypeDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),

    #[webapi(
        accessor_property,
        getter = radio_node_list_value_getter,
        setter = radio_node_list_value_setter
    )]
    value: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLCollection")]
struct HtmlCollectionPrototypeMembersDeclaration {
    #[webapi(
        accessor_property,
        enumerable,
        getter = html_collection_length_getter
    )]
    length: (),

    #[webapi(
        method,
        length = 1,
        enumerable,
        callback = html_collection_item_callback
    )]
    item: (),

    #[webapi(
        method,
        length = 1,
        enumerable,
        callback = collection_named_item_callback
    )]
    named_item: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "NodeList")]
struct NodeListPrototypeMembersDeclaration {
    #[webapi(accessor_property, enumerable, getter = node_list_length_getter)]
    length: (),

    #[webapi(method, length = 1, callback = node_list_item_callback)]
    item: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoEntries,
        enumerable
    )]
    entries: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoKeys,
        enumerable
    )]
    keys: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        enumerable
    )]
    values: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoForEach,
        enumerable
    )]
    for_each: (),

    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLFormControlsCollection")]
struct FormControlsCollectionPrototypeDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLAllCollection")]
struct HtmlAllCollectionPrototypeDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),
}

/// Build a static NodeList wrapper from a slice of DOM handles.
pub(in crate::native_bridge) fn build_node_list_from_handles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handles: &[DomHandle],
) -> v8::Local<'s, v8::Object> {
    if handles.len() <= STATIC_HANDLE_NODE_LIST_EAGER_LIMIT {
        let values = handles
            .iter()
            .copied()
            .map(|handle| {
                let value = wrapped_handle_value(scope, runtime_ptr, handle).unwrap_or_else(|| {
                    panic!("failed to materialize static NodeList handle `{handle:?}`")
                });
                v8::Global::new(scope, value)
            })
            .collect::<Vec<_>>();
        return build_collection_wrapper(scope, runtime_ptr, &values, CollectionKind::NodeList);
    }
    build_static_handle_node_list_wrapper(scope, runtime_ptr, handles)
}

/// Build a live NodeList or HTMLCollection wrapper for a DOM node query.
pub(in crate::native_bridge) fn build_live_collection_for_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    root: DomHandle,
    collection_kind: CollectionKind,
    query_kind: LiveCollectionQueryKind,
    query: Option<String>,
    include_root: bool,
) -> v8::Local<'s, v8::Object> {
    let tag_name_html_document = (query_kind == LiveCollectionQueryKind::TagName).then(|| {
        unsafe { &*runtime_ptr }
            .dom_host()
            .node_document_is_html_document(root)
            .unwrap_or(false)
    });
    build_live_collection_wrapper(
        scope,
        runtime_ptr,
        LiveCollectionDescriptor {
            collection_kind,
            query_kind,
            root,
            query,
            include_root,
            tag_name_html_document,
            resolution_cache: Default::default(),
        },
    )
}

pub(in crate::native_bridge) fn build_live_html_children_collection_for_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    root: DomHandle,
) -> v8::Local<'s, v8::Object> {
    build_live_collection_for_node(
        scope,
        runtime_ptr,
        root,
        CollectionKind::HtmlCollection,
        LiveCollectionQueryKind::Children,
        None,
        false,
    )
}

pub(in crate::native_bridge) fn build_live_child_node_list_for_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    root: DomHandle,
) -> v8::Local<'s, v8::Object> {
    build_live_collection_for_node(
        scope,
        runtime_ptr,
        root,
        CollectionKind::NodeList,
        LiveCollectionQueryKind::ChildNodes,
        None,
        false,
    )
}

/// Build a static collection (NodeList / HTMLCollection) V8 object from pre-resolved values.
pub(in crate::native_bridge) fn build_collection_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    items: &[v8::Global<v8::Value>],
    kind: CollectionKind,
) -> v8::Local<'s, v8::Object> {
    let template = {
        let host = unsafe { &mut *runtime_ptr };
        host.native_bridge_mut().collection_wrapper_template()
    };
    let wrapper = template
        .new_instance(scope)
        .expect("failed to instantiate static collection wrapper");
    let runtime_external = v8::External::new(scope, runtime_ptr as *mut c_void);
    assert!(
        wrapper.set_internal_field(0, runtime_external.into()),
        "static collection wrapper must expose its runtime field"
    );
    let kind_tag = match kind {
        CollectionKind::NodeList => -2.0,
        CollectionKind::HtmlCollection => -3.0,
        CollectionKind::FormControlsCollection => -4.0,
        CollectionKind::OptionsCollection => -5.0,
        CollectionKind::RadioNodeList => -6.0,
    };
    assert!(
        wrapper.set_internal_field(1, v8::Number::new(scope, kind_tag).into()),
        "static collection wrapper must expose its kind field"
    );
    set_collection_prototype(scope, wrapper, kind);

    let length = items.len();
    assert!(
        wrapper.set_internal_field(
            STATIC_COLLECTION_LENGTH_INTERNAL_FIELD,
            v8::Number::new(scope, length as f64).into(),
        ),
        "static collection wrapper must expose its length field"
    );
    for (index, item_global) in items.iter().enumerate() {
        let item = v8::Local::new(scope, item_global);
        assert!(
            wrapper
                .set_index(scope, index as u32, item)
                .is_some_and(|updated| updated),
            "failed to install static collection index `{index}`"
        );
        if matches!(
            kind,
            CollectionKind::HtmlCollection
                | CollectionKind::FormControlsCollection
                | CollectionKind::OptionsCollection
        ) {
            define_html_collection_named_properties(scope, wrapper, item);
        }
    }

    set_private_value(
        scope,
        wrapper,
        STATIC_COLLECTION_LENGTH_SLOT,
        v8::Number::new(scope, length as f64).into(),
    );
    wrapper
}

fn build_static_handle_node_list_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handles: &[DomHandle],
) -> v8::Local<'s, v8::Object> {
    let template = {
        let host = unsafe { &mut *runtime_ptr };
        host.native_bridge_mut()
            .static_handle_node_list_wrapper_template()
    };
    let wrapper = template
        .new_instance(scope)
        .expect("failed to instantiate handle-backed static NodeList wrapper");
    let runtime_external = v8::External::new(scope, runtime_ptr as *mut c_void);
    assert!(
        wrapper.set_internal_field(0, runtime_external.into()),
        "handle-backed static NodeList must expose its runtime field"
    );
    assert!(
        wrapper.set_internal_field(1, v8::Number::new(scope, -2.0).into()),
        "handle-backed static NodeList must expose its kind field"
    );
    assert!(
        wrapper.set_internal_field(
            STATIC_COLLECTION_LENGTH_INTERNAL_FIELD,
            v8::Number::new(scope, handles.len() as f64).into(),
        ),
        "handle-backed static NodeList must expose its length field"
    );
    set_collection_prototype(scope, wrapper, CollectionKind::NodeList);

    let collection_id = {
        let host = unsafe { &mut *runtime_ptr };
        host.native_bridge_mut()
            .register_static_handle_collection(handles.to_vec())
    };
    assert!(
        wrapper.set_internal_field(
            STATIC_HANDLE_COLLECTION_ID_INTERNAL_FIELD,
            v8::Number::new(scope, collection_id as f64).into(),
        ),
        "handle-backed static NodeList must expose its collection id field"
    );
    set_private_value(
        scope,
        wrapper,
        STATIC_COLLECTION_LENGTH_SLOT,
        v8::Number::new(scope, handles.len() as f64).into(),
    );
    wrapper
}

/// Build a live-resolving NodeList or HTMLCollection V8 object (queries DOM on each access).
pub(in crate::native_bridge) fn build_live_collection_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    descriptor: LiveCollectionDescriptor,
) -> v8::Local<'s, v8::Object> {
    {
        let host = unsafe { &mut *runtime_ptr };
        if let Some(wrapper) = host
            .native_bridge_mut()
            .cached_live_collection_wrapper(scope, &descriptor)
        {
            return wrapper;
        }
    }

    let collection_id = {
        let host = unsafe { &mut *runtime_ptr };
        host.native_bridge_mut()
            .register_live_collection(descriptor.clone())
    };
    let template = {
        let host = unsafe { &mut *runtime_ptr };
        host.native_bridge_mut().live_collection_wrapper_template()
    };
    let wrapper = template
        .new_instance(scope)
        .expect("failed to instantiate live collection wrapper");
    let runtime_external = v8::External::new(scope, runtime_ptr as *mut c_void);
    assert!(
        wrapper.set_internal_field(0, runtime_external.into()),
        "live collection wrapper must expose its runtime field"
    );
    assert!(
        wrapper.set_internal_field(1, v8::Number::new(scope, collection_id as f64).into()),
        "live collection wrapper must expose its descriptor id field"
    );
    set_collection_prototype(scope, wrapper, descriptor.collection_kind);
    {
        let host = unsafe { &mut *runtime_ptr };
        host.native_bridge_mut()
            .cache_live_collection_wrapper(scope, descriptor, wrapper);
    }
    wrapper
}

fn define_html_collection_named_properties(
    scope: &mut v8::PinScope<'_, '_>,
    wrapper: v8::Local<'_, v8::Object>,
    value: v8::Local<'_, v8::Value>,
) {
    let Ok(item) = v8::Local::<v8::Object>::try_from(value) else {
        return;
    };

    for attribute_name in ["id", "name"] {
        let Some(key_text) = element_attribute_for_object(scope, item, attribute_name) else {
            continue;
        };
        if key_text.is_empty() {
            continue;
        }
        let Some(key) = v8_string(scope, &key_text) else {
            continue;
        };
        if wrapper
            .get(scope, key.into())
            .is_some_and(|existing| !existing.is_null_or_undefined())
        {
            continue;
        }
        let _ =
            wrapper.define_own_property(scope, key.into(), value, v8::PropertyAttribute::DONT_ENUM);
    }
}

fn set_collection_prototype(
    scope: &mut v8::PinScope<'_, '_>,
    wrapper: v8::Local<'_, v8::Object>,
    kind: CollectionKind,
) {
    let prototype_name = match kind {
        CollectionKind::NodeList => "NodeList",
        CollectionKind::HtmlCollection => "HTMLCollection",
        CollectionKind::FormControlsCollection => "HTMLFormControlsCollection",
        CollectionKind::OptionsCollection => "HTMLOptionsCollection",
        CollectionKind::RadioNodeList => "RadioNodeList",
    };
    set_named_constructor_prototype(scope, wrapper, prototype_name);
}

pub(crate) fn install_collection_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "NodeList" => {
            NodeListPrototypeMembersDeclaration::initialize_prototype_template(scope, prototype)
        }
        "HTMLCollection" => {
            HtmlCollectionPrototypeMembersDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        "HTMLFormControlsCollection" => {
            FormControlsCollectionPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        "HTMLOptionsCollection" => {
            OptionsCollectionPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "RadioNodeList" => {
            RadioNodeListPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "HTMLAllCollection" => {
            HtmlAllCollectionPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

fn node_list_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    collection_length_getter_for(scope, args, rv, is_node_list_kind);
}

fn html_collection_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    collection_length_getter_for(scope, args, rv, is_html_collection_kind);
}

fn options_collection_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    collection_length_getter_for(scope, args, rv, |kind| {
        kind == CollectionKind::OptionsCollection
    });
}

fn collection_length_getter_for<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    accepts: impl FnOnce(CollectionKind) -> bool,
) {
    let receiver = args.this();
    let Some(kind) = collection_kind_from_object(scope, receiver) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    if !accepts(kind) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    let Some(length) = native_collection_length(scope, receiver) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(v8::Number::new(scope, length as f64).into());
}

fn native_collection_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<usize> {
    if let Some(length) = static_collection_length_from_internal_field(scope, object) {
        return Some(length);
    }
    if let Ok((runtime_ptr, descriptor)) = live_collection_descriptor_from_object(scope, object) {
        return Some(descriptor.resolve(unsafe { &*runtime_ptr }).len());
    }
    get_private_value(scope, object, STATIC_COLLECTION_LENGTH_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as usize)
}

fn static_collection_length_from_internal_field<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<usize> {
    if object.internal_field_count() <= STATIC_COLLECTION_LENGTH_INTERNAL_FIELD {
        return None;
    }
    object
        .get_internal_field(scope, STATIC_COLLECTION_LENGTH_INTERNAL_FIELD)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as usize)
}
