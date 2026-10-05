use super::*;

#[test]
fn video_playback_quality_snapshots_and_receiver_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://video-playback-quality.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    vm.eval(include_str!("video_playback_quality.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn video_playback_quality_native_snapshots_preserve_nonzero_uint32_counters() {
    let mut vm = new_storage_page_task_executor_test_vm("https://video-playback-native.test/");
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let first =
            crate::context_bootstrap::new_video_playback_quality(scope, 12.5, u32::MAX, 8, 3);
        let second = crate::context_bootstrap::new_video_playback_quality(scope, 13.75, 19, 2, 1);
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, first, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let global = scope.get_current_context().global(scope);
        for (name, value) in [
            ("nativeQuality", first),
            ("laterQuality", second),
            ("nativeQualityProxy", proxy.into()),
        ] {
            let name = crate::util::v8str(scope, name);
            assert_eq!(
                global.create_data_property(scope, name.into(), value.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const p = VideoPlaybackQuality.prototype, getter = name => Object.getOwnPropertyDescriptor(p, name).get;
        const values = {creationTime:12.5,totalVideoFrames:2**32-1,droppedVideoFrames:8,corruptedVideoFrames:3};
        for (const [name, expected] of Object.entries(values)) {
          for (const value of [nativeQuality, nativeQualityProxy]) if (getter(name).call(value) !== expected) throw Error(name + ' native value');
        }
        if (laterQuality.totalVideoFrames !== 19 || nativeQuality.totalVideoFrames !== 2**32-1) throw Error('snapshot independence');
        let traps = 0, error;
        try {getter('totalVideoFrames').call(new Proxy(nativeQualityProxy, {get() {traps++;}}));} catch (caught) {error = caught;}
        if (!(error instanceof TypeError) || traps !== 0) throw Error('author proxy');
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn video_playback_quality_accepts_registered_native_video_proxy() {
    let mut vm = new_storage_page_task_executor_test_vm("https://video-playback-proxy.test/");
    vm.eval("globalThis.realVideo = document.createElement('video')")
        .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "realVideo");
        let video = global.get(scope, key.into()).unwrap();
        let video = v8::Local::<v8::Object>::try_from(video).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, video, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeVideoProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const get = HTMLVideoElement.prototype.getVideoPlaybackQuality;
        if (!(get.call(nativeVideoProxy) instanceof VideoPlaybackQuality)) throw Error('registered native video proxy');
        let error, reads = 0;
        try {get.call(new Proxy(nativeVideoProxy, {get() {reads++;}}));} catch (caught) {error = caught;}
        if (!(error instanceof TypeError) || reads !== 0) throw Error('author video proxy');
        return true;
    })()"#).unwrap(), "true");
}
