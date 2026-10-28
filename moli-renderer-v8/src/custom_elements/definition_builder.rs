use super::super::util::v8str;
use super::definition::{CustomElementDefineError, CustomElementDefinition};
pub(super) use super::definition_callbacks::custom_element_constructor_prototype;
use super::definition_callbacks::{callbacks_for_prototype, form_callbacks_for_prototype};
pub(super) use super::definition_constructor_source::constructor_source_is_non_constructable;
pub(super) use super::definition_extends::is_supported_built_in_extends_target;
use super::definition_sequence::sequence_property_for_constructor;
use anyhow::Result;

pub(super) fn build_custom_element_definition<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
    extends_local_name: Option<String>,
) -> Result<CustomElementDefinition, CustomElementDefineError> {
    let prototype = custom_element_constructor_prototype(scope, constructor)?;
    let mut callbacks = callbacks_for_prototype(scope, prototype)?;
    let observed_attributes = if callbacks.attribute_changed.is_some() {
        sequence_property_for_constructor(scope, constructor, "observedAttributes")?
    } else {
        Vec::new()
    };
    let disabled_features =
        sequence_property_for_constructor(scope, constructor, "disabledFeatures")?;
    let disables_shadow = disabled_features.iter().any(|feature| feature == "shadow");
    let disables_internals = disabled_features
        .iter()
        .any(|feature| feature == "internals");
    let form_associated = constructor
        .get(scope, v8str(scope, "formAssociated").into())
        .ok_or(CustomElementDefineError::PendingException)?
        .boolean_value(scope);
    if form_associated {
        form_callbacks_for_prototype(scope, prototype, &mut callbacks)?;
    }

    Ok(CustomElementDefinition {
        constructor: v8::Global::new(scope, constructor),
        observed_attributes: observed_attributes.into(),
        callbacks,
        disables_shadow,
        disables_internals,
        form_associated,
        extends_local_name: extends_local_name.map(Into::into),
    })
}
