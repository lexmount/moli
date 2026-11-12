use super::super::{document_runtime::DomHandle, native_bridge::PageScriptHost};
use super::definition::{CustomElementDefinition, CustomElementStore};
use super::definition_name_for_handle;

impl CustomElementStore {
    pub(crate) fn definition_extends_local_name(&self, name: &str) -> Option<&str> {
        self.definitions
            .get(name)
            .and_then(|definition| definition.extends_local_name.as_deref())
    }

    pub(crate) fn has_autonomous_definition(&self, local_name: &str) -> bool {
        self.definitions
            .get(local_name)
            .is_some_and(|definition| definition.extends_local_name.is_none())
    }

    pub(super) fn has_definition(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    pub(super) fn observed_attributes_for_definition(&self, name: &str) -> Option<&[String]> {
        self.definitions
            .get(name)
            .map(|definition| definition.observed_attributes.as_slice())
    }

    pub(super) fn observes_attribute_for_handle(
        &self,
        host_ptr: *mut PageScriptHost,
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
            self.definitions.get(local_name)
        } else {
            let is_name = match is_name {
                Some(is_name) => is_name,
                None => return false,
            };
            self.definition_extends_local_name(is_name)
                .filter(|extends_local_name| *extends_local_name == local_name)
                .and_then(|_| self.definitions.get(is_name))
        };
        definition.is_some_and(|definition| definition.form_associated)
    }

    pub(crate) fn definition_disables_shadow_for_handle(
        &self,
        host_ptr: *mut PageScriptHost,
        handle: DomHandle,
    ) -> bool {
        self.upgraded_definition_names
            .get(&handle)
            .and_then(|name| self.definitions.get(name))
            .or_else(|| {
                definition_name_for_handle(host_ptr, handle)
                    .and_then(|candidate| self.definitions.get(&candidate))
            })
            .is_some_and(|definition| definition.disables_shadow)
    }

    pub(crate) fn definition_allows_internals_for_handle(
        &self,
        host_ptr: *mut PageScriptHost,
        handle: DomHandle,
    ) -> bool {
        if !self.is_upgraded_handle(handle) && !self.is_pending_construction_handle(handle) {
            return false;
        }
        definition_name_for_handle(host_ptr, handle)
            .and_then(|name| self.definitions.get(&name))
            .is_some_and(|definition| {
                definition.extends_local_name.is_none() && !definition.disables_internals
            })
    }

    pub(super) fn definition_for_handle(
        &self,
        host_ptr: *mut PageScriptHost,
        handle: DomHandle,
    ) -> Option<&CustomElementDefinition> {
        let name = self.upgraded_definition_names.get(&handle)?;
        self.definitions.get(name).or_else(|| {
            definition_name_for_handle(host_ptr, handle)
                .and_then(|candidate| self.definitions.get(&candidate))
        })
    }
}
