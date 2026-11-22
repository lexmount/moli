use v8::{Context, Global, Local, PinScope, RegExp, RegExpCreationFlags};

// Blink's ScriptRegexp uses the same budget for user-agent regular expressions.
const NATIVE_REGEXP_BACKTRACK_LIMIT: u32 = 1_000_000;

struct NativeRegExpContext(Global<Context>);

fn native_regexp_context<'s>(scope: &mut PinScope<'s, '_>) -> Local<'s, Context> {
    if let Some(context) = scope.get_slot::<NativeRegExpContext>() {
        return Local::new(scope, &context.0);
    }
    // This context contains no author objects. Keeping it in an isolate slot
    // shares it across host matches without retaining a page's realm graph.
    let context = Context::new(scope, Default::default());
    let retained = Global::new(scope, context);
    scope.set_slot(NativeRegExpContext(retained));
    context
}

/// A bounded user-agent regular expression, isolated from author intrinsics
/// and the observable legacy RegExp statics of every author realm.
pub struct NativeRegExp<'s> {
    context: Local<'s, Context>,
    regexp: Local<'s, RegExp>,
}

impl<'s> NativeRegExp<'s> {
    /// Compile within the current handle scope and catch syntax errors.
    pub fn new(
        scope: &mut PinScope<'s, '_>,
        pattern: &str,
        flags: RegExpCreationFlags,
    ) -> Option<Self> {
        Self::new_with_backtrack_limit(scope, pattern, flags, NATIVE_REGEXP_BACKTRACK_LIMIT)
    }

    fn new_with_backtrack_limit(
        scope: &mut PinScope<'s, '_>,
        pattern: &str,
        flags: RegExpCreationFlags,
        backtrack_limit: u32,
    ) -> Option<Self> {
        if scope.is_execution_terminating() {
            return None;
        }
        let pattern = v8::String::new(scope, pattern)?;
        let context = native_regexp_context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        let regexp = RegExp::new_with_backtrack_limit(&scope, pattern, flags, backtrack_limit);
        if regexp.is_none() {
            scope.reset();
        }
        regexp.map(|regexp| Self { context, regexp })
    }

    /// `Some(false)` includes exhausted backtracking budgets. `None` indicates
    /// an exceptional execution failure, distinct from a failed match.
    pub fn is_match(
        &self,
        scope: &mut PinScope<'_, '_>,
        subject: Local<v8::String>,
    ) -> Option<bool> {
        let scope = &mut v8::ContextScope::new(scope, self.context);
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        let result = self.regexp.exec(&scope, subject);
        if result.is_none() {
            scope.reset();
        }
        result.map(|value| !value.is_null())
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, pin::pin, rc::Rc};

    use super::*;

    fn eval<'s>(scope: &mut PinScope<'s, '_>, source: &str) -> Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }

    #[test]
    fn native_regexp_does_not_read_author_intrinsics_or_change_realm_statics() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let contexts = [
            Context::new(scope, Default::default()),
            Context::new(scope, Default::default()),
        ];
        for context in contexts {
            let scope = &mut v8::ContextScope::new(scope, context);
            eval(
                scope,
                r#"
                globalThis.originalRegExp = RegExp;
                /(sentinel)/.exec('sentinel');
                RegExp.prototype.exec = () => { throw new Error('author exec'); };
                Object.defineProperty(globalThis, 'RegExp', {
                  get() { throw new Error('author RegExp'); }
                });
                "#,
            );
            let subject = v8::String::new(scope, "different").unwrap();
            let regexp =
                NativeRegExp::new(scope, "(different)", RegExpCreationFlags::UNICODE_SETS).unwrap();
            assert_eq!(regexp.is_match(scope, subject), Some(true));
            assert_eq!(scope.get_current_context(), context);
            assert!(eval(scope, "originalRegExp.$1 === 'sentinel'").is_true());
        }
    }

    #[test]
    fn native_regexp_treats_exhausted_backtracking_as_a_failed_match() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let pattern = r"^([a-z]+)\1$";
        let flags = RegExpCreationFlags::UNICODE_SETS;
        let subject = v8::String::new(scope, "abcabc").unwrap();
        let limited = NativeRegExp::new_with_backtrack_limit(scope, pattern, flags, 1).unwrap();
        assert_eq!(limited.is_match(scope, subject), Some(false));

        let regular = NativeRegExp::new(scope, pattern, flags).unwrap();
        assert_eq!(regular.is_match(scope, subject), Some(true));
    }

    #[test]
    fn native_regexp_context_survives_until_isolate_teardown() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let context_released = Rc::new(Cell::new(false));
        let context_probe = {
            let scope = pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let author_context = Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, author_context);
            let native_context = native_regexp_context(scope);
            let released = Rc::clone(&context_released);
            v8::Weak::with_guaranteed_finalizer(
                scope,
                native_context,
                Box::new(move || released.set(true)),
            )
        };

        isolate.low_memory_notification();
        assert!(!context_probe.is_empty());
        drop(isolate);
        assert!(context_released.get());
    }
}
