//! Manual binding microbenchmark. It measures native lookup/validation only,
//! with no JavaScript invocation or Window identity lookup. Run on an idle
//! machine with an optimized test binary:
//!
//! CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256 cargo test --release -p moli-renderer-v8
//! benchmark_common_runtime_binding_resolution -- --ignored --nocapture --test-threads=1

use std::{hint::black_box, time::Instant};

use super::*;

// Keep the former call chain here as a benchmark reference, outside production
// code. Both paths use the same Context, owner, compiler, and sampling loop.
fn legacy_validate_context(
    binding: &CommonRuntimeBinding,
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<()> {
    let context = scope.get_current_context();
    let installed = context.get_slot::<CommonRuntimeBinding>()?;
    if !std::ptr::eq(binding, installed.as_ref()) {
        return None;
    }
    binding.runtime()?;
    match &binding.owner {
        RuntimeOwner::Window { .. } => {
            if page_context_is_disconnected(context) {
                return None;
            }
            binding.window_host_ptr()?;
        }
        RuntimeOwner::Worker { state, .. } => {
            state.upgrade()?;
        }
    }
    Some(())
}

fn legacy_from_scope(scope: &mut v8::PinScope<'_, '_>) -> Option<Rc<CommonRuntimeBinding>> {
    let binding = scope
        .get_current_context()
        .get_slot::<CommonRuntimeBinding>()?;
    legacy_validate_context(&binding, scope)?;
    Some(binding)
}

fn legacy_execution_context(
    binding: &CommonRuntimeBinding,
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<RuntimeExecutionContext> {
    legacy_validate_context(binding, scope)?;
    Some(match binding.owner {
        RuntimeOwner::Window { host_ptr, .. } => RuntimeExecutionContext::Window(host_ptr),
        RuntimeOwner::Worker { .. } => RuntimeExecutionContext::Worker,
    })
}

fn window_host(owner: RuntimeExecutionContext) -> Option<*mut JsContextHost> {
    match owner {
        RuntimeExecutionContext::Window(host) => Some(host),
        RuntimeExecutionContext::Worker => None,
    }
}

fn sample(
    scope: &mut v8::PinScope<'_, '_>,
    iterations: usize,
    operation: &mut impl FnMut(&mut v8::PinScope<'_, '_>) -> Option<*mut JsContextHost>,
) -> f64 {
    // Bound V8's temporary handles to this sample, identically for both paths.
    let scope = std::pin::pin!(v8::HandleScope::new(scope));
    let scope = &mut scope.init();
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(operation(black_box(&mut *scope)));
    }
    start.elapsed().as_secs_f64() * 1e9 / iterations as f64
}

fn compare(
    scope: &mut v8::PinScope<'_, '_>,
    label: &str,
    mut legacy: impl FnMut(&mut v8::PinScope<'_, '_>) -> Option<*mut JsContextHost>,
    mut resolved: impl FnMut(&mut v8::PinScope<'_, '_>) -> Option<*mut JsContextHost>,
) {
    const ITERATIONS: usize = 500_000;
    const SAMPLES: usize = 9;
    assert!(legacy(scope).is_some());
    assert!(resolved(scope).is_some());
    sample(scope, 20_000, &mut legacy);
    sample(scope, 20_000, &mut resolved);
    let mut before = [0.0; SAMPLES];
    let mut after = [0.0; SAMPLES];
    for index in 0..SAMPLES {
        // Alternate order to avoid consistently giving one path a warmer CPU.
        if index % 2 == 0 {
            before[index] = sample(scope, ITERATIONS, &mut legacy);
            after[index] = sample(scope, ITERATIONS, &mut resolved);
        } else {
            after[index] = sample(scope, ITERATIONS, &mut resolved);
            before[index] = sample(scope, ITERATIONS, &mut legacy);
        }
    }
    before.sort_by(f64::total_cmp);
    after.sort_by(f64::total_cmp);
    println!(
        "{label}: legacy median {:.1} ns/op [{:.1}, {:.1}], resolved median {:.1} ns/op [{:.1}, {:.1}]; {SAMPLES} samples x {ITERATIONS} iterations",
        before[SAMPLES / 2],
        before[0],
        before[SAMPLES - 1],
        after[SAMPLES / 2],
        after[0],
        after[SAMPLES - 1],
    );
}

#[test]
#[ignore = "manual optimized microbenchmark; no performance thresholds in CI"]
fn benchmark_common_runtime_binding_resolution() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let common = Rc::new(CommonRuntimeState::default());
    let lifecycle = Rc::new(Cell::new(ContextHostLifecycle::Active));
    // Lookup does not dereference the host. Identity validation and JS
    // invocation are deliberately outside this microbenchmark.
    let host = std::ptr::NonNull::<JsContextHost>::dangling().as_ptr();
    CommonRuntimeBinding::install_window(context, &common, host, &lifecycle);
    let scope = &mut v8::ContextScope::new(scope, context);
    let (binding, _) = CommonRuntimeBinding::from_scope(scope).unwrap();
    compare(
        scope,
        "Window typed callback preparation",
        |scope| legacy_from_scope(scope)?.window_host_ptr(),
        |scope| window_host(CommonRuntimeBinding::from_scope(scope)?.1),
    );
    compare(
        scope,
        "Window traced callback preparation",
        |scope| {
            let binding = legacy_from_scope(scope)?;
            window_host(legacy_execution_context(&binding, scope)?)
        },
        |scope| window_host(CommonRuntimeBinding::from_scope(scope)?.1),
    );
    compare(
        scope,
        "Window pre-invocation binding validation",
        |scope| {
            legacy_execution_context(&binding, scope)?;
            binding.window_host_ptr()
        },
        |scope| window_host(binding.callback_execution_context(scope)?),
    );
}
