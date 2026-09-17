use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBKeyRange.upperBound")]
struct IdbKeyRangeUpperBoundArgs<'s> {
    #[webidl(required, converter = "raw")]
    upper: v8::Local<'s, v8::Value>,
    #[webidl(default = false)]
    open: bool,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_upper_bound_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbKeyRangeUpperBoundArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(upper) = require_idb_key(scope, parsed.upper) else {
        return;
    };
    let range = IdbKeyRangeQuery {
        lower: None,
        upper: Some(upper),
        lower_open: false,
        upper_open: parsed.open,
    };
    if let Some(object) = create_key_range_object(scope, &range) {
        rv.set(object.into());
    } else {
        rv.set_undefined();
    }
}
