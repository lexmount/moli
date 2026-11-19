pub(super) use moli_webapi::web_mcp_values::{
    ExecuteOptions, GetOptions, RegisterOptions, ToolAnnotations, ToolDefinition, ToolReference,
};

use super::bindings::{dom_error, js_string};
use super::state::{DocumentTools, RegisteredTool};
use crate::web_api_interfaces;
use crate::webidl::WebIdlDictionarySerialize;

use crate::{abort_signal_route::ResolvedAbortSignal, webidl};

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

// Both option dictionaries convert signal last, before the runtime acts on the result.
pub(super) fn validate_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    signal: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), webidl::WebIdlError> {
    if signal.is_some_and(|signal| ResolvedAbortSignal::resolve(scope, signal).is_none()) {
        return Err(webidl::WebIdlError::cannot_convert(
            webidl::Context::member("ModelContextOptions", "signal"),
            "AbortSignal",
        ));
    }
    Ok(())
}

pub(super) fn validate_reference_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Result<(), webidl::WebIdlError> {
    if !web_api_interfaces::Window::is_instance(scope, window)
        && crate::native_bridge::lightweight_popup_id_from_window(scope, window).is_none()
    {
        return Err(webidl::WebIdlError::cannot_convert(
            webidl::Context::member("RegisteredTool", "window"),
            "Window",
        ));
    }
    Ok(())
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
    ToolReference {
        name: name.to_owned(),
        description: metadata.description.clone(),
        title: Some(metadata.title.clone()),
        origin: document.origin.ascii_serialization(),
        window: v8::Local::new(scope, &document.window),
        input_schema: schema,
        annotations: metadata.annotations,
    }
    .to_js_snapshot(scope, webidl::SnapshotPrototype::Ordinary)
}
