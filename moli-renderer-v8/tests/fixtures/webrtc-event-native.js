(() => {
  const assert = (value, message) => { if (!value) throw Error(message); };
  const other = document.querySelector('iframe').contentWindow;
  for (const w of [window, other]) {
    const C = w.RTCTrackEvent;
    const stream = new w.MediaStream(), second = new MediaStream();
    const init = {receiver: rtpReceiver, track: audio, transceiver: rtpTransceiver, streams: [stream, second, stream]};
    const event = new C('track\ud800', {...init, bubbles: true, cancelable: true, composed: true});
    assert(event instanceof C && event.type === 'track\ud800' && event.bubbles && event.cancelable && event.composed && !event.isTrusted, 'native event header');
    assert(event.receiver === rtpReceiver && event.track === audio && event.transceiver === rtpTransceiver, 'required object identities');
    assert(event.streams instanceof w.Array && Object.isFrozen(event.streams) && event.streams === event.streams && event.streams.length === 3 && event.streams[0] === stream && event.streams[1] === second && event.streams[2] === stream, 'frozen snapshot with duplicates');
    init.streams.length = 0;
    assert(event.streams.length === 3 && !Reflect.set(event.streams, 0, second), 'snapshot is independent');
    for (const property of ['receiver','track','streams','transceiver']) {
      assert(!Object.hasOwn(event, property) && !Reflect.set(event, property, null), 'readonly prototype attribute');
      const get = Object.getOwnPropertyDescriptor((w === window ? other : window).RTCTrackEvent.prototype, property).get;
      assert(get.call(event) === event[property], 'borrowed cross realm getter');
    }
    for (const streams of [undefined, [], new Set([stream]), {*[Symbol.iterator]() { yield stream; yield stream; }}]) {
      const result = new C('track', {...init, streams});
      assert(Object.isFrozen(result.streams), 'every stream result frozen');
      if (streams === undefined) assert(result.streams.length === 0, 'default sequence');
    }
    const reads = [];
    const source = {get bubbles() { reads.push('bubbles'); return false; }, get cancelable() { reads.push('cancelable'); return false; }, get composed() { reads.push('composed'); return false; },
      get receiver() { reads.push('receiver'); return rtpReceiver; },
      get streams() { reads.push('streams'); return {get [Symbol.iterator]() { reads.push('iterator'); return function*() { reads.push('value'); yield stream; }; }}; },
      get track() { reads.push('track'); return audio; }, get transceiver() { reads.push('transceiver'); return rtpTransceiver; }};
    const prototype = Object.create(C.prototype), target = new Proxy(function Target() {}, {get(object, key) { if (key === 'prototype') { reads.push('prototype'); return prototype; } return object[key]; }});
    const constructed = Reflect.construct(C, [{toString() { reads.push('type'); return 'track'; }}, source], target);
    assert(reads.join() === 'type,bubbles,cancelable,composed,receiver,streams,iterator,value,track,transceiver,prototype' && Object.getPrototypeOf(constructed) === prototype, 'full conversion order before prototype');
    let error, marker = {}, count = 0;
    try { new C('track', {...init, streams: [new Proxy(stream, {})], get track() { count++; return audio; }}); } catch (caught) { error = caught; }
    assert(error instanceof w.TypeError && count === 0, 'stream element native brand precedes track read');
    try { new C('track', {...init, streams: {get [Symbol.iterator]() { throw marker; }}}); } catch (caught) { error = caught; }
    assert(error === marker, 'iterator property exception identity');
    for (const [key, real] of [['receiver',rtpReceiver],['track',audio],['transceiver',rtpTransceiver]]) {
      let traps = 0;
      const revoked = Proxy.revocable(real,{}); revoked.revoke();
      for (const bad of [undefined,null,{},Object.create(real),new Proxy(real,{get() {traps++;throw Error('trap');}}),revoked.proxy]) {
        error = undefined;
        try { new C('track', {...init,[key]:bad}); } catch (caught) { error = caught; }
        assert(error instanceof w.TypeError, 'required interface brand');
      }
      assert(traps === 0, 'no author interface Proxy traps');
    }
    const proxyEvent = new C('track', {receiver:nativeReceiver,track:nativeaudio,transceiver:nativeTransceiver,streams:[stream]});
    assert(proxyEvent.receiver === nativeReceiver && proxyEvent.track === nativeaudio && proxyEvent.transceiver === nativeTransceiver, 'registered native proxy argument identities');
    for (const key of ['receiver','track','transceiver']) Object.defineProperty(proxyEvent[key], 'constructor', {configurable:true,get() {throw Error('author constructor read');}});
    assert(new C('track', {receiver:proxyEvent.receiver,track:proxyEvent.track,transceiver:proxyEvent.transceiver}).track === nativeaudio, 'brand uses private identity');
    class Derived extends C {}
    assert(new Derived('track', init) instanceof Derived, 'track subclass');
    const targetEvent = new w.EventTarget(); let observed = 0;
    targetEvent.addEventListener(event.type, e => { assert(e === event && e.streams[0] === stream, 'dispatch payload identity'); observed++; e.preventDefault(); });
    assert(targetEvent.dispatchEvent(event) === false && observed === 1, 'native track EventTarget dispatch');
    event.initEvent('reset', false, false);
    assert(event.streams.length === 3 && event.receiver === rtpReceiver && event.track === audio && event.composed, 'legacy init preserves native payload');
    const array = new C('track', {...init, streams: [stream]}).streams;
    const savedArray = w.Array, savedFreeze = w.Object.freeze;
    try {
      w.Array = () => {throw Error('author Array');}; w.Object.freeze = () => {throw Error('author freeze');};
      assert(new C('track', {...init,streams:[stream]}).streams[0] === stream && Object.getPrototypeOf(array) === savedArray.prototype, 'intrinsic frozen allocation');
    } finally { w.Array = savedArray; w.Object.freeze = savedFreeze; }
  }
  globalThis.trackEvent = new RTCTrackEvent('track', {receiver:rtpReceiver,track:audio,transceiver:rtpTransceiver,streams:[new MediaStream()]});
  globalThis.toneEvent = new RTCDTMFToneChangeEvent('tone', {tone:'\ud800'});
  return true;
})()
