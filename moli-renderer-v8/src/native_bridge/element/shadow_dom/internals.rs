use crate::{
    context_bootstrap::exposed_interfaces::ensure_intrinsic_interface_prototype,
    custom_elements,
    document_runtime::DomHandle,
    dom::native::Node,
    native_bridge::{
        JsContextHost, collections,
        identity::{CollectionKind, LiveCollectionQueryKind},
    },
    snapshot_form_data_value,
    util::{
        get_private_object, get_private_value, set_private_value, string_from_utf16_units_lossy,
        throw_type_error, v8_string, v8_value_to_dom_string_u16, v8str,
    },
    webidl_iterator::{
        SetlikeWebIdlIteratorKind, SetlikeWebIdlIteratorMethod, call_setlike_webidl_for_each,
        new_setlike_webidl_iterator,
    },
};
use dom::ElementState as StyloElementState;
use moli_dom::forms::FormControlValidity;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use super::super::super::{
    document::XHTML_NS,
    node::{node_runtime_and_handle_from_object, node_runtime_and_handle_from_object_or_detached},
    throw_dom_exception,
};
use super::super::form_associated_form_owner;
use super::super::forms::{dispatch_invalid_event, form_control_is_effectively_disabled};
use super::super::reflection::property_dom_string_value;

const ELEMENT_INTERNALS_SLOT: &str = "__moliElementInternals";
const ELEMENT_INTERNALS_TARGET_SLOT: &str = "__moliElementInternalsTarget";
const ELEMENT_INTERNALS_STATES_SLOT: &str = "__moliElementInternalsStates";
const ELEMENT_INTERNALS_STATES_TARGET_SLOT: &str = "__moliElementInternalsStatesTarget";
const CUSTOM_STATE_SET_BACKING_SLOT: &str = "__moliCustomStateSetBacking";
const ELEMENT_INTERNALS_FORM_VALUE_SLOT: &str = "__moliElementInternalsFormValue";
const ELEMENT_INTERNALS_VALIDITY_SLOT: &str = "__moliElementInternalsValidity";
const ELEMENT_INTERNALS_VALIDATION_MESSAGE_SLOT: &str = "__moliElementInternalsValidationMessage";
#[derive(WebApiObject)]
#[webapi(interface = "ElementInternals")]
struct ElementInternalsDeclaration<'scope> {
    #[webapi(slot = ELEMENT_INTERNALS_TARGET_SLOT)]
    target: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    name = "ElementInternals",
    enumerable,
    prototype_to_string_tag = "ElementInternals"
)]
struct ElementInternalsCorePrototypeDeclaration {
    #[webapi(accessor_property, getter = element_internals_shadow_root_getter_callback)]
    shadow_root: (),
    #[webapi(accessor_property, getter = element_internals_states_getter_callback)]
    states: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ElementInternals", enumerable)]
struct ElementInternalsAccessibilityPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "role")
    )]
    role: (),
    #[webapi(
        accessor_property = "ariaAtomic",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaAtomic")
    )]
    aria_atomic: (),
    #[webapi(
        accessor_property = "ariaAutoComplete",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaAutoComplete")
    )]
    aria_auto_complete: (),
    #[webapi(
        accessor_property = "ariaBrailleLabel",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaBrailleLabel")
    )]
    aria_braille_label: (),
    #[webapi(
        accessor_property = "ariaBrailleRoleDescription",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaBrailleRoleDescription")
    )]
    aria_braille_role_description: (),
    #[webapi(
        accessor_property = "ariaBusy",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaBusy")
    )]
    aria_busy: (),
    #[webapi(
        accessor_property = "ariaChecked",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaChecked")
    )]
    aria_checked: (),
    #[webapi(
        accessor_property = "ariaColCount",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaColCount")
    )]
    aria_col_count: (),
    #[webapi(
        accessor_property = "ariaColIndex",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaColIndex")
    )]
    aria_col_index: (),
    #[webapi(
        accessor_property = "ariaColSpan",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaColSpan")
    )]
    aria_col_span: (),
    #[webapi(
        accessor_property = "ariaCurrent",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaCurrent")
    )]
    aria_current: (),
    #[webapi(
        accessor_property = "ariaDisabled",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaDisabled")
    )]
    aria_disabled: (),
    #[webapi(
        accessor_property = "ariaExpanded",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaExpanded")
    )]
    aria_expanded: (),
    #[webapi(
        accessor_property = "ariaHasPopup",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaHasPopup")
    )]
    aria_has_popup: (),
    #[webapi(
        accessor_property = "ariaHidden",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaHidden")
    )]
    aria_hidden: (),
    #[webapi(
        accessor_property = "ariaInvalid",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaInvalid")
    )]
    aria_invalid: (),
    #[webapi(
        accessor_property = "ariaKeyShortcuts",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaKeyShortcuts")
    )]
    aria_key_shortcuts: (),
    #[webapi(
        accessor_property = "ariaLabel",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaLabel")
    )]
    aria_label: (),
    #[webapi(
        accessor_property = "ariaLevel",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaLevel")
    )]
    aria_level: (),
    #[webapi(
        accessor_property = "ariaLive",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaLive")
    )]
    aria_live: (),
    #[webapi(
        accessor_property = "ariaModal",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaModal")
    )]
    aria_modal: (),
    #[webapi(
        accessor_property = "ariaMultiLine",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaMultiLine")
    )]
    aria_multi_line: (),
    #[webapi(
        accessor_property = "ariaMultiSelectable",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaMultiSelectable")
    )]
    aria_multi_selectable: (),
    #[webapi(
        accessor_property = "ariaOrientation",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaOrientation")
    )]
    aria_orientation: (),
    #[webapi(
        accessor_property = "ariaPlaceholder",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaPlaceholder")
    )]
    aria_placeholder: (),
    #[webapi(
        accessor_property = "ariaPosInSet",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaPosInSet")
    )]
    aria_pos_in_set: (),
    #[webapi(
        accessor_property = "ariaPressed",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaPressed")
    )]
    aria_pressed: (),
    #[webapi(
        accessor_property = "ariaReadOnly",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaReadOnly")
    )]
    aria_read_only: (),
    #[webapi(
        accessor_property = "ariaRelevant",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaRelevant")
    )]
    aria_relevant: (),
    #[webapi(
        accessor_property = "ariaRequired",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaRequired")
    )]
    aria_required: (),
    #[webapi(
        accessor_property = "ariaRoleDescription",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaRoleDescription")
    )]
    aria_role_description: (),
    #[webapi(
        accessor_property = "ariaRowCount",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaRowCount")
    )]
    aria_row_count: (),
    #[webapi(
        accessor_property = "ariaRowIndex",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaRowIndex")
    )]
    aria_row_index: (),
    #[webapi(
        accessor_property = "ariaRowSpan",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaRowSpan")
    )]
    aria_row_span: (),
    #[webapi(
        accessor_property = "ariaSelected",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaSelected")
    )]
    aria_selected: (),
    #[webapi(
        accessor_property = "ariaSetSize",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaSetSize")
    )]
    aria_set_size: (),
    #[webapi(
        accessor_property = "ariaSort",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaSort")
    )]
    aria_sort: (),
    #[webapi(
        accessor_property = "ariaValueMax",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaValueMax")
    )]
    aria_value_max: (),
    #[webapi(
        accessor_property = "ariaValueMin",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaValueMin")
    )]
    aria_value_min: (),
    #[webapi(
        accessor_property = "ariaValueNow",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaValueNow")
    )]
    aria_value_now: (),
    #[webapi(
        accessor_property = "ariaValueText",
        getter = element_internals_value_getter_callback,
        setter = element_internals_value_setter_callback,
        data = v8str(scope, "ariaValueText")
    )]
    aria_value_text: (),
    #[webapi(
        accessor_property = "ariaActiveDescendantElement",
        getter = element_internals_value_getter_callback,
        setter = element_internals_element_reference_value_setter_callback,
        data = v8str(scope, "ariaActiveDescendantElement")
    )]
    aria_active_descendant_element: (),
    #[webapi(
        accessor_property = "ariaControlsElements",
        getter = element_internals_value_getter_callback,
        setter = element_internals_element_reference_value_setter_callback,
        data = v8str(scope, "ariaControlsElements")
    )]
    aria_controls_elements: (),
    #[webapi(
        accessor_property = "ariaDescribedByElements",
        getter = element_internals_value_getter_callback,
        setter = element_internals_element_reference_value_setter_callback,
        data = v8str(scope, "ariaDescribedByElements")
    )]
    aria_described_by_elements: (),
    #[webapi(
        accessor_property = "ariaDetailsElements",
        getter = element_internals_value_getter_callback,
        setter = element_internals_element_reference_value_setter_callback,
        data = v8str(scope, "ariaDetailsElements")
    )]
    aria_details_elements: (),
    #[webapi(
        accessor_property = "ariaErrorMessageElements",
        getter = element_internals_value_getter_callback,
        setter = element_internals_element_reference_value_setter_callback,
        data = v8str(scope, "ariaErrorMessageElements")
    )]
    aria_error_message_elements: (),
    #[webapi(
        accessor_property = "ariaFlowToElements",
        getter = element_internals_value_getter_callback,
        setter = element_internals_element_reference_value_setter_callback,
        data = v8str(scope, "ariaFlowToElements")
    )]
    aria_flow_to_elements: (),
    #[webapi(
        accessor_property = "ariaLabelledByElements",
        getter = element_internals_value_getter_callback,
        setter = element_internals_element_reference_value_setter_callback,
        data = v8str(scope, "ariaLabelledByElements")
    )]
    aria_labelled_by_elements: (),
    #[webapi(
        accessor_property = "ariaOwnsElements",
        getter = element_internals_value_getter_callback,
        setter = element_internals_element_reference_value_setter_callback,
        data = v8str(scope, "ariaOwnsElements")
    )]
    aria_owns_elements: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "ElementInternals", enumerable)]
