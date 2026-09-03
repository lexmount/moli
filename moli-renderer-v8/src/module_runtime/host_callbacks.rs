use moli_webapi_declare::WebApiObject;
use url::Url;

use super::{ModuleAttributesKey, ModuleImportPhase, PendingDynamicModuleImport};
use crate::{
    context_bootstrap::child_browsing_context_handle_for_current_realm_scope,
    planning::ScriptFetchMetadata,
    util::{
        callback_arg_string, context_host_ptr_from_global_bridge,
        script_base_url_from_continuation_data, script_base_url_from_host_defined_options,
        script_nonce_from_host_defined_options, script_parser_inserted_from_host_defined_options,
        throw_type_error, v8_string,
    },
};

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ImportMetaDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    url: v8::Local<'scope, v8::String>,

    #[webapi(
        method,
        enumerable,
        callback = import_meta_resolve_callback,
        data = self.url,
        length = 1
    )]
    resolve: (),
}

pub(crate) unsafe extern "C" fn initialize_import_meta_object_callback(
    context: v8::Local<'_, v8::Context>,
    module: v8::Local<'_, v8::Module>,
    meta: v8::Local<'_, v8::Object>,
) {
    v8::callback_scope!(unsafe scope, context);
    let Ok(module_url) = v8::Local::<v8::String>::try_from(module.get_resource_name(scope)) else {
        return;
    };
    let _ = ImportMetaDeclaration::new(module_url).initialize(scope, meta);
}

pub(crate) fn dynamic_import_callback<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let specifier = specifier.to_rust_string_lossy(scope);
    queue_native_dynamic_import(
        scope,
        host_defined_options,
        resource_name,
        resolver,
        &specifier,
        import_attributes,
        ModuleImportPhase::Evaluation,
    );
    Some(promise)
}

pub(crate) fn dynamic_import_with_phase_callback<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    phase: v8::ModuleImportPhase,
    import_attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    match phase {
        v8::ModuleImportPhase::kEvaluation => dynamic_import_callback(
            scope,
            host_defined_options,
            resource_name,
            specifier,
            import_attributes,
        ),
        v8::ModuleImportPhase::kSource => queue_source_phase_dynamic_import(
            scope,
            host_defined_options,
            resource_name,
            specifier,
            import_attributes,
            ModuleImportPhase::Source,
        ),
        v8::ModuleImportPhase::kDefer => reject_unsupported_dynamic_import_phase(
            scope,
            "defer-phase dynamic import is not supported yet",
        ),
    }
}

fn queue_native_dynamic_import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    specifier: &str,
    import_attributes: v8::Local<'s, v8::FixedArray>,
    phase: ModuleImportPhase,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        reject_dynamic_import(scope, resolver, "dynamic import host is not available");
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let base_url = dynamic_import_base_url(scope, host_defined_options, resource_name, host);
    let fetch_metadata = dynamic_import_referrer_fetch_metadata(scope, host_defined_options);
    let attributes = dynamic_import_attributes(scope, import_attributes);
    if let Some(invalid_key) = attributes.invalid_import_attribute_key() {
        reject_dynamic_import(
            scope,
            resolver,
            &format!("Invalid attribute key \"{invalid_key}\"."),
        );
        return;
    }
    let child_handle = child_browsing_context_handle_for_current_realm_scope(scope);
    let Some(owner) = host.current_dynamic_module_import_owner(scope, child_handle) else {
        reject_dynamic_import(
            scope,
            resolver,
            "dynamic import document owner is not available",
        );
        return;
    };
    let resolved_url = if child_handle.is_none() {
        match host.resolve_module_specifier_with_base(specifier, &base_url) {
            Ok(url) => Some(url),
            Err(error) => {
                reject_dynamic_import(scope, resolver, &error);
                return;
            }
        }
    } else {
        None
    };
    let mut request = PendingDynamicModuleImport::new(
        v8::Global::new(scope, scope.get_current_context()),
        v8::Global::new(scope, resolver),
        owner,
        specifier,
        base_url,
        attributes,
        phase,
    )
    .with_referrer_fetch_metadata(fetch_metadata);
    if let Some(resolved_url) = resolved_url {
        request = request.with_resolved_url(resolved_url);
    }
    host.queue_native_dynamic_module_import(request);
}

