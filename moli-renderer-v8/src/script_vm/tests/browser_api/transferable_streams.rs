use super::*;

#[test]
fn failed_message_serialization_does_not_transfer_readable_stream() {
    let mut vm = new_storage_test_vm("https://stream-transfer-failure.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue("still-readable");
      controller.close();
    }
  });
  let errorName = "no-throw";
  try {
    postMessage({stream, uncloneable: () => {}}, "*", [stream]);
  } catch (error) {
    errorName = error.name;
  }
  const lockedAfterFailure = stream.locked;
  let readerAcquired = false;
  try {
    stream.getReader();
    readerAcquired = true;
  } catch (_) {}
  return `${errorName}:${lockedAfterFailure}:${readerAcquired}`;
})()
"#,
        )
        .expect("failed ReadableStream transfer should evaluate");

    assert_eq!(result, "DataCloneError:false:true");
}

#[test]
fn transfer_list_transfers_readable_stream_absent_from_message_data() {
    let mut vm = new_storage_test_vm("https://stream-transfer-list.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const stream = new ReadableStream();
  postMessage({}, "*", [stream]);
  return String(stream.locked);
})()
"#,
        )
        .expect("unused ReadableStream transfer should evaluate");

    assert_eq!(result, "true");
}

#[tokio::test]
async fn transferred_streams_absent_from_message_data_release_unclaimed_channels() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-transfer-unclaimed.test/",
        &loader,
    );
    let registry = vm._context_host.borrow().message_port_registry();
    assert_eq!(registry.endpoint_count(), 0);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__unclaimedStreamTransferDelivered = false;
  addEventListener("message", () => {
    __unclaimedStreamTransferDelivered = true;
  }, {once: true});
  const readable = new ReadableStream();
  const writable = new WritableStream();
  const transform = new TransformStream();
  postMessage({}, "*", [readable, writable, transform]);
  return [
    readable.locked,
    writable.locked,
    transform.readable.locked,
    transform.writable.locked,
  ].join(":");
})()
"#,
        )
        .expect("unclaimed ReadableStream transfer should evaluate");

    assert_eq!(result, "true:true:true:true");
    assert_eq!(registry.endpoint_count(), 8);
    for _ in 0..20 {
        let delivered = vm
            .eval("String(__unclaimedStreamTransferDelivered)")
            .expect("unclaimed transfer delivery state should evaluate");
        if delivered == "true" && registry.endpoint_count() == 0 {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("unclaimed transfer cleanup should advance page tasks");
    }
    assert_eq!(
        vm.eval("String(__unclaimedStreamTransferDelivered)")
            .expect("unclaimed transfer delivery should evaluate"),
        "true"
    );
    assert_eq!(registry.endpoint_count(), 0);
}

#[test]
fn writable_and_transform_streams_require_transfer_and_reject_locked_endpoints_atomically() {
    let mut vm = new_storage_test_vm("https://stream-transfer-admission.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const names = [];
  for (const stream of [new WritableStream(), new TransformStream()]) {
    try {
      postMessage({stream}, "*");
      names.push("no-throw");
    } catch (error) {
      names.push(error.name);
    }
  }

  const writable = new WritableStream();
  writable.getWriter();
  try {
    postMessage(writable, "*", [writable]);
    names.push("unlocked-writable");
  } catch (error) {
    names.push(error.name);
  }

  const transform = new TransformStream();
  transform.writable.getWriter();
  try {
    postMessage(transform, "*", [transform]);
    names.push("unlocked-transform");
  } catch (error) {
    names.push(error.name);
  }
  names.push(String(transform.readable.locked));
  return names.join(":");
})()
"#,
        )
        .expect("stream transfer admission checks should evaluate");

    assert_eq!(
        result,
        "DataCloneError:DataCloneError:DataCloneError:DataCloneError:false"
    );
}

