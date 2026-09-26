use super::*;

#[test]
fn readable_byte_stream_tee_preserves_byob_views_terminal_types_and_cancel_reasons() {
    let mut vm = new_storage_test_vm("https://byte-stream-tee-matrix.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamTeeMatrixResult = null;
          (async () => {
            const output = {};

            let sourceView;
            const offsetSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                const request = controller.byobRequest;
                sourceView = [
                  request !== null,
                  request.view.buffer.byteLength,
                  request.view.byteOffset,
                  request.view.byteLength
                ];
                request.view.set([1, 2, 3]);
                request.respond(3);
                controller.close();
              }
            });
            const [offsetLeft, offsetRight] = offsetSource.tee();
            const offsetInput = new ArrayBuffer(16);
            const offsetLeftRead = offsetLeft
              .getReader({ mode: "byob" })
              .read(new Uint8Array(offsetInput, 4, 6));
            const offsetRightRead = offsetRight.getReader().read();
            const offsetLeftResult = await offsetLeftRead;
            const offsetRightResult = await offsetRightRead;
            offsetLeftResult.value[0] = 9;
            output.offset = [
              sourceView,
              [
                Array.from(offsetLeftResult.value),
                offsetLeftResult.value.buffer.byteLength,
                offsetLeftResult.value.byteOffset,
                offsetInput.byteLength
              ],
              [
                Array.from(offsetRightResult.value),
                offsetRightResult.value.buffer.byteLength,
                offsetRightResult.value.byteOffset
              ]
            ];

            let defaultSourceSawNull;
            const defaultFirstSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                defaultSourceSawNull = controller.byobRequest === null;
                controller.enqueue(new Uint8Array([4, 5]));
                controller.close();
              }
            });
            const [defaultFirstLeft, defaultFirstRight] = defaultFirstSource.tee();
            const defaultFirstRightRead = defaultFirstRight.getReader().read();
            await Promise.resolve();
            const defaultFirstLeftRead = defaultFirstLeft
              .getReader({ mode: "byob" })
              .read(new Uint8Array(4));
            output.defaultFirst = [
              defaultSourceSawNull,
              Array.from((await defaultFirstLeftRead).value),
              Array.from((await defaultFirstRightRead).value)
            ];

            let terminalSourceSawRequest;
            const terminalSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                const request = controller.byobRequest;
                terminalSourceSawRequest = request !== null;
                controller.close();
                request.respond(0);
              }
            });
            const [terminalLeft, terminalRight] = terminalSource.tee();
            const terminalLeftRead = terminalLeft
              .getReader({ mode: "byob" })
              .read(new Uint16Array(new ArrayBuffer(8), 2, 2));
            const terminalRightRead = terminalRight
              .getReader({ mode: "byob" })
              .read(new Uint32Array(2));
            const terminalLeftResult = await terminalLeftRead;
            const terminalRightResult = await terminalRightRead;
            output.terminal = [
              terminalSourceSawRequest,
              [
                terminalLeftResult.done,
                terminalLeftResult.value.constructor.name,
                terminalLeftResult.value.byteOffset,
                terminalLeftResult.value.byteLength,
                terminalLeftResult.value.buffer.byteLength
              ],
              [
                terminalRightResult.done,
                terminalRightResult.value.constructor.name,
                terminalRightResult.value.byteOffset,
                terminalRightResult.value.byteLength,
                terminalRightResult.value.buffer.byteLength
              ]
            ];

            let cancelReason;
            let cancelCalls = 0;
            const cancelSource = new ReadableStream({
              type: "bytes",
              cancel(reason) {
                cancelCalls += 1;
                cancelReason = reason;
                return 7;
              }
            });
            const [cancelLeft, cancelRight] = cancelSource.tee();
            let firstCancelSettled = false;
            const firstCancel = cancelLeft.cancel("left").then(value => {
              firstCancelSettled = true;
              return value;
            });
            await Promise.resolve();
            const settledBeforeSecondCancel = firstCancelSettled;
            const secondCancel = cancelRight.cancel("right");
            const cancelValues = await Promise.all([firstCancel, secondCancel]);
            output.cancel = [
              settledBeforeSecondCancel,
              cancelCalls,
              cancelReason,
              cancelValues
            ];

            globalThis.__byteStreamTeeMatrixResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream tee matrix should initialize");

    for _ in 0..128 {
        vm.eval("0").expect("byte stream tee matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamTeeMatrixResult")
        .expect("byte stream tee matrix result should evaluate");
    assert_eq!(
        result,
        r#"{"offset":[[true,16,4,6],[[9,2,3],16,4,0],[[1,2,3],3,0]],"defaultFirst":[true,[4,5],[4,5]],"terminal":[true,[true,"Uint16Array",2,0,8],[true,"Uint32Array",0,0,8]],"cancel":[false,1,["left","right"],[null,null]]}"#
    );
}

#[test]
fn readable_byte_stream_tee_propagates_demand_error_and_single_branch_cancel() {
    let mut vm = new_storage_test_vm("https://byte-stream-tee-lifecycle.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamTeeLifecycleResult = null;
          (async () => {
            const output = {};

            let idlePulls = 0;
            const idleSource = new ReadableStream({
              type: "bytes",
              pull() { idlePulls += 1; }
            });
            idleSource.tee();
            await Promise.resolve();
            await Promise.resolve();
            output.noDemandPulls = idlePulls;

            const sourceError = new Error("tee-source-error");
            const errorSource = new ReadableStream({
              type: "bytes",
              pull(controller) { controller.error(sourceError); }
            });
            const [errorLeft, errorRight] = errorSource.tee();
            const errorLeftReader = errorLeft.getReader();
            const errorRightReader = errorRight.getReader({ mode: "byob" });
            const errorResults = await Promise.all([
              errorLeftReader.read().then(
                () => false,
                error => error === sourceError
              ),
              errorRightReader.read(new Uint8Array(4)).then(
                () => false,
                error => error === sourceError
              ),
              errorLeftReader.closed.then(
                () => false,
                error => error === sourceError
              ),
              errorRightReader.closed.then(
                () => false,
                error => error === sourceError
              )
            ]);
            output.error = errorResults;

            let cancelCalls = 0;
            let produced = 0;
            const cancelSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                produced += 1;
                controller.enqueue(new Uint8Array([produced]));
                if (produced === 3) controller.close();
              },
              cancel() { cancelCalls += 1; }
            });
            const [cancelLeft, cancelRight] = cancelSource.tee();
            let firstCancelSettled = false;
            const firstCancel = cancelLeft.cancel("unused").then(value => {
              firstCancelSettled = true;
              return value;
            });
            await Promise.resolve();
            const settledBeforeDrain = firstCancelSettled;
            const cancelRightReader = cancelRight.getReader({ mode: "byob" });
            const drained = [];
            for (;;) {
              const result = await cancelRightReader.read(new Uint8Array(2));
              if (result.done) break;
              drained.push(...result.value);
            }
            output.singleCancel = [
              settledBeforeDrain,
              await firstCancel,
              drained,
              produced,
              cancelCalls
            ];

            let bufferedProduced = 0;
            const bufferedSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                bufferedProduced += 1;
                controller.enqueue(new Uint8Array([10 + bufferedProduced]));
                if (bufferedProduced === 3) controller.close();
              }
            });
            const [fast, slow] = bufferedSource.tee();
            const fastReader = fast.getReader();
            const fastValues = [];
            for (;;) {
              const result = await fastReader.read();
              if (result.done) break;
              fastValues.push(...result.value);
            }
            const slowReader = slow.getReader({ mode: "byob" });
            const slowValues = [];
            for (;;) {
              const result = await slowReader.read(new Uint8Array(1));
              if (result.done) break;
              slowValues.push(...result.value);
            }
            output.fastSlow = [fastValues, slowValues, bufferedProduced];

            globalThis.__byteStreamTeeLifecycleResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream tee lifecycle matrix should initialize");

    for _ in 0..160 {
        vm.eval("0")
            .expect("byte stream tee lifecycle matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamTeeLifecycleResult")
        .expect("byte stream tee lifecycle result should evaluate");
    assert_eq!(
        result,
        r#"{"noDemandPulls":0,"error":[true,true,true,true],"singleCancel":[false,null,[1,2,3],3,0],"fastSlow":[[11,12,13],[11,12,13],3]}"#
    );
}

