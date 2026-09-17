use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IDBFactory.cmp")]
struct IdbFactoryCmpArgs<'s> {
    #[webidl(required, converter = "raw")]
    first: v8::Local<'s, v8::Value>,
    #[webidl(required, converter = "raw")]
    second: v8::Local<'s, v8::Value>,
}

pub(in crate::context_bootstrap::indexed_db) fn idb_factory_cmp_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<IdbFactoryCmpArgs<'s>>(scope, &args) else {
        return;
    };
    let _ = indexed_db_runtime_factory(scope);
    let Some(left) = require_idb_key(scope, parsed.first) else {
        return;
    };
    let Some(right) = require_idb_key(scope, parsed.second) else {
        return;
    };
    rv.set_int32(compare_idb_keys(&left, &right));
}