#[tokio::test]
async fn transferred_writable_stream_forwards_writes_and_close_with_backpressure() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://writable-stream-transfer.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__transferredWritableResult = "pending";
  const events = [];
  const original = new WritableStream({
    write(chunk) { events.push(`write:${chunk}`); },
    close() {
      events.push("close");
      __transferredWritableResult = events.join(",");
    }
  });
  addEventListener("message", async event => {
    try {
      const writer = event.data.getWriter();
      await writer.write("a");
      await writer.write("b");
      await writer.close();
    } catch (error) {
      __transferredWritableResult = `error:${error.name}:${error.message}`;
    }
  }, {once: true});
  postMessage(original, "*", [original]);
  return String(original.locked);
})()
"#,
        )
        .expect("transferred WritableStream setup should evaluate");

    assert_eq!(result, "true");
    for _ in 0..100 {
        if vm
            .eval("__transferredWritableResult")
            .expect("transferred writable result should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should advance transferred writable messages");
    }
    assert_eq!(
        vm.eval("__transferredWritableResult")
            .expect("transferred writable result should evaluate"),
        "write:a,write:b,close"
    );
}

#[tokio::test]
async fn transferred_transform_stream_materializes_both_remote_endpoints() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://transform-stream-transfer.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__transferredTransformResult = "pending";
  const original = new TransformStream({
    transform(chunk, controller) { controller.enqueue(chunk.toUpperCase()); }
  });
  addEventListener("message", async event => {
    try {
      const reader = event.data.readable.getReader();
      const writer = event.data.writable.getWriter();
      const write = writer.write("hello");
      const first = await reader.read();
      const close = writer.close();
      const second = await reader.read();
      await Promise.all([write, close]);
      __transferredTransformResult = `${first.value}:${first.done}:${second.done}`;
    } catch (error) {
      __transferredTransformResult = `error:${error.name}:${error.message}`;
    }
  }, {once: true});
  postMessage(original, "*", [original]);
  return `${original.readable.locked}:${original.writable.locked}`;
})()
"#,
        )
        .expect("transferred TransformStream setup should evaluate");

    assert_eq!(result, "true:true");
    for _ in 0..150 {
        if vm
            .eval("__transferredTransformResult")
            .expect("transferred transform result should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should advance transferred transform messages");
    }
    assert_eq!(
        vm.eval("__transferredTransformResult")
            .expect("transferred transform result should evaluate"),
        "HELLO:false:true"
    );
}

#[test]
fn later_transfer_failure_does_not_roll_back_committed_readable_stream() {
    let mut vm = new_storage_test_vm("https://stream-transfer-partial.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const first = new ReadableStream();
  const alreadyLocked = new ReadableStream();
  alreadyLocked.getReader();
  let errorName = "no-throw";
  try {
    postMessage({}, "*", [first, alreadyLocked]);
  } catch (error) {
    errorName = error.name;
  }
  return `${errorName}:${first.locked}:${alreadyLocked.locked}`;
})()
"#,
        )
        .expect("partially committed ReadableStream transfer should evaluate");

    assert_eq!(result, "DataCloneError:true:true");
}

#[tokio::test]
async fn transferred_readable_stream_ids_follow_transfer_list_order() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-transfer-order.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__orderedStreamResult = "pending";
  const first = new ReadableStream({
    start(controller) {
      controller.enqueue("first");
      controller.close();
    }
  });
  const second = new ReadableStream({
    start(controller) {
      controller.enqueue("second");
      controller.close();
    }
  });
  addEventListener("message", async event => {
    const firstResult = await event.data.first.getReader().read();
    const secondResult = await event.data.second.getReader().read();
    __orderedStreamResult = `${firstResult.value}:${secondResult.value}`;
  }, {once: true});
  postMessage({second, first}, "*", [first, second]);
  return `${first.locked}:${second.locked}`;
})()
"#,
        )
        .expect("ordered ReadableStream transfer should evaluate");

    assert_eq!(result, "true:true");
    for _ in 0..20 {
        if vm
            .eval("__orderedStreamResult")
            .expect("ordered ReadableStream result should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should deliver ordered ReadableStreams");
    }
    assert_eq!(
        vm.eval("__orderedStreamResult")
            .expect("ordered ReadableStream result should evaluate"),
        "first:second"
    );
}

