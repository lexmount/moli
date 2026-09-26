use super::*;

#[test]
fn readable_stream_underlying_source_algorithms_match_bad_sources_wpt() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__badSourceEvents = [];
                const events = globalThis.__badSourceEvents;
                const startGetterError = new Error("start-getter");
                const startMethodError = new Error("start-method");
                const pullGetterError = new Error("pull-getter");
                const pullMethodError = new Error("pull-method");
                const cancelGetterError = new Error("cancel-getter");
                const cancelMethodError = new Error("cancel-method");

                const nameOf = callback => {
                    try {
                        callback();
                        return "ok";
                    } catch (error) {
                        return error && error.message;
                    }
                };

                events.push("start-getter:" + nameOf(() => new ReadableStream({
                    get start() {
                        throw startGetterError;
                    }
                })));
                events.push("start-method:" + nameOf(() => new ReadableStream({
                    start() {
                        throw startMethodError;
                    }
                })));
                events.push("pull-getter:" + nameOf(() => new ReadableStream({
                    get pull() {
                        throw pullGetterError;
                    }
                })));
                events.push("cancel-getter:" + nameOf(() => new ReadableStream({
                    get cancel() {
                        throw cancelGetterError;
                    }
                })));

                const pullStream = new ReadableStream({
                    pull() {
                        throw pullMethodError;
                    }
                });
                const pullReader = pullStream.getReader();
                pullReader.closed.then(
                    () => events.push("pull-method:resolved"),
                    error => events.push("pull-method:" + (error === pullMethodError))
                );

                new ReadableStream({
                    cancel() {
                        throw cancelMethodError;
                    }
                }).cancel().then(
                    () => events.push("cancel-method:resolved"),
                    error => events.push("cancel-method:" + (error === cancelMethodError))
                );

                let counter = 0;
                const singleGetStream = new ReadableStream({
                    get pull() {
                        ++counter;
                        if (counter === 1) {
                            return controller => controller.enqueue("a");
                        }
                        throw new Error("second-get");
                    }
                });
                const singleGetReader = singleGetStream.getReader();
                Promise.all([
                    singleGetReader.read(),
                    singleGetReader.read(),
                ]).then(([first, second]) => {
                    events.push(
                        `single-get:${counter}:${first.value}:${first.done}:${second.value}:${second.done}`
                    );
                });

                let methodCounter = 0;
                const secondPullStream = new ReadableStream({
                    pull(controller) {
                        ++methodCounter;
                        if (methodCounter === 1) {
                            controller.enqueue("first");
                            return;
                        }
                        throw new Error("second-method");
                    }
                });
                const secondPullReader = secondPullStream.getReader();
                secondPullReader.read().then(({ value, done }) => {
                    events.push(`second-method-read:${value}:${done}`);
                });
                secondPullReader.closed.then(
                    () => events.push("second-method-closed:resolved"),
                    error => events.push(
                        `second-method-closed:${methodCounter}:${error.message}`
                    )
                );

                return JSON.stringify(events);
            })()
            "#,
        )
        .expect("ReadableStream bad underlying source setup should evaluate");
    assert_eq!(
        initial,
        r#"["start-getter:start-getter","start-method:start-method","pull-getter:pull-getter","cancel-getter:cancel-getter"]"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__badSourceEvents.sort())")
        .expect("ReadableStream bad underlying source promises should settle");
    assert_eq!(
        settled,
        r#"["cancel-getter:cancel-getter","cancel-method:true","pull-getter:pull-getter","pull-method:true","second-method-closed:2:second-method","second-method-read:first:false","single-get:1:a:false:a:false","start-getter:start-getter","start-method:start-method"]"#
    );
}

#[test]
fn readable_stream_controller_state_checks_match_bad_sources_wpt() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__controllerStateEvents = [];
                const events = globalThis.__controllerStateEvents;
                const nameOf = callback => {
                    try {
                        callback();
                        return "ok";
                    } catch (error) {
                        return error && error.name;
                    }
                };

                let canceledEmptyController;
                const canceledEmpty = new ReadableStream({
                    start(controller) {
                        canceledEmptyController = controller;
                    }
                });
                canceledEmpty.cancel();
                events.push("enqueue-canceled-empty:" + nameOf(() => {
                    canceledEmptyController.enqueue("a");
                }));
                events.push("close-canceled-empty:" + nameOf(() => {
                    canceledEmptyController.close();
                }));
                canceledEmpty.getReader().closed.then(() => {
                    events.push("canceled-empty-closed");
                });

                let canceledQueuedController;
                const canceledQueued = new ReadableStream({
                    start(controller) {
                        canceledQueuedController = controller;
                        controller.enqueue("a");
                    }
                });
                canceledQueued.cancel();
                events.push("enqueue-canceled-queued:" + nameOf(() => {
                    canceledQueuedController.enqueue("b");
                }));
                events.push("close-canceled-queued:" + nameOf(() => {
                    canceledQueuedController.close();
                }));
                canceledQueued.getReader().closed.then(() => {
                    events.push("canceled-queued-closed");
                });

                new ReadableStream({
                    start(controller) {
                        controller.close();
                        events.push("enqueue-closed:" + nameOf(() => controller.enqueue("a")));
                        events.push("close-closed:" + nameOf(() => controller.close()));
                        events.push("error-after-close:" + nameOf(() => controller.error()));
                    }
                }).getReader().closed.then(() => events.push("closed-stream-closed"));

                const error = new Error("boom");
                new ReadableStream({
                    start(controller) {
                        controller.error(error);
                        events.push("enqueue-errored:" + nameOf(() => controller.enqueue("a")));
                        events.push("close-errored:" + nameOf(() => controller.close()));
                        events.push("error-twice:" + nameOf(() => controller.error()));
                    }
                }).getReader().closed.then(
                    () => events.push("errored-stream:resolved"),
                    reason => events.push("errored-stream:" + (reason === error))
                );

                return JSON.stringify(events);
            })()
            "#,
        )
        .expect("ReadableStream controller state setup should evaluate");
    assert_eq!(
        initial,
        r#"["enqueue-canceled-empty:TypeError","close-canceled-empty:TypeError","enqueue-canceled-queued:TypeError","close-canceled-queued:TypeError","enqueue-closed:TypeError","close-closed:TypeError","error-after-close:ok","enqueue-errored:TypeError","close-errored:TypeError","error-twice:ok"]"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__controllerStateEvents.sort())")
        .expect("ReadableStream controller state promises should settle");
    assert_eq!(
        settled,
        r#"["canceled-empty-closed","canceled-queued-closed","close-canceled-empty:TypeError","close-canceled-queued:TypeError","close-closed:TypeError","close-errored:TypeError","closed-stream-closed","enqueue-canceled-empty:TypeError","enqueue-canceled-queued:TypeError","enqueue-closed:TypeError","enqueue-errored:TypeError","error-after-close:ok","error-twice:ok","errored-stream:true"]"#
    );
}

