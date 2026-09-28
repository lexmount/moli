use super::*;
use crate::{context_bootstrap::exposed_interfaces::TemplateBuildProfile, web_api_interfaces};

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSStyleValue, enumerable)]
struct CssStyleValueConstructorDeclaration {
    #[webapi(static_method, callback = parse_callback, length = 2)]
    parse: (),
    #[webapi(static_method = "parseAll", callback = parse_all_callback, length = 2)]
    parse_all: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CSSStyleValue.parse")]
struct ParseArgs {
    #[webidl(required, converter = "usv_string")]
    property: String,
    #[webidl(required, converter = "usv_string")]
    css_text: String,
}

pub(super) fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
    profile: TemplateBuildProfile,
) {
    if interface_name == "CSSStyleValue" && profile == TemplateBuildProfile::Window {
        CssStyleValueConstructorDeclaration::initialize_template(scope, template);
    }
}

fn parse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(values) = parse_values(scope, &args)
        && let Some(value) = values.first()
    {
        rv.set((*value).into());
    }
}

fn parse_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(values) = parse_values(scope, &args) {
        let values = values
            .into_iter()
            .map(v8::Local::<v8::Value>::from)
            .collect::<Vec<_>>();
        rv.set(v8::Array::new_with_elements(scope, &values).into());
    }
}

fn parse_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    // Convert both arguments before property/grammar validation (and propagate
    // exceptions unchanged). Static WebIDL operations do not check their this.
    let arguments = webidl::parse_args::<ParseArgs>(scope, args)?;
    let property = if arguments.property.starts_with("--") {
        arguments.property
    } else {
        arguments.property.to_ascii_lowercase()
    };
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let base_url = base_url(scope);
    let Some(parsed) =
        moli_css_parse::parse_typed_style_value(&property, &arguments.css_text, base_url.as_ref())
    else {
        throw_type_error(scope, "Invalid CSS property or value");
        return None;
    };
    Some(values::from_parsed(
        scope,
        &property,
        &arguments.css_text,
        parsed,
    ))
}

pub(super) fn base_url(scope: &mut v8::PinScope<'_, '_>) -> Option<url::Url> {
    let host = unsafe { &*crate::util::context_host_ptr_from_global_bridge(scope)? };
    let binding = host.current_runtime_window_execution_context_binding(scope)?;
    let loader = host.document_resource_loader_for_dispatch_scope(binding.dispatch_scope())?;
    Some(
        host.subresource_request_environment(&loader, binding.dispatch_scope())?
            .base_url,
    )
}
