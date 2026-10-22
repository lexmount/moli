use crate::dom::native::Node;
use crate::util::throw_type_error;
use crate::{web_api_interfaces, webidl};

use super::super::super::{
    node::{
        node_is_element, node_or_foreign_arg_handle_allow_detached,
        node_runtime_and_handle_from_args_or_detached,
    },
    throw_dom_exception,
};
use super::super::trusted_types::{TrustedHtmlSink, trusted_html_sink_dom_string};
use super::{
    insertion::{
        insert_adjacent_context_handle, insert_adjacent_document_handle, insert_adjacent_handle,
        insert_adjacent_html_fragment_handle,
    },
    position::{InsertAdjacentPosition, parse_insert_adjacent_position},
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.insertAdjacentElement")]
struct InsertAdjacentElementArgs<'s> {
    #[webidl(required)]
    position: String,
    #[webidl(required, interface = web_api_interfaces::Element)]
    element: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.insertAdjacentText")]
struct InsertAdjacentTextArgs {
    #[webidl(required)]
    position: String,
    #[webidl(required, converter = "raw")]
    data: webidl::DomString16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Element.insertAdjacentHTML")]
struct InsertAdjacentHtmlArgs<'s> {
    #[webidl(required)]
    position: String,
    #[webidl(required)]
    text: v8::Local<'s, v8::Value>,
}

fn parse_insert_adjacent_position_or_throw(
    scope: &mut v8::PinScope<'_, '_>,
    value: &str,
) -> Option<InsertAdjacentPosition> {
    let position = parse_insert_adjacent_position(value);
    if position.is_none() {
        throw_dom_exception(
            scope,
            "SyntaxError",
            12,
            "The value provided ('position') is not one of 'beforebegin', 'afterbegin', 'beforeend', or 'afterend'.",
        );
    }
    position
}

fn throw_insert_adjacent_element_type_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_type_error(
        scope,
        "Failed to execute 'insertAdjacentElement' on 'Element': parameter 2 is not of type 'Element'.",
    );
}

pub(in crate::native_bridge) fn node_insert_adjacent_element_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, target)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_null();
        return;
    };
    let Some(parsed) = webidl::parse_args::<InsertAdjacentElementArgs>(scope, &args) else {
        return;
    };
    let Some(position) = parse_insert_adjacent_position_or_throw(scope, &parsed.position) else {
        return;
    };
    let needs_parent = matches!(
        position,
        InsertAdjacentPosition::BeforeBegin | InsertAdjacentPosition::AfterEnd
    );
    if needs_parent
        && unsafe { &*runtime_ptr }
            .dom_host()
            .node(target)
            .and_then(Node::parent_node)
            .is_none()
    {
        rv.set_null();
        return;
    }
    if needs_parent
        && unsafe { &*runtime_ptr }
            .dom_host()
            .node(target)
            .and_then(Node::parent_node)
            .and_then(|parent| unsafe { &*runtime_ptr }.dom_host().node(parent))
            .and_then(Node::as_document)
            .is_some()
    {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    let document_handle =
        insert_adjacent_document_handle(unsafe { &*runtime_ptr }, target, position);
    let Some(handle) = node_or_foreign_arg_handle_allow_detached(
        scope,
        runtime_ptr,
        document_handle,
        parsed.element.into(),
    ) else {
        throw_insert_adjacent_element_type_error(scope);
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        throw_insert_adjacent_element_type_error(scope);
        return;
    }
    if !insert_adjacent_handle(scope, runtime_ptr, target, position, handle) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    // Web IDL retains the original interface argument, including a registered
    // native Proxy. Returning another wrapper would change observable identity.
    rv.set(parsed.element.into());
}

