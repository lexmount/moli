use super::*;

fn stream_test_vm() -> StandaloneScriptVmHarness {
    new_storage_test_vm("https://stream-runtime.test/")
}

#[test]
fn stream_constructor_dictionaries_follow_webidl_order_and_callback_conversion() {
    let mut vm = stream_test_vm();

    let result = vm
        .eval(
            r#"
JSON.stringify((() => {
  const reads = [];
  const calls = [];
  const receivers = [];
  const dictionary = (prefix, keys, values) => {
    const object = {};
    for (const key of keys) {
      Object.defineProperty(object, key, {
        get() {
          reads.push(`${prefix}.${key}`);
          return values[key];
        }
      });
    }
    return object;
  };
  const callback = (label, body = () => {}) => new Proxy(
    function(...args) {
      receivers.push(this === objects[label]);
      body(...args);
    },
    {
      apply(target, receiver, args) {
        calls.push(label);
        return Reflect.apply(target, receiver, args);
      }
    }
  );
  const objects = {};

  objects.readable = dictionary(
    "readable",
    ["autoAllocateChunkSize", "cancel", "pull", "start", "type"],
    {
      autoAllocateChunkSize: undefined,
      cancel: undefined,
      pull: undefined,
      start: callback("readable", controller => controller.close()),
      type: undefined
    }
  );
  const readableStrategy = dictionary(
    "readableStrategy",
    ["highWaterMark", "size"],
    { highWaterMark: 1, size: undefined }
  );
  new ReadableStream(objects.readable, readableStrategy);

  objects.writable = dictionary(
    "writable",
    ["abort", "close", "start", "type", "write"],
    {
      abort: undefined,
      close: undefined,
      start: callback("writable"),
      type: undefined,
      write: undefined
    }
  );
  const writableStrategy = dictionary(
    "writableStrategy",
    ["highWaterMark", "size"],
    { highWaterMark: 1, size: undefined }
  );
  new WritableStream(objects.writable, writableStrategy);

  objects.transform = dictionary(
    "transform",
    ["cancel", "flush", "readableType", "start", "transform", "writableType"],
    {
      cancel: undefined,
      flush: undefined,
      readableType: undefined,
      start: callback("transform"),
      transform: undefined,
      writableType: undefined
    }
  );
  const transformWritableStrategy = dictionary(
    "transformWritableStrategy",
    ["highWaterMark", "size"],
    { highWaterMark: 1, size: undefined }
  );
  const transformReadableStrategy = dictionary(
    "transformReadableStrategy",
    ["highWaterMark", "size"],
    { highWaterMark: 0, size: undefined }
  );
  new TransformStream(
    objects.transform,
    transformWritableStrategy,
    transformReadableStrategy
  );

  const marker = new Error("dictionary getter");
  const abruptReads = [];
  const abrupt = {};
  for (const key of ["abort", "close", "start", "type", "write"]) {
    Object.defineProperty(abrupt, key, {
      get() {
        abruptReads.push(key);
        if (key === "start") {
          throw marker;
        }
        return undefined;
      }
    });
  }
  let abruptIdentity = false;
  try {
    new WritableStream(abrupt, {
      get highWaterMark() {
        abruptReads.push("strategy");
        return 1;
      }
    });
  } catch (error) {
    abruptIdentity = error === marker;
  }

  let nullMemberError = "none";
  try {
    new WritableStream({ write: null });
  } catch (error) {
    nullMemberError = error && error.name;
  }

  const revoked = Proxy.revocable(function() {}, {});
  revoked.revoke();
  let revokedAccepted = true;
  try {
    new WritableStream({ write: revoked.proxy });
  } catch {
    revokedAccepted = false;
  }

  return {
    reads,
    calls,
    receivers,
    abruptReads,
    abruptIdentity,
    nullMemberError,
    revokedAccepted
  };
})())
"#,
        )
        .expect("Streams Web IDL dictionary conversion should evaluate");

    assert_eq!(
        result,
        r#"{"reads":["readableStrategy.highWaterMark","readableStrategy.size","readable.autoAllocateChunkSize","readable.cancel","readable.pull","readable.start","readable.type","writableStrategy.highWaterMark","writableStrategy.size","writable.abort","writable.close","writable.start","writable.type","writable.write","transformWritableStrategy.highWaterMark","transformWritableStrategy.size","transformReadableStrategy.highWaterMark","transformReadableStrategy.size","transform.cancel","transform.flush","transform.readableType","transform.start","transform.transform","transform.writableType"],"calls":["readable","writable","transform"],"receivers":[true,true,true],"abruptReads":["strategy","abort","close","start"],"abruptIdentity":true,"nullMemberError":"TypeError","revokedAccepted":true}"#
    );
}

#[test]
fn stream_webidl_algorithms_preserve_receivers_controllers_and_promise_results() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__streamWebIdlFacts = [];
  globalThis.__streamWebIdlEvents = [];
  const facts = globalThis.__streamWebIdlFacts;
  const events = globalThis.__streamWebIdlEvents;

  let sourceController;
  const source = {
    start(controller) {
      sourceController = controller;
      controller.enqueue("readable");
      controller.close();
    }
  };
  const readable = new ReadableStream(source, {
    highWaterMark: 3,
    size: new Proxy(
      function(chunk) {
        "use strict";
        facts.push({
          name: "size",
          receiver: this === undefined,
          chunk,
          argumentCount: arguments.length
        });
        return {
          valueOf() {
            events.push("size:valueOf");
            return 2;
          }
        };
      },
      {
        apply(target, receiver, args) {
          events.push(`size:apply:${receiver === undefined}`);
          return Reflect.apply(target, receiver, args);
        }
      }
    )
  });
  readable.getReader().read().then(result => {
    events.push(`readable:${result.value}`);
  });

  let sinkController;
  const sink = {
    start(controller) {
      sinkController = controller;
      facts.push({
        name: "sink-start",
        receiver: this === sink,
        argumentCount: arguments.length
      });
    },
    write(chunk, controller) {
      facts.push({
        name: "sink-write",
        receiver: this === sink,
        controller: controller === sinkController,
        chunk,
        argumentCount: arguments.length
      });
      return {
        then(resolve) {
          events.push("sink:thenable");
          resolve();
        }
      };
    }
  };
  new WritableStream(sink).getWriter().write("writable").then(() => {
    events.push("sink:fulfilled");
  });

  let transformController;
  const transformer = {
    start(controller) {
      transformController = controller;
      facts.push({
        name: "transform-start",
        receiver: this === transformer,
        argumentCount: arguments.length
      });
    },
    transform(chunk, controller) {
      facts.push({
        name: "transform",
        receiver: this === transformer,
        controller: controller === transformController,
        chunk,
        argumentCount: arguments.length
      });
      controller.enqueue(`${chunk}!`);
      return {
        then(resolve) {
          events.push("transform:thenable");
          resolve();
        }
      };
    }
  };
  const transformed = new TransformStream(
    transformer,
    undefined,
    { highWaterMark: 1 }
  );
  transformed.readable.getReader().read().then(result => {
    events.push(`transform:read:${result.value}`);
  });
  transformed.writable.getWriter().write("chunk").then(() => {
    events.push("transform:fulfilled");
  });

  const marker = new Error("sink write marker");
  new WritableStream({
    write() {
      throw marker;
    }
  }).getWriter().write("throw").then(
    () => events.push("throw:fulfilled"),
    error => events.push(`throw:${error === marker}`)
  );
})()
"#,
    )
    .expect("Streams Web IDL callback semantics should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__streamWebIdlEvents)")
            .expect("Streams Web IDL callback promises should drain");
        if events.contains("transform:fulfilled")
            && events.contains("sink:fulfilled")
            && events.contains("throw:true")
        {
            break;
        }
    }

    let result = vm
        .eval(
            r#"
JSON.stringify({
  facts: globalThis.__streamWebIdlFacts,
  observations: {
    sizeProxyAppliedWithUndefinedReceiver:
      __streamWebIdlEvents.includes("size:apply:true"),
    sizeResultConvertedToNumber:
      __streamWebIdlEvents.includes("size:valueOf"),
    readableChunkDelivered:
      __streamWebIdlEvents.includes("readable:readable"),
    transformedChunkDelivered:
      __streamWebIdlEvents.includes("transform:read:chunk!"),
    thrownReasonPreserved:
      __streamWebIdlEvents.includes("throw:true"),
    sinkThenableBeforeFulfilled:
      __streamWebIdlEvents.indexOf("sink:thenable") <
      __streamWebIdlEvents.indexOf("sink:fulfilled"),
    transformThenableBeforeFulfilled:
      __streamWebIdlEvents.indexOf("transform:thenable") <
      __streamWebIdlEvents.indexOf("transform:fulfilled")
  }
})
"#,
        )
        .expect("Streams Web IDL callback results should evaluate");

    assert_eq!(
        result,
        r#"{"facts":[{"name":"size","receiver":true,"chunk":"readable","argumentCount":1},{"name":"sink-start","receiver":true,"argumentCount":1},{"name":"transform-start","receiver":true,"argumentCount":1},{"name":"sink-write","receiver":true,"controller":true,"chunk":"writable","argumentCount":2},{"name":"transform","receiver":true,"controller":true,"chunk":"chunk","argumentCount":2}],"observations":{"sizeProxyAppliedWithUndefinedReceiver":true,"sizeResultConvertedToNumber":true,"readableChunkDelivered":true,"transformedChunkDelivered":true,"thrownReasonPreserved":true,"sinkThenableBeforeFulfilled":true,"transformThenableBeforeFulfilled":true}}"#
    );
}

#[test]
fn transform_stream_cancel_callback_uses_one_shared_terminal_residence() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__transformCancelCallbackEvents = [];
  const events = globalThis.__transformCancelCallbackEvents;

  const readableReason = new Error("readable reason");
  const readableTransformer = {
    cancel(reason) {
      events.push(`readable:${this === readableTransformer}:${reason === readableReason}`);
      return {
        then(resolve) {
          events.push("readable:thenable");
          resolve();
        }
      };
    }
  };
  new TransformStream(readableTransformer).readable.cancel(readableReason).then(
    () => events.push("readable:fulfilled"),
    error => events.push(`readable:rejected:${error && error.name}`)
  );

  const abortReason = new Error("abort reason");
  const abortTransformer = {
    cancel(reason) {
      events.push(`abort:${this === abortTransformer}:${reason === abortReason}`);
    },
    flush() {
      events.push("abort:unexpected-flush");
    }
  };
  new TransformStream(abortTransformer).writable.abort(abortReason).then(
    () => events.push("abort:fulfilled"),
    error => events.push(`abort:rejected:${error && error.name}`)
  );

  const nestedCancelReason = new Error("nested cancel");
  const nestedAbortReason = new Error("nested abort");
  let nestedCalls = 0;
  let nestedAbortPromise;
  let nested;
  nested = new TransformStream({
    cancel(reason) {
      events.push(`nested:${++nestedCalls}:${reason === nestedCancelReason}`);
      nestedAbortPromise = nested.writable.abort(nestedAbortReason);
    },
    flush() {
      events.push("nested:unexpected-flush");
    }
  });
  const nestedCancelPromise = nested.readable.cancel(nestedCancelReason);
  Promise.allSettled([nestedCancelPromise, nestedAbortPromise]).then(results => {
    events.push(
      `nested:settled:${nestedCalls}:${results[0].reason === nestedAbortReason}:` +
      `${results[1].reason === nestedAbortReason}`
    );
  });
})()
"#,
    )
    .expect("TransformStream cancel callback setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__transformCancelCallbackEvents)")
            .expect("TransformStream cancel callback promises should drain");
        if events.contains("nested:settled") {
            break;
        }
    }

    assert_eq!(
        vm.eval(
            r#"
JSON.stringify({
  readableCallback:
    __transformCancelCallbackEvents.includes("readable:true:true"),
  readableThenableAssimilated:
    __transformCancelCallbackEvents.indexOf("readable:thenable") <
    __transformCancelCallbackEvents.indexOf("readable:fulfilled"),
  abortCallback:
    __transformCancelCallbackEvents.includes("abort:true:true"),
  abortFulfilled:
    __transformCancelCallbackEvents.includes("abort:fulfilled"),
  nestedSharedFinish:
    __transformCancelCallbackEvents.includes("nested:settled:1:true:true"),
  unexpectedFlush:
    __transformCancelCallbackEvents.some(event =>
      event.includes("unexpected-flush")
    )
})
"#
        )
        .expect("TransformStream cancel callback results should evaluate"),
        r#"{"readableCallback":true,"readableThenableAssimilated":true,"abortCallback":true,"abortFulfilled":true,"nestedSharedFinish":true,"unexpectedFlush":false}"#
    );
}

#[test]
fn stream_webidl_callbacks_use_and_retire_with_the_exact_callback_realm() {
    let mut vm = stream_test_vm();
    vm.eval(
        r#"
(() => {
  const root =
    document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__streamCallbackFrame = frame;
})()
"#,
    )
    .expect("Streams callback child Realm setup should evaluate");
    materialize_single_child_default_realm_for_test(&mut vm, "Streams callback child Realm setup");

    vm.eval(
        r#"
(() => {
  const frame = __streamCallbackFrame;
  const child = frame.contentWindow;
  globalThis.__streamCallbackChild = child;
  globalThis.__streamCallbackRealmEvents = [];

  const source = {};
  source.pull = child.Function(
    "source",
    `return function(controller) {
      parent.__streamCallbackRealmEvents.push({
        callbackRealm: globalThis === parent.__streamCallbackChild,
        receiver: this === source,
        controllerRealm:
          Object.getPrototypeOf(controller).constructor ===
          parent.ReadableStreamDefaultController,
        argumentCount: arguments.length
      });
      controller.close();
    };`
  )(source);
  new ReadableStream(source).getReader().read();

  let releaseStart;
  const startGate = new Promise(resolve => {
    releaseStart = resolve;
  });
  const retiredSource = {
    start() {
      return startGate;
    },
    pull: child.Function(
      `parent.__streamCallbackRealmEvents.push("retired-callback-ran");`
    )
  };
  const retiredRead = new ReadableStream(retiredSource).getReader().read();
  retiredRead.then(
    () => __streamCallbackRealmEvents.push("retired:fulfilled"),
    error => __streamCallbackRealmEvents.push(`retired:${error && error.name}`)
  );
  // The implicit start result of the first stream is promise-resolved. Retire
  // the child Realm only after that normal pull has had its start reaction,
  // so this test keeps the successful and retired callback cases independent.
  Promise.resolve().then(() => {
    frame.remove();
    releaseStart();
  });
})()
"#,
    )
    .expect("Streams callback Realm/retirement setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__streamCallbackRealmEvents)")
            .expect("Streams callback Realm/retirement promises should drain");
        if events.contains("retired:TypeError") {
            break;
        }
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__streamCallbackRealmEvents)")
            .expect("Streams callback Realm/retirement results should evaluate"),
        r#"[{"callbackRealm":true,"receiver":true,"controllerRealm":true,"argumentCount":1},"retired:TypeError"]"#
    );
}

#[test]
fn writable_stream_writer_release_lock_rejects_ready_closed_and_operations() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writerReleaseResults = [];
  const writable = new WritableStream();
  const writer = writable.getWriter();
  writer.releaseLock();
  try {
    void writer.desiredSize;
    globalThis.__writerReleaseResults.push("desired:missing");
  } catch (error) {
    globalThis.__writerReleaseResults.push(`desired:${error.name}`);
  }
  writer.closed.then(
    () => globalThis.__writerReleaseResults.push("closed:fulfilled"),
    error => {
      globalThis.__writerReleaseError = error;
      globalThis.__writerReleaseResults.push(`closed:${error.name}`);
    }
  );
  writer.ready.then(
    () => globalThis.__writerReleaseResults.push("ready:fulfilled"),
    error => globalThis.__writerReleaseResults.push(
      `ready:${error.name}:${error === globalThis.__writerReleaseError}`
    )
  );
  writer.write("x").then(
    () => globalThis.__writerReleaseResults.push("write:fulfilled"),
    error => globalThis.__writerReleaseResults.push(`write:${error.name}`)
  );
  writer.close().then(
    () => globalThis.__writerReleaseResults.push("close:fulfilled"),
    error => globalThis.__writerReleaseResults.push(`close:${error.name}`)
  );
  writer.abort().then(
    () => globalThis.__writerReleaseResults.push("abort:fulfilled"),
    error => globalThis.__writerReleaseResults.push(`abort:${error.name}`)
  );
})()
"#,
    )
    .expect("writable writer releaseLock setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("JSON.stringify(globalThis.__writerReleaseResults.sort())")
            .expect("writable writer releaseLock promises should drain");
        if result.contains("write:TypeError") && result.contains("ready:TypeError:true") {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__writerReleaseResults.sort())")
        .expect("writable writer releaseLock results should evaluate");
    assert_eq!(
        result,
        r#"["abort:TypeError","close:TypeError","closed:TypeError","desired:TypeError","ready:TypeError:true","write:TypeError"]"#
    );
}

#[test]
fn writable_stream_writer_desired_size_reflects_closing_and_errored_state() {
    let mut vm = stream_test_vm();

    let result = vm
        .eval(
            r#"
JSON.stringify((() => {
  const closingWriter = new WritableStream().getWriter();
  closingWriter.close();
  const errored = new WritableStream({
    start(controller) {
      controller.error(new Error("boom"));
    }
  }).getWriter();
  return {
    closing: closingWriter.desiredSize,
    errored: errored.desiredSize
  };
})())
"#,
        )
        .expect("writable writer desiredSize states should evaluate");

    assert_eq!(result, r#"{"closing":1,"errored":null}"#);
}

#[test]
fn writable_stream_ready_residence_is_created_for_observable_backpressure() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableReadyResidenceResult = "pending";
  let resolveWrite;
  let reportWriteStarted;
  const writeStarted = new Promise(resolve => { reportWriteStarted = resolve; });
  const writable = new WritableStream({
    write() {
      reportWriteStarted();
      return new Promise(resolve => { resolveWrite = resolve; });
    }
  }, new CountQueuingStrategy({ highWaterMark: 1 }));
  const writer = writable.getWriter();
  const before = writer.ready;
  const write = writer.write("chunk");
  const during1 = writer.ready;
  const during2 = writer.ready;
  let readySettled = false;
  during1.then(() => { readySettled = true; });

  Promise.resolve().then(async () => {
    const pendingBeforeSinkSettlement = !readySettled;
    await writeStarted;
    resolveWrite();
    await Promise.all([write, during1]);
    globalThis.__writableReadyResidenceResult = JSON.stringify({
      replacedResolvedPromise: before !== during1,
      stableWhileBackpressured: during1 === during2,
      pendingBeforeSinkSettlement,
      settledAfterWrite: readySettled,
      desiredSize: writer.desiredSize
    });
  }).catch(error => {
    globalThis.__writableReadyResidenceResult =
      `error:${error && error.name}:${error && error.message}`;
  });
})()
"#,
    )
    .expect("writable ready residence setup should evaluate");

    for _ in 0..16 {
        let result = vm
            .eval("globalThis.__writableReadyResidenceResult")
            .expect("writable ready residence should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__writableReadyResidenceResult")
        .expect("writable ready residence result should evaluate");
    assert_eq!(
        result,
        r#"{"replacedResolvedPromise":true,"stableWhileBackpressured":true,"pendingBeforeSinkSettlement":true,"settledAfterWrite":true,"desiredSize":1}"#
    );
}

