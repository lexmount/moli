use super::*;

fn install_frames(vm: &mut ScriptVm) {
    use crate::context_bootstrap::encoded_frames::{FrameKind, native_frame_for_test};
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, kind, metadata) in [
            (
                "audioFrame",
                FrameKind::Audio,
                r#"{"rtpTimestamp":4294967295,"contributingSources":[7,8],"mimeType":"audio/opus","sequenceNumber":-3,"audioLevel":0.25}"#,
            ),
            (
                "videoFrame",
                FrameKind::Video,
                r#"{"rtpTimestamp":123,"contributingSources":[7,8],"mimeType":"video/VP8","dependencies":[1,2],"frameId":3,"width":640,"height":480,"timestamp":-123}"#,
            ),
        ] {
            let json = crate::util::v8str(scope, metadata);
            let metadata = v8::json::parse(scope, json).unwrap().to_object(scope).unwrap();
            let frame = native_frame_for_test(scope, kind, &[1, 2, 3, 4], metadata);
            assert_eq!(
                global.create_data_property(scope, crate::util::v8str(scope, name).into(), frame.into()),
                Some(true)
            );
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, frame, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let proxy_name = format!("native{name}");
            assert_eq!(
                global.create_data_property(scope, crate::util::v8_string(scope, &proxy_name).unwrap().into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    }).unwrap();
}

#[test]
fn rtc_encoded_frame_interfaces_reject_invalid_originals_and_receivers_before_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-encoded-frames.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    assert_eq!(
        vm.eval(include_str!(
            "../../../tests/fixtures/rtc-encoded-frame-interfaces.js"
        ))
        .unwrap(),
        "true"
    );
}

#[test]
fn rtc_encoded_frame_native_copies_use_internal_data_and_inherited_metadata_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-encoded-frames.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    install_frames(&mut vm);
    assert_eq!(
        vm.eval(include_str!(
            "../../../tests/fixtures/rtc-encoded-frame-values.js"
        ))
        .unwrap(),
        "true"
    );
}

