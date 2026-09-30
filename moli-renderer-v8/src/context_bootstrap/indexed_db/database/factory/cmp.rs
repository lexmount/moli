use super::*;
use crate::webidl;
use moli_indexeddb::Key;

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
    let Some(left) = comparison_key_or_throw(scope, parsed.first) else {
        return;
    };
    let Some(right) = comparison_key_or_throw(scope, parsed.second) else {
        return;
    };
    rv.set_int32(compare_idb_keys(&left, &right));
}

// Key conversion can invoke array getters. Preserve an author's exception and
// report only genuine invalid-key results as DataError in the binding's realm.
fn comparison_key_or_throw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Key> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let parsed = parse_idb_key(&mut scope, value);
    if scope.has_caught() {
        scope.rethrow();
        return None;
    }
    match parsed {
        Ok(Some(key)) => Some(key),
        Ok(None) | Err(_) => {
            let exception = dom_exception_value(
                &mut scope,
                "The value is not a valid IndexedDB key.",
                "DataError",
            );
            scope.throw_exception(exception);
            scope.rethrow();
            None
        }
    }
}