struct ElementInternalsFormPrototypeDeclaration {
    #[webapi(method, length = 1, callback = element_internals_set_form_value_callback)]
    set_form_value: (),
    #[webapi(method, length = 1, callback = element_internals_set_validity_callback)]
    set_validity: (),
    #[webapi(method, length = 0, callback = element_internals_check_validity_callback)]
    check_validity: (),
    #[webapi(method, length = 0, callback = element_internals_check_validity_callback)]
    report_validity: (),
    #[webapi(accessor_property, getter = element_internals_form_getter_callback)]
    form: (),
    #[webapi(accessor_property, getter = element_internals_will_validate_getter_callback)]
    will_validate: (),
    #[webapi(accessor_property, getter = element_internals_validity_getter_callback)]
    validity: (),
    #[webapi(accessor_property, getter = element_internals_validation_message_getter_callback)]
    validation_message: (),
    #[webapi(accessor_property, getter = element_internals_labels_getter_callback)]
    labels: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "ValidityState", data_properties, enumerable)]
struct ValidityStateDeclaration {
    value_missing: bool,
    type_mismatch: bool,
    pattern_mismatch: bool,
    too_long: bool,
    too_short: bool,
    range_underflow: bool,
    range_overflow: bool,
    step_mismatch: bool,
    bad_input: bool,
    custom_error: bool,
    valid: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "ValidityState", data_properties, enumerable)]
