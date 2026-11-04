use std::sync::LazyLock;

use super::{Attribute, CustomElementState, ElementControlState};
use crate::forms::{InputType, InputValueSanitizationContext};
use crate::native::NativeNodeId;

static EMPTY_CONTROL_STATE: LazyLock<ElementControlState> =
    LazyLock::new(ElementControlState::default);

/// State that is absent from the common `Element` record until an element
/// actually needs one of the stateful Web API surfaces.
///
/// Reads borrow a single immutable default value. Mutations materialize an
/// element-owned payload before changing it. Both the large control state and
/// uncommon identity/association fields stay behind that one common-record
/// pointer. Attribute synchronization is kept here so an unrelated attribute
/// on an ordinary element does not accidentally allocate rare state.
#[derive(Debug, Clone, Default)]
pub(super) struct ElementRareData {
    payload: Option<Box<ElementRareDataPayload>>,
}

#[derive(Debug, Clone, Default)]
struct ElementRareDataPayload {
    // Keep the control state inline in the out-of-line payload. Stateful
    // elements then pay one rare allocation, while common elements still pay
    // only the nullable payload pointer.
    control_state: Option<ElementControlState>,
    custom_element_state: Option<CustomElementState>,
    custom_element_is_name: Option<String>,
    parser_associated_form_owner: Option<NativeNodeId>,
    template_contents: Option<NativeNodeId>,
    attribute_utf16_units: Vec<(String, Box<[u16]>)>,
}

impl ElementRareDataPayload {
    fn is_empty(&self) -> bool {
        self.control_state.is_none()
            && self.custom_element_state.is_none()
            && self.custom_element_is_name.is_none()
            && self.parser_associated_form_owner.is_none()
            && self.template_contents.is_none()
            && self.attribute_utf16_units.is_empty()
    }
}

impl ElementRareData {
    pub(super) fn from_element_parts(
        namespace: &str,
        local_name: &str,
        attributes: &[Attribute],
        custom_element_state: CustomElementState,
    ) -> Self {
        let control_state =
            ElementControlState::from_element_parts(namespace, local_name, attributes);
        let custom_element_state = (custom_element_state != CustomElementState::Uncustomized)
            .then_some(custom_element_state);
        if control_state.is_none() && custom_element_state.is_none() {
            return Self::default();
        }
        Self {
            payload: Some(Box::new(ElementRareDataPayload {
                control_state,
                custom_element_state,
                custom_element_is_name: None,
                parser_associated_form_owner: None,
                template_contents: None,
                attribute_utf16_units: Vec::new(),
            })),
        }
    }

    pub(super) fn attribute_utf16_units(&self, qualified_name: &str) -> Option<&[u16]> {
        self.payload
            .as_deref()?
            .attribute_utf16_units
            .iter()
            .find(|(name, _)| name == qualified_name)
            .map(|(_, units)| units.as_ref())
    }

    pub(super) fn set_attribute_utf16_units(
        &mut self,
        qualified_name: String,
        units: Option<Box<[u16]>>,
    ) -> bool {
        if self.attribute_utf16_units(&qualified_name) == units.as_deref() {
            return false;
        }

        if let Some(units) = units {
            let values = &mut self.payload_mut().attribute_utf16_units;
            if let Some((_, current)) = values.iter_mut().find(|(name, _)| name == &qualified_name)
            {
                *current = units;
            } else {
                values.push((qualified_name, units));
            }
        } else if let Some(payload) = self.payload.as_deref_mut() {
            payload
                .attribute_utf16_units
                .retain(|(name, _)| name != &qualified_name);
        }
        self.release_empty_payload();
        true
    }

    pub(super) fn control_state(&self) -> &ElementControlState {
        self.payload
            .as_deref()
            .and_then(|payload| payload.control_state.as_ref())
            .unwrap_or(&EMPTY_CONTROL_STATE)
    }

    pub(super) fn control_state_mut(&mut self) -> &mut ElementControlState {
        self.payload_mut()
            .control_state
            .get_or_insert_with(ElementControlState::default)
    }

    pub(super) fn clear_explicit_element_references(&mut self, attribute: &str) {
        if let Some(control_state) = self
            .payload
            .as_deref_mut()
            .and_then(|payload| payload.control_state.as_mut())
        {
            control_state.clear_explicit_element_references(attribute);
        }
    }

