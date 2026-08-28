use super::super::*;
use super::types::*;
use crate::native_bridge::document::detached_tree_query_version;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_xpath::SnapshotValue;
use std::ffi::c_void;

#[derive(Clone, Copy)]
pub(super) enum XPathIteratorMutationState<'s> {
    Live {
        runtime_ptr: *mut JsContextHost,
        query_version: u64,
    },
    ObjectTree {
        root: v8::Local<'s, v8::Object>,
        query_version: u64,
    },
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XPathResult.snapshotItem")]
struct XPathResultSnapshotItemArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(WebApiObject)]
#[webapi(interface = "XPathResult")]
struct XPathResultBaseDeclaration<'scope> {
    #[webapi(slot = XPATH_RESULT_TYPE_SLOT)]
    result_type: u32,
    #[webapi(slot = XPATH_RESULT_STRING_VALUE_SLOT, constructor_default = "")]
    string_value: &'static str,
    #[webapi(slot = XPATH_RESULT_NUMBER_VALUE_SLOT, constructor_default = 0.0)]
    number_value: f64,
    #[webapi(slot = XPATH_RESULT_BOOLEAN_VALUE_SLOT, constructor_default = false)]
    boolean_value: bool,
    #[webapi(slot = XPATH_RESULT_SINGLE_NODE_VALUE_SLOT, init = "null")]
    _single_node_value: (),
    #[webapi(slot = XPATH_RESULT_SNAPSHOT_LENGTH_SLOT, constructor_default = 0)]
    snapshot_length: u32,
    #[webapi(slot = XPATH_RESULT_INDEX_SLOT, constructor_default = 0)]
    index: i32,
    #[webapi(slot = XPATH_RESULT_NODES_SLOT, constructor_default = Vec::new())]
    nodes: Vec<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "XPathResult", enumerable)]
struct XPathResultPrototypeDeclaration {
    #[webapi(accessor_property, getter = xpath_result_result_type_getter_callback)]
    result_type: (),
    #[webapi(accessor_property, getter = xpath_result_invalid_iterator_state_getter_callback)]
    invalid_iterator_state: (),
    #[webapi(accessor_property, getter = xpath_result_string_value_getter_callback)]
    string_value: (),
    #[webapi(accessor_property, getter = xpath_result_number_value_getter_callback)]
    number_value: (),
    #[webapi(accessor_property, getter = xpath_result_boolean_value_getter_callback)]
    boolean_value: (),
    #[webapi(accessor_property, getter = xpath_result_single_node_value_getter_callback)]
    single_node_value: (),
    #[webapi(accessor_property, getter = xpath_result_snapshot_length_getter_callback)]
    snapshot_length: (),
    #[webapi(method, length = 0, callback = xpath_result_iterate_next_callback)]
    iterate_next: (),
    #[webapi(method, length = 0, callback = xpath_result_snapshot_item_callback)]
    snapshot_item: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "XPathResult")]
struct XPathResultConstantsDeclaration {
    #[webapi(constant = "ANY_TYPE", value = XPATH_ANY_TYPE)]
    any_type: (),

    #[webapi(constant = "NUMBER_TYPE", value = XPATH_NUMBER_TYPE)]
    number_type: (),

    #[webapi(constant = "STRING_TYPE", value = XPATH_STRING_TYPE)]
    string_type: (),

    #[webapi(constant = "BOOLEAN_TYPE", value = XPATH_BOOLEAN_TYPE)]
    boolean_type: (),

    #[webapi(
        constant = "UNORDERED_NODE_ITERATOR_TYPE",
        value = XPATH_UNORDERED_NODE_ITERATOR_TYPE
    )]
    unordered_node_iterator_type: (),

    #[webapi(
        constant = "ORDERED_NODE_ITERATOR_TYPE",
        value = XPATH_ORDERED_NODE_ITERATOR_TYPE
    )]
    ordered_node_iterator_type: (),

    #[webapi(
        constant = "UNORDERED_NODE_SNAPSHOT_TYPE",
        value = XPATH_UNORDERED_NODE_SNAPSHOT_TYPE
    )]
    unordered_node_snapshot_type: (),

    #[webapi(
        constant = "ORDERED_NODE_SNAPSHOT_TYPE",
        value = XPATH_ORDERED_NODE_SNAPSHOT_TYPE
    )]
    ordered_node_snapshot_type: (),

    #[webapi(
        constant = "ANY_UNORDERED_NODE_TYPE",
        value = XPATH_ANY_UNORDERED_NODE_TYPE
    )]
    any_unordered_node_type: (),

    #[webapi(
        constant = "FIRST_ORDERED_NODE_TYPE",
        value = XPATH_FIRST_ORDERED_NODE_TYPE
    )]
    first_ordered_node_type: (),
}