#[tokio::test]
async fn window_post_message_preserves_message_event_ports_with_readable_stream_transfer() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-port-transfer.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__streamPortTransferResult = "pending";
  const channel = new MessageChannel();
  const stream = new ReadableStream();
  postMessage({stream, port: channel.port2}, "*", [stream, channel.port2]);
  addEventListener("message", event => {
    __streamPortTransferResult = JSON.stringify({
      ports: event.ports.length,
      dataPort: event.data.port instanceof MessagePort,
      samePort: event.data.port === event.ports[0],
      stream: event.data.stream instanceof ReadableStream
    });
  }, {once: true});
  return `${stream.locked}:${channel.port2.postMessage === undefined}`;
})()
"#,
        )
        .expect("combined ReadableStream and MessagePort transfer should evaluate");

    assert_eq!(result, "true:false");
    for _ in 0..20 {
        if vm
            .eval("__streamPortTransferResult")
            .expect("combined transfer result should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should deliver combined transfer");
    }
    assert_eq!(
        vm.eval("__streamPortTransferResult")
            .expect("combined transfer result should evaluate"),
        r#"{"ports":1,"dataPort":true,"samePort":true,"stream":true}"#
    );
}

#[tokio::test]
async fn window_post_message_transferred_readable_stream_forwards_later_chunks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-transfer.test/path",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__transferredStreamResult = "pending";
  let controller;
  const original = new ReadableStream({
    start(value) {
      controller = value;
    }
  });
  addEventListener("message", async event => {
    try {
      const reader = event.data.getReader();
      const chunks = [];
      for (;;) {
        const {value, done} = await reader.read();
        if (done) {
          break;
        }
        chunks.push(value);
      }
      __transferredStreamResult = chunks.join(",");
    } catch (error) {
      __transferredStreamResult = "error:" + error.name + ":" + error.message;
    }
  }, {once: true});
  postMessage(original, "*", [original]);
  for (let index = 0; index < 10; index += 1) {
    controller.enqueue(index);
  }
  controller.close();
  return String(original.locked);
})()
"#,
        )
        .expect("transferred ReadableStream setup should evaluate");

    assert_eq!(result, "true");
    for _ in 0..100 {
        if vm
            .eval("__transferredStreamResult")
            .expect("transferred ReadableStream result should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should advance transferred ReadableStream messages");
    }
    assert_eq!(
        vm.eval("__transferredStreamResult")
            .expect("transferred ReadableStream result should evaluate"),
        "0,1,2,3,4,5,6,7,8,9"
    );
}

#[tokio::test]
async fn retransferred_readable_stream_preserves_each_transfer_channel() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-retransfer.test/path",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__retransferredStreamResult = "pending";
  let controller;
  const original = new ReadableStream({
    start(value) {
      controller = value;
    }
  });
  let transferCount = 0;
  addEventListener("message", event => {
    if (transferCount++ === 0) {
      postMessage(event.data, "*", [event.data]);
      return;
    }
    event.data.getReader().read().then(
      ({value, done}) => {
        __retransferredStreamResult = `value:${value}:${done}`;
      },
      error => {
        __retransferredStreamResult = `error:${error.name}:${error.message}`;
      }
    );
  });
  postMessage(original, "*", [original]);
  controller.enqueue("through-two-channels");
  return String(original.locked);
})()
"#,
        )
        .expect("retransferred ReadableStream setup should evaluate");

    assert_eq!(result, "true");
    for _ in 0..100 {
        if vm
            .eval("__retransferredStreamResult")
            .expect("retransferred ReadableStream result should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should advance both ReadableStream transfer channels");
    }
    assert_eq!(
        vm.eval("__retransferredStreamResult")
            .expect("retransferred ReadableStream result should evaluate"),
        "value:through-two-channels:false"
    );
}

