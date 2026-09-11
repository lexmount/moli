use super::super::*;
use super::XPathEvaluationError;
use super::evaluation::{evaluate_xpath_over_live_dom, evaluate_xpath_over_object_tree};
use super::resolver::V8XPathNamespaceResolver;
use super::result::is_supported_xpath_result_type;
use crate::native_bridge::{
    document::{detached_node_type, detached_tree_root_object},
    node_runtime_and_handle_from_object,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XPathEvaluator, enumerable)]
struct XPathEvaluatorPrototypeDeclaration {
    #[webapi(method, length = 2, callback = xpath_evaluator_evaluate_callback)]
    evaluate: (),
    #[webapi(
        method = "createNSResolver",
        length = 1,
        callback = xpath_evaluator_create_ns_resolver_callback
    )]
    create_ns_resolver: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.evaluate")]
struct DetachedDocumentEvaluateArgs<'s> {
    #[webidl(index = 1, required)]
    expression: String,
    #[webidl(index = 2, with = document_evaluate_context_node_arg)]
    context_node: v8::Local<'s, v8::Object>,
    #[webidl(index = 3, converter = "callback_interface", nullable)]
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
    #[webidl(index = 4, default = 0)]
    result_type: u16,
    #[webidl(index = 5, with = document_evaluate_existing_result_arg)]
    _existing_result: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.evaluate")]
struct DocumentEvaluateArgs<'s> {
    #[webidl(required)]
    expression: String,
    #[webidl(index = 1, with = document_evaluate_context_node_arg)]
    context_node: v8::Local<'s, v8::Object>,
    #[webidl(index = 2, converter = "callback_interface", nullable)]
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
    #[webidl(index = 3, default = 0)]
    result_type: u16,
    #[webidl(index = 4, with = document_evaluate_existing_result_arg)]
    _existing_result: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XPathEvaluator.evaluate")]
struct XPathEvaluatorEvaluateArgs<'s> {
    #[webidl(required)]
    expression: String,
    #[webidl(index = 1, with = document_evaluate_context_node_arg)]
    context_node: v8::Local<'s, v8::Object>,
    #[webidl(index = 2, converter = "callback_interface", nullable)]
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
    #[webidl(index = 3, default = 0)]
    result_type: u16,
    #[webidl(index = 4, with = document_evaluate_existing_result_arg)]
    _existing_result: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createNSResolver")]
struct DocumentCreateNsResolverArgs<'s> {
    #[webidl(required, with = document_evaluate_context_node_arg)]
    node_resolver: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XPathEvaluator.createNSResolver")]
struct XPathEvaluatorCreateNsResolverArgs<'s> {
    #[webidl(required, with = document_evaluate_context_node_arg)]
    node_resolver: v8::Local<'s, v8::Object>,
}

fn document_evaluate_context_node_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    let value = args.get(index);
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(webidl::WebIdlError::custom_message(
            "Document.evaluate requires a context Node",
        ));
    };
    if detached_node_type(scope, object).is_some_and(|node_type| node_type > 0) {
        Ok(object)
    } else {
        Err(webidl::WebIdlError::custom_message(
            "Document.evaluate requires a context Node",
        ))
    }
}

fn document_evaluate_existing_result_arg<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    if args.length() <= index {
        return Ok(None);
    }
    let value = args.get(index);
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    v8::Local::<v8::Object>::try_from(value)
        .map(Some)
        .map_err(|_| {
            webidl::WebIdlError::custom_message(
                "Document.evaluate existing result must be an object or null",
            )
        })
}

fn throw_xpath_evaluation_error(scope: &mut v8::PinScope<'_, '_>, error: XPathEvaluationError) {
    match error {
        XPathEvaluationError::Namespace => throw_dom_exception(
            scope,
            "NamespaceError",
            14,
            "The XPath expression contains an unresolvable namespace prefix",
        ),
        XPathEvaluationError::InvalidExpression => throw_dom_exception(
            scope,
            "SyntaxError",
            12,
            "Failed to evaluate XPath expression",
        ),
    }
}