#[test]
fn readable_byte_stream_tee_preserves_original_view_and_serializes_byob_close() {
    let mut vm = new_storage_test_vm("https://byte-stream-tee-read-owner.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteTeeReadOwnerResult = null;
          (async () => {
            const output = {};

            let offsetPulls = 0;
            const offsetSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                offsetPulls += 1;
                if (offsetPulls === 1) {
                  const buffer = new Uint8Array([1, 2, 3]).buffer;
                  controller.enqueue(new Uint8Array(buffer, 2));
                }
              }
            });
            const [offsetLeft, offsetRight] = offsetSource.tee();
            const [leftResult, rightResult] = await Promise.all([
              offsetLeft.getReader().read(),
              offsetRight.getReader().read()
            ]);
            output.offset = [
              [
                leftResult.value.byteOffset,
                leftResult.value.byteLength,
                leftResult.value.buffer.byteLength,
                Array.from(leftResult.value)
              ],
              [
                rightResult.value.byteOffset,
                rightResult.value.byteLength,
                rightResult.value.buffer.byteLength,
                Array.from(rightResult.value)
              ],
              leftResult.value.buffer !== rightResult.value.buffer
            ];

            let closeController;
            const closeSource = new ReadableStream({
              type: "bytes",
              start(controller) { closeController = controller; }
            });
            const [closeLeft, closeRight] = closeSource.tee();
            const closeLeftReader = closeLeft.getReader({ mode: "byob" });
            const closeRightReader = closeRight.getReader({ mode: "byob" });
            const reads = [
              closeLeftReader.read(new Uint8Array(1)),
              closeLeftReader.read(new Uint8Array(1)),
              closeRightReader.read(new Uint8Array(1)),
              closeRightReader.read(new Uint8Array(1))
            ];
            while (closeController.byobRequest === null) {
              await Promise.resolve();
            }
            closeController.byobRequest.view[0] = 0x11;
            closeController.byobRequest.respond(1);
            closeController.close();
            const closeResults = await Promise.all(reads);
            output.close = closeResults.map(result => [
              Array.from(result.value),
              result.value.byteOffset,
              result.value.byteLength,
              result.value.buffer.byteLength,
              result.done
            ]);

            globalThis.__byteTeeReadOwnerResult = JSON.stringify(output);
          })().catch(error => {
            globalThis.__byteTeeReadOwnerResult = `error:${error.name}:${error.message}`;
          });
        })()
        "#,
    )
    .expect("byte tee read owner matrix should initialize");

    for _ in 0..96 {
        vm.eval("0")
            .expect("byte tee read owner matrix should drain");
    }
    assert_eq!(
        vm.eval("globalThis.__byteTeeReadOwnerResult")
            .expect("byte tee read owner matrix result should evaluate"),
        r#"{"offset":[[2,1,3,[3]],[0,1,1,[3]],true],"close":[[[17],0,1,1,false],[[],0,0,1,true],[[17],0,1,1,false],[[],0,0,1,true]]}"#
    );
}