struct ValidityStateFlagsUpdateDeclaration {
    value_missing: bool,
    type_mismatch: bool,
    pattern_mismatch: bool,
    too_long: bool,
    too_short: bool,
    range_underflow: bool,
    range_overflow: bool,
    step_mismatch: bool,
    bad_input: bool,
    custom_error: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "ValidityState", data_properties, enumerable)]
struct ValidityStateValidUpdateDeclaration {
    valid: bool,
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = "CustomStateSet")]
struct CustomStateSetObjectDeclaration<'s> {
    #[webapi(slot = CUSTOM_STATE_SET_BACKING_SLOT)]
    backing: v8::Local<'s, v8::Set>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CustomStateSet", enumerable)]
struct CustomStateSetPrototypeDeclaration {
    #[webapi(accessor_property, getter = custom_state_set_size_getter)]
    size: (),
    #[webapi(method, length = 1, callback = custom_state_set_add_callback)]
    add: (),
    #[webapi(method, length = 1, callback = custom_state_set_delete_callback)]
    delete: (),
    #[webapi(method, length = 0, callback = custom_state_set_clear_callback)]
    clear: (),
    #[webapi(method, length = 1, callback = custom_state_set_has_callback)]
    has: (),
    #[webapi(method, length = 0, callback = custom_state_set_entries_callback)]
    entries: (),
    #[webapi(method, length = 0, callback = custom_state_set_values_callback)]
    keys: (),
    #[webapi(method, length = 0, callback = custom_state_set_values_callback)]
    values: (),
    #[webapi(method, length = 1, callback = custom_state_set_for_each_callback)]
    for_each: (),
    #[webapi(alias = "values", symbol = "iterator")]
    iterator: (),
}

