use super::*;

#[test]
fn navigator_geolocation_is_same_object_and_rejects_insecure_requests_asynchronously() {
    let mut vm = new_storage_test_vm("http://insecure-geolocation.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
              const geolocation = navigator.geolocation;
              const methodShape = name => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  Geolocation.prototype,
                  name
                );
                return [
                  descriptor.value.name,
                  descriptor.value.length,
                  descriptor.enumerable,
                  descriptor.configurable
                ].join(":");
              };
              const throws = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return error && error.name;
                }
              };
              const navigatorDescriptor = Object.getOwnPropertyDescriptor(
                Navigator.prototype,
                "geolocation"
              );
              globalThis.__geolocationErrors = [];
              let synchronous = true;
              const watchId = geolocation.watchPosition(
                () => __geolocationErrors.push("watch-success"),
                error => __geolocationErrors.push([
                  "watch",
                  synchronous,
                  error.code,
                  error instanceof GeolocationPositionError,
                  error.PERMISSION_DENIED === 1,
                  GeolocationPositionError.PERMISSION_DENIED === 1
                ].join(":"))
              );
              geolocation.getCurrentPosition(
                () => __geolocationErrors.push("current-success"),
                error => __geolocationErrors.push([
                  "current",
                  synchronous,
                  error.code,
                  error instanceof GeolocationPositionError,
                  error.PERMISSION_DENIED === 1,
                  GeolocationPositionError.PERMISSION_DENIED === 1
                ].join(":"))
              );
              const result = {
                sameObject: geolocation === navigator.geolocation,
                instance: geolocation instanceof Geolocation,
                navigatorOwn: Object.hasOwn(navigator, "geolocation"),
                navigatorDescriptor: [
                  navigatorDescriptor.get.name,
                  navigatorDescriptor.get.length,
                  navigatorDescriptor.enumerable,
                  navigatorDescriptor.configurable
                ].join(":"),
                methods: [
                  methodShape("getCurrentPosition"),
                  methodShape("watchPosition"),
                  methodShape("clearWatch")
                ],
                watchId: Number.isInteger(watchId) && watchId > 0,
                immediateErrors: __geolocationErrors.length,
                invalid: [
                  throws(() => geolocation.getCurrentPosition()),
                  throws(() => geolocation.getCurrentPosition(null)),
                  throws(() => geolocation.getCurrentPosition(() => {}, 4)),
                  throws(() => geolocation.getCurrentPosition(() => {}, () => {}, 4)),
                  throws(() => Geolocation.prototype.getCurrentPosition.call({}, () => {}))
                ]
              };
              synchronous = false;
              return JSON.stringify(result);
            })()
            "#,
        )
        .expect("insecure Geolocation surface should evaluate");

    assert_eq!(
        initial,
        r#"{"sameObject":true,"instance":true,"navigatorOwn":false,"navigatorDescriptor":"get geolocation:0:true:true","methods":["getCurrentPosition:1:true:true","watchPosition:1:true:true","clearWatch:1:true:true"],"watchId":true,"immediateErrors":0,"invalid":["TypeError","TypeError","TypeError","TypeError","TypeError"]}"#
    );

    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("Geolocation watch error task should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("Geolocation current-position error task should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__geolocationErrors)")
            .expect("Geolocation error results should evaluate"),
        r#"["watch:false:1:true:true:true","current:false:1:true:true:true"]"#
    );
}
#[test]
fn navigator_geolocation_zero_timeout_reports_timeout_in_secure_context() {
    let mut vm = new_storage_test_vm("https://secure-geolocation.test/");
    vm.set_permission_overrides(&[crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::Value::String("geolocation".to_owned()),
        setting: "granted".to_owned(),
        origin: None,
        embedded_origin: None,
    }]);
    vm.eval(
        r#"
        globalThis.__geolocationTimeoutCode = 0;
        navigator.geolocation.getCurrentPosition(
          () => { globalThis.__geolocationTimeoutCode = -1; },
          error => { globalThis.__geolocationTimeoutCode = error.code; },
          { timeout: -100, maximumAge: -100, enableHighAccuracy: "yes" }
        );
        "#,
    )
    .expect("secure Geolocation timeout request should evaluate");
    assert_eq!(
        vm.eval("String(globalThis.__geolocationTimeoutCode)")
            .expect("Geolocation timeout should remain pending"),
        "0"
    );
    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("Geolocation timeout task should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
    assert_eq!(
        vm.eval("String(globalThis.__geolocationTimeoutCode)")
            .expect("Geolocation timeout result should evaluate"),
        "3"
    );
}
#[test]
fn navigator_geolocation_error_uses_webidl_callback_function_semantics() {
    let mut vm = new_storage_test_vm("https://geolocation-callback-semantics.test/");
    vm.eval(
        r#"
        const frame = document.createElement("iframe");
        (document.body || document.documentElement || document).appendChild(frame);
        globalThis.__geolocationCallbackFrame = frame;
        "#,
    )
    .expect("Geolocation callback Realm setup should evaluate");
    materialize_single_child_default_realm_for_test(&mut vm, "Geolocation callback Realm");

    vm.eval(
        r#"
        (() => {
          const child = __geolocationCallbackFrame.contentWindow;
          globalThis.__geolocationCallbackFacts = null;
          globalThis.__geolocationProxyCalls = 0;
          const success = child.Function("return new Proxy(function() {}, {})")();
          const error = child.Function(`
            return new Proxy(
              function(error) {
                "use strict";
                parent.__geolocationCallbackFacts = {
                  callbackRealm:
                    globalThis === parent.__geolocationCallbackFrame.contentWindow,
                  receiverUndefined: this === undefined,
                  argumentCount: arguments.length,
                  errorTargetRealm:
                    Object.getPrototypeOf(error) ===
                      parent.GeolocationPositionError.prototype,
                  code: error.code,
                  proxyCalls: parent.__geolocationProxyCalls
                };
              },
              {
                apply(target, receiver, argumentsList) {
                  parent.__geolocationProxyCalls++;
                  if (receiver !== undefined)
                    throw new Error("Geolocation callback receiver was not undefined");
                  return Reflect.apply(target, receiver, argumentsList);
                }
              }
            );
          `)();
          navigator.geolocation.getCurrentPosition(success, error);
          return "scheduled";
        })()
        "#,
    )
    .expect("Geolocation Web IDL callbacks should schedule");

    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("Geolocation callback task should run"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
    assert_eq!(
        vm.eval("JSON.stringify(__geolocationCallbackFacts)")
            .expect("Geolocation callback facts should evaluate"),
        r#"{"callbackRealm":true,"receiverUndefined":true,"argumentCount":1,"errorTargetRealm":true,"code":2,"proxyCalls":1}"#
    );
}
#[test]
fn navigator_geolocation_clear_watch_cancels_only_the_exact_pending_watch() {
    let mut vm = new_storage_test_vm("https://geolocation-clear-watch.test/");
    let ids = vm
        .eval(
            r#"
            (() => {
              globalThis.__geolocationCancelledWatchRan = false;
              globalThis.__geolocationCollidingTimerRan = false;
              const timerId = setTimeout(
                () => { __geolocationCollidingTimerRan = true; },
                0
              );
              const watchId = navigator.geolocation.watchPosition(
                () => {},
                () => { __geolocationCancelledWatchRan = true; }
              );
              navigator.geolocation.clearWatch(watchId);
              return JSON.stringify({ timerId, watchId });
            })()
            "#,
        )
        .expect("Geolocation watch cancellation should evaluate");
    assert_eq!(
        ids, r#"{"timerId":1,"watchId":1}"#,
        "the witness deliberately collides independent timer and watch id spaces"
    );

    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("colliding ordinary timer should remain runnable"),
        crate::host::HostTimeoutRunResult::Consumed
    ));
    assert!(
        !vm.has_ready_timeout(),
        "clearWatch must remove the pending Geolocation task without cancelling the timer"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ watch: __geolocationCancelledWatchRan, timer: __geolocationCollidingTimerRan })"
        )
        .expect("Geolocation cancellation result should evaluate"),
        r#"{"watch":false,"timer":true}"#
    );
}
#[test]
fn navigator_geolocation_retires_with_target_or_callback_window() {
    let mut vm = new_storage_test_vm("https://geolocation-window-retirement.test/");
    vm.eval(
        r#"
        const callbackFrame = document.createElement("iframe");
        const targetFrame = document.createElement("iframe");
        const parent = document.body || document.documentElement || document;
        parent.appendChild(callbackFrame);
        parent.appendChild(targetFrame);
        globalThis.__geolocationCallbackRetiredRan = false;
        globalThis.__geolocationTargetRetiredRan = false;
        globalThis.__geolocationCallbackRetirementFrame = callbackFrame;
        globalThis.__geolocationTargetRetirementFrame = targetFrame;
        "#,
    )
    .expect("Geolocation retirement setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval(
        r#"
        (() => {
          const callbackChild = __geolocationCallbackRetirementFrame.contentWindow;
          const targetChild = __geolocationTargetRetirementFrame.contentWindow;
          navigator.geolocation.getCurrentPosition(
            () => {},
            callbackChild.Function(
              `parent.__geolocationCallbackRetiredRan = true;`
            )
          );
          targetChild.navigator.geolocation.getCurrentPosition(
            () => {},
            () => { __geolocationTargetRetiredRan = true; }
          );
          __geolocationCallbackRetirementFrame.remove();
          __geolocationTargetRetirementFrame.remove();
          return "scheduled-and-retired";
        })()
        "#,
    )
    .expect("Geolocation retirement requests should schedule");

    for _ in 0..2 {
        if vm.has_ready_timeout() {
            let _ = vm
                .run_next_timeout_for_test()
                .expect("any surviving retired Geolocation task should be consumed");
        }
    }
    assert_eq!(
        vm.eval(
            "JSON.stringify({ callback: __geolocationCallbackRetiredRan, target: __geolocationTargetRetiredRan })"
        )
        .expect("Geolocation retirement result should evaluate"),
        r#"{"callback":false,"target":false}"#
    );
}
#[test]
fn navigator_geolocation_callback_exceptions_use_the_callback_window() {
    let mut vm = new_storage_test_vm("https://geolocation-callback-error.test/");
    vm.eval(
        r#"
        const frame = document.createElement("iframe");
        (document.body || document.documentElement || document).appendChild(frame);
        globalThis.__geolocationErrorFrame = frame;
        "#,
    )
    .expect("Geolocation error callback Realm setup should evaluate");
    materialize_single_child_default_realm_for_test(&mut vm, "Geolocation error callback Realm");

    vm.eval(
        r#"
        (() => {
          const child = __geolocationErrorFrame.contentWindow;
          globalThis.__geolocationReportedErrors = [];
          child.onerror = child.Function(
            "message",
            `parent.__geolocationReportedErrors.push(message); return true;`
          );
          navigator.geolocation.getCurrentPosition(
            () => {},
            child.Function(`throw new RangeError("geolocation-callback-error");`)
          );
          return "scheduled";
        })()
        "#,
    )
    .expect("throwing Geolocation callback should schedule");

    assert!(matches!(
        vm.run_next_timeout_for_test()
            .expect("throwing Geolocation callback task should complete"),
        crate::host::HostTimeoutRunResult::CallbackError(_)
    ));
    assert_eq!(
        vm.eval("JSON.stringify(__geolocationReportedErrors)")
            .expect("Geolocation callback error projection should evaluate"),
        r#"["Uncaught RangeError: geolocation-callback-error"]"#
    );
}
#[test]
fn navigator_media_capabilities_rejects_insecure_encrypted_queries_as_promises() {
    let mut vm = new_storage_test_vm("http://insecure-media-capabilities.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
              const capabilities = navigator.mediaCapabilities;
              const descriptor = Object.getOwnPropertyDescriptor(
                Navigator.prototype,
                "mediaCapabilities"
              );
              const methodShape = name => {
                const method = Object.getOwnPropertyDescriptor(
                  MediaCapabilities.prototype,
                  name
                );
                return [
                  method.value.name,
                  method.value.length,
                  method.enumerable,
                  method.configurable
                ].join(":");
              };
              const video = {
                contentType: 'video/webm; codecs="vp09.00.10.08"',
                width: 800,
                height: 600,
                bitrate: 3000,
                framerate: 24
              };
              let synchronousThrow = "none";
              let encrypted;
              try {
                encrypted = capabilities.decodingInfo({
                  type: "file",
                  video,
                  keySystemConfiguration: { keySystem: "org.w3.clearkey" }
                });
              } catch (error) {
                synchronousThrow = error && error.name;
              }
              const invalid = capabilities.decodingInfo({
                type: "file",
                keySystemConfiguration: { keySystem: "org.w3.clearkey" }
              });
              const marker = new Error("configuration getter marker");
              const getterFailure = capabilities.decodingInfo({
                type: "file",
                get video() { throw marker; }
              });
              let brandError = "none";
              let brandFailure;
              try {
                brandFailure = MediaCapabilities.prototype.decodingInfo.call({}, {
                  type: "file",
                  video
                });
              } catch (error) {
                brandError = error && error.name;
              }
              globalThis.__mediaCapabilitiesSettlement = "pending";
              Promise.all([
                encrypted.then(
                  () => "encrypted:resolved",
                  error => `encrypted:${error.name}:${error instanceof DOMException}`
                ),
                invalid.then(
                  () => "invalid:resolved",
                  error => `invalid:${error.name}`
                ),
                getterFailure.then(
                  () => "getter:resolved",
                  error => `getter:${error === marker}`
                ),
                brandFailure.then(
                  () => "brand:resolved",
                  error => `brand:${error.name}`
                )
              ]).then(values => {
                globalThis.__mediaCapabilitiesSettlement = values.join("|");
              });
              return JSON.stringify({
                sameObject: capabilities === navigator.mediaCapabilities,
                instance: capabilities instanceof MediaCapabilities,
                tag: Object.prototype.toString.call(capabilities),
                navigatorOwn: Object.hasOwn(navigator, "mediaCapabilities"),
                navigatorDescriptor: [
                  descriptor.get.name,
                  descriptor.get.length,
                  descriptor.enumerable,
                  descriptor.configurable
                ].join(":"),
                methods: [methodShape("decodingInfo"), methodShape("encodingInfo")],
                promise: encrypted instanceof Promise,
                synchronousThrow,
                brandPromise: brandFailure instanceof Promise,
                brandError
              });
            })()
            "#,
        )
        .expect("insecure MediaCapabilities surface should evaluate");

    assert_eq!(
        initial,
        r#"{"sameObject":true,"instance":true,"tag":"[object MediaCapabilities]","navigatorOwn":false,"navigatorDescriptor":"get mediaCapabilities:0:true:true","methods":["decodingInfo:1:true:true","encodingInfo:1:true:true"],"promise":true,"synchronousThrow":"none","brandPromise":true,"brandError":"none"}"#
    );
    assert_eq!(
        vm.eval("String(globalThis.__mediaCapabilitiesSettlement)")
            .expect("MediaCapabilities rejections should settle"),
        "encrypted:SecurityError:true|invalid:TypeError|getter:true|brand:TypeError"
    );
}
#[test]
fn navigator_media_capabilities_resolves_normalized_headless_results() {
    let mut vm = new_storage_test_vm("https://secure-media-capabilities.test/");

    vm.eval(
        r#"
        (() => {
          const decodingConfiguration = {
            type: "file",
            video: {
              contentType: 'video/webm; codecs="vp09.00.10.08"',
              width: 800,
              height: 600,
              bitrate: 3000,
              framerate: 24
            }
          };
          const encodingConfiguration = {
            type: "record",
            audio: { contentType: 'audio/webm; codecs="opus"' }
          };
          globalThis.__mediaCapabilitiesResults = "pending";
          Promise.all([
            navigator.mediaCapabilities.decodingInfo(decodingConfiguration),
            navigator.mediaCapabilities.encodingInfo(encodingConfiguration)
          ]).then(([decoding, encoding]) => {
            globalThis.__mediaCapabilitiesResults = JSON.stringify({
              decoding: [
                typeof decoding.supported,
                decoding.smooth,
                decoding.powerEfficient,
                decoding.keySystemAccess,
                decoding.configuration === decodingConfiguration,
                decoding.configuration.type,
                decoding.configuration.video.framerate
              ],
              encoding: [
                typeof encoding.supported,
                encoding.smooth,
                encoding.powerEfficient,
                encoding.configuration === encodingConfiguration,
                encoding.configuration.type,
                encoding.configuration.audio.contentType
              ]
            });
          });
        })()
        "#,
    )
    .expect("secure MediaCapabilities queries should evaluate");

    assert_eq!(
        vm.eval("String(globalThis.__mediaCapabilitiesResults)")
            .expect("MediaCapabilities results should settle"),
        r#"{"decoding":["boolean",false,false,null,false,"file",24],"encoding":["boolean",false,false,false,"record","audio/webm; codecs=\"opus\""]}"#
    );
}
