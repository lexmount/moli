use super::*;
use crate::native_bridge::throw_dom_exception;
use crate::util::{get_private_object, get_private_value, set_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DOMImplementation)]
struct DomImplementationSingletonDeclaration<'scope> {
    #[webapi(slot = DOM_IMPLEMENTATION_OWNER_DOCUMENT_SLOT)]
    owner_document: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DOMImplementation, receiver)]
struct DomImplementationPrototypeMethodsDeclaration {
    #[webapi(method, enumerable, length = 0, callback = dom_implementation_has_feature_callback)]
    has_feature: (),
    #[webapi(
        method,
        enumerable,
        length = 3,
        callback = dom_implementation_create_document_type_callback
    )]
    create_document_type: (),
    #[webapi(
        method = "createHTMLDocument",
        enumerable,
        length = 0,
        callback = dom_implementation_create_html_document_callback
    )]
    create_html_document: (),
    #[webapi(
        method,
        enumerable,
        length = 2,
        callback = dom_implementation_create_document_callback
    )]
    create_document: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMImplementation.createDocumentType")]
struct DomImplementationCreateDocumentTypeArgs {
    #[webidl(
        required,
        converter = "dom_string16",
        missing_message = "Failed to execute 'createDocumentType' on 'DOMImplementation': 3 arguments required."
    )]
    qualified_name: Vec<u16>,
    #[webidl(
        required,
        converter = "dom_string16",
        missing_message = "Failed to execute 'createDocumentType' on 'DOMImplementation': 3 arguments required."
    )]
    public_id: Vec<u16>,
    #[webidl(
        required,
        converter = "dom_string16",
        missing_message = "Failed to execute 'createDocumentType' on 'DOMImplementation': 3 arguments required."
    )]
    system_id: Vec<u16>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMImplementation.createHTMLDocument")]
struct DomImplementationCreateHtmlDocumentArgs {
    title: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMImplementation.createDocument")]
struct DomImplementationCreateDocumentArgs<'s> {
    #[webidl(
        required,
        name = "namespace",
        nullable,
        missing_message = "Failed to execute 'createDocument' on 'DOMImplementation': 2 arguments required."
    )]
    namespace_uri: Option<String>,
    #[webidl(
        required,
        name = "qualifiedName",
        treat_null_as_empty_string,
        missing_message = "Failed to execute 'createDocument' on 'DOMImplementation': 2 arguments required."
    )]
    qualified_name: String,
    #[webidl(index = 2, nullable, interface = web_api_interfaces::DocumentType)]
    doctype: Option<v8::Local<'s, v8::Object>>,
}

fn dom_implementation_has_feature_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::Boolean::new(scope, true).into());
}

const DOM_IMPLEMENTATION_OWNER_DOCUMENT_SLOT: &str = "__moliDOMImplementationOwnerDocument";

fn current_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    global
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn dom_implementation_owner_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    implementation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(
        scope,
        implementation,
        DOM_IMPLEMENTATION_OWNER_DOCUMENT_SLOT,
    )
    .or_else(|| current_document_object(scope))
}

fn create_document_type_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: &DomImplementationCreateDocumentTypeArgs,
    owner_document: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let qualified_name = crate::util::v8_string_from_utf16_units(scope, &parsed.qualified_name)?;
    let public_id = crate::util::v8_string_from_utf16_units(scope, &parsed.public_id)?;
    let system_id = crate::util::v8_string_from_utf16_units(scope, &parsed.system_id)?;
    let mut argv = vec![qualified_name.into(), public_id.into(), system_id.into()];
    if let Some(owner_document) = owner_document {
        argv.push(owner_document.into());
    }
    let (bridge, method) = global_bridge_method(scope, "__createDetachedDocumentType")?;
    method.call(scope, bridge.into(), &argv)
}

