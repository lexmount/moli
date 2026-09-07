use super::*;

#[test]
fn webrtc_ice_event_constructor_preserves_candidates_and_converts_nullable_urls() {
    let mut vm = new_storage_test_vm("http://webrtc-ice-event.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const rejects = fn => {
          try { fn(); return false; } catch (error) { return error instanceof TypeError; }
        };
        const candidate = new RTCIceCandidate({sdpMid: 'audio'});
        const rawType = 'ice\ud800\0\udc00';
        const event = new RTCPeerConnectionIceEvent(rawType, {candidate, url: 'turn:\ud800'});
        const checks = [RTCPeerConnectionIceEvent.length === 1,
          rejects(() => new RTCPeerConnectionIceEvent()),
          rejects(() => RTCPeerConnectionIceEvent('ice')),
          rejects(() => new RTCPeerConnectionIceEvent(Symbol())),
          event.type === rawType, event.candidate === candidate, event.url === 'turn:\ufffd',
          new RTCPeerConnectionIceEvent(undefined).type === 'undefined',
          new RTCPeerConnectionIceEvent(null).type === 'null'];
        for (const init of [undefined, null, {}, {candidate: null, url: null},
          {candidate: undefined, url: undefined}]) {
          const empty = new RTCPeerConnectionIceEvent('ice', init);
          checks.push(empty.candidate === null, empty.url === null,
            !empty.bubbles, !empty.cancelable, !empty.composed);
        }
        for (const url of ['', false, 4, 5n, ['not a URL']]) {
          checks.push(new RTCPeerConnectionIceEvent('ice', {url}).url === String(url));
        }
        for (const init of [true, 1, 'init', Symbol(), 1n, {url: Symbol()},
          {candidate: {}}, {candidate: false}, {candidate: 1},
          {candidate: Object.create(candidate)}, {candidate: new Proxy(candidate, {})}]) {
          checks.push(rejects(() => new RTCPeerConnectionIceEvent('ice', init)));
        }
        Object.setPrototypeOf(candidate, null);
        checks.push(new RTCPeerConnectionIceEvent('ice', {candidate}).candidate === candidate);
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#,
        )
        .expect(
            "ICE events should preserve candidate identity and the DOMString/USVString distinction",
        );
    assert_eq!(result, "true");
}