#[test]
fn writable_stream_queue_total_uses_incremental_double_arithmetic() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableQueueTotalResult = "pending";
  const failures = [];
  const check = (label, actual, expected) => {
    if (!Object.is(actual, expected)) {
      failures.push({ label, actual, expected });
    }
  };
  const setup = () => new WritableStream({}, {
    size(chunk) {
      return chunk;
    },
    highWaterMark: 0
  }).getWriter();

  (async () => {
    {
      const writer = setup();
      const writes = [writer.write(2), writer.write(Number.MAX_SAFE_INTEGER)];
      check("large/queued", writer.desiredSize, 0 - 2 - Number.MAX_SAFE_INTEGER);
      await Promise.all(writes);
      check("large/settled", writer.desiredSize, 0);
    }

    {
      const writer = setup();
      const writes = [writer.write(1e-16), writer.write(1)];
      check("tiny-clamp/queued", writer.desiredSize, 0 - 1e-16 - 1);
      await Promise.all(writes);
      check("tiny-clamp/settled", writer.desiredSize, 0);
    }

    {
      const writer = setup();
      const writes = [writer.write(1e-16), writer.write(1), writer.write(2e-16)];
      check("tiny-positive/queued", writer.desiredSize, 0 - 1e-16 - 1 - 2e-16);
      await Promise.all(writes);
      check(
        "tiny-positive/settled",
        writer.desiredSize,
        0 - 1e-16 - 1 - 2e-16 + 1e-16 + 1 + 2e-16
      );
    }

    {
      const writer = setup();
      const writes = [writer.write(2e-16), writer.write(1)];
      check("tiny-zero/queued", writer.desiredSize, 0 - 2e-16 - 1);
      await Promise.all(writes);
      check("tiny-zero/settled", writer.desiredSize, 0 - 2e-16 - 1 + 2e-16 + 1);
    }

    globalThis.__writableQueueTotalResult = JSON.stringify(failures);
  })().catch(error => {
    globalThis.__writableQueueTotalResult =
      `error:${error && error.name}:${error && error.message}`;
  });
})()
"#,
    )
    .expect("WritableStream floating-point queue total setup should evaluate");

    for _ in 0..24 {
        let result = vm
            .eval("globalThis.__writableQueueTotalResult")
            .expect("WritableStream floating-point queue total should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__writableQueueTotalResult")
        .expect("WritableStream floating-point queue total result should evaluate");
    assert_eq!(result, "[]");
}

#[test]
fn writable_stream_reentrant_strategy_preserves_nested_write_order() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableReentrantWriteResult = "pending";
  const events = [];
  let writer;
  const writable = new WritableStream({
    write(chunk) {
      events.push(`sink.write:${chunk}`);
    }
  }, {
    size(chunk) {
      events.push(`size:${chunk}`);
      if (chunk > 0) {
        writer.write(chunk - 1).then(
          () => events.push(`writer.write done:${chunk - 1}`),
          error => events.push(`writer.write error:${chunk - 1}:${error && error.name}`)
        );
      }
      return chunk;
    }
  });
  writer = writable.getWriter();
  writer.write(2).then(
    () => {
      events.push("writer.write done:2");
      globalThis.__writableReentrantWriteResult = JSON.stringify(events);
    },
    error => {
      globalThis.__writableReentrantWriteResult =
        `error:${error && error.name}:${error && error.message}`;
    }
  );
})()
"#,
    )
    .expect("WritableStream reentrant write setup should evaluate");

    for _ in 0..24 {
        let result = vm
            .eval("globalThis.__writableReentrantWriteResult")
            .expect("WritableStream reentrant writes should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__writableReentrantWriteResult")
        .expect("WritableStream reentrant write result should evaluate");
    assert_eq!(
        result,
        r#"["size:2","size:1","size:0","sink.write:0","sink.write:1","writer.write done:0","sink.write:2","writer.write done:1","writer.write done:2"]"#
    );
}

#[test]
fn writable_stream_reentrant_strategy_honors_error_close_and_abort() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableReentrantTerminalResult = "pending";
  const error1 = { name: "error1" };

  (async () => {
    const errorCase = await (async () => {
      const order = [];
      const sink = [];
      let controller;
      const writable = new WritableStream({
        start(value) {
          controller = value;
        },
        write() {
          sink.push("write");
        }
      }, {
        size() {
          controller.error(error1);
          return 1;
        }
      });
      const writer = writable.getWriter();
      const ready1 = writer.ready.then(
        () => order.push("ready1"),
        error => order.push(`ready1-error:${error === error1}`)
      );
      const write = writer.write("a").then(
        () => order.push("write-fulfilled"),
        error => order.push(`write:${error === error1}`)
      );
      const ready2 = writer.ready.then(
        () => order.push("ready2-fulfilled"),
        error => order.push(`ready2:${error === error1}`)
      );
      const closed = writer.closed.then(
        () => order.push("closed-fulfilled"),
        error => order.push(`closed:${error === error1}`)
      );
      await Promise.all([ready1, write, ready2, closed]);
      return { order, sink };
    })();

    const closeCase = await (async () => {
      const sink = [];
      let writer;
      let closePromise;
      const writable = new WritableStream({
        write() {
          sink.push("write");
        },
        close() {
          sink.push("close");
        }
      }, {
        size() {
          closePromise = writer.close();
          return 1;
        }
      });
      writer = writable.getWriter();
      const write = writer.write("a").then(
        () => "fulfilled",
        error => error && error.name
      );
      const [writeResult] = await Promise.all([write, closePromise]);
      return { writeResult, sink };
    })();

    const abortCase = await (async () => {
      const sink = [];
      let writer;
      let abortPromise;
      const writable = new WritableStream({
        write() {
          sink.push("write");
        },
        abort(reason) {
          sink.push(`abort:${reason === error1}`);
        }
      }, {
        size() {
          abortPromise = writer.abort(error1);
          return 1;
        }
      });
      writer = writable.getWriter();
      const write = writer.write("a").then(
        () => "fulfilled",
        error => `rejected:${error === error1}`
      );
      const [writeResult] = await Promise.all([write, abortPromise]);
      return { writeResult, sink };
    })();

    globalThis.__writableReentrantTerminalResult = JSON.stringify({
      errorCase,
      closeCase,
      abortCase
    });
  })().catch(error => {
    globalThis.__writableReentrantTerminalResult =
      `error:${error && error.name}:${error && error.message}`;
  });
})()
"#,
    )
    .expect("WritableStream reentrant terminal setup should evaluate");

    for _ in 0..32 {
        let result = vm
            .eval("globalThis.__writableReentrantTerminalResult")
            .expect("WritableStream reentrant terminal operations should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__writableReentrantTerminalResult")
        .expect("WritableStream reentrant terminal result should evaluate");
    assert_eq!(
        result,
        r#"{"errorCase":{"order":["ready1","write:true","ready2:true","closed:true"],"sink":[]},"closeCase":{"writeResult":"TypeError","sink":["close"]},"abortCase":{"writeResult":"rejected:true","sink":["abort:true"]}}"#
    );
}

#[test]
fn writable_stream_reentrant_strategy_release_revalidates_writer_ownership() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableReentrantReleaseResult = "pending";

  (async () => {
    const released = await (async () => {
      const sink = [];
      let writer;
      const writable = new WritableStream({
        write() {
          sink.push("write");
        }
      }, {
        size() {
          writer.releaseLock();
          return 1;
        }
      });
      writer = writable.getWriter();
      const write = writer.write("a").then(
        () => "fulfilled",
        error => error && error.name
      );
      const ready = writer.ready.then(
        () => "fulfilled",
        error => error && error.name
      );
      const closed = writer.closed.then(
        () => "fulfilled",
        error => error && error.name
      );
      const [writeResult, readyResult, closedResult] = await Promise.all([
        write,
        ready,
        closed
      ]);
      return { writeResult, readyResult, closedResult, sink };
    })();

    const reacquired = await (async () => {
      const sink = [];
      let writable;
      let writer1;
      let writer2;
      let write2;
      let close2;
      let closed2;
      const strategy = {
        size(chunk) {
          if (chunk > 0) {
            writer1.releaseLock();
            writer2 = writable.getWriter();
            write2 = writer2.write(0);
            close2 = writer2.close();
            closed2 = writer2.closed;
          }
          return 1;
        }
      };
      writable = new WritableStream({
        write(chunk) {
          sink.push(`write:${chunk}`);
        },
        close() {
          sink.push("close");
        }
      }, strategy);
      writer1 = writable.getWriter();
      const write1 = writer1.write(1).then(
        () => "fulfilled",
        error => error && error.name
      );
      const ready1 = writer1.ready.then(
        () => "fulfilled",
        error => error && error.name
      );
      const closed1 = writer1.closed.then(
        () => "fulfilled",
        error => error && error.name
      );
      const [write1Result, ready1Result, closed1Result, write2Result, close2Result,
        closed2Result] = await Promise.all([
          write1,
          ready1,
          closed1,
          write2.then(() => "fulfilled", error => error && error.name),
          close2.then(() => "fulfilled", error => error && error.name),
          closed2.then(() => "fulfilled", error => error && error.name)
        ]);
      return {
        write1Result,
        ready1Result,
        closed1Result,
        write2Result,
        close2Result,
        closed2Result,
        sink
      };
    })();

    globalThis.__writableReentrantReleaseResult = JSON.stringify({
      released,
      reacquired
    });
  })().catch(error => {
    globalThis.__writableReentrantReleaseResult =
      `error:${error && error.name}:${error && error.message}`;
  });
})()
"#,
    )
    .expect("WritableStream reentrant release setup should evaluate");

    for _ in 0..40 {
        let result = vm
            .eval("globalThis.__writableReentrantReleaseResult")
            .expect("WritableStream reentrant release operations should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__writableReentrantReleaseResult")
        .expect("WritableStream reentrant release result should evaluate");
    assert_eq!(
        result,
        r#"{"released":{"writeResult":"TypeError","readyResult":"TypeError","closedResult":"TypeError","sink":[]},"reacquired":{"write1Result":"TypeError","ready1Result":"TypeError","closed1Result":"TypeError","write2Result":"fulfilled","close2Result":"fulfilled","closed2Result":"fulfilled","sink":["write:0","close"]}}"#
    );
}

#[test]
fn writable_stream_error_with_undefined_reason_rejects_writer_promises() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableUndefinedErrorResults = [];
  const writable = new WritableStream({
    start(controller) {
      controller.error();
    }
  });
  const writer = writable.getWriter();
  globalThis.__writableUndefinedErrorResults.push(
    `desired:${writer.desiredSize === null}`
  );
  writer.ready.then(
    () => globalThis.__writableUndefinedErrorResults.push("ready:fulfilled"),
    error => globalThis.__writableUndefinedErrorResults.push(
      `ready:${error === undefined}`
    )
  );
  writer.closed.then(
    () => globalThis.__writableUndefinedErrorResults.push("closed:fulfilled"),
    error => globalThis.__writableUndefinedErrorResults.push(
      `closed:${error === undefined}`
    )
  );
  writer.write("x").then(
    () => globalThis.__writableUndefinedErrorResults.push("write:fulfilled"),
    error => globalThis.__writableUndefinedErrorResults.push(
      `write:${error === undefined}`
    )
  );
  writer.abort().then(
    () => globalThis.__writableUndefinedErrorResults.push("abort:fulfilled"),
    error => globalThis.__writableUndefinedErrorResults.push(
      `abort:${error === undefined}`
    )
  );
})()
"#,
    )
    .expect("writable undefined error setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("JSON.stringify(globalThis.__writableUndefinedErrorResults.sort())")
            .expect("writable undefined error promises should drain");
        if result.contains("abort:fulfilled") && result.contains("write:true") {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__writableUndefinedErrorResults.sort())")
        .expect("writable undefined error results should evaluate");
    assert_eq!(
        result,
        // The start algorithm has not fulfilled yet, so the stream is still
        // `erroring`, not `errored`. Chromium rejects this pending abort with
        // the stored error (which is undefined here).
        r#"["abort:true","closed:true","desired:true","ready:true","write:true"]"#
    );
}

#[test]
fn writable_stream_abort_during_start_defers_close_rejection_until_abort_settles() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableAbortStartOrder = "pending";
  const error = new Error("abort");
  const events = [];
  const writable = new WritableStream();
  const writer = writable.getWriter();
  const write1 = writer.write(1);
  const abort = writer.abort(error);
  const write2 = writer.write(2);
  const close = writer.close();
  write1.catch(() => events.push("write1"));
  abort.then(() => events.push("abort"));
  write2.catch(() => events.push("write2"));
  close.catch(() => events.push("close"));
  Promise.all([
    write1.catch(() => {}),
    abort,
    write2.catch(() => {}),
    close.catch(() => {})
  ]).then(() => {
    globalThis.__writableAbortStartOrder = JSON.stringify(events);
  });
})()
"#,
    )
    .expect("WritableStream abort-during-start setup should evaluate");

    for _ in 0..16 {
        let result = vm
            .eval("globalThis.__writableAbortStartOrder")
            .expect("WritableStream abort-during-start promises should drain");
        if result != "pending" {
            break;
        }
    }
    let result = vm
        .eval("globalThis.__writableAbortStartOrder")
        .expect("WritableStream abort-during-start order should evaluate");
    assert_eq!(result, r#"["write2","write1","abort","close"]"#);
}

#[test]
fn writable_stream_abort_without_reason_uses_abort_error_for_controller_signal() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableDefaultSignalReason = "pending";
  let controller;
  let notifyWrite;
  const writeCalled = new Promise(resolve => { notifyWrite = resolve; });
  const writable = new WritableStream({
    start(value) { controller = value; },
    write() {
      notifyWrite();
      return new Promise(() => {});
    }
  });
  const writer = writable.getWriter();
  writer.write(1);
  writeCalled.then(() => {
    writer.abort();
    globalThis.__writableDefaultSignalReason = JSON.stringify({
      aborted: controller.signal.aborted,
      domException: controller.signal.reason instanceof DOMException,
      name: controller.signal.reason && controller.signal.reason.name
    });
  });
})()
"#,
    )
    .expect("WritableStream default signal reason setup should evaluate");

    for _ in 0..16 {
        let result = vm
            .eval("globalThis.__writableDefaultSignalReason")
            .expect("WritableStream default signal reason should drain");
        if result != "pending" {
            break;
        }
    }
    let result = vm
        .eval("globalThis.__writableDefaultSignalReason")
        .expect("WritableStream default signal reason should evaluate");
    assert_eq!(
        result,
        r#"{"aborted":true,"domException":true,"name":"AbortError"}"#
    );
}

#[test]
fn writable_stream_size_strategy_error_rejects_closed_and_pipe_to() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableSizeErrorResults = [];
  const sizeError = new RangeError("bad size");
  const direct = new WritableStream({}, {
    highWaterMark: 1,
    size() {
      throw sizeError;
    }
  });
  const writer = direct.getWriter();
  writer.closed.then(
    () => globalThis.__writableSizeErrorResults.push("closed:fulfilled"),
    error => globalThis.__writableSizeErrorResults.push(
      `closed:${error === sizeError}:${error.name}:${writer.desiredSize === null}`
    )
  );
  writer.write("direct").then(
    () => globalThis.__writableSizeErrorResults.push("write:fulfilled"),
    error => globalThis.__writableSizeErrorResults.push(
      `write:${error === sizeError}:${error.name}`
    )
  );

  const readable = new ReadableStream({
    start(controller) {
      controller.enqueue("pipe");
    },
    cancel(reason) {
      globalThis.__writableSizeErrorResults.push(`cancel:${reason === sizeError}`);
    }
  });
  const pipedWritable = new WritableStream({}, {
    highWaterMark: 1,
    size() {
      throw sizeError;
    }
  });
  pipedWritable.getWriter().releaseLock();
  readable.pipeTo(pipedWritable).then(
    () => globalThis.__writableSizeErrorResults.push("pipe:fulfilled"),
    error => globalThis.__writableSizeErrorResults.push(
      `pipe:${error === sizeError}:${readable.locked}:${pipedWritable.locked}`
    )
  );
})()
"#,
    )
    .expect("writable size strategy error setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("JSON.stringify(globalThis.__writableSizeErrorResults.sort())")
            .expect("writable size strategy error promises should drain");
        if result.contains("closed:true:RangeError:true")
            && result.contains("pipe:true:false:false")
        {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__writableSizeErrorResults.sort())")
        .expect("writable size strategy error results should evaluate");
    assert_eq!(
        result,
        r#"["cancel:true","closed:true:RangeError:true","pipe:true:false:false","write:true:RangeError"]"#
    );
}

#[test]
fn writable_stream_abort_rejects_closed_without_calling_method_apply_or_call() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableAbortMethodResults = [];
  function sinkMethod() {}
  sinkMethod.apply = () => globalThis.__writableAbortMethodResults.push("apply");
  sinkMethod.call = () => globalThis.__writableAbortMethodResults.push("call");
  const sink = {
    start: sinkMethod,
    write: sinkMethod,
    close: sinkMethod,
    abort: sinkMethod
  };

  const writer1 = new WritableStream(sink).getWriter();
  writer1.write("a");
  writer1.close();
  writer1.closed.then(
    () => globalThis.__writableAbortMethodResults.push("writer1:closed"),
    error => globalThis.__writableAbortMethodResults.push(`writer1:${error && error.name}`)
  );

  const abortReason = new Error("abort");
  abortReason.name = "abortReason";
  const writer2 = new WritableStream(sink).getWriter();
  writer2.abort(abortReason);
  writer2.closed.then(
    () => globalThis.__writableAbortMethodResults.push("writer2:fulfilled"),
    error => globalThis.__writableAbortMethodResults.push(
      `writer2:${error === abortReason}:${error.name}`
    )
  );

  const writer3 = new WritableStream({
    start: sinkMethod,
    write: sinkMethod,
    close: sinkMethod
  }).getWriter();
  writer3.abort(abortReason);
  writer3.closed.then(
    () => globalThis.__writableAbortMethodResults.push("writer3:fulfilled"),
    error => globalThis.__writableAbortMethodResults.push(
      `writer3:${error === abortReason}:${error.name}`
    )
  );
})()
"#,
    )
    .expect("writable abort method setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("JSON.stringify(globalThis.__writableAbortMethodResults.sort())")
            .expect("writable abort method promises should drain");
        if result.contains("writer1:closed") && result.contains("writer3:true:abortReason") {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__writableAbortMethodResults.sort())")
        .expect("writable abort method results should evaluate");
    assert_eq!(
        result,
        r#"["writer1:closed","writer2:true:abortReason","writer3:true:abortReason"]"#
    );
}

