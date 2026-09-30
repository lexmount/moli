use super::*;

fn global_object<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, name).unwrap();
    v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap()
}

fn copied_coefficients<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Vec<f32> {
    let value = crate::util::get_private_value(scope, owner, slot).unwrap();
    let buffer = v8::Local::<v8::ArrayBuffer>::try_from(value).unwrap();
    buffer
        .get_backing_store()
        .chunks_exact(size_of::<f32>())
        .map(|bytes| f32::from_ne_bytes(std::array::from_fn(|i| bytes[i].get())))
        .collect()
}

#[test]
fn periodic_waves_and_oscillators_use_native_interfaces_and_webidl_controls() {
    let mut vm = new_storage_page_task_executor_test_vm("https://periodic-waves.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__periodicDone = value, error => globalThis.__periodicDone = String(error));", include_str!("periodic_wave_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__periodicDone").unwrap(), "true");
}

#[test]
fn custom_waveforms_require_a_backend_only_when_scheduled_and_reachable() {
    let mut vm = new_storage_page_task_executor_test_vm("https://periodic-backend.test/");
    vm.eval(&format!("({}).then(value => globalThis.__periodicBackendDone = value, error => globalThis.__periodicBackendDone = String(error));", include_str!("periodic_wave_backend_guards.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__periodicBackendDone").unwrap(), "true");
}

#[test]
fn periodic_coefficients_snapshot_floats_clear_dc_and_retain_normalization() {
    let mut vm = new_storage_test_vm("https://periodic-coefficients.test/");
    vm.eval(
        r#"
const context = new OfflineAudioContext(1, 16, 48000);
const real = new Float64Array([100, 1.1, -2.2]);
const imag = new Float32Array([99, .5, -.75]);
globalThis.wave = context.createPeriodicWave(real, imag, {disableNormalization: true});
real.fill(10); imag.fill(20);
structuredClone(real.buffer, {transfer: [real.buffer]});
structuredClone(imag.buffer, {transfer: [imag.buffer]});
wave.real = [0, 50]; wave.imag = [0, 60];
wave.__moliPeriodicWaveReal = [0, 70]; wave.__moliPeriodicWaveImag = [0, 80];
wave.__moliPeriodicWaveNormalize = true;
globalThis.defaultWave = new PeriodicWave(context);
globalThis.realOnly = new PeriodicWave(context, {real: [5, 2]});
globalThis.imagOnly = new PeriodicWave(context, {imag: [5, 3]});
globalThis.oscillator = new OscillatorNode(context, {periodicWave: wave});
oscillator.__moliOscillatorPeriodicWave = null;
"#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let wave = global_object(scope, "wave");
        assert_eq!(
            copied_coefficients(scope, wave, "__moliPeriodicWaveReal"),
            vec![0.0, 1.1_f32, -2.2_f32]
        );
        assert_eq!(
            copied_coefficients(scope, wave, "__moliPeriodicWaveImag"),
            vec![0.0, 0.5, -0.75]
        );
        assert!(
            !crate::util::get_private_value(scope, wave, "__moliPeriodicWaveNormalize")
                .unwrap()
                .boolean_value(scope)
        );
        for (name, real, imag) in [
            ("defaultWave", vec![0.0, 0.0], vec![0.0, 1.0]),
            ("realOnly", vec![0.0, 2.0], vec![0.0, 0.0]),
            ("imagOnly", vec![0.0, 0.0], vec![0.0, 3.0]),
        ] {
            let owner = global_object(scope, name);
            assert_eq!(
                copied_coefficients(scope, owner, "__moliPeriodicWaveReal"),
                real
            );
            assert_eq!(
                copied_coefficients(scope, owner, "__moliPeriodicWaveImag"),
                imag
            );
            assert!(
                crate::util::get_private_value(scope, owner, "__moliPeriodicWaveNormalize")
                    .unwrap()
                    .boolean_value(scope)
            );
        }
        let oscillator = global_object(scope, "oscillator");
        assert!(
            crate::util::get_private_value(scope, oscillator, "__moliOscillatorPeriodicWave")
                .unwrap()
                .strict_equals(wave.into())
        );
        Ok(())
    })
    .unwrap();
}
