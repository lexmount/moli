mod callbacks;
mod evaluation;
mod expression;
mod live_dom;
mod resolver;
mod result;
mod snapshot;
mod types;

#[cfg(feature = "devtools")]
pub(crate) use live_dom::evaluate_live_xpath_search_node_handles;

pub(in crate::native_bridge) use callbacks::{
    bridge_detached_document_evaluate_callback, node_document_create_expression_callback,
    node_document_create_ns_resolver_callback, node_document_evaluate_callback,
};

pub(crate) fn install_xpath_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    match interface_name {
        "XPathEvaluator" => callbacks::install_xpath_evaluator_template_bindings(scope, template),
        "XPathExpression" => callbacks::install_xpath_expression_template_bindings(scope, template),
        "XPathResult" => result::install_xpath_result_template_bindings(scope, template),
        _ => {}
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum XPathEvaluationError {
    InvalidExpression,
    Namespace,
}