#[test]
fn stream_constructors_preserve_subclass_prototypes() {
    let mut vm = stream_test_vm();

    let result = vm
        .eval(
            r#"
JSON.stringify((() => {
  class ReadableSubclass extends ReadableStream {
    extraFunction() {
      return true;
    }
  }
  class WritableSubclass extends WritableStream {
    extraFunction() {
      return true;
    }
  }
  class TransformSubclass extends TransformStream {
    extraFunction() {
      return true;
    }
  }

  const readable = new ReadableSubclass();
  const writable = new WritableSubclass();
  const transform = new TransformSubclass();
  const readableLocked = Object.getOwnPropertyDescriptor(
    ReadableStream.prototype, "locked"
  ).get;
  const writableLocked = Object.getOwnPropertyDescriptor(
    WritableStream.prototype, "locked"
  ).get;

  return {
    readableInstance:
      readable instanceof ReadableStream && readable instanceof ReadableSubclass,
    readableBrand: readableLocked.call(readable) === readable.locked,
    readableExtra: readable.extraFunction(),
    writableInstance:
      writable instanceof WritableStream && writable instanceof WritableSubclass,
    writableBrand: writableLocked.call(writable) === writable.locked,
    writableExtra: writable.extraFunction(),
    transformInstance:
      transform instanceof TransformStream && transform instanceof TransformSubclass,
    transformExtra: transform.extraFunction()
  };
})())
"#,
        )
        .expect("stream subclass constructor probe should evaluate");

    assert_eq!(
        result,
        r#"{"readableInstance":true,"readableBrand":true,"readableExtra":true,"writableInstance":true,"writableBrand":true,"writableExtra":true,"transformInstance":true,"transformExtra":true}"#
    );
}

#[test]
fn readable_reader_access_transitions_acquire_release_and_reject_locked_cancel() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableAccessResult = "pending";
  const readable = new ReadableStream();
  const before = readable.locked;
  const reader = readable.getReader();
  const acquired = readable.locked;
  const closed = reader.closed.then(
    () => "closed:fulfilled",
    error => `closed:${error && error.name}`
  );
  const cancel = readable.cancel().then(
    () => "cancel:fulfilled",
    error => `cancel:${error && error.name}`
  );
  reader.releaseLock();
  const released = readable.locked;
  reader.releaseLock();
  const releasedAgain = readable.locked;
  const replacement = readable.getReader();
  const reacquired = readable.locked;

  const byteStream = new ReadableStream({ type: "bytes" });
  const byobReader = byteStream.getReader({ mode: "byob" });
  const byobAcquired = byteStream.locked;
  byobReader.releaseLock();

  Promise.all([closed, cancel]).then(values => {
    globalThis.__readableAccessResult = JSON.stringify({
      before,
      acquired,
      released,
      releasedAgain,
      reacquired,
      byobAcquired,
      byobReleased: byteStream.locked,
      values
    });
    replacement.releaseLock();
  });
})()
"#,
    )
    .expect("ReadableStream access transition setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__readableAccessResult")
            .expect("ReadableStream access transition promises should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableAccessResult")
        .expect("ReadableStream access transition result should evaluate");
    assert_eq!(
        result,
        r#"{"before":false,"acquired":true,"released":false,"releasedAgain":false,"reacquired":true,"byobAcquired":true,"byobReleased":false,"values":["closed:TypeError","cancel:TypeError"]}"#
    );
}

#[test]
fn readable_stream_tee_inside_strategy_size_fans_out_current_enqueue() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableTeeResult = "pending";
  function readableStreamToArray(readable) {
    const reader = readable.getReader();
    const chunks = [];
    function pump() {
      return reader.read().then(result => {
        if (result.done) {
          return chunks;
        }
        chunks.push(result.value);
        return pump();
      });
    }
    return pump();
  }

  let controller;
  let branch1;
  let branch2;
  const readable = new ReadableStream({
    start(c) {
      controller = c;
    }
  }, {
    size() {
      [branch1, branch2] = readable.tee();
      return 1;
    }
  });
  controller.enqueue("a");
  const locked = readable.locked;
  controller.close();
  Promise.all([
    readableStreamToArray(branch1),
    readableStreamToArray(branch2)
  ]).then(
    ([first, second]) => {
      globalThis.__readableTeeResult = JSON.stringify({ locked, first, second });
    },
    error => {
      globalThis.__readableTeeResult = `error:${error && error.name}:${error && error.message}`;
    }
  );
})()
"#,
    )
    .expect("ReadableStream tee inside size setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__readableTeeResult")
            .expect("ReadableStream tee inside size should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableTeeResult")
        .expect("ReadableStream tee inside size result should evaluate");
    assert_eq!(result, r#"{"locked":true,"first":["a"],"second":["a"]}"#);
}

#[test]
fn readable_stream_size_reentry_commits_to_current_queue_generation() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableSizeReentryResult = "pending";
  const events = [];
  let controller;
  let reader;
  let readInsideSize;
  let drainInsideSize = false;
  const readable = new ReadableStream({
    start(c) {
      controller = c;
    }
  }, {
    size(chunk) {
      events.push(`size:${chunk}`);
      if (drainInsideSize) {
        drainInsideSize = false;
        readInsideSize = reader.read();
      }
      return 1;
    }
  });

  controller.enqueue("seed");
  reader = readable.getReader();
  drainInsideSize = true;
  controller.enqueue("outer");
  controller.close();

  Promise.all([readInsideSize, reader.read(), reader.read()]).then(
    ([first, second, terminal]) => {
      globalThis.__readableSizeReentryResult = JSON.stringify({
        events,
        first,
        second,
        terminal
      });
    },
    error => {
      globalThis.__readableSizeReentryResult =
        `error:${error && error.name}:${error && error.message}`;
    }
  );
})()
"#,
    )
    .expect("ReadableStream size reentry setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__readableSizeReentryResult")
            .expect("ReadableStream size reentry should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableSizeReentryResult")
        .expect("ReadableStream size reentry result should evaluate");
    assert_eq!(
        result,
        r#"{"events":["size:seed","size:outer"],"first":{"value":"seed","done":false},"second":{"value":"outer","done":false},"terminal":{"done":true}}"#
    );
}

#[test]
fn readable_stream_queue_total_uses_incremental_double_arithmetic() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableQueueTotalResult = "pending";
  const failures = [];
  const check = (label, actual, expected) => {
    if (!Object.is(actual, expected)) {
      failures.push({ label, actual, expected });
    }
  };
  const setup = () => {
    let controller;
    const readable = new ReadableStream({
      start(value) {
        controller = value;
      }
    }, {
      size(chunk) {
        return chunk;
      },
      highWaterMark: 0
    });
    return { controller, reader: readable.getReader() };
  };

  (async () => {
    {
      const { controller, reader } = setup();
      controller.enqueue(2);
      check("large/enqueue-1", controller.desiredSize, 0 - 2);
      controller.enqueue(Number.MAX_SAFE_INTEGER);
      check(
        "large/enqueue-2",
        controller.desiredSize,
        0 - Number.MAX_SAFE_INTEGER - 2
      );
      await reader.read();
      check(
        "large/dequeue-1",
        controller.desiredSize,
        0 - Number.MAX_SAFE_INTEGER - 2 + 2
      );
      await reader.read();
      check("large/dequeue-2", controller.desiredSize, 0);
    }

    {
      const { controller, reader } = setup();
      controller.enqueue(1e-16);
      check("tiny-clamp/enqueue-1", controller.desiredSize, 0 - 1e-16);
      controller.enqueue(1);
      check("tiny-clamp/enqueue-2", controller.desiredSize, 0 - 1e-16 - 1);
      await reader.read();
      check("tiny-clamp/dequeue-1", controller.desiredSize, 0 - 1e-16 - 1 + 1e-16);
      await reader.read();
      check("tiny-clamp/dequeue-2", controller.desiredSize, 0);
    }

    {
      const { controller, reader } = setup();
      controller.enqueue(1e-16);
      check("tiny-positive/enqueue-1", controller.desiredSize, 0 - 1e-16);
      controller.enqueue(1);
      check("tiny-positive/enqueue-2", controller.desiredSize, 0 - 1e-16 - 1);
      controller.enqueue(2e-16);
      check(
        "tiny-positive/enqueue-3",
        controller.desiredSize,
        0 - 1e-16 - 1 - 2e-16
      );
      await reader.read();
      check(
        "tiny-positive/dequeue-1",
        controller.desiredSize,
        0 - 1e-16 - 1 - 2e-16 + 1e-16
      );
      await reader.read();
      check(
        "tiny-positive/dequeue-2",
        controller.desiredSize,
        0 - 1e-16 - 1 - 2e-16 + 1e-16 + 1
      );
      await reader.read();
      check(
        "tiny-positive/dequeue-3",
        controller.desiredSize,
        0 - 1e-16 - 1 - 2e-16 + 1e-16 + 1 + 2e-16
      );
    }

    {
      const { controller, reader } = setup();
      controller.enqueue(2e-16);
      check("tiny-zero/enqueue-1", controller.desiredSize, 0 - 2e-16);
      controller.enqueue(1);
      check("tiny-zero/enqueue-2", controller.desiredSize, 0 - 2e-16 - 1);
      await reader.read();
      check("tiny-zero/dequeue-1", controller.desiredSize, 0 - 2e-16 - 1 + 2e-16);
      await reader.read();
      check("tiny-zero/dequeue-2", controller.desiredSize, 0);
    }

    globalThis.__readableQueueTotalResult = JSON.stringify(failures);
  })().catch(error => {
    globalThis.__readableQueueTotalResult =
      `error:${error && error.name}:${error && error.message}`;
  });
})()
"#,
    )
    .expect("ReadableStream floating-point queue total setup should evaluate");

    for _ in 0..16 {
        let result = vm
            .eval("globalThis.__readableQueueTotalResult")
            .expect("ReadableStream floating-point queue total should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableQueueTotalResult")
        .expect("ReadableStream floating-point queue total result should evaluate");
    assert_eq!(result, "[]");
}

#[test]
fn readable_stream_reentrant_strategies_match_chromium_wpt() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableReentrantWpt = {};
  const results = globalThis.__readableReentrantWpt;
  const error = new Error("error1");
  const spin = async (count = 12) => {
    for (let i = 0; i < count; ++i) await Promise.resolve();
  };
  const collect = async readable => {
    const reader = readable.getReader();
    const chunks = [];
    while (true) {
      const result = await reader.read();
      if (result.done) return chunks;
      chunks.push(result.value);
    }
  };
  const run = (name, body) => {
    results[name] = "pending";
    Promise.resolve().then(body).then(
      value => { results[name] = value; },
      failure => {
        results[name] = `unexpected:${failure && failure.name}:${failure && failure.message}`;
      }
    );
  };

  run("nestedEnqueue", async () => {
    let controller;
    let calls = 0;
    const readable = new ReadableStream({
      start(value) { controller = value; }
    }, {
      size() {
        calls += 1;
        if (calls < 2) controller.enqueue("b");
        return 1;
      }
    });
    controller.enqueue("a");
    controller.close();
    return { chunks: await collect(readable), calls };
  });

  run("closeEmptyQueue", async () => {
    let controller;
    const readable = new ReadableStream({
      start(value) { controller = value; }
    }, {
      size() {
        controller.close();
        return 1;
      }
    });
    controller.enqueue("a");
    return collect(readable);
  });

  run("closeQueued", async () => {
    let controller;
    let calls = 0;
    const readable = new ReadableStream({
      start(value) { controller = value; }
    }, {
      size() {
        calls += 1;
        if (calls === 2) controller.close();
        return 1;
      }
    });
    controller.enqueue("a");
    controller.enqueue("b");
    return collect(readable);
  });

  run("error", async () => {
    let controller;
    const readable = new ReadableStream({
      start(value) { controller = value; }
    }, {
      size() {
        controller.error(error);
        return 1;
      }
    });
    controller.enqueue("a");
    try {
      await readable.getReader().read();
      return "fulfilled";
    } catch (failure) {
      return failure === error ? "same-error" : "other-error";
    }
  });

  run("desiredSize", async () => {
    let controller;
    const observed = [];
    const readable = new ReadableStream({
      start(value) { controller = value; }
    }, {
      size() {
        observed.push(controller.desiredSize);
        return 1;
      },
      highWaterMark: 1
    });
    controller.enqueue("a");
    controller.close();
    return { observed, chunks: await collect(readable) };
  });

  run("cancel", async () => {
    let controller;
    let cancelPromise;
    const observations = [];
    let readable;
    readable = new ReadableStream({
      start(value) { controller = value; },
      cancel(reason) {
        observations.push(reason === error ? "same-reason" : "other-reason");
        try {
          controller.enqueue();
          observations.push("enqueue-fulfilled");
        } catch (failure) {
          observations.push(`enqueue-${failure && failure.name}`);
        }
      }
    }, {
      size() {
        cancelPromise = readable.cancel(error);
        return 1;
      },
      highWaterMark: Infinity
    });
    controller.enqueue("a");
    const reader = readable.getReader();
    await Promise.all([reader.closed, cancelPromise]);
    return observations;
  });

  run("pipeTo", async () => {
    let controller;
    let pipePromise;
    const events = [];
    const writable = new WritableStream({
      write(chunk) { events.push(`write:${chunk}`); },
      close() { events.push("close"); }
    });
    let readable;
    readable = new ReadableStream({
      start(value) { controller = value; }
    }, {
      size() {
        if (!pipePromise) pipePromise = readable.pipeTo(writable);
        return 1;
      },
      highWaterMark: 1
    });
    controller.enqueue("a");
    controller.enqueue("a");
    await spin();
    controller.close();
    await pipePromise;
    return events;
  });

  run("read", async () => {
    let controller;
    let reader;
    let readPromise;
    let readResolved = false;
    let calls = 0;
    const readable = new ReadableStream({
      start(value) { controller = value; }
    }, {
      size() {
        readPromise = reader.read();
        calls += 1;
        return 1;
      },
      highWaterMark: 0
    });
    reader = readable.getReader();
    controller.enqueue("a");
    readPromise.then(() => { readResolved = true; });
    await spin();
    const resolvedBeforeSecondEnqueue = readResolved;
    controller.enqueue("b");
    const first = await readPromise;
    const second = await reader.read();
    return {
      resolvedBeforeSecondEnqueue,
      calls,
      values: [first.value, first.done, second.value, second.done]
    };
  });

  run("getReader", async () => {
    let controller;
    let reader;
    const readable = new ReadableStream({
      start(value) { controller = value; }
    }, {
      size() {
        reader = readable.getReader();
        return 1;
      }
    });
    controller.enqueue("a");
    const result = await reader.read();
    return [result.value, result.done];
  });
})()
"#,
    )
    .expect("ReadableStream reentrant WPT cases should initialize");

    for _ in 0..64 {
        let result = vm
            .eval("JSON.stringify(globalThis.__readableReentrantWpt)")
            .expect("ReadableStream reentrant WPT cases should drain");
        if !result.contains("pending") {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__readableReentrantWpt)")
        .expect("ReadableStream reentrant WPT result should evaluate");
    assert_eq!(
        result,
        r#"{"nestedEnqueue":{"chunks":["b","a"],"calls":2},"closeEmptyQueue":[],"closeQueued":["a","b"],"error":"same-error","desiredSize":{"observed":[1],"chunks":["a"]},"cancel":["same-reason","enqueue-TypeError"],"pipeTo":["write:a","write:a","close"],"read":{"resolvedBeforeSecondEnqueue":false,"calls":1,"values":["b",false,"a",false]},"getReader":["a",false]}"#
    );
}

