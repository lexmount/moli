use super::*;

#[test]
fn transform_finish_pending_cancel_observes_writable_error_at_fulfillment() {
    let mut vm = stream_test_vm();
    vm.eval(
        r#"
(() => {
  globalThis.__finishEvents = [];
  const error = { name: 'controller error' };
  let controller, release;
  const gate = new Promise(resolve => { release = resolve; });
  const stream = new TransformStream({
    start(c) { controller = c; },
    cancel() { return gate; }
  });
  const writer = stream.writable.getWriter();
  stream.readable.cancel({ name: 'cancel reason' }).then(
    () => __finishEvents.push('cancel:fulfilled'),
    reason => __finishEvents.push(`cancel:${reason === error}`)
  );
  writer.closed.catch(reason => {
    __finishEvents.push(`closed:${reason === error}`);
    // The writable has completed Erroring -> Errored before cancel can finish.
    release();
  });
  controller.error(error);
})()
"#,
    )
    .expect("pending cancel setup");
    assert_eq!(
        vm.eval("JSON.stringify(__finishEvents)")
            .expect("pending cancel result"),
        r#"["closed:true","cancel:true"]"#
    );
}

#[test]
fn transform_finish_pending_cancel_rejection_keeps_callback_error() {
    let mut vm = stream_test_vm();
    vm.eval(
        r#"
(() => {
  globalThis.__finishEvents = [];
  const storedError = { name: 'controller error' };
  const callbackError = { name: 'callback error' };
  let controller, reject;
  const gate = new Promise((_, rejectPromise) => { reject = rejectPromise; });
  const stream = new TransformStream({
    start(c) { controller = c; },
    cancel() { return gate; }
  });
  const writer = stream.writable.getWriter();
  stream.readable.cancel({ name: 'cancel reason' }).then(
    () => __finishEvents.push('cancel:fulfilled'),
    reason => __finishEvents.push(`cancel:${reason === callbackError}`)
  );
  writer.closed.catch(reason => {
    __finishEvents.push(`closed:${reason === storedError}`);
    reject(callbackError);
  });
  controller.error(storedError);
})()
"#,
    )
    .expect("rejecting cancel setup");
    assert_eq!(
        vm.eval("JSON.stringify(__finishEvents)")
            .expect("rejecting cancel result"),
        r#"["closed:true","cancel:true"]"#
    );
}

#[test]
fn transform_finish_cancellation_observes_algorithm_return_kind() {
    let mut vm = stream_test_vm();
    vm.eval(
        r#"
(async () => {
  globalThis.__finishEvents = [];
  for (const during of [false, true]) {
    for (const kind of ['absent', 'undefined', 'promise']) {
      if (during && kind === 'absent') continue;
      const error = {};
      let controller, calls = 0;
      const transformer = { start(c) { controller = c; } };
      if (kind !== 'absent') transformer.cancel = () => {
        calls++;
        if (during) controller.error(error);
        if (kind === 'promise') return Promise.resolve();
      };
      const stream = new TransformStream(transformer);
      const writer = stream.writable.getWriter();
      const closed = writer.closed.then(() => false, reason => reason === error);
      const canceled = stream.readable.cancel({}).then(
        () => 'fulfilled', reason => reason === error ? 'stored-error' : 'wrong-error'
      );
      if (!during) controller.error(error);
      __finishEvents.push(`${kind}:${during}:${await canceled}:${await closed}:${calls}`);
    }
  }
})()
"#,
    )
    .expect("cancel return kind setup");
    assert_eq!(
        vm.eval("JSON.stringify(__finishEvents)")
            .expect("cancel return kind results"),
        r#"["absent:false:fulfilled:true:0","undefined:false:fulfilled:true:1","promise:false:stored-error:true:1","undefined:true:fulfilled:true:1","promise:true:stored-error:true:1"]"#
    );
}

#[test]
fn transform_finish_webidl_callback_adopts_returned_promise_once() {
    let mut vm = stream_test_vm();
    vm.eval(
        r#"
(async () => {
  globalThis.__finishEvents = [];
  for (const throws of [false, true]) {
    let gets = 0, calls = 0;
    const error = {};
    const returned = Promise.resolve();
    const then = Promise.prototype.then;
    Object.defineProperty(returned, 'then', {
      get() {
        gets++;
        if (throws) throw error;
        return function(...args) {
          calls++;
          return Reflect.apply(then, this, args);
        };
      }
    });
    const stream = new TransformStream({ cancel() { return returned; } });
    const writer = stream.writable.getWriter();
    const cancelReason = {};
    const closed = writer.closed.then(() => false, reason => reason === (throws ? error : cancelReason));
    const result = await stream.readable.cancel(cancelReason).then(
      () => 'fulfilled', reason => reason === error ? 'callback-error' : 'wrong-error'
    );
    __finishEvents.push(`${throws}:${gets}:${calls}:${result}:${await closed}`);
  }
})()
"#,
    )
    .expect("Web IDL returned Promise setup");
    assert_eq!(
        vm.eval("JSON.stringify(__finishEvents)")
            .expect("Web IDL returned Promise result"),
        r#"["false:1:1:fulfilled:true","true:1:0:callback-error:true"]"#
    );
}
