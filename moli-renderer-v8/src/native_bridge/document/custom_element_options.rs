use crate::custom_elements;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{JsContextHost, throw_dom_exception};
use crate::util::{throw_type_error, v8str};
use crate::web_api_interfaces;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createElement")]
pub(in crate::native_bridge::document) struct DocumentCreateElementArgs {
    #[webidl(required)]
    pub(in crate::native_bridge::document) local_name: String,
    #[webidl(with = create_element_options_argument)]
    pub(in crate::native_bridge::document) options: CreateElementOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createElementNS")]
pub(in crate::native_bridge::document) struct DocumentCreateElementNsArgs {
    #[webidl(required, nullable)]
    pub(in crate::native_bridge::document) namespace: Option<String>,
    #[webidl(required)]
    pub(in crate::native_bridge::document) qualified_name: String,
    #[webidl(with = create_element_options_argument)]
    pub(in crate::native_bridge::document) options: CreateElementOptions,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ElementCreationOptions")]
pub(in crate::native_bridge::document) struct CreateElementOptions {
    // An omitted nullable member and an explicitly null registry have different
    // meanings. Keep its presence while reusing native interface conversion.
    #[webidl(name = "customElementRegistry", with = nullable_creation_registry)]
    pub(in crate::native_bridge::document) registry_association:
        Option<custom_elements::CustomElementRegistryAssociation>,
    #[webidl(name = "is")]
    pub(in crate::native_bridge::document) is_name: Option<String>,
}

impl<'s> webidl::WebIdlConverter<'s> for CreateElementOptions {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        // Objects select the dictionary even if they are boxed strings or have
        // a toString method. Nullish values select the empty dictionary.
        if value.is_object() || value.is_null_or_undefined() {
            return webidl::parse_dictionary::<Self>(scope, value, context)
                .map(Option::unwrap_or_default);
        }
        // Other primitives select the legacy DOMString member, which the DOM
        // algorithm ignores. Conversion still rejects Symbols.
        webidl::convert::<webidl::DomString16>(scope, value, context)?;
        Ok(Self::default())
    }
}

fn create_element_options_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<CreateElementOptions, webidl::WebIdlError> {
    let prefix = if index == 1 {
        "Document.createElement"
    } else {
        "Document.createElementNS"
    };
    webidl::argument(
        scope,
        args,
        index,
        webidl::Context::argument(prefix, index as usize + 1),
    )
}

fn nullable_creation_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Result<Option<custom_elements::CustomElementRegistryAssociation>, webidl::WebIdlError> {
    let context = webidl::Context::member("ElementCreationOptions", member);
    let Some(value) = webidl::property_result(scope, object, member, context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    if value.is_null() {
        return Ok(Some(
            custom_elements::CustomElementRegistryAssociation::Null,
        ));
    }
    let registry = webidl::convert_with_options::<webidl::InterfaceObject>(
        scope,
        value,
        context,
        &webidl::InterfaceOptions {
            name: web_api_interfaces::CustomElementRegistry::NAME,
            brand_check: web_api_interfaces::CustomElementRegistry::is_instance,
        },
    )?;
    let target = moli_webapi_declare::web_api_object_target(scope, registry.0)
        .expect("converted CustomElementRegistry has native identity");
    let key = custom_elements::registry_store_key(scope, target)
        .ok_or_else(|| webidl::WebIdlError::cannot_convert(context, "CustomElementRegistry"))?;
    Ok(Some(
        custom_elements::CustomElementRegistryAssociation::Registry(key),
    ))
}

impl CreateElementOptions {
    pub(in crate::native_bridge::document) fn initialize_non_html_element(
        &self,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) {
        let runtime = unsafe { &mut *runtime_ptr };
        if let Some(registry) = self.registry_association {
            runtime.set_custom_element_registry_association(handle, registry);
        }
        if let Some(is_name) = &self.is_name {
            runtime
                .dom_host_mut()
                .set_custom_element_is_name(handle, Some(is_name.clone()));
        }
    }

    pub(in crate::native_bridge::document) fn flatten(
        mut self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime_ptr: *mut JsContextHost,
        document_handle: DomHandle,
    ) -> Option<Self> {
        if self.registry_association.is_some() && self.is_name.is_some() {
            throw_dom_exception(
                scope,
                "NotSupportedError",
                9,
                "An explicit CustomElementRegistry cannot be combined with is.",
            );
            return None;
        }
        if !validate_registry_association_for_document(
            scope,
            runtime_ptr,
            document_handle,
            self.registry_association,
        ) {
            return None;
        }
        if self.registry_association.is_none() {
            self.registry_association =
                unsafe { &*runtime_ptr }.custom_element_registry_association(document_handle);
        }
        Some(self)
    }
}

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