#[test]
fn transform_stream_reentrant_readable_strategy_matches_chromium_wpt() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__transformReentrantWpt = {};
  const results = globalThis.__transformReentrantWpt;
  const error = new Error("error1");
  const spin = async (count = 16) => {
    for (let i = 0; i < count; ++i) await Promise.resolve();
  };
  const collect = async readable => {
    const reader = readable.getReader();
    const chunks = [];
    while (true) {
      const result = await reader.read();
      if (result.done) return chunks;
      chunks.push(result.value);
    }
  };
  const run = (name, body) => {
    results[name] = "pending";
    Promise.resolve().then(body).then(
      value => { results[name] = value; },
      failure => {
        results[name] = `unexpected:${failure && failure.name}:${failure && failure.message}`;
      }
    );
  };

  run("nestedEnqueue", async () => {
    let controller;
    let calls = 0;
    const transform = new TransformStream({
      start(value) { controller = value; }
    }, undefined, {
      size() {
        calls += 1;
        if (calls < 2) controller.enqueue("b");
        return 1;
      },
      highWaterMark: Infinity
    });
    const writer = transform.writable.getWriter();
    await Promise.all([writer.write("a"), writer.close()]);
    return { chunks: await collect(transform.readable), calls };
  });

  run("terminate", async () => {
    let controller;
    const transform = new TransformStream({
      start(value) { controller = value; }
    }, undefined, {
      size() {
        controller.terminate();
        return 1;
      },
      highWaterMark: Infinity
    });
    await transform.writable.getWriter().write("a");
    return collect(transform.readable);
  });

  run("error", async () => {
    let controller;
    const transform = new TransformStream({
      start(value) { controller = value; }
    }, undefined, {
      size() {
        controller.error(error);
        return 1;
      },
      highWaterMark: Infinity
    });
    const writer = transform.writable.getWriter();
    writer.closed.catch(() => {});
    await writer.write("a");
    try {
      await transform.readable.getReader().read();
      return "fulfilled";
    } catch (failure) {
      return failure === error ? "same-error" : "other-error";
    }
  });

  run("desiredSize", async () => {
    let controller;
    const observed = [];
    const transform = new TransformStream({
      start(value) { controller = value; }
    }, undefined, {
      size() {
        observed.push(controller.desiredSize);
        return 1;
      },
      highWaterMark: 1
    });
    const writer = transform.writable.getWriter();
    await Promise.all([writer.write("a"), writer.close()]);
    return { observed, chunks: await collect(transform.readable) };
  });

  run("cancel", async () => {
    let cancelPromise;
    const transform = new TransformStream({}, undefined, {
      size() {
        cancelPromise = transform.readable.cancel(error);
        return 1;
      },
      highWaterMark: Infinity
    });
    const writer = transform.writable.getWriter();
    const closed = writer.closed.then(
      () => "closed-fulfilled",
      failure => failure === error ? "closed-same-error" : "closed-other-error"
    );
    await writer.write("a");
    await cancelPromise;
    return closed;
  });

  run("pipeTo", async () => {
    let controller;
    let pipePromise;
    const events = [];
    const writable = new WritableStream({
      write(chunk) { events.push(`write:${chunk}`); },
      close() { events.push("close"); }
    });
    const transform = new TransformStream({
      start(value) { controller = value; }
    }, undefined, {
      size() {
        if (!pipePromise) pipePromise = transform.readable.pipeTo(writable);
        return 1;
      },
      highWaterMark: 1
    });
    await spin();
    controller.enqueue("a");
    controller.enqueue("a");
    await spin();
    controller.terminate();
    await pipePromise;
    return events;
  });

  run("read", async () => {
    let controller;
    let reader;
    let readPromise;
    let calls = 0;
    const transform = new TransformStream({
      start(value) { controller = value; }
    }, undefined, {
      size() {
        readPromise = reader.read();
        calls += 1;
        return 1;
      },
      highWaterMark: 0
    });
    reader = transform.readable.getReader();
    const writer = transform.writable.getWriter();
    let writeResolved = false;
    const writePromise = writer.write("b").then(() => { writeResolved = true; });
    await spin();
    const resolvedBeforeEnqueue = writeResolved;
    controller.enqueue("a");
    await spin();
    const result = await readPromise;
    await writePromise;
    return {
      resolvedBeforeEnqueue,
      writeResolved,
      calls,
      value: [result.value, result.done]
    };
  });

  run("nestedWriterWrite", async () => {
    let writer;
    let nestedWrite;
    let calls = 0;
    const transform = new TransformStream({}, undefined, {
      size() {
        calls += 1;
        if (calls < 2) nestedWrite = writer.write("a");
        return 1;
      },
      highWaterMark: Infinity
    });
    writer = transform.writable.getWriter();
    await spin();
    const outerWrite = writer.write("b");
    await Promise.all([nestedWrite, outerWrite, writer.close()]);
    return { chunks: await collect(transform.readable), calls };
  });

  run("syncNestedWriterWrite", async () => {
    let controller;
    let writer;
    let nestedWrite;
    let calls = 0;
    const transform = new TransformStream({
      start(value) { controller = value; }
    }, undefined, {
      size() {
        calls += 1;
        if (calls < 2) nestedWrite = writer.write("a");
        return 1;
      },
      highWaterMark: Infinity
    });
    writer = transform.writable.getWriter();
    await spin();
    controller.enqueue("b");
    await Promise.all([nestedWrite, writer.close()]);
    return { chunks: await collect(transform.readable), calls };
  });

  run("close", async () => {
    let controller;
    let writer;
    let closePromise;
    const transform = new TransformStream({
      start(value) { controller = value; }
    }, undefined, {
      size() {
        closePromise = writer.close();
        return 1;
      },
      highWaterMark: 1
    });
    writer = transform.writable.getWriter();
    const reader = transform.readable.getReader();
    await spin();
    controller.enqueue("a");
    const first = await reader.read();
    const second = await reader.read();
    await closePromise;
    return [first.value, first.done, second.value === undefined, second.done];
  });

  run("abort", async () => {
    let abortPromise;
    let controller;
    const transform = new TransformStream({
      start(value) { controller = value; }
    }, undefined, {
      size() {
        abortPromise = transform.writable.abort(error);
        return 1;
      },
      highWaterMark: 1
    });
    const reader = transform.readable.getReader();
    await spin();
    controller.enqueue("a");
    const first = await reader.read();
    let rejection;
    try {
      await reader.read();
      rejection = "fulfilled";
    } catch (failure) {
      rejection = failure === error ? "same-error" : "other-error";
    }
    await abortPromise;
    return [first.value, first.done, rejection];
  });
})()
"#,
    )
    .expect("TransformStream reentrant WPT cases should initialize");

    for _ in 0..96 {
        let result = vm
            .eval("JSON.stringify(globalThis.__transformReentrantWpt)")
            .expect("TransformStream reentrant WPT cases should drain");
        if !result.contains("pending") {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__transformReentrantWpt)")
        .expect("TransformStream reentrant WPT result should evaluate");
    assert_eq!(
        result,
        r#"{"nestedEnqueue":{"chunks":["b","a"],"calls":2},"terminate":[],"error":"same-error","desiredSize":{"observed":[1],"chunks":["a"]},"cancel":"closed-same-error","pipeTo":["write:a","write:a","close"],"read":{"resolvedBeforeEnqueue":false,"writeResolved":true,"calls":1,"value":["b",false]},"nestedWriterWrite":{"chunks":["b","a"],"calls":2},"syncNestedWriterWrite":{"chunks":["a","b"],"calls":2},"close":["a",false,true,true],"abort":["a",false,"same-error"]}"#
    );
}

#[test]
fn transform_stream_backpressure_matches_chromium_wpt() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__transformBackpressureWpt = "pending";
  const spin = async (count = 16) => {
    for (let i = 0; i < count; ++i) await Promise.resolve();
  };
  (async () => {
    const results = {};

    {
      const events = [];
      let settled = false;
      const transform = new TransformStream({
        transform(chunk, controller) {
          events.push(chunk);
          controller.enqueue(chunk);
        }
      });
      transform.writable.getWriter().write("a").then(
        () => { settled = true; },
        () => { settled = true; }
      );
      await spin();
      results.noReader = { events, settled };
    }

    {
      const events = [];
      const transform = new TransformStream({
        transform(chunk, controller) {
          events.push(chunk);
          controller.enqueue(chunk);
        }
      }, undefined, { highWaterMark: 1 });
      const writer = transform.writable.getWriter();
      let firstSettled = false;
      let secondSettled = false;
      writer.write("a").then(() => { firstSettled = true; });
      writer.write("b").then(() => { secondSettled = true; });
      await spin();
      results.hwmOne = { events, firstSettled, secondSettled };
    }

    {
      const events = [];
      const transform = new TransformStream({
        transform(chunk) { events.push(chunk); }
      }, undefined, { highWaterMark: 1 });
      const writer = transform.writable.getWriter();
      await Promise.all([0, 1, 2, 3].map(chunk => writer.write(chunk)));
      results.discard = events;
    }

    {
      const transform = new TransformStream(undefined, undefined, {
        highWaterMark: 0
      });
      const writer = transform.writable.getWriter();
      const reader = transform.readable.getReader();
      const readPromise = reader.read();
      const writePromise = writer.write("a");
      const read = await readPromise;
      await writePromise;
      results.readBeforeWrite = [read.value, read.done];
    }

    {
      let called = false;
      const transform = new TransformStream({
        transform() { called = true; }
      }, undefined, { highWaterMark: Infinity });
      await spin();
      const writePromise = transform.writable.getWriter().write();
      results.synchronous = called;
      await writePromise;
    }

    {
      const ordering = {};
      for (const highWaterMark of [0, 1, 2, Infinity]) {
        const events = [];
        let index = 0;
        const source = new ReadableStream({
          pull(controller) {
            index += 1;
            events.push(`pull:${index}`);
            if (index <= 3) controller.enqueue(index);
            else controller.close();
          }
        }, { highWaterMark: 1 });
        const output = source.pipeThrough(new TransformStream({
          transform(chunk, controller) {
            events.push(`transform:${chunk}`);
            controller.enqueue(chunk);
          }
        }, { highWaterMark: 1 }, { highWaterMark }));
        const reader = output.getReader();
        while (!(await reader.read()).done) {}
        ordering[String(highWaterMark)] = events;
      }
      results.pipeOrdering = ordering;
    }

    {
      let resolveTransform;
      let transformCalled = false;
      const transform = new TransformStream({
        transform() {
          transformCalled = true;
          return new Promise(resolve => { resolveTransform = resolve; });
        }
      }, undefined, { highWaterMark: Infinity });
      const writer = transform.writable.getWriter();
      await spin();
      const writePromise = writer.write("a");
      const whileBlocked = {
        transformCalled,
        desiredSize: writer.desiredSize
      };
      resolveTransform();
      await writePromise;
      await spin();
      results.blocking = {
        whileBlocked,
        desiredSizeAfter: writer.desiredSize
      };
    }

    globalThis.__transformBackpressureWpt = JSON.stringify(results);
  })().catch(error => {
    globalThis.__transformBackpressureWpt =
      `error:${error && error.name}:${error && error.message}`;
  });
})()
"#,
    )
    .expect("TransformStream backpressure WPT cases should initialize");

    for _ in 0..96 {
        let result = vm
            .eval("globalThis.__transformBackpressureWpt")
            .expect("TransformStream backpressure WPT cases should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__transformBackpressureWpt")
        .expect("TransformStream backpressure WPT result should evaluate");
    assert_eq!(
        result,
        r#"{"noReader":{"events":[],"settled":false},"hwmOne":{"events":["a"],"firstSettled":true,"secondSettled":false},"discard":[0,1,2,3],"readBeforeWrite":["a",false],"synchronous":true,"pipeOrdering":{"0":["pull:1","pull:2","transform:1","pull:3","transform:2","pull:4","transform:3"],"1":["pull:1","pull:2","transform:1","pull:3","transform:2","pull:4","transform:3"],"2":["pull:1","pull:2","transform:1","pull:3","transform:2","pull:4","transform:3"],"Infinity":["pull:1","pull:2","transform:1","pull:3","transform:2","pull:4","transform:3"]},"blocking":{"whileBlocked":{"transformCalled":true,"desiredSize":0},"desiredSizeAfter":1}}"#
    );
}

#[test]
fn readable_stream_queue_writes_ignore_array_prototype_indexed_setters() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableQueuePrototypeResult = "pending";
  let controller;
  const readable = new ReadableStream({
    start(c) {
      controller = c;
    }
  });
  const previous = Object.getOwnPropertyDescriptor(Array.prototype, "0");
  let setterCalls = 0;
  let enqueueError = null;
  Object.defineProperty(Array.prototype, "0", {
    configurable: true,
    set() {
      setterCalls += 1;
    }
  });
  try {
    controller.enqueue("chunk");
  } catch (error) {
    enqueueError = error && error.name;
  } finally {
    if (previous) {
      Object.defineProperty(Array.prototype, "0", previous);
    } else {
      delete Array.prototype[0];
    }
  }
  controller.close();
  readable.getReader().read().then(
    result => {
      globalThis.__readableQueuePrototypeResult = JSON.stringify({
        setterCalls,
        enqueueError,
        value: result.value,
        done: result.done
      });
    },
    error => {
      globalThis.__readableQueuePrototypeResult =
        `error:${error && error.name}:${error && error.message}`;
    }
  );
})()
"#,
    )
    .expect("ReadableStream queue prototype pollution setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__readableQueuePrototypeResult")
            .expect("ReadableStream queue prototype pollution should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableQueuePrototypeResult")
        .expect("ReadableStream queue prototype pollution result should evaluate");
    assert_eq!(
        result,
        r#"{"setterCalls":0,"enqueueError":null,"value":"chunk","done":false}"#
    );
}

#[test]
fn readable_stream_tee_copies_existing_queue_and_close_to_branches() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableTeeExistingResult = "pending";
  function readableStreamToArray(readable) {
    const reader = readable.getReader();
    const chunks = [];
    function pump() {
      return reader.read().then(result => {
        if (result.done) {
          return chunks;
        }
        chunks.push(result.value);
        return pump();
      });
    }
    return pump();
  }

  const readable = new ReadableStream({
    start(controller) {
      controller.enqueue("a");
      controller.enqueue("b");
      controller.close();
    }
  });
  const branches = readable.tee();
  Promise.all([
    readableStreamToArray(branches[0]),
    readableStreamToArray(branches[1])
  ]).then(
    ([first, second]) => {
      globalThis.__readableTeeExistingResult = JSON.stringify({
        locked: readable.locked,
        first,
        second
      });
    },
    error => {
      globalThis.__readableTeeExistingResult =
        `error:${error && error.name}:${error && error.message}`;
    }
  );
})()
"#,
    )
    .expect("ReadableStream tee existing queue setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__readableTeeExistingResult")
            .expect("ReadableStream tee existing queue should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableTeeExistingResult")
        .expect("ReadableStream tee existing queue result should evaluate");
    assert_eq!(
        result,
        r#"{"locked":true,"first":["a","b"],"second":["a","b"]}"#
    );
}

#[test]
fn readable_stream_tee_internal_read_does_not_assimilate_iterator_result_thenables() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const state = globalThis.__teeThenInterception = {
    intercepted: [],
    events: [],
    settled: false,
    rejected: null
  };
  let callCount = 0;
  Object.prototype.then = function(resolve) {
    if (!this.done) state.intercepted.push(this.value);
    const replacement = Object.create(null);
    replacement.done = ++callCount === 3;
    replacement.value = callCount;
    resolve(replacement);
    if (replacement.done) delete Object.prototype.then;
  };

  const source = new ReadableStream({
    start(controller) {
      controller.enqueue("a");
      controller.close();
    }
  });
  const [branch1, branch2] = source.tee();
  branch1.pipeTo(new WritableStream({
    write(chunk) { state.events.push(`write:${chunk}`); },
    close() { state.events.push("close"); }
  })).then(
    () => {
      delete Object.prototype.then;
      branch2.cancel();
      state.settled = true;
    },
    error => {
      delete Object.prototype.then;
      state.rejected = error && error.name;
      state.settled = true;
    }
  );
})()
"#,
    )
    .expect("tee then-interception setup should evaluate");

    for _ in 0..12 {
        let settled = vm
            .eval("globalThis.__teeThenInterception.settled")
            .expect("tee then-interception should drain microtasks");
        if settled == "true" {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__teeThenInterception)")
        .expect("tee then-interception result should evaluate");
    assert_eq!(
        result,
        r#"{"intercepted":[],"events":["write:a","close"],"settled":true,"rejected":null}"#
    );
}

#[test]
fn readable_stream_tee_rejects_locked_and_invalid_receivers() {
    let mut vm = stream_test_vm();

    let result = vm
        .eval(
            r#"
JSON.stringify((() => {
  const results = [];
  const readable = new ReadableStream();
  const reader = readable.getReader();
  try {
    readable.tee();
    results.push("locked:missing");
  } catch (error) {
    results.push(`locked:${error.name}`);
  }
  reader.releaseLock();
  try {
    ReadableStream.prototype.tee.call({});
    results.push("invalid:missing");
  } catch (error) {
    results.push(`invalid:${error.name}`);
  }
  return results;
})())
"#,
        )
        .expect("ReadableStream tee receiver errors should evaluate");

    assert_eq!(result, r#"["locked:TypeError","invalid:TypeError"]"#);
}

#[test]
fn readable_stream_tee_canceling_both_branches_aggregates_reasons() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableTeeCancelResult = "pending";
  const reason1 = new Error("first");
  const reason2 = new Error("second");
  let cancelReason;
  const readable = new ReadableStream({
    cancel(reason) {
      cancelReason = reason;
    }
  });
  const [branch1, branch2] = readable.tee();
  Promise.all([
    branch1.cancel(reason1),
    branch2.cancel(reason2)
  ]).then(
    () => {
      globalThis.__readableTeeCancelResult = JSON.stringify({
        isArray: Array.isArray(cancelReason),
        first: cancelReason && cancelReason[0] === reason1,
        second: cancelReason && cancelReason[1] === reason2
      });
    },
    error => {
      globalThis.__readableTeeCancelResult = `error:${error && error.name}`;
    }
  );
})()
"#,
    )
    .expect("ReadableStream tee cancel aggregation setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__readableTeeCancelResult")
            .expect("ReadableStream tee cancel aggregation should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableTeeCancelResult")
        .expect("ReadableStream tee cancel aggregation result should evaluate");
    assert_eq!(result, r#"{"isArray":true,"first":true,"second":true}"#);
}

#[test]
fn readable_stream_tee_rejects_branch_cancel_when_original_cancel_throws() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableTeeCancelRejectResult = "pending";
  const cancelError = { name: "cancelError" };
  const readable = new ReadableStream({
    cancel() {
      throw cancelError;
    }
  });
  const [branch1, branch2] = readable.tee();
  Promise.all([
    branch1.cancel().then(
      () => "first:fulfilled",
      error => `first:${error === cancelError}:${error && error.name}`
    ),
    branch2.cancel().then(
      () => "second:fulfilled",
      error => `second:${error === cancelError}:${error && error.name}`
    )
  ]).then(results => {
    globalThis.__readableTeeCancelRejectResult = JSON.stringify(results.sort());
  });
})()
"#,
    )
    .expect("ReadableStream tee cancel rejection setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__readableTeeCancelRejectResult")
            .expect("ReadableStream tee cancel rejection should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableTeeCancelRejectResult")
        .expect("ReadableStream tee cancel rejection result should evaluate");
    assert_eq!(
        result,
        r#"["first:true:cancelError","second:true:cancelError"]"#
    );
}

#[test]
fn readable_stream_tee_pulls_once_when_branches_have_capacity() {
    let mut vm = stream_test_vm();

    let initial = vm
        .eval(
            r#"
JSON.stringify((() => {
  globalThis.__readableTeeInitialPullEvents = [];
  const readable = new ReadableStream({
    pull() {
      globalThis.__readableTeeInitialPullEvents.push("pull");
    }
  }, { highWaterMark: 0 });
  readable.tee();
  return globalThis.__readableTeeInitialPullEvents;
})())
"#,
        )
        .expect("ReadableStream tee initial pull should evaluate");
    assert_eq!(initial, "[]");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__readableTeeInitialPullEvents)")
            .expect("ReadableStream tee initial pull should drain start reactions");
        if events != "[]" {
            break;
        }
    }
    let result = vm
        .eval("JSON.stringify(globalThis.__readableTeeInitialPullEvents)")
        .expect("ReadableStream tee initial pull result should evaluate");

    assert_eq!(result, r#"["pull"]"#);
}

#[test]
fn readable_stream_tee_pulls_again_after_both_branch_queues_drain() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableTeePullResult = "pending";
  const events = [];
  let next = 0;
  const readable = new ReadableStream({
    pull(controller) {
      next += 1;
      events.push(`pull:${next}`);
      controller.enqueue(next);
    }
  }, { highWaterMark: 0 });
  const [reader1, reader2] = readable.tee().map(branch => branch.getReader());
  Promise.all([
    reader1.read(),
    reader2.read()
  ]).then(
    results => {
      globalThis.__readableTeePullResult = JSON.stringify({
        events,
        values: results.map(result => result.value)
      });
    },
    error => {
      globalThis.__readableTeePullResult = `error:${error && error.name}:${error && error.message}`;
    }
  );
})()
"#,
    )
    .expect("ReadableStream tee branch pull setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__readableTeePullResult")
            .expect("ReadableStream tee branch pull should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableTeePullResult")
        .expect("ReadableStream tee branch pull result should evaluate");
    assert_eq!(result, r#"{"events":["pull:1","pull:2"],"values":[1,1]}"#);
}