#[test]
fn transform_stream_readable_writable_getters_are_on_prototypes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const encoder = new TextEncoderStream();
                const decoder = new TextDecoderStream();
                const transform = new TransformStream();
                const getterDescriptor = (prototype, name) => {
                    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                    return [
                        name,
                        typeof descriptor?.get,
                        descriptor?.get?.name,
                        descriptor?.get?.length,
                        typeof descriptor?.set,
                        descriptor?.enumerable,
                        descriptor?.configurable
                    ].join(':');
                };
                const encoderReadableDescriptor = Object.getOwnPropertyDescriptor(TextEncoderStream.prototype, 'readable');
                const encoderWritableDescriptor = Object.getOwnPropertyDescriptor(TextEncoderStream.prototype, 'writable');
                const decoderReadableDescriptor = Object.getOwnPropertyDescriptor(TextDecoderStream.prototype, 'readable');
                const decoderWritableDescriptor = Object.getOwnPropertyDescriptor(TextDecoderStream.prototype, 'writable');
                const transformReadableDescriptor = Object.getOwnPropertyDescriptor(TransformStream.prototype, 'readable');
                const transformWritableDescriptor = Object.getOwnPropertyDescriptor(TransformStream.prototype, 'writable');
                const encoderReadable = encoderReadableDescriptor.get;
                const encoderWritable = encoderWritableDescriptor.get;
                const decoderReadable = decoderReadableDescriptor.get;
                const decoderWritable = decoderWritableDescriptor.get;
                const transformReadable = transformReadableDescriptor.get;
                const transformWritable = transformWritableDescriptor.get;
                return [
                    getterDescriptor(TextEncoderStream.prototype, 'readable'),
                    getterDescriptor(TextEncoderStream.prototype, 'writable'),
                    getterDescriptor(TextDecoderStream.prototype, 'readable'),
                    getterDescriptor(TextDecoderStream.prototype, 'writable'),
                    getterDescriptor(TransformStream.prototype, 'readable'),
                    getterDescriptor(TransformStream.prototype, 'writable'),
                    typeof encoderReadable,
                    typeof encoderWritable,
                    encoderReadable.call(encoder) === encoder.readable,
                    encoderWritable.call(encoder) === encoder.writable,
                    decoderReadable.call(decoder) === decoder.readable,
                    decoderWritable.call(decoder) === decoder.writable,
                    transformReadable.call(transform) === transform.readable,
                    transformWritable.call(transform) === transform.writable,
                    encoder.hasOwnProperty('readable'),
                    encoder.hasOwnProperty('writable'),
                    decoder.hasOwnProperty('readable'),
                    decoder.hasOwnProperty('writable'),
                    transform.hasOwnProperty('readable'),
                    transform.hasOwnProperty('writable'),
                ].join('|');
            })()
            "#,
        )
        .expect("TransformStream prototype getters should evaluate");

    assert_eq!(
        result,
        "readable:function:get readable:0:undefined:true:true|writable:function:get writable:0:undefined:true:true|readable:function:get readable:0:undefined:true:true|writable:function:get writable:0:undefined:true:true|readable:function:get readable:0:undefined:true:true|writable:function:get writable:0:undefined:true:true|function|function|true|true|true|true|true|true|false|false|false|false|false|false"
    );
}

#[test]
fn child_window_stream_constructors_keep_lengths_and_result_realms() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__childStreamRealmEvents = [];
                const frame = document.createElement('iframe');
                (document.body || document.documentElement || document).appendChild(frame);
                const win = frame.contentWindow;
                globalThis.__childStreamRealmWindow = win;

                const readable = new win.ReadableStream({
                    start(controller) {
                        controller.enqueue('ok');
                    }
                });
                readable.getReader().read().then(result => {
                    globalThis.__childStreamRealmEvents.push([
                        'readable',
                        Object.getPrototypeOf(result) === win.Object.prototype,
                        result.constructor === win.Object
                    ].join(':'));
                });

                const encoder = new win.TextEncoderStream();
                encoder.readable.getReader().read().then(result => {
                    globalThis.__childStreamRealmEvents.push([
                        'encoder',
                        Object.getPrototypeOf(result) === win.Object.prototype,
                        result.constructor === win.Object,
                        Object.getPrototypeOf(result.value) === win.Uint8Array.prototype,
                        encoder.hasOwnProperty('readable'),
                        encoder.hasOwnProperty('writable')
                    ].join(':'));
                });
                encoder.writable.getWriter().write('A');

                return [
                    win.ReadableStream.length,
                    win.WritableStream.length,
                    win.TransformStream.length,
                    win.TextEncoderStream.length,
                    win.TextDecoderStream.length,
                    JSON.stringify(globalThis.__childStreamRealmEvents)
                ].join('|');
            })()
            "#,
        )
        .expect("child stream realm setup should evaluate");
    assert_eq!(initial, "0|0|0|0|0|[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__childStreamRealmEvents.sort())")
        .expect("child stream realm promises should settle");
    assert_eq!(
        settled,
        r#"["encoder:true:true:true:false:false","readable:true:true"]"#
    );
}

#[test]
fn borrowed_stream_methods_create_readers_iterators_and_tee_branches_in_stream_realm() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const frame = document.createElement('iframe');
                (document.body || document.documentElement || document).appendChild(frame);
                const other = frame.contentWindow;

                const stream = new ReadableStream();
                const borrowedReader = other.ReadableStream.prototype.getReader.call(stream);
                const constructedReader = new other.ReadableStreamDefaultReader(new ReadableStream());
                const readerRealms = [
                    borrowedReader instanceof ReadableStreamDefaultReader,
                    borrowedReader instanceof other.ReadableStreamDefaultReader,
                    constructedReader instanceof ReadableStreamDefaultReader,
                    constructedReader instanceof other.ReadableStreamDefaultReader
                ];
                borrowedReader.releaseLock();
                constructedReader.releaseLock();

                const iterator = other.ReadableStream.prototype.values.call(new ReadableStream(), {
                    preventCancel: true
                });
                const mainIteratorPrototype = Object.getPrototypeOf(
                    new ReadableStream().values({ preventCancel: true })
                );
                const iteratorRealm = Object.getPrototypeOf(iterator) === mainIteratorPrototype;
                iterator.return();

                const branches = other.ReadableStream.prototype.tee.call(new ReadableStream());

                let pendingController;
                const pendingStream = new ReadableStream({
                    start(controller) {
                        pendingController = controller;
                    }
                });
                const pendingReader = other.ReadableStream.prototype.getReader.call(pendingStream);
                const pendingRead = pendingReader.read();
                globalThis.__borrowedStreamReadResultRealm = [];
                pendingRead.then(result => {
                    globalThis.__borrowedStreamReadResultRealm.push(
                        result instanceof other.Object,
                        result instanceof Object,
                        result.done
                    );
                });
                other.ReadableStreamDefaultController.prototype.close.call(pendingController);

                return JSON.stringify({
                    readerRealms,
                    iteratorRealm,
                    branchRealms: branches.map(branch => [
                        branch instanceof ReadableStream,
                        branch instanceof other.ReadableStream
                    ])
                });
            })()
            "#,
        )
        .expect("borrowed stream method realms should evaluate");

    assert_eq!(
        result,
        r#"{"readerRealms":[true,false,false,true],"iteratorRealm":true,"branchRealms":[[true,false],[true,false]]}"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__borrowedStreamReadResultRealm)")
        .expect("borrowed stream read result realm should settle");
    assert_eq!(settled, "[true,false,true]");
}

