use std::cell::RefCell;

use indexmap::IndexMap;
use style::{stylesheets::UrlExtraData, stylist::RegisterCustomPropertyResult};

use super::{MoliStyleEngine, source_dirty::StyleSourceDirtyReason, source_id::StyleScopeId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CssCustomPropertyRegistration {
    pub(crate) name: String,
    pub(crate) syntax: String,
    pub(crate) inherits: bool,
    pub(crate) initial_value: Option<String>,
}

/// One successful registration paired with the URL environment in which its
/// initial value was parsed. A later `<base>` mutation must not reinterpret an
/// already-registered custom property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CssCustomPropertyRegistrationRecord {
    pub(crate) registration: CssCustomPropertyRegistration,
    pub(crate) base_url: url::Url,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CssCustomPropertyRegistrationError {
    AlreadyRegistered,
}

#[derive(Debug, Default)]
pub(super) struct CssCustomPropertyRegistry {
    registrations: RefCell<IndexMap<String, CssCustomPropertyRegistrationRecord>>,
}

impl CssCustomPropertyRegistry {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn clear(&self) {
        self.registrations.borrow_mut().clear();
    }

    fn register(
        &self,
        registration: CssCustomPropertyRegistration,
        base_url: url::Url,
    ) -> Result<(), CssCustomPropertyRegistrationError> {
        let mut registrations = self.registrations.borrow_mut();
        if registrations.contains_key(&registration.name) {
            return Err(CssCustomPropertyRegistrationError::AlreadyRegistered);
        }
        registrations.insert(
            registration.name.clone(),
            CssCustomPropertyRegistrationRecord {
                registration,
                base_url,
            },
        );
        Ok(())
    }

    fn registration(&self, name: &str) -> Option<CssCustomPropertyRegistration> {
        self.registrations
            .borrow()
            .get(name)
            .map(|record| record.registration.clone())
    }

    pub(super) fn registration_records(&self) -> Vec<CssCustomPropertyRegistrationRecord> {
        self.registrations.borrow().values().cloned().collect()
    }
}

impl MoliStyleEngine {
    pub(crate) fn validate_css_custom_property_registration(
        &self,
        registration: &CssCustomPropertyRegistration,
        base_url: url::Url,
    ) -> RegisterCustomPropertyResult {
        style::stylist::Stylist::validate_custom_property_registration(
            &UrlExtraData::from(base_url),
            &registration.name,
            &registration.syntax,
            registration.initial_value.as_deref(),
        )
    }

    pub(crate) fn register_css_custom_property_for_document(
        &mut self,
        document: crate::document_runtime::DomHandle,
        registration: CssCustomPropertyRegistration,
        base_url: url::Url,
    ) -> Result<(), CssCustomPropertyRegistrationError> {
        let world = self.world_for_document(document);
        world
            .registered_custom_properties
            .register(registration, base_url)?;
        world.document_state.note_style_input_change();
        world.document_state.record_source_dirty_scope(
            StyleScopeId::Document(document),
            StyleSourceDirtyReason::CustomPropertyRegistration,
            std::iter::empty(),
            [document],
        );
        Ok(())
    }

    pub(crate) fn registered_css_custom_property_registration_for_document(
        &self,
        document: crate::document_runtime::DomHandle,
        name: &str,
    ) -> Option<CssCustomPropertyRegistration> {
        self.world_for_document(document)
            .registered_custom_properties
            .registration(name)
    }

    pub(crate) fn script_css_custom_property_registration_records_for_document(
        &self,
        document: crate::document_runtime::DomHandle,
    ) -> Vec<CssCustomPropertyRegistrationRecord> {
        self.world_for_document(document)
            .registered_custom_properties
            .registration_records()
    }
}