#[test]
fn readable_stream_tee_propagates_original_pull_error_after_branch_demand() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__readableTeePullErrorResult = "pending";
  const pullError = { name: "pullError" };
  const readable = new ReadableStream({
    start(controller) {
      controller.enqueue("a");
      controller.enqueue("b");
    },
    pull() {
      throw pullError;
    }
  });
  const [reader1, reader2] = readable.tee().map(branch => branch.getReader());
  const closed1 = reader1.closed.then(
    () => "closed1:fulfilled",
    error => `closed1:${error === pullError}:${error && error.name}`
  );
  const closed2 = reader2.closed.then(
    () => "closed2:fulfilled",
    error => `closed2:${error === pullError}:${error && error.name}`
  );
  reader1.read().then(first => {
    return reader1.read().then(second => {
      return Promise.all([
        closed1,
        closed2,
        Promise.resolve(`read1:${first.value}:${first.done}`),
        Promise.resolve(`read2:${second.value}:${second.done}`),
        reader2.read().then(
          () => "read3:fulfilled",
          error => `read3:${error === pullError}:${error && error.name}`
        ),
        reader1.read().then(
          () => "read4:fulfilled",
          error => `read4:${error === pullError}:${error && error.name}`
        )
      ]);
    });
  }).then(results => {
    globalThis.__readableTeePullErrorResult = JSON.stringify(results.sort());
  }, error => {
    globalThis.__readableTeePullErrorResult = `error:${error && error.name}:${error && error.message}`;
  });
})()
"#,
    )
    .expect("ReadableStream tee pull error setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__readableTeePullErrorResult")
            .expect("ReadableStream tee pull error should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableTeePullErrorResult")
        .expect("ReadableStream tee pull error result should evaluate");
    assert_eq!(
        result,
        r#"["closed1:true:pullError","closed2:true:pullError","read1:a:false","read2:b:false","read3:true:pullError","read4:true:pullError"]"#
    );
}

#[test]
fn readable_stream_tee_stops_pulling_after_source_errors_with_both_branches_reading() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__teeStopsAfterError = {
    events: [],
    phase: "setup",
    reads: "pending",
    closed: "pending"
  };
  const state = globalThis.__teeStopsAfterError;
  const error = { name: "tee-error" };
  const readable = new ReadableStream({
    start(controller) {
      state.sourceController = controller;
    },
    pull() {
      state.events.push(`pull:${state.phase}`);
    }
  }, { highWaterMark: 0 });
  const [reader1, reader2] = readable.tee().map(branch => branch.getReader());
  state.controller = null;
  state.readable = readable;
  state.error = error;
  state.reader1 = reader1;
  state.reader2 = reader2;
})()
"#,
    )
    .expect("tee terminal pull setup should evaluate");

    for _ in 0..8 {
        if vm
            .eval("globalThis.__teeStopsAfterError.events.length")
            .expect("initial tee pull should drain")
            == "1"
        {
            break;
        }
    }
    vm.eval(
        r#"
(() => {
  const state = globalThis.__teeStopsAfterError;
  state.phase = "enqueue";
  state.sourceController.enqueue("a");
  Promise.all([state.reader1.read(), state.reader2.read()]).then(() => {
    state.reads = "done";
    state.phase = "reads-done";
  });
})()
"#,
    )
    .expect("tee branch reads should start");

    for _ in 0..8 {
        if vm
            .eval("globalThis.__teeStopsAfterError.reads")
            .expect("tee branch reads should drain")
            == "done"
        {
            break;
        }
    }
    vm.eval(
        r#"
(() => {
  const state = globalThis.__teeStopsAfterError;
  Promise.all([
    state.reader1.closed.catch(error => error),
    state.reader2.closed.catch(error => error)
  ]).then(() => { state.closed = "done"; });
  state.phase = "error-call";
  state.sourceController.error(state.error);
  state.phase = "error-returned";
})()
"#,
    )
    .expect("tee source error should evaluate");
    for _ in 0..8 {
        let _ = vm
            .eval("globalThis.__teeStopsAfterError.closed")
            .expect("tee source error should drain");
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__teeStopsAfterError.events)")
            .expect("tee source pull events should evaluate"),
        r#"["pull:setup","pull:enqueue"]"#
    );
}

#[test]
fn writable_stream_release_lock_rejects_ready_before_closed() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__writableReleaseOrdering = [];
  const ws = new WritableStream();
  const writer = ws.getWriter();
  writer.ready.then(() => {
    const writerPromise = writer.write("dummy");
    const readyPromise = writer.ready.catch(() => {
      globalThis.__writableReleaseOrdering.push("ready");
    });
    const closedPromise = writer.closed.catch(() => {
      globalThis.__writableReleaseOrdering.push("closed");
    });
    writer.releaseLock();
    Promise.all([readyPromise, closedPromise]).then(() => {
      globalThis.__writableReleaseOrdering.push(
        `order:${globalThis.__writableReleaseOrdering.join(",")}`
      );
      return Promise.all([writerPromise, ws.abort()]);
    }).then(
      () => globalThis.__writableReleaseOrdering.push("done"),
      error => globalThis.__writableReleaseOrdering.push(`error:${error && error.name}`)
    );
  });
})()
"#,
    )
    .expect("writable release ordering setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("JSON.stringify(globalThis.__writableReleaseOrdering)")
            .expect("writable release ordering promises should drain");
        if result.contains("\"done\"") || result.contains("\"error:") {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__writableReleaseOrdering)")
        .expect("writable release ordering result should evaluate");
    assert_eq!(result, r#"["ready","closed","order:ready,closed","done"]"#);
}

#[test]
fn transform_stream_terminate_then_error_with_queued_chunk_errors_readable() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const error1 = new Error("error1");
  const ts = new TransformStream({
    start(controller) {
      controller.enqueue(0);
      controller.terminate();
      controller.error(error1);
    }
  });
  globalThis.__transformTerminateResults = [];
  ts.writable.abort().then(
    () => globalThis.__transformTerminateResults.push("abort:fulfilled"),
    error => globalThis.__transformTerminateResults.push(`abort:${error.name}`)
  );
  ts.readable.cancel().then(
    () => globalThis.__transformTerminateResults.push("cancel:fulfilled"),
    error => globalThis.__transformTerminateResults.push(`cancel:${error === error1}:${error.message}`)
  );
  ts.readable.getReader().closed.then(
    () => globalThis.__transformTerminateResults.push("closed:fulfilled"),
    error => globalThis.__transformTerminateResults.push(`closed:${error === error1}:${error.message}`)
  );
})()
"#,
    )
    .expect("terminate/error queued setup should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__transformTerminateResults.sort())")
        .expect("terminate/error queued promises should settle");

    assert_eq!(
        result,
        r#"["abort:TypeError","cancel:true:error1","closed:true:error1"]"#
    );
}

#[test]
fn transform_stream_terminate_then_error_without_queued_chunk_keeps_readable_closed() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const error1 = new Error("error1");
  const ts = new TransformStream({
    start(controller) {
      controller.terminate();
      controller.error(error1);
    }
  });
  globalThis.__transformTerminateResults = [];
  ts.writable.abort().then(
    () => globalThis.__transformTerminateResults.push("abort:fulfilled"),
    error => globalThis.__transformTerminateResults.push(`abort:${error.name}`)
  );
  ts.readable.cancel().then(
    () => globalThis.__transformTerminateResults.push("cancel:fulfilled"),
    error => globalThis.__transformTerminateResults.push(`cancel:${error === error1}:${error.message}`)
  );
  ts.readable.getReader().closed.then(
    () => globalThis.__transformTerminateResults.push("closed:fulfilled"),
    error => globalThis.__transformTerminateResults.push(`closed:${error === error1}:${error.message}`)
  );
})()
"#,
    )
    .expect("terminate/error empty setup should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__transformTerminateResults.sort())")
        .expect("terminate/error empty promises should settle");

    assert_eq!(
        result,
        r#"["abort:TypeError","cancel:fulfilled","closed:fulfilled"]"#
    );
}

#[test]
fn transform_stream_terminate_inside_flush_allows_writer_close_to_succeed() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const ts = new TransformStream({
    flush(controller) {
      controller.terminate();
    }
  });
  const writer = ts.writable.getWriter();
  const reader = ts.readable.getReader();
  globalThis.__transformTerminateResults = [];
  writer.close().then(
    () => globalThis.__transformTerminateResults.push("close:fulfilled"),
    error => globalThis.__transformTerminateResults.push(`close:${error.name}`)
  );
  writer.closed.then(
    () => globalThis.__transformTerminateResults.push("writerClosed:fulfilled"),
    error => globalThis.__transformTerminateResults.push(`writerClosed:${error.name}`)
  );
  reader.closed.then(
    () => globalThis.__transformTerminateResults.push("readerClosed:fulfilled"),
    error => globalThis.__transformTerminateResults.push(`readerClosed:${error.name}`)
  );
})()
"#,
    )
    .expect("terminate in flush setup should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__transformTerminateResults.sort())")
        .expect("terminate in flush promises should settle");

    assert_eq!(
        result,
        r#"["close:fulfilled","readerClosed:fulfilled","writerClosed:fulfilled"]"#
    );
}

#[test]
fn transform_stream_writable_abort_errors_readable_with_abort_reason() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const abortReason = new Error("abort reason");
  abortReason.name = "abortReason";
  const ts = new TransformStream();
  const reader = ts.readable.getReader();
  globalThis.__transformAbortResults = [];
  ts.writable.abort(abortReason).then(
    () => globalThis.__transformAbortResults.push("abort:fulfilled"),
    error => globalThis.__transformAbortResults.push(`abort:${error.name}`)
  );
  reader.read().then(
    () => globalThis.__transformAbortResults.push("read:fulfilled"),
    error => globalThis.__transformAbortResults.push(`read:${error === abortReason}:${error.name}`)
  );
})()
"#,
    )
    .expect("writable abort setup should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__transformAbortResults.sort())")
        .expect("writable abort promises should settle");

    assert_eq!(result, r#"["abort:fulfilled","read:true:abortReason"]"#);
}

#[test]
fn transform_stream_writer_abort_reason_wins_over_following_readable_cancel() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const abortReason = new Error("abort reason");
  abortReason.name = "abortReason";
  const cancelReason = new Error("cancel reason");
  cancelReason.name = "cancelReason";
  const ts = new TransformStream();
  const writer = ts.writable.getWriter();
  globalThis.__transformAbortResults = [];
  writer.abort(abortReason).then(
    () => globalThis.__transformAbortResults.push("abort:fulfilled"),
    error => globalThis.__transformAbortResults.push(`abort:${error.name}`)
  );
  ts.readable.cancel(cancelReason).then(
    () => globalThis.__transformAbortResults.push("cancel:fulfilled"),
    error => globalThis.__transformAbortResults.push(`cancel:${error === abortReason}:${error.name}`)
  );
  writer.closed.then(
    () => globalThis.__transformAbortResults.push("closed:fulfilled"),
    error => globalThis.__transformAbortResults.push(`closed:${error === abortReason}:${error.name}`)
  );
})()
"#,
    )
    .expect("writer abort/cancel setup should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__transformAbortResults.sort())")
        .expect("writer abort/cancel promises should settle");

    assert_eq!(
        result,
        r#"["abort:fulfilled","cancel:fulfilled","closed:true:abortReason"]"#
    );
}

#[test]
fn transform_stream_controller_error_after_readable_cancel_preserves_cancel_result() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const controllerReason = new Error("controller reason");
  controllerReason.name = "controllerReason";
  const cancelReason = new Error("cancel reason");
  cancelReason.name = "cancelReason";
  let controller;
  const ts = new TransformStream({
    start(c) {
      controller = c;
    }
  });
  const writer = ts.writable.getWriter();
  globalThis.__transformCancelResults = [];
  ts.readable.cancel(cancelReason).then(
    () => globalThis.__transformCancelResults.push("cancel:fulfilled"),
    error => globalThis.__transformCancelResults.push(`cancel:${error.name}`)
  );
  controller.error(controllerReason);
  writer.closed.then(
    () => globalThis.__transformCancelResults.push("closed:fulfilled"),
    error => globalThis.__transformCancelResults.push(`closed:${error === controllerReason}:${error.name}`)
  );
})()
"#,
    )
    .expect("controller error after cancel setup should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__transformCancelResults.sort())")
        .expect("controller error after cancel promises should settle");

    assert_eq!(
        result,
        r#"["cancel:fulfilled","closed:true:controllerReason"]"#
    );
}

#[test]
fn transform_stream_strategy_size_error_keeps_controller_error_reason() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const controllerReason = new URIError("controller reason");
  const sizeError = new Error("redundant error");
  let controller;
  globalThis.__transformStrategyPriorityEvents = [];
  const strategy = {
    highWaterMark: 1,
    size() {
      controller.error(controllerReason);
      throw sizeError;
    }
  };
  const ts = new TransformStream({
    start(c) {
      controller = c;
    },
    transform(chunk, c) {
      try {
        c.enqueue(chunk);
      } catch (error) {
        globalThis.__transformStrategyPriorityEvents.push(
          `enqueue:${error === sizeError}:${error.message}`
        );
      }
    }
  }, undefined, strategy);
  const writer = ts.writable.getWriter();
  writer.write("chunk").then(
    () => globalThis.__transformStrategyPriorityEvents.push("write:fulfilled"),
    error => globalThis.__transformStrategyPriorityEvents.push(`write:${error.message}`)
  );
  writer.closed.catch(error => {
    globalThis.__transformStrategyPriorityEvents.push(
      `writerClosed:${error === controllerReason}:${error.name}:${error.message}`
    );
  });
  ts.readable.getReader().closed.catch(error => {
    globalThis.__transformStrategyPriorityEvents.push(
      `readableClosed:${error === controllerReason}:${error.name}:${error.message}`
    );
  });
})()
"#,
    )
    .expect("strategy size first error setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("JSON.stringify(globalThis.__transformStrategyPriorityEvents.sort())")
            .expect("strategy size first error promises should settle");
        if result.contains("writerClosed") && result.contains("readableClosed") {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__transformStrategyPriorityEvents.sort())")
        .expect("strategy size first error result should evaluate");
    assert_eq!(
        result,
        r#"["enqueue:false:controller reason","readableClosed:true:URIError:controller reason","write:fulfilled","writerClosed:true:URIError:controller reason"]"#
    );
}

#[test]
fn transform_stream_strategy_size_uncaught_error_rethrows_size_error() {
    let mut vm = stream_test_vm();

    let result = vm
        .eval(
            r#"
(() => {
  const controllerReason = new URIError("controller reason");
  const sizeError = new Error("redundant error");
  let controller;
  try {
    new TransformStream({
      start(c) {
        controller = c;
        c.enqueue("chunk");
      },
      transform() {}
    }, undefined, {
      size() {
        controller.error(controllerReason);
        throw sizeError;
      }
    });
    return "no throw";
  } catch (error) {
    return `${error === sizeError}:${error.name}:${error.message}`;
  }
})()
"#,
        )
        .expect("strategy size uncaught error probe should evaluate");

    assert_eq!(result, "false:URIError:controller reason");
}

#[test]
fn readable_stream_pipe_through_uses_internal_pipe_and_validates_pair() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeThroughInternalEvents = [];
  const originalPipeTo = ReadableStream.prototype.pipeTo;
  ReadableStream.prototype.pipeTo = () => globalThis.__pipeThroughInternalEvents.push("patched-pipeTo");
  try {
    const source = new ReadableStream({
      start(controller) {
        controller.enqueue("a");
        controller.close();
      }
    });
    const readable = new ReadableStream();
    const writable = new WritableStream({
      write(chunk) {
        globalThis.__pipeThroughInternalEvents.push(`write:${chunk}`);
      },
      close() {
        globalThis.__pipeThroughInternalEvents.push("close");
      }
    });
    globalThis.__pipeThroughInternalEvents.push(source.pipeThrough({ readable, writable }) === readable);
  } finally {
    ReadableStream.prototype.pipeTo = originalPipeTo;
  }
})()
"#,
    )
    .expect("pipeThrough internal pipe probe should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeThroughInternalEvents)")
            .expect("pipeThrough internal pipe events should drain microtasks");
        if events.contains("close") {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__pipeThroughInternalEvents)")
        .expect("pipeThrough internal pipe events should settle");
    assert_eq!(result, r#"[true,"write:a","close"]"#);
}

