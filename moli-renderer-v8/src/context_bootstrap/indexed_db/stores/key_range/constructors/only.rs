use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBKeyRange.only")]
struct IdbKeyRangeOnlyArgs<'s> {
    #[webidl(required, converter = "raw")]
    value: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_only_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbKeyRangeOnlyArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(key) = require_idb_key(scope, parsed.value) else {
        return;
    };
    let range = IdbKeyRangeQuery {
        lower: Some(key.clone()),
        upper: Some(key),
        lower_open: false,
        upper_open: false,
    };
    if let Some(object) = create_key_range_object(scope, &range) {
        rv.set(object.into());
    } else {
        rv.set_undefined();
    }
}