#[test]
fn readable_byte_stream_close_and_invalidated_request_paths_match_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-terminal-state.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamTerminalResult = null;
          (async () => {
            const output = {};
            let closeController;
            const closeStream = new ReadableStream({
              type: "bytes",
              start(controller) { closeController = controller; }
            });
            const closeReader = closeStream.getReader({ mode: "byob" });
            const closeRead = closeReader.read(new Uint16Array(4), { min: 2 });
            const partialRequest = closeController.byobRequest;
            partialRequest.view.set([0x11, 0x22]);
            partialRequest.respond(2);
            const terminalRequest = closeController.byobRequest;
            closeController.close();
            terminalRequest.respond(0);
            const partial = await closeRead;
            const terminal = await closeReader.read(new Uint16Array(2));
            output.close = [
              Array.from(new Uint8Array(
                partial.value.buffer,
                partial.value.byteOffset,
                partial.value.byteLength
              )),
              partial.done,
              terminal.value.constructor.name,
              terminal.value.byteLength,
              terminal.done,
              terminalRequest.view
            ];

            let enqueueController;
            const enqueueStream = new ReadableStream({
              type: "bytes",
              start(controller) { enqueueController = controller; }
            });
            const enqueueReader = enqueueStream.getReader({ mode: "byob" });
            const enqueueRead = enqueueReader.read(new Uint16Array(4));
            const stale = enqueueController.byobRequest;
            const staleView = stale.view;
            enqueueController.enqueue(new Uint8Array([1, 2, 3]));
            const enqueueResult = await enqueueRead;
            let staleError;
            try { stale.respond(1); } catch (error) { staleError = error.name; }
            output.enqueue = [
              Array.from(new Uint8Array(
                enqueueResult.value.buffer,
                enqueueResult.value.byteOffset,
                enqueueResult.value.byteLength
              )),
              stale.view,
              staleView.byteLength,
              staleError
            ];

            let alignmentController;
            const alignmentStream = new ReadableStream({
              type: "bytes",
              start(controller) { alignmentController = controller; }
            });
            const alignmentReader = alignmentStream.getReader({ mode: "byob" });
            const alignmentRead = alignmentReader
              .read(new Uint16Array(4), { min: 2 })
              .catch(error => error.name);
            const alignmentRequest = alignmentController.byobRequest;
            alignmentRequest.view[0] = 1;
            alignmentRequest.respond(1);
            let closeError;
            try { alignmentController.close(); } catch (error) { closeError = error.name; }
            output.alignment = [closeError, await alignmentRead];

            const queuedCloseStream = new ReadableStream({
              type: "bytes",
              start(controller) {
                controller.enqueue(new Uint8Array([1, 2, 3]));
                controller.close();
              }
            });
            const queuedCloseReader = queuedCloseStream.getReader({ mode: "byob" });
            const queuedCloseFirst = await queuedCloseReader.read(new Uint16Array(2));
            const queuedCloseSecond = await queuedCloseReader
              .read(new Uint16Array(2))
              .then(value => value, error => error.name);
            output.queuedClose = [
              Array.from(new Uint8Array(
                queuedCloseFirst.value.buffer,
                queuedCloseFirst.value.byteOffset,
                queuedCloseFirst.value.byteLength
              )),
              queuedCloseFirst.done,
              queuedCloseSecond,
              await queuedCloseReader.closed.then(() => "closed", error => error.name)
            ];

            globalThis.__byteStreamTerminalResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream terminal oracle should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("byte stream terminal oracle should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamTerminalResult")
        .expect("byte stream terminal result should evaluate");
    assert_eq!(
        result,
        r#"{"close":[[17,34],true,"Uint16Array",0,true,null],"enqueue":[[1,2],null,0,"TypeError"],"alignment":["TypeError","TypeError"],"queuedClose":[[1,2],false,"TypeError","TypeError"]}"#
    );
}

#[test]
fn readable_byte_stream_cancel_and_error_settle_pending_byob_reads() {
    let mut vm = new_storage_test_vm("https://byte-stream-cancel-error.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamCancelErrorResult = null;
          (async () => {
            const output = {};

            let cancelController;
            let cancelReason;
            const cancelStream = new ReadableStream({
              type: "bytes",
              start(controller) { cancelController = controller; },
              cancel(reason) { cancelReason = reason; }
            });
            const cancelReader = cancelStream.getReader({ mode: "byob" });
            const cancelInput = new ArrayBuffer(4);
            const cancelRead = cancelReader.read(new Uint16Array(cancelInput));
            await Promise.resolve();
            const partialRequest = cancelController.byobRequest;
            partialRequest.view[0] = 7;
            partialRequest.respond(1);
            const terminalRequest = cancelController.byobRequest;
            const reason = { kind: "stop" };
            const cancelResult = cancelReader.cancel(reason);
            const canceledRead = await cancelRead;
            output.cancel = [
              canceledRead.value,
              canceledRead.done,
              await cancelResult,
              cancelReason === reason,
              cancelInput.byteLength,
              terminalRequest.view,
              cancelController.byobRequest,
              await cancelReader.closed.then(() => "closed", error => error.name)
            ];
            output.staleAfterCancel = (() => {
              try { terminalRequest.respond(0); return "resolved"; }
              catch (error) { return error.name; }
            })();

            let errorController;
            const streamError = { kind: "source-error" };
            const errorStream = new ReadableStream({
              type: "bytes",
              start(controller) { errorController = controller; }
            });
            const errorReader = errorStream.getReader({ mode: "byob" });
            const errorInput = new ArrayBuffer(4);
            const errorRead = errorReader
              .read(new Uint8Array(errorInput))
              .then(() => false, error => error === streamError);
            const errorClosed = errorReader.closed
              .then(() => false, error => error === streamError);
            await Promise.resolve();
            const errorRequest = errorController.byobRequest;
            errorController.error(streamError);
            output.error = [
              await errorRead,
              await errorClosed,
              errorInput.byteLength,
              errorRequest.view,
              errorController.byobRequest
            ];

            const alreadyErroredReason = { kind: "already" };
            const alreadyErrored = new ReadableStream({
              type: "bytes",
              start(controller) { controller.error(alreadyErroredReason); }
            });
            const untouchedInput = new ArrayBuffer(8);
            const alreadyResult = await alreadyErrored
              .getReader({ mode: "byob" })
              .read(new Uint8Array(untouchedInput))
              .then(() => false, error => error === alreadyErroredReason);
            output.alreadyErrored = [alreadyResult, untouchedInput.byteLength];

            globalThis.__byteStreamCancelErrorResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream cancel/error matrix should initialize");

    for _ in 0..64 {
        vm.eval("0")
            .expect("byte stream cancel/error matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamCancelErrorResult")
        .expect("byte stream cancel/error result should evaluate");
    assert_eq!(
        result,
        r#"{"cancel":[null,true,null,true,0,null,null,"closed"],"staleAfterCancel":"TypeError","error":[true,true,0,null,null],"alreadyErrored":[true,8]}"#
    );
}

#[test]
fn readable_byte_stream_preserves_every_array_buffer_view_brand_and_offset() {
    let mut vm = new_storage_test_vm("https://byte-stream-view-brands.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamViewBrandsResult = null;
          (async () => {
            const cases = [
              ["DataView", buffer => new DataView(buffer, 8, 16), 1],
              ["Int8Array", buffer => new Int8Array(buffer, 8, 16), 1],
              ["Uint8Array", buffer => new Uint8Array(buffer, 8, 16), 1],
              ["Uint8ClampedArray", buffer => new Uint8ClampedArray(buffer, 8, 16), 1],
              ["Int16Array", buffer => new Int16Array(buffer, 8, 8), 2],
              ["Uint16Array", buffer => new Uint16Array(buffer, 8, 8), 2],
              ["Int32Array", buffer => new Int32Array(buffer, 8, 4), 4],
              ["Uint32Array", buffer => new Uint32Array(buffer, 8, 4), 4],
              ["Float16Array", buffer => new Float16Array(buffer, 8, 8), 2],
              ["Float32Array", buffer => new Float32Array(buffer, 8, 4), 4],
              ["Float64Array", buffer => new Float64Array(buffer, 8, 2), 8],
              ["BigInt64Array", buffer => new BigInt64Array(buffer, 8, 2), 8],
              ["BigUint64Array", buffer => new BigUint64Array(buffer, 8, 2), 8]
            ];
            const output = [];
            for (const [name, makeView, elementSize] of cases) {
              let controller;
              const stream = new ReadableStream({
                type: "bytes",
                start(value) { controller = value; }
              });
              const reader = stream.getReader({ mode: "byob" });
              const input = new ArrayBuffer(48);
              const read = reader.read(makeView(input));
              const request = controller.byobRequest;
              for (let index = 0; index < elementSize; index += 1) {
                request.view[index] = index + 1;
              }
              request.respond(elementSize);
              controller.close();
              const result = await read;
              output.push([
                name,
                result.value.constructor.name,
                result.value.byteOffset,
                result.value.byteLength,
                result.value.buffer.byteLength,
                input.byteLength,
                result.done
              ]);
            }
            globalThis.__byteStreamViewBrandsResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream view-brand matrix should initialize");

    for _ in 0..96 {
        vm.eval("0")
            .expect("byte stream view-brand matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamViewBrandsResult")
        .expect("byte stream view-brand matrix result should evaluate");
    assert_eq!(
        result,
        r#"[["DataView","DataView",8,1,48,0,false],["Int8Array","Int8Array",8,1,48,0,false],["Uint8Array","Uint8Array",8,1,48,0,false],["Uint8ClampedArray","Uint8ClampedArray",8,1,48,0,false],["Int16Array","Int16Array",8,2,48,0,false],["Uint16Array","Uint16Array",8,2,48,0,false],["Int32Array","Int32Array",8,4,48,0,false],["Uint32Array","Uint32Array",8,4,48,0,false],["Float16Array","Float16Array",8,2,48,0,false],["Float32Array","Float32Array",8,4,48,0,false],["Float64Array","Float64Array",8,8,48,0,false],["BigInt64Array","BigInt64Array",8,8,48,0,false],["BigUint64Array","BigUint64Array",8,8,48,0,false]]"#
    );
}

#[test]
fn readable_byte_stream_rejects_non_transferable_buffers_and_read_option_failures() {
    let mut vm = new_storage_test_vm("https://byte-stream-invalid-buffers.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamInvalidBuffersResult = null;
          (async () => {
            const output = {};
            let controller;
            const stream = new ReadableStream({
              type: "bytes",
              start(value) { controller = value; }
            });
            const reader = stream.getReader({ mode: "byob" });

            const resizable = new ArrayBuffer(16, { maxByteLength: 32 });
            output.resizableRead = await reader
              .read(new Uint8Array(resizable, 4, 8))
              .then(() => "resolved", error => error.name);
            let enqueueResizable;
            try {
              controller.enqueue(new Uint8Array(resizable, 4, 8));
              enqueueResizable = "resolved";
            } catch (error) {
              enqueueResizable = error.name;
            }
            output.resizable = [
              output.resizableRead,
              enqueueResizable,
              resizable.byteLength,
              controller.byobRequest
            ];

            const memory = new WebAssembly.Memory({ initial: 1 });
            output.wasm = await reader
              .read(new Uint8Array(memory.buffer, 0, 8))
              .then(() => "resolved", error => error.name);

            let getterCount = 0;
            const getterError = new Error("minimum getter");
            output.getter = await reader
              .read(new Uint8Array(8), {
                get min() {
                  getterCount += 1;
                  throw getterError;
                }
              })
              .then(
                () => "resolved",
                error => [error === getterError, error.message, getterCount]
              );
            output.minimums = await Promise.all([
              reader.read(new Uint8Array(8), { min: 0 })
                .then(() => "resolved", error => error.name),
              reader.read(new Uint16Array(2), { min: 3 })
                .then(() => "resolved", error => error.name),
              reader.read({}, {
                get min() { getterCount += 100; return 1; }
              }).then(() => "resolved", error => [error.name, getterCount])
            ]);
            globalThis.__byteStreamInvalidBuffersResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream invalid-buffer matrix should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("byte stream invalid-buffer matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamInvalidBuffersResult")
        .expect("byte stream invalid-buffer matrix result should evaluate");
    assert_eq!(
        result,
        r#"{"resizableRead":"TypeError","resizable":["TypeError","TypeError",16,null],"wasm":"TypeError","getter":[true,"minimum getter",1],"minimums":["TypeError","RangeError",["TypeError",1]]}"#
    );
}

#[test]
fn readable_byte_stream_respond_with_new_view_validation_order_matches_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-new-view-validation.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamNewViewValidationResult = null;
          (async () => {
            const exceptionName = callback => {
              try { callback(); return "resolved"; }
              catch (error) { return error.name; }
            };
            const requestFor = async closed => {
              let controller;
              const stream = new ReadableStream({
                type: "bytes",
                start(value) { controller = value; }
              });
              const reader = stream.getReader({ mode: "byob" });
              reader.read(new Uint8Array([4, 5, 6])).catch(() => {});
              await Promise.resolve();
              if (closed) controller.close();
              return [controller, controller.byobRequest];
            };

            const [, readableDetachedRequest] = await requestFor(false);
            const readableDetached = new Uint8Array([1, 2, 3]);
            readableDetached.buffer.transfer();

            const [, readableZeroRequest] = await requestFor(false);
            const [, readableSubviewRequest] = await requestFor(false);
            const [closedZeroController, closedZeroRequest] = await requestFor(true);
            const [, closedDetachedRequest] = await requestFor(true);
            const closedDetached = new Uint8Array([1, 2, 3]);
            closedDetached.buffer.transfer();

            let movedController;
            const movedStream = new ReadableStream({
              type: "bytes",
              start(value) { movedController = value; }
            });
            const movedReader = movedStream.getReader({ mode: "byob" });
            const movedRead = movedReader.read(new Uint8Array([4, 5, 6]));
            await Promise.resolve();
            const movedRequest = movedController.byobRequest;
            const movedBuffer = movedRequest.view.buffer.transfer();
            const movedView = new Uint8Array(movedBuffer, 0, 1);
            movedView[0] = 42;
            const movedCall = exceptionName(() => movedRequest.respondWithNewView(movedView));
            const movedResult = await movedRead;

            globalThis.__byteStreamNewViewValidationResult = JSON.stringify({
              readableDetached: exceptionName(() =>
                readableDetachedRequest.respondWithNewView(readableDetached)),
              readableZeroBuffer: exceptionName(() =>
                readableZeroRequest.respondWithNewView(new Uint8Array())),
              readableZeroSubview: exceptionName(() =>
                readableSubviewRequest.respondWithNewView(
                  new Uint8Array(readableSubviewRequest.view.buffer, 0, 0)
                )),
              closedZeroBuffer: exceptionName(() =>
                closedZeroRequest.respondWithNewView(new Uint8Array())),
              closedDetached: exceptionName(() =>
                closedDetachedRequest.respondWithNewView(closedDetached)),
              movedOriginal: [
                movedCall,
                Array.from(movedResult.value),
                movedResult.value.buffer.byteLength,
                movedRequest.view
              ],
              closedStillHasRequest: closedZeroController.byobRequest === closedZeroRequest
            });
          })();
        })()
        "#,
    )
    .expect("respondWithNewView validation matrix should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("respondWithNewView validation matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamNewViewValidationResult")
        .expect("respondWithNewView validation result should evaluate");
    assert_eq!(
        result,
        r#"{"readableDetached":"TypeError","readableZeroBuffer":"TypeError","readableZeroSubview":"TypeError","closedZeroBuffer":"RangeError","closedDetached":"TypeError","movedOriginal":["resolved",[42],3,null],"closedStillHasRequest":true}"#
    );
}

#[test]
fn readable_byte_stream_webidl_dictionary_conversion_matches_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-webidl-conversion.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamWebIdlResult = null;
          (async () => {
            const primitiveResults = [];
            for (const options of [1, "x", true, Symbol("options")]) {
              const stream = new ReadableStream({
                type: "bytes",
                start(controller) {
                  controller.enqueue(new Uint8Array([9]));
                  controller.close();
                }
              });
              const result = await stream
                .getReader({ mode: "byob" })
                .read(new Uint8Array(2), options);
              primitiveResults.push([Array.from(result.value), result.done]);
            }

            const inheritedOptions = Object.create({ min: 2 });
            const inheritedStream = new ReadableStream({
              type: "bytes",
              start(controller) {
                controller.enqueue(new Uint8Array([1, 2]));
                controller.close();
              }
            });
            const inherited = await inheritedStream
              .getReader({ mode: "byob" })
              .read(new Uint8Array(4), inheritedOptions);

            const constructorResult = callback => {
              try { callback(); return "constructed"; }
              catch (error) { return error.name; }
            };
            globalThis.__byteStreamWebIdlResult = JSON.stringify({
              primitiveResults,
              inherited: [Array.from(inherited.value), inherited.done],
              autoAllocateZero: [
                constructorResult(() => new ReadableStream({ autoAllocateChunkSize: 0 })),
                constructorResult(() => new ReadableStream({
                  type: "bytes",
                  autoAllocateChunkSize: 0
                }))
              ]
            });
          })();
        })()
        "#,
    )
    .expect("byte stream Web IDL conversion matrix should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("byte stream Web IDL conversion matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamWebIdlResult")
        .expect("byte stream Web IDL conversion result should evaluate");
    assert_eq!(
        result,
        r#"{"primitiveResults":[[[9],false],[[9],false],[[9],false],[[9],false]],"inherited":[[1,2],false],"autoAllocateZero":["constructed","TypeError"]}"#
    );
}

#[test]
fn blob_and_fetch_body_streams_expose_the_readable_byte_stream_byob_contract() {
    let mut vm = new_storage_test_vm("https://body-byte-streams.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__bodyByteStreamsResult = null;
          (async () => {
            const blobReader = new Blob([new Uint8Array([1, 2, 3, 4])])
              .stream()
              .getReader({ mode: "byob" });
            const blob = await blobReader.read(new Uint8Array(8));
            const blobDone = await blobReader.read(new Uint8Array(2));

            const response = new Response(new Uint8Array([5, 6, 7]));
            const responseReader = response.body.getReader({ mode: "byob" });
            const body = await responseReader.read(new Uint8Array(8));
            const bodyDone = await responseReader.read(new Uint8Array(2));

            globalThis.__bodyByteStreamsResult = JSON.stringify({
              blob: [Array.from(blob.value), blob.done, blobDone.value.byteLength, blobDone.done],
              response: [Array.from(body.value), body.done, bodyDone.value.byteLength, bodyDone.done],
              bodyUsed: response.bodyUsed
            });
          })();
        })()
        "#,
    )
    .expect("body byte streams should initialize");

    for _ in 0..48 {
        vm.eval("0").expect("body byte streams should drain");
    }
    let result = vm
        .eval("globalThis.__bodyByteStreamsResult")
        .expect("body byte stream result should evaluate");
    assert_eq!(
        result,
        r#"{"blob":[[1,2,3,4],false,0,true],"response":[[5,6,7],false,0,true],"bodyUsed":true}"#
    );
}

#[test]
fn plain_data_surfaces_preserve_expected_shapes() {
    let mut vm = new_storage_test_vm("https://plain-data-shapes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = new Uint8Array(8);
  const encoded = new TextEncoder().encodeInto('hé', target);
  const params = new URLSearchParams('a=1&a=2&b=3');
  const headers = new Headers([['x-a', '1'], ['x-b', '2']]);
  return JSON.stringify({
    encodeInto: [Object.keys(encoded).join(','), encoded.read, encoded.written, Array.from(target.slice(0, 3)).join(',')].join('|'),
    params: [params.getAll('a').join(','), Array.from(params.entries()).map((pair) => pair.join('=')).join(',')].join('|'),
    headers: [
      Array.from(headers.keys()).join(','),
      Array.from(headers.values()).join(','),
      Array.from(headers.entries()).map((pair) => pair.join('=')).join(',')
    ].join('|'),
    perfTiming: Object.keys(performance.timing).join(','),
    perfNavigation: [Object.keys(performance.navigation).join(','), performance.navigation.type, performance.navigation.redirectCount].join('|'),
    perfSupported: PerformanceObserver.supportedEntryTypes.join(',')
  });
})()
"#,
        )
        .expect("serde_v8 plain data shape probe should evaluate");

    assert_eq!(
        result,
        r#"{"encodeInto":"read,written|2|3|104,195,169","params":"1,2|a=1,a=2,b=3","headers":"x-a,x-b|1,2|x-a=1,x-b=2","perfTiming":"navigationStart,unloadEventStart,unloadEventEnd,redirectStart,redirectEnd,fetchStart,domainLookupStart,domainLookupEnd,connectStart,connectEnd,secureConnectionStart,requestStart,responseStart,responseEnd,domLoading,domInteractive,domContentLoadedEventStart,domContentLoadedEventEnd,domComplete,loadEventStart,loadEventEnd","perfNavigation":"|0|0","perfSupported":"mark,measure,navigation,resource"}"#
    );
}

#[test]
fn performance_observer_callbacks_apply_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://performance-observer-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const values = [];
  values.push(probe(() => new PerformanceObserver()));
  values.push(probe(() => new PerformanceObserver({})));

  const observer = new PerformanceObserver(() => {});
  values.push(observer instanceof PerformanceObserver);
  values.push(probe(() => observer.observe()));
  values.push(probe(() => observer.observe(null)));
  values.push(probe(() => observer.observe({})));
  values.push(probe(() => observer.observe({ type: Symbol('entry-type') })));
  values.push(probe(() => observer.observe({ entryTypes: Symbol('entry-types') })));
  values.push(probe(() => observer.observe({ entryTypes: [Symbol('entry-type')] })));

  let typeCalls = 0;
  let bufferedValueOfCalls = 0;
  values.push(probe(() => {
    observer.observe({
      type: { toString() { typeCalls += 1; return 'mark'; } },
      buffered: { valueOf() { bufferedValueOfCalls += 1; return false; } },
    });
    return `${typeCalls}:${bufferedValueOfCalls}`;
  }));

  values.push(probe(() => observer.observe({ entryTypes: ['measure'] })));

  const entryTypesObserver = new PerformanceObserver(() => {});
  let entryTypeCalls = 0;
  values.push(probe(() => {
    entryTypesObserver.observe({
      entryTypes: ['mark', { toString() { entryTypeCalls += 1; return 'mark'; } }, 'measure', 'mark'],
    });
    return entryTypeCalls;
  }));
  performance.mark('po-webidl');
  values.push(entryTypesObserver.takeRecords().map((entry) => `${entry.entryType}:${entry.name}`).join(','));
  return values.join('|');
})()
"#,
        )
        .expect("PerformanceObserver WebIDL conversion probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|true|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|1:0|throw:InvalidModificationError|1|mark:po-webidl"
    );
}

