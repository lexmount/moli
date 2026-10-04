/// A rooted single-operation Web IDL callback interface and the Realm anchors
/// captured at conversion.
///
/// The callback's relevant Realm belongs to the original callback object. It
/// must not be rediscovered later from a callback-interface operation such as
/// `handleEvent`, because that operation may be replaced with a function from a
/// different Realm after registration.
pub struct WebIdlCallbackInterface {
    callback: v8::Global<v8::Object>,
    relevant_context: v8::Global<v8::Context>,
    incumbent_context: v8::Global<v8::Context>,
    callable_at_conversion: bool,
}

impl WebIdlCallbackInterface {
    pub fn new<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        callback: v8::Local<'s, v8::Object>,
        relevant_context: v8::Local<'s, v8::Context>,
        incumbent_context: v8::Local<'s, v8::Context>,
    ) -> Self {
        Self {
            callback: v8::Global::new(scope, callback),
            relevant_context: v8::Global::new(scope, relevant_context),
            incumbent_context: v8::Global::new(scope, incumbent_context),
            callable_at_conversion: callback.is_callable(),
        }
    }

    /// Creates an invocation snapshot that no longer borrows the owner registry.
    ///
    /// Event dispatch can therefore release all registration-store borrows
    /// before entering user code and its synchronous reentrancy.
    pub fn prepare(&self, scope: &mut v8::PinScope<'_, '_>) -> PreparedWebIdlCallbackInterface {
        PreparedWebIdlCallbackInterface::new(
            scope,
            v8::Local::new(scope, &self.callback),
            v8::Local::new(scope, &self.relevant_context),
            v8::Local::new(scope, &self.incumbent_context),
            self.callable_at_conversion,
        )
    }

    pub fn matches<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        candidate: v8::Local<'s, v8::Object>,
    ) -> bool {
        v8::Local::new(scope, &self.callback).strict_equals(candidate.into())
    }

    pub fn value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        v8::Local::new(scope, &self.callback).into()
    }

    pub fn relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        v8::Local::new(scope, &self.relevant_context)
    }
    pub fn incumbent_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        v8::Local::new(scope, &self.incumbent_context)
    }

    pub fn callable_at_conversion(&self) -> bool {
        self.callable_at_conversion
    }
}

/// An independently rooted callback-interface snapshot ready for synchronous
/// invocation.
pub struct PreparedWebIdlCallbackInterface {
    callback: v8::Global<v8::Object>,
    relevant_context: v8::Global<v8::Context>,
    incumbent_context: v8::Global<v8::Context>,
    callable_at_conversion: bool,
}

impl PreparedWebIdlCallbackInterface {
    /// Root a snapshot of callback-interface facts held by a traced owner.
    pub fn new(
        scope: &mut v8::PinScope<'_, '_>,
        callback: v8::Local<'_, v8::Object>,
        relevant_context: v8::Local<'_, v8::Context>,
        incumbent_context: v8::Local<'_, v8::Context>,
        callable_at_conversion: bool,
    ) -> Self {
        Self {
            callback: v8::Global::new(scope, callback),
            relevant_context: v8::Global::new(scope, relevant_context),
            incumbent_context: v8::Global::new(scope, incumbent_context),
            callable_at_conversion,
        }
    }

    pub fn callback<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        v8::Local::new(scope, &self.callback)
    }

    pub fn relevant_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        v8::Local::new(scope, &self.relevant_context)
    }

    pub fn incumbent_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Context> {
        v8::Local::new(scope, &self.incumbent_context)
    }

    pub fn callable_at_conversion(&self) -> bool {
        self.callable_at_conversion
    }
}