#[test]
fn readable_stream_pipe_through_preserves_pair_getter_errors() {
    let mut vm = stream_test_vm();

    let result = vm
        .eval(
            r#"
(() => {
  const source = new ReadableStream();
  const readableError = new Error("readable threw");
  const writableError = new Error("writable threw");
  const out = [];
  try {
    source.pipeThrough({
      get readable() {
        throw readableError;
      },
      get writable() {
        throw writableError;
      }
    }, {
      get preventAbort() {
        throw new Error("options threw");
      }
    });
  } catch (error) {
    out.push(error === readableError, error.message);
  }
  try {
    source.pipeThrough({
      readable: new ReadableStream(),
      get writable() {
        throw writableError;
      }
    });
  } catch (error) {
    out.push(error === writableError, error.message);
  }
  return JSON.stringify(out);
})()
"#,
        )
        .expect("pipeThrough getter error probe should evaluate");

    assert_eq!(result, r#"[true,"readable threw",true,"writable threw"]"#);
}

#[test]
fn readable_stream_pipe_through_validates_signal_and_locked_writable() {
    let mut vm = stream_test_vm();

    let result = vm
        .eval(
            r#"
(() => {
  const out = [];
  const source = new ReadableStream();
  const pair = { readable: new ReadableStream(), writable: new WritableStream() };
  try {
    source.pipeThrough(pair, { signal: null });
  } catch (error) {
    out.push(error.name);
  }
  const lockedWritable = new WritableStream();
  lockedWritable.getWriter();
  try {
    source.pipeThrough({ readable: new ReadableStream(), writable: lockedWritable });
  } catch (error) {
    out.push(error.name);
  }
  const controller = new AbortController();
  out.push(source.pipeThrough({
    readable: new ReadableStream(),
    writable: new WritableStream()
  }, { signal: controller.signal }) instanceof ReadableStream);
  return JSON.stringify(out);
})()
"#,
        )
        .expect("pipeThrough validation probe should evaluate");

    assert_eq!(result, r#"["TypeError","TypeError",true]"#);
}

#[test]
fn readable_stream_pipe_through_rejects_forged_stream_prototypes() {
    let mut vm = stream_test_vm();

    let result = vm
        .eval(
            r#"
(() => {
  const out = [];
  const forgedReadable = Object.create(ReadableStream.prototype);
  const forgedWritable = Object.create(WritableStream.prototype);
  const validPair = () => ({ readable: new ReadableStream(), writable: new WritableStream() });
  try {
    ReadableStream.prototype.pipeThrough.call(forgedReadable, validPair());
    out.push("forged-this accepted");
  } catch (error) {
    out.push(error.name);
  }
  let writableGetterCalled = false;
  try {
    new ReadableStream().pipeThrough({
      readable: forgedReadable,
      get writable() {
        writableGetterCalled = true;
        return new WritableStream();
      }
    });
    out.push("forged-readable accepted");
  } catch (error) {
    out.push(error.name, writableGetterCalled);
  }
  let readableGetterCalled = false;
  try {
    new ReadableStream().pipeThrough({
      get readable() {
        readableGetterCalled = true;
        return new ReadableStream();
      },
      writable: forgedWritable
    });
    out.push("forged-writable accepted");
  } catch (error) {
    out.push(readableGetterCalled, error.name);
  }
  return JSON.stringify(out);
})()
"#,
        )
        .expect("pipeThrough strict brand probe should evaluate");

    assert_eq!(
        result,
        r#"["TypeError","TypeError",false,true,"TypeError"]"#
    );
}

#[test]
fn readable_stream_pipe_through_duck_typed_pass_through_drains_async_pulls() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeThroughDuckTypedEvents = [];
  function duckTypedPassThroughTransform() {
    let enqueueInReadable;
    let closeReadable;
    return {
      writable: new WritableStream({
        write(chunk) {
          globalThis.__pipeThroughDuckTypedEvents.push(`write:${chunk}`);
          enqueueInReadable(chunk);
        },
        close() {
          globalThis.__pipeThroughDuckTypedEvents.push("close-writable");
          closeReadable();
        }
      }),
      readable: new ReadableStream({
        start(controller) {
          enqueueInReadable = controller.enqueue.bind(controller);
          closeReadable = controller.close.bind(controller);
        }
      })
    };
  }
  function readableStreamToArray(readable) {
    const reader = readable.getReader();
    const chunks = [];
    function pump() {
      return reader.read().then(result => {
        if (result.done) {
          return chunks;
        }
        chunks.push(result.value);
        return pump();
      });
    }
    return pump();
  }
  let current = 0;
  const source = new ReadableStream({
    start() {
      return Promise.resolve();
    },
    pull(controller) {
      current += 1;
      globalThis.__pipeThroughDuckTypedEvents.push(`pull:${current}`);
      if (current <= 5) {
        controller.enqueue(current);
      } else {
        controller.close();
      }
      return Promise.resolve();
    }
  });
  globalThis.__pipeThroughDuckTypedResult = "pending";
  const readableEnd = source.pipeThrough(duckTypedPassThroughTransform());
  readableStreamToArray(readableEnd).then(
    chunks => {
      globalThis.__pipeThroughDuckTypedEvents.push("resolved");
      globalThis.__pipeThroughDuckTypedResult = JSON.stringify(chunks);
    },
    error => {
      globalThis.__pipeThroughDuckTypedEvents.push("rejected");
      globalThis.__pipeThroughDuckTypedResult = `error:${error && error.message}`;
    }
  );
})()
"#,
    )
    .expect("duck-typed pass-through pipeThrough setup should evaluate");

    for _ in 0..16 {
        let result = vm
            .eval("globalThis.__pipeThroughDuckTypedResult")
            .expect("duck-typed pass-through pipeThrough should drain microtasks");
        if result != "pending" {
            break;
        }
    }
    let result = vm
        .eval(
            "JSON.stringify({ result: globalThis.__pipeThroughDuckTypedResult, events: globalThis.__pipeThroughDuckTypedEvents })",
        )
        .expect("duck-typed pass-through pipeThrough should settle");

    assert_eq!(
        result,
        r#"{"result":"[1,2,3,4,5]","events":["pull:1","write:1","pull:2","pull:3","write:2","pull:4","write:3","pull:5","write:4","pull:6","write:5","close-writable","resolved"]}"#
    );
}

#[test]
fn readable_pipe_delivers_enqueued_chunk_while_pull_promise_remains_pending() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pendingPullPipeResult = {
    direct: [],
    transform: [],
    directSettled: false,
    transformSettled: false
  };
  const result = globalThis.__pendingPullPipeResult;
  const never = () => new Promise(() => {});

  const directSource = new ReadableStream({
    pull(controller) {
      result.direct.push("pull");
      controller.enqueue("direct");
      return never();
    }
  }, { highWaterMark: 0 });
  directSource.pipeTo(new WritableStream({
    write(chunk) { result.direct.push(`write:${chunk}`); }
  })).then(
    () => { result.directSettled = true; },
    () => { result.directSettled = true; }
  );

  const transformSource = new ReadableStream({
    pull(controller) {
      result.transform.push("pull");
      controller.enqueue("transform");
      return never();
    }
  }, { highWaterMark: 0 });
  const output = transformSource.pipeThrough(new TransformStream({
    transform(chunk, controller) {
      result.transform.push(`transform:${chunk}`);
      controller.enqueue(chunk);
    }
  }));
  output.getReader().read().then(
    value => {
      result.transform.push(`read:${value.value}:${value.done}`);
      result.transformSettled = true;
    },
    () => { result.transformSettled = true; }
  );
})()
"#,
    )
    .expect("pending pull pipe cases should initialize");

    for _ in 0..16 {
        let result = vm
            .eval("JSON.stringify(globalThis.__pendingPullPipeResult)")
            .expect("pending pull pipe cases should drain");
        if result.contains("write:direct") && result.contains("read:transform:false") {
            break;
        }
    }

    let result = vm
        .eval("JSON.stringify(globalThis.__pendingPullPipeResult)")
        .expect("pending pull pipe result should evaluate");
    assert_eq!(
        result,
        r#"{"direct":["pull","write:direct"],"transform":["pull","transform:transform","read:transform:false"],"directSettled":false,"transformSettled":true}"#
    );
}

#[test]
fn readable_stream_pending_read_pulls_after_async_start() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__asyncStartReadEvents = [];
  let pulls = 0;
  const stream = new ReadableStream({
    start() {
      globalThis.__asyncStartReadEvents.push("start");
      return Promise.resolve();
    },
    pull(controller) {
      pulls += 1;
      globalThis.__asyncStartReadEvents.push(`pull:${pulls}`);
      controller.enqueue(`chunk:${pulls}`);
    }
  }, { highWaterMark: 0 });
  stream.getReader().read().then(
    result => globalThis.__asyncStartReadEvents.push(`${result.value}:${result.done}`),
    error => globalThis.__asyncStartReadEvents.push(`error:${error && error.name}`)
  );
})()
"#,
    )
    .expect("async start pending read setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__asyncStartReadEvents)")
            .expect("async start pending read should drain microtasks");
        if events.contains("chunk:1:false") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__asyncStartReadEvents)")
        .expect("async start pending read should settle");
    assert_eq!(events, r#"["start","pull:1","chunk:1:false"]"#);
}

#[test]
fn readable_stream_pipe_through_handles_writable_error_and_prevent_abort() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeThroughErrorEvents = [];
  const sinkError = new Error("sink");
  const source = new ReadableStream({
    pull() {
      globalThis.__pipeThroughErrorEvents.push("pull");
    },
    cancel() {
      globalThis.__pipeThroughErrorEvents.push("cancel");
    }
  }, { highWaterMark: 0 });
  const erroredWritable = new WritableStream({
    start(controller) {
      globalThis.__pipeThroughErrorEvents.push(`writable-error:${typeof controller.error}`);
      controller.error(sinkError);
    }
  });
  source.pipeThrough(
    { writable: erroredWritable, readable: new ReadableStream() },
    { preventCancel: true }
  );

  const erroredSource = new ReadableStream({
    start(controller) {
      controller.error();
    }
  });
  const notAbortedWritable = new WritableStream({
    close() {
      globalThis.__pipeThroughErrorEvents.push("close");
    },
    abort() {
      globalThis.__pipeThroughErrorEvents.push("abort");
    }
  });
  erroredSource
    .pipeTo(notAbortedWritable, { preventAbort: true })
    .catch(error => {
      globalThis.__pipeThroughErrorEvents.push(`prevent-abort:${String(error)}`);
    });
})()
"#,
    )
    .expect("pipeThrough error/preventAbort setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeThroughErrorEvents)")
            .expect("pipeThrough error/preventAbort should drain microtasks");
        if events.contains("prevent-abort:undefined") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeThroughErrorEvents)")
        .expect("pipeThrough error/preventAbort events should settle");
    assert_eq!(
        events,
        r#"["writable-error:function","prevent-abort:undefined"]"#
    );
}

#[test]
fn readable_stream_pipe_through_prevent_close_keeps_writable_open() {
    let mut vm = stream_test_vm();

    let result = vm
        .eval(
            r#"
(() => {
  const events = [];
  const source = new ReadableStream({
    start(controller) {
      controller.close();
    }
  });
  const pair = {
    readable: new ReadableStream(),
    writable: new WritableStream({
      close() {
        events.push("close");
      }
    })
  };
  source.pipeThrough(pair, { preventClose: true });
  return JSON.stringify(events);
})()
"#,
        )
        .expect("pipeThrough preventClose probe should evaluate");

    assert_eq!(result, r#"[]"#);
}

#[test]
fn readable_stream_pipe_to_validates_brand_and_locked_as_rejected_promises() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToValidationEvents = [];
  function observe(label, thunk) {
    try {
      const result = thunk();
      globalThis.__pipeToValidationEvents.push(`${label}:returned:${result instanceof Promise}`);
      result.then(
        () => globalThis.__pipeToValidationEvents.push(`${label}:fulfilled`),
        error => globalThis.__pipeToValidationEvents.push(`${label}:rejected:${error && error.name}`)
      );
    } catch (error) {
      globalThis.__pipeToValidationEvents.push(`${label}:threw:${error && error.name}`);
    }
  }

  observe("bad-this", () => ReadableStream.prototype.pipeTo.call({}, new WritableStream()));
  observe("bad-dest", () => new ReadableStream().pipeTo({}));

  const lockedReadable = new ReadableStream();
  lockedReadable.getReader();
  observe("source-locked", () => lockedReadable.pipeTo(new WritableStream()));

  const lockedWritable = new WritableStream();
  lockedWritable.getWriter();
  observe("dest-locked", () => new ReadableStream().pipeTo(lockedWritable));
})()
"#,
    )
    .expect("pipeTo validation setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToValidationEvents)")
            .expect("pipeTo validation should drain promise reactions");
        if events.matches(":rejected:TypeError").count() == 4 {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToValidationEvents.sort())")
        .expect("pipeTo validation events should settle");
    assert_eq!(
        events,
        r#"["bad-dest:rejected:TypeError","bad-dest:returned:true","bad-this:rejected:TypeError","bad-this:returned:true","dest-locked:rejected:TypeError","dest-locked:returned:true","source-locked:rejected:TypeError","source-locked:returned:true"]"#
    );
}

#[test]
fn readable_stream_pipe_options_reject_non_object_values_with_entrypoint_semantics() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeOptionConversionEvents = [];
  function observePipeTo(label, options) {
    try {
      const promise = new ReadableStream().pipeTo(new WritableStream(), options);
      globalThis.__pipeOptionConversionEvents.push(`${label}:returned:${promise instanceof Promise}`);
      promise.then(
        () => globalThis.__pipeOptionConversionEvents.push(`${label}:fulfilled`),
        error => globalThis.__pipeOptionConversionEvents.push(`${label}:rejected:${error && error.name}`)
      );
    } catch (error) {
      globalThis.__pipeOptionConversionEvents.push(`${label}:threw:${error && error.name}`);
    }
  }
  function observePipeThrough(label, options) {
    try {
      new ReadableStream().pipeThrough(new TransformStream(), options);
      globalThis.__pipeOptionConversionEvents.push(`${label}:returned`);
    } catch (error) {
      globalThis.__pipeOptionConversionEvents.push(`${label}:threw:${error && error.name}`);
    }
  }

  observePipeTo("pipeTo-symbol", Symbol("options"));
  observePipeTo("pipeTo-boolean", true);
  observePipeThrough("pipeThrough-symbol", Symbol("options"));
  observePipeThrough("pipeThrough-boolean", true);
})()
"#,
    )
    .expect("pipe options conversion setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeOptionConversionEvents)")
            .expect("pipe options conversion promise reactions should drain");
        if events.matches(":rejected:TypeError").count() == 2 {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeOptionConversionEvents.sort())")
        .expect("pipe options conversion events should settle");
    assert_eq!(
        events,
        r#"["pipeThrough-boolean:threw:TypeError","pipeThrough-symbol:threw:TypeError","pipeTo-boolean:rejected:TypeError","pipeTo-boolean:returned:true","pipeTo-symbol:rejected:TypeError","pipeTo-symbol:returned:true"]"#
    );
}

#[test]
fn readable_stream_pipe_to_aborts_on_signal_and_honors_prevent_flags() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeAbortEvents = [];

  const reason = new Error("pipe abort");
  const controller = new AbortController();
  const readable = new ReadableStream({
    pull() {
      globalThis.__pipeAbortEvents.push("pull");
    },
    cancel(value) {
      globalThis.__pipeAbortEvents.push(`cancel:${value === reason}`);
    }
  }, { highWaterMark: 0 });
  const writable = new WritableStream({
    abort(value) {
      globalThis.__pipeAbortEvents.push(`abort:${value === reason}`);
    }
  });
  readable.pipeTo(writable, { signal: controller.signal }).then(
    () => globalThis.__pipeAbortEvents.push("pipe:fulfilled"),
    value => globalThis.__pipeAbortEvents.push(
      `pipe:${value === reason}:${readable.locked}:${writable.locked}`
    )
  );
  globalThis.__pipeAbortEvents.push(`locked:${readable.locked}:${writable.locked}`);
  Promise.resolve().then(() => {
    globalThis.__pipeAbortEvents.push("abort-call");
    controller.abort(reason);
    globalThis.__pipeAbortEvents.push(`after:${readable.locked}:${writable.locked}`);
  });

  const preventedReason = new Error("prevented abort");
  const preventedController = new AbortController();
  const preventedReadable = new ReadableStream({
    cancel() {
      globalThis.__pipeAbortEvents.push("prevented-cancel");
    }
  }, { highWaterMark: 0 });
  const preventedWritable = new WritableStream({
    abort() {
      globalThis.__pipeAbortEvents.push("prevented-abort");
    }
  });
  preventedController.abort(preventedReason);
  preventedReadable.pipeTo(preventedWritable, {
    signal: preventedController.signal,
    preventAbort: true,
    preventCancel: true
  }).then(
    () => globalThis.__pipeAbortEvents.push("prevented-pipe:fulfilled"),
    value => globalThis.__pipeAbortEvents.push(
      `prevented-pipe:${value === preventedReason}:` +
      `${preventedReadable.locked}:${preventedWritable.locked}`
    )
  );
  globalThis.__pipeAbortEvents.push(
    `prevented-locked:${preventedReadable.locked}:${preventedWritable.locked}`
  );
})()
"#,
    )
    .expect("pipeTo abort signal setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeAbortEvents)")
            .expect("pipeTo abort signal events should drain");
        if events.contains("pipe:true:false:false")
            && events.contains("prevented-pipe:true:false:false")
        {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeAbortEvents)")
        .expect("pipeTo abort signal events should settle");
    assert_eq!(
        events,
        // Chromium retains both locks until the abort/cancel action promises
        // settle; abort dispatch itself only claims shutdown. An already
        // aborted signal follows the same asynchronous shutdown path even
        // when both propagation actions are prevented.
        r#"["locked:true:true","prevented-locked:true:true","pull","abort-call","after:true:true","abort:true","cancel:true","prevented-pipe:true:false:false","pipe:true:false:false"]"#
    );
}

#[test]
fn readable_stream_pipe_abort_actions_own_all_rejections() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeAbortRejectionEvents = [];
  addEventListener("unhandledrejection", event => {
    globalThis.__pipeAbortRejectionEvents.push(`unhandled:${event.reason.message}`);
    event.preventDefault();
  });

  function abortedController() {
    const controller = new AbortController();
    controller.abort(new Error("signal"));
    return controller;
  }

  const sourceError = new Error("source-only");
  new ReadableStream({
    cancel() { return Promise.reject(sourceError); }
  }, { highWaterMark: 0 })
    .pipeTo(new WritableStream(), { signal: abortedController().signal })
    .catch(error => globalThis.__pipeAbortRejectionEvents.push(`source:${error.message}`));

  const sinkError = new Error("sink-only");
  new ReadableStream({}, { highWaterMark: 0 })
    .pipeTo(new WritableStream({
      abort() { return Promise.reject(sinkError); }
    }), { signal: abortedController().signal })
    .catch(error => globalThis.__pipeAbortRejectionEvents.push(`sink:${error.message}`));

  const sourceSecondError = new Error("source-second");
  const sinkFirstError = new Error("sink-first");
  new ReadableStream({
    cancel() { return Promise.reject(sourceSecondError); }
  }, { highWaterMark: 0 })
    .pipeTo(new WritableStream({
      abort() { return Promise.reject(sinkFirstError); }
    }), { signal: abortedController().signal })
    .catch(error => globalThis.__pipeAbortRejectionEvents.push(`both:${error.message}`));

  const pendingPullError = new Error("pending-pull");
  let rejectCancel;
  const cancelPromise = new Promise((_, reject) => { rejectCancel = reject; });
  const pendingPullController = new AbortController();
  new ReadableStream({
    async pull() { await cancelPromise; },
    cancel(reason) { rejectCancel(reason); }
  })
    .pipeTo(new WritableStream(), { signal: pendingPullController.signal })
    .catch(error => globalThis.__pipeAbortRejectionEvents.push(`pending:${error.message}`));
  Promise.resolve().then(() => pendingPullController.abort(pendingPullError));
})()
"#,
    )
    .expect("pipe abort rejection ownership setup should evaluate");

    for _ in 0..8 {
        vm.eval("0")
            .expect("pipe abort rejection ownership checkpoint should evaluate");
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeAbortRejectionEvents.sort())")
        .expect("pipe abort rejection ownership events should evaluate");
    assert_eq!(
        events,
        r#"["both:sink-first","pending:pending-pull","sink:sink-only","source:source-only"]"#
    );
}

