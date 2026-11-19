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

use moli_webidl as webidl;
use std::collections::HashSet;

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "MutationObserverInit")]
pub struct MutationObserverInit {
    #[webidl(sequence, converter = "raw")]
    attribute_filter: Option<Vec<webidl::DomString16>>,
    attribute_old_value: Option<bool>,
    attributes: Option<bool>,
    character_data: Option<bool>,
    character_data_old_value: Option<bool>,
    #[webidl(default = false)]
    child_list: bool,
    #[webidl(default = false)]
    subtree: bool,
}

#[derive(Clone, Default)]
pub struct MutationObserverOptions {
    pub child_list: bool,
    pub attributes: bool,
    pub character_data: bool,
    pub subtree: bool,
    pub attribute_old_value: bool,
    pub character_data_old_value: bool,
    pub attribute_filter: Option<HashSet<String>>,
}

impl TryFrom<MutationObserverInit> for MutationObserverOptions {
    type Error = &'static str;

    fn try_from(init: MutationObserverInit) -> Result<Self, Self::Error> {
        let attributes = init
            .attributes
            .unwrap_or(init.attribute_old_value.is_some() || init.attribute_filter.is_some());
        let character_data = init
            .character_data
            .unwrap_or(init.character_data_old_value.is_some());
        let attribute_old_value = init.attribute_old_value.unwrap_or(false);
        let character_data_old_value = init.character_data_old_value.unwrap_or(false);

        if !init.child_list && !attributes && !character_data {
            return Err(
                "Failed to execute 'observe' on 'MutationObserver': at least one of childList, attributes, or characterData must be true.",
            );
        }
        if !attributes && (attribute_old_value || init.attribute_filter.is_some()) {
            return Err(
                "Failed to execute 'observe' on 'MutationObserver': attributeOldValue and attributeFilter require attributes to be true.",
            );
        }
        if !character_data && character_data_old_value {
            return Err(
                "Failed to execute 'observe' on 'MutationObserver': characterDataOldValue requires characterData to be true.",
            );
        }

        Ok(Self {
            child_list: init.child_list,
            attributes,
            character_data,
            subtree: init.subtree,
            attribute_old_value,
            character_data_old_value,
            // Native mutation local names are UTF-8. Ill-formed UTF-16 entries
            // cannot match them; retain the filter even if no entries can match.
            attribute_filter: init.attribute_filter.map(|names| {
                names
                    .into_iter()
                    .filter_map(|name| String::from_utf16(&name.0).ok())
                    .collect()
            }),
        })
    }
}