pub(crate) fn install_element_internals_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "ElementInternals" {
        let prototype = template.prototype_template(scope);
        ElementInternalsCorePrototypeDeclaration::initialize_prototype_template(scope, prototype);
        ElementInternalsAccessibilityPrototypeDeclaration::initialize_prototype_template(
            scope, prototype,
        );
        ElementInternalsFormPrototypeDeclaration::initialize_prototype_template(scope, prototype);
    }
    if interface_name == "CustomStateSet" {
        CustomStateSetPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

pub(in crate::native_bridge) fn element_attach_internals_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_undefined();
        return;
    };
    let target = args.this();
    if get_private_value(scope, target, ELEMENT_INTERNALS_SLOT).is_some() {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "ElementInternals has already been attached.",
        );
        return;
    }
    if !unsafe { &*runtime_ptr }
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.definition_allows_internals_for_handle(runtime_ptr, handle))
    {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "ElementInternals cannot be attached to this element.",
        );
        return;
    }
    let internals = ElementInternalsDeclaration { target }
        .bind(scope)
        .expect("ElementInternals declaration should bind");
    set_private_value(scope, target, ELEMENT_INTERNALS_SLOT, internals.into());
    rv.set(internals.into());
}

fn custom_state_set_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = ensure_intrinsic_interface_prototype(scope, "CustomStateSet").ok()?;
    let object = CustomStateSetObjectDeclaration::new(v8::Set::new(scope))
        .bind(scope)
        .ok()?;
    (object.set_prototype(scope, prototype.into()) == Some(true)).then_some(object)
}

fn initialize_custom_state_set_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    states: v8::Local<'s, v8::Object>,
    target: Option<v8::Local<'s, v8::Object>>,
) {
    if let Some(target) = target {
        set_private_value(
            scope,
            states,
            ELEMENT_INTERNALS_STATES_TARGET_SLOT,
            target.into(),
        );
        let Some(backing) = custom_state_set_backing(scope, states) else {
            return;
        };
        if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, target) {
            for state in unsafe { &*runtime_ptr }
                .dom_host()
                .custom_state_names(handle)
            {
                if let Some(value) = v8_string(scope, &state) {
                    let _ = backing.add(scope, value.into());
                }
            }
        }
    }
}

fn internals_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    internals: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, internals, ELEMENT_INTERNALS_TARGET_SLOT)
}

pub(crate) fn element_internals_form_value_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let internals = get_private_object(scope, target, ELEMENT_INTERNALS_SLOT)?;
    get_private_value(scope, internals, ELEMENT_INTERNALS_FORM_VALUE_SLOT)
}

pub(crate) fn element_internals_validity_for_target_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> FormControlValidity {
    let Some(internals) = element_internals_for_target_handle(scope, runtime_ptr, handle) else {
        return FormControlValidity::default();
    };
    validity_state_from_internals(scope, internals)
}

pub(crate) fn element_internals_validation_message_for_target_handle(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> String {
    let Some(internals) = element_internals_for_target_handle(scope, runtime_ptr, handle) else {
        return String::new();
    };
    get_private_value(scope, internals, ELEMENT_INTERNALS_VALIDATION_MESSAGE_SLOT)
        .and_then(|message| message.to_string(scope))
        .map(|message| message.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

pub(crate) fn element_internals_will_validate_for_handle(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> bool {
    internals_will_validate(runtime, handle)
}

fn element_internals_for_target_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, handle)?;
    get_private_object(scope, target, ELEMENT_INTERNALS_SLOT)
}

fn element_internals_shadow_root_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(target) = internals_target(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, target)
    else {
        rv.set_null();
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(root_handle) = runtime.dom_host().shadow_root_handle(handle) else {
        rv.set_null();
        return;
    };
    if !runtime
        .dom_host()
        .shadow_root_available_to_element_internals(root_handle)
        .unwrap_or(false)
    {
        rv.set_null();
        return;
    }
    let shadow_root = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, root_handle)
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(shadow_root);
}

fn element_internals_states_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let internals = args.this();
    if let Some(states) = get_private_object(scope, internals, ELEMENT_INTERNALS_STATES_SLOT) {
        rv.set(states.into());
        return;
    }
    let target = internals_target(scope, internals);
    let Some(states) = custom_state_set_object(scope) else {
        throw_type_error(scope, "Failed to create CustomStateSet");
        return;
    };
    initialize_custom_state_set_object(scope, states, target);
    set_private_value(
        scope,
        internals,
        ELEMENT_INTERNALS_STATES_SLOT,
        states.into(),
    );
    rv.set(states.into());
}

