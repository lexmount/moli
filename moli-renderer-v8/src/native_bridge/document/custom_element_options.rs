use crate::custom_elements;
use crate::document_runtime::DomHandle;
use crate::native_bridge::{PageScriptHost, throw_dom_exception};
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

pub(in crate::native_bridge::document) type CreateElementOptions =
    moli_webapi::dom_options::CreateElementOptions<
        custom_elements::CustomElementRegistryAssociation,
    >;

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

impl<'s> webidl::WebIdlConverter<'s> for custom_elements::CustomElementRegistryAssociation {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if value.is_null() {
            return Ok(Self::Null);
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
        Ok(Self::Registry(custom_elements::registry_store_key(
            scope, target,
        )))
    }
}

pub(in crate::native_bridge::document) fn initialize_non_html_element(
    options: &CreateElementOptions,
    runtime_ptr: *mut PageScriptHost,
    handle: DomHandle,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    if let Some(registry) = options.registry_association {
        runtime.set_custom_element_registry_association(handle, registry);
    }
    if let Some(is_name) = &options.is_name {
        runtime
            .dom_host_mut()
            .set_custom_element_is_name(handle, Some(is_name.clone()));
    }
}

pub(in crate::native_bridge::document) fn flatten_create_element_options(
    mut options: CreateElementOptions,
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document_handle: DomHandle,
) -> Option<CreateElementOptions> {
    if options.registry_association.is_some() && options.is_name.is_some() {
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
        options.registry_association,
    ) {
        return None;
    }
    if options.registry_association.is_none() {
        options.registry_association =
            unsafe { &*runtime_ptr }.custom_element_registry_association(document_handle);
    }
    Some(options)
}

pub(in crate::native_bridge) fn validate_registry_association_for_document(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
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
