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
