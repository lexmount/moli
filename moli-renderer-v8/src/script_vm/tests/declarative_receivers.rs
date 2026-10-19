use super::*;

fn receiver_test_vm() -> crate::runtime::PageVmTaskExecutorTestHarness {
    let mut vm = new_storage_page_task_executor_test_vm("https://declarative-receivers.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("declarative_receivers.js")).unwrap();
    vm
}

fn finish_receiver_checks(vm: &mut crate::runtime::PageVmTaskExecutorTestHarness) {
    vm.eval("receiverChecks.finish()").unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("receiverChecks.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(receiverChecks.failures)").unwrap(),
        "[]"
    );
}

#[test]
fn webrtc_declarations_validate_foreign_receivers_before_converting_arguments() {
    let mut vm = receiver_test_vm();
    vm.eval(r#"(() => {
      const {child, assert, check, getters} = receiverChecks;
      for (const realm of [window, child]) {
        const other = realm === window ? child : window;
        const pc = new other.RTCPeerConnection();
        const channel = pc.createDataChannel('foreign');
        getters(realm, 'RTCPeerConnection', pc, ['signalingState', 'iceGatheringState', 'iceConnectionState',
          'connectionState', 'localDescription', 'currentLocalDescription', 'pendingLocalDescription']);
        for (const name of ['getConfiguration', 'setConfiguration', 'createDataChannel', 'close']) {
          check(realm, pc, realm.RTCPeerConnection.prototype[name]);
        }
        for (const name of ['createOffer', 'setLocalDescription']) {
          check(realm, pc, realm.RTCPeerConnection.prototype[name], true);
        }
        assert(realm.RTCPeerConnection.prototype.createDataChannel.call(pc, 'borrowed').label === 'borrowed', 'foreign peer method');
        getters(realm, 'RTCDataChannel', channel, ['label', 'ordered', 'maxPacketLifeTime', 'maxRetransmits',
          'protocol', 'negotiated', 'id', 'readyState', 'bufferedAmount', 'binaryType']);
        for (const name of ['send', 'close']) check(realm, channel, realm.RTCDataChannel.prototype[name]);
        const candidate = new other.RTCIceCandidate({candidate: 'candidate:1 1 UDP 1 127.0.0.1 9 typ host', sdpMid: '0'});
        getters(realm, 'RTCIceCandidate', candidate, ['candidate', 'sdpMid', 'sdpMLineIndex', 'foundation',
          'component', 'priority', 'address', 'protocol', 'port', 'type', 'tcpType', 'relatedAddress',
          'relatedPort', 'usernameFragment', 'relayProtocol', 'url']);
        check(realm, candidate, realm.RTCIceCandidate.prototype.toJSON);
        assert(realm.RTCIceCandidate.prototype.toJSON.call(candidate).candidate === candidate.candidate, 'foreign ICE method');
        const description = new other.RTCSessionDescription({type: 'offer', sdp: 'foreign'});
        getters(realm, 'RTCSessionDescription', description, ['type', 'sdp']);
        check(realm, description, realm.RTCSessionDescription.prototype.toJSON);
        assert(realm.RTCSessionDescription.prototype.toJSON.call(description).sdp === 'foreign', 'foreign description method');
        realm.RTCPeerConnection.prototype.close.call(pc);
      }
    })()"#).unwrap();
    finish_receiver_checks(&mut vm);
}

