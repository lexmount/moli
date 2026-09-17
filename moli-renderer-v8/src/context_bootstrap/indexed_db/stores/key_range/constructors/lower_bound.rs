use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBKeyRange.lowerBound")]
struct IdbKeyRangeLowerBoundArgs<'s> {
    #[webidl(required, converter = "raw")]
    lower: v8::Local<'s, v8::Value>,
    #[webidl(default = false)]
    open: bool,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_lower_bound_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbKeyRangeLowerBoundArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(lower) = require_idb_key(scope, parsed.lower) else {
        return;
    };
    let range = IdbKeyRangeQuery {
        lower: Some(lower),
        upper: None,
        lower_open: parsed.open,
        upper_open: false,
    };
    if let Some(object) = create_key_range_object(scope, &range) {
        rv.set(object.into());
    } else {
        rv.set_undefined();
    }
}
