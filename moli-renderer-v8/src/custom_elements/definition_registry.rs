use super::super::util::v8_string;
use super::CustomElementRegistryKey;
use super::definition::{
    CustomElementDefinition, CustomElementRegistryData, CustomElementStore, PendingWhenDefined,
};
use super::definition_builder::{
    build_custom_element_definition, constructor_source_is_non_constructable,
    is_supported_built_in_extends_target,
};
use super::definition_error::CustomElementDefineError;
use crate::dom::custom_elements::is_valid_custom_element_name;
use crate::native_bridge::JsContextHost;
use std::rc::Rc;

#[cfg(test)]
mod tests;

pub(crate) fn define_custom_element_for_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    registry_key: CustomElementRegistryKey,
    name: &str,
    constructor: v8::Local<'s, v8::Function>,
    extends_local_name: Option<String>,
) -> Result<bool, CustomElementDefineError> {
    let constructor_value: v8::Local<'_, v8::Value> = constructor.into();
    if constructor_value.is_async_function() || constructor_value.is_generator_function() {
        return Err(CustomElementDefineError::ConstructorNotConstructable);
    }
    if constructor_source_is_non_constructable(scope, constructor_value)? {
        return Err(CustomElementDefineError::ConstructorNotConstructable);
    }
    if !is_valid_custom_element_name(name) {
        return Err(CustomElementDefineError::InvalidName(name.to_owned()));
    }

    let registry = unsafe { &mut *host_ptr }
        .custom_elements_mut_for_registry_key(registry_key)
        .registry_data();
    {
        let definitions = registry.definitions.borrow();
        if definitions.contains_key(name) {
            return Err(CustomElementDefineError::NameAlreadyDefined(
                name.to_owned(),
            ));
        }
        if definitions.values().any(|definition| {
            let registered_constructor = v8::Local::new(scope, &definition.constructor);
            registered_constructor.strict_equals(constructor.into())
        }) {
            return Err(CustomElementDefineError::ConstructorAlreadyRegistered);
        }
        if let Some(extends_local_name) = extends_local_name.as_deref()
            && !is_supported_built_in_extends_target(extends_local_name)
        {
            return Err(CustomElementDefineError::InvalidExtendsTarget(
                extends_local_name.to_owned(),
            ));
        }
        if registry.definition_is_running.get() {
            return Err(CustomElementDefineError::DefinitionAlreadyRunning);
        }
        registry.definition_is_running.set(true);
    }

    // Constructor property access can synchronously call define() again. Do
    // not retain a native store reference or a definition-table borrow across
    // that re-entry. The registry data itself remains stable if a page retires.
    let definition_result = build_custom_element_definition(scope, constructor, extends_local_name);
    registry.definition_is_running.set(false);

    let definition = definition_result?;
    let disables_shadow = definition.disables_shadow;
    registry.install_definition(scope, name, definition);
    Ok(disables_shadow)
}

impl CustomElementStore {
    pub(crate) fn definition_constructor<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        name: &str,
    ) -> Option<v8::Local<'s, v8::Function>> {
        self.registry.definition_constructor(scope, name)
    }

    pub(crate) fn get<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        name: &str,
    ) -> Option<v8::Local<'s, v8::Function>> {
        self.definition_constructor(scope, name)
    }

    pub(crate) fn get_name<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        constructor: v8::Local<'_, v8::Function>,
    ) -> Option<v8::Local<'s, v8::String>> {
        let name = self.definition_name_for_constructor(scope, constructor)?;
        v8_string(scope, &name)
    }

    pub(crate) fn definition_name_for_constructor(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        constructor: v8::Local<'_, v8::Function>,
    ) -> Option<String> {
        self.definition_for_constructor(scope, constructor)
            .map(|(name, _)| name)
    }

    pub(crate) fn definition_for_constructor(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        constructor: v8::Local<'_, v8::Function>,
    ) -> Option<(String, Option<String>)> {
        self.registry
            .definitions
            .borrow()
            .iter()
            .find(|(_, definition)| {
                let registered_constructor = v8::Local::new(scope, &definition.constructor);
                registered_constructor.strict_equals(constructor.into())
            })
            .map(|(name, definition)| {
                (
                    name.clone(),
                    definition.extends_local_name.as_deref().map(str::to_owned),
                )
            })
    }
}

impl CustomElementRegistryData {
    pub(super) fn definition(&self, name: &str) -> Option<Rc<CustomElementDefinition>> {
        self.definitions.borrow().get(name).cloned()
    }

    fn definition_constructor<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        name: &str,
    ) -> Option<v8::Local<'s, v8::Function>> {
        self.definition(name)
            .map(|definition| v8::Local::new(scope, &definition.constructor))
    }

    pub(super) fn install_definition(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        name: &str,
        definition: CustomElementDefinition,
    ) {
        let constructor = v8::Local::new(scope, &definition.constructor);
        self.definitions
            .borrow_mut()
            .insert(name.to_owned(), Rc::new(definition));
        let pending = self.pending_when_defined.borrow_mut().remove(name);
        if let Some(pending) = pending {
            // Promise resolution reads constructor.then and can re-enter this
            // registry. Both table borrows must end before executing it.
            let resolver = v8::Local::new(scope, &pending.resolver);
            let _ = resolver.resolve(scope, constructor.into());
        }
    }

    pub(crate) fn when_defined<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        name: &str,
    ) -> Option<v8::Local<'s, v8::Promise>> {
        if let Some(constructor) = self.definition_constructor(scope, name) {
            let resolver = v8::PromiseResolver::new(scope)?;
            let _ = resolver.resolve(scope, constructor.into());
            return Some(resolver.get_promise(scope));
        }

        if let Some(pending) = self.pending_when_defined.borrow().get(name) {
            return Some(v8::Local::new(scope, &pending.promise));
        }

        let resolver = v8::PromiseResolver::new(scope)?;
        let promise = resolver.get_promise(scope);
        self.pending_when_defined.borrow_mut().insert(
            name.to_owned(),
            PendingWhenDefined {
                promise: v8::Global::new(scope, promise),
                resolver: v8::Global::new(scope, resolver),
            },
        );
        Some(promise)
    }
}
