use std::borrow::Cow;

use super::super::*;
use super::XPathEvaluationError;
use super::evaluation::{evaluate_xpath_over_live_dom, evaluate_xpath_over_object_tree};
use super::expression::{expression_for_receiver, new_xpath_expression};
use super::resolver::V8XPathNamespaceResolver;
use super::result::is_supported_xpath_result_type;
use crate::native_bridge::{
    document::detached_tree_root_object, node_runtime_and_handle_from_object,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XPathEvaluator, enumerable, receiver)]
struct XPathEvaluatorPrototypeDeclaration {
    #[webapi(method, length = 1, callback = xpath_evaluator_create_expression_callback)]
    create_expression: (),
    #[webapi(method, length = 2, callback = xpath_evaluator_evaluate_callback)]
    evaluate: (),
    #[webapi(
        method = "createNSResolver",
        length = 1,
        callback = xpath_evaluator_create_ns_resolver_callback
    )]
    create_ns_resolver: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XPathExpression, enumerable, receiver)]
struct XPathExpressionPrototypeDeclaration {
    #[webapi(method, length = 1, callback = xpath_expression_evaluate_callback)]
    evaluate: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createExpression")]
struct DocumentCreateExpressionArgs {
    #[webidl(required)]
    expression: String,
    #[webidl(index = 1, converter = "callback_interface", nullable)]
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XPathEvaluator.createExpression")]
struct XPathEvaluatorCreateExpressionArgs {
    #[webidl(required)]
    expression: String,
    #[webidl(index = 1, converter = "callback_interface", nullable)]
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XPathExpression.evaluate")]
struct XPathExpressionEvaluateArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    context_node: v8::Local<'s, v8::Object>,
    #[webidl(index = 1, default = 0)]
    result_type: u16,
    #[webidl(index = 2, nullable)]
    _existing_result: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.evaluate")]
struct DetachedDocumentEvaluateArgs<'s> {
    #[webidl(index = 1, required)]
    expression: String,
    #[webidl(index = 2, interface = web_api_interfaces::Node)]
    context_node: v8::Local<'s, v8::Object>,
    #[webidl(index = 3, converter = "callback_interface", nullable)]
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
    #[webidl(index = 4, default = 0)]
    result_type: u16,
    #[webidl(index = 5, nullable)]
    _existing_result: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.evaluate")]
struct DocumentEvaluateArgs<'s> {
    #[webidl(required)]
    expression: String,
    #[webidl(index = 1, interface = web_api_interfaces::Node)]
    context_node: v8::Local<'s, v8::Object>,
    #[webidl(index = 2, converter = "callback_interface", nullable)]
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
    #[webidl(index = 3, default = 0)]
    result_type: u16,
    #[webidl(index = 4, nullable)]
    _existing_result: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XPathEvaluator.evaluate")]
struct XPathEvaluatorEvaluateArgs<'s> {
    #[webidl(required)]
    expression: String,
    #[webidl(index = 1, interface = web_api_interfaces::Node)]
    context_node: v8::Local<'s, v8::Object>,
    #[webidl(index = 2, converter = "callback_interface", nullable)]
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
    #[webidl(index = 3, default = 0)]
    result_type: u16,
    #[webidl(index = 4, nullable)]
    _existing_result: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.createNSResolver")]
struct DocumentCreateNsResolverArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    node_resolver: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XPathEvaluator.createNSResolver")]
struct XPathEvaluatorCreateNsResolverArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Node)]
    node_resolver: v8::Local<'s, v8::Object>,
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

pub(super) fn install_xpath_expression_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    XPathExpressionPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

enum XPathInput<'a> {
    Source(&'a str, Option<webidl::WebIdlCallbackInterface>),
    Compiled(&'a moli_xpath::Expression),
}

fn compile_xpath(
    scope: &mut v8::PinScope<'_, '_>,
    expression: &str,
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
) -> Result<moli_xpath::Expression, XPathEvaluationError> {
    let namespace_resolver =
        namespace_resolver.map(|callback| V8XPathNamespaceResolver::new(scope, callback));
    // Preserve local-name case: the evaluated node's document determines HTML
    // matching, even when this expression is created by an HTML Document.
    moli_xpath::parse(expression, namespace_resolver, false).map_err(|error| match error {
        moli_xpath::ParserError::FailedToResolveNamespacePrefix => XPathEvaluationError::Namespace,
        _ => XPathEvaluationError::InvalidExpression,
    })
}

fn evaluate_xpath<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    receiver: v8::Local<'a, v8::Object>,
    input: XPathInput<'_>,
    context_node: v8::Local<'a, v8::Object>,
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

    let Some(context) = receiver.get_creation_context(scope) else {
        rv.set_null();
        return;
    };
    let result = {
        let scope = &mut v8::ContextScope::new(scope, context);
        // Resolver callbacks may adopt or mutate the context node. Select its
        // current tree only after compilation, with no DOM borrow or snapshot
        // retained across author code. Compiled expressions reuse their AST.
        let expression = match input {
            XPathInput::Source(source, resolver) => {
                compile_xpath(scope, source, resolver).map(Cow::Owned)
            }
            XPathInput::Compiled(expression) => Ok(Cow::Borrowed(expression)),
        };
        expression.and_then(|expression| {
            evaluate_parsed_xpath_for_context(
                scope,
                &expression,
                context_node,
                requested_result_type,
            )
        })
    };
    match result {
        Ok(Some(result)) => rv.set(result.into()),
        Ok(None) => rv.set_null(),
        Err(error) => throw_xpath_evaluation_error(scope, error),
    }
}

fn evaluate_parsed_xpath_for_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    expression: &moli_xpath::Expression,
    context_node: v8::Local<'s, v8::Object>,
    requested_result_type: u32,
) -> Result<Option<v8::Local<'s, v8::Object>>, XPathEvaluationError> {
    if let Ok((runtime_ptr, context_handle)) =
        node_runtime_and_handle_from_object(scope, context_node)
    {
        evaluate_xpath_over_live_dom(
            scope,
            runtime_ptr,
            expression,
            context_handle,
            requested_result_type,
        )
    } else {
        let root = detached_tree_root_object(scope, context_node).unwrap_or(context_node);
        evaluate_xpath_over_object_tree(
            scope,
            root,
            expression,
            Some(context_node),
            requested_result_type,
        )
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
        XPathInput::Source(&parsed.expression, parsed.namespace_resolver),
        parsed.context_node,
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
    evaluate_xpath(
        scope,
        root,
        XPathInput::Source(&parsed.expression, parsed.namespace_resolver),
        parsed.context_node,
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
    evaluate_xpath(
        scope,
        args.this(),
        XPathInput::Source(&parsed.expression, parsed.namespace_resolver),
        parsed.context_node,
        u32::from(parsed.result_type),
        rv,
    );
}

fn create_xpath_expression<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    expression: &str,
    namespace_resolver: Option<webidl::WebIdlCallbackInterface>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(context) = receiver.get_creation_context(scope) else {
        return;
    };
    let result = {
        let scope = &mut v8::ContextScope::new(scope, context);
        compile_xpath(scope, expression, namespace_resolver)
            .map(|expression| new_xpath_expression(scope, expression))
    };
    match result {
        Ok(Some(expression)) => rv.set(expression.into()),
        Ok(None) => {}
        Err(error) => throw_xpath_evaluation_error(scope, error),
    }
}

pub(in crate::native_bridge) fn node_document_create_expression_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentCreateExpressionArgs>(scope, &args) else {
        return;
    };
    create_xpath_expression(
        scope,
        args.this(),
        &parsed.expression,
        parsed.namespace_resolver,
        rv,
    );
}

fn xpath_evaluator_create_expression_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<XPathEvaluatorCreateExpressionArgs>(scope, &args)
    else {
        return;
    };
    create_xpath_expression(
        scope,
        args.this(),
        &parsed.expression,
        parsed.namespace_resolver,
        rv,
    );
}

fn xpath_expression_evaluate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<XPathExpressionEvaluateArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(expression) = expression_for_receiver(scope, args.this()) else {
        return;
    };
    evaluate_xpath(
        scope,
        args.this(),
        XPathInput::Compiled(&expression),
        parsed.context_node,
        u32::from(parsed.result_type),
        rv,
    );
}
