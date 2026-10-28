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
fn timer_nesting_survives_the_callback_microtask_checkpoint() {
    let mut vm = new_storage_test_vm("https://timer-nesting.test/");
    vm.eval(
        "globalThis.fired = 0; function step() { ++fired; queueMicrotask(() => setTimeout(step, 0)); } setTimeout(step, 0)",
    )
    .unwrap();
    run_first_six_callbacks(&mut vm);
    assert_eq!(
        vm.document_runtime.take_next_timeout_delay_for_test(),
        Some(4)
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