#[test]
fn readable_stream_pipe_through_forwards_future_text_encoder_chunks() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__pipeThroughEvents = [];
                const source = new ReadableStream({
                    start(controller) {
                        globalThis.__pipeThroughController = controller;
                    }
                });
                const encoded = source.pipeThrough(new TextEncoderStream());
                globalThis.__pipeThroughReader = encoded.getReader();
                globalThis.__pipeThroughReader.read().then(({ value, done }) => {
                    globalThis.__pipeThroughEvents.push(
                        `${Array.from(value || []).join(",")}:${done}`
                    );
                });
                return JSON.stringify(globalThis.__pipeThroughEvents);
            })()
            "#,
        )
        .expect("pipeThrough setup should evaluate");
    assert_eq!(initial, "[]");

    vm.eval(
        r#"
        (() => {
            globalThis.__pipeThroughController.enqueue("abc");
            return JSON.stringify(globalThis.__pipeThroughEvents);
        })()
        "#,
    )
    .expect("future pipeThrough enqueue should evaluate");
    let after_enqueue = vm
        .eval("JSON.stringify(globalThis.__pipeThroughEvents)")
        .expect("future pipeThrough enqueue should settle");
    assert_eq!(after_enqueue, r#"["97,98,99:false"]"#);

    vm.eval(
        r#"
        (() => {
            globalThis.__pipeThroughReader.read().then(({ value, done }) => {
                globalThis.__pipeThroughEvents.push(`${String(value)}:${done}`);
            });
            globalThis.__pipeThroughController.close();
            return JSON.stringify(globalThis.__pipeThroughEvents);
        })()
        "#,
    )
    .expect("future pipeThrough close should evaluate");
    let after_close = vm
        .eval("JSON.stringify(globalThis.__pipeThroughEvents)")
        .expect("future pipeThrough close should settle");
    assert_eq!(after_close, r#"["97,98,99:false","undefined:true"]"#);
}

#[test]
fn tee_cancel_settles_when_source_was_closed_with_queued_chunks() {
    let mut vm = new_storage_test_vm("https://tee-closed-source.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
              globalThis.__teeClosedSourceEvents = [];
              const source = new ReadableStream({
                start(controller) {
                  controller.enqueue("one");
                  controller.enqueue("two");
                  controller.close();
                },
                cancel(reason) {
                  globalThis.__teeClosedSourceEvents.push(`source-cancel:${String(reason)}`);
                }
              });
              const [left, right] = source.tee();
              const leftReader = left.getReader();
              const rightReader = right.getReader();
              Promise.all([
                leftReader.read().then(({ value }) => {
                  globalThis.__teeClosedSourceEvents.push(`left:${value}`);
                  return leftReader.cancel("left-stop").then(() => {
                    globalThis.__teeClosedSourceEvents.push("left-canceled");
                  });
                }),
                (async () => {
                  const values = [];
                  while (true) {
                    const { value, done } = await rightReader.read();
                    if (done) break;
                    values.push(value);
                  }
                  globalThis.__teeClosedSourceEvents.push(`right:${values.join("|")}`);
                })()
              ]).then(() => globalThis.__teeClosedSourceEvents.push("settled"));
              return JSON.stringify(globalThis.__teeClosedSourceEvents);
            })()
            "#,
        )
        .expect("closed source tee setup should evaluate");
    assert_eq!(initial, "[]");

    for _ in 0..8 {
        vm.eval("0")
            .expect("closed source tee promise chain should drain");
    }
    let settled = vm
        .eval("JSON.stringify(globalThis.__teeClosedSourceEvents)")
        .expect("closed source tee events should evaluate");
    // Chromium drains the surviving branch before the shared tee cancel
    // promise reaction runs once the already-closed source is observed.
    assert_eq!(
        settled,
        r#"["left:one","right:one|two","left-canceled","settled"]"#
    );
}

#[test]
fn writable_stream_controller_signal_aborts_before_underlying_sink_abort() {
    let mut vm = new_storage_test_vm("https://writable-controller-signal.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
              globalThis.__writableControllerSignalEvents = [];
              let capturedSignal;
              const stream = new WritableStream({
                start(controller) {
                  capturedSignal = controller.signal;
                  __writableControllerSignalEvents.push(`start:${controller.signal.aborted}`);
                  controller.signal.addEventListener("abort", () => {
                    __writableControllerSignalEvents.push(`signal:${String(controller.signal.reason)}`);
                  });
                },
                write(chunk, controller) {
                  __writableControllerSignalEvents.push(
                    `write:${chunk}:${controller.signal === capturedSignal}:${controller.signal.aborted}`
                  );
                },
                abort(reason) {
                  __writableControllerSignalEvents.push(
                    `sink:${String(reason)}:${capturedSignal.aborted}:${String(capturedSignal.reason)}`
                  );
                }
              });
              const writer = stream.getWriter();
              writer.write("one").then(() => writer.abort("stop")).then(() => {
                __writableControllerSignalEvents.push("settled");
              });
              return JSON.stringify({
                events: __writableControllerSignalEvents,
                signalTag: Object.prototype.toString.call(capturedSignal),
                descriptor: (() => {
                  const descriptor = Object.getOwnPropertyDescriptor(
                    WritableStreamDefaultController.prototype,
                    "signal"
                  );
                  return [typeof descriptor.get, descriptor.enumerable, descriptor.configurable];
                })()
              });
            })()
            "#,
        )
        .expect("WritableStream controller signal setup should evaluate");
    assert_eq!(
        initial,
        r#"{"events":["start:false"],"signalTag":"[object AbortSignal]","descriptor":["function",true,true]}"#
    );

    for _ in 0..8 {
        vm.eval("0")
            .expect("WritableStream controller signal promise chain should drain");
    }
    let settled = vm
        .eval("JSON.stringify(globalThis.__writableControllerSignalEvents)")
        .expect("WritableStream controller signal events should evaluate");
    assert_eq!(
        settled,
        r#"["start:false","write:one:true:false","signal:stop","sink:stop:true:stop","settled"]"#
    );
}

