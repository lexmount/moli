use std::{cell::Cell, pin::pin, rc::Rc};

unsafe extern "C" fn constructor_enabled(context: v8::Local<'_, v8::Context>) -> bool {
    context
        .get_slot::<Cell<bool>>()
        .is_some_and(|allowed| allowed.get())
}

fn evaluate(scope: &mut v8::PinScope<'_, '_>, source: &str) -> String {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap()
        .to_rust_string_lossy(scope)
}

#[test]
fn shared_array_buffer_constructor_uses_the_requested_context_native_policy() {
    moli_v8_test_util::ensure_v8_with_flags_and_platform(
        Some("--enable-sharedarraybuffer-per-context"),
        || v8::new_default_platform(0, false).make_shared(),
    );
    let mut isolate = v8::Isolate::new(Default::default());
    isolate.set_shared_array_buffer_constructor_enabled_callback(constructor_enabled);
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let unbound = v8::Context::new(scope, Default::default());
    let allowed = v8::Context::new(scope, Default::default());
    let denied = v8::Context::new(scope, Default::default());
    allowed.set_slot(Rc::new(Cell::new(true)));
    denied.set_slot(Rc::new(Cell::new(false)));
    {
        let scope = &mut v8::ContextScope::new(scope, denied);
        assert_eq!(evaluate(scope, "typeof SharedArrayBuffer"), "undefined");
        evaluate(
            scope,
            "Object.defineProperty(globalThis, 'crossOriginIsolated', {get() {throw 42}})",
        );
        // The requested Context's native policy wins over the entered Context.
        scope.install_conditional_features(allowed);
        scope.install_conditional_features(denied);
        scope.install_conditional_features(unbound);
        assert_eq!(
            evaluate(scope, "'SharedArrayBuffer' in globalThis"),
            "false"
        );
        assert_eq!(
            evaluate(
                scope,
                "new WebAssembly.Memory({shared:true,initial:1,maximum:1}).buffer.byteLength"
            ),
            "65536"
        );
    }
    {
        let scope = &mut v8::ContextScope::new(scope, allowed);
        assert_eq!(
            evaluate(
                scope,
                r#"(() => {
            const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'SharedArrayBuffer');
            const intrinsic = new WebAssembly.Memory({shared:true,initial:1,maximum:1}).buffer.constructor;
            return [descriptor.value === intrinsic, descriptor.writable, descriptor.enumerable,
                descriptor.configurable, new SharedArrayBuffer(8).byteLength].join('|');
        })()"#
            ),
            "true|true|false|true|8"
        );
        evaluate(scope, "globalThis.SharedArrayBuffer = 17");
        scope.install_conditional_features(allowed);
        assert_eq!(evaluate(scope, "SharedArrayBuffer"), "17");
    }
    {
        let scope = &mut v8::ContextScope::new(scope, unbound);
        assert_eq!(
            evaluate(scope, "'SharedArrayBuffer' in globalThis"),
            "false"
        );
    }
}