fn dom_implementation_create_document_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DomImplementationCreateDocumentTypeArgs>(scope, &args)
    else {
        return;
    };
    if !is_valid_document_type_name(&parsed.qualified_name) {
        throw_dom_exception(
            scope,
            "InvalidCharacterError",
            5,
            "String contains an invalid character",
        );
        return;
    }
    let owner_document = dom_implementation_owner_document(scope, args.this());
    let relevant_context = owner_document
        .and_then(|document| crate::native_bridge::node_relevant_context(scope, document))
        .or_else(|| args.this().get_creation_context(scope))
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    match create_document_type_value(target_scope, &parsed, owner_document) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn is_valid_document_type_name(name: &[u16]) -> bool {
    !name
        .iter()
        .any(|unit| matches!(*unit, 0 | 9 | 10 | 12 | 13 | 32 | 62))
}

fn dom_implementation_create_html_document_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DomImplementationCreateHtmlDocumentArgs>(scope, &args)
    else {
        return;
    };
    let owner_document = dom_implementation_owner_document(scope, args.this());
    let relevant_context = owner_document
        .and_then(|document| crate::native_bridge::node_relevant_context(scope, document))
        .or_else(|| args.this().get_creation_context(scope))
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    match create_html_document_value(target_scope, parsed.title.as_deref(), owner_document) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn create_html_document_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    title: Option<&str>,
    origin_document: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let (bridge, method) = global_bridge_method(scope, "__createDetachedHTMLDocument")?;
    let call_args = match title {
        Some(title) => vec![v8_string(scope, title)?.into()],
        None => Vec::new(),
    };
    let value = method.call(scope, bridge.into(), &call_args)?;
    if let Some(origin_document) = origin_document {
        let document = v8::Local::<v8::Object>::try_from(value).ok()?;
        crate::native_bridge::document::inherit_detached_document_origin(
            scope,
            document,
            origin_document,
        );
    }
    Some(value)
}

fn create_document_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    namespace_uri: Option<&str>,
    qualified_name: &str,
    doctype: Option<v8::Local<'s, v8::Object>>,
    origin_document: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let namespace_uri = match namespace_uri {
        Some(namespace_uri) => v8_string(scope, namespace_uri)?.into(),
        None => v8::null(scope).into(),
    };
    let qualified_name = v8_string(scope, qualified_name)?.into();
    let doctype = doctype
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    let argv = [namespace_uri, qualified_name, doctype];
    let (bridge, method) = global_bridge_method(scope, "__createDetachedXmlDocument")?;
    let value = method.call(scope, bridge.into(), &argv)?;
    if let Some(origin_document) = origin_document {
        let document = v8::Local::<v8::Object>::try_from(value).ok()?;
        crate::native_bridge::document::inherit_detached_document_origin(
            scope,
            document,
            origin_document,
        );
    }
    Some(value)
}

fn dom_implementation_create_document_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DomImplementationCreateDocumentArgs>(scope, &args)
    else {
        return;
    };
    let owner_document = dom_implementation_owner_document(scope, args.this());
    let relevant_context = owner_document
        .and_then(|document| crate::native_bridge::node_relevant_context(scope, document))
        .or_else(|| args.this().get_creation_context(scope))
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    match create_document_value(
        target_scope,
        parsed.namespace_uri.as_deref(),
        &parsed.qualified_name,
        parsed.doctype,
        owner_document,
    ) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

pub(crate) fn ensure_dom_implementation_singleton<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(singleton) = get_private_value(scope, global, DOM_IMPLEMENTATION_SINGLETON_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(singleton);
    }
    let owner_document = global
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let singleton = DomImplementationSingletonDeclaration { owner_document }
        .bind(scope)
        .ok()?;
    set_private_value(
        scope,
        global,
        DOM_IMPLEMENTATION_SINGLETON_SLOT,
        singleton.into(),
    );
    Some(singleton)
}

pub(in crate::context_bootstrap) fn install_dom_implementation_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    if interface_name == "DOMImplementation" {
        DomImplementationPrototypeMethodsDeclaration::initialize_prototype_template(
            scope, prototype,
        );
    }
}
