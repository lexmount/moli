use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBKeyRange.includes")]
struct IdbKeyRangeIncludesArgs<'s> {
    #[webidl(required, converter = "raw")]
    key: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_key_range_includes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbKeyRangeIncludesArgs<'s>>(scope, &args) else {
        return;
    };
    let Some(range) = parse_key_range_from_value(scope, args.this().into()) else {
        return;
    };
    let Some(key) = require_idb_key(scope, parsed.key) else {
        return;
    };
    rv.set_bool(key_in_range(&key, &range));
}
