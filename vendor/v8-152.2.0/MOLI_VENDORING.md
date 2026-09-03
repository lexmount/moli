This vendored `rusty_v8` tree is intentionally slimmed for Moli's default
prebuilt workflow.

Base:
- rusty_v8 crate version `152.2.0`
- V8 version `15.2.124.1`
- the Rust wrapper and public headers are vendored here, while the default
  build links the matching upstream prebuilt archive

Kept:
- `build.rs`
- `Cargo.toml`
- `README.md`
- `src/`
- `gen/`
- `v8/include/`

Moli extensions:
- `build.rs` compiles Moli's small V8 C++ shims through
  `build_moli_v8_ext()`. Keep new Moli-owned C++ shims in separate
  `src/*_ext.cc` files instead of editing upstream `src/binding.cc` directly.
- Windows shims use `clang-cl`, matching V8's supported Windows compiler and
  the MSVC ABI of the prebuilt archive. V8's public headers use Clang builtins
  and cannot be compiled as C++ translation units by `cl.exe`.
- `src/object_template_ext.cc`, `src/object_ext.cc`, and
  `src/function_template_ext.cc` expose embedder APIs required by Moli's DOM
  and Web IDL bindings.
- `src/context_ext.cc` exposes global detachment and backup-incumbent-context
  support used by Moli's browsing-context lifecycle.
- `src/inspector_context_ext.cc` and `src/inspector_session_ext.cc` expose the
  inspector context/session operations required by Moli's CDP implementation.
- `src/module_ext.cc` exposes the synthetic-module export hook used to write
  V8's uninitialized binding sentinel for Wasm `v128` namespace cells.
- `src/wasm_ext.cc` exposes compile-time imports for JS string builtins and
  imported string constants. Calls without compile options stay on rusty_v8's
  native Rust/C ABI path; this avoids crossing the prebuilt-libc++/host-libstdc++
  `std::span` ABI boundary from the local C++ extension.
- `src/cpu_profiler_ext.cc` and `src/cpu_profiler.rs` expose the sampling
  profiler used by Moli diagnostics.

Version-specific corrections:
- `src/function.rs` declares `v8::Function::GetName()` as returning `Value`,
  matching V8's public API. Declaring it as `String` is unsound for callable
  proxies and other non-string names.
- the snapshot path in `src/isolate.rs` drops isolate slots and guaranteed
  context-annex finalizers before disposing the isolate handle. This lets their
  `Weak` globals reset while V8 is live and prevents unserialized native-context
  handles from reaching `SnapshotCreator::CreateBlob()`.
- ordinary isolate teardown uses the same ordering before V8's final teardown
  GC. Weak handles also retain their callback data until V8's second pass has
  run, even when the guaranteed-finalizer map is being drained.
- ordinary isolate teardown and snapshot-creator teardown are covered by
  `context_annex_weak_handles_are_safe_during_isolate_teardown` and
  `snapshot_creator_cleans_up_context_annex_before_creating_blob` in
  `moli-renderer-v8/src/script_vm/document_isolate.rs`.

Removed from the upstream crate:
- GN/Ninja/Chromium source-build inputs such as `build/`, `buildtools/`,
  `third_party/`, `tools/`, the rest of `v8/`, and crate-local tests/examples.

Implications:
- default prebuilt linking remains supported
- `V8_FROM_SOURCE=1` is supported only after restoring the full upstream
  source-build layout; the slim tree fails early and lists the missing inputs

`serde_v8 0.320.0` depends on `deno_v8 0.3.0`, whose published manifest still
selects V8 150. The adjacent slim `vendor/deno_v8-0.3.0` patch changes only that
dependency constraint to V8 152 so serde_v8 and Moli share one V8 type universe.