pub(in crate::native_bridge) fn node_insert_adjacent_text_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, target)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<InsertAdjacentTextArgs>(scope, &args) else {
        return;
    };
    let Some(position) = parse_insert_adjacent_position_or_throw(scope, &parsed.position) else {
        return;
    };
    let needs_parent = matches!(
        position,
        InsertAdjacentPosition::BeforeBegin | InsertAdjacentPosition::AfterEnd
    );
    if needs_parent
        && unsafe { &*runtime_ptr }
            .dom_host()
            .node(target)
            .and_then(Node::parent_node)
            .is_none()
    {
        rv.set_undefined();
        return;
    }
    if needs_parent
        && unsafe { &*runtime_ptr }
            .dom_host()
            .node(target)
            .and_then(Node::parent_node)
            .and_then(|parent| unsafe { &*runtime_ptr }.dom_host().node(parent))
            .and_then(Node::as_document)
            .is_some()
    {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    let document = unsafe { &*runtime_ptr }
        .dom_host()
        .owner_document_handle(target);
    let handle = unsafe { &mut *runtime_ptr }
        .create_text_node_from_utf16_units(document, parsed.data.0.into());
    if !insert_adjacent_handle(scope, runtime_ptr, target, position, handle) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_insert_adjacent_node_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, target)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_bool(false);
        return;
    };
    let Some(position) = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        rv.set_bool(false);
        return;
    };
    let Some(position) = parse_insert_adjacent_position_or_throw(scope, &position) else {
        return;
    };
    let needs_parent = matches!(
        position,
        InsertAdjacentPosition::BeforeBegin | InsertAdjacentPosition::AfterEnd
    );
    if needs_parent
        && unsafe { &*runtime_ptr }
            .dom_host()
            .node(target)
            .and_then(Node::parent_node)
            .is_none()
    {
        rv.set_bool(false);
        return;
    }
    if needs_parent
        && unsafe { &*runtime_ptr }
            .dom_host()
            .node(target)
            .and_then(Node::parent_node)
            .and_then(|parent| unsafe { &*runtime_ptr }.dom_host().node(parent))
            .and_then(Node::as_document)
            .is_some()
    {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    let document_handle =
        insert_adjacent_document_handle(unsafe { &*runtime_ptr }, target, position);
    let Some(handle) =
        node_or_foreign_arg_handle_allow_detached(scope, runtime_ptr, document_handle, args.get(1))
    else {
        rv.set_bool(false);
        return;
    };
    if !insert_adjacent_handle(scope, runtime_ptr, target, position, handle) {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    rv.set_bool(true);
}

pub(in crate::native_bridge) fn node_insert_adjacent_html_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, target)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let Some(parsed) = webidl::parse_args::<InsertAdjacentHtmlArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(text) = trusted_html_sink_dom_string(
        scope,
        runtime_ptr,
        parsed.text,
        TrustedHtmlSink::ElementInsertAdjacentHtml,
    ) else {
        return;
    };
    let Some(position) = parse_insert_adjacent_position_or_throw(scope, &parsed.position) else {
        return;
    };
    let needs_parent = matches!(
        position,
        InsertAdjacentPosition::BeforeBegin | InsertAdjacentPosition::AfterEnd
    );
    if needs_parent {
        let parent = unsafe { &*runtime_ptr }
            .dom_host()
            .node(target)
            .and_then(Node::parent_node);
        let has_no_modifiable_parent = parent.is_none_or(|parent| {
            unsafe { &*runtime_ptr }
                .dom_host()
                .node(parent)
                .is_none_or(Node::is_document)
        });
        if has_no_modifiable_parent {
            throw_dom_exception(
                scope,
                "NoModificationAllowedError",
                7,
                "The element has no modifiable parent for sibling insertion.",
            );
            return;
        }
    }
    let Some(document_handle) =
        insert_adjacent_document_handle(unsafe { &*runtime_ptr }, target, position)
    else {
        rv.set_undefined();
        return;
    };
    let Some(context_handle) =
        insert_adjacent_context_handle(unsafe { &*runtime_ptr }, target, position)
    else {
        rv.set_undefined();
        return;
    };
    let inserted =
        crate::custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
            unsafe { &mut *runtime_ptr }.insert_adjacent_html(
                scope,
                runtime_ptr,
                document_handle,
                context_handle,
                &text,
                |_runtime, scope, host_ptr, fragment| {
                    insert_adjacent_html_fragment_handle(
                        scope, host_ptr, target, position, fragment,
                    )
                },
            )
        });
    let inserted = match inserted {
        Ok(inserted) => inserted,
        Err(error) => {
            throw_dom_exception(scope, "SyntaxError", 12, &error.to_string());
            return;
        }
    };
    if !inserted {
        throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
        return;
    }
    rv.set_undefined();
}