#[test]
fn webidl_sequence_conversion_uses_iterator_without_mutable_array_from() {
    let mut vm = new_storage_test_vm("https://webidl-sequence-iterator.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}:${error && error.message}`;
    }
  };
  const results = [];

  const originalArrayFrom = Array.from;
  let arrayFromCalls = 0;
  Array.from = () => {
    arrayFromCalls += 1;
    throw new Error('array-from-polluted');
  };
  try {
    const observer = new IntersectionObserver(() => {}, {
      threshold: [0.25, 0.75],
    });
    results.push(`arrayFrom:${arrayFromCalls}:${observer.thresholds.join(',')}`);
  } finally {
    Array.from = originalArrayFrom;
  }

  const originalArrayIterator = Array.prototype[Symbol.iterator];
  let arrayIteratorCalls = 0;
  Array.prototype[Symbol.iterator] = function() {
    arrayIteratorCalls += 1;
    return {
      next() {
        throw new RangeError('array-iterator-used');
      },
    };
  };
  try {
    results.push(`arrayIterator:${arrayIteratorCalls}:${
      probe(() => new IntersectionObserver(() => {}, { threshold: [0.5] }))
    }:${arrayIteratorCalls}`);
  } finally {
    Array.prototype[Symbol.iterator] = originalArrayIterator;
  }

  const order = [];
  const entryTypes = {
    [Symbol.iterator]() {
      order.push('iterator');
      let index = 0;
      return {
        next() {
          order.push(`next:${index}`);
          if (index === 0) {
            index += 1;
            return {
              get done() {
                order.push('done:0');
                return false;
              },
              get value() {
                order.push('value:0');
                return {
                  toString() {
                    order.push('toString:0');
                    return 'mark';
                  },
                };
              },
            };
          }
          if (index === 1) {
            index += 1;
            return {
              get done() {
                order.push('done:1');
                return false;
              },
              get value() {
                order.push('value:1');
                return {
                  toString() {
                    order.push('toString:1');
                    return 'measure';
                  },
                };
              },
            };
          }
          return {
            get done() {
              order.push('done:2');
              return true;
            },
          };
        },
      };
    },
  };
  const orderedObserver = new PerformanceObserver(() => {});
  orderedObserver.observe({ entryTypes });
  results.push(`order:${order.join(',')}`);

  const throwingNext = {
    [Symbol.iterator]() {
      return {
        next() {
          throw new RangeError('next-boom');
        },
      };
    },
  };
  results.push(`nextThrow:${
    probe(() => new PerformanceObserver(() => {}).observe({ entryTypes: throwingNext }))
  }`);

  const throwingDone = {
    [Symbol.iterator]() {
      return {
        next() {
          return {
            get done() {
              throw new TypeError('done-boom');
            },
          };
        },
      };
    },
  };
  results.push(`doneThrow:${
    probe(() => new PerformanceObserver(() => {}).observe({ entryTypes: throwingDone }))
  }`);

  const throwingValue = {
    [Symbol.iterator]() {
      return {
        next() {
          return {
            done: false,
            get value() {
              throw new SyntaxError('value-boom');
            },
          };
        },
      };
    },
  };
  results.push(`valueThrow:${
    probe(() => new PerformanceObserver(() => {}).observe({ entryTypes: throwingValue }))
  }`);

  let throwingElementIteratorClosed = false;
  const throwingElementConversion = {
    [Symbol.iterator]() {
      let finished = false;
      return {
        next() {
          if (finished) {
            return { done: true };
          }
          finished = true;
          return {
            done: false,
            value: {
              toString() {
                throw new URIError('string-boom');
              },
            },
          };
        },
        return() {
          throwingElementIteratorClosed = true;
          throw new EvalError('close-must-not-replace-element-error');
        },
      };
    },
  };
  results.push(`elementThrow:${
    probe(() => new PerformanceObserver(() => {}).observe({
      entryTypes: throwingElementConversion,
    }))
  }:${throwingElementIteratorClosed}`);

  return results.join('|');
})()
"#,
        )
        .expect("WebIDL sequence iterator conversion probe should evaluate");

    assert_eq!(
        result,
        "arrayFrom:0:0.25,0.75|arrayIterator:0:throw:RangeError:array-iterator-used:1|order:iterator,next:0,done:0,value:0,toString:0,next:1,done:1,value:1,toString:1,next:2,done:2|nextThrow:throw:RangeError:next-boom|doneThrow:throw:TypeError:done-boom|valueThrow:throw:SyntaxError:value-boom|elementThrow:throw:URIError:string-boom:false"
    );
}