fn custom_state_set_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    states: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let target = get_private_object(scope, states, ELEMENT_INTERNALS_STATES_TARGET_SLOT)?;
    node_runtime_and_handle_from_object(scope, target).ok()
}

fn custom_state_set_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    states: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Set>> {
    get_private_value(scope, states, CUSTOM_STATE_SET_BACKING_SLOT)
        .and_then(|value| v8::Local::<v8::Set>::try_from(value).ok())
}

fn require_custom_state_set_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    states: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<v8::Local<'s, v8::Set>> {
    custom_state_set_backing(scope, states).or_else(|| {
        throw_type_error(
            scope,
            &format!("Failed to execute '{member}' on 'CustomStateSet': Illegal invocation."),
        );
        None
    })
}

fn require_custom_state_set_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    states: v8::Local<'s, v8::Object>,
    member: &str,
) -> Option<(*mut JsContextHost, DomHandle)> {
    custom_state_set_target(scope, states).or_else(|| {
        throw_type_error(
            scope,
            &format!("Failed to execute '{member}' on 'CustomStateSet': Illegal invocation."),
        );
        None
    })
}

fn custom_state_set_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<String> {
    let units = v8_value_to_dom_string_u16(scope, args.get(0), false)?;
    Some(string_from_utf16_units_lossy(units.as_slice()))
}

fn custom_state_set_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        require_custom_state_set_target(scope, args.this(), "get size")
    else {
        return;
    };
    let size = unsafe { &*runtime_ptr }
        .dom_host()
        .custom_state_names(handle)
        .len()
        .min(u32::MAX as usize) as u32;
    rv.set_uint32(size);
}

fn custom_state_set_add_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let Some((runtime_ptr, handle)) = require_custom_state_set_target(scope, this, "add") else {
        return;
    };
    let Some(backing) = require_custom_state_set_backing(scope, this, "add") else {
        return;
    };
    let Some(state_name) = custom_state_set_argument(scope, &args) else {
        return;
    };
    let Some(value) = v8_string(scope, &state_name) else {
        return;
    };
    if backing.add(scope, value.into()).is_none() {
        return;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let old_custom_states = runtime.dom_host().custom_state_names(handle);
    if runtime
        .dom_host_mut()
        .insert_custom_state(handle, &state_name)
    {
        runtime.note_custom_state_style_activity(handle, &state_name, old_custom_states);
    }
    rv.set(this.into());
}

fn custom_state_set_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let Some((runtime_ptr, handle)) = require_custom_state_set_target(scope, this, "delete") else {
        return;
    };
    let Some(backing) = require_custom_state_set_backing(scope, this, "delete") else {
        return;
    };
    let Some(state_name) = custom_state_set_argument(scope, &args) else {
        return;
    };
    let Some(value) = v8_string(scope, &state_name) else {
        return;
    };
    let _ = backing.delete(scope, value.into());
    let runtime = unsafe { &mut *runtime_ptr };
    let old_custom_states = runtime.dom_host().custom_state_names(handle);
    let deleted = runtime
        .dom_host_mut()
        .remove_custom_state(handle, &state_name);
    if deleted {
        runtime.note_custom_state_style_activity(handle, &state_name, old_custom_states);
    }
    rv.set_bool(deleted);
}

fn custom_state_set_clear_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let Some((runtime_ptr, handle)) = require_custom_state_set_target(scope, this, "clear") else {
        return;
    };
    let Some(backing) = require_custom_state_set_backing(scope, this, "clear") else {
        return;
    };
    backing.clear();
    let runtime = unsafe { &mut *runtime_ptr };
    let state_names = runtime.dom_host().custom_state_names(handle);
    if runtime.dom_host_mut().clear_custom_states(handle) {
        let old_custom_states = state_names.clone();
        runtime.note_custom_states_style_activity(handle, state_names, old_custom_states);
    }
    rv.set_undefined();
}

