use super::*;

pub(crate) fn dispatch_indexed_db_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_idb_event_target(scope, args.this()) {
        return;
    }
    let Some((event, _)) = crate::context_bootstrap::event_target_dispatch::prepare_script_dispatch(
        scope,
        args.get(0),
    ) else {
        return;
    };
    rv.set_bool(dispatch_idb_event_object(scope, args.this(), event));
}

fn require_idb_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> bool {
    if indexed_db_typed_owner_scope(scope, target).is_some() {
        return true;
    }
    throw_type_error(scope, "Illegal invocation");
    false
}
