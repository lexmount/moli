use super::*;

#[test]
fn media_recorder_frontend_preserves_inactive_state_conversion_handlers_and_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://media-recorder.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    let result = vm
        .eval(include_str!(
            "../../../tests/fixtures/media-recorder-frontend.js"
        ))
        .unwrap();
    assert_eq!(
        result,
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
            .unwrap()
    );
}

#[test]
fn media_recorder_registered_native_proxies_share_private_state_and_stream_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-recorder.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'; globalThis.stream = new MediaStream(); globalThis.recorder = new MediaRecorder(stream)").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [("stream", "nativeStream"), ("recorder", "nativeRecorder")] {
            let value = global
                .get(scope, crate::util::v8str(scope, name).into())
                .unwrap();
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, object, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(
                global.create_data_property(
                    scope,
                    crate::util::v8str(scope, proxy_name).into(),
                    proxy.into()
                ),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const assert = (condition,name) => {if (!condition) throw Error(name)};
      const other = document.querySelector('iframe').contentWindow;
      const state = Object.getOwnPropertyDescriptor(other.MediaRecorder.prototype,'state').get;
      const streamGetter = Object.getOwnPropertyDescriptor(other.MediaRecorder.prototype,'stream').get;
      assert(state.call(nativeRecorder) === 'inactive' && streamGetter.call(nativeRecorder) === stream, 'native recorder proxy shares private slots');
      assert(new other.MediaRecorder(nativeStream).stream === nativeStream, 'stream argument retains registered proxy identity');
      assert(other.MediaRecorder.prototype.stop.call(nativeRecorder) === undefined, 'native proxy method');
      let conversions = 0, error;
      try {other.MediaRecorder.prototype.start.call(nativeRecorder,{valueOf() {conversions++; return 1}})} catch(caught) {error=caught}
      assert(conversions === 1 && error instanceof other.DOMException && error.name === 'NotSupportedError' && !(error instanceof DOMException), 'valid proxy converts input then uses callee error realm');
      let traps = 0;
      const proxy = new Proxy(nativeRecorder, {get() {traps++; throw Error('get')}, getPrototypeOf() {traps++; throw Error('prototype')}});
      const revoked = Proxy.revocable(nativeRecorder,{}); revoked.revoke();
      for (const receiver of [proxy,revoked.proxy,Object.create(nativeRecorder),Object.create(other.MediaRecorder.prototype)]) {
        try {other.MediaRecorder.prototype.start.call(receiver,{valueOf() {conversions++; return 1}})} catch(caught) {error=caught}
        assert(error instanceof other.TypeError && !(error instanceof TypeError), 'author wrapper fails brand check');
      }
      assert(conversions === 1 && traps === 0 && state.call(recorder) === 'inactive', 'no trap or argument conversion on author wrappers');
      const log = [];
      Object.getOwnPropertyDescriptor(other.MediaRecorder.prototype,'onstop').set.call(nativeRecorder, function(event) {assert(this===recorder && event.target===recorder,'EventTarget identity'); log.push('stop')});
      recorder.dispatchEvent(new Event('stop')); assert(log.join() === 'stop', 'native proxy handler uses original EventTarget');
      for (const [object,names] of [[stream,['active','getTracks']],[recorder,['state','stream']]]) {
        for (const name of names) Object.defineProperty(object,name,{get() {throw Error('author getter '+name)}});
      }
      try {other.MediaRecorder.prototype.start.call(nativeRecorder)} catch(caught) {error=caught}
      assert(error instanceof other.DOMException && error.name==='NotSupportedError', 'private state checks ignore author properties');
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn media_recorder_inert_live_tracks_remain_unrecorded_and_options_convert_before_allocation() {
    let mut vm = new_storage_page_task_executor_test_vm("https://recorder-options.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    super::media_streams::install_inert_media_tracks(&mut vm);
    assert_eq!(vm.eval(r#"(() => {
      const assert = (condition,name) => {if (!condition) throw Error(name)};
      const other = document.querySelector('iframe').contentWindow, stream = new MediaStream([audio,video]);
      assert(stream.active, 'native live inert track stream');
      for (const options of [{},{videoKeyFrameIntervalCount:0},{videoKeyFrameIntervalDuration:0},{videoKeyFrameIntervalCount:0,videoKeyFrameIntervalDuration:0}]) {
        const recorder = new other.MediaRecorder(stream,options), events = [];
        for (const name of ['start','stop','dataavailable','pause','resume','error']) recorder.addEventListener(name,()=>events.push(name));
        let error; try {recorder.start()} catch(caught) {error=caught}
        assert(error instanceof other.DOMException && error.name==='NotSupportedError' && recorder.state==='inactive' && recorder.mimeType==='', 'frontend cannot encode inert tracks');
        recorder.stop(); assert(events.length===0, 'no fabricated backend events');
      }
      for (const type of ['video/webm','audio/ogg','video/mp4','audio/wav','Audio/WebM',' video/webm','\ud800']) assert(!other.MediaRecorder.isTypeSupported(type), 'no recording encoder support');
      const log = [], target = new Proxy(function() {}, {get(object,key) {if(key==='prototype') log.push('prototype'); return Reflect.get(object,key)}});
      const options = {get mimeType() {log.push('type'); return 'audio/banana'}, get videoBitsPerSecond() {log.push('rate'); return 1}};
      let error; try {Reflect.construct(other.MediaRecorder,[stream,options],target)} catch(caught) {error=caught}
      assert(error instanceof other.DOMException && error.name==='NotSupportedError' && log.join()==='type,rate,prototype','conversion, allocation, backend validation order');
      const marker = {}; log.length=0;
      try {Reflect.construct(other.MediaRecorder,[stream,{get mimeType() {log.push('type'); throw marker}}],target)} catch(caught) {error=caught}
      assert(error===marker && log.join()==='type','conversion exception prevents prototype lookup');
      const savedRecorder = other.MediaRecorder, savedStream = MediaStream;
      other.MediaRecorder = function() {throw Error('poisoned MediaRecorder')};
      globalThis.MediaStream = function() {throw Error('poisoned MediaStream')};
      const recorder = new savedRecorder(stream);
      assert(recorder.stream===stream && recorder.state==='inactive','intrinsic brands do not depend on public constructors');
      other.MediaRecorder = savedRecorder; globalThis.MediaStream = savedStream;
      return true;
    })()"#).unwrap(), "true");
}