#[test]
fn url_search_params_sequence_discrimination_reads_iterator_once() {
    let mut vm = new_storage_test_vm("https://url-search-params-sequence.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function* pairs() {
    yield ['first', 'one'];
    yield ['second', 'two'];
  }
  let iteratorGets = 0;
  const init = {};
  Object.defineProperty(init, Symbol.iterator, {
    get() {
      iteratorGets += 1;
      return pairs;
    }
  });
  const originalArrayFrom = Array.from;
  Array.from = () => { throw new Error('polluted Array.from'); };
  let params;
  try {
    params = new URLSearchParams(init);
  } finally {
    Array.from = originalArrayFrom;
  }
  let stringArrayError = 'none';
  try {
    new URLSearchParams(['key', 'value']);
  } catch (error) {
    stringArrayError = error && error.name;
  }

  let outerClosed = false;
  let invalidPairError = 'none';
  try {
    new URLSearchParams({
      [Symbol.iterator]() {
        let done = false;
        return {
          next() {
            if (done) return { done: true };
            done = true;
            return { done: false, value: ['short'] };
          },
          return() {
            outerClosed = true;
            throw new SyntaxError('close error');
          }
        };
      }
    });
  } catch (error) {
    invalidPairError = error && error.name;
  }

  let innerClosed = false;
  let innerError = 'none';
  const throwingPair = {
    [Symbol.iterator]() {
      let index = 0;
      return {
        next() {
          index += 1;
          if (index === 1) return { done: false, value: 'key' };
          return {
            done: false,
            value: { toString() { throw new RangeError('value error'); } }
          };
        },
        return() {
          innerClosed = true;
          throw new SyntaxError('close error');
        }
      };
    }
  };
  try {
    new URLSearchParams([throwingPair]);
  } catch (error) {
    innerError = error && error.name;
  }
  return [
    iteratorGets,
    params.get('first'),
    params.get('second'),
    stringArrayError,
    `${invalidPairError}:${outerClosed}`,
    `${innerError}:${innerClosed}`
  ].join('|');
})()
"#,
        )
        .expect("URLSearchParams sequence discrimination probe should evaluate");

    assert_eq!(
        result,
        "1|one|two|TypeError|TypeError:false|RangeError:false"
    );
}