#[tokio::test]
async fn navigator_declarations_preserve_receiver_checks_and_promise_identity_across_realms() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://declarative-receivers.test/",
        &loader,
    );
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("declarative_receivers.js")).unwrap();
    vm.eval(r#"(async () => {
      const {child, assert, check, getters} = receiverChecks;
      for (const realm of [window, child]) {
        const other = realm === window ? child : window;
        const nav = other.navigator;
        for (const name of ['getGamepads', 'javaEnabled', 'sendBeacon', 'vibrate', 'getAutoplayPolicy']) {
          check(realm, nav, realm.Navigator.prototype[name]);
        }
        check(realm, nav, realm.Navigator.prototype.getBattery, true);
        getters(realm, 'Navigator', nav, ['cookieEnabled']);
        assert(realm.Navigator.prototype.javaEnabled.call(nav) === false, 'foreign Navigator method');
        const localConnection = realm.navigator.connection;
        const saveData = Object.getOwnPropertyDescriptor(localConnection, 'saveData').get;
        check(realm, nav.connection, saveData);
        assert(saveData.call(nav.connection) === nav.connection.saveData, 'foreign connection getter');
        for (const name of ['addEventListener', 'removeEventListener']) check(realm, nav.connection, localConnection[name]);
        const battery = await nav.getBattery();
        const localBattery = await realm.navigator.getBattery();
        for (const name of ['addEventListener', 'removeEventListener', 'dispatchEvent']) check(realm, battery, localBattery[name]);
        check(realm, nav.permissions, realm.Permissions.prototype.query, true);
        check(realm, nav.userAgentData, realm.NavigatorUAData.prototype.toJSON);
        check(realm, nav.userAgentData, realm.NavigatorUAData.prototype.getHighEntropyValues, true);
        assert(realm.NavigatorUAData.prototype.toJSON.call(nav.userAgentData).platform === nav.userAgentData.platform, 'foreign UAData method');
        for (const name of ['getCurrentPosition', 'watchPosition', 'clearWatch']) check(realm, nav.geolocation, realm.Geolocation.prototype[name]);
        realm.Geolocation.prototype.clearWatch.call(nav.geolocation, 0);
        for (const name of ['decodingInfo', 'encodingInfo']) check(realm, nav.mediaCapabilities, realm.MediaCapabilities.prototype[name], true);
        for (const name of ['read', 'readText', 'write', 'writeText']) check(realm, nav.clipboard, realm.Clipboard.prototype[name], true);
        check(realm, nav.mediaDevices, realm.MediaDevices.prototype.getUserMedia, true);
        const item = new other.ClipboardItem({'text/plain': new other.Blob(['foreign'])});
        getters(realm, 'ClipboardItem', item, ['types', 'presentationStyle']);
        check(realm, item, realm.ClipboardItem.prototype.getType, true);
        const originalPromise = other.ClipboardItem.prototype.getType.call(item, 'text/plain');
        assert(realm.ClipboardItem.prototype.getType.call(item, 'text/plain') === originalPromise, 'getType preserves stored Promise identity');
        assert(realm.ClipboardItem.supports.call({}, 'text/plain'), 'static method has no receiver guard');
        const bucket = await nav.storageBuckets.open('receiver-bucket');
        getters(realm, 'StorageBucket', bucket, ['name', 'caches']);
        const factoryGetter = Object.getOwnPropertyDescriptor(realm.StorageBucket.prototype, 'indexedDB').get;
        check(realm, bucket, factoryGetter);
        assert(typeof factoryGetter.call(bucket).open === 'function', 'foreign bucket factory getter');
      }
      return 'ok';
    })().then(result => {globalThis.receiverSetup = result;}, error => {globalThis.receiverSetup = String(error);})"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(typeof receiverSetup !== 'undefined')",
        "true",
        "receiver setup should settle through storage tasks",
    )
    .await;
    assert_eq!(vm.eval("receiverSetup").unwrap(), "ok");
    finish_receiver_checks(&mut vm);
}

#[test]
fn canvas_declarations_check_setters_before_conversion_and_keep_foreign_backing_state() {
    let mut vm = receiver_test_vm();
    vm.eval(r#"(async () => {
      const {child, assert, check, getters} = receiverChecks;
      for (const realm of [window, child]) {
        const other = realm === window ? child : window;
        const canvas = new other.OffscreenCanvas(2, 3);
        getters(realm, 'OffscreenCanvas', canvas, ['width', 'height']);
        for (const name of ['width', 'height']) {
          const setter = Object.getOwnPropertyDescriptor(realm.OffscreenCanvas.prototype, name).set;
          check(realm, canvas, setter);
          setter.call(canvas, 4);
          assert(canvas[name] === 4, 'foreign canvas setter updates receiver');
        }
        for (const name of ['getContext', 'convertToBlob']) check(realm, canvas, realm.OffscreenCanvas.prototype[name], name === 'convertToBlob');
        assert(realm.OffscreenCanvas.prototype.getContext.call(canvas, '2d') === canvas.getContext('2d'), 'foreign canvas context identity');
        const bitmap = await other.createImageBitmap(canvas);
        getters(realm, 'ImageBitmap', bitmap, ['width', 'height']);
        check(realm, bitmap, realm.ImageBitmap.prototype.close);
        realm.ImageBitmap.prototype.close.call(bitmap);
        assert(bitmap.width === 0 && bitmap.height === 0, 'foreign bitmap close updates receiver');
        const data = new other.ImageData(2, 3);
        getters(realm, 'ImageData', data, ['width', 'height', 'colorSpace', 'pixelFormat', 'data']);
        let error;
        const invalidSource = realm.createImageBitmap(new Proxy(canvas, {}));
        assert(Object.getPrototypeOf(invalidSource) === realm.Promise.prototype, 'image source rejection Promise realm');
        try {await invalidSource;} catch (caught) {error = caught;}
        assert(Object.getPrototypeOf(error) === realm.TypeError.prototype, 'image source parameter keeps native interface check');
      }
      return 'ok';
    })().then(result => {globalThis.receiverSetup = result;}, error => {globalThis.receiverSetup = String(error);})"#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("receiverSetup").unwrap(),
        "ok"
    );
    finish_receiver_checks(&mut vm);
}