#[test]
fn pipe_through_resumes_after_transform_backpressure_and_closes_once() {
    let mut vm = new_storage_test_vm("https://transform-backpressure.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__transformBackpressureResult = null;
          const input = ["alpha", "beta", "gamma"];
          let index = 0;
          const source = new ReadableStream({
            pull(controller) {
              if (index === input.length) {
                controller.close();
                return;
              }
              controller.enqueue(input[index++]);
            }
          }, { highWaterMark: 1 });
          globalThis.__transformBackpressureSource = source;
          const output = source.pipeThrough(new TransformStream({
            transform(chunk, controller) {
              controller.enqueue(chunk.toUpperCase());
            },
            flush(controller) {
              controller.enqueue("FLUSH");
            }
          }, { highWaterMark: 1 }, { highWaterMark: 1 }));
          (async () => {
            const values = [];
            for await (const value of output) values.push(value);
            globalThis.__transformBackpressureResult = JSON.stringify({
              values,
              sourceLocked: source.locked,
              outputLocked: output.locked
            });
          })();
        })()
        "#,
    )
    .expect("transform backpressure pipeline should initialize");

    for _ in 0..32 {
        vm.eval("0")
            .expect("transform backpressure pipeline should drain");
    }
    let result = vm
        .eval("globalThis.__transformBackpressureResult")
        .expect("transform backpressure result should evaluate");
    assert_eq!(
        result,
        r#"{"values":["ALPHA","BETA","GAMMA","FLUSH"],"sourceLocked":true,"outputLocked":false}"#
    );
    let source_unlocked = vm
        .eval("String(!globalThis.__transformBackpressureSource.locked)")
        .expect("transform source lock should evaluate after pipe shutdown finalization");
    assert_eq!(source_unlocked, "true");
}

#[test]
fn transform_pipe_observes_source_refill_before_write_and_releases_after_shutdown() {
    let mut vm = new_storage_test_vm("https://transform-pipe-order.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__transformPipeOrderResult = null;
          const lifecycle = [];
          const input = ["alpha", "beta", "gamma"];
          let index = 0;
          const source = new ReadableStream({
            pull(controller) {
              const chunk = input[index];
              if (chunk === undefined) {
                lifecycle.push("source-close");
                controller.close();
                return;
              }
              index += 1;
              lifecycle.push(`source:${chunk}:${String(controller.desiredSize)}`);
              controller.enqueue(chunk);
            }
          }, { highWaterMark: 1 });
          globalThis.__transformPipeOrderSource = source;
          const output = source.pipeThrough(new TransformStream({
            transform(chunk, controller) {
              lifecycle.push(`transform:${chunk}:${String(controller.desiredSize)}`);
              controller.enqueue(`${index}:${chunk.toUpperCase()}`);
            },
            flush(controller) {
              lifecycle.push(`flush:${String(controller.desiredSize)}`);
              controller.enqueue("FLUSH");
            }
          }, { highWaterMark: 1 }, { highWaterMark: 1 }));
          (async () => {
            const reader = output.getReader();
            const values = [];
            while (true) {
              const { value, done } = await reader.read();
              if (done) break;
              values.push(value);
            }
            await reader.closed;
            reader.releaseLock();
            globalThis.__transformPipeOrderResult = JSON.stringify({
              values,
              lifecycle,
              sourceLockedAtReaderClose: source.locked,
              outputLocked: output.locked
            });
          })();
        })()
        "#,
    )
    .expect("transform pipe order should initialize");

    for _ in 0..32 {
        vm.eval("0").expect("transform pipe order should drain");
    }
    let result = vm
        .eval("globalThis.__transformPipeOrderResult")
        .expect("transform pipe order result should evaluate");
    assert_eq!(
        result,
        r#"{"values":["2:ALPHA","3:BETA","3:GAMMA","FLUSH"],"lifecycle":["source:alpha:1","source:beta:1","transform:alpha:1","source:gamma:1","transform:beta:1","source-close","transform:gamma:1","flush:1"],"sourceLockedAtReaderClose":true,"outputLocked":false}"#
    );
    let source_unlocked = vm
        .eval("String(!globalThis.__transformPipeOrderSource.locked)")
        .expect("transform pipe source lock should evaluate after shutdown");
    assert_eq!(source_unlocked, "true");
}

#[test]
fn text_encoder_decoder_pipe_chain_releases_each_pipe_after_one_close() {
    let mut vm = new_storage_test_vm("https://encoding-pipe-close.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__encodingPipeCloseResult = null;
          const input = ["react:", "café:", "東京"];
          let index = 0;
          const source = new ReadableStream({
            pull(controller) {
              if (index === input.length) {
                controller.close();
                return;
              }
              controller.enqueue(input[index++]);
            }
          });
          const pipeline = source
            .pipeThrough(new TextEncoderStream())
            .pipeThrough(new TextDecoderStream("utf-8", { fatal: true }));
          (async () => {
            const values = [];
            for await (const value of pipeline) values.push(value);
            globalThis.__encodingPipeCloseResult = JSON.stringify({
              values,
              combined: values.join(""),
              sourceLocked: source.locked,
              pipelineLocked: pipeline.locked
            });
          })();
        })()
        "#,
    )
    .expect("encoding pipe chain should initialize");

    for _ in 0..48 {
        vm.eval("0").expect("encoding pipe chain should drain");
    }
    let result = vm
        .eval("globalThis.__encodingPipeCloseResult")
        .expect("encoding pipe close result should evaluate");
    assert_eq!(
        result,
        r#"{"values":["react:","café:","東京"],"combined":"react:café:東京","sourceLocked":false,"pipelineLocked":false}"#
    );
}

