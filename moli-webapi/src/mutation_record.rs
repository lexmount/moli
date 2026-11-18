use crate::interfaces as web_api_interfaces;
use moli_v8_util::{callback_data_index_value, callback_data_item, get_private_value};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, web_api_object_target};
const TYPE_SLOT: &str = "__moliMutationRecordType";
const TARGET_SLOT: &str = "__moliMutationRecordTarget";
const ADDED_NODES_SLOT: &str = "__moliMutationRecordAddedNodes";
const REMOVED_NODES_SLOT: &str = "__moliMutationRecordRemovedNodes";
const PREVIOUS_SIBLING_SLOT: &str = "__moliMutationRecordPreviousSibling";
const NEXT_SIBLING_SLOT: &str = "__moliMutationRecordNextSibling";
const ATTRIBUTE_NAME_SLOT: &str = "__moliMutationRecordAttributeName";
const ATTRIBUTE_NAMESPACE_SLOT: &str = "__moliMutationRecordAttributeNamespace";
const OLD_VALUE_SLOT: &str = "__moliMutationRecordOldValue";

const ATTRIBUTE_SLOTS: &[&str] = &[
    TYPE_SLOT,
    TARGET_SLOT,
    ADDED_NODES_SLOT,
    REMOVED_NODES_SLOT,
    PREVIOUS_SIBLING_SLOT,
    NEXT_SIBLING_SLOT,
    ATTRIBUTE_NAME_SLOT,
    ATTRIBUTE_NAMESPACE_SLOT,
    OLD_VALUE_SLOT,
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MutationRecord)]
pub struct MutationRecordDeclaration<'s> {
    #[webapi(slot = TYPE_SLOT)]
    pub record_type: v8::Local<'s, v8::Value>,
    #[webapi(slot = TARGET_SLOT)]
    pub target: v8::Local<'s, v8::Value>,
    #[webapi(slot = ADDED_NODES_SLOT)]
    pub added_nodes: v8::Local<'s, v8::Value>,
    #[webapi(slot = REMOVED_NODES_SLOT)]
    pub removed_nodes: v8::Local<'s, v8::Value>,
    #[webapi(slot = PREVIOUS_SIBLING_SLOT)]
    pub previous_sibling: v8::Local<'s, v8::Value>,
    #[webapi(slot = NEXT_SIBLING_SLOT)]
    pub next_sibling: v8::Local<'s, v8::Value>,
    #[webapi(slot = ATTRIBUTE_NAME_SLOT)]
    pub attribute_name: v8::Local<'s, v8::Value>,
    #[webapi(slot = ATTRIBUTE_NAMESPACE_SLOT)]
    pub attribute_namespace: v8::Local<'s, v8::Value>,
    #[webapi(slot = OLD_VALUE_SLOT)]
    pub old_value: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MutationRecord, enumerable)]
struct MutationRecordPrototypeDeclaration {
    #[webapi(accessor_property = "type", getter = attribute_getter, data = callback_data_index_value(scope, 0))]
    record_type: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 1))]
    target: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 2))]
    added_nodes: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 3))]
    removed_nodes: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 4))]
    previous_sibling: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 5))]
    next_sibling: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 6))]
    attribute_name: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 7))]
    attribute_namespace: (),
    #[webapi(accessor_property, getter = attribute_getter, data = callback_data_index_value(scope, 8))]
    old_value: (),
}

pub fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    MutationRecordPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, ATTRIBUTE_SLOTS, "MutationRecord attributes")
    else {
        return;
    };
    let target = web_api_object_target(scope, args.this())
        .expect("validated MutationRecord receiver has native backing");
    rv.set(
        get_private_value(scope, target, slot)
            .expect("initialized MutationRecord retains its private attributes"),
    );
}
