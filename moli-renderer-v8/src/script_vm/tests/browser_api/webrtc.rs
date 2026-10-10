use super::*;

#[tokio::test(flavor = "current_thread")]
async fn webrtc_signaling_surface_stays_explicit_about_the_missing_ice_transport() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://webrtc-surface.test/", &loader);

    vm.exec(
        r#"
        (() => {
          const descriptor = (owner, name) => {
            const value = Object.getOwnPropertyDescriptor(owner, name);
            return value && [
              value.enumerable,
              value.configurable,
              value.writable,
              typeof value.value,
              value.value?.name,
              value.value?.length,
              typeof value.get,
              typeof value.set,
            ];
          };
          const throwsTypeError = callback => {
            try {
              callback();
              return false;
            } catch (error) {
              return error instanceof TypeError;
            }
          };
          const configuration = {
            iceCandidatePoolSize: 1,
            iceServers: [
              { urls: "stun:stun.cloudflare.com:3478" },
              { urls: "stun:stun.l.google.com:19302" },
              { urls: "stun:stun1.l.google.com:19302" },
            ],
          };
          const peer = new RTCPeerConnection(configuration);
          const channel = peer.createDataChannel("");
          const events = [];
          for (const type of ["icegatheringstatechange", "icecandidate"]) {
            peer.addEventListener(type, event => events.push([
              event.type,
              peer.iceGatheringState,
              event.candidate === null,
            ]));
          }
          const audio = RTCRtpReceiver.getCapabilities("audio");
          const video = RTCRtpReceiver.getCapabilities("video");
          const offerPromise = peer.createOffer({
            offerToReceiveAudio: true,
            offerToReceiveVideo: true,
          });
          globalThis.__webrtcProbe = {
            constructor: [
              descriptor(window, "RTCPeerConnection"),
              RTCPeerConnection.name,
              RTCPeerConnection.length,
              Object.prototype.toString.call(RTCPeerConnection.prototype),
              Object.getPrototypeOf(RTCPeerConnection.prototype) === EventTarget.prototype,
              throwsTypeError(() => RTCPeerConnection()),
            ],
            receiver: [
              descriptor(window, "RTCRtpReceiver"),
              descriptor(RTCRtpReceiver, "getCapabilities"),
              throwsTypeError(() => new RTCRtpReceiver()),
              audio.codecs.length,
              audio.headerExtensions.length,
              audio.codecs[0].mimeType,
              video.codecs.length,
              video.headerExtensions.length,
              video.codecs[0].mimeType,
              RTCRtpReceiver.getCapabilities("bogus"),
            ],
            methodDescriptors: ["createDataChannel", "createOffer", "setLocalDescription", "close"]
              .map(name => [name, descriptor(RTCPeerConnection.prototype, name)]),
            initial: [
              Object.prototype.toString.call(peer),
              peer instanceof RTCPeerConnection,
              peer instanceof EventTarget,
              peer.signalingState,
              peer.iceGatheringState,
              peer.iceConnectionState,
              peer.connectionState,
              peer.localDescription,
              peer.currentLocalDescription,
              peer.pendingLocalDescription,
              Object.prototype.toString.call(channel),
              channel instanceof RTCDataChannel,
              channel instanceof EventTarget,
              channel.label,
              channel.ordered,
              channel.maxPacketLifeTime,
              channel.maxRetransmits,
              channel.protocol,
              channel.negotiated,
              channel.id,
              channel.readyState,
              channel.bufferedAmount,
              channel.binaryType,
            ],
            promiseTag: Object.prototype.toString.call(offerPromise),
            settled: false,
          };
          offerPromise
            .then(offer => {
              __webrtcProbe.offer = [
                Object.prototype.toString.call(offer),
                offer.constructor.name,
                offer.type,
                offer.sdp.includes("m=audio"),
                offer.sdp.includes("m=video"),
                offer.sdp.includes("m=application"),
              ];
              return peer.setLocalDescription(offer);
            })
            .then(value => {
              __webrtcProbe.setLocalResult = typeof value;
              __webrtcProbe.afterSetLocal = [
                peer.signalingState,
                peer.iceGatheringState,
                peer.iceConnectionState,
                peer.connectionState,
                peer.localDescription?.type,
                peer.currentLocalDescription,
                peer.pendingLocalDescription?.type,
                events,
              ];
              peer.close();
              __webrtcProbe.afterClose = [
                peer.signalingState,
                peer.iceGatheringState,
                peer.iceConnectionState,
                peer.connectionState,
              ];
              __webrtcProbe.settled = true;
            });
        })()
        "#,
        None,
    )
    .expect("WebRTC signaling probe should execute");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__webrtcProbe.settled)",
        "true",
        "signaling must complete through selected networking tasks",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(globalThis.__webrtcProbe)")
        .expect("WebRTC signaling probe should be readable");
    let value: serde_json::Value =
        serde_json::from_str(&result).expect("WebRTC signaling probe should return JSON");

    assert_eq!(value["constructor"][1], "RTCPeerConnection");
    assert_eq!(value["constructor"][2], 0);
    assert_eq!(value["constructor"][3], "[object RTCPeerConnection]");
    assert_eq!(value["constructor"][4], true);
    assert_eq!(value["constructor"][5], true);
    assert_eq!(
        value["constructor"][0],
        serde_json::json!([
            false,
            true,
            true,
            "function",
            "RTCPeerConnection",
            0,
            "undefined",
            "undefined"
        ])
    );
    assert_eq!(value["receiver"][2], true);
    assert_eq!(value["receiver"][3], 8);
    assert_eq!(value["receiver"][4], 4);
    assert_eq!(value["receiver"][5], "audio/opus");
    assert_eq!(value["receiver"][6], 19);
    assert_eq!(value["receiver"][7], 11);
    assert_eq!(value["receiver"][8], "video/VP8");
    assert!(value["receiver"][9].is_null());
    assert_eq!(
        value["initial"],
        serde_json::json!([
            "[object RTCPeerConnection]",
            true,
            true,
            "stable",
            "new",
            "new",
            "new",
            null,
            null,
            null,
            "[object RTCDataChannel]",
            true,
            true,
            "",
            true,
            null,
            null,
            "",
            false,
            null,
            "connecting",
            0,
            "arraybuffer"
        ])
    );
    assert_eq!(value["promiseTag"], "[object Promise]");
    assert_eq!(
        value["offer"],
        serde_json::json!(["[object Object]", "Object", "offer", true, true, true])
    );
    assert_eq!(value["setLocalResult"], "undefined");
    assert_eq!(
        value["afterSetLocal"],
        serde_json::json!([
            "have-local-offer",
            "new",
            "new",
            "new",
            "offer",
            null,
            "offer",
            []
        ])
    );
    assert_eq!(
        value["afterClose"],
        serde_json::json!(["closed", "new", "closed", "closed"])
    );
    assert_eq!(value["settled"], true);

    for entry in value["methodDescriptors"]
        .as_array()
        .expect("WebRTC method descriptors")
    {
        let name = entry[0].as_str().expect("WebRTC method name");
        let descriptor = &entry[1];
        assert_eq!(descriptor[0], true, "{name} enumerable");
        assert_eq!(descriptor[1], true, "{name} configurable");
        assert_eq!(descriptor[2], true, "{name} writable");
        assert_eq!(descriptor[3], "function", "{name} value type");
        assert_eq!(descriptor[4], name, "{name} function name");
        assert_eq!(descriptor[5], usize::from(name == "createDataChannel"));
    }
}
