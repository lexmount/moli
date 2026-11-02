use super::super::{document_runtime::DomHandle, native_bridge::JsContextHost};
use super::definition::{CustomElementDefinition, CustomElementStore};
use super::definition_name_for_handle;
use std::rc::Rc;

impl CustomElementStore {
    pub(crate) fn definition_extends_local_name(&self, name: &str) -> Option<Rc<str>> {
        self.registry
            .definition(name)
            .and_then(|definition| definition.extends_local_name.clone())
    }

    pub(crate) fn has_autonomous_definition(&self, local_name: &str) -> bool {
        self.registry
            .definition(local_name)
            .is_some_and(|definition| definition.extends_local_name.is_none())
    }

    pub(super) fn has_definition(&self, name: &str) -> bool {
        self.registry.definitions.borrow().contains_key(name)
    }

    pub(super) fn observed_attributes_for_definition(&self, name: &str) -> Option<Rc<[String]>> {
        self.registry
            .definition(name)
            .map(|definition| definition.observed_attributes.clone())
    }

    pub(super) fn observes_attribute_for_handle(
        &self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        name: &str,
    ) -> bool {
        self.definition_for_handle(host_ptr, handle)
            .is_some_and(|definition| {
                definition
                    .observed_attributes
                    .iter()
                    .any(|candidate| candidate == name)
            })
    }

    pub(super) fn is_form_associated_definition_for_element(
        &self,
        local_name: &str,
        is_name: Option<&str>,
    ) -> bool {
        let definition = if self.has_autonomous_definition(local_name) {
            self.registry.definition(local_name)
        } else {
            let is_name = match is_name {
                Some(is_name) => is_name,
                None => return false,
            };
            self.definition_extends_local_name(is_name)
                .filter(|extends_local_name| extends_local_name.as_ref() == local_name)
                .and_then(|_| self.registry.definition(is_name))
        };
        definition.is_some_and(|definition| definition.form_associated)
    }

    pub(crate) fn definition_disables_shadow_for_handle(
        &self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> bool {
        self.upgraded_definition_names
            .get(&handle)
            .and_then(|name| self.registry.definition(name))
            .or_else(|| {
                definition_name_for_handle(host_ptr, handle)
                    .and_then(|candidate| self.registry.definition(&candidate))
            })
            .is_some_and(|definition| definition.disables_shadow)
    }

    pub(crate) fn definition_allows_internals_for_handle(
        &self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> bool {
        if !self.is_upgraded_handle(handle) && !self.is_pending_construction_handle(handle) {
            return false;
        }
        definition_name_for_handle(host_ptr, handle)
            .and_then(|name| self.registry.definition(&name))
            .is_some_and(|definition| {
                definition.extends_local_name.is_none() && !definition.disables_internals
            })
    }

    pub(super) fn definition_for_handle(
        &self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<Rc<CustomElementDefinition>> {
        let name = self.upgraded_definition_names.get(&handle)?;
        self.registry.definition(name).or_else(|| {
            definition_name_for_handle(host_ptr, handle)
                .and_then(|candidate| self.registry.definition(&candidate))
        })
    }
}