fn queue_source_phase_dynamic_import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
    phase: ModuleImportPhase,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let specifier = specifier.to_rust_string_lossy(scope);
    queue_native_dynamic_import(
        scope,
        host_defined_options,
        resource_name,
        resolver,
        &specifier,
        import_attributes,
        phase,
    );
    Some(promise)
}

fn dynamic_import_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_defined_options: v8::Local<'s, v8::Data>,
    resource_name: v8::Local<'s, v8::Value>,
    host: &crate::native_bridge::JsContextHost,
) -> Url {
    if let Some(base_url) = script_base_url_from_host_defined_options(scope, host_defined_options) {
        return base_url;
    }
    if let Some(base_url) = scope
        .get_current_host_defined_options()
        .and_then(|options| script_base_url_from_host_defined_options(scope, options))
    {
        return base_url;
    }
    if let Some(base_url) = script_base_url_from_continuation_data(scope) {
        return base_url;
    }
    if let Some(base_url) =
        dynamic_import_base_url_from_compiled_string_resource(scope, resource_name, host)
    {
        return base_url;
    }
    resource_name
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|value| Url::parse(&value).ok())
        .unwrap_or_else(|| host.document_url().clone())
}

fn dynamic_import_referrer_fetch_metadata(
    scope: &mut v8::PinScope<'_, '_>,
    host_defined_options: v8::Local<'_, v8::Data>,
) -> ScriptFetchMetadata {
    let nonce = script_nonce_from_host_defined_options(scope, host_defined_options).or_else(|| {
        scope
            .get_current_host_defined_options()
            .and_then(|options| script_nonce_from_host_defined_options(scope, options))
    });
    let parser_inserted =
        script_parser_inserted_from_host_defined_options(scope, host_defined_options)
            .or_else(|| {
                scope
                    .get_current_host_defined_options()
                    .and_then(|options| {
                        script_parser_inserted_from_host_defined_options(scope, options)
                    })
            })
            .unwrap_or(false);
    ScriptFetchMetadata {
        nonce,
        parser_inserted,
        ..ScriptFetchMetadata::default()
    }
}

fn dynamic_import_base_url_from_compiled_string_resource(
    scope: &mut v8::PinScope<'_, '_>,
    resource_name: v8::Local<'_, v8::Value>,
    host: &crate::native_bridge::JsContextHost,
) -> Option<Url> {
    let resource_url = resource_name
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|value| Url::parse(&value).ok())?;
    host.script_base_url_for_compiled_string_resource(&resource_url)
}

fn dynamic_import_attributes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    import_attributes: v8::Local<'s, v8::FixedArray>,
) -> ModuleAttributesKey {
    let mut pairs = Vec::new();
    let mut index = 0;
    while index + 1 < import_attributes.length() {
        let key = import_attributes
            .get(scope, index)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        let value = import_attributes
            .get(scope, index + 1)
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope));
        if let (Some(key), Some(value)) = (key, value) {
            pairs.push((key, value));
        }
        index += 2;
    }
    ModuleAttributesKey::from_pairs(pairs)
}

fn reject_dynamic_import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    message: &str,
) {
    let exception = v8_string(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, exception);
}

fn reject_unsupported_dynamic_import_phase<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let exception = v8_string(scope, message)
        .map(|message| v8::Exception::syntax_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, exception);
    Some(promise)
}

fn import_meta_resolve_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        throw_type_error(scope, "Module specifier resolution is not available.");
        return;
    };
    let Some(specifier) = callback_arg_string(scope, &args, 0) else {
        throw_type_error(scope, "Module specifier must be a string.");
        return;
    };
    let Some(base_url) = v8::Local::<v8::String>::try_from(args.data())
        .ok()
        .and_then(|value| url::Url::parse(&value.to_rust_string_lossy(scope)).ok())
    else {
        throw_type_error(scope, "Module base URL is invalid.");
        return;
    };
    let resolved =
        unsafe { &mut *host_ptr }.resolve_module_specifier_with_base(&specifier, &base_url);
    match resolved {
        Ok(module_url) => {
            let Some(value) = v8_string(scope, module_url.as_str()) else {
                throw_type_error(scope, "Failed to allocate resolved module URL.");
                return;
            };
            rv.set(value.into());
        }
        Err(error) => throw_type_error(scope, &error),
    }
}