fn custom_state_set_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let Some((runtime_ptr, handle)) = require_custom_state_set_target(scope, this, "has") else {
        return;
    };
    if require_custom_state_set_backing(scope, this, "has").is_none() {
        return;
    }
    let Some(state_name) = custom_state_set_argument(scope, &args) else {
        return;
    };
    rv.set_bool(
        unsafe { &*runtime_ptr }
            .dom_host()
            .has_custom_state(handle, &state_name),
    );
}

fn custom_state_set_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_custom_state_set_target(scope, args.this(), "entries").is_none() {
        return;
    }
    let Some(backing) = require_custom_state_set_backing(scope, args.this(), "entries") else {
        return;
    };
    set_custom_state_set_iterator(
        scope,
        backing,
        SetlikeWebIdlIteratorMethod::Entries,
        &mut rv,
    );
}

fn custom_state_set_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_custom_state_set_target(scope, args.this(), "values").is_none() {
        return;
    }
    let Some(backing) = require_custom_state_set_backing(scope, args.this(), "values") else {
        return;
    };
    set_custom_state_set_iterator(scope, backing, SetlikeWebIdlIteratorMethod::Values, &mut rv);
}

fn set_custom_state_set_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    backing: v8::Local<'s, v8::Set>,
    method: SetlikeWebIdlIteratorMethod,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(iterator) = new_setlike_webidl_iterator(
        scope,
        backing,
        SetlikeWebIdlIteratorKind::CustomStateSet,
        method,
    ) {
        rv.set(iterator.into());
    } else {
        rv.set_undefined();
    }
}

fn custom_state_set_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if require_custom_state_set_target(scope, args.this(), "forEach").is_none() {
        return;
    }
    let Some(backing) = require_custom_state_set_backing(scope, args.this(), "forEach") else {
        return;
    };
    if let Some(result) = call_setlike_webidl_for_each(
        scope,
        backing,
        args.this(),
        args.get(0),
        args.get(1),
        "CustomStateSet forEach",
    ) {
        rv.set(result);
    }
}

fn element_internals_value_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<String> {
    Some(format!(
        "__moliElementInternalsValue:{}",
        data.to_string(scope)?.to_rust_string_lossy(scope)
    ))
}

fn element_internals_value_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = element_internals_value_slot(scope, args.data()) else {
        rv.set_null();
        return;
    };
    match get_private_value(scope, args.this(), &slot) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

fn element_internals_value_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = element_internals_value_slot(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let argument = args.get(0);
    if argument.is_null_or_undefined() {
        let value = v8::null(scope);
        set_private_value(scope, args.this(), &slot, value.into());
        rv.set_undefined();
        return;
    }
    let Some(value) =
        property_dom_string_value(scope, argument, "ElementInternals", "ARIA reflection")
    else {
        return;
    };
    let Some(value) = v8_string(scope, &value) else {
        return;
    };
    set_private_value(scope, args.this(), &slot, value.into());
    rv.set_undefined();
}

fn element_internals_element_reference_value_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = element_internals_value_slot(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    set_private_value(scope, args.this(), &slot, args.get(0));
    rv.set_undefined();
}

fn ensure_form_associated_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    internals: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let target = internals_target(scope, internals)?;
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, target) else {
        throw_form_not_supported(scope);
        return None;
    };
    if !custom_elements::is_form_associated_custom_element_handle(unsafe { &*runtime_ptr }, handle)
    {
        throw_form_not_supported(scope);
        return None;
    }
    Some((runtime_ptr, handle))
}

fn throw_form_not_supported(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "The target element is not a form-associated custom element.",
    );
}

fn element_internals_set_form_value_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if ensure_form_associated_target(scope, args.this()).is_none() {
        return;
    }
    let value = snapshot_form_data_value(scope, args.get(0));
    set_private_value(scope, args.this(), ELEMENT_INTERNALS_FORM_VALUE_SLOT, value);
    rv.set_undefined();
}

