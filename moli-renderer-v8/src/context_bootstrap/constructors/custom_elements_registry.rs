use super::*;
use crate::webidl;
use anyhow::Result;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CustomElementRegistry", enumerable)]
struct CustomElementRegistryPrototypeMethodsDeclaration {
    #[webapi(method, enumerable, length = 2, callback = custom_elements_define_callback)]
    define: (),

    #[webapi(method, enumerable, length = 1, callback = custom_elements_get_callback)]
    get: (),

    #[webapi(method, enumerable, length = 1, callback = custom_elements_get_name_callback)]
    get_name: (),

    #[webapi(
        method,
        enumerable,
        length = 1,
        callback = custom_elements_when_defined_callback
    )]
    when_defined: (),

    #[webapi(
        method,
        enumerable,
        length = 1,
        callback = custom_elements_initialize_callback
    )]
    initialize: (),

    #[webapi(method, enumerable, length = 1, callback = custom_elements_upgrade_callback)]
    upgrade: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CustomElementRegistry.define")]
struct CustomElementsDefineArgs<'s> {
    #[webidl(
        required,
        missing_message = "customElements.define(name, constructor) requires a name"
    )]
    name: String,
    #[webidl(
        required,
        missing_message = "customElements.define(name, constructor) requires a constructor function"
    )]
    constructor: v8::Local<'s, v8::Function>,
    #[webidl(with = parse_element_definition_options_arg)]
    options: ElementDefinitionOptions,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ElementDefinitionOptions")]
struct ElementDefinitionOptions {
    extends: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CustomElementRegistry.get")]
struct CustomElementsGetArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CustomElementRegistry.getName")]
struct CustomElementsGetNameArgs<'s> {
    #[webidl(required)]
    constructor: v8::Local<'s, v8::Function>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CustomElementRegistry.upgrade")]
struct CustomElementsUpgradeArgs<'s> {
    #[webidl(
        required,
        missing_message = "customElements.upgrade(root) requires a Node root"
    )]
    root: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CustomElementRegistry.initialize")]
struct CustomElementsInitializeArgs<'s> {
    #[webidl(
        required,
        missing_message = "customElements.initialize(root) requires a Node root"
    )]
    root: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CustomElementRegistry.whenDefined")]
struct CustomElementsWhenDefinedArgs {
    #[webidl(required)]
    name: String,
}

pub(in crate::context_bootstrap) fn install_custom_element_registry_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "CustomElementRegistry" {
        CustomElementRegistryPrototypeMethodsDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

pub(in crate::context_bootstrap) fn custom_elements_registry_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'CustomElementRegistry': Please use the 'new' operator.",
        );
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        throw_type_error(
            scope,
            "Missing native bridge host for CustomElementRegistry",
        );
        return;
    };
    let registry = args.this();
    let scoped_id =
        unsafe { &mut *host_ptr }.create_scoped_custom_elements_registry(scope, registry);
    custom_elements::mark_scoped_custom_elements_registry(scope, registry, scoped_id);
    rv.set(registry.into());
}

pub(in crate::context_bootstrap) fn custom_elements_define_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CustomElementsDefineArgs>(scope, &args) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        throw_type_error(
            scope,
            "Missing native bridge host for customElements.define",
        );
        return;
    };
    let extends_local_name = parsed.options.extends;
    let is_autonomous_definition = extends_local_name.is_none();
    let registry_key = custom_elements::registry_store_key(scope, args.this());
    let disables_shadow = match custom_elements::define_custom_element_for_registry(
        scope,
        host_ptr,
        registry_key,
        &parsed.name,
        parsed.constructor,
        extends_local_name,
    ) {
        Ok(disables_shadow) => disables_shadow,
        Err(err) => {
            if err.is_pending_exception() {
                return;
            }
            if let Some(message) = err.type_error_message() {
                throw_type_error(scope, &message);
                return;
            }
            let exception = dom_exception_value(scope, &err.message(), err.dom_exception_name());
            scope.throw_exception(exception);
            return;
        }
    };
    if disables_shadow {
        unsafe { &mut *host_ptr }
            .dom_host_mut()
            .register_shadow_disabled_custom_element_definition(&parsed.name);
    }
    if matches!(
        registry_key,
        custom_elements::CustomElementRegistryKey::Global
            | custom_elements::CustomElementRegistryKey::Child(_)
    ) && is_autonomous_definition
    {
        // Parser streams keep their own set of autonomous names so they can
        // yield at parser-created custom element boundaries before constructing
        // the element. This is a token-time handoff signal, not an upgrade
        // request; the registry-specific upgrade pass below still owns late
        // upgrades for already-created elements.
        unsafe { &mut *host_ptr }.note_parser_defined_autonomous_custom_element(&parsed.name);
    }
    match registry_key {
        custom_elements::CustomElementRegistryKey::Global => {
            custom_elements::upgrade_existing_definition_for_child(
                scope,
                host_ptr,
                None,
                &parsed.name,
            );
        }
        custom_elements::CustomElementRegistryKey::Child(handle) => {
            custom_elements::upgrade_existing_definition_for_child(
                scope,
                host_ptr,
                Some(handle),
                &parsed.name,
            );
        }
        custom_elements::CustomElementRegistryKey::Scoped(_) => {
            custom_elements::upgrade_existing_definition_for_registry(
                scope,
                host_ptr,
                registry_key,
                &parsed.name,
            );
        }
    }
}

