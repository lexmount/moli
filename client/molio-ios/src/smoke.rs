use std::{ffi::c_int, pin::pin};

// (module (func (export "add") (param i32 i32) (result i32)
//   local.get 0 local.get 1 i32.add))
const ADD_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f, 0x01,
    0x7f, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00, 0x0a, 0x09,
    0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b,
];

/// Returns the Wasm result after checking that malformed Wasm throws a CompileError.
#[unsafe(no_mangle)]
pub extern "C" fn molio_v8_smoke() -> c_int {
    moli_v8_init::ensure_v8_initialized_with_flags(Some("--trace-drumbrake-execution"), || {
        v8::new_default_platform(0, false).make_shared()
    });
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let source = format!(
        "(() => {{
            try {{
                new WebAssembly.Module(Uint8Array.of(0, 1, 2));
                return -2;
            }} catch (error) {{
                if (!(error instanceof WebAssembly.CompileError)) return -3;
            }}
            const module = new WebAssembly.Module(Uint8Array.from({ADD_WASM:?}));
            return new WebAssembly.Instance(module).exports.add(20, 22);
        }})()"
    );
    let result = v8::String::new(scope, &source)
        .and_then(|source| v8::Script::compile(scope, source, None))
        .and_then(|script| script.run(scope))
        .and_then(|result| result.int32_value(scope))
        .unwrap_or(-1);
    if result == 42 {
        eprintln!("V8 smoke PASS: invalid Wasm raised CompileError, add(20, 22) = 42");
    }
    result
}