#[test]
fn rtc_encoded_frame_structured_clones_preserve_graph_aliases_and_transfer_data() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-encoded-frames.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    install_frames(&mut vm);
    assert_eq!(vm.eval(r#"(() => {
      const assert = (value, name) => { if (!value) throw Error(name); };
      const other = document.querySelector('iframe').contentWindow;
      for (const [name, frame] of [['RTCEncodedAudioFrame', audioFrame], ['RTCEncodedVideoFrame', videoFrame]]) {
        for (const ordered of [[frame, frame.data, frame], [frame.data, frame, frame]]) {
          const clone = other.structuredClone(ordered);
          const [copied, data] = ordered[0] === frame ? [clone[0], clone[1]] : [clone[1], clone[0]];
          assert(copied instanceof other[name] && copied !== frame && copied === clone[2], name + ' graph identity');
          assert(data instanceof other.ArrayBuffer && copied.data === data && data !== frame.data, name + ' nested buffer alias');
          assert(Array.from(new Uint8Array(data)).join() === '1,2,3,4', name + ' copied bytes');
          assert(JSON.stringify(copied.getMetadata()) === JSON.stringify(frame.getMetadata()), name + ' copied metadata');
        }
        const oldData = frame.data;
        const clone = other.structuredClone([frame, oldData, frame], {transfer: [oldData]});
        assert(oldData.byteLength === 0 && frame.data === oldData, name + ' transfer detaches exposed data');
        assert(clone[0].data === clone[1] && clone[0] === clone[2] && clone[1].byteLength === 4, name + ' transferred alias');
        let error;
        try { new window[name](frame); } catch (caught) { error = caught; }
        assert(error instanceof TypeError, name + ' copying detached data throws');
        for (const value of [frame, {frame}]) {
          error = undefined;
          try { structuredClone(value); } catch (caught) { error = caught; }
          assert(error instanceof DOMException && error.name === 'DataCloneError', name + ' detached nested data rejects');
        }
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn rtc_encoded_frame_storage_serialization_is_rejected_before_author_properties() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-encoded-frames.test/");
    install_frames(&mut vm);
    assert_eq!(vm.eval(r#"(() => {
      for (const frame of [audioFrame, videoFrame]) {
        for (const name of ['data', 'getMetadata', 'type']) Object.defineProperty(frame, name, {get() {throw Error('public frame property')}});
        let error;
        try { history.replaceState({frame}, ''); } catch (caught) { error = caught; }
        if (!(error instanceof DOMException) || error.name !== 'DataCloneError') throw Error('frame stored in history');
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn rtc_encoded_frame_cross_isolate_clone_respects_worker_exposure_and_private_intrinsics() {
    use crate::context_bootstrap::{
        exposed_interfaces::RealmKind, install_worker_lazy_exposed_interfaces,
    };
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-encoded-frames.test/");
    install_frames(&mut vm);
    vm.eval("globalThis.__wireSource = [audioFrame, audioFrame.data, audioFrame, videoFrame, videoFrame.data]; true").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    let payload = vm
        .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let value = global
                .get(scope, crate::util::v8str(scope, "__wireSource").into())
                .unwrap();
            Ok(crate::structured_clone::serialize_for_wire_for_runtime(scope, value).unwrap())
        })
        .unwrap();
    for realm in [
        RealmKind::DedicatedWorker,
        RealmKind::SharedWorker,
        RealmKind::ServiceWorker,
    ] {
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);
        install_worker_lazy_exposed_interfaces(scope, global, realm, true).unwrap();
        crate::context_bootstrap::exposed_interfaces::capture_eager_intrinsic_interfaces(
            scope, global, realm,
        )
        .unwrap();
        let source = crate::util::v8str(
            scope,
            "for (const name of ['RTCEncodedAudioFrame','RTCEncodedVideoFrame']) Object.defineProperty(globalThis, name, {get() {throw Error('public constructor lookup');}, configurable: true}); true",
        );
        let script = v8::Script::compile(scope, source, None).unwrap();
        crate::script_execution::execute_compiled_script(scope, script).unwrap();
        let catch = std::pin::pin!(v8::TryCatch::new(scope));
        let scope = &mut catch.init();
        let value = crate::structured_clone::deserialize_from_wire(scope, &payload);
        if matches!(realm, RealmKind::DedicatedWorker) {
            let value = value.expect("dedicated workers accept encoded frame values");
            let values = v8::Local::<v8::Array>::try_from(value).unwrap();
            let audio = values
                .get_index(scope, 0)
                .unwrap()
                .to_object(scope)
                .unwrap();
            let video = values
                .get_index(scope, 3)
                .unwrap()
                .to_object(scope)
                .unwrap();
            assert!(crate::web_api_interfaces::RTCEncodedAudioFrame::is_instance(scope, audio));
            assert!(crate::web_api_interfaces::RTCEncodedVideoFrame::is_instance(scope, video));
            assert!(
                values
                    .get_index(scope, 2)
                    .unwrap()
                    .strict_equals(audio.into())
            );
            assert_eq!(
                global.create_data_property(
                    scope,
                    crate::util::v8str(scope, "__wire").into(),
                    value
                ),
                Some(true)
            );
            let source = crate::util::v8str(
                scope,
                "__wire[0].data === __wire[1] && __wire[3].data === __wire[4] && __wire[3].type === 'key' && __wire[0].getMetadata().mimeType === 'audio/opus' && new Uint8Array(__wire[4]).join() === '1,2,3,4'",
            );
            let script = v8::Script::compile(scope, source, None).unwrap();
            assert!(
                crate::script_execution::execute_compiled_script(scope, script)
                    .unwrap()
                    .boolean_value(scope)
            );
        } else {
            assert!(
                value.is_none(),
                "frames are not exposed in shared/service workers"
            );
            let error = scope.exception().unwrap().to_object(scope).unwrap();
            assert_eq!(
                error
                    .get(scope, crate::util::v8str(scope, "name").into())
                    .unwrap()
                    .to_rust_string_lossy(scope),
                "DataCloneError"
            );
        }
    }
}