fn element_internals_set_validity_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = ensure_form_associated_target(scope, args.this()) else {
        return;
    };
    let old_state = unsafe { &*runtime_ptr }.retained_current_element_state(handle);
    let validity = ensure_validity_object(scope, args.this());
    let flags = args.get(0).to_object(scope);
    let value_missing = validity_flag_enabled(scope, flags, "valueMissing");
    let type_mismatch = validity_flag_enabled(scope, flags, "typeMismatch");
    let pattern_mismatch = validity_flag_enabled(scope, flags, "patternMismatch");
    let too_long = validity_flag_enabled(scope, flags, "tooLong");
    let too_short = validity_flag_enabled(scope, flags, "tooShort");
    let range_underflow = validity_flag_enabled(scope, flags, "rangeUnderflow");
    let range_overflow = validity_flag_enabled(scope, flags, "rangeOverflow");
    let step_mismatch = validity_flag_enabled(scope, flags, "stepMismatch");
    let bad_input = validity_flag_enabled(scope, flags, "badInput");
    let custom_error = validity_flag_enabled(scope, flags, "customError");
    let valid = ![
        value_missing,
        type_mismatch,
        pattern_mismatch,
        too_long,
        too_short,
        range_underflow,
        range_overflow,
        step_mismatch,
        bad_input,
        custom_error,
    ]
    .into_iter()
    .any(|enabled| enabled);
    let _ = ValidityStateFlagsUpdateDeclaration::new(
        value_missing,
        type_mismatch,
        pattern_mismatch,
        too_long,
        too_short,
        range_underflow,
        range_overflow,
        step_mismatch,
        bad_input,
        custom_error,
    )
    .initialize(scope, validity);
    if !valid && args.get(1).is_undefined() {
        throw_type_error(
            scope,
            "ElementInternals.setValidity requires a message for invalid state.",
        );
        return;
    }
    if !validate_set_validity_anchor(scope, runtime_ptr, handle, args.get(2)) {
        return;
    }
    let _ = ValidityStateValidUpdateDeclaration::new(valid).initialize(scope, validity);
    let message = if valid {
        v8str(scope, "").into()
    } else {
        args.get(1)
            .to_string(scope)
            .map(Into::into)
            .unwrap_or_else(|| v8str(scope, "").into())
    };
    set_private_value(
        scope,
        args.this(),
        ELEMENT_INTERNALS_VALIDATION_MESSAGE_SLOT,
        message,
    );
    unsafe { &mut *runtime_ptr }.note_element_state_style_activity_with_old_state(
        handle,
        StyloElementState::VALIDITY_STATES,
        old_state,
    );
    rv.set_undefined();
}

fn validity_flag_enabled(
    scope: &mut v8::PinScope<'_, '_>,
    flags: Option<v8::Local<'_, v8::Object>>,
    name: &'static str,
) -> bool {
    flags
        .and_then(|object| object.get(scope, v8str(scope, name).into()))
        .is_some_and(|value| value.boolean_value(scope))
}

fn element_internals_form_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = ensure_form_associated_target(scope, args.this()) else {
        return;
    };
    let Some(form_handle) = form_associated_form_owner(unsafe { &*runtime_ptr }, handle) else {
        rv.set_null();
        return;
    };
    match unsafe { &mut *runtime_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, form_handle)
    {
        Some(form) => rv.set(form.into()),
        None => rv.set_null(),
    }
}

fn element_internals_will_validate_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = ensure_form_associated_target(scope, args.this()) else {
        return;
    };
    rv.set_bool(internals_will_validate(unsafe { &*runtime_ptr }, handle));
}

fn element_internals_validity_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if ensure_form_associated_target(scope, args.this()).is_none() {
        return;
    }
    rv.set(ensure_validity_object(scope, args.this()).into());
}

fn element_internals_validation_message_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if ensure_form_associated_target(scope, args.this()).is_none() {
        return;
    }
    match get_private_value(
        scope,
        args.this(),
        ELEMENT_INTERNALS_VALIDATION_MESSAGE_SLOT,
    ) {
        Some(message) => rv.set(message),
        None => rv.set_empty_string(),
    }
}

fn element_internals_check_validity_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = ensure_form_associated_target(scope, args.this()) else {
        return;
    };
    let is_valid = !internals_will_validate(unsafe { &*runtime_ptr }, handle)
        || validity_state_from_internals(scope, args.this()).valid();
    if !is_valid {
        dispatch_invalid_event(scope, runtime_ptr, handle);
    }
    rv.set_bool(is_valid);
}

fn element_internals_labels_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = ensure_form_associated_target(scope, args.this()) else {
        return;
    };
    let labels = collections::build_live_collection_for_node(
        scope,
        runtime_ptr,
        handle,
        CollectionKind::NodeList,
        LiveCollectionQueryKind::Labels,
        None,
        false,
    );
    rv.set(labels.into());
}

