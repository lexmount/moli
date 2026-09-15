use crate::webidl::{PreparedWebIdlCallbackInterface, WebIdlCallbackInterface};
use crate::{
    callback_invocation::{
        SynchronousWebIdlCallbackOutcome, invoke_synchronous_webidl_callback_interface,
    },
    dom::native::NodeType,
    {
        document_runtime::DomHandle,
        native_bridge::{JsContextHost, WindowExecutionContextIdentity},
        util::throw_type_error,
        webidl,
    },
};
use std::fmt;

const FILTER_ACCEPT: u16 = 1;
const FILTER_REJECT: u16 = 2;
const FILTER_SKIP: u16 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TraversalFilterResult {
    Accept,
    Reject,
    Skip,
    Other,
    Exception,
}

/// The callback-interface value owned by one TreeWalker or NodeIterator.
///
/// Traversal owns registration lifetime and reentrancy. The callback kernel
/// owns the rooted callback plus its relevant/incumbent contexts. The renderer
/// identity is deliberately kept beside those engine-neutral facts so a
/// detached or replaced Window cannot re-enter through a still-reachable
/// traversal wrapper.
pub(in crate::native_bridge) struct TraversalFilter {
    callback: WebIdlCallbackInterface,
    execution_context: Option<WindowExecutionContextIdentity>,
}

impl TraversalFilter {
    pub(super) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        runtime_ptr: *mut JsContextHost,
        callback: WebIdlCallbackInterface,
    ) -> Self {
        let prepared = callback.prepare(scope);
        let relevant_context = prepared.relevant_context(scope);
        let execution_context = unsafe { &*runtime_ptr }
            .window_execution_context_identity_for_v8_context(scope, relevant_context);
        Self {
            callback,
            execution_context,
        }
    }

    pub(super) fn prepare(&self, scope: &mut v8::PinScope<'_, '_>) -> PreparedTraversalFilter {
        PreparedTraversalFilter {
            callback: self.callback.prepare(scope),
            execution_context: self.execution_context,
        }
    }
}

impl fmt::Debug for TraversalFilter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TraversalFilter")
            .field("execution_context", &self.execution_context)
            .finish_non_exhaustive()
    }
}

/// An independently rooted traversal callback snapshot.
///
/// The traversal store is no longer borrowed when user code runs. This is
/// required because `acceptNode` may synchronously mutate or re-enter the same
/// TreeWalker/NodeIterator.
pub(super) struct PreparedTraversalFilter {
    callback: PreparedWebIdlCallbackInterface,
    execution_context: Option<WindowExecutionContextIdentity>,
}

impl PreparedTraversalFilter {
    pub(super) fn value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        self.callback.callback(scope).into()
    }
}

pub(super) fn traversal_filter_result(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    filter: Option<&PreparedTraversalFilter>,
    node: DomHandle,
    what_to_show: u32,
) -> TraversalFilterResult {
    if !accepts_what_to_show(runtime_ptr, node, what_to_show) {
        return TraversalFilterResult::Skip;
    }
    call_traversal_filter(scope, runtime_ptr, filter, node)
}

fn accepts_what_to_show(
    runtime_ptr: *mut JsContextHost,
    node: DomHandle,
    what_to_show: u32,
) -> bool {
    let mask = unsafe { &*runtime_ptr }
        .dom_host()
        .node(node)
        .map(node_type_show_mask)
        .unwrap_or_default();
    mask != 0 && (what_to_show & mask) != 0
}

fn node_type_show_mask(node: &crate::dom::native::Node) -> u32 {
    match node.node_type() {
        NodeType::Element => 0x1,
        NodeType::Text => 0x4,
        NodeType::CDataSection => 0x8,
        NodeType::ProcessingInstruction => 0x40,
        NodeType::Comment => 0x80,
        NodeType::Document => 0x100,
        NodeType::DocumentType => 0x200,
        NodeType::DocumentFragment => 0x400,
    }
}

fn call_traversal_filter(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    filter: Option<&PreparedTraversalFilter>,
    node: DomHandle,
) -> TraversalFilterResult {
    let Some(filter) = filter else {
        return TraversalFilterResult::Accept;
    };
    if filter.execution_context.is_some_and(|identity| {
        !unsafe { &*runtime_ptr }.window_execution_context_identity_is_current(identity)
    }) {
        throw_type_error(
            scope,
            "NodeFilter callback belongs to a detached browsing context.",
        );
        return TraversalFilterResult::Exception;
    }
    let Some(node_wrapper) = super::wrapped_traversal_node_value(scope, runtime_ptr, node, node)
    else {
        return TraversalFilterResult::Accept;
    };
    let callback_this = v8::undefined(scope).into();
    let arguments = [node_wrapper];
    match invoke_synchronous_webidl_callback_interface(
        scope,
        &filter.callback,
        callback_this,
        "acceptNode",
        &arguments,
        |scope, value| match webidl::convert::<webidl::UnsignedShort>(
            scope,
            value,
            webidl::Context::member("NodeFilter", "acceptNode"),
        ) {
            Ok(value) => Some(value.0),
            Err(error) => {
                webidl::throw_error(scope, &error);
                None
            }
        },
    ) {
        SynchronousWebIdlCallbackOutcome::Returned(FILTER_ACCEPT) => TraversalFilterResult::Accept,
        SynchronousWebIdlCallbackOutcome::Returned(FILTER_REJECT) => TraversalFilterResult::Reject,
        SynchronousWebIdlCallbackOutcome::Returned(FILTER_SKIP) => TraversalFilterResult::Skip,
        SynchronousWebIdlCallbackOutcome::Returned(_) => TraversalFilterResult::Other,
        SynchronousWebIdlCallbackOutcome::Threw(report) => {
            let Some(exception) = report
                .exception
                .as_ref()
                .map(|exception| v8::Local::new(scope, exception))
            else {
                return TraversalFilterResult::Exception;
            };
            scope.throw_exception(exception);
            TraversalFilterResult::Exception
        }
        SynchronousWebIdlCallbackOutcome::Terminated => TraversalFilterResult::Exception,
    }
}
