//! Host entry points into JavaScript. Keep the microtask nesting boundary at
//! the actual V8 invocation; entering a context alone does not execute script.

pub(crate) fn execute_compiled_script<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    script: v8::Local<'_, v8::Script>,
) -> Option<v8::Local<'s, v8::Value>> {
    run(scope, |scope| script.run(scope))
}

pub(crate) fn evaluate_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    module: v8::Local<'_, v8::Module>,
) -> Option<v8::Local<'s, v8::Value>> {
    run(scope, |scope| module.evaluate(scope))
}

pub(crate) fn call_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    function: v8::Local<'_, v8::Function>,
    receiver: v8::Local<'_, v8::Value>,
    arguments: &[v8::Local<'_, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    run(scope, |scope| function.call(scope, receiver, arguments))
}

pub(crate) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'_, v8::Function>,
    arguments: &[v8::Local<'_, v8::Value>],
) -> Option<v8::Local<'s, v8::Object>> {
    run(scope, |scope| constructor.new_instance(scope, arguments))
}

pub(crate) fn freeze_animation_time(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::native_bridge::AnimationScriptTimeGuard> {
    let (_, crate::common_runtime::RuntimeExecutionContext::Window(host)) =
        crate::common_runtime::CommonRuntimeBinding::from_scope(scope)?
    else {
        return None;
    };
    Some(
        unsafe { &*host }
            .animation_transition_state()
            .enter_script_time(std::time::Instant::now()),
    )
}

pub(crate) fn perform_animation_microtask_checkpoint(scope: &mut v8::PinScope<'_, '_>) {
    let _sample = freeze_animation_time(scope);
    scope.perform_microtask_checkpoint();
}

// Page and worker isolates use Explicit policy: this records native nesting,
// while the script owner still chooses when to checkpoint. Inspector's Scoped
// policy checkpoints on the outermost scope exit. Auto keeps V8's API behavior.
fn run<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    execute: impl FnOnce(&mut v8::PinScope<'s, '_>) -> R,
) -> R {
    let _sample = freeze_animation_time(scope);
    moli_v8_util::with_microtasks_scope(scope, execute)
}