fn ensure_validity_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    internals: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    if let Some(validity) = get_private_object(scope, internals, ELEMENT_INTERNALS_VALIDITY_SLOT) {
        return validity;
    }
    let validity = ValidityStateDeclaration {
        value_missing: false,
        type_mismatch: false,
        pattern_mismatch: false,
        too_long: false,
        too_short: false,
        range_underflow: false,
        range_overflow: false,
        step_mismatch: false,
        bad_input: false,
        custom_error: false,
        valid: true,
    }
    .bind(scope)
    .expect("ValidityState declaration should bind");
    set_private_value(
        scope,
        internals,
        ELEMENT_INTERNALS_VALIDITY_SLOT,
        validity.into(),
    );
    validity
}

fn validity_state_from_internals<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    internals: v8::Local<'s, v8::Object>,
) -> FormControlValidity {
    let validity = ensure_validity_object(scope, internals);
    FormControlValidity {
        value_missing: validity_bool_property(scope, validity, "valueMissing"),
        type_mismatch: validity_bool_property(scope, validity, "typeMismatch"),
        pattern_mismatch: validity_bool_property(scope, validity, "patternMismatch"),
        too_long: validity_bool_property(scope, validity, "tooLong"),
        too_short: validity_bool_property(scope, validity, "tooShort"),
        range_underflow: validity_bool_property(scope, validity, "rangeUnderflow"),
        range_overflow: validity_bool_property(scope, validity, "rangeOverflow"),
        step_mismatch: validity_bool_property(scope, validity, "stepMismatch"),
        bad_input: validity_bool_property(scope, validity, "badInput"),
        custom_error: validity_bool_property(scope, validity, "customError"),
    }
}

fn validity_bool_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    validity: v8::Local<'s, v8::Object>,
    property: &'static str,
) -> bool {
    validity
        .get(scope, v8str(scope, property).into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn validate_set_validity_anchor(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: DomHandle,
    anchor: v8::Local<'_, v8::Value>,
) -> bool {
    if anchor.is_undefined() {
        return true;
    }
    let Some(anchor) = v8::Local::<v8::Object>::try_from(anchor).ok() else {
        throw_type_error(
            scope,
            "ElementInternals.setValidity anchor must be an element.",
        );
        return false;
    };
    let Ok((anchor_runtime_ptr, anchor_handle)) =
        node_runtime_and_handle_from_object(scope, anchor)
    else {
        throw_type_error(
            scope,
            "ElementInternals.setValidity anchor must be an element.",
        );
        return false;
    };
    if anchor_runtime_ptr != runtime_ptr {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The validation anchor is not a descendant of the target element.",
        );
        return false;
    }
    let runtime = unsafe { &*runtime_ptr };
    let is_html_element = runtime
        .dom_host()
        .node(anchor_handle)
        .and_then(Node::as_element)
        .is_some_and(|element| element.namespace() == XHTML_NS);
    if !is_html_element {
        throw_type_error(
            scope,
            "ElementInternals.setValidity anchor must be a HTMLElement.",
        );
        return false;
    }
    if !shadow_including_tree_contains(runtime, target, anchor_handle) {
        throw_dom_exception(
            scope,
            "NotFoundError",
            8,
            "The validation anchor is not a descendant of the target element.",
        );
        return false;
    }
    true
}

fn shadow_including_tree_contains(
    runtime: &JsContextHost,
    root: DomHandle,
    handle: DomHandle,
) -> bool {
    let mut current = handle;
    loop {
        if current == root {
            return true;
        }
        if let Some(parent) = runtime.dom_host().parent_node(current) {
            current = parent;
            continue;
        }
        if runtime.dom_host().is_shadow_root(current)
            && let Some(host) = runtime.dom_host().shadow_root_host(current)
        {
            current = host;
            continue;
        }
        return false;
    }
}

fn internals_will_validate(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return false;
    };
    if form_control_is_effectively_disabled(runtime, handle) || element.has_attribute("readonly") {
        return false;
    }
    let mut current = runtime.dom_host().parent_node(handle);
    while let Some(parent) = current {
        let Some(parent_element) = runtime.dom_host().node(parent).and_then(Node::as_element)
        else {
            current = runtime.dom_host().parent_node(parent);
            continue;
        };
        if parent_element.is_html_element("datalist")
            || (parent_element.is_html_element("fieldset")
                && parent_element.has_attribute("disabled"))
        {
            return false;
        }
        current = runtime.dom_host().parent_node(parent);
    }
    true
}
