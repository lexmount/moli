use super::store::{headers_entries_from_init, initialize_headers_object};
use super::*;
use moli_webidl as webidl;

pub fn headers_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Headers': Please use the 'new' operator.",
        );
        return;
    }

    let obj = args.this();
    let init_arg = args.get(0);
    let entries = match headers_entries_from_init(scope, init_arg) {
        Ok(entries) => entries,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    initialize_headers_object(scope, obj, &entries);

    rv.set(obj.into());
}
