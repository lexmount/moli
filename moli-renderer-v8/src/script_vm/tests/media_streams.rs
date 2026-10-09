use super::*;

#[test]
fn media_stream_frontend_preserves_empty_streams_overloads_receivers_and_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://media-streams.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    assert_eq!(
        vm.eval(include_str!(
            "../../../tests/fixtures/media-stream-frontend.js"
        ))
        .unwrap(),
        "true"
    );
}

pub(super) fn install_inert_media_tracks(vm: &mut ScriptVm) {
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name, kind, label) in [
            ("audio", "nativeaudio", "audio", &[0x61, 0xd800][..]),
            ("video", "nativevideo", "video", &[0x76][..]),
        ] {
            let track = crate::context_bootstrap::inert_track_for_test(scope, kind, label);
            assert_eq!(
                global.create_data_property(
                    scope,
                    crate::util::v8str(scope, name).into(),
                    track.into()
                ),
                Some(true)
            );
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, track, handler).unwrap();
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
}

#[test]
fn native_media_track_sets_snapshots_clones_and_stop_use_private_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-media-streams.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    install_inert_media_tracks(&mut vm);
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name)};
      const other = document.querySelector('iframe').contentWindow;
      const state = Object.getOwnPropertyDescriptor(MediaStreamTrack.prototype, 'readyState').get;
      const tracks = [audio, video];
      const stream = new MediaStream([audio, nativeaudio, audio, video]);
      assert(stream.getTracks().length === 2 && stream.active, 'native identity deduplication');
      assert(stream.getAudioTracks()[0] === audio && stream.getVideoTracks()[0] === video, 'native kind filters');
      assert(audio.label === 'a\ud800' && audio.enabled && !audio.muted && audio.readyState === 'live', 'source metadata');
      const id = audio.id;
      for (const [key, value] of [['id', id], ['kind', 'audio'], ['readyState', 'live']]) {
        Object.defineProperty(audio, key, {get() {throw Error('author track getter read: '+key)}});
      }
      assert(stream.getTrackById(id) === audio && stream.getAudioTracks()[0] === audio && stream.active, 'private metadata');
      const snapshot = stream.getTracks(); snapshot.length = 0;
      assert(stream.getTracks().length === 2, 'fresh track set snapshot');
      const copied = new other.MediaStream(stream);
      assert(copied instanceof other.MediaStream && copied.id !== stream.id && copied.getAudioTracks()[0] === audio, 'copy shares original tracks');
      Object.defineProperty(stream, Symbol.iterator, {get() {throw Error('stream overload consulted iterator')}});
      assert(new MediaStream(stream).getTracks()[0] === audio, 'native stream overload');
      let conversions = 0;
      audio.enabled = {valueOf() {conversions++; throw Error('ToBoolean coercion')}};
      assert(conversions === 0 && audio.enabled, 'boolean setter does not coerce objects');
      audio.enabled = false;
      assert(stream.active, 'disabled tracks remain live');
      let ended = 0; audio.onended = () => ended++; audio.addEventListener('ended', () => ended++);
      let mutations = 0; stream.onaddtrack = stream.onremovetrack = () => mutations++;
      stream.removeTrack(nativeaudio);
      assert(stream.getAudioTracks().length === 0 && state.call(audio) === 'live', 'native proxy removal');
      stream.addTrack(nativeaudio); stream.addTrack(audio);
      assert(stream.getTracks().length === 2 && mutations === 0, 'script mutations have no notifications');
      const cloned = other.MediaStream.prototype.clone.call(stream);
      assert(cloned instanceof MediaStream && cloned.id !== stream.id && cloned.getTracks().length === 2, 'clone allocation realm');
      const audioClone = cloned.getAudioTracks()[0];
      assert(audioClone instanceof MediaStreamTrack && audioClone !== audio && audioClone.id !== id && audioClone.label === 'a\ud800' && !audioClone.enabled, 'native track clone metadata');
      assert(copied.getTracks() instanceof other.Array && copied.clone().getTracks()[0] instanceof other.MediaStreamTrack, 'stream allocation realm applies to cloned tracks');
      MediaStreamTrack.prototype.stop.call(nativeaudio);
      assert(ended === 0 && copied.getAudioTracks()[0] === audio, 'stop has no ended event or removal');
      assert(state.call(audio) === 'ended' && audioClone.readyState === 'live', 'stopping one clone leaves the source in use');
      MediaStreamTrack.prototype.stop.call(video);
      assert(!stream.active && !copied.active && cloned.active, 'active follows shared original track states');
      const endedClone = other.MediaStreamTrack.prototype.clone.call(audio);
      assert(endedClone.readyState === 'ended' && endedClone.id !== id, 'ended track clone');
      audio.enabled = true;
      assert(audio.enabled, 'ended track enabled setter');
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn native_media_track_sequences_and_events_reject_author_proxies_without_traps() {
    let mut vm = new_storage_page_task_executor_test_vm("https://media-track-brands.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    install_inert_media_tracks(&mut vm);
    assert_eq!(vm.eval(r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name)};
      const other = document.querySelector('iframe').contentWindow;
      let traps = 0;
      const proxy = new Proxy(audio, {get() {traps++; throw Error('get')}, getPrototypeOf() {traps++; throw Error('prototype')}});
      const revoked = Proxy.revocable(audio, {}); revoked.revoke();
      const stream = new MediaStream();
      for (const invalid of [{}, Object.create(audio), Object.create(MediaStreamTrack.prototype), proxy, revoked.proxy]) {
        for (const action of [() => new other.MediaStream([invalid]), () => other.MediaStream.prototype.addTrack.call(stream, invalid),
            () => new other.MediaStreamTrackEvent('addtrack', {track: invalid}), () => other.MediaStreamTrack.prototype.clone.call(invalid)]) {
          let error; try {action()} catch(caught) {error=caught}
          assert(error instanceof other.TypeError && !(error instanceof TypeError), 'callee realm brand errors');
        }
      }
      assert(traps === 0 && stream.getTracks().length === 0, 'invalid native inputs are not unwrapped');
      const log = [], marker = {};
      const iterable = {[Symbol.iterator]() {log.push('iterator'); return {next() {log.push('next'); return {value: proxy, done:false}}, get return() {throw Error('IteratorClose')}}}};
      let error; try {new MediaStream(iterable)} catch(caught) {error=caught}
      assert(error instanceof TypeError && log.join() === 'iterator,next' && traps === 0, 'sequence converts each item before next and does not IteratorClose');
      const event = new other.MediaStreamTrackEvent('addtrack', {bubbles:true,cancelable:true,composed:true,track:nativeaudio});
      assert(event.track === nativeaudio && event.track === event.track && event.bubbles && event.cancelable && event.composed && !event.isTrusted, 'native track event');
      const descriptor = Object.getOwnPropertyDescriptor(MediaStreamTrackEvent.prototype, 'track');
      assert(descriptor.get.call(event) === nativeaudio && !descriptor.set, 'track event cross-realm accessor');
      const getterLog = [];
      const init = {get bubbles() {getterLog.push('bubbles'); return false}, get cancelable() {getterLog.push('cancelable'); return false},
        get composed() {getterLog.push('composed'); return false}, get track() {getterLog.push('track'); throw marker}};
      try {new MediaStreamTrackEvent('type', init)} catch(caught) {error=caught}
      assert(error === marker && getterLog.join() === 'bubbles,cancelable,composed,track', 'inherited dictionary conversion and exception identity');
      return true;
    })()"#).unwrap(), "true");
}
