use super::*;

#[test]
fn audio_buffers_use_native_channel_storage_and_webidl_bindings() {
    let mut vm = new_storage_page_task_executor_test_vm("https://audio-buffers.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(&format!("({}).then(value => globalThis.__audioBufferDone = value, error => globalThis.__audioBufferDone = String(error));", include_str!("audio_buffer_interfaces.js"))).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__audioBufferDone").unwrap(), "true");
}

fn audio_channel_stores(
    vm: &mut StandaloneScriptVmHarness,
    object_name: &'static str,
    slot: &str,
) -> Vec<v8::SharedRef<v8::BackingStore>> {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, object_name);
        let object = global.get(scope, key.into()).unwrap();
        let object = v8::Local::<v8::Object>::try_from(object).unwrap();
        let channels = crate::util::get_private_value(scope, object, slot).unwrap();
        let channels = v8::Local::<v8::Array>::try_from(channels).unwrap();
        Ok((0..channels.length())
            .map(|index| {
                let channel = channels.get_index(scope, index).unwrap();
                v8::Local::<v8::ArrayBuffer>::try_from(channel)
                    .unwrap()
                    .get_backing_store()
            })
            .collect())
    })
    .unwrap()
}

fn acquired_source_stores(
    vm: &mut StandaloneScriptVmHarness,
    name: &'static str,
) -> Vec<v8::SharedRef<v8::BackingStore>> {
    audio_channel_stores(vm, name, "__moliBufferSourceAcquiredBuffer")
}

fn first_sample(store: &v8::BackingStore) -> f32 {
    f32::from_ne_bytes(std::array::from_fn(|index| store[index].get()))
}

#[test]
fn audio_acquisition_shares_pcm_and_separates_only_writable_channels() {
    let mut vm = new_storage_test_vm("https://audio-acquisition.test/");
    vm.exec(
        r#"
        var audioContext = new OfflineAudioContext(2, 128, 48000);
        var audioBuffer = audioContext.createBuffer(2, 8, 48000);
        var left = audioBuffer.getChannelData(0), right = audioBuffer.getChannelData(1);
        var alias = new Float32Array(left.buffer);
        left.fill(3); right.fill(4);
        function startSource() {
            const source = audioContext.createBufferSource();
            source.buffer = audioBuffer;
            source.start();
            return source;
        }
        "#,
        None,
    )
    .unwrap();
    let original = audio_channel_stores(&mut vm, "audioBuffer", "__moliAudioBufferChannels");
    vm.exec("var first = startSource();", None).unwrap();
    assert_eq!(
        vm.eval("[left.length, right.length, alias.length].join()")
            .unwrap(),
        "0,0,0",
        "acquisition must detach all aliases before sharing their backing stores"
    );
    let first = acquired_source_stores(&mut vm, "first");
    for channel in 0..2 {
        assert_eq!(first[channel].data(), original[channel].data());
    }

    vm.exec(
        r#"
        var readback = new Float32Array(8);
        audioBuffer.copyFromChannel(readback, 0);
        audioBuffer.copyToChannel(new Float32Array(0), 0);
        audioBuffer.copyToChannel(new Float32Array([99]), 1, 100);
        try { audioBuffer.copyToChannel(new Float32Array([99]), 99); } catch (_) {}
        var second = startSource();
        var convolver = new ConvolverNode(audioContext, {buffer: audioBuffer});
        "#,
        None,
    )
    .unwrap();
    let second = acquired_source_stores(&mut vm, "second");
    let convolver = audio_channel_stores(&mut vm, "convolver", "__moliConvolverAcquiredContent");
    for channel in 0..2 {
        assert_eq!(first[channel].data(), second[channel].data());
        assert_eq!(first[channel].data(), convolver[channel].data());
    }
    assert_eq!(
        vm.eval("readback.every(value => value === 3)").unwrap(),
        "true"
    );

    vm.exec(
        "var writable = audioBuffer.getChannelData(0); writable[0] = 9;",
        None,
    )
    .unwrap();
    let channels = audio_channel_stores(&mut vm, "audioBuffer", "__moliAudioBufferChannels");
    assert_ne!(first[0].data(), channels[0].data());
    assert_eq!(
        first[1].data(),
        channels[1].data(),
        "the untouched channel stays shared"
    );
    assert_eq!(first_sample(&first[0]), 3.0);
    assert_eq!(first_sample(&channels[0]), 9.0);
    assert_eq!(
        vm.eval("writable === audioBuffer.getChannelData(0)")
            .unwrap(),
        "true"
    );

    vm.exec(
        "var third = startSource(); convolver.buffer = audioBuffer;",
        None,
    )
    .unwrap();
    let third = acquired_source_stores(&mut vm, "third");
    let convolver = audio_channel_stores(&mut vm, "convolver", "__moliConvolverAcquiredContent");
    for channel in 0..2 {
        assert_eq!(third[channel].data(), channels[channel].data());
        assert_eq!(third[channel].data(), convolver[channel].data());
    }
    assert_eq!(vm.eval("writable.length").unwrap(), "0");

    vm.exec(
        "audioBuffer.copyToChannel(new Float32Array([10]), 1); var fourth = startSource();",
        None,
    )
    .unwrap();
    let fourth = acquired_source_stores(&mut vm, "fourth");
    assert_eq!(third[0].data(), fourth[0].data());
    assert_ne!(third[1].data(), fourth[1].data());
    assert_eq!(first_sample(&first[0]), 3.0);
    assert_eq!(first_sample(&first[1]), 4.0);
    assert_eq!(first_sample(&third[0]), 9.0);
    assert_eq!(first_sample(&third[1]), 4.0);
    assert_eq!(first_sample(&fourth[0]), 9.0);
    assert_eq!(first_sample(&fourth[1]), 10.0);
}

#[test]
fn audio_acquisition_does_not_share_storage_transferred_to_script() {
    let mut vm = new_storage_test_vm("https://audio-transfer.test/");
    vm.exec(
        r#"
        var audioContext = new OfflineAudioContext(2, 128, 48000);
        var audioBuffer = audioContext.createBuffer(2, 8, 48000);
        var left = audioBuffer.getChannelData(0), right = audioBuffer.getChannelData(1);
        var transferred = structuredClone(left.buffer, {transfer: [left.buffer]});
        var source = audioContext.createBufferSource();
        source.buffer = audioBuffer; source.start();
        new Float32Array(transferred)[0] = 9;
        "#,
        None,
    )
    .unwrap();
    assert!(acquired_source_stores(&mut vm, "source").is_empty());
    assert_eq!(
        vm.eval("right.length").unwrap(),
        "8",
        "failed acquisition leaves other views attached"
    );
    assert_eq!(
        vm.eval("new Float32Array(transferred)[0]").unwrap(),
        "9",
        "script retains its transferred storage even though acquisition failed"
    );
}
