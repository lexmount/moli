use crate::custom_elements;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{JsContextHost, throw_dom_exception};
use crate::util::{throw_type_error, v8str};

pub(in crate::native_bridge) fn validate_registry_association_for_document(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    registry_association: Option<custom_elements::CustomElementRegistryAssociation>,
) -> bool {
    let Some(registry_association) = registry_association else {
        return true;
    };
    if custom_elements::registry_association_matches_document(
        unsafe { &*runtime_ptr },
        document_handle,
        registry_association,
    ) {
        return true;
    }
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "CustomElementRegistry belongs to a different document.",
    );
    false
}

#[derive(Clone, Copy)]
pub(in crate::native_bridge::document) struct ImportNodeOptions {
    pub(in crate::native_bridge::document) deep: bool,
    pub(in crate::native_bridge::document) fallback_registry:
        Option<custom_elements::CustomElementRegistryAssociation>,
}

pub(in crate::native_bridge::document) fn parse_import_node_options(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<ImportNodeOptions> {
    if value.is_undefined() || value.is_null() {
        return Some(ImportNodeOptions {
            deep: false,
            fallback_registry: None,
        });
    }
    if !value.is_object() {
        return Some(ImportNodeOptions {
            deep: value.boolean_value(scope),
            fallback_registry: None,
        });
    }

    let options = value.to_object(scope)?;
    let self_only = options
        .get(scope, v8str(scope, "selfOnly").into())
        .is_some_and(|value| value.boolean_value(scope));
    let registry_value = options.get(scope, v8str(scope, "customElementRegistry").into());
    let fallback_registry = match registry_value {
        Some(value) if value.is_null() => {
            throw_type_error(
                scope,
                "Document.importNode customElementRegistry cannot be null.",
            );
            return None;
        }
        Some(value) => custom_elements::registry_association_from_value(scope, value),
        None => None,
    };

    Some(ImportNodeOptions {
        deep: !self_only,
        fallback_registry,
    })
}