#[test]
fn performance_observer_declared_slots_ignore_prototype_spoofing() {
    let mut vm = new_storage_test_vm("https://performance-observer-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const observer = new PerformanceObserver(() => {});
  observer.observe({ entryTypes: ['mark'] });
  performance.mark('declared-observer');
  const records = observer.takeRecords();
  const ownSlots = Object.getOwnPropertyNames(observer)
    .filter(name => name.startsWith('__moliPerformanceObserver'))
    .sort();

  PerformanceObserver.prototype.__moliPerformanceObserverCallbackId = 1;
  PerformanceObserver.prototype.__moliPerformanceObserverPending = ['spoofed'];
  PerformanceObserver.prototype.__moliPerformanceObserverType = 'mark';
  PerformanceObserver.prototype.__moliPerformanceObserverEntryTypes = ['mark'];
  PerformanceObserver.prototype.__moliPerformanceObserverActive = true;
  PerformanceObserver.prototype.__moliPerformanceObserverScheduled = true;

  const fake = Object.create(PerformanceObserver.prototype);
  const fakeRecords = PerformanceObserver.prototype.takeRecords.call(fake);
  const fakeObserve = probe(() => PerformanceObserver.prototype.observe.call(fake, {
    entryTypes: ['measure'],
  }));
  return JSON.stringify({
    real: [
      records.length,
      records[0] && records[0].entryType,
      records[0] && records[0].name,
      observer.takeRecords().length
    ].join('|'),
    fake: [
      fakeRecords.length,
      fakeRecords[0],
      fakeObserve,
      fake.takeRecords().length
    ].map(value => value === undefined ? 'undefined' : String(value)).join('|'),
    ownSlots
  });
})()
"#,
        )
        .expect("PerformanceObserver declared slots should ignore prototype spoofing");

    assert_eq!(
        result,
        r#"{"real":"1|mark|declared-observer|0","fake":"0|undefined|undefined|0","ownSlots":[]}"#
    );
}

