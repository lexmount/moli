use super::*;

fn rtc_platform_vm(
    loader: &ResourceRequestClient,
) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://rtc-platform.test/", loader);
    vm.eval("function check(value, message) { if (!value) throw new Error(message || 'check failed'); }")
        .unwrap();
    vm
}

#[tokio::test(flavor = "current_thread")]
async fn peer_configuration_snapshots_and_channels_use_native_tasks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = rtc_platform_vm(&loader);

    assert_eq!(vm.eval(r#"(() => {
            const input = {iceServers: [{urls: ['stun:example.test:3478']}], iceCandidatePoolSize: 2};
            class Peer extends RTCPeerConnection {}
            const peer = new Peer(input);
            check(peer instanceof Peer && peer instanceof EventTarget);
            input.iceServers[0].urls[0] = 'stun:changed.test';
            const config = peer.getConfiguration();
            check(config.iceServers[0].urls[0] === 'stun:example.test:3478');
            config.iceServers[0].urls.push('stun:other.test');
            check(peer.getConfiguration().iceServers[0].urls.length === 1);
            peer.setConfiguration({iceServers: [{urls: 'stun:new.test'}], iceCandidatePoolSize: 2});
            check(peer.getConfiguration().iceServers[0].urls[0] === 'stun:new.test');
            const channel = peer.createDataChannel('label\ud800', {ordered: false, maxRetransmits: 3, protocol: 'test'});
            check(channel instanceof RTCDataChannel && channel instanceof EventTarget);
            check(channel.label === 'label\ufffd' && channel.ordered === false && channel.maxRetransmits === 3);
            check(channel.protocol === 'test' && channel.maxPacketLifeTime === null && channel.id === null);
            check(channel.binaryType === 'arraybuffer' && channel.readyState === 'connecting' && channel.bufferedAmount === 0);
            globalThis.nativePeer = peer;
            globalThis.nativeChannel = channel;
            channel.close(); check(channel.readyState === 'closing');
            return true;
        })()"#).unwrap(), "true");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "nativeChannel.readyState",
        "closed",
        "the native channel close task must finish before checking its final state",
    )
    .await;
    assert_eq!(vm.eval(r#"(() => {
        nativePeer.close();
        check(nativePeer.signalingState === 'closed' && nativePeer.iceConnectionState === 'closed' && nativePeer.connectionState === 'closed');
        return true;
    })()"#).unwrap(), "true");
}

#[tokio::test(flavor = "current_thread")]
async fn peer_and_channel_conversions_validate_before_state_and_preserve_exceptions() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = rtc_platform_vm(&loader);

    assert_eq!(vm.eval(r#"(() => {
            function fails(name, body) {
                let caught; try {body();} catch (error) {caught = error;}
                check(caught && caught.name === name, 'expected ' + name + ', got ' + caught);
            }
            for (const url of ['https://example.test', 'stun:example.test/path', 'stun:example.test#x', 'stun:example.test?transport=udp']) {
                fails('SyntaxError', () => new RTCPeerConnection({iceServers: [{urls: url}]}));
            }
            fails('InvalidAccessError', () => new RTCPeerConnection({iceServers: [{urls: 'turn:example.test'}]}));
            fails('TypeError', () => new RTCPeerConnection({iceCandidatePoolSize: 256}));
            const peer = new RTCPeerConnection();
            fails('TypeError', () => peer.createDataChannel('c', {maxRetransmits: 1, maxPacketLifeTime: 1}));
            fails('TypeError', () => peer.createDataChannel('c', {negotiated: true}));
            fails('TypeError', () => peer.createDataChannel('c', {id: 65535}));
            fails('TypeError', () => RTCPeerConnection.prototype.close.call(Object.create(RTCPeerConnection.prototype)));
            fails('TypeError', () => RTCDataChannel.prototype.close.call(Object.create(RTCDataChannel.prototype)));
            const sentinel = {}, order = [];
            peer.close();
            let caught;
            try {peer.createDataChannel({toString() {order.push('label'); return 'c';}}, {
                get id() {order.push('id'); throw sentinel;}
            });} catch (error) {caught = error;}
            check(caught === sentinel && order.join(',') === 'label,id');
            fails('InvalidStateError', () => peer.createDataChannel('c'));
            return true;
        })()"#).unwrap(), "true");
}

#[tokio::test(flavor = "current_thread")]
async fn signaling_promises_keep_descriptions_and_report_closed_errors() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = rtc_platform_vm(&loader);

    vm.eval(r#"
            globalThis.peerDone = false;
            globalThis.peerFailure = null;
            (async () => {
                const peer = new RTCPeerConnection();
                peer.createDataChannel('data');
                const offer = await peer.createOffer({offerToReceiveAudio: 1});
                check(offer.type === 'offer' && offer.sdp.includes('m=audio') && offer.sdp.includes('m=application'));
                await peer.setLocalDescription(offer);
                check(peer.localDescription instanceof RTCSessionDescription);
                check(peer.localDescription.type === 'offer' && peer.localDescription.sdp === offer.sdp);
                check(peer.pendingLocalDescription === peer.localDescription && peer.currentLocalDescription === null);
                check(peer.signalingState === 'have-local-offer' && peer.iceGatheringState === 'new');
                peer.close();
                const failed = peer.createOffer();
                check(failed instanceof Promise);
                let caught; try {await failed;} catch (error) {caught = error;}
                check(caught && caught.name === 'InvalidStateError');
                peerDone = true;
            })().catch(error => peerFailure = String(error));
        "#).unwrap();

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(peerDone || peerFailure !== null)",
        "true",
        "signaling promises must settle through the native operations queue",
    )
    .await;
    assert_eq!(vm.eval("String(peerFailure)").unwrap(), "null");
    assert_eq!(vm.eval("String(peerDone)").unwrap(), "true");
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_native_values_use_canonical_intrinsics_and_fresh_capability_objects() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = rtc_platform_vm(&loader);

    assert_eq!(vm.eval(r#"(() => {
            const OriginalChannel = RTCDataChannel;
            const peer = new RTCPeerConnection();
            Object.defineProperty(globalThis, 'RTCDataChannel', {configurable: true, get() {throw new Error('public constructor');}});
            const channel = peer.createDataChannel('canonical');
            check(channel instanceof OriginalChannel && Object.getPrototypeOf(channel) === OriginalChannel.prototype);
            const caps = RTCRtpReceiver.getCapabilities('audio');
            check(Array.isArray(caps.codecs) && caps.codecs.length > 0 && Array.isArray(caps.headerExtensions));
            caps.codecs.length = 0;
            check(RTCRtpReceiver.getCapabilities('audio').codecs.length > 0);
            check(RTCRtpReceiver.getCapabilities('unknown') === null);
            check(RTCRtpReceiver.getCapabilities('video').codecs.length > 0);
            peer.close();
            return true;
        })()"#).unwrap(), "true");
}
