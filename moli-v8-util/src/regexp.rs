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
    /// Retain the compiled expression in private platform-object state. It must
    /// not be exposed to author code or have its lastIndex/properties modified.
    pub fn compiled(&self) -> Local<'s, RegExp> {
        self.regexp
    }

    /// Restore an expression previously compiled by `NativeRegExp::new`.
    pub fn from_compiled(scope: &mut PinScope<'s, '_>, regexp: Local<'s, RegExp>) -> Self {
        Self {
            context: native_regexp_context(scope),
            regexp,
        }
    }

    /// Compile without exposing syntax errors to the surrounding script.
    pub fn new(
        scope: &mut PinScope<'s, '_>,
        pattern: Local<v8::String>,
        flags: RegExpCreationFlags,
    ) -> Option<Self> {
        let context = native_regexp_context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        let regexp =
            RegExp::new_with_backtrack_limit(&scope, pattern, flags, NATIVE_REGEXP_BACKTRACK_LIMIT);
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
        self.exec(scope, subject).map(|value| !value.is_null())
    }

    /// Execute in the private realm, preserving captures without invoking
    /// author RegExp methods or updating author realms' legacy RegExp statics.
    /// Null represents a failed match (including an exhausted budget).
    pub fn exec<'t>(
        &self,
        scope: &mut PinScope<'t, '_>,
        subject: Local<v8::String>,
    ) -> Option<Local<'t, v8::Value>> {
        let scope = &mut v8::ContextScope::new(scope, self.context);
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        let result = self.regexp.exec(&scope, subject);
        if result.is_none() {
            scope.reset();
        }
        result.map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::*;

    fn eval<'s>(scope: &mut PinScope<'s, '_>, source: &str) -> Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }

    #[test]
    fn native_regexp_bounds_backtracking_and_keeps_invalid_patterns_distinct() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = v8::String::new(scope, "^(?:(\\d+)*)$").unwrap();
        let regexp = NativeRegExp::new(scope, source, RegExpCreationFlags::UNICODE_SETS).unwrap();
        let matching = v8::String::new(scope, "12345678901234567890123456789123456789").unwrap();
        let rejecting = v8::String::new(scope, "12345678901234567890123456789123456789z").unwrap();
        assert_eq!(regexp.is_match(scope, matching), Some(true));
        assert_eq!(regexp.is_match(scope, rejecting), Some(false));
        let invalid = v8::String::new(scope, "[").unwrap();
        assert!(NativeRegExp::new(scope, invalid, RegExpCreationFlags::UNICODE_SETS).is_none());
        assert!(eval(scope, "true").is_true());
        assert_eq!(scope.get_current_context(), context);
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
            let source = v8::String::new(scope, "(different)").unwrap();
            let subject = v8::String::new(scope, "different").unwrap();
            let regexp =
                NativeRegExp::new(scope, source, RegExpCreationFlags::UNICODE_SETS).unwrap();
            assert_eq!(regexp.is_match(scope, subject), Some(true));
            assert_eq!(scope.get_current_context(), context);
            assert!(eval(scope, "originalRegExp.$1 === 'sentinel'").is_true());
        }
    }
}