#[test]
fn global_runtime_queues_hide_slots_and_ignore_spoofing() {
    let mut vm = new_storage_test_vm("https://global-runtime-queues.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const queueNames = [
    '__moliFileReaderQueue',
    '__moliResizeObserverQueue',
    '__moliPerformanceObserverQueue'
  ];
  const reflectedQueues = () => Object.getOwnPropertyNames(globalThis)
    .filter(name => queueNames.includes(name))
    .sort();
  const before = reflectedQueues();
  for (const name of queueNames) {
    globalThis[name] = false;
  }

  const performanceObserver = new PerformanceObserver(() => {});
  performanceObserver.observe({ entryTypes: ['mark'] });
  performance.mark('global-queue-mark');
  const performanceRecords = performanceObserver.takeRecords()
    .map(entry => `${entry.entryType}:${entry.name}`)
    .join(',');

  const target = document.createElement('div');
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.appendChild(target);
  const resizeObserver = new ResizeObserver(() => {});
  resizeObserver.observe(target);
  const resizeRecords = resizeObserver.takeRecords();

  const reader = new FileReader();
  reader.readAsText(new Blob(['abc']));

  return JSON.stringify({
    before,
    afterSpoof: reflectedQueues(),
    performanceRecords,
    resizeRecords: [
      resizeRecords.length,
      resizeRecords[0] && resizeRecords[0].target === target
    ].join('|'),
    fileReaderStarted: [
      reader.readyState === FileReader.LOADING,
      reader.result === null,
      Object.getOwnPropertyNames(reader)
        .filter(name => name.startsWith('__moliFileReader'))
        .join(',')
    ].join('|')
  });
})()
"#,
        )
        .expect("global runtime queue slots should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"before":[],"afterSpoof":["__moliFileReaderQueue","__moliPerformanceObserverQueue","__moliResizeObserverQueue"],"performanceRecords":"mark:global-queue-mark","resizeRecords":"1|true","fileReaderStarted":"true|true|"}"#
    );
}

#[test]
fn intersection_observer_servo_aligned_options_surface() {
    let mut vm = new_storage_test_vm("https://intersection-observer-options-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const defaultObserver = new IntersectionObserver(() => {});
  const observer = new IntersectionObserver(() => {}, {
    rootMargin: '1px 2% 3px',
    scrollMargin: '4px 5% 6px 7%',
    threshold: new Set([0.75, 0.25, 0.25]),
    delay: 1,
    trackVisibility: true,
  });
  const emptyThresholdObserver = new IntersectionObserver(() => {}, { threshold: [] });
  let rootMarginCalls = 0;
  let delayCalls = 0;
  const converted = new IntersectionObserver(() => {}, {
    rootMargin: { toString() { rootMarginCalls += 1; return '8px'; } },
    delay: { valueOf() { delayCalls += 1; return 5; } },
    trackVisibility: true,
  });
  const text = document.createTextNode('text');
  const element = document.createElement('div');
  const targetObserver = new IntersectionObserver(() => {});
  const scrollMarginDescriptor =
    Object.getOwnPropertyDescriptor(IntersectionObserver.prototype, 'scrollMargin');
  const delayDescriptor =
    Object.getOwnPropertyDescriptor(IntersectionObserver.prototype, 'delay');
  const trackVisibilityDescriptor =
    Object.getOwnPropertyDescriptor(IntersectionObserver.prototype, 'trackVisibility');
  return [
    defaultObserver.scrollMargin,
    defaultObserver.delay,
    defaultObserver.trackVisibility,
    observer.rootMargin,
    observer.scrollMargin,
    JSON.stringify(observer.thresholds),
    observer.delay,
    observer.trackVisibility,
    JSON.stringify(emptyThresholdObserver.thresholds),
    converted.rootMargin,
    converted.delay,
    `${rootMarginCalls}:${delayCalls}`,
    typeof scrollMarginDescriptor.get,
    String(scrollMarginDescriptor.enumerable),
    typeof delayDescriptor.get,
    typeof trackVisibilityDescriptor.get,
    probe(() => new IntersectionObserver(() => {}, 1)),
    probe(() => new IntersectionObserver(() => {}, { rootMargin: null })),
    probe(() => new IntersectionObserver(() => {}, { scrollMargin: '1em' })),
    probe(() => new IntersectionObserver(() => {}, { root: text })),
    probe(() => new IntersectionObserver(() => {}, { root: document }) instanceof IntersectionObserver),
    probe(() => targetObserver.observe(text)),
    probe(() => targetObserver.observe(document)),
    probe(() => targetObserver.observe(element)),
    probe(() => targetObserver.unobserve(text)),
    probe(() => targetObserver.unobserve()),
  ].join('|');
})()
"#,
        )
        .expect("IntersectionObserver Servo-aligned option surface should evaluate");

    assert_eq!(
        result,
        "0px 0px 0px 0px|0|false|1px 2% 3px 2%|4px 5% 6px 7%|[0.25,0.75]|100|true|[0]|8px 8px 8px 8px|100|1:1|function|true|function|function|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|true|throw:TypeError|throw:TypeError|undefined|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn resize_observer_callbacks_apply_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://resize-observer-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const values = [];
  values.push(probe(() => new ResizeObserver()));
  values.push(probe(() => new ResizeObserver({})));

  const observer = new ResizeObserver(() => {});
  const element = document.createElement('div');
  const text = document.createTextNode('text');
  body.appendChild(element);
  values.push(observer instanceof ResizeObserver);
  values.push(probe(() => observer.observe()));
  values.push(probe(() => observer.observe(text)));
  values.push(probe(() => observer.observe(element, 1)));
  values.push(probe(() => observer.observe(element, { box: 'invalid-box' })));

  let boxCalls = 0;
  values.push(probe(() => {
    observer.observe(element, {
      box: { toString() { boxCalls += 1; return 'border-box'; } },
    });
    return `${boxCalls}:${observer.takeRecords().length}`;
  }));
  values.push(probe(() => observer.unobserve()));
  values.push(probe(() => observer.unobserve(text)));
  values.push(probe(() => {
    observer.observe(element, { box: 'content-box' });
    observer.unobserve(element);
    return observer.takeRecords().length;
  }));
  return values.join('|');
})()
"#,
        )
        .expect("ResizeObserver WebIDL conversion probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|true|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|1:1|throw:TypeError|throw:TypeError|0"
    );
}