fn parse_element_definition_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ElementDefinitionOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("CustomElementRegistry.define", (index + 1) as usize);
    webidl::dictionary_arg(args, index, context)?
        .map(|object| webidl::parse_dictionary_object::<ElementDefinitionOptions>(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

pub(in crate::context_bootstrap) fn custom_elements_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CustomElementsGetArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let registry_key = custom_elements::registry_store_key(scope, args.this());
    match unsafe { &*host_ptr }
        .custom_elements_for_registry_key(registry_key)
        .and_then(|store| store.get(scope, &parsed.name))
    {
        Some(constructor) => rv.set(constructor.into()),
        None => rv.set_undefined(),
    }
}

pub(in crate::context_bootstrap) fn custom_elements_get_name_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CustomElementsGetNameArgs>(scope, &args) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let registry_key = custom_elements::registry_store_key(scope, args.this());
    match unsafe { &*host_ptr }
        .custom_elements_for_registry_key(registry_key)
        .and_then(|store| store.get_name(scope, parsed.constructor))
    {
        Some(name) => rv.set(name.into()),
        None => rv.set(v8::null(scope).into()),
    }
}

pub(in crate::context_bootstrap) fn custom_elements_upgrade_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CustomElementsUpgradeArgs>(scope, &args) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        throw_type_error(scope, "customElements.upgrade(root) requires a Node root");
        return;
    };
    if let Some(root_handle) = crate::native_bridge::document::detached_native_handle_for_runtime(
        scope,
        host_ptr,
        parsed.root,
    ) {
        let registry_key = custom_elements::registry_store_key(scope, args.this());
        let _ = custom_elements::upgrade_subtree_if_defined_for_registry(
            scope,
            host_ptr,
            root_handle,
            registry_key,
        );
        return;
    }
    if let Ok((host_ptr, root_handle)) = node_runtime_and_handle_from_object(scope, parsed.root) {
        let registry_key = custom_elements::registry_store_key(scope, args.this());
        let _ = custom_elements::upgrade_subtree_if_defined_for_registry(
            scope,
            host_ptr,
            root_handle,
            registry_key,
        );
        return;
    }
    throw_type_error(scope, "customElements.upgrade(root) requires a Node root");
}

pub(in crate::context_bootstrap) fn custom_elements_initialize_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CustomElementsInitializeArgs>(scope, &args) else {
        return;
    };
    let Ok((host_ptr, root_handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, parsed.root)
    else {
        throw_type_error(
            scope,
            "customElements.initialize(root) requires a Node root",
        );
        return;
    };
    let registry_key = custom_elements::registry_store_key(scope, args.this());
    let registry_association =
        custom_elements::CustomElementRegistryAssociation::Registry(registry_key);
    if matches!(
        registry_key,
        custom_elements::CustomElementRegistryKey::Global
    ) && unsafe { &*host_ptr }
        .dom_host()
        .node(root_handle)
        .is_some_and(crate::dom::native::Node::is_document)
    {
        let exception = dom_exception_value(
            scope,
            "Global custom element registry cannot initialize a Document root",
            "NotSupportedError",
        );
        scope.throw_exception(exception);
        return;
    }
    let Some(root_document) = (unsafe { &*host_ptr })
        .dom_host()
        .owner_document_handle(root_handle)
    else {
        let exception = dom_exception_value(
            scope,
            "CustomElementRegistry cannot initialize a root without an owner document",
            "NotSupportedError",
        );
        scope.throw_exception(exception);
        return;
    };
    if !custom_elements::registry_association_matches_document_default(
        unsafe { &*host_ptr },
        root_document,
        registry_association,
    ) {
        let exception = dom_exception_value(
            scope,
            "CustomElementRegistry belongs to a different document.",
            "NotSupportedError",
        );
        scope.throw_exception(exception);
        return;
    }
    let _ = custom_elements::initialize_registry_for_subtree(
        scope,
        host_ptr,
        root_handle,
        registry_key,
    );
}

fn invalid_custom_element_name_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let message = format!("Invalid custom element name `{name}`");
    let error = dom_exception_value(scope, &message, "SyntaxError");
    let _ = resolver.reject(scope, error);
    Some(promise)
}

pub(in crate::context_bootstrap) fn custom_elements_when_defined_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CustomElementsWhenDefinedArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let name = parsed.name;
    if !custom_elements::is_valid_custom_element_name(&name) {
        match invalid_custom_element_name_promise(scope, &name) {
            Some(promise) => rv.set(promise.into()),
            None => rv.set_undefined(),
        }
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let registry_key = custom_elements::registry_store_key(scope, args.this());
    match unsafe { &mut *host_ptr }
        .custom_elements_mut_for_registry_key(registry_key)
        .when_defined(scope, &name)
    {
        Some(promise) => rv.set(promise.into()),
        None => rv.set_undefined(),
    }
}