#[tokio::test]
async fn terminating_echo_worker_does_not_error_retransferred_readable_stream() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-worker-termination.test/path",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__terminatedWorkerStreamResult = "pending";
  (async () => {
    const worker = new Worker(
      "data:text/javascript,onmessage%20%3D%20event%20%3D%3E%20postMessage(event.data%2C%20%5Bevent.data%5D)%3B"
    );
    let controller;
    const original = new ReadableStream({
      start(value) {
        controller = value;
      }
    });
    const transferred = await new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(new Error(event.message));
      worker.postMessage(original, [original]);
    });
    const reader = transferred.getReader();
    const reads = [];
    for (let value = 0; value < 50; ++value) {
      await new Promise(resolve => setTimeout(resolve, 0));
      controller.enqueue(value);
      reads.push(reader.read().then(result => {
        if (result.done || result.value !== value) {
          throw new Error(`unexpected read ${value}`);
        }
      }));
    }

    worker.terminate();
    for (let value = 50; value < 60; ++value) {
      controller.enqueue(value);
      reads.push(reader.read().then(
        () => { throw new Error("post-termination read resolved"); },
        error => { throw error; }
      ));
      await new Promise(resolve => setTimeout(resolve, 0));
    }

    Promise.all(reads).catch(error => {
      __terminatedWorkerStreamResult = `error:${error.name}:${error.message}`;
    });
    for (let turn = 0; turn < 4; ++turn) {
      await new Promise(resolve => setTimeout(resolve, 0));
    }
    if (__terminatedWorkerStreamResult === "pending") {
      __terminatedWorkerStreamResult = "pass";
    }
  })().catch(error => {
    __terminatedWorkerStreamResult = `error:${error.name}:${error.message}`;
  });
  return "started";
})()
"#,
        )
        .expect("worker termination stream setup should evaluate");

    assert_eq!(result, "started");
    for _ in 0..1_000 {
        if vm
            .eval("__terminatedWorkerStreamResult")
            .expect("worker termination stream result should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should advance worker and stream tasks");
    }
    assert_eq!(
        vm.eval("__terminatedWorkerStreamResult")
            .expect("worker termination stream result should evaluate"),
        "pass"
    );
}

#[tokio::test]
async fn window_post_message_transferred_readable_stream_serializes_chunks_on_demand() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-transfer-lazy.test/path",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__transferredStreamReady = false;
  globalThis.__transferredStreamSerializationCount = 0;
  globalThis.__transferredStreamReadResult = "pending";
  let controller;
  const original = new ReadableStream({
    start(value) {
      controller = value;
    }
  });
  addEventListener("message", event => {
    globalThis.__transferredStreamReader = event.data.getReader();
    __transferredStreamReady = true;
  }, {once: true});
  postMessage(original, "*", [original]);
  globalThis.__enqueueTransferredStreamChunk = () => {
    controller.enqueue({
      get getter() {
        __transferredStreamSerializationCount += 1;
        return "chunk";
      }
    });
  };
  return String(original.locked);
})()
"#,
        )
        .expect("transferred ReadableStream lazy serialization setup should evaluate");

    assert_eq!(result, "true");
    for _ in 0..20 {
        if vm
            .eval("String(__transferredStreamReady)")
            .expect("transferred ReadableStream readiness should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should deliver transferred ReadableStream");
    }
    assert_eq!(
        vm.eval("String(__transferredStreamReady)")
            .expect("transferred ReadableStream readiness should evaluate"),
        "true"
    );
    assert_eq!(
        vm.eval("__enqueueTransferredStreamChunk(); String(__transferredStreamSerializationCount)")
            .expect("transferred ReadableStream chunk should enqueue"),
        "0"
    );
    for _ in 0..10 {
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should flush without reading transferred chunk");
    }
    assert_eq!(
        vm.eval("String(__transferredStreamSerializationCount)")
            .expect("transferred ReadableStream serialization count should evaluate"),
        "0"
    );

    vm.eval(
        r#"
__transferredStreamReader.read().then(
  result => { __transferredStreamReadResult = result.value.getter; },
  error => { __transferredStreamReadResult = "error:" + error.name; }
)
"#,
    )
    .expect("transferred ReadableStream read should start");
    for _ in 0..20 {
        if vm
            .eval("__transferredStreamReadResult")
            .expect("transferred ReadableStream read result should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should deliver transferred ReadableStream chunk");
    }
    assert_eq!(
        vm.eval("__transferredStreamReadResult")
            .expect("transferred ReadableStream read result should evaluate"),
        "chunk"
    );
    assert_eq!(
        vm.eval("String(__transferredStreamSerializationCount)")
            .expect("transferred ReadableStream serialization count should evaluate"),
        "1"
    );
}

