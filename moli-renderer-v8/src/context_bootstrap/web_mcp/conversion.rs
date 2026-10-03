use super::bindings::{dom_error, js_string};
use super::state::{DocumentTools, RegisteredTool};
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

use crate::{abort_signal_route::ResolvedAbortSignal, webidl};

#[derive(Clone, Copy, Default, PartialEq, Eq, webidl::WebIdlDictionary)]
#[webidl(prefix = "ToolAnnotations")]
pub(super) struct ToolAnnotations {
    #[webidl(name = "consequentialHint", default = false)]
    pub(super) consequential: bool,
    #[webidl(default = false)]
    pub(super) debugging: bool,
    #[webidl(name = "readOnlyHint", default = false)]
    pub(super) read_only: bool,
    #[webidl(name = "untrustedContentHint", default = false)]
    pub(super) untrusted_content: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ModelContextTool")]
pub(super) struct ToolDefinition<'s> {
    #[webidl(with = annotations_member)]
    pub(super) annotations: Option<ToolAnnotations>,
    #[webidl(required, converter = "dom_string")]
    pub(super) description: String,
    #[webidl(required, converter = "callback_function")]
    pub(super) execute: webidl::WebIdlCallbackFunction,
    #[webidl(name = "inputSchema")]
    pub(super) input_schema: Option<v8::Local<'s, v8::Object>>,
    #[webidl(required, converter = "dom_string")]
    pub(super) name: String,
    #[webidl(converter = "usv_string")]
    pub(super) title: Option<String>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ModelContextRegisterToolOptions")]
pub(super) struct RegisterOptions<'s> {
    #[webidl(name = "exposedTo", converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) exposed_to: webidl::Sequence<webidl::UsvString>,
    #[webidl(with = signal_member)]
    pub(super) signal: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ModelContextGetToolOptions")]
pub(super) struct GetOptions {
    #[webidl(name = "fromOrigins", converter = "raw", default = webidl::Sequence(Vec::new()))]
    pub(super) from_origins: webidl::Sequence<webidl::UsvString>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ExecuteToolOptions")]
pub(super) struct ExecuteOptions<'s> {
    #[webidl(with = signal_member)]
    pub(super) signal: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RegisteredTool")]
pub(super) struct ToolReference<'s> {
    #[webidl(with = annotations_member)]
    pub(super) annotations: Option<ToolAnnotations>,
    #[webidl(required, converter = "dom_string")]
    pub(super) description: String,
    #[webidl(name = "inputSchema")]
    pub(super) input_schema: Option<v8::Local<'s, v8::Object>>,
    #[webidl(required, converter = "dom_string")]
    pub(super) name: String,
    #[webidl(required, converter = "usv_string")]
    pub(super) origin: String,
    #[webidl(converter = "dom_string")]
    pub(super) title: Option<String>,
    #[webidl(with = window_member)]
    pub(super) window: v8::Local<'s, v8::Object>,
}

pub(super) fn dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    webidl::dictionary_arg(
        args,
        index,
        webidl::Context::argument(prefix, (index + 1) as usize),
    )
    .map(|value| value.unwrap_or_else(|| crate::util::new_null_prototype_object(scope)))
}

fn annotations_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<ToolAnnotations>, webidl::WebIdlError> {
    let context = webidl::Context::member("ModelContextTool", name);
    let Some(value) = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined())
    else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(Some(ToolAnnotations::default()));
    }
    let dictionary = webidl::convert::<v8::Local<v8::Object>>(scope, value, context)?;
    webidl::parse_dictionary_object::<ToolAnnotations>(scope, dictionary).map(Some)
}

fn signal_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    let signal = webidl::optional_member(
        scope,
        object,
        name,
        webidl::Context::member("ModelContextTool", name),
    )?;
    if signal.is_some_and(|signal| ResolvedAbortSignal::resolve(scope, signal).is_none()) {
        return Err(webidl::WebIdlError::cannot_convert(
            webidl::Context::member("ModelContextOptions", name),
            "AbortSignal",
        ));
    }
    Ok(signal)
}

fn window_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    let context = webidl::Context::member("RegisteredTool", name);
    let window = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined())
        .ok_or_else(|| webidl::WebIdlError::missing_required(context))?;
    let window = webidl::convert::<v8::Local<v8::Object>>(scope, window, context)?;
    if !web_api_interfaces::Window::is_instance(scope, window)
        && crate::native_bridge::lightweight_popup_id_from_window(scope, window).is_none()
    {
        return Err(webidl::WebIdlError::cannot_convert(context, "Window"));
    }
    Ok(window)
}

pub(super) fn validate_origins<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    strings: webidl::Sequence<webidl::UsvString>,
) -> Result<Vec<url::Origin>, webidl::WebIdlError> {
    let context = webidl::Context::member("ModelContextOptions", "origins");
    strings
        .0
        .into_iter()
        .map(|value| {
            let origin = url::Url::parse(&value.0).ok();
            if !origin.as_ref().is_some_and(|url| {
                moli_url::is_potentially_trustworthy_url(url)
                    && matches!(url.origin(), url::Origin::Tuple(..))
            }) {
                // The algorithm reports SecurityError, rather than accepting insecure
                // or opaque origins merely because the calling realm is secure.
                let error = dom_error(
                    scope,
                    "SecurityError",
                    "Only secure origins are allowed in the origins list.",
                );
                scope.throw_exception(error);
                return Err(webidl::WebIdlError::pending_exception(context));
            }
            Ok(origin.expect("validated origin").origin())
        })
        .collect()
}

pub(super) fn stringify_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<String, webidl::WebIdlError> {
    let context = webidl::Context::member("ModelContext", "input");
    let value = v8::json::stringify(scope, object.into())
        .ok_or_else(|| webidl::WebIdlError::pending_exception(context))?;
    let value = value.to_rust_string_lossy(scope);
    if value == "undefined" {
        return Err(webidl::WebIdlError::custom_message(
            "toJSON() returned undefined",
        ));
    }
    Ok(value)
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable, scope_lifetime = 's, no_dynamic_constructor)]
struct ToolSnapshot<'s, 'metadata> {
    name: &'metadata str,
    description: &'metadata str,
    title: &'metadata str,
    origin: String,
    window: v8::Local<'s, v8::Object>,
    input_schema: Option<v8::Local<'s, v8::Object>>,
    annotations: Option<AnnotationSnapshot>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct AnnotationSnapshot {
    read_only_hint: bool,
    consequential_hint: bool,
    untrusted_content_hint: bool,
    debugging: bool,
}

pub(super) fn tool_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: &DocumentTools,
    name: &str,
    tool: &RegisteredTool,
) -> Option<v8::Local<'s, v8::Object>> {
    let metadata = &tool.metadata;
    let schema = if let Some(schema) = &metadata.input_schema {
        let value = v8::json::parse(scope, js_string(scope, schema))?;
        Some(v8::Local::<v8::Object>::try_from(value).ok()?)
    } else {
        None
    };
    ToolSnapshot {
        name,
        description: &metadata.description,
        title: &metadata.title,
        origin: document.origin.ascii_serialization(),
        window: v8::Local::new(scope, &document.window),
        input_schema: schema,
        annotations: metadata.annotations.map(|annotations| AnnotationSnapshot {
            read_only_hint: annotations.read_only,
            consequential_hint: annotations.consequential,
            untrusted_content_hint: annotations.untrusted_content,
            debugging: annotations.debugging,
        }),
    }
    .bind(scope)
    .ok()
}