    pub(super) fn custom_element_state(&self) -> CustomElementState {
        self.payload
            .as_deref()
            .and_then(|payload| payload.custom_element_state)
            .unwrap_or(CustomElementState::Uncustomized)
    }

    pub(super) fn set_custom_element_state(&mut self, state: CustomElementState) -> bool {
        if self.custom_element_state() == state {
            return false;
        }

        if state == CustomElementState::Uncustomized {
            if let Some(payload) = self.payload.as_deref_mut() {
                payload.custom_element_state = None;
            }
        } else {
            self.payload_mut().custom_element_state = Some(state);
        }
        self.release_empty_payload();
        true
    }

    pub(super) fn custom_element_is_name(&self) -> Option<&str> {
        self.payload
            .as_deref()
            .and_then(|payload| payload.custom_element_is_name.as_deref())
    }

    pub(super) fn set_custom_element_is_name(&mut self, is_name: Option<String>) -> bool {
        if self.custom_element_is_name() == is_name.as_deref() {
            return false;
        }

        if let Some(is_name) = is_name {
            self.payload_mut().custom_element_is_name = Some(is_name);
        } else if let Some(payload) = self.payload.as_deref_mut() {
            payload.custom_element_is_name = None;
        }
        self.release_empty_payload();
        true
    }

    pub(super) fn parser_associated_form_owner(&self) -> Option<NativeNodeId> {
        self.payload
            .as_deref()
            .and_then(|payload| payload.parser_associated_form_owner)
    }

    pub(super) fn set_parser_associated_form_owner(&mut self, owner: Option<NativeNodeId>) -> bool {
        if self.parser_associated_form_owner() == owner {
            return false;
        }

        if let Some(owner) = owner {
            self.payload_mut().parser_associated_form_owner = Some(owner);
        } else if let Some(payload) = self.payload.as_deref_mut() {
            payload.parser_associated_form_owner = None;
        }
        self.release_empty_payload();
        true
    }

    pub(super) fn template_contents(&self) -> Option<NativeNodeId> {
        self.payload
            .as_deref()
            .and_then(|payload| payload.template_contents)
    }

    pub(super) fn set_template_contents(&mut self, template_contents: Option<NativeNodeId>) {
        if self.template_contents() == template_contents {
            return;
        }

        if let Some(template_contents) = template_contents {
            self.payload_mut().template_contents = Some(template_contents);
        } else if let Some(payload) = self.payload.as_deref_mut() {
            payload.template_contents = None;
        }
        self.release_empty_payload();
    }

    pub(super) fn sync_control_state_from_attribute(
        &mut self,
        namespace: &str,
        local_name: &str,
        input_type: InputType,
        input_context: InputValueSanitizationContext<'_>,
        attribute_name: &str,
        attribute_value: Option<&str>,
    ) {
        if let Some(control_state) = self
            .payload
            .as_deref_mut()
            .and_then(|payload| payload.control_state.as_mut())
        {
            control_state.sync_from_attribute_parts(
                namespace,
                local_name,
                input_type,
                input_context,
                attribute_name,
                attribute_value,
            );
            return;
        }

        // A nonce has an observable distinction between null and the empty
        // string. Other attribute-backed control state belongs to element
        // kinds whose state is materialized during construction. Setting or
        // removing an unrelated attribute must leave an ordinary element
        // allocation-free.
        if attribute_name != "nonce" || attribute_value.is_none() {
            return;
        }

        self.control_state_mut().sync_from_attribute_parts(
            namespace,
            local_name,
            input_type,
            input_context,
            attribute_name,
            attribute_value,
        );
    }

    #[cfg(test)]
    pub(super) fn is_materialized(&self) -> bool {
        self.payload.is_some()
    }

    fn payload_mut(&mut self) -> &mut ElementRareDataPayload {
        self.payload
            .get_or_insert_with(|| Box::new(ElementRareDataPayload::default()))
    }

    fn release_empty_payload(&mut self) {
        if self
            .payload
            .as_deref()
            .is_some_and(ElementRareDataPayload::is_empty)
        {
            self.payload = None;
        }
    }
}