#[test]
fn webrtc_data_channel_event_requires_a_genuine_channel_and_checks_arity_first() {
    let mut vm = new_storage_test_vm("https://webrtc-data-channel-event.test/");
    let result = vm.eval(r#"
      (() => {
        const rejects = fn => {
          try { fn(); return false; } catch (error) { return error instanceof TypeError; }
        };
        const peer = new RTCPeerConnection();
        const channel = peer.createDataChannel('test');
        const event = new RTCDataChannelEvent('channel', {channel});
        let converted = false;
        const checks = [RTCDataChannelEvent.length === 2, event.channel === channel,
          rejects(() => RTCDataChannelEvent('channel', {channel})),
          rejects(() => new RTCDataChannelEvent()),
          rejects(() => new RTCDataChannelEvent({toString() { converted = true; return 'channel'; }})),
          !converted];
        for (const init of [undefined, null, {}, {channel: null}, {channel: undefined},
          false, 1, 'init', Symbol(), 1n, {channel: {}},
          {channel: Object.create(RTCDataChannel.prototype)},
          {channel: Object.create(channel)}, {channel: new Proxy(channel, {})},
          {channel: new RTCIceCandidate({sdpMid: 'audio'})}]) {
          checks.push(rejects(() => new RTCDataChannelEvent('channel', init)));
        }
        Object.defineProperty(RTCDataChannel, Symbol.hasInstance, {
          configurable: true, value() { throw new Error('must not use instanceof'); }
        });
        try {
          checks.push(rejects(() => new RTCDataChannelEvent('channel', {channel: {}})),
            new RTCDataChannelEvent('channel', {channel}).channel === channel);
        } finally { delete RTCDataChannel[Symbol.hasInstance]; }
        Object.setPrototypeOf(channel, null);
        checks.push(new RTCDataChannelEvent('channel', {channel}).channel === channel);
        peer.close();
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#).expect("data channel events should validate arity before conversion and use native interface branding");
    assert_eq!(result, "true");
}

#[test]
fn webrtc_event_dictionary_conversion_reads_inherited_members_first_and_preserves_exceptions() {
    let mut vm = new_storage_test_vm("https://webrtc-event-conversion.test/");
    let result = vm.eval(r#"
      (() => {
        const candidate = new RTCIceCandidate({sdpMid: 'audio'});
        const peer = new RTCPeerConnection();
        const channel = peer.createDataChannel('test');
        const checks = [];
        for (const [Ctor, ownMembers] of [
          [RTCPeerConnectionIceEvent, ['candidate', 'url']], [RTCDataChannelEvent, ['channel']]
        ]) {
          const names = ['bubbles', 'cancelable', 'composed', ...ownMembers];
          const reads = [];
          const dictionary = Object.create({bubbles: [], cancelable: 1, composed: 'yes',
            candidate, channel, url: 'url'});
          const event = new Ctor({toString() { reads.push('type'); return 'converted'; }},
            new Proxy(dictionary, {get(object, name) { reads.push(name); return object[name]; }}));
          checks.push(reads.join(',') === ['type', ...names].join(','),
            event.type === 'converted', event.bubbles, event.cancelable, event.composed);
          const sentinel = new RangeError('original getter failure');
          for (const name of names) {
            const bad = {candidate, channel};
            Object.defineProperty(bad, name, {get() { throw sentinel; }});
            try { new Ctor('event', bad); checks.push(false); }
            catch (error) { checks.push(error === sentinel); }
          }
          try { new Ctor({toString() { throw sentinel; }}, {candidate, channel}); checks.push(false); }
          catch (error) { checks.push(error === sentinel); }
        }
        const sentinel = new RangeError('URL conversion');
        try {
          new RTCPeerConnectionIceEvent('ice', {url: {toString() { throw sentinel; }}});
          checks.push(false);
        } catch (error) { checks.push(error === sentinel); }
        let urlRead = false;
        try {
          new RTCPeerConnectionIceEvent('ice', {candidate: {}, get url() { urlRead = true; return ''; }});
          checks.push(false);
        } catch (error) { checks.push(error instanceof TypeError, !urlRead); }
        Object.defineProperty(Object.prototype, 'bubbles', {
          configurable: true, get() { throw sentinel; }
        });
        try {
          checks.push(new RTCPeerConnectionIceEvent('ice', null).candidate === null);
          try { new RTCDataChannelEvent('channel', null); checks.push(false); }
          catch (error) { checks.push(error instanceof TypeError); }
        } finally { delete Object.prototype.bubbles; }
        peer.close();
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#).expect("WebRTC event dictionaries should convert EventInit first without swallowing getters or exceptions");
    assert_eq!(result, "true");
}

#[test]
fn webrtc_event_members_are_readonly_branded_and_survive_dispatch_and_reinitialization() {
    let mut vm = new_storage_test_vm("https://webrtc-event-dispatch.test/");
    let result = vm.eval(r#"
      (() => {
        const candidate = new RTCIceCandidate({sdpMid: 'audio'});
        const peer = new RTCPeerConnection();
        const channel = peer.createDataChannel('test');
        const checks = [];
        const rejects = fn => {
          try { fn(); return false; } catch (error) { return error instanceof TypeError; }
        };
        for (const [Ctor, members] of [
          [RTCPeerConnectionIceEvent, {candidate, url: 'turn:server'}], [RTCDataChannelEvent, {channel}]
        ]) {
          const event = new Ctor('test', {...members, bubbles: true, cancelable: true, composed: true});
          checks.push(event instanceof Ctor, event instanceof Event,
            Object.getPrototypeOf(Ctor.prototype) === Event.prototype,
            Object.prototype.toString.call(event) === '[object ' + Ctor.name + ']',
            event.isTrusted === false, typeof event.timeStamp === 'number');
          for (const [name, original] of Object.entries(members)) {
            const descriptor = Object.getOwnPropertyDescriptor(Ctor.prototype, name);
            if (!descriptor) return 'missing:' + name;
            checks.push(!Object.hasOwn(event, name), descriptor.enumerable, descriptor.configurable,
              descriptor.get.name === 'get ' + name, descriptor.get.length === 0, !descriptor.set);
            event[name] = 'changed';
            checks.push(event[name] === original, rejects(() => { 'use strict'; event[name] = null; }));
            for (const fake of [undefined, null, {}, Ctor.prototype, new Event('test'),
              Object.create(event), new Proxy(event, {})]) {
              checks.push(rejects(() => descriptor.get.call(fake)));
            }
          }
          const target = new EventTarget();
          let delivered = false;
          target.addEventListener('test', received => {
            delivered = received === event && received.target === target && received.currentTarget === target;
            received.preventDefault();
          });
          checks.push(target.dispatchEvent(event) === false, delivered, event.defaultPrevented,
            event.isTrusted === false, event.currentTarget === null);
          event.initEvent('again', false, false);
          checks.push(event.type === 'again', !event.defaultPrevented, !event.bubbles, !event.cancelable);
          for (const [name, original] of Object.entries(members)) checks.push(event[name] === original);
        }
        peer.close();
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#).expect("WebRTC event attributes should retain native identity through normal Event operations");
    assert_eq!(result, "true");
}

#[test]
fn webrtc_event_constructors_support_subclasses_foreign_members_and_new_target_realms() {
    let mut vm = new_parsed_test_vm(
        "https://webrtc-event-realms.test/",
        "<!doctype html><iframe></iframe>",
    );
    let result = vm.eval(r#"
      (() => {
        const child = document.querySelector('iframe').contentWindow;
        const foreignPeer = new child.RTCPeerConnection();
        const candidate = new child.RTCIceCandidate({sdpMid: 'foreign'});
        const channel = foreignPeer.createDataChannel('foreign');
        const checks = [];
        for (const [name, member, value] of [
          ['RTCPeerConnectionIceEvent', 'candidate', candidate], ['RTCDataChannelEvent', 'channel', channel]
        ]) {
          const Ctor = window[name];
          const init = {[member]: value};
          class Derived extends Ctor {}
          const derived = new Derived('event', init);
          const foreign = new child[name]('event', init);
          const newTarget = child.Function('');
          newTarget.prototype = 0;
          let prototypeReads = 0;
          const target = new Proxy(newTarget, {get(object, name) {
            if (name === 'prototype') prototypeReads++;
            return Reflect.get(object, name);
          }});
          const fallback = Reflect.construct(Ctor, ['event', init], target);
          checks.push(derived instanceof Derived, derived instanceof Ctor,
            derived[member] === value, foreign[member] === value,
            Object.getOwnPropertyDescriptor(Ctor.prototype, member).get.call(foreign) === value,
            Object.getPrototypeOf(fallback) === child[name].prototype, fallback[member] === value,
            prototypeReads === 1);
        }
        foreignPeer.close();
        return checks.every(Boolean) || JSON.stringify(checks);
      })()
    "#).expect("WebRTC event constructors should preserve genuine cross-realm members and NewTarget semantics");
    assert_eq!(result, "true");
}