#[test]
fn resize_observer_entries_expose_box_size_arrays() {
    let mut vm = new_storage_test_vm("https://resize-observer-box-size.test/");

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.style.cssText = 'width: 41px; height: 23px';
  body.appendChild(target);
yield; // Publish this scene before reading its geometry.
  const observer = new ResizeObserver(() => {});
  observer.observe(target, { box: 'border-box' });
  const first = observer.takeRecords()[0];
  observer.observe(target, { box: 'device-pixel-content-box' });
  const second = observer.takeRecords()[0];
  return [
    first.target === target,
    Array.isArray(first.contentBoxSize),
    Array.isArray(first.borderBoxSize),
    Array.isArray(first.devicePixelContentBoxSize),
    first.contentBoxSize.length,
    first.contentBoxSize[0].inlineSize,
    first.contentBoxSize[0].blockSize,
    first.borderBoxSize[0].inlineSize,
    first.borderBoxSize[0].blockSize,
    second.devicePixelContentBoxSize[0].inlineSize,
    second.devicePixelContentBoxSize[0].blockSize,
    Object.keys(first.contentBoxSize[0]).join(',')
  ].join('|');
})()
"#,
    )
    .expect("ResizeObserver box-size entries should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|1|41|23|41|23|41|23|inlineSize,blockSize"
    );
}

#[test]
fn resize_observer_callback_runs_after_microtask_checkpoint() {
    let mut vm = new_storage_test_vm("https://resize-observer-delivery.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  globalThis.__resizeObserverLog = [];
  const target = document.createElement('div');
  target.style.cssText = 'width: 10px; height: 20px';
  body.appendChild(target);
  const observer = new ResizeObserver((entries, instance) => {
    globalThis.__resizeObserverLog.push(`${entries.length}:${instance === observer}:${entries[0].target === target}`);
  });
  observer.observe(target);
  return 'scheduled';
})()
"#,
        )
        .expect("ResizeObserver delivery setup should evaluate");

    assert_eq!(result, "scheduled");
    let delivered = vm
        .eval("globalThis.__resizeObserverLog.join('|')")
        .expect("ResizeObserver delivery log should evaluate");
    assert_eq!(delivered, "1:true:true");
}

#[test]
fn resize_observer_declared_slots_ignore_prototype_spoofing() {
    let mut vm = new_storage_test_vm("https://resize-observer-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const stringify = value => value === undefined ? 'undefined' : String(value);
  const target = document.createElement('div');
  target.style.cssText = 'width: 10px; height: 20px';
  body.appendChild(target);
  const observer = new ResizeObserver(() => {});
  observer.observe(target, { box: 'border-box' });
  const records = observer.takeRecords();
  const ownSlots = Object.getOwnPropertyNames(observer)
    .filter(name => name.startsWith('__moliResizeObserver'))
    .sort();

  ResizeObserver.prototype.__moliResizeObserverCallbackId = 1;
  ResizeObserver.prototype.__moliResizeObserverTargets = [{ target }];
  ResizeObserver.prototype.__moliResizeObserverPendingTargets = [{ target }];
  ResizeObserver.prototype.__moliResizeObserverScheduled = true;

  const fake = Object.create(ResizeObserver.prototype);
  const fakeRecords = ResizeObserver.prototype.takeRecords.call(fake);
  const fakeObserve = ResizeObserver.prototype.observe.call(fake, target);
  const fakeAfterObserve = fake.takeRecords();
  return JSON.stringify({
    real: [
      records.length,
      records[0] && records[0].target === target,
      records[0] && records[0].borderBoxSize.length,
      observer.takeRecords().length
    ].join('|'),
    fake: [
      fakeRecords.length,
      fakeRecords[0],
      fakeObserve,
      fakeAfterObserve.length,
      fakeAfterObserve[0]
    ].map(stringify).join('|'),
    ownSlots
  });
})()
"#,
        )
        .expect("ResizeObserver declared slots should ignore prototype spoofing");

    assert_eq!(
        result,
        r#"{"real":"1|true|1|0","fake":"0|undefined|undefined|0|undefined","ownSlots":[]}"#
    );
}

#[test]
fn resize_observer_observed_records_ignore_public_spoofing() {
    let mut vm = new_storage_test_vm("https://resize-observer-record-slots.test/");

    let result = eval_with_layout_publications(&mut vm,
            r#"
(function* () {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  const other = document.createElement('section');
  target.style.cssText = 'width: 10px; height: 20px';
  other.style.cssText = 'width: 30px; height: 40px';
  body.append(target, other);

  Object.prototype.__moliResizeObserverRecordTarget = other;
  Object.prototype.__moliResizeObserverRecordBox = 'device-pixel-content-box';
  target.__moliResizeObserverRecordTarget = other;
  target.__moliResizeObserverRecordBox = 'border-box';

yield; // Publish this scene before reading its geometry.
  const observer = new ResizeObserver(() => {});
  observer.observe(target, { box: 'content-box' });
  observer.observe(target, { box: 'border-box' });
  const first = observer.takeRecords();

  observer.observe(target);
  observer.observe(other);
  observer.unobserve(target);
  const second = observer.takeRecords();

  return JSON.stringify({
    first: [
      first.length,
      first[0] && first[0].target === target,
      first[0] && first[0].contentRect.width,
      first[0] && Object.getOwnPropertyNames(first[0]).some(name => name.startsWith('__moliResizeObserverRecord'))
    ].join('|'),
    second: [
      second.length,
      second[0] && second[0].target === other,
      second[0] && second[0].contentRect.width
    ].join('|'),
    targetSpoofVisible: target.__moliResizeObserverRecordTarget === other
  });
})()
"#,
        )
        .expect("ResizeObserver observed records should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"first":"1|true|10|false","second":"1|true|30","targetSpoofVisible":true}"#
    );
}

#[test]
fn vtt_cue_constructor_applies_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://vtt-cue-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const values = [];
  values.push(probe(() => new VTTCue()));
  values.push(probe(() => new VTTCue(0)));
  values.push(probe(() => new VTTCue(0, 1)));
  values.push(probe(() => new VTTCue(Symbol('start'), 1, 'text')));
  values.push(probe(() => new VTTCue(0, Infinity, 'text')));
  values.push(probe(() => new VTTCue(0, 1, Symbol('text'))));

  let startCalls = 0;
  let textCalls = 0;
  const cue = new VTTCue(
    { valueOf() { startCalls += 1; return 1.25; } },
    '2.5',
    { toString() { textCalls += 1; return 'caption'; } },
  );
  values.push([cue.startTime, cue.endTime, cue.text, cue instanceof VTTCue, cue instanceof TextTrackCue, startCalls, textCalls].join(','));
  return values.join('|');
})()
"#,
        )
        .expect("VTTCue WebIDL conversion probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|1.25,2.5,caption,true,true,1,1"
    );
}