pub(super) fn install_xpath_result_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    XPathResultConstantsDeclaration::initialize_template(scope, template);
    XPathResultConstantsDeclaration::initialize_prototype_template(scope, prototype);
    XPathResultPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn set_xpath_result_slot(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, object, name, value);
}

fn build_xpath_result_base<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result_type: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    XPathResultBaseDeclaration::new(result_type)
        .bind(scope)
        .ok()
}

pub(super) fn build_xpath_nodes_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    nodes: &[v8::Local<'s, v8::Object>],
    requested_result_type: u32,
    iterator_mutation_state: Option<XPathIteratorMutationState<'s>>,
) -> Option<v8::Local<'s, v8::Object>> {
    if requested_result_type == XPATH_BOOLEAN_TYPE {
        return build_xpath_scalar_result(
            scope,
            SnapshotValue::Boolean(!nodes.is_empty()),
            requested_result_type,
        );
    }
    let result_type = match requested_result_type {
        XPATH_ANY_TYPE => XPATH_UNORDERED_NODE_ITERATOR_TYPE,
        XPATH_UNORDERED_NODE_ITERATOR_TYPE
        | XPATH_ORDERED_NODE_ITERATOR_TYPE
        | XPATH_UNORDERED_NODE_SNAPSHOT_TYPE
        | XPATH_ORDERED_NODE_SNAPSHOT_TYPE
        | XPATH_ANY_UNORDERED_NODE_TYPE
        | XPATH_FIRST_ORDERED_NODE_TYPE => requested_result_type,
        _ => XPATH_UNORDERED_NODE_ITERATOR_TYPE,
    };
    let object = build_xpath_result_base(scope, result_type)?;
    let node_array = build_object_array(scope, nodes);
    set_xpath_result_slot(scope, object, XPATH_RESULT_NODES_SLOT, node_array.into());
    set_xpath_result_slot(
        scope,
        object,
        XPATH_RESULT_SNAPSHOT_LENGTH_SLOT,
        v8::Integer::new(scope, nodes.len() as i32).into(),
    );
    if xpath_result_type_is_iterator(result_type)
        && let Some(iterator_mutation_state) = iterator_mutation_state
    {
        match iterator_mutation_state {
            XPathIteratorMutationState::Live {
                runtime_ptr,
                query_version,
            } => {
                let external = v8::External::new(scope, runtime_ptr as *mut c_void);
                set_xpath_result_slot(scope, object, XPATH_RESULT_RUNTIME_SLOT, external.into());
                set_xpath_result_slot(
                    scope,
                    object,
                    XPATH_RESULT_QUERY_VERSION_SLOT,
                    v8::BigInt::new_from_u64(scope, query_version).into(),
                );
            }
            XPathIteratorMutationState::ObjectTree {
                root,
                query_version,
            } => {
                set_xpath_result_slot(
                    scope,
                    object,
                    XPATH_RESULT_OBJECT_TREE_ROOT_SLOT,
                    root.into(),
                );
                set_xpath_result_slot(
                    scope,
                    object,
                    XPATH_RESULT_QUERY_VERSION_SLOT,
                    v8::BigInt::new_from_u64(scope, query_version).into(),
                );
            }
        }
    }
    if let Some(first) = nodes.first() {
        set_xpath_result_slot(
            scope,
            object,
            XPATH_RESULT_SINGLE_NODE_VALUE_SLOT,
            (*first).into(),
        );
    }
    Some(object)
}

fn xpath_result_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: v8::Local<'s, v8::Object>,
) -> u32 {
    get_private_value(scope, result, XPATH_RESULT_TYPE_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(XPATH_ANY_TYPE)
}

fn xpath_result_type_is_iterator(result_type: u32) -> bool {
    matches!(
        result_type,
        XPATH_UNORDERED_NODE_ITERATOR_TYPE | XPATH_ORDERED_NODE_ITERATOR_TYPE
    )
}

fn xpath_result_type_is_snapshot(result_type: u32) -> bool {
    matches!(
        result_type,
        XPATH_UNORDERED_NODE_SNAPSHOT_TYPE | XPATH_ORDERED_NODE_SNAPSHOT_TYPE
    )
}

fn xpath_result_type_is_single_node(result_type: u32) -> bool {
    matches!(
        result_type,
        XPATH_ANY_UNORDERED_NODE_TYPE | XPATH_FIRST_ORDERED_NODE_TYPE
    )
}

fn xpath_result_throw_type_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    throw_type_error(scope, message);
}

