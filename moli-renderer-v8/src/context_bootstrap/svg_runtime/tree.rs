use crate::{
    native_bridge::{
        document::SVG_NS, node_runtime_and_handle_from_object_or_detached,
        set_wrapped_handle_or_null_for_receiver,
    },
    webidl,
};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGSVGElement.getElementById")]
struct GetElementByIdArgs {
    #[webidl(required, converter = "dom_string16")]
    element_id: Vec<u16>,
}

pub(super) fn get_element_by_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<GetElementByIdArgs>(scope, &args) else {
        return;
    };
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated receiver check validates native SVGSVGElement identity");
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        rv.set_null();
        return;
    };
    let result = unsafe { &*runtime_ptr }
        .dom_host()
        .element_handle_by_id_in_descendants_utf16(handle, &parsed.element_id);
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, receiver, result);
}

pub(super) fn viewport_element_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated receiver check validates native SVGElement identity");
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        rv.set_null();
        return;
    };
    let host = unsafe { &*runtime_ptr }.dom_host();
    let Some(node) = host.node(handle) else {
        rv.set_null();
        return;
    };
    // An SVG fragment nested in HTML or foreignObject has its own outermost SVG.
    if node.local_name() == Some("svg")
        && node
            .parent_node_id()
            .and_then(|parent| host.node(parent))
            .is_none_or(|parent| {
                parent.namespace() != Some(SVG_NS) || parent.local_name() == Some("foreignObject")
            })
    {
        rv.set_null();
        return;
    }
    let mut current = node.parent_node_id();
    let mut viewport = None;
    while let Some(candidate) = current {
        let Some(ancestor) = host.node(candidate) else {
            break;
        };
        if ancestor.namespace() == Some(SVG_NS)
            && matches!(ancestor.local_name(), Some("svg" | "symbol" | "image"))
        {
            viewport = Some(candidate);
            break;
        }
        current = ancestor
            .parent_node_id()
            .or_else(|| host.shadow_root_host(candidate));
    }
    set_wrapped_handle_or_null_for_receiver(scope, &mut rv, runtime_ptr, receiver, viewport);
}