#[test]
fn readable_byte_stream_byob_reads_transfer_buffers_and_preserve_remainders() {
    let mut vm = new_storage_test_vm("https://byte-stream-byob.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamByobResult = null;
          const sourceBytes = new Uint8Array([1, 2, 3, 127, 128, 254, 255]);
          const lifecycle = [];
          const stream = new ReadableStream({
            type: "bytes",
            start(controller) {
              lifecycle.push(`start:${String(controller.desiredSize)}`);
              lifecycle.push(Object.prototype.toString.call(controller));
              controller.enqueue(sourceBytes);
              lifecycle.push(`enqueue:${sourceBytes.byteLength}`);
              controller.close();
            }
          });
          const reader = stream.getReader({ mode: "byob" });
          (async () => {
            const firstBuffer = new ArrayBuffer(3);
            const first = await reader.read(new Uint8Array(firstBuffer));
            const secondBuffer = new ArrayBuffer(8);
            const second = await reader.read(new Uint8Array(secondBuffer));
            const terminalBuffer = new ArrayBuffer(2);
            const terminal = await reader.read(new Uint8Array(terminalBuffer));
            reader.releaseLock();
            globalThis.__byteStreamByobResult = JSON.stringify({
              first: Array.from(first.value),
              firstInput: firstBuffer.byteLength,
              second: Array.from(second.value),
              secondInput: secondBuffer.byteLength,
              terminal: [terminal.done, terminal.value.byteLength, terminalBuffer.byteLength],
              lifecycle,
              byobReader: reader instanceof ReadableStreamBYOBReader,
              unlocked: !stream.locked
            });
          })();
        })()
        "#,
    )
    .expect("byte stream BYOB reads should initialize");

    for _ in 0..12 {
        vm.eval("0").expect("byte stream BYOB reads should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamByobResult")
        .expect("byte stream BYOB result should evaluate");
    assert_eq!(
        result,
        r#"{"first":[1,2,3],"firstInput":0,"second":[127,128,254,255],"secondInput":0,"terminal":[true,0,0],"lifecycle":["start:0","[object ReadableByteStreamController]","enqueue:0"],"byobReader":true,"unlocked":true}"#
    );
}

#[test]
fn readable_byte_stream_pending_byob_read_is_fulfilled_by_pull() {
    let mut vm = new_storage_test_vm("https://byte-stream-pending-byob.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
              globalThis.__pendingByobResult = null;
              let pulls = 0;
              const stream = new ReadableStream({
                type: "bytes",
                pull(controller) {
                  pulls += 1;
                  controller.enqueue(new Uint8Array([9, 8]));
                  controller.close();
                }
              });
              const reader = new ReadableStreamBYOBReader(stream);
              const input = new ArrayBuffer(4);
              reader.read(new Uint8Array(input)).then(({ value, done }) => {
                reader.releaseLock();
                globalThis.__pendingByobResult = JSON.stringify({
                  value: Array.from(value),
                  done,
                  input: input.byteLength,
                  pulls,
                  unlocked: !stream.locked
                });
              });
              return `${input.byteLength}:${pulls}`;
            })()
            "#,
        )
        .expect("pending BYOB read should initialize");
    assert_eq!(initial, "0:0");

    for _ in 0..8 {
        vm.eval("0").expect("pending BYOB read should drain");
    }
    let result = vm
        .eval("globalThis.__pendingByobResult")
        .expect("pending BYOB result should evaluate");
    assert_eq!(
        result,
        r#"{"value":[9,8],"done":false,"input":0,"pulls":1,"unlocked":true}"#
    );
}

#[test]
fn readable_byte_stream_byob_request_auto_allocation_and_typed_partial_respond_match_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-byob-request.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamByobRequestResult = null;
          const output = {};
          let autoPull = 0;
          const autoStream = new ReadableStream({
            type: "bytes",
            autoAllocateChunkSize: 4,
            pull(controller) {
              autoPull += 1;
              const request = controller.byobRequest;
              output.autoRequest = [
                request === controller.byobRequest,
                request instanceof ReadableStreamBYOBRequest,
                request.view.constructor.name,
                request.view.byteOffset,
                request.view.byteLength
              ];
              request.view.set([7, 8]);
              request.respond(2);
              output.autoStale = [request.view, controller.byobRequest];
              controller.close();
            }
          });

          let typedPull = 0;
          const typedStream = new ReadableStream({
            type: "bytes",
            pull(controller) {
              typedPull += 1;
              const request = controller.byobRequest;
              request.view[0] = typedPull === 1 ? 0x11 : 0x22;
              request.respond(1);
              if (typedPull === 2) controller.close();
            }
          });

          (async () => {
            const auto = await autoStream.getReader().read();
            output.auto = [Array.from(auto.value), auto.done, autoPull];

            const input = new ArrayBuffer(8);
            const typed = await typedStream
              .getReader({ mode: "byob" })
              .read(new Uint16Array(input, 2, 2));
            output.typed = [
              typed.value.constructor.name,
              typed.value.byteOffset,
              typed.value.byteLength,
              typed.value[0],
              input.byteLength,
              typedPull
            ];
            output.surface = [
              ReadableStreamBYOBReader.prototype.read.length,
              ReadableStreamBYOBRequest.prototype.respond.length,
              ReadableStreamBYOBRequest.prototype.respondWithNewView.length,
              Object.prototype.toString.call(Object.getPrototypeOf(controllerForTag()))
            ];
            globalThis.__byteStreamByobRequestResult = JSON.stringify(output);
          })();

          function controllerForTag() {
            let captured;
            new ReadableStream({ type: "bytes", start(controller) { captured = controller; } });
            return captured;
          }
        })()
        "#,
    )
    .expect("BYOB request oracle should initialize");

    for _ in 0..24 {
        vm.eval("0").expect("BYOB request oracle should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamByobRequestResult")
        .expect("BYOB request oracle result should evaluate");
    assert_eq!(
        result,
        r#"{"autoRequest":[true,true,"Uint8Array",0,4],"autoStale":[null,null],"auto":[[7,8],false,1],"typed":["Uint16Array",2,2,8721,0,2],"surface":[1,1,1,"[object ReadableByteStreamController]"]}"#
    );
}