#[tokio::test]
async fn transferred_readable_stream_cancel_rejects_uncloneable_reason() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-cancel-reason.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__transferredCancelResult = "pending";
  globalThis.__originalCancelReason = "pending";
  const original = new ReadableStream({
    cancel(reason) {
      __originalCancelReason = `${reason.name}:${reason instanceof DOMException}`;
    }
  });
  addEventListener("message", event => {
    event.data.cancel(Symbol("uncloneable")).then(
      () => { __transferredCancelResult = "resolved"; },
      error => {
        __transferredCancelResult = `${error.name}:${error instanceof DOMException}`;
      }
    );
  }, {once: true});
  postMessage(original, "*", [original]);
})()
"#,
    )
    .expect("transferred ReadableStream cancel setup should evaluate");

    for _ in 0..20 {
        let result = vm
            .eval("`${__transferredCancelResult}|${__originalCancelReason}`")
            .expect("transferred ReadableStream cancel state should evaluate");
        if result != "pending|pending" && !result.ends_with("|pending") {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should deliver transferred stream cancel messages");
    }
    assert_eq!(
        vm.eval("`${__transferredCancelResult}|${__originalCancelReason}`")
            .expect("transferred ReadableStream cancel result should evaluate"),
        "DataCloneError:true|DataCloneError:true"
    );
}

#[tokio::test]
async fn transferred_readable_stream_propagates_source_error() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-source-error.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__transferredSourceErrorResult = "pending";
  let controller;
  const original = new ReadableStream({
    start(value) {
      controller = value;
    }
  });
  addEventListener("message", event => {
    event.data.getReader().read().then(
      () => { __transferredSourceErrorResult = "resolved"; },
      reason => { __transferredSourceErrorResult = `rejected:${reason}`; }
    );
  }, {once: true});
  postMessage(original, "*", [original]);
  controller.error("source-boom");
})()
"#,
    )
    .expect("transferred ReadableStream source error setup should evaluate");

    for _ in 0..30 {
        if vm
            .eval("__transferredSourceErrorResult")
            .expect("transferred source error state should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should deliver transferred source error");
    }
    assert_eq!(
        vm.eval("__transferredSourceErrorResult")
            .expect("transferred source error result should evaluate"),
        "rejected:source-boom"
    );
}

#[tokio::test]
async fn transferred_readable_stream_forwards_cloneable_cancel_reason_once() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-cancel-forward.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__transferredCancelForwardResult = "pending";
  globalThis.__originalCancelCount = 0;
  globalThis.__originalCancelCode = "pending";
  const original = new ReadableStream({
    cancel(reason) {
      __originalCancelCount += 1;
      __originalCancelCode = reason.code;
    }
  });
  addEventListener("message", event => {
    event.data.cancel({code: "stop"}).then(
      () => { __transferredCancelForwardResult = "resolved"; },
      error => { __transferredCancelForwardResult = `rejected:${error.name}`; }
    );
  }, {once: true});
  postMessage(original, "*", [original]);
})()
"#,
    )
    .expect("transferred ReadableStream cancel forwarding setup should evaluate");

    for _ in 0..30 {
        let current = vm
            .eval(
                "`${__transferredCancelForwardResult}|${__originalCancelCount}|${__originalCancelCode}`",
            )
            .expect("transferred cancel forwarding state should evaluate");
        if current == "resolved|1|stop" {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should deliver transferred cancel reason");
    }
    assert_eq!(
        vm.eval(
            "`${__transferredCancelForwardResult}|${__originalCancelCount}|${__originalCancelCode}`",
        )
        .expect("transferred cancel forwarding result should evaluate"),
        "resolved|1|stop"
    );
}

#[tokio::test]
async fn transferred_readable_stream_chunk_clone_failure_errors_receiver() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://stream-chunk-clone-error.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__transferredChunkCloneResult = "pending";
  const original = new ReadableStream({
    start(controller) {
      controller.enqueue(Symbol("uncloneable"));
    }
  });
  addEventListener("message", event => {
    event.data.getReader().read().then(
      () => { __transferredChunkCloneResult = "resolved"; },
      error => {
        __transferredChunkCloneResult = `${error.name}:${error instanceof DOMException}`;
      }
    );
  }, {once: true});
  postMessage(original, "*", [original]);
})()
"#,
    )
    .expect("transferred ReadableStream chunk clone error setup should evaluate");

    for _ in 0..30 {
        if vm
            .eval("__transferredChunkCloneResult")
            .expect("transferred chunk clone state should evaluate")
            != "pending"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn()
            .await
            .expect("wait driver should deliver transferred chunk clone error");
    }
    assert_eq!(
        vm.eval("__transferredChunkCloneResult")
            .expect("transferred chunk clone result should evaluate"),
        "DataCloneError:true"
    );
}
