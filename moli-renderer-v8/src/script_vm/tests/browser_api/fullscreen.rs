use super::*;

#[test]
fn fullscreen_enabled_reflects_committed_document_policy() {
    let mut vm = new_parsed_test_vm(
        "https://fullscreen-policy.test/",
        "<!doctype html><iframe id='child'></iframe>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const child = document.getElementById("child");
  child.allow = "sync-xhr 'none'";
  return JSON.stringify({
    enabled: [
      document.fullscreenEnabled,
      child.contentDocument.fullscreenEnabled,
      new Document().fullscreenEnabled
    ],
    allow: [child.allow, child.getAttribute("allow")]
  });
})()
"#,
        )
        .expect("fullscreen policy probe should evaluate");

    assert_eq!(
        result,
        r#"{"enabled":[true,true,false],"allow":["sync-xhr 'none'","sync-xhr 'none'"]}"#
    );
}

#[tokio::test]
async fn unsupported_fullscreen_requests_reject_and_queue_spec_shaped_error_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm(
        "https://fullscreen-unsupported.test/",
        "<!doctype html><html><body><div></div></body></html>",
    );

    let before = vm
        .eval(
            r#"
(() => {
  globalThis.__fullscreenEvents = [];
  globalThis.__fullscreenRejections = [];
  globalThis.__fullscreenConversionRejections = [];
  globalThis.__fullscreenOptionReads = [];
  document.body.setAttribute("onfullscreenerror", "globalThis.__fullscreenAttributeRan = true");
  const contentAttributeInstalled = typeof document.body.onfullscreenerror === "function";
  document.onfullscreenerror = event => {
    __fullscreenEvents.push([
      event.type,
      event.target === document.body,
      event.bubbles,
      event.cancelable,
      event.composed
    ].join(":"));
  };
  const options = {
    get keyboardLock() {
      __fullscreenOptionReads.push("keyboardLock");
      return "none";
    },
    get navigationUI() {
      __fullscreenOptionReads.push("navigationUI");
      return "hide";
    }
  };
  const request = document.body.requestFullscreen(options);
  const exit = document.exitFullscreen();
  request.catch(error => __fullscreenRejections.push(`request:${error.name}`));
  exit.catch(error => __fullscreenRejections.push(`exit:${error.name}`));
  const getterError = new RangeError("navigationUI getter failed");
  const conversionInputs = [
    ["enum", { navigationUI: "invalid-value" }, null],
    ["string", "foo", null],
    ["number", 123, null],
    ["getter", {
      get navigationUI() {
        throw getterError;
      }
    }, getterError]
  ];
  const conversionPromiseShapes = conversionInputs.map(([label, input, expected]) => {
    try {
      const promise = document.body.requestFullscreen(input);
      promise.catch(error => __fullscreenConversionRejections.push(
        `${label}:${error.name}:${expected === null || error === expected}`
      ));
      return promise instanceof Promise;
    } catch (error) {
      return `throw:${error.name}`;
    }
  });
  const outcome = callback => {
    try {
      callback();
      return "return";
    } catch (error) {
      return error.name;
    }
  };
  return JSON.stringify({
    requestShape: [
      typeof Element.prototype.requestFullscreen,
      Element.prototype.requestFullscreen.length,
      Object.hasOwn(Element.prototype, "onfullscreenchange"),
      Object.hasOwn(Element.prototype, "onfullscreenerror")
    ],
    documentShape: [
      typeof Document.prototype.exitFullscreen,
      Document.prototype.exitFullscreen.length,
      Object.hasOwn(Document.prototype, "onfullscreenchange"),
      Object.hasOwn(Document.prototype, "onfullscreenerror"),
      document.fullscreenEnabled
    ],
    promises: request instanceof Promise && exit instanceof Promise,
    conversionPromiseShapes,
    contentAttributeInstalled,
    optionReads: __fullscreenOptionReads,
    wrongReceivers: [
      outcome(() => Element.prototype.requestFullscreen.call({})),
      outcome(() => Document.prototype.exitFullscreen.call({}))
    ],
    events: __fullscreenEvents.length
  });
})()
"#,
        )
        .expect("fullscreen rejection setup should evaluate");

    assert_eq!(
        before,
        r#"{"requestShape":["function",0,true,true],"documentShape":["function",0,true,true,true],"promises":true,"conversionPromiseShapes":[true,true,true,true],"contentAttributeInstalled":true,"optionReads":["keyboardLock","navigationUI"],"wrongReceivers":["TypeError","TypeError"],"events":0}"#,
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("fullscreen error task should drain");

    let after = vm
        .eval(
            r#"
JSON.stringify({
  events: __fullscreenEvents,
  rejections: __fullscreenRejections.sort(),
  conversionRejections: __fullscreenConversionRejections.sort(),
  contentAttributeRan: globalThis.__fullscreenAttributeRan === true
})
"#,
        )
        .expect("fullscreen rejection result should evaluate");
    assert_eq!(
        after,
        r#"{"events":["fullscreenerror:true:true:false:true"],"rejections":["exit:TypeError","request:TypeError"],"conversionRejections":["enum:TypeError:true","getter:RangeError:true","number:TypeError:true","string:TypeError:true"],"contentAttributeRan":true}"#,
    );
}