fn xpath_result_baseline_query_version<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let value = get_private_value(scope, result, XPATH_RESULT_QUERY_VERSION_SLOT)?;
    let big = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (version, _lossless) = big.u64_value();
    Some(version)
}

fn xpath_result_runtime_ptr<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: v8::Local<'s, v8::Object>,
) -> Option<*mut JsContextHost> {
    if crate::util::page_context_is_disconnected(scope.get_current_context())
        || result
            .get_creation_context(scope)
            .is_some_and(crate::util::page_context_is_disconnected)
    {
        return None;
    }
    let value = get_private_value(scope, result, XPATH_RESULT_RUNTIME_SLOT)?;
    let external = v8::Local::<v8::External>::try_from(value).ok()?;
    let runtime_ptr = external.value() as *mut JsContextHost;
    (!runtime_ptr.is_null()).then_some(runtime_ptr)
}

fn xpath_result_object_tree_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, result, XPATH_RESULT_OBJECT_TREE_ROOT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn xpath_result_iterator_invalidated<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: v8::Local<'s, v8::Object>,
) -> bool {
    if !xpath_result_type_is_iterator(xpath_result_type(scope, result)) {
        return false;
    }
    let Some(baseline) = xpath_result_baseline_query_version(scope, result) else {
        return false;
    };
    if let Some(runtime_ptr) = xpath_result_runtime_ptr(scope, result) {
        let current = unsafe { &*runtime_ptr }.dom_host().query_version();
        return current != baseline;
    }
    if let Some(root) = xpath_result_object_tree_root(scope, result) {
        return detached_tree_query_version(scope, root).is_none_or(|current| current != baseline);
    }
    false
}

fn xpath_result_invalid_iterator_state_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let invalidated = xpath_result_iterator_invalidated(scope, args.this());
    rv.set(v8::Boolean::new(scope, invalidated).into());
}

fn xpath_result_result_type_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let result_type = xpath_result_type(scope, args.this());
    rv.set(v8::Integer::new_from_unsigned(scope, result_type).into());
}

fn xpath_result_number_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let result = args.this();
    if xpath_result_type(scope, result) != XPATH_NUMBER_TYPE {
        xpath_result_throw_type_error(scope, "The result type is not a number.");
        return;
    }
    if let Some(value) = get_private_value(scope, result, XPATH_RESULT_NUMBER_VALUE_SLOT) {
        rv.set(value);
    }
}

fn xpath_result_string_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let result = args.this();
    if xpath_result_type(scope, result) != XPATH_STRING_TYPE {
        xpath_result_throw_type_error(scope, "The result type is not a string.");
        return;
    }
    if let Some(value) = get_private_value(scope, result, XPATH_RESULT_STRING_VALUE_SLOT) {
        rv.set(value);
    }
}

fn xpath_result_boolean_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let result = args.this();
    if xpath_result_type(scope, result) != XPATH_BOOLEAN_TYPE {
        xpath_result_throw_type_error(scope, "The result type is not a boolean.");
        return;
    }
    if let Some(value) = get_private_value(scope, result, XPATH_RESULT_BOOLEAN_VALUE_SLOT) {
        rv.set(value);
    }
}

fn xpath_result_single_node_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let result = args.this();
    if !xpath_result_type_is_single_node(xpath_result_type(scope, result)) {
        xpath_result_throw_type_error(scope, "The result type is not a single node.");
        return;
    }
    if let Some(value) = get_private_value(scope, result, XPATH_RESULT_SINGLE_NODE_VALUE_SLOT) {
        rv.set(value);
    }
}

fn xpath_result_snapshot_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let result = args.this();
    if !xpath_result_type_is_snapshot(xpath_result_type(scope, result)) {
        xpath_result_throw_type_error(scope, "The result type is not a snapshot.");
        return;
    }
    if let Some(value) = get_private_value(scope, result, XPATH_RESULT_SNAPSHOT_LENGTH_SLOT) {
        rv.set(value);
    }
}

