use super::*;

#[tokio::test]
async fn pointer_lock_without_activation_queues_errors_and_rejects_each_promise() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm(
        "https://pointer-lock-no-activation.test/",
        "<!doctype html><html><body></body></html>",
    );

    let before = vm
        .eval(
            r#"
(() => {
  globalThis.__pointerLockErrors = [];
  globalThis.__pointerLockRejections = [];
  document.onpointerlockerror = event => {
    __pointerLockErrors.push([
      event.type,
      event.isTrusted,
      event.bubbles,
      event.cancelable
    ].join(":"));
  };
  const first = document.body.requestPointerLock();
  const second = document.body.requestPointerLock();
  first.catch(error => __pointerLockRejections.push(error.name));
  second.catch(error => __pointerLockRejections.push(error.name));
  return JSON.stringify({
    methodType: typeof document.body.requestPointerLock,
    methodLength: document.body.requestPointerLock.length,
    promises: first instanceof Promise && second instanceof Promise && first !== second,
    initialTarget: document.pointerLockElement,
    errorAccessor: Object.hasOwn(Document.prototype, "onpointerlockerror"),
    changeAccessor: Object.hasOwn(Document.prototype, "onpointerlockchange"),
    errors: __pointerLockErrors.length,
    rejections: __pointerLockRejections.length
  });
})()
"#,
        )
        .expect("pointer lock rejection setup should evaluate");

    assert_eq!(
        before,
        r#"{"methodType":"function","methodLength":0,"promises":true,"initialTarget":null,"errorAccessor":true,"changeAccessor":true,"errors":0,"rejections":0}"#,
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("pointer lock error tasks should drain");

    let after = vm
        .eval(
            r#"
JSON.stringify({
  errors: __pointerLockErrors,
  rejections: __pointerLockRejections.sort(),
  target: document.pointerLockElement
})
"#,
        )
        .expect("pointer lock rejection result should evaluate");
    assert_eq!(
        after,
        r#"{"errors":["pointerlockerror:true:false:false","pointerlockerror:true:false:false"],"rejections":["NotAllowedError","NotAllowedError"],"target":null}"#,
    );
}

#[tokio::test]
async fn activated_pointer_lock_reports_unsupported_after_observable_option_conversion() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_test_vm(
        "https://pointer-lock-unsupported.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm._context_host.borrow_mut().notify_user_activation();
    let result = vm.eval(
        r#"
(() => {
  globalThis.__pointerLockUnsupported = [];
  globalThis.__pointerLockOptionReads = 0;
  document.addEventListener("pointerlockerror", () => {
    __pointerLockUnsupported.push("error");
  });
  const options = {
    get unadjustedMovement() {
      __pointerLockOptionReads++;
      return false;
    }
  };
  document.body.requestPointerLock(options).catch(error => {
    __pointerLockUnsupported.push(error.name);
  });
  return JSON.stringify({
    optionReads: __pointerLockOptionReads,
    target: document.pointerLockElement,
    exitUndefined: document.exitPointerLock() === undefined
  });
})()
"#,
    );
    assert_eq!(
        result.expect("activated pointer lock request should evaluate"),
        r#"{"optionReads":1,"target":null,"exitUndefined":true}"#,
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("unsupported pointer lock error task should drain");
    assert_eq!(
        vm.eval("__pointerLockUnsupported.join('|')")
            .expect("unsupported pointer lock result should evaluate"),
        "error|NotSupportedError",
    );
}
