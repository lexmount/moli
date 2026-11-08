use super::*;

#[test]
fn media_player_initialization_can_register_controls_and_connect_gain_nodes() {
    let mut vm = new_storage_html_test_vm("http://media-player.test/");
    assert_eq!(
        vm.eval(include_str!("media_player_init.js"))
            .expect("media-player initialization should complete"),
        "ok"
    );
}

#[test]
fn media_session_is_same_object_with_native_receivers_and_per_window_state() {
    let mut vm = new_storage_html_test_vm("http://media-session.test/");
    assert_eq!(
        vm.eval(r#"
(() => {
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const typeError = (fn) => {
    try { fn(); } catch (error) { assert(error.name === 'TypeError', error.name); return; }
    throw new Error('expected TypeError');
  };
  const session = navigator.mediaSession;
  assert(session === navigator.mediaSession && session instanceof MediaSession, 'SameObject');
  assert(Object.getPrototypeOf(session) === MediaSession.prototype, 'native prototype');
  assert(!Object.hasOwn(navigator, 'mediaSession'), 'prototype getter');
  assert(Reflect.set(navigator, 'mediaSession', {}) === false, 'readonly');
  assert(session.playbackState === 'none', 'initial playback state');
  typeError(() => new MediaSession());
  assert(session.setActionHandler.length === 2, 'method arity');
  for (const action of ['play','pause','seekbackward','seekforward','previoustrack',
      'nexttrack','skipad','stop','seekto','togglemicrophone','togglecamera','hangup',
      'previousslide','nextslide','enterpictureinpicture']) {
    assert(session.setActionHandler(action, () => { throw new Error('must not invoke'); }) === undefined, 'register');
    session.setActionHandler(action, null);
    session.setActionHandler(action, undefined);
  }
  typeError(() => session.setActionHandler());
  typeError(() => session.setActionHandler('play'));
  typeError(() => session.setActionHandler('unknown', () => {}));
  for (const bad of [0, 'callback', {}, {handleEvent(){}}]) {
    typeError(() => session.setActionHandler('play', bad));
  }
  const marker = new Error('conversion');
  try { session.setActionHandler({toString(){throw marker;}}, null); }
  catch (e) { assert(e === marker, 'action conversion exception'); }
  for (const state of ['playing','paused','none']) {
    session.playbackState = state;
    assert(session.playbackState === state, 'playback state '+state);
    session.playbackState = 'invalid';
    assert(session.playbackState === state, 'invalid enum ignored');
  }
  typeError(() => {session.playbackState = Symbol();});
  const descriptor = Object.getOwnPropertyDescriptor(MediaSession.prototype, 'playbackState');
  for (const bad of [{}, MediaSession.prototype, Object.create(session), new Proxy(session,{})]) {
    typeError(() => session.setActionHandler.call(bad, 'play', null));
    typeError(() => descriptor.get.call(bad));
    typeError(() => descriptor.set.call(bad, 'playing'));
  }
  document.body.innerHTML = '<iframe></iframe>';
  const other = document.querySelector('iframe').contentWindow;
  const getter = Object.getOwnPropertyDescriptor(Navigator.prototype, 'mediaSession').get;
  const otherSession = getter.call(other.navigator);
  assert(otherSession === other.navigator.mediaSession && otherSession !== session, 'per-window identity');
  assert(Object.getPrototypeOf(otherSession) === other.MediaSession.prototype, 'receiver realm');
  otherSession.playbackState = 'playing';
  assert(session.playbackState === 'none', 'isolated state');
  session.setActionHandler.call(otherSession, 'play', null);
  return 'ok';
})()
"#).expect("MediaSession identity and Web IDL contracts should evaluate"),
        "ok"
    );
}

#[test]
fn gain_factory_creates_independent_parameters_and_validates_graph_connections() {
    let mut vm = new_storage_html_test_vm("https://gain-node.test/");
    assert_eq!(vm.eval(r#"
(() => {
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const throws = (name, fn) => {
    try { fn(); } catch (e) { assert(e.name === name, e.name+' != '+name); return; }
    throw new Error('expected '+name);
  };
  assert(Object.hasOwn(BaseAudioContext.prototype, 'createGain'), 'shared factory');
  assert(BaseAudioContext.prototype.createGain.length === 0, 'factory arity');
  for (const context of [new AudioContext(), new OfflineAudioContext(1,64,44100)]) {
    const gain = context.createGain(), other = context.createGain();
    assert(gain instanceof GainNode && gain instanceof AudioNode, 'node inheritance');
    assert(gain.context === context, 'context ownership');
    assert(gain.numberOfInputs === 1 && gain.numberOfOutputs === 1, 'ports');
    assert(gain.channelCount === 2 && gain.channelCountMode === 'max' && gain.channelInterpretation === 'speakers', 'channel defaults');
    assert(gain.gain instanceof AudioParam && gain.gain === gain.gain, 'SameObject parameter');
    assert(gain.gain.value === 1 && gain.gain.defaultValue === 1, 'default gain');
    assert(gain.gain.minValue === -3.4028234663852886e38 && gain.gain.maxValue === 3.4028234663852886e38, 'float bounds');
    assert(!Object.hasOwn(gain,'gain') && Reflect.set(gain,'gain',{}) === false, 'readonly parameter');
    gain.gain.value = 0.5;
    assert(other.gain.value === 1 && gain.gain.defaultValue === 1, 'independent parameters');
    assert(gain.gain.setValueAtTime(0.25,0) === gain.gain, 'existing AudioParam methods');
    assert(gain.connect(other) === other && other.connect(context.destination) === context.destination, 'graph connectivity');
    throws('InvalidAccessError', () => gain.connect(new AudioContext().destination));
    throws('IndexSizeError', () => gain.connect(other,1));
    assert(gain.disconnect(other) === undefined, 'remove connection');
    throws('InvalidAccessError', () => gain.disconnect(other));
    const getter = Object.getOwnPropertyDescriptor(GainNode.prototype,'gain').get;
    for (const bad of [{}, GainNode.prototype, Object.create(gain), new Proxy(gain,{})]) {
      throws('TypeError', () => getter.call(bad));
    }
    for (const bad of [{}, BaseAudioContext.prototype, Object.create(context), new Proxy(context,{})]) {
      throws('TypeError', () => BaseAudioContext.prototype.createGain.call(bad));
    }
  }
  return 'ok';
})()
"#).expect("GainNode factory and graph contracts should evaluate"), "ok");
}

#[test]
fn media_element_audio_source_validates_ownership_receivers_and_ports() {
    let mut vm = new_storage_html_test_vm("http://media-source-node.test/");
    assert_eq!(vm.eval(r#"
(() => {
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const throws = (name, fn) => {
    try { fn(); } catch (e) { assert(e.name === name, e.name+' != '+name); return; }
    throw new Error('expected '+name);
  };
  const context = new AudioContext();
  assert(context.createMediaElementSource.length === 1, 'method arity');
  assert(typeof OfflineAudioContext.prototype.createMediaElementSource === 'undefined', 'realtime only');
  for (const tag of ['audio','video']) {
    const element = document.createElement(tag);
    const source = context.createMediaElementSource(element);
    assert(source instanceof MediaElementAudioSourceNode && source instanceof AudioNode, 'inheritance');
    assert(source.context === context && source.mediaElement === element, 'ownership');
    assert(source.numberOfInputs === 0 && source.numberOfOutputs === 1, 'source ports');
    assert(Reflect.set(source,'mediaElement',{}) === false, 'readonly media element');
    const gain = context.createGain();
    assert(source.connect(gain) === gain, 'source connection');
    throws('IndexSizeError', () => gain.connect(source));
    throws('InvalidStateError', () => context.createMediaElementSource(element));
    throws('InvalidStateError', () => new AudioContext().createMediaElementSource(element));
    source.disconnect();
    throws('InvalidStateError', () => context.createMediaElementSource(element));
    assert(context.createMediaElementSource(element.cloneNode()).mediaElement !== element, 'clone has independent ownership');
    const getter = Object.getOwnPropertyDescriptor(MediaElementAudioSourceNode.prototype,'mediaElement').get;
    for (const bad of [{}, Object.create(source), new Proxy(source,{})]) {
      throws('TypeError', () => getter.call(bad));
    }
  }
  for (const bad of [undefined, null, {}, document.createElement('div'), Object.create(new Audio()), new Proxy(new Audio(),{})]) {
    throws('TypeError', () => context.createMediaElementSource(bad));
  }
  for (const bad of [{}, new OfflineAudioContext(1,64,44100), Object.create(context), new Proxy(context,{})]) {
    throws('TypeError', () => context.createMediaElementSource.call(bad,new Audio()));
  }
  return 'ok';
})()
"#).expect("MediaElementAudioSourceNode contracts should evaluate"), "ok");
}

#[test]
fn media_session_accepts_screen_share_and_voice_activity_actions() {
    let mut vm = new_storage_html_test_vm("https://media-session-actions.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
      for (const action of ['togglescreenshare', 'voiceactivity']) {
        navigator.mediaSession.setActionHandler(action, () => {});
        navigator.mediaSession.setActionHandler(action, null);
      }
      return 'ok';
    })()"#
        )
        .unwrap(),
        "ok"
    );
}

#[test]
fn gain_constructor_converts_options_and_preserves_native_defaults() {
    let mut vm = new_storage_html_test_vm("https://gain-constructor.test/");
    assert_eq!(vm.eval(r#"(() => {
      const check = (ok, message) => { if (!ok) throw Error(message); };
      const throws = (name, fn) => {
        try { fn(); } catch (e) { check(e.name === name, e.name); return; }
        throw Error('expected '+name);
      };
      check(GainNode.length === 1, 'arity');
      for (const context of [new AudioContext(), new OfflineAudioContext(1,64,44100)]) {
        const node = new GainNode(context, {gain:0.1,channelCount:1,
          channelCountMode:'explicit',channelInterpretation:'discrete'});
        check(node instanceof GainNode && node instanceof AudioNode, 'inheritance');
        check(node.context === context && node.gain.value === Math.fround(0.1), 'context and gain');
        check(node.gain.defaultValue === 1 && new GainNode(context).gain.value === 1, 'native defaults');
        check(node.channelCount === 1 && node.channelCountMode === 'explicit' && node.channelInterpretation === 'discrete', 'channel options');
        check(node.connect(context.destination) === context.destination, 'graph');
        class CustomGain extends GainNode {}
        const custom = new CustomGain(context, {gain:0.25});
        check(custom instanceof CustomGain && custom.gain.value === 0.25, 'subclass');
        for (const gain of [NaN, Infinity, -Infinity]) throws('TypeError', () => new GainNode(context, {gain}));
        throws('NotSupportedError', () => new GainNode(context, {channelCount:0}));
        throws('TypeError', () => new GainNode(context, {channelCountMode:'invalid'}));
        throws('TypeError', () => new GainNode(context, {channelInterpretation:'invalid'}));
        const order = [];
        new GainNode(context, {
          get channelCount(){order.push('count');return 2},
          get channelCountMode(){order.push('mode');return 'max'},
          get channelInterpretation(){order.push('interpretation');return 'speakers'},
          get gain(){order.push('gain');return 0.5}
        });
        check(order.join() === 'count,mode,interpretation,gain', 'dictionary order');
      }
      throws('TypeError', () => GainNode(new AudioContext()));
      for (const context of [undefined,null,{},Object.create(new AudioContext())]) throws('TypeError', () => new GainNode(context));
      return 'ok';
    })()"#).unwrap(), "ok");
}

#[test]
fn media_element_source_constructor_and_factory_share_ownership() {
    let mut vm = new_storage_html_test_vm("https://source-constructor.test/");
    assert_eq!(vm.eval(r#"(() => {
      const check = (ok, message) => { if (!ok) throw Error(message); };
      const throws = (name, fn) => {
        try { fn(); } catch (e) { check(e.name === name, e.name); return; }
        throw Error('expected '+name);
      };
      const context = new AudioContext();
      check(MediaElementAudioSourceNode.length === 2, 'arity');
      for (const tag of ['audio', 'video']) {
        const mediaElement = document.createElement(tag);
        class Source extends MediaElementAudioSourceNode {}
        const node = new Source(context, {mediaElement,
          get channelCount(){throw Error('unknown member must not be read')},
          get channelCountMode(){throw Error('unknown member must not be read')}});
        check(node instanceof Source && node instanceof AudioNode, 'inheritance');
        check(node.context === context && node.mediaElement === mediaElement, 'ownership');
        check(node.numberOfInputs === 0 && node.numberOfOutputs === 1 && node.channelCount === 2, 'ports');
        throws('InvalidStateError', () => context.createMediaElementSource(mediaElement));
        throws('InvalidStateError', () => new MediaElementAudioSourceNode(new AudioContext(), {mediaElement}));
        const clone = mediaElement.cloneNode();
        const factory = context.createMediaElementSource(clone);
        throws('InvalidStateError', () => new MediaElementAudioSourceNode(context, {mediaElement:clone}));
        check(factory.connect(context.createGain()) instanceof GainNode, 'factory graph');
      }
      const mediaElement = new Audio();
      const marker = Error('dictionary getter');
      try { new MediaElementAudioSourceNode(context, {get mediaElement(){throw marker}}); }
      catch(e) { check(e === marker, 'conversion exception'); }
      check(context.createMediaElementSource(mediaElement).mediaElement === mediaElement, 'failed constructor must not bind');
      throws('TypeError', () => MediaElementAudioSourceNode(context, {mediaElement:new Audio()}));
      throws('TypeError', () => new MediaElementAudioSourceNode(new OfflineAudioContext(1,64,44100), {mediaElement:new Audio()}));
      for (const options of [undefined,null,{}, {mediaElement:{}}, {mediaElement:document.createElement('div')}]) {
        throws('TypeError', () => new MediaElementAudioSourceNode(context, options));
      }
      return 'ok';
    })()"#).unwrap(), "ok");
}

#[test]
fn media_element_source_ownership_is_shared_between_wrapper_worlds() {
    let mut vm = new_storage_html_test_vm("https://source-worlds.test/");
    vm.eval(r#"
      const audio = document.createElement('audio'); audio.id = 'main-source'; audio.mainOnly = true;
      document.body.append(audio); globalThis.mainSource = new AudioContext().createMediaElementSource(audio);
    "#).unwrap();
    let isolated = vm
        .create_isolated_world("media-source-owner", false)
        .unwrap();
    assert_eq!(
        vm.eval_in_isolated_context(
            isolated,
            r#"(() => {
      const audio = document.querySelector('#main-source');
      if (audio.mainOnly !== undefined) throw Error('wrappers must be isolated');
      try { new AudioContext().createMediaElementSource(audio); } catch(e) { return e.name; }
      return 'allowed';
    })()"#
        )
        .unwrap(),
        "InvalidStateError"
    );
    vm.eval_in_isolated_context(isolated, r#"
      const video = document.createElement('video'); video.id = 'isolated-source';
      document.body.append(video); globalThis.isolatedSource = new AudioContext().createMediaElementSource(video);
    "#).unwrap();
    assert_eq!(vm.eval(r#"(() => {
      try { new AudioContext().createMediaElementSource(document.querySelector('#isolated-source')); }
      catch(e) { return e.name; }
      return 'allowed';
    })()"#).unwrap(), "InvalidStateError");
    vm.destroy_isolated_world_context(isolated);
}

#[test]
fn media_session_playback_state_is_shared_between_wrapper_worlds() {
    let mut vm = new_storage_html_test_vm("https://media-session-worlds.test/");
    vm.eval(
        "navigator.mediaSession.marker = 'main'; navigator.mediaSession.playbackState = 'playing'",
    )
    .unwrap();
    let isolated = vm
        .create_isolated_world("media-session-state", false)
        .unwrap();
    assert_eq!(vm.eval_in_isolated_context(isolated,
      "JSON.stringify([navigator.mediaSession.playbackState,navigator.mediaSession.marker===undefined,navigator.mediaSession instanceof MediaSession])").unwrap(),
      "[\"playing\",true,true]");
    vm.eval_in_isolated_context(isolated, "navigator.mediaSession.playbackState = 'paused'")
        .unwrap();
    assert_eq!(
        vm.eval("navigator.mediaSession.playbackState").unwrap(),
        "paused"
    );
    vm.destroy_isolated_world_context(isolated);
    assert_eq!(
        vm.eval("navigator.mediaSession.playbackState").unwrap(),
        "paused"
    );
}

#[test]
fn media_session_releases_isolated_worlds_and_unregistered_handlers() {
    for register_handler in [false, true] {
        let mut vm = new_storage_html_test_vm("https://media-session-gc.test/");
        let isolated = vm.create_isolated_world("media-session-gc", false).unwrap();
        vm.eval_in_isolated_context(
            isolated,
            if register_handler {
                "navigator.mediaSession.setActionHandler('play', () => {}); 'registered'"
            } else {
                "navigator.mediaSession.playbackState"
            },
        )
        .unwrap();
        let context = vm
            .renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(
                    scope,
                    &vm.page_isolated_world_contexts
                        .context(isolated)
                        .unwrap()
                        .context,
                );
                v8::Weak::new(scope, context)
            });
        vm.destroy_isolated_world_context(isolated);
        for _ in 0..2 {
            vm.renderer_document_isolate
                .with_renderer_document_isolate_mut(|isolate| isolate.low_memory_notification());
        }
        assert_eq!(
            !context.is_empty(),
            register_handler,
            "only an explicitly registered callback should retain its realm"
        );
        if register_handler {
            vm.eval("navigator.mediaSession.setActionHandler('play', null)")
                .unwrap();
            for _ in 0..2 {
                vm.renderer_document_isolate
                    .with_renderer_document_isolate_mut(|isolate| {
                        isolate.low_memory_notification()
                    });
            }
            assert!(
                context.is_empty(),
                "unregistering from another world must release the callback's realm"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn media_session_state_belongs_to_each_window_and_resets_on_navigation() {
    let mut vm = new_storage_html_test_vm("https://media-session-navigation.test/");
    vm.eval("navigator.mediaSession.playbackState = 'playing'")
        .unwrap();
    let isolated = crate::script_vm::tests::window_node_worlds::child_node_world(
        &mut vm,
        "media-session-child",
    )
    .await;
    assert_eq!(
        vm.eval_in_isolated_context(isolated, "navigator.mediaSession.playbackState")
            .unwrap(),
        "none"
    );
    vm.eval_in_isolated_context(isolated, "navigator.mediaSession.playbackState = 'paused'")
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([navigator.mediaSession.playbackState,frame.contentWindow.navigator.mediaSession.playbackState])")
            .unwrap(),
        "[\"playing\",\"paused\"]"
    );
    vm.eval("globalThis.oldSession=frame.contentWindow.navigator.mediaSession; frame.srcdoc='<!doctype html><body>next</body>'")
        .unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "MediaSession navigation").await;
    assert_eq!(
        vm.eval("JSON.stringify([navigator.mediaSession.playbackState,frame.contentWindow.navigator.mediaSession.playbackState,oldSession.playbackState,oldSession===frame.contentWindow.navigator.mediaSession])")
            .unwrap(),
        "[\"playing\",\"none\",\"paused\",false]"
    );
}
