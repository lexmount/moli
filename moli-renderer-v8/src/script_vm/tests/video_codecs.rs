use super::*;

#[test]
fn video_codec_constructors_validate_before_prototype_lookup_and_use_native_brands() {
    let mut vm = new_storage_page_task_executor_test_vm("https://video-codecs.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const other = document.querySelector('iframe').contentWindow;
        for (const name of ['VideoDecoder', 'VideoEncoder']) {
            const C = window[name], O = other[name], log = [];
            const init = {get error() {log.push('error'); return () => {}},
                get output() {log.push('output'); return () => {}}};
            const target = new Proxy(function() {}, {get(object, key) {
                if (key === 'prototype') log.push('prototype');
                return Reflect.get(object, key);
            }});
            const codec = Reflect.construct(C, [init], target);
            if (log.join(',') !== 'error,output,prototype') throw Error(log);
            const state = Object.getOwnPropertyDescriptor(C.prototype, 'state').get;
            const foreignState = Object.getOwnPropertyDescriptor(O.prototype, 'state').get;
            if (state.call(codec) !== 'unconfigured' || foreignState.call(codec) !== 'unconfigured') throw Error('cross-realm brand');
            for (const receiver of [{}, Object.create(codec), new Proxy(codec, {})]) {
                let reads = 0, caught;
                try { O.prototype.configure.call(receiver, {get codec() {reads++; return 'bogus'}}); }
                catch (error) { caught = error; }
                if (!(caught instanceof other.TypeError) || reads !== 0) throw Error('receiver conversion order');
            }
            C.prototype.close.call(codec);
            let caught;
            try { C.prototype.configure.call(codec, {}); } catch (error) {caught = error;}
            if (!(caught instanceof TypeError)) throw Error('invalid config before closed-state check');
            const marker = {};
            caught = undefined;
            try { new C({get error() {throw marker}, get output() {throw Error('read output')}}); }
            catch (error) {caught = error;}
            if (caught !== marker) throw Error('getter exception preservation');
        }
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn video_codec_registered_native_proxies_share_state_and_reject_author_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://video-codec-native-proxy.test/");
    vm.eval("globalThis.codec = new VideoDecoder({error() {}, output() {}})")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let codec = global
            .get(scope, crate::util::v8str(scope, "codec").into())
            .unwrap();
        let codec = v8::Local::<v8::Object>::try_from(codec).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, codec, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativeProxy").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const proto = VideoDecoder.prototype;
        proto.configure.call(nativeProxy, {codec: 'moli.unsupported'});
        if (codec.state !== 'configured' || nativeProxy.state !== 'configured') throw Error('shared state');
        const chunk = new EncodedVideoChunk({type: 'key', timestamp: 0, data: new Uint8Array([1])});
        Object.defineProperty(chunk, 'type', {get() {throw Error('author chunk type read')}});
        proto.decode.call(nativeProxy, chunk);
        if (codec.decodeQueueSize !== 1) throw Error('native chunk');
        const revoked = Proxy.revocable(chunk, {}); revoked.revoke();
        for (const value of [new Proxy(chunk, {}), revoked.proxy, Object.create(chunk)]) {
            let error;
            try { proto.decode.call(nativeProxy, value); } catch (caught) {error = caught;}
            if (!(error instanceof TypeError) || codec.decodeQueueSize !== 1) throw Error('chunk brand');
        }
        proto.reset.call(nativeProxy);
        if (codec.state !== 'unconfigured' || codec.decodeQueueSize !== 0) throw Error('reset');
        proto.close.call(nativeProxy);
        return codec.state === 'closed';
    })()"#).unwrap(), "true");
}