#[test]
fn readable_stream_pipe_source_error_during_write_waits_for_published_write() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeReentrantSourceErrorEvents = [];
  const sourceError = new Error("source failed during write");
  let readableController;
  const readable = new ReadableStream({
    start(controller) { readableController = controller; }
  });
  let resolveWrite;
  const writable = new WritableStream({
    write() {
      globalThis.__pipeReentrantSourceErrorEvents.push("write");
      readableController.error(sourceError);
      return new Promise(resolve => { resolveWrite = resolve; });
    }
  });
  readable.pipeTo(writable, { preventAbort: true }).then(
    () => globalThis.__pipeReentrantSourceErrorEvents.push("pipe:fulfilled"),
    error => globalThis.__pipeReentrantSourceErrorEvents.push(
      `pipe:${error === sourceError}:${readable.locked}:${writable.locked}`
    )
  );
  readableController.enqueue("chunk");
  globalThis.__resolvePipeReentrantWrite = () => {
    globalThis.__pipeReentrantSourceErrorEvents.push("write:resolve");
    resolveWrite();
  };
})()
"#,
    )
    .expect("reentrant source error pipe setup should evaluate");

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__pipeReentrantSourceErrorEvents)")
            .expect("reentrant source error must wait for write publication"),
        r#"["write"]"#
    );

    vm.eval("globalThis.__resolvePipeReentrantWrite()")
        .expect("reentrant source error write should resolve");
    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeReentrantSourceErrorEvents)")
            .expect("reentrant source error pipe should drain");
        if events.contains("pipe:true:false:false") {
            break;
        }
    }
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__pipeReentrantSourceErrorEvents)")
            .expect("reentrant source error pipe events should settle"),
        r#"["write","write:resolve","pipe:true:false:false"]"#
    );
}

#[test]
fn readable_stream_pipe_prefers_an_already_erroring_destination_reason() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeErroringDestinationResult = "pending";
  const sourceError = new Error("source");
  const destinationError = new Error("destination");
  const events = [];
  const source = new ReadableStream({
    start(controller) { controller.error(sourceError); }
  });
  const destination = new WritableStream({
    start(controller) { controller.error(destinationError); },
    abort() { events.push("abort"); }
  });
  source.pipeTo(destination).then(
    () => { globalThis.__pipeErroringDestinationResult = "fulfilled"; },
    error => {
      globalThis.__pipeErroringDestinationResult = JSON.stringify({
        destinationError: error === destinationError,
        events,
        sourceLocked: source.locked,
        destinationLocked: destination.locked
      });
    }
  );
})()
"#,
    )
    .expect("erroring destination pipe setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__pipeErroringDestinationResult")
            .expect("erroring destination pipe should drain");
        if result != "pending" {
            break;
        }
    }
    assert_eq!(
        vm.eval("globalThis.__pipeErroringDestinationResult")
            .expect("erroring destination pipe should settle"),
        r#"{"destinationError":true,"events":[],"sourceLocked":false,"destinationLocked":false}"#
    );
}

#[test]
fn readable_stream_pipe_to_signal_uses_internal_abort_algorithm() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeAbortAlgorithmEvents = [];
  const originalAdd = AbortSignal.prototype.addEventListener;
  const originalRemove = AbortSignal.prototype.removeEventListener;
  AbortSignal.prototype.addEventListener = function() {
    globalThis.__pipeAbortAlgorithmEvents.push("patched-add");
    throw new Error("patched addEventListener should not be used");
  };
  AbortSignal.prototype.removeEventListener = function() {
    globalThis.__pipeAbortAlgorithmEvents.push("patched-remove");
    throw new Error("patched removeEventListener should not be used");
  };

  const reason = new Error("algorithm abort");
  const controller = new AbortController();
  const readable = new ReadableStream({
    pull() {
      globalThis.__pipeAbortAlgorithmEvents.push("pull");
    },
    cancel(value) {
      globalThis.__pipeAbortAlgorithmEvents.push(`cancel:${value === reason}`);
    }
  }, { highWaterMark: 0 });
  const writable = new WritableStream({
    abort(value) {
      globalThis.__pipeAbortAlgorithmEvents.push(`abort:${value === reason}`);
    }
  });
  const pipePromise = readable.pipeTo(writable, { signal: controller.signal });
  controller.signal.dispatchEvent(new Event("abort"));
  globalThis.__pipeAbortAlgorithmEvents.push(
    `after-dispatch:${readable.locked}:${writable.locked}`
  );
  Promise.resolve().then(() => controller.abort(reason));
  pipePromise.then(
    () => globalThis.__pipeAbortAlgorithmEvents.push("pipe:fulfilled"),
    value => {
      globalThis.__pipeAbortAlgorithmEvents.push(
        `pipe:${value === reason}:${readable.locked}:${writable.locked}`
      );
      AbortSignal.prototype.addEventListener = originalAdd;
      AbortSignal.prototype.removeEventListener = originalRemove;
    }
  );
})()
"#,
    )
    .expect("pipeTo internal abort algorithm setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeAbortAlgorithmEvents)")
            .expect("pipeTo internal abort algorithm events should drain");
        if events.contains("pipe:true:false:false") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeAbortAlgorithmEvents)")
        .expect("pipeTo internal abort algorithm events should settle");
    assert_eq!(
        events,
        r#"["after-dispatch:true:true","pull","abort:true","cancel:true","pipe:true:false:false"]"#
    );
}

#[test]
fn readable_stream_pipe_to_locks_both_streams_and_releases_on_close() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToLockEvents = [];
  let controller;
  const readable = new ReadableStream({
    start(c) {
      controller = c;
    }
  }, { highWaterMark: 0 });
  const writable = new WritableStream({
    close() {
      globalThis.__pipeToLockEvents.push("close");
    }
  });
  readable.pipeTo(writable).then(
    () => globalThis.__pipeToLockEvents.push(`settled:${readable.locked}:${writable.locked}`),
    error => globalThis.__pipeToLockEvents.push(`rejected:${error && error.name}`)
  );
  globalThis.__pipeToLockEvents.push(`locked:${readable.locked}:${writable.locked}`);
  controller.close();
})()
"#,
    )
    .expect("pipeTo lock setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToLockEvents)")
            .expect("pipeTo lock events should drain microtasks");
        if events.contains("settled:false:false") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToLockEvents)")
        .expect("pipeTo lock events should settle");
    assert_eq!(
        events,
        r#"["locked:true:true","close","settled:false:false"]"#
    );
}

#[test]
fn readable_stream_pipe_to_does_not_write_synchronously_from_enqueue() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToAsyncEnqueueEvents = [];
  let controller;
  const readable = new ReadableStream({
    start(c) {
      controller = c;
    }
  }, { highWaterMark: 0 });
  const writable = new WritableStream({
    write(chunk) {
      globalThis.__pipeToAsyncEnqueueEvents.push(`write:${chunk}`);
    },
    close() {
      globalThis.__pipeToAsyncEnqueueEvents.push("close");
    }
  });
  readable.pipeTo(writable).then(
    () => globalThis.__pipeToAsyncEnqueueEvents.push("pipe:fulfilled"),
    error => globalThis.__pipeToAsyncEnqueueEvents.push(`pipe:rejected:${error && error.name}`)
  );
  Promise.resolve().then(() => {
    globalThis.__pipeToAsyncEnqueueEvents.push("before-enqueue");
    controller.enqueue("chunk");
    globalThis.__pipeToAsyncEnqueueEvents.push(
      `after-enqueue:${globalThis.__pipeToAsyncEnqueueEvents.includes("write:chunk")}`
    );
    controller.close();
  });
})()
"#,
    )
    .expect("pipeTo async enqueue setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToAsyncEnqueueEvents)")
            .expect("pipeTo async enqueue events should drain microtasks");
        if events.contains("pipe:fulfilled") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToAsyncEnqueueEvents)")
        .expect("pipeTo async enqueue events should settle");
    assert_eq!(
        events,
        r#"["before-enqueue","after-enqueue:false","write:chunk","close","pipe:fulfilled"]"#
    );
}

#[test]
fn readable_stream_pipe_to_waits_when_destination_desired_size_is_zero() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToBackpressureEvents = [];
  const error = new Error("backpressured");
  let writableController;
  const readable = new ReadableStream({
    start(controller) {
      controller.enqueue("a");
      controller.enqueue("b");
      controller.close();
    },
    pull() {
      globalThis.__pipeToBackpressureEvents.push("pull");
    },
    cancel(reason) {
      globalThis.__pipeToBackpressureEvents.push(`cancel:${reason === error}`);
    }
  });
  const writable = new WritableStream({
    start(controller) {
      writableController = controller;
    },
    write(chunk) {
      globalThis.__pipeToBackpressureEvents.push(`write:${chunk}`);
    }
  }, new CountQueuingStrategy({ highWaterMark: 0 }));
  readable.pipeTo(writable, { preventCancel: true }).then(
    () => globalThis.__pipeToBackpressureEvents.push("pipe:fulfilled"),
    reason => globalThis.__pipeToBackpressureEvents.push(
      `pipe:${reason === error}:${readable.locked}:${writable.locked}`
    )
  );
  Promise.resolve().then(() => {
    globalThis.__pipeToBackpressureEvents.push(
      `before-error:${globalThis.__pipeToBackpressureEvents.includes("pull")}:` +
      `${globalThis.__pipeToBackpressureEvents.includes("write:a")}`
    );
    writableController.error(error);
  });
})()
"#,
    )
    .expect("pipeTo backpressure setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToBackpressureEvents)")
            .expect("pipeTo backpressure events should drain microtasks");
        if events.contains("pipe:true:false:false") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToBackpressureEvents)")
        .expect("pipeTo backpressure events should settle");
    assert_eq!(
        events,
        r#"["before-error:false:false","pipe:true:false:false"]"#
    );
}

#[test]
fn writable_stream_serializes_sink_writes_behind_in_flight_write() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__sinkWriteQueueEvents = [];
  let resolveFirst;
  const writable = new WritableStream({
    write(chunk) {
      globalThis.__sinkWriteQueueEvents.push(`write:${chunk}`);
      if (chunk === "a") {
        return new Promise(resolve => {
          resolveFirst = resolve;
          globalThis.__resolveFirstSinkWrite = resolve;
        });
      }
      return undefined;
    }
  }, new CountQueuingStrategy({ highWaterMark: 3 }));
  const writer = writable.getWriter();
  writer.write("a").then(() => globalThis.__sinkWriteQueueEvents.push("first:fulfilled"));
  writer.write("b").then(() => globalThis.__sinkWriteQueueEvents.push("second:fulfilled"));
  Promise.resolve().then(() => {
    globalThis.__sinkWriteQueueEvents.push(
      `checkpoint:${globalThis.__sinkWriteQueueEvents.includes("write:b")}`
    );
  });
})()
"#,
    )
    .expect("writable sink write queue setup should evaluate");

    let events = vm
        .eval("JSON.stringify(globalThis.__sinkWriteQueueEvents)")
        .expect("writable sink write queue should reach checkpoint");
    assert_eq!(events, r#"["write:a","checkpoint:false"]"#);

    vm.eval("globalThis.__resolveFirstSinkWrite()")
        .expect("first sink write should resolve");
    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__sinkWriteQueueEvents)")
            .expect("writable sink write queue should drain");
        if events.contains("second:fulfilled") {
            break;
        }
    }

    let result = vm
        .eval(
            r#"
(() => {
  const events = globalThis.__sinkWriteQueueEvents;
  return JSON.stringify({
    writeA: events.indexOf("write:a"),
    writeB: events.indexOf("write:b"),
    first: events.indexOf("first:fulfilled"),
    second: events.indexOf("second:fulfilled")
  });
})()
"#,
        )
        .expect("writable sink write queue final state should evaluate");
    assert_eq!(result, r#"{"writeA":0,"writeB":2,"first":3,"second":4}"#);
}

#[test]
fn writable_stream_queues_sink_close_behind_in_flight_writes() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__sinkCloseQueueEvents = [];
  let resolveFirst;
  let resolveClose;
  const writable = new WritableStream({
    write(chunk) {
      globalThis.__sinkCloseQueueEvents.push(`write:${chunk}`);
      if (chunk === "a") {
        return new Promise(resolve => {
          resolveFirst = resolve;
          globalThis.__resolveFirstSinkCloseWrite = resolve;
        });
      }
      return undefined;
    },
    close() {
      globalThis.__sinkCloseQueueEvents.push("sink:close");
      return new Promise(resolve => {
        resolveClose = resolve;
      });
    }
  }, new CountQueuingStrategy({ highWaterMark: 3 }));
  const writer = writable.getWriter();
  writer.write("a").then(() => globalThis.__sinkCloseQueueEvents.push("first:fulfilled"));
  writer.write("b").then(() => globalThis.__sinkCloseQueueEvents.push("second:fulfilled"));
  writer.close().then(() => globalThis.__sinkCloseQueueEvents.push("close:fulfilled"));
  globalThis.__resolveQueuedSinkClose = () => resolveClose();
})()
"#,
    )
    .expect("writable sink close queue setup should evaluate");

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__sinkCloseQueueEvents)")
            .expect("queued sink close should wait behind the first write"),
        r#"["write:a"]"#
    );

    vm.eval("globalThis.__resolveFirstSinkCloseWrite()")
        .expect("first queued sink write should resolve");
    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__sinkCloseQueueEvents)")
            .expect("queued sink close should advance");
        if events.contains("sink:close") {
            break;
        }
    }
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__sinkCloseQueueEvents)")
            .expect("queued sink close should start after both writes"),
        r#"["write:a","write:b","first:fulfilled","sink:close","second:fulfilled"]"#
    );

    vm.eval("globalThis.__resolveQueuedSinkClose()")
        .expect("queued sink close should resolve");
    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__sinkCloseQueueEvents)")
            .expect("queued sink close fulfillment should drain");
        if events.contains("close:fulfilled") {
            break;
        }
    }
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__sinkCloseQueueEvents)")
            .expect("queued sink close should settle"),
        r#"["write:a","write:b","first:fulfilled","sink:close","second:fulfilled","close:fulfilled"]"#
    );
}

#[test]
fn writable_sink_queue_resumes_large_burst_after_in_flight_write() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const writeTotal = 1026;
  globalThis.__sinkBurstWrites = 0;
  globalThis.__sinkBurstSettled = 0;
  globalThis.__sinkBurstResult = "pending";
  const writable = new WritableStream({
    write(chunk) {
      globalThis.__sinkBurstWrites += 1;
      if (chunk === 0) {
        return new Promise(resolve => {
          globalThis.__resolveSinkBurstFirst = resolve;
        });
      }
      return undefined;
    }
  }, new CountQueuingStrategy({ highWaterMark: 2048 }));
  const writer = writable.getWriter();
  const writes = Array.from({ length: writeTotal }, (_, chunk) =>
    writer.write(chunk).then(() => {
      globalThis.__sinkBurstSettled += 1;
    })
  );
  Promise.all(writes).then(
    () => { globalThis.__sinkBurstResult = "fulfilled"; },
    error => { globalThis.__sinkBurstResult = `rejected:${error && error.name}`; }
  );
})()
"#,
    )
    .expect("writable sink burst setup should evaluate");

    vm.eval("globalThis.__resolveSinkBurstFirst()")
        .expect("first writable sink write should resolve");
    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__sinkBurstResult")
            .expect("writable sink continuation should drain microtasks");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval(
            "JSON.stringify({ result: globalThis.__sinkBurstResult, writes: globalThis.__sinkBurstWrites, settled: globalThis.__sinkBurstSettled })",
        )
        .expect("writable sink continuation result should evaluate");
    assert_eq!(
        result,
        r#"{"result":"fulfilled","writes":1026,"settled":1026}"#
    );
}

#[test]
fn transform_writable_queue_resumes_large_burst_after_in_flight_write() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const writeTotal = 1026;
  let resolveFirst;
  globalThis.__transformBurstRuns = 0;
  globalThis.__transformBurstSettled = 0;
  globalThis.__transformBurstResult = "pending";
  const transform = new TransformStream({
    transform(chunk, controller) {
      globalThis.__transformBurstRuns += 1;
      controller.enqueue(chunk);
      if (chunk === 0) {
        return new Promise(resolve => {
          globalThis.__resolveTransformBurstFirst = resolve;
        });
      }
      return undefined;
    }
  }, { highWaterMark: 2048 }, { highWaterMark: 2048 });
  const writer = transform.writable.getWriter();
  const writes = Array.from({ length: writeTotal }, (_, chunk) =>
    writer.write(chunk).then(() => {
      globalThis.__transformBurstSettled += 1;
    })
  );
  Promise.all(writes).then(
    () => { globalThis.__transformBurstResult = "fulfilled"; },
    error => {
      globalThis.__transformBurstResult = `rejected:${error && error.name}`;
    }
  );
})()
"#,
    )
    .expect("transform writable burst setup should evaluate");

    vm.eval("globalThis.__resolveTransformBurstFirst()")
        .expect("first transform write should resolve");
    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__transformBurstResult")
            .expect("transform writable continuation should drain microtasks");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval(
            "JSON.stringify({ result: globalThis.__transformBurstResult, runs: globalThis.__transformBurstRuns, settled: globalThis.__transformBurstSettled })",
        )
        .expect("transform writable continuation result should evaluate");
    assert_eq!(
        result,
        r#"{"result":"fulfilled","runs":1026,"settled":1026}"#
    );
}

#[test]
fn readable_pipe_drain_completes_large_ready_burst() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  const chunkTotal = 1026;
  globalThis.__pipeBurstWrites = 0;
  globalThis.__pipeBurstResult = "pending";
  const readable = new ReadableStream({
    start(controller) {
      for (let chunk = 0; chunk < chunkTotal; chunk += 1) {
        controller.enqueue(chunk);
      }
      controller.close();
    }
  }, new CountQueuingStrategy({ highWaterMark: 2048 }));
  const writable = new WritableStream({
    write() {
      globalThis.__pipeBurstWrites += 1;
    }
  }, new CountQueuingStrategy({ highWaterMark: 2048 }));
  readable.pipeTo(writable).then(
    () => { globalThis.__pipeBurstResult = "fulfilled"; },
    error => { globalThis.__pipeBurstResult = `rejected:${error && error.name}`; }
  );
})()
"#,
    )
    .expect("readable pipe burst setup should evaluate");

    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__pipeBurstResult")
            .expect("readable pipe continuation should drain microtasks");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval(
            "JSON.stringify({ result: globalThis.__pipeBurstResult, writes: globalThis.__pipeBurstWrites })",
        )
        .expect("readable pipe continuation result should evaluate");
    assert_eq!(result, r#"{"result":"fulfilled","writes":1026}"#);
}