pub(super) fn build_xpath_scalar_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: SnapshotValue,
    requested_result_type: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let result_type = match requested_result_type {
        XPATH_ANY_TYPE | XPATH_STRING_TYPE => XPATH_STRING_TYPE,
        XPATH_NUMBER_TYPE => XPATH_NUMBER_TYPE,
        XPATH_BOOLEAN_TYPE => XPATH_BOOLEAN_TYPE,
        _ => XPATH_STRING_TYPE,
    };
    let object = build_xpath_result_base(scope, result_type)?;
    match result_type {
        XPATH_NUMBER_TYPE => {
            set_xpath_result_slot(
                scope,
                object,
                XPATH_RESULT_NUMBER_VALUE_SLOT,
                v8::Number::new(scope, xpath_value_number(&value)).into(),
            );
        }
        XPATH_BOOLEAN_TYPE => {
            set_xpath_result_slot(
                scope,
                object,
                XPATH_RESULT_BOOLEAN_VALUE_SLOT,
                v8::Boolean::new(scope, xpath_value_boolean(&value)).into(),
            );
        }
        _ => {
            let string = v8_string(scope, &xpath_value_string(&value))?;
            set_xpath_result_slot(scope, object, XPATH_RESULT_STRING_VALUE_SLOT, string.into());
        }
    }
    Some(object)
}

fn xpath_value_boolean(value: &SnapshotValue) -> bool {
    match value {
        SnapshotValue::Boolean(value) => *value,
        SnapshotValue::Number(value) => *value != 0.0 && !value.is_nan(),
        SnapshotValue::String(value) => !value.is_empty(),
        SnapshotValue::Nodes(value) => !value.is_empty(),
    }
}

fn xpath_value_number(value: &SnapshotValue) -> f64 {
    match value {
        SnapshotValue::Boolean(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        SnapshotValue::Number(value) => *value,
        SnapshotValue::String(value) => value.trim_ascii().parse().unwrap_or(f64::NAN),
        SnapshotValue::Nodes(_) => xpath_value_string(value)
            .trim_ascii()
            .parse()
            .unwrap_or(f64::NAN),
    }
}

fn xpath_value_string(value: &SnapshotValue) -> String {
    match value {
        SnapshotValue::Boolean(value) => value.to_string(),
        SnapshotValue::Number(value) => {
            if value.is_infinite() {
                if value.is_sign_negative() {
                    "-Infinity".to_owned()
                } else {
                    "Infinity".to_owned()
                }
            } else if *value == 0.0 {
                "0".to_owned()
            } else {
                value.to_string()
            }
        }
        SnapshotValue::String(value) => value.clone(),
        SnapshotValue::Nodes(_) => String::new(),
    }
}

fn xpath_result_iterate_next_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let result = args.this();
    if !xpath_result_type_is_iterator(xpath_result_type(scope, result)) {
        xpath_result_throw_type_error(scope, "The result type is not an iterator.");
        return;
    }
    if xpath_result_iterator_invalidated(scope, result) {
        throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "The document has mutated since this XPath iterator was created.",
        );
        return;
    }
    let Some(nodes) = get_private_value(scope, result, XPATH_RESULT_NODES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        rv.set_null();
        return;
    };
    let index = get_private_value(scope, result, XPATH_RESULT_INDEX_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    if index >= nodes.length() {
        rv.set_null();
        return;
    }
    let value = nodes
        .get_index(scope, index)
        .unwrap_or_else(|| v8::null(scope).into());
    set_xpath_result_slot(
        scope,
        result,
        XPATH_RESULT_INDEX_SLOT,
        v8::Integer::new_from_unsigned(scope, index + 1).into(),
    );
    rv.set(value);
}

fn xpath_result_snapshot_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let result = args.this();
    if !xpath_result_type_is_snapshot(xpath_result_type(scope, result)) {
        xpath_result_throw_type_error(scope, "The result type is not a snapshot.");
        return;
    }
    let Some(nodes) = get_private_value(scope, result, XPATH_RESULT_NODES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<XPathResultSnapshotItemArgs>(scope, &args) else {
        return;
    };
    if parsed.index >= nodes.length() {
        rv.set_null();
        return;
    }
    let value = nodes
        .get_index(scope, parsed.index)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

pub(super) fn is_supported_xpath_result_type(result_type: u32) -> bool {
    matches!(
        result_type,
        XPATH_ANY_TYPE
            | XPATH_NUMBER_TYPE
            | XPATH_STRING_TYPE
            | XPATH_BOOLEAN_TYPE
            | XPATH_UNORDERED_NODE_ITERATOR_TYPE
            | XPATH_ORDERED_NODE_ITERATOR_TYPE
            | XPATH_UNORDERED_NODE_SNAPSHOT_TYPE
            | XPATH_ORDERED_NODE_SNAPSHOT_TYPE
            | XPATH_ANY_UNORDERED_NODE_TYPE
            | XPATH_FIRST_ORDERED_NODE_TYPE
    )
}
