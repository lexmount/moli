use super::*;
use crate::dom::native::DomStringValue;
use crate::native_bridge::element::{
    TrustedAttributeSetter,
    set_live_element_attribute_ns_utf16_units_appending_to_current_reaction_queue,
    set_live_element_attribute_utf16_units_appending_to_current_reaction_queue,
    trusted_attribute_value_string16,
};
use crate::util::v8_string_from_utf16_units;

impl<'s> AttrReference<'s> {
    pub fn set_value(&self, scope: &mut v8::PinScope<'s, '_>, value: DomStringValue) {
        let Some(original_node) = self.owner(scope) else {
            self.store_value(scope, &value);
            return;
        };
        let Some(input) = v8_string_from_utf16_units(scope, &value.utf16_units()) else {
            return;
        };
        let Some(units) = trusted_attribute_value_string16(
            scope,
            Some((original_node.runtime_ptr, original_node.handle)),
            self.namespace.as_deref(),
            &self.local_name,
            input.into(),
            TrustedAttributeSetter::AttrValue,
        ) else {
            return;
        };
        let value = DomStringValue::from_utf16(&units);
        // A default policy can detach this attribute or move it to another
        // element. Continue updating the same Attr using its current owner.
        let Some(owner) = self.owner_element(scope) else {
            self.store_value(scope, &value);
            return;
        };
        let Some(node) = self.owner(scope) else {
            return;
        };
        let Some(name) = self.name(scope) else {
            return;
        };
        let prefix = self.prefix(scope);
        self.store_value(scope, &value);
        let Some(context) = owner.get_creation_context(scope) else {
            return;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let detached = detached_native_handle_for_runtime(scope, node.runtime_ptr, owner).is_some();
        let normalized_name = unsafe { &*node.runtime_ptr }
            .dom_host()
            .dom()
            .normalized_attribute_name(node.handle, &name);
        custom_elements::with_custom_element_reaction_scope(scope, node.runtime_ptr, |scope| {
            if detached {
                if self.namespace.is_none() && normalized_name.as_deref() == Some(name.as_str()) {
                    write_detached_native_attribute_appending_to_current_reaction_queue(
                        scope, owner, &name, value,
                    );
                } else {
                    write_detached_native_attribute_ns_appending_to_current_reaction_queue(
                        scope,
                        owner,
                        self.namespace.as_deref(),
                        prefix.as_deref(),
                        &name,
                        &self.local_name,
                        value,
                    );
                }
            } else if self.namespace.is_none() && normalized_name.as_deref() == Some(name.as_str())
            {
                set_live_element_attribute_utf16_units_appending_to_current_reaction_queue(
                    scope,
                    node.runtime_ptr,
                    node.handle,
                    &name,
                    value.as_str_lossy(),
                    units,
                );
            } else {
                set_live_element_attribute_ns_utf16_units_appending_to_current_reaction_queue(
                    scope,
                    node.runtime_ptr,
                    node.handle,
                    self.namespace.as_deref(),
                    prefix.as_deref(),
                    &self.local_name,
                    &name,
                    value.as_str_lossy(),
                    units,
                );
            }
        });
    }

    fn store_value(&self, scope: &mut v8::PinScope<'s, '_>, value: &DomStringValue) {
        if let Some(value) = v8_string_from_utf16_units(scope, &value.utf16_units()) {
            let _ = self
                .state
                .set(scope, v8str(scope, "value").into(), value.into());
        }
    }
}
