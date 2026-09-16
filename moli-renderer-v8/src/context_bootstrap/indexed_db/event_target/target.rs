use super::*;

pub(crate) fn dispatch_indexed_db_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_idb_event_target(scope, args.this()) {
        return;
    }
    let target = args.this();
    super::dispatch::dispatch_indexed_db_script_event(scope, args, target, rv);
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