#[test]
fn readable_stream_pipe_to_queues_writes_until_previous_sink_write_finishes() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToQueuedWriteEvents = [];
  const unread = ["b", "c", "d"];
  let resolveFirst;
  const readable = new ReadableStream({
    pull(controller) {
      const chunk = unread.shift();
      globalThis.__pipeToQueuedWriteEvents.push(`pull:${chunk}`);
      controller.enqueue(chunk);
      if (unread.length === 0) {
        controller.close();
      }
    }
  }, new CountQueuingStrategy({ highWaterMark: 0 }));
  const writable = new WritableStream({
    write(chunk) {
      globalThis.__pipeToQueuedWriteEvents.push(`write:${chunk}`);
      if (!resolveFirst) {
        return new Promise(resolve => {
          resolveFirst = resolve;
          globalThis.__resolvePipeFirstSinkWrite = resolve;
        });
      }
      return undefined;
    },
    close() {
      globalThis.__pipeToQueuedWriteEvents.push("close");
    }
  }, new CountQueuingStrategy({ highWaterMark: 3 }));
  const writer = writable.getWriter();
  writer.write("a");
  globalThis.__pipeToQueuedWriteEvents.push(`desired:${writer.desiredSize}`);
  writer.releaseLock();
  readable.pipeTo(writable).then(
    () => globalThis.__pipeToQueuedWriteEvents.push("pipe:fulfilled"),
    error => globalThis.__pipeToQueuedWriteEvents.push(`pipe:rejected:${error && error.name}`)
  );
  globalThis.__pipeToQueuedWriteUnread = () => unread.length;
})()
"#,
    )
    .expect("pipeTo queued write setup should evaluate");

    for _ in 0..8 {
        let unread = vm
            .eval("String(globalThis.__pipeToQueuedWriteUnread())")
            .expect("pipeTo queued write unread length should evaluate");
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToQueuedWriteEvents)")
            .expect("pipeTo queued write events should evaluate");
        if unread == "1" && !events.contains("write:b") {
            break;
        }
    }

    let before = vm
        .eval(
            r#"
JSON.stringify({
  unread: globalThis.__pipeToQueuedWriteUnread(),
  wroteB: globalThis.__pipeToQueuedWriteEvents.includes("write:b"),
  events: globalThis.__pipeToQueuedWriteEvents
})
"#,
        )
        .expect("pipeTo queued write pre-resolution state should evaluate");
    assert_eq!(
        before,
        r#"{"unread":1,"wroteB":false,"events":["desired:2","pull:b","write:a","pull:c"]}"#
    );

    vm.eval("globalThis.__resolvePipeFirstSinkWrite()")
        .expect("first pipe sink write should resolve");
    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToQueuedWriteEvents)")
            .expect("pipeTo queued write final events should drain");
        if events.contains("pipe:fulfilled") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToQueuedWriteEvents)")
        .expect("pipeTo queued write final events should evaluate");
    // PipeTo owns its first read before the sink's existing write settles.
    // Once that write releases backpressure, dequeuing each source-prefill
    // chunk opens the next HWM slot before the following sink write.
    assert_eq!(
        events,
        r#"["desired:2","pull:b","write:a","pull:c","write:b","pull:d","write:c","write:d","close","pipe:fulfilled"]"#
    );
}

#[test]
fn readable_stream_pipe_to_waits_for_async_destination_close() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToAsyncCloseEvents = [];
  let resolveClose;
  const readable = new ReadableStream({
    start(controller) {
      controller.close();
    }
  });
  const writable = new WritableStream({
    close() {
      globalThis.__pipeToAsyncCloseEvents.push("close:start");
      return new Promise(resolve => {
        resolveClose = () => {
          globalThis.__pipeToAsyncCloseEvents.push("close:resolve");
          resolve();
        };
      });
    }
  });
  readable.pipeTo(writable).then(
    () => globalThis.__pipeToAsyncCloseEvents.push(
      `pipe:fulfilled:${readable.locked}:${writable.locked}`
    ),
    error => globalThis.__pipeToAsyncCloseEvents.push(`pipe:rejected:${error && error.name}`)
  );
  globalThis.__pipeToAsyncCloseEvents.push(`locked:${readable.locked}:${writable.locked}`);
  globalThis.__resolvePipeToAsyncClose = () => resolveClose();
})()
"#,
    )
    .expect("pipeTo async close setup should evaluate");

    let before = vm
        .eval(
            r#"
JSON.stringify({
  events: globalThis.__pipeToAsyncCloseEvents,
  settled: globalThis.__pipeToAsyncCloseEvents.some(event => event.startsWith("pipe:"))
})
"#,
        )
        .expect("pipeTo async close pending state should evaluate");
    assert_eq!(
        before,
        r#"{"events":["locked:true:true","close:start"],"settled":false}"#
    );

    vm.eval("globalThis.__resolvePipeToAsyncClose()")
        .expect("pipeTo async close should resolve");
    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToAsyncCloseEvents)")
            .expect("pipeTo async close should drain microtasks");
        if events.contains("pipe:fulfilled:false:false") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToAsyncCloseEvents)")
        .expect("pipeTo async close events should settle");
    assert_eq!(
        events,
        r#"["locked:true:true","close:start","close:resolve","pipe:fulfilled:false:false"]"#
    );
}

#[test]
fn readable_stream_pipe_to_rejects_async_destination_close() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToAsyncCloseRejectEvents = [];
  const closeError = new Error("async close failed");
  let rejectClose;
  const readable = new ReadableStream({
    start(controller) {
      controller.close();
    }
  });
  const writable = new WritableStream({
    close() {
      globalThis.__pipeToAsyncCloseRejectEvents.push("close:start");
      return new Promise((_, reject) => {
        rejectClose = () => {
          globalThis.__pipeToAsyncCloseRejectEvents.push("close:reject");
          reject(closeError);
        };
      });
    }
  });
  readable.pipeTo(writable).then(
    () => globalThis.__pipeToAsyncCloseRejectEvents.push("pipe:fulfilled"),
    error => globalThis.__pipeToAsyncCloseRejectEvents.push(
      `pipe:${error === closeError}:${readable.locked}:${writable.locked}`
    )
  );
  globalThis.__pipeToAsyncCloseRejectEvents.push(`locked:${readable.locked}:${writable.locked}`);
  globalThis.__rejectPipeToAsyncClose = () => rejectClose();
})()
"#,
    )
    .expect("pipeTo async close rejection setup should evaluate");

    let before = vm
        .eval("JSON.stringify(globalThis.__pipeToAsyncCloseRejectEvents)")
        .expect("pipeTo async close rejection pending events should evaluate");
    assert_eq!(before, r#"["locked:true:true","close:start"]"#);

    vm.eval("globalThis.__rejectPipeToAsyncClose()")
        .expect("pipeTo async close should reject");
    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToAsyncCloseRejectEvents)")
            .expect("pipeTo async close rejection should drain microtasks");
        if events.contains("pipe:true:false:false") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToAsyncCloseRejectEvents)")
        .expect("pipeTo async close rejection events should settle");
    assert_eq!(
        events,
        r#"["locked:true:true","close:start","close:reject","pipe:true:false:false"]"#
    );
}

#[test]
fn readable_stream_pipe_to_prevent_close_waits_for_pending_write() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToPreventClosePendingEvents = [];
  let resolveWrite;
  const readable = new ReadableStream({
    start(controller) {
      controller.enqueue("chunk");
      controller.close();
    }
  });
  const writable = new WritableStream({
    write(chunk) {
      globalThis.__pipeToPreventClosePendingEvents.push(`write:${chunk}`);
      return new Promise(resolve => {
        resolveWrite = () => {
          globalThis.__pipeToPreventClosePendingEvents.push("write:resolve");
          resolve();
        };
      });
    },
    close() {
      globalThis.__pipeToPreventClosePendingEvents.push("close");
    }
  });
  readable.pipeTo(writable, { preventClose: true }).then(
    () => globalThis.__pipeToPreventClosePendingEvents.push(
      `pipe:fulfilled:${readable.locked}:${writable.locked}`
    ),
    error => globalThis.__pipeToPreventClosePendingEvents.push(`pipe:rejected:${error && error.name}`)
  );
  globalThis.__resolvePipeToPreventCloseWrite = () => resolveWrite();
})()
"#,
    )
    .expect("pipeTo preventClose pending write setup should evaluate");

    let before = vm
        .eval(
            r#"
JSON.stringify({
  events: globalThis.__pipeToPreventClosePendingEvents,
  settled: globalThis.__pipeToPreventClosePendingEvents.some(event => event.startsWith("pipe:"))
})
"#,
        )
        .expect("pipeTo preventClose pending state should evaluate");
    assert_eq!(before, r#"{"events":["write:chunk"],"settled":false}"#);

    vm.eval("globalThis.__resolvePipeToPreventCloseWrite()")
        .expect("pipeTo preventClose write should resolve");
    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToPreventClosePendingEvents)")
            .expect("pipeTo preventClose should drain microtasks");
        if events.contains("pipe:fulfilled:false:false") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToPreventClosePendingEvents)")
        .expect("pipeTo preventClose events should settle");
    assert_eq!(
        events,
        r#"["write:chunk","write:resolve","pipe:fulfilled:false:false"]"#
    );
}

#[test]
fn readable_stream_pipe_to_rejects_write_errors_and_honors_prevent_cancel() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToWriteRejectEvents = [];
  function run(label, options) {
    const readable = new ReadableStream({
      start(controller) {
        controller.enqueue(label);
      },
      cancel(reason) {
        globalThis.__pipeToWriteRejectEvents.push(`${label}:cancel:${String(reason)}:${reason === undefined}`);
      }
    });
    const writable = new WritableStream({
      write(chunk) {
        globalThis.__pipeToWriteRejectEvents.push(`${label}:write:${chunk}`);
        return Promise.reject(undefined);
      }
    });
    readable.pipeTo(writable, options).then(
      () => globalThis.__pipeToWriteRejectEvents.push(`${label}:pipe:fulfilled`),
      reason => globalThis.__pipeToWriteRejectEvents.push(`${label}:pipe:${String(reason)}:${reason === undefined}`)
    );
  }
  run("cancel", {});
  run("prevent", { preventCancel: true });
})()
"#,
    )
    .expect("pipeTo write rejection setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToWriteRejectEvents)")
            .expect("pipeTo write rejection should drain microtasks");
        if events.contains("prevent:pipe:undefined:true")
            && events.contains("cancel:pipe:undefined:true")
        {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToWriteRejectEvents.sort())")
        .expect("pipeTo write rejection events should settle");
    assert_eq!(
        events,
        r#"["cancel:cancel:undefined:true","cancel:pipe:undefined:true","cancel:write:cancel","prevent:pipe:undefined:true","prevent:write:prevent"]"#
    );
}

#[test]
fn readable_stream_pipe_to_rejects_sync_close_throw_and_releases_locks() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeToCloseThrowEvents = [];
  const closeError = new Error("close failed");
  const readable = new ReadableStream({
    start(controller) {
      controller.close();
    }
  });
  const writable = new WritableStream({
    close() {
      globalThis.__pipeToCloseThrowEvents.push("close");
      throw closeError;
    }
  });
  readable.pipeTo(writable).then(
    () => globalThis.__pipeToCloseThrowEvents.push("pipe:fulfilled"),
    error => globalThis.__pipeToCloseThrowEvents.push(
      `pipe:${error === closeError}:${readable.locked}:${writable.locked}`
    )
  );
})()
"#,
    )
    .expect("pipeTo close throw setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeToCloseThrowEvents)")
            .expect("pipeTo close throw should drain microtasks");
        if events.contains("pipe:true:false:false") {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeToCloseThrowEvents)")
        .expect("pipeTo close throw events should settle");
    assert_eq!(events, r#"["close","pipe:true:false:false"]"#);
}

#[test]
fn readable_stream_pipe_options_getter_errors_keep_entrypoint_semantics_and_order() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
(() => {
  globalThis.__pipeOptionErrorEvents = [];
  class ThrowingOptions {
    constructor(name) {
      this.name = name;
      this.touched = [];
    }
    get preventClose() {
      this.maybeThrow("preventClose");
      return false;
    }
    get preventAbort() {
      this.maybeThrow("preventAbort");
      return false;
    }
    get preventCancel() {
      this.maybeThrow("preventCancel");
      return false;
    }
    get signal() {
      this.maybeThrow("signal");
      return undefined;
    }
    maybeThrow(name) {
      this.touched.push(name);
      if (this.name === name) {
        throw new Error(name);
      }
    }
  }
  for (const name of ["preventAbort", "preventCancel", "preventClose", "signal"]) {
    const pipeToOptions = new ThrowingOptions(name);
    try {
      const promise = new ReadableStream().pipeTo(new WritableStream(), pipeToOptions);
      globalThis.__pipeOptionErrorEvents.push(`pipeTo:${name}:returned:${promise instanceof Promise}`);
      promise.then(
        () => globalThis.__pipeOptionErrorEvents.push(`pipeTo:${name}:fulfilled`),
        error => globalThis.__pipeOptionErrorEvents.push(
          `pipeTo:${name}:rejected:${error.message}:${pipeToOptions.touched.join(",")}`
        )
      );
    } catch (error) {
      globalThis.__pipeOptionErrorEvents.push(`pipeTo:${name}:threw:${error.message}`);
    }

    const pipeThroughOptions = new ThrowingOptions(name);
    try {
      new ReadableStream().pipeThrough(new TransformStream(), pipeThroughOptions);
      globalThis.__pipeOptionErrorEvents.push(`pipeThrough:${name}:returned`);
    } catch (error) {
      globalThis.__pipeOptionErrorEvents.push(
        `pipeThrough:${name}:threw:${error.message}:${pipeThroughOptions.touched.join(",")}`
      );
    }
  }
})()
"#,
    )
    .expect("pipe option getter error setup should evaluate");

    for _ in 0..8 {
        let events = vm
            .eval("JSON.stringify(globalThis.__pipeOptionErrorEvents)")
            .expect("pipe option getter errors should drain promise reactions");
        if events.matches(":rejected:").count() == 4 {
            break;
        }
    }

    let events = vm
        .eval("JSON.stringify(globalThis.__pipeOptionErrorEvents.sort())")
        .expect("pipe option getter error events should settle");
    assert_eq!(
        events,
        r#"["pipeThrough:preventAbort:threw:preventAbort:preventAbort","pipeThrough:preventCancel:threw:preventCancel:preventAbort,preventCancel","pipeThrough:preventClose:threw:preventClose:preventAbort,preventCancel,preventClose","pipeThrough:signal:threw:signal:preventAbort,preventCancel,preventClose,signal","pipeTo:preventAbort:rejected:preventAbort:preventAbort","pipeTo:preventAbort:returned:true","pipeTo:preventCancel:rejected:preventCancel:preventAbort,preventCancel","pipeTo:preventCancel:returned:true","pipeTo:preventClose:rejected:preventClose:preventAbort,preventCancel,preventClose","pipeTo:preventClose:returned:true","pipeTo:signal:rejected:signal:preventAbort,preventCancel,preventClose,signal","pipeTo:signal:returned:true"]"#
    );
}

#[test]
fn readable_stream_from_preserves_async_sequence_and_cancellation_semantics() {
    let mut vm = stream_test_vm();

    vm.eval(
        r#"
globalThis.__readableStreamFromResult = "";
(async () => {
  const descriptor = Object.getOwnPropertyDescriptor(ReadableStream, "from");
  const facts = {
    surface: [
      typeof descriptor.value,
      descriptor.value.name,
      descriptor.value.length,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable
    ]
  };

  let nullError = "none";
  try {
    ReadableStream.from(null);
  } catch (error) {
    nullError = error.name;
  }
  facts.nullError = nullError;

  const openMarker = new Error("open marker");
  let openIdentity = false;
  try {
    ReadableStream.from({
      [Symbol.asyncIterator]() {
        throw openMarker;
      }
    });
  } catch (error) {
    openIdentity = error === openMarker;
  }
  facts.openIdentity = openIdentity;

  const syncReader = ReadableStream.from([
    Promise.resolve("a"),
    "b"
  ]).getReader();
  facts.syncValues = [
    (await syncReader.read()).value,
    (await syncReader.read()).value,
    (await syncReader.read()).done
  ];

  let nextCalls = 0;
  const asyncIterator = {
    async next() {
      nextCalls += 1;
      return nextCalls === 1
        ? { value: "async", done: false }
        : { value: undefined, done: true };
    },
    [Symbol.asyncIterator]() {
      return this;
    }
  };
  const asyncReader = ReadableStream.from(asyncIterator).getReader();
  facts.asyncValues = [
    (await asyncReader.read()).value,
    (await asyncReader.read()).done,
    nextCalls
  ];

  const finishedReader = ReadableStream.from({
    next() {
      return Promise.resolve({
        done: true,
        get value() {
          throw new Error("finished iteration value must not be read");
        }
      });
    },
    [Symbol.asyncIterator]() {
      return this;
    }
  }).getReader();
  facts.finishedSkipsValue = (await finishedReader.read()).done;

  const nextMarker = new Error("next marker");
  const rejectedReader = ReadableStream.from({
    next() {
      throw nextMarker;
    },
    [Symbol.asyncIterator]() {
      return this;
    }
  }).getReader();
  facts.nextIdentity = await rejectedReader.read().then(
    () => false,
    error => error === nextMarker
  );

  const cancelReason = new Error("cancel reason");
  let returnReason;
  let releaseReturn;
  const cancelReader = ReadableStream.from({
    next() {
      throw new Error("next must not run");
    },
    return(reason) {
      returnReason = reason;
      return new Promise(resolve => {
        releaseReturn = () => resolve({ done: true });
      });
    },
    [Symbol.asyncIterator]() {
      return this;
    }
  }).getReader();
  let cancelSettled = false;
  const cancelPromise = cancelReader.cancel(cancelReason).then(() => {
    cancelSettled = true;
  });
  await Promise.resolve();
  facts.cancelPending = !cancelSettled && returnReason === cancelReason;
  releaseReturn();
  await cancelPromise;
  facts.cancelSettled = cancelSettled;

  const undefinedReturnReader = ReadableStream.from({
    next() {
      throw new Error("next must not run");
    },
    return() {},
    [Symbol.asyncIterator]() {
      return this;
    }
  }).getReader();
  facts.undefinedReturnRejects = await undefinedReturnReader.cancel().then(
    () => false,
    error => error instanceof TypeError
  );

  let reentrantReader;
  let reentrantReturns = 0;
  const reentrantStream = ReadableStream.from({
    async next() {
      await reentrantReader.cancel();
      return { value: "ignored", done: false };
    },
    async return() {
      reentrantReturns += 1;
    },
    [Symbol.asyncIterator]() {
      return this;
    }
  });
  reentrantReader = reentrantStream.getReader();
  facts.reentrant = [
    (await reentrantReader.read()).done,
    reentrantReturns
  ];

  globalThis.__readableStreamFromResult = JSON.stringify(facts);
})().catch(error => {
  globalThis.__readableStreamFromResult = `ERROR:${error && error.stack || error}`;
});
"#,
    )
    .expect("ReadableStream.from setup should evaluate");

    let mut result = String::new();
    for _ in 0..32 {
        result = vm
            .eval("globalThis.__readableStreamFromResult")
            .expect("ReadableStream.from reactions should drain");
        if !result.is_empty() {
            break;
        }
    }

    assert_eq!(
        result,
        r#"{"surface":["function","from",1,true,true,true],"nullError":"TypeError","openIdentity":true,"syncValues":["a","b",true],"asyncValues":["async",true,2],"finishedSkipsValue":true,"nextIdentity":true,"cancelPending":true,"cancelSettled":true,"undefinedReturnRejects":true,"reentrant":[true,1]}"#
    );
}
