use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBKeyRange.bound")]
struct IdbKeyRangeBoundArgs<'s> {
    #[webidl(required, converter = "raw")]
    lower: v8::Local<'s, v8::Value>,
    #[webidl(required, converter = "raw")]
    upper: v8::Local<'s, v8::Value>,
    #[webidl(default = false)]
    lower_open: bool,
    #[webidl(default = false)]
    upper_open: bool,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_bound_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbKeyRangeBoundArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(lower) = require_idb_key(scope, parsed.lower) else {
        return;
    };
    let Some(upper) = require_idb_key(scope, parsed.upper) else {
        return;
    };
    if compare_idb_keys(&lower, &upper) > 0 {
        let error = dom_exception_value(
            scope,
            "Failed to execute 'bound': lower must not be greater than upper.",
            "DataError",
        );
        scope.throw_exception(error);
        return;
    }
    let range = IdbKeyRangeQuery {
        lower: Some(lower),
        upper: Some(upper),
        lower_open: parsed.lower_open,
        upper_open: parsed.upper_open,
    };
    if let Some(object) = create_key_range_object(scope, &range) {
        rv.set(object.into());
    } else {
        rv.set_undefined();
    }
}