#[test]
fn readable_stream_start_promise_boundary_matches_chromium_wpt() {
    let mut vm = new_storage_test_vm("https://readable-start-boundary.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__readableStartBoundaryResult = "pending";

          (async () => {
            let implicitPulls = 0;
            const implicit = new ReadableStream({
              pull(controller) {
                implicitPulls += 1;
                controller.enqueue("first");
                controller.enqueue("second");
                controller.close();
              }
            }, { highWaterMark: 0 });
            const implicitReader = implicit.getReader();
            const implicitReads = Promise.all([
              implicitReader.read(),
              implicitReader.read()
            ]);
            const implicitSyncPulls = implicitPulls;
            const [implicitFirst, implicitSecond] = await implicitReads;

            let explicitPulls = 0;
            const explicit = new ReadableStream({
              start(controller) {
                controller.enqueue("queued-in-start");
              },
              pull() {
                explicitPulls += 1;
              }
            });
            const explicitRead = explicit.getReader().read();
            const explicitSyncPulls = explicitPulls;
            const explicitResult = await explicitRead;
            await Promise.resolve();

            let closedPulls = 0;
            const closed = new ReadableStream({
              start(controller) {
                controller.enqueue("terminal");
                controller.close();
                return Promise.resolve();
              },
              pull() {
                closedPulls += 1;
              }
            });
            const closedReader = closed.getReader();
            const closedFirst = await closedReader.read();
            const closedSecond = await closedReader.read();
            await closedReader.closed;

            globalThis.__readableStartBoundaryResult = JSON.stringify({
              implicit: {
                syncPulls: implicitSyncPulls,
                pulls: implicitPulls,
                values: [
                  [implicitFirst.value, implicitFirst.done],
                  [implicitSecond.value, implicitSecond.done]
                ]
              },
              explicit: {
                syncPulls: explicitSyncPulls,
                pulls: explicitPulls,
                value: [explicitResult.value, explicitResult.done]
              },
              closed: {
                pulls: closedPulls,
                values: [
                  [closedFirst.value, closedFirst.done],
                  [closedSecond.value === undefined, closedSecond.done]
                ]
              }
            });
          })().catch(error => {
            globalThis.__readableStartBoundaryResult =
              `error:${error && error.name}:${error && error.message}`;
          });
        })()
        "#,
    )
    .expect("ReadableStream start promise boundary setup should evaluate");

    for _ in 0..24 {
        let result = vm
            .eval("globalThis.__readableStartBoundaryResult")
            .expect("ReadableStream start promise boundary should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableStartBoundaryResult")
        .expect("ReadableStream start promise boundary result should evaluate");
    assert_eq!(
        result,
        r#"{"implicit":{"syncPulls":0,"pulls":1,"values":[["first",false],["second",false]]},"explicit":{"syncPulls":0,"pulls":1,"value":["queued-in-start",false]},"closed":{"pulls":0,"values":[["terminal",false],[true,true]]}}"#
    );
}

#[test]
fn readable_byte_stream_respond_after_enqueue_matches_chromium_wpt() {
    let mut vm = new_storage_test_vm("https://byte-stream-respond-after-enqueue.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamRespondAfterEnqueueResult = "pending";
          (async () => {
            const freshStream = new ReadableStream({
              type: "bytes",
              autoAllocateChunkSize: 10,
              pull(controller) {
                controller.enqueue(new Uint8Array([1, 2, 3]));
                controller.byobRequest.respond(10);
              }
            });
            const fresh = await freshStream.getReader().read();

            const cachedStream = new ReadableStream({
              type: "bytes",
              autoAllocateChunkSize: 10,
              pull(controller) {
                const request = controller.byobRequest;
                controller.enqueue(new Uint8Array([1, 2, 3]));
                request.respond(10);
              }
            });
            const cached = await cachedStream.getReader().read();

            const doubleStream = new ReadableStream({
              type: "bytes",
              autoAllocateChunkSize: 10,
              pull(controller) {
                controller.enqueue(new Uint8Array([1, 2, 3]));
                controller.byobRequest.respond(2);
              }
            });
            const doubleReader = doubleStream.getReader();
            const [first, second] = await Promise.all([
              doubleReader.read(),
              doubleReader.read()
            ]);

            globalThis.__byteStreamRespondAfterEnqueueResult = JSON.stringify({
              fresh: [Array.from(fresh.value), fresh.done],
              cached: [Array.from(cached.value), cached.done],
              double: [
                Array.from(first.value),
                first.done,
                Array.from(second.value),
                second.done
              ]
            });
          })().catch(error => {
            globalThis.__byteStreamRespondAfterEnqueueResult =
              `error:${error && error.name}:${error && error.message}`;
          });
        })()
        "#,
    )
    .expect("byte stream respond-after-enqueue WPT should initialize");

    for _ in 0..48 {
        let result = vm
            .eval("globalThis.__byteStreamRespondAfterEnqueueResult")
            .expect("byte stream respond-after-enqueue WPT should drain");
        if result != "pending" {
            break;
        }
    }
    let result = vm
        .eval("globalThis.__byteStreamRespondAfterEnqueueResult")
        .expect("byte stream respond-after-enqueue result should evaluate");
    assert_eq!(
        result,
        r#"{"fresh":[[1,2,3],false],"cached":[[1,2,3],false],"double":[[1,2,3],false,[0,0],false]}"#
    );
}

#[test]
fn readable_byte_stream_commits_all_fillable_descriptors_before_resolving_promises() {
    let mut vm = new_storage_test_vm("https://byte-stream-reentrant-resolution.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamReentrantResolutionResult = null;
          (async () => {
            let controller;
            const stream = new ReadableStream({
              type: "bytes",
              start(value) { controller = value; }
            });
            const reader = stream.getReader({ mode: "byob" });
            const length = 0x4000;
            const read1 = reader.read(new Uint8Array(0x100));
            const read2 = reader.read(
              new BigUint64Array(new ArrayBuffer(length), length - 8, 1)
            );

            let thenObserved = false;
            let requestWasNull = false;
            Object.defineProperty(Object.prototype, "then", {
              configurable: true,
              get() {
                if (!thenObserved) {
                  thenObserved = true;
                  requestWasNull = controller.byobRequest === null;
                }
                return undefined;
              }
            });

            try {
              controller.enqueue(new Uint8Array(0x110).fill(0x42));
              const result1 = await read1;
              const result2 = await read2;
              globalThis.__byteStreamReentrantResolutionResult = JSON.stringify({
                thenObserved,
                requestWasNull,
                first: [
                  result1.done,
                  result1.value.byteLength,
                  result1.value.every(value => value === 0x42)
                ],
                second: [
                  result2.done,
                  result2.value.constructor.name,
                  result2.value.byteOffset,
                  result2.value.length,
                  result2.value[0].toString(16)
                ]
              });
            } finally {
              delete Object.prototype.then;
            }
          })();
        })()
        "#,
    )
    .expect("reentrant BYOB descriptor resolution should initialize");

    for _ in 0..32 {
        vm.eval("0")
            .expect("reentrant BYOB descriptor resolution should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamReentrantResolutionResult")
        .expect("reentrant BYOB descriptor resolution result should evaluate");
    assert_eq!(
        result,
        r#"{"thenObserved":true,"requestWasNull":true,"first":[false,256,true],"second":[false,"BigUint64Array",16376,1,"4242424242424242"]}"#
    );
}

