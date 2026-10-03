use super::super::super::node::node_runtime_and_handle_from_object_or_detached;
use super::super::{has_reflected_attribute, remove_reflected_attribute, set_reflected_attribute};

pub(in crate::native_bridge) fn link_disabled_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    rv.set(
        v8::Boolean::new(
            scope,
            has_reflected_attribute(unsafe { &*runtime_ptr }, handle, "disabled"),
        )
        .into(),
    );
}

pub(in crate::native_bridge) fn link_disabled_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let owner = args.this();
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, owner)
    else {
        rv.set_undefined();
        return;
    };
    let disabled = args.get(0).boolean_value(scope);
    if disabled {
        set_reflected_attribute(scope, runtime_ptr, handle, "disabled", "");
        rv.set_undefined();
        return;
    }

    if unsafe { &*runtime_ptr }
        .dom_host()
        .get_attribute(handle, "disabled")
        .is_none()
    {
        rv.set_undefined();
        return;
    }
    remove_reflected_attribute(scope, runtime_ptr, handle, "disabled");
    rv.set_undefined();
}
