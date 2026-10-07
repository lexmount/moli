use crate::custom_elements;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{JsContextHost, throw_dom_exception};
use crate::{web_api_interfaces, webidl};

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

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ImportNodeOptions")]
struct ImportNodeDictionary<'s> {
    #[webidl(interface = web_api_interfaces::CustomElementRegistry)]
    custom_element_registry: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = false)]
    self_only: bool,
}

pub(in crate::native_bridge::document) fn parse_import_node_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<ImportNodeOptions> {
    // The optional argument defaults to false for missing/undefined values.
    // Null selects the dictionary member of the union, whose selfOnly default
    // makes the import deep. Other primitives select boolean via ToBoolean.
    if value.is_undefined() || (!value.is_object() && !value.is_null()) {
        return Some(ImportNodeOptions {
            deep: value.boolean_value(scope),
            fallback_registry: None,
        });
    }

    let options = match webidl::parse_dictionary::<ImportNodeDictionary>(
        scope,
        value,
        webidl::Context::argument("Document.importNode", 2),
    ) {
        Ok(options) => options.unwrap_or_default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let fallback_registry = options.custom_element_registry.and_then(|registry| {
        let target = moli_webapi_declare::web_api_object_target(scope, registry)
            .expect("converted CustomElementRegistry has native identity");
        custom_elements::registry_association_from_value(scope, target.into())
    });

    Some(ImportNodeOptions {
        deep: !options.self_only,
        fallback_registry,
    })
}