#[test]
fn readable_byte_stream_auto_allocation_close_and_byob_request_surface_match_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-auto-close.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamAutoCloseResult = null;
          const probe = callback => {
            try { return String(callback()); } catch (error) { return error.name; }
          };
          let controller;
          let firstRequest;
          let firstAlias;
          const stream = new ReadableStream({
            type: "bytes",
            autoAllocateChunkSize: 4,
            pull(value) {
              controller = value;
              firstRequest = value.byobRequest;
              firstAlias = firstRequest.view;
              value.close();
            }
          });

          (async () => {
            const terminal = await stream.getReader().read();
            const before = [
              terminal.value === undefined,
              terminal.done,
              firstRequest.view.constructor.name,
              firstRequest.view.byteLength,
              firstAlias.byteLength,
              controller.byobRequest === firstRequest
            ];

            firstRequest.respond(0);
            const secondRequest = controller.byobRequest;
            const secondAlias = secondRequest.view;
            const middle = [
              firstRequest.view,
              firstAlias.byteLength,
              secondRequest === firstRequest,
              secondAlias.constructor.name,
              secondAlias.byteLength
            ];

            secondRequest.respondWithNewView(new Uint8Array(
              secondAlias.buffer,
              secondAlias.byteOffset,
              0
            ));
            const after = [
              secondRequest.view,
              secondAlias.byteLength,
              controller.byobRequest === secondRequest,
              controller.byobRequest.view.byteLength
            ];

            const viewDescriptor = Object.getOwnPropertyDescriptor(
              ReadableStreamBYOBRequest.prototype,
              "view"
            );
            const surface = [
              Object.prototype.toString.call(firstRequest),
              probe(() => new ReadableStreamBYOBRequest()),
              probe(() => viewDescriptor.get.call({})),
              probe(() => ReadableStreamBYOBRequest.prototype.respond.call({}, 0)),
              viewDescriptor.enumerable,
              viewDescriptor.configurable
            ];
            globalThis.__byteStreamAutoCloseResult = JSON.stringify({
              before,
              middle,
              after,
              surface
            });
          })();
        })()
        "#,
    )
    .expect("closed auto-allocation state machine should initialize");

    for _ in 0..32 {
        vm.eval("0")
            .expect("closed auto-allocation state machine should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamAutoCloseResult")
        .expect("closed auto-allocation result should evaluate");
    assert_eq!(
        result,
        r#"{"before":[true,true,"Uint8Array",4,4,true],"middle":[null,0,false,"Uint8Array",4],"after":[null,0,false,4],"surface":["[object ReadableStreamBYOBRequest]","TypeError","TypeError","TypeError",true,true]}"#
    );
}

#[test]
fn readable_byte_stream_min_new_view_release_and_byte_tee_match_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-state-machine.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamStateMachineResult = null;
          (async () => {
            const output = {};

            let pulls = 0;
            const minimumStream = new ReadableStream({
              type: "bytes",
              pull(controller) {
                pulls += 1;
                const request = controller.byobRequest;
                request.view[0] = pulls;
                request.respond(1);
                if (pulls === 4) controller.close();
              }
            });
            const minimumInput = new ArrayBuffer(12);
            const minimum = await minimumStream
              .getReader({ mode: "byob" })
              .read(new Uint16Array(minimumInput, 2, 4), { min: 2 });
            output.minimum = [
              minimum.value.constructor.name,
              minimum.value.byteOffset,
              minimum.value.byteLength,
              Array.from(new Uint8Array(
                minimum.value.buffer,
                minimum.value.byteOffset,
                minimum.value.byteLength
              )),
              minimumInput.byteLength,
              pulls
            ];

            let replacementController;
            const replacementStream = new ReadableStream({
              type: "bytes",
              start(controller) { replacementController = controller; }
            });
            const replacementReader = replacementStream.getReader({ mode: "byob" });
            const replacementPromise = replacementReader.read(
              new Uint8Array(new ArrayBuffer(8), 2, 4)
            );
            const replacementRequest = replacementController.byobRequest;
            const replacement = new Uint8Array(
              replacementRequest.view.buffer,
              replacementRequest.view.byteOffset,
              2
            );
            replacement.set([5, 6]);
            replacementRequest.respondWithNewView(replacement);
            const replacementResult = await replacementPromise;
            output.replacement = [
              Array.from(replacementResult.value),
              replacementResult.value.byteOffset,
              replacementResult.value.buffer.byteLength,
              replacementRequest.view,
              replacementController.byobRequest
            ];

            let releaseController;
            const releaseStream = new ReadableStream({
              type: "bytes",
              start(controller) { releaseController = controller; }
            });
            const firstReader = releaseStream.getReader({ mode: "byob" });
            const releasedRead = firstReader.read(new Uint8Array(4)).catch(error => error.name);
            const releasedRequest = releaseController.byobRequest;
            releasedRequest.view[0] = 9;
            firstReader.releaseLock();
            const secondReader = releaseStream.getReader({ mode: "byob" });
            const replacementRead = secondReader.read(new Uint8Array(4));
            releaseController.enqueue(new Uint8Array([8]));
            releaseController.close();
            output.release = [
              await releasedRead,
              Array.from((await replacementRead).value),
              releasedRequest.view,
              releaseController.byobRequest
            ];

            let sourceRequest;
            const teeSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                sourceRequest = controller.byobRequest;
                controller.enqueue(new Uint8Array([3, 4]));
                controller.close();
              }
            });
            const [left, right] = teeSource.tee();
            const leftRead = left.getReader({ mode: "byob" }).read(new Uint8Array(4));
            const rightRead = right.getReader().read();
            output.tee = [
              Array.from((await leftRead).value),
              Array.from((await rightRead).value),
              sourceRequest !== null,
              sourceRequest.view
            ];

            globalThis.__byteStreamStateMachineResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream state machine oracle should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("byte stream state machine oracle should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamStateMachineResult")
        .expect("byte stream state machine result should evaluate");
    assert_eq!(
        result,
        r#"{"minimum":["Uint16Array",2,4,[1,2,3,4],0,4],"replacement":[[5,6],2,8,null,null],"release":["TypeError",[8],null,null],"tee":[[3,4],[3,4],true,null]}"#
    );
}