pub(super) fn install_xpath_evaluator_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    XPathEvaluatorPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

#[allow(clippy::too_many_arguments)]
fn evaluate_xpath<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    root: v8::Local<'a, v8::Object>,
    live_runtime_ptr: Option<*mut JsContextHost>,
    expression: &str,
    context_node: v8::Local<'a, v8::Object>,
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
    requested_result_type: u32,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_supported_xpath_result_type(requested_result_type) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Unsupported XPath result type",
        );
        return;
    }

    if let Some(runtime_ptr) = live_runtime_ptr
        && let Some(context_handle) = node_arg_handle(scope, runtime_ptr, context_node.into())
    {
        let namespace_resolver =
            namespace_resolver.map(|callback| V8XPathNamespaceResolver::new(scope, callback));
        match evaluate_xpath_over_live_dom(
            scope,
            runtime_ptr,
            expression,
            context_handle,
            namespace_resolver,
            requested_result_type,
        ) {
            Ok(Some(result)) => rv.set(result.into()),
            Ok(None) => rv.set_null(),
            Err(error) => throw_xpath_evaluation_error(scope, error),
        }
        return;
    }

    let namespace_resolver =
        namespace_resolver.map(|callback| V8XPathNamespaceResolver::new(scope, callback));
    match evaluate_xpath_over_object_tree(
        scope,
        root,
        expression,
        Some(context_node),
        namespace_resolver,
        requested_result_type,
    ) {
        Ok(Some(result)) => rv.set(result.into()),
        Ok(None) => rv.set_null(),
        Err(error) => throw_xpath_evaluation_error(scope, error),
    }
}

pub(in crate::native_bridge) fn node_document_create_ns_resolver_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentCreateNsResolverArgs<'a>>(scope, &args) else {
        return;
    };
    rv.set(parsed.node_resolver.into());
}

fn xpath_evaluator_create_ns_resolver_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<XPathEvaluatorCreateNsResolverArgs<'a>>(scope, &args)
    else {
        return;
    };
    rv.set(parsed.node_resolver.into());
}

pub(in crate::native_bridge) fn bridge_detached_document_evaluate_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(root) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<DetachedDocumentEvaluateArgs<'a>>(scope, &args) else {
        return;
    };
    evaluate_xpath(
        scope,
        root,
        None,
        &parsed.expression,
        parsed.context_node,
        parsed.namespace_resolver,
        u32::from(parsed.result_type),
        rv,
    );
}

pub(in crate::native_bridge) fn node_document_evaluate_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let root = args.this();
    let Some(parsed) = webidl::parse_args::<DocumentEvaluateArgs<'a>>(scope, &args) else {
        return;
    };
    let live_runtime_ptr = node_runtime_and_handle_from_object(scope, root)
        .ok()
        .map(|(runtime_ptr, _)| runtime_ptr);
    evaluate_xpath(
        scope,
        root,
        live_runtime_ptr,
        &parsed.expression,
        parsed.context_node,
        parsed.namespace_resolver,
        u32::from(parsed.result_type),
        rv,
    );
}

fn xpath_evaluator_evaluate_callback<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    args: v8::FunctionCallbackArguments<'a>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<XPathEvaluatorEvaluateArgs<'a>>(scope, &args) else {
        return;
    };
    let live_runtime_ptr = node_runtime_and_handle_from_object(scope, parsed.context_node)
        .ok()
        .map(|(runtime_ptr, _)| runtime_ptr);
    let root = if live_runtime_ptr.is_some() {
        parsed.context_node
    } else {
        detached_tree_root_object(scope, parsed.context_node).unwrap_or(parsed.context_node)
    };
    evaluate_xpath(
        scope,
        root,
        live_runtime_ptr,
        &parsed.expression,
        parsed.context_node,
        parsed.namespace_resolver,
        u32::from(parsed.result_type),
        rv,
    );
}