#[test]
fn shared_value_and_event_declarations_keep_native_brands_across_realms() {
    let mut vm = receiver_test_vm();
    vm.eval(r#"(() => {
      const {child, assert, check, getters} = receiverChecks;
      for (const realm of [window, child]) {
        const other = realm === window ? child : window;
        const channel = new other.BroadcastChannel('receiver-check');
        getters(realm, 'BroadcastChannel', channel, ['name', 'onmessage', 'onmessageerror']);
        for (const name of ['postMessage', 'close']) check(realm, channel, realm.BroadcastChannel.prototype[name]);
        const localChannel = new realm.BroadcastChannel('receiver-check');
        for (const name of ['addEventListener', 'removeEventListener', 'dispatchEvent']) check(realm, channel, localChannel[name]);
        for (const name of ['onmessage', 'onmessageerror']) {
          const setter = Object.getOwnPropertyDescriptor(realm.BroadcastChannel.prototype, name).set;
          check(realm, channel, setter);
          const handler = () => {};
          setter.call(channel, handler);
          assert(channel[name] === handler, 'foreign channel handler identity');
        }
        let events = 0;
        const listener = () => events++;
        localChannel.addEventListener.call(channel, 'probe', listener);
        localChannel.dispatchEvent.call(channel, new other.Event('probe'));
        localChannel.removeEventListener.call(channel, 'probe', listener);
        assert(events === 1, 'foreign channel listener state');
        channel.close(); localChannel.close();
        const notification = new other.Notification('foreign');
        getters(realm, 'Notification', notification, ['title', 'data', 'tag', 'dir', 'lang', 'body', 'icon',
          'image', 'badge', 'vibrate', 'timestamp', 'renotify', 'silent', 'requireInteraction', 'actions']);
        getters(realm, 'Notification', notification, ['onclick', 'onerror', 'onshow', 'onclose']);
        for (const name of ['onclick', 'onerror', 'onshow', 'onclose']) {
          const setter = Object.getOwnPropertyDescriptor(realm.Notification.prototype, name).set;
          check(realm, notification, setter);
          const handler = () => {};
          setter.call(notification, handler);
          assert(notification[name] === handler, 'foreign notification handler identity');
        }
        const localNotification = new realm.Notification('local');
        for (const name of ['addEventListener', 'removeEventListener', 'dispatchEvent', 'close']) check(realm, notification, localNotification[name]);
        const encoder = new other.TextEncoder();
        const decoder = new other.TextDecoder();
        getters(realm, 'TextEncoder', encoder, ['encoding']);
        getters(realm, 'TextDecoder', decoder, ['encoding', 'fatal', 'ignoreBOM']);
        for (const name of ['encode', 'encodeInto']) check(realm, encoder, realm.TextEncoder.prototype[name]);
        check(realm, decoder, realm.TextDecoder.prototype.decode);
        const encoded = realm.TextEncoder.prototype.encode.call(encoder, 'foreign');
        assert(realm.TextDecoder.prototype.decode.call(decoder, encoded) === 'foreign', 'foreign codec backing state');
        const point = new other.DOMPoint(1, 2, 3, 4);
        getters(realm, 'DOMPoint', point, ['x', 'y', 'z', 'w']);
        for (const name of ['x', 'y', 'z', 'w']) check(realm, point, Object.getOwnPropertyDescriptor(realm.DOMPoint.prototype, name).set);
        check(realm, point, realm.DOMPoint.prototype.toJSON);
        const setter = Object.getOwnPropertyDescriptor(realm.DOMPoint.prototype, 'x').set;
        Object.setPrototypeOf(point, null);
        setter.call(point, 9);
        assert(realm.DOMPoint.prototype.toJSON.call(point).x === 9, 'native brand survives prototype mutation');
        class Encoder extends other.TextEncoder {}
        assert(realm.TextEncoder.prototype.encode.call(new Encoder(), 'subclass').length === 8, 'native subclass receiver');
        const keyword = new other.CSSKeywordValue('auto');
        check(realm, keyword, realm.CSSStyleValue.prototype.toString);
        assert(realm.CSSStyleValue.prototype.toString.call(keyword) === 'auto', 'foreign Typed OM value');
      }
    })()"#).unwrap();
    finish_receiver_checks(&mut vm);
}