#[test]
fn readable_byte_stream_release_hands_retained_descriptors_to_replacement_readers() {
    let mut vm = new_storage_test_vm("https://byte-stream-reader-handoff.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamReaderHandoffResult = null;
          (async () => {
            const output = {};

            let remainderController;
            const remainderStream = new ReadableStream({
              type: "bytes",
              start(controller) { remainderController = controller; }
            });
            const remainderReader1 = remainderStream.getReader({ mode: "byob" });
            const remainderRead1 = remainderReader1
              .read(new Uint8Array([1, 2, 3]))
              .catch(error => error.name);
            await Promise.resolve();
            const remainderRequest = remainderController.byobRequest;
            remainderReader1.releaseLock();
            const remainderReader2 = remainderStream.getReader({ mode: "byob" });
            const remainderRead2 = remainderReader2.read(new Uint8Array([4, 5]));
            const retainedRemainderRequest = remainderController.byobRequest === remainderRequest;
            const retainedRemainderView = Array.from(remainderRequest.view);
            remainderRequest.view.set([6, 7, 8]);
            remainderRequest.respond(3);
            const remainderResult2 = await remainderRead2;
            const remainderResult3 = await remainderReader2.read(new Uint8Array(3));
            output.remainder = [
              await remainderRead1,
              retainedRemainderRequest,
              retainedRemainderView,
              Array.from(remainderResult2.value),
              Array.from(remainderResult3.value),
              remainderRequest.view,
              remainderController.byobRequest
            ];

            let partialController;
            const partialStream = new ReadableStream({
              type: "bytes",
              start(controller) { partialController = controller; }
            });
            const partialReader1 = partialStream.getReader({ mode: "byob" });
            const partialRead1 = partialReader1
              .read(new Uint16Array(1))
              .catch(error => error.name);
            await Promise.resolve();
            const partialRequest1 = partialController.byobRequest;
            partialRequest1.view[0] = 0x11;
            partialRequest1.respond(1);
            const partialRequest2 = partialController.byobRequest;
            partialReader1.releaseLock();
            const partialReader2 = partialStream.getReader({ mode: "byob" });
            const partialRead2 = partialReader2.read(new Uint16Array(1));
            const retainedPartialRequest = partialController.byobRequest === partialRequest2;
            partialRequest2.view[0] = 0x22;
            partialRequest2.respond(1);
            const partialResult = await partialRead2;
            output.partial = [
              await partialRead1,
              retainedPartialRequest,
              Array.from(new Uint8Array(
                partialResult.value.buffer,
                partialResult.value.byteOffset,
                partialResult.value.byteLength
              )),
              partialResult.value.constructor.name,
              partialRequest2.view
            ];

            let closeController;
            const closeStream = new ReadableStream({
              type: "bytes",
              start(controller) { closeController = controller; }
            });
            const closeReader1 = closeStream.getReader({ mode: "byob" });
            const closeRead1 = closeReader1.read(new Uint8Array(3)).catch(error => error.name);
            await Promise.resolve();
            const closeRequest = closeController.byobRequest;
            closeReader1.releaseLock();
            const closeReader2 = closeStream.getReader({ mode: "byob" });
            const closeRead2 = closeReader2.read(new Uint8Array([4, 5, 6]));
            closeController.close();
            const retainedCloseRequest = closeController.byobRequest === closeRequest;
            closeRequest.respond(0);
            const closeResult = await closeRead2;
            output.close = [
              await closeRead1,
              retainedCloseRequest,
              closeResult.done,
              closeResult.value.constructor.name,
              closeResult.value.byteLength,
              Array.from(closeResult.value),
              closeRequest.view,
              closeController.byobRequest
            ];

            let autoController;
            const autoStream = new ReadableStream({
              type: "bytes",
              autoAllocateChunkSize: 4,
              start(controller) { autoController = controller; }
            });
            const autoReader1 = autoStream.getReader();
            const autoRead1 = autoReader1.read().catch(error => error.name);
            await Promise.resolve();
            const autoRequest = autoController.byobRequest;
            autoReader1.releaseLock();
            const autoReader2 = autoStream.getReader();
            const autoRead2 = autoReader2.read();
            const retainedAutoRequest = autoController.byobRequest === autoRequest;
            autoRequest.view[0] = 11;
            autoRequest.respond(1);
            const autoResult = await autoRead2;
            output.auto = [
              await autoRead1,
              retainedAutoRequest,
              Array.from(autoResult.value),
              autoResult.value.buffer.byteLength,
              autoRequest.view,
              autoController.byobRequest
            ];

            let defaultController;
            const defaultStream = new ReadableStream({
              type: "bytes",
              start(controller) { defaultController = controller; }
            });
            const defaultReader1 = defaultStream.getReader({ mode: "byob" });
            const defaultRead1 = defaultReader1
              .read(new Uint16Array(1))
              .catch(error => error.name);
            await Promise.resolve();
            const defaultRequest1 = defaultController.byobRequest;
            defaultRequest1.view[0] = 0x11;
            defaultRequest1.respond(1);
            const defaultRequest2 = defaultController.byobRequest;
            defaultReader1.releaseLock();
            const defaultReader2 = defaultStream.getReader();
            const defaultRead2 = defaultReader2.read();
            defaultController.enqueue(new Uint8Array([0x22]));
            const defaultResult2 = await defaultRead2;
            const defaultResult3 = await defaultReader2.read();
            output.defaultAfterPartial = [
              await defaultRead1,
              Array.from(defaultResult2.value),
              Array.from(defaultResult3.value),
              defaultRequest2.view,
              defaultController.byobRequest
            ];

            let respondDefaultController;
            const respondDefaultStream = new ReadableStream({
              type: "bytes",
              start(controller) { respondDefaultController = controller; }
            });
            const respondDefaultReader1 = respondDefaultStream.getReader({ mode: "byob" });
            const respondDefaultRead1 = respondDefaultReader1
              .read(new Uint8Array(3))
              .catch(error => error.name);
            await Promise.resolve();
            const respondDefaultRequest = respondDefaultController.byobRequest;
            respondDefaultReader1.releaseLock();
            const respondDefaultRead2 = respondDefaultStream.getReader().read();
            respondDefaultRequest.view.set([31, 32]);
            respondDefaultRequest.respond(2);
            output.defaultAfterRespond = [
              await respondDefaultRead1,
              Array.from((await respondDefaultRead2).value),
              respondDefaultRequest.view,
              respondDefaultController.byobRequest
            ];

            globalThis.__byteStreamReaderHandoffResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream reader handoff matrix should initialize");

    for _ in 0..96 {
        vm.eval("0")
            .expect("byte stream reader handoff matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamReaderHandoffResult")
        .expect("byte stream reader handoff result should evaluate");
    assert_eq!(
        result,
        r#"{"remainder":["TypeError",true,[1,2,3],[6,7],[8],null,null],"partial":["TypeError",true,[17,34],"Uint16Array",null],"close":["TypeError",true,true,"Uint8Array",0,[],null,null],"auto":["TypeError",true,[11],4,null,null],"defaultAfterPartial":["TypeError",[17],[34],null,null],"defaultAfterRespond":["TypeError",[31,32],null,null]}"#
    );
}
