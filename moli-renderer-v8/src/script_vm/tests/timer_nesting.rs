use super::*;

#[test]
fn timer_nesting_clamps_intervals_after_six_unclamped_callbacks() {
    let mut vm = new_storage_test_vm("https://timer-nesting.test/");
    vm.eval("globalThis.fired = 0; setInterval(() => ++fired, 0)")
        .unwrap();
    run_first_six_callbacks(&mut vm);
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(4)
    );
}

#[test]
fn timer_nesting_resets_after_a_completed_task() {
    let mut vm = new_storage_test_vm("https://timer-nesting.test/");
    vm.eval(
        "globalThis.fired = 0; function step() { ++fired; setTimeout(step, 0); } setTimeout(step, 0)",
    )
    .unwrap();
    run_first_six_callbacks(&mut vm);
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(4)
    );
    vm.eval("setTimeout(() => {}, 0)").unwrap();
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(0)
    );
}

#[test]
fn timer_nesting_resets_after_a_callback_throws() {
    let mut vm = new_storage_test_vm("https://timer-nesting.test/");
    vm.eval(
        "globalThis.fired = 0; window.onerror = () => true; function step() { ++fired; setTimeout(step, 0); if (fired === 6) throw new Error('timer'); } setTimeout(step, 0)",
    )
    .unwrap();
    run_first_six_callbacks(&mut vm);
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(4)
    );
    vm.eval("setTimeout(() => {}, 0)").unwrap();
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(0)
    );
}

fn run_first_six_callbacks(vm: &mut ScriptVm) {
    for _ in 0..6 {
        assert!(
            vm.run_next_timeout_for_test()
                .expect("timer callback should run")
                .consumed_heap_head()
        );
    }
    assert_eq!(vm.eval("fired").unwrap(), "6");
}

#[test]
fn timer_nesting_clamps_function_chains_after_six_callbacks() {
    let mut vm = new_storage_test_vm("https://timer-nesting.test/");
    vm.eval(
        "globalThis.fired = 0; function step() { ++fired; setTimeout(step, 0); } setTimeout(step, 0)",
    )
    .unwrap();
    run_first_six_callbacks(&mut vm);
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(4)
    );
}

#[test]
fn timer_nesting_clamps_string_chains_after_six_callbacks() {
    let mut vm = new_storage_test_vm("https://timer-nesting.test/");
    vm.eval(
        "globalThis.fired = 0; function step() { ++fired; setTimeout('step()', 0); } setTimeout('step()', 0)",
    )
    .unwrap();
    run_first_six_callbacks(&mut vm);
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(4)
    );
}

#[test]
fn timer_nesting_resets_inside_callback_microtask_checkpoint() {
    let mut vm = new_storage_test_vm("https://timer-nesting.test/");
    vm.eval(
        "globalThis.fired = 0; function step() { ++fired; queueMicrotask(() => setTimeout(step, 0)); } setTimeout(step, 0)",
    )
    .unwrap();
    run_first_six_callbacks(&mut vm);
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(0)
    );
}

#[test]
fn timer_microtask_timeout_keeps_requested_delay_after_a_nested_callback() {
    for schedule in [
        "queueMicrotask",
        "callback => Promise.resolve().then(callback)",
    ] {
        let mut vm = new_storage_test_vm("https://timer-microtask.test/");
        vm.eval(&format!(
            r#"
globalThis.fired = 0;
const scheduleMicrotask = {schedule};
function step() {{
  if (++fired < 6) {{
    setTimeout(step, 0);
    return;
  }}
  setTimeout(() => {{}}, 1);
  scheduleMicrotask(() => setTimeout(() => {{}}, 1));
}}
setTimeout(step, 0);
"#,
        ))
        .unwrap();
        run_first_six_callbacks(&mut vm);
        let mut delays = [
            vm.document_runtime.take_next_timeout_delay_for_test(),
            vm.document_runtime.take_next_timeout_delay_for_test(),
        ];
        delays.sort();
        assert_eq!(delays, [Some(1), Some(4)], "{schedule}");
    }
}

#[test]
fn timer_microtask_timeout_does_not_reset_the_interval_rearm_level() {
    let mut vm = new_storage_test_vm("https://timer-microtask-interval.test/");
    vm.eval(
        r#"
globalThis.fired = 0;
setInterval(() => {
  if (++fired === 6) queueMicrotask(() => setTimeout(() => {}, 1));
}, 0);
"#,
    )
    .unwrap();
    run_first_six_callbacks(&mut vm);
    let mut delays = [
        vm.document_runtime.take_next_timeout_delay_for_test(),
        vm.document_runtime.take_next_timeout_delay_for_test(),
    ];
    delays.sort();
    assert_eq!(delays, [Some(1), Some(4)]);
}

#[test]
fn timer_microtask_checkpoint_suspends_worker_level_and_restores_outer_task() {
    use crate::timer_nesting::{TimerNestingLevel, TimerNestingOwner, TimerNestingScope};

    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let task = TimerNestingScope::enter(scope.as_mut());
    let owner = TimerNestingOwner::Worker;
    let mut level = TimerNestingLevel::default();
    for _ in 0..6 {
        level = level.next();
    }
    level.activate(scope, owner);
    assert_eq!(
        TimerNestingLevel::for_new_timer(scope, owner).clamp_delay_ms(1),
        4
    );
    let checkpoint = crate::script_cleanup::MicrotaskCheckpointScope::enter(scope)
        .expect("the agent checkpoint should start");
    assert_eq!(
        TimerNestingLevel::for_new_timer(scope, owner).clamp_delay_ms(1),
        1
    );
    assert!(crate::script_cleanup::MicrotaskCheckpointScope::enter(scope).is_none());
    assert_eq!(
        TimerNestingLevel::for_new_timer(scope, owner).clamp_delay_ms(1),
        1
    );
    drop(checkpoint);
    assert_eq!(
        TimerNestingLevel::for_new_timer(scope, owner).clamp_delay_ms(1),
        4
    );
    drop(task);
    assert_eq!(
        TimerNestingLevel::for_new_timer(scope, owner).clamp_delay_ms(1),
        1
    );
}

#[test]
fn timer_nesting_does_not_clamp_browser_algorithm_timeouts() {
    let mut vm = new_storage_test_vm("https://timer-nesting.test/");
    vm.eval(
        "globalThis.fired = 0; function step() { if (++fired < 6) { setTimeout(step, 0); } else { globalThis.signal = AbortSignal.timeout(0); setTimeout(() => {}, 0); } } setTimeout(step, 0)",
    )
    .unwrap();
    run_first_six_callbacks(&mut vm);
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(0)
    );
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(4)
    );
}
