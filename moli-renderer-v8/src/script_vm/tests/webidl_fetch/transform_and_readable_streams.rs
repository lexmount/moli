use super::*;

#[test]
fn transform_stream_flush_waits_for_pending_transform_promise() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformFlushWaitEvents = [];
                let resolveTransform;
                globalThis.__resolveTransformFlushWait = () => resolveTransform();
                const stream = new TransformStream({
                    transform() {
                        globalThis.__transformFlushWaitEvents.push("transform");
                        return new Promise(resolve => {
                            resolveTransform = resolve;
                        });
                    },
                    flush() {
                        globalThis.__transformFlushWaitEvents.push("flush");
                        return new Promise(() => {});
                    }
                }, undefined, { highWaterMark: 1 });
                const writer = stream.writable.getWriter();
                writer.write("a").then(() => {
                    globalThis.__transformFlushWaitEvents.push("write:resolved");
                });
                writer.close().then(() => {
                    globalThis.__transformFlushWaitEvents.push("close:resolved");
                });
                stream.readable.getReader().closed.then(() => {
                    globalThis.__transformFlushWaitEvents.push("readable:closed");
                });
                return JSON.stringify(globalThis.__transformFlushWaitEvents);
            })()
            "#,
        )
        .expect("TransformStream pending transform flush setup should evaluate");
    assert_eq!(initial, "[]");

    let before_resolve = vm
        .eval("JSON.stringify(globalThis.__transformFlushWaitEvents)")
        .expect("TransformStream pending transform flush should not run early");
    assert_eq!(before_resolve, r#"["transform"]"#);

    let after_resolve = vm
        .eval(
            r#"
            (() => {
                globalThis.__resolveTransformFlushWait();
                return JSON.stringify(globalThis.__transformFlushWaitEvents);
            })()
            "#,
        )
        .expect("TransformStream pending transform resolver should evaluate");
    assert_eq!(after_resolve, r#"["transform"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformFlushWaitEvents)")
        .expect("TransformStream flush should run after transform settles");
    assert_eq!(settled, r#"["transform","flush","write:resolved"]"#);
}

#[test]
fn transform_stream_flush_can_enqueue_before_readable_closes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformFlushEnqueueEvents = [];
                let savedController;
                const stream = new TransformStream({
                    start(controller) {
                        savedController = controller;
                    },
                    transform() {},
                    flush() {
                        savedController.enqueue("x");
                        savedController.enqueue("y");
                        globalThis.__transformFlushEnqueueEvents.push("flush");
                    }
                });
                const reader = stream.readable.getReader();
                const writer = stream.writable.getWriter();
                writer.write("a");
                writer.close().then(() => {
                    globalThis.__transformFlushEnqueueEvents.push("close:resolved");
                });
                reader.read().then(({ value, done }) => {
                    globalThis.__transformFlushEnqueueEvents.push(`read1:${value}:${done}`);
                    return reader.read();
                }).then(({ value, done }) => {
                    globalThis.__transformFlushEnqueueEvents.push(`read2:${value}:${done}`);
                });
                return JSON.stringify(globalThis.__transformFlushEnqueueEvents);
            })()
            "#,
        )
        .expect("TransformStream flush enqueue setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformFlushEnqueueEvents)")
        .expect("TransformStream flush enqueue promises should settle");
    assert_eq!(
        settled,
        r#"["flush","read1:x:false","read2:y:false","close:resolved"]"#
    );
}

#[test]
fn transform_stream_flush_error_rejects_writer_close() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformFlushErrorEvents = [];
                const error = new Error("flush-boom");
                const stream = new TransformStream({
                    flush(controller) {
                        controller.error(error);
                    }
                });
                stream.writable.getWriter().close().then(
                    () => globalThis.__transformFlushErrorEvents.push("close:resolved"),
                    error => globalThis.__transformFlushErrorEvents.push(`close:${error.message}`)
                );
                return JSON.stringify(globalThis.__transformFlushErrorEvents);
            })()
            "#,
        )
        .expect("TransformStream flush error setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformFlushErrorEvents)")
        .expect("TransformStream flush error close should settle");
    assert_eq!(settled, r#"["close:flush-boom"]"#);
}

#[test]
fn transform_stream_start_promise_gates_transform_and_flush() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformStartGateEvents = [];
                let resolveStart;
                globalThis.__resolveTransformStartGate = () => {
                    globalThis.__transformStartGateEvents.push("start:resolve");
                    resolveStart();
                };
                const stream = new TransformStream({
                    start() {
                        globalThis.__transformStartGateEvents.push("start");
                        return new Promise(resolve => {
                            resolveStart = resolve;
                        });
                    },
                    transform(chunk, controller) {
                        globalThis.__transformStartGateEvents.push(`transform:${chunk}`);
                        controller.enqueue(chunk);
                    },
                    flush() {
                        globalThis.__transformStartGateEvents.push("flush");
                    }
                }, undefined, { highWaterMark: 1 });
                const writer = stream.writable.getWriter();
                writer.write("a").then(() => {
                    globalThis.__transformStartGateEvents.push("write:resolved");
                });
                writer.close().then(() => {
                    globalThis.__transformStartGateEvents.push("close:resolved");
                });
                return JSON.stringify(globalThis.__transformStartGateEvents);
            })()
            "#,
        )
        .expect("TransformStream start gating setup should evaluate");
    assert_eq!(initial, r#"["start"]"#);

    let before_start = vm
        .eval("JSON.stringify(globalThis.__transformStartGateEvents)")
        .expect("TransformStream start promise should gate transform");
    assert_eq!(before_start, r#"["start"]"#);

    let after_resolve = vm
        .eval(
            r#"
            (() => {
                globalThis.__resolveTransformStartGate();
                return JSON.stringify(globalThis.__transformStartGateEvents);
            })()
            "#,
        )
        .expect("TransformStream start resolver should evaluate");
    assert_eq!(after_resolve, r#"["start","start:resolve"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformStartGateEvents)")
        .expect("TransformStream write and flush should run after start resolves");
    assert_eq!(
        settled,
        r#"["start","start:resolve","transform:a","flush","write:resolved","close:resolved"]"#
    );
}

#[test]
fn transform_stream_defined_readable_or_writable_type_throws_range_error() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const events = [];
                try {
                    new TransformStream({ readableType: "bytes" });
                    events.push("readable:constructed");
                } catch (error) {
                    events.push(`readable:${error.name}`);
                }
                try {
                    new TransformStream({ writableType: "bytes" });
                    events.push("writable:constructed");
                } catch (error) {
                    events.push(`writable:${error.name}`);
                }
                return JSON.stringify(events);
            })()
            "#,
        )
        .expect("TransformStream defined readable/writable type validation should evaluate");
    assert_eq!(result, r#"["readable:RangeError","writable:RangeError"]"#);
}

#[test]
fn transform_stream_start_rejection_errors_pending_write_without_transform() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformStartRejectWriteEvents = [];
                const error = new Error("start-boom");
                const stream = new TransformStream({
                    start() {
                        return Promise.reject(error);
                    },
                    transform() {
                        globalThis.__transformStartRejectWriteEvents.push("transform");
                    }
                }, undefined, { highWaterMark: 1 });
                const writer = stream.writable.getWriter();
                writer.write("a").then(
                    () => globalThis.__transformStartRejectWriteEvents.push("write:resolved"),
                    error => globalThis.__transformStartRejectWriteEvents.push(`write:${error.message}`)
                );
                stream.readable.getReader().read().catch(error => {
                    globalThis.__transformStartRejectWriteEvents.push(`read:${error.message}`);
                });
                return JSON.stringify(globalThis.__transformStartRejectWriteEvents);
            })()
            "#,
        )
        .expect("TransformStream start rejection write setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformStartRejectWriteEvents.sort())")
        .expect("TransformStream start rejection should error queued write");
    assert_eq!(settled, r#"["read:start-boom","write:start-boom"]"#);
}

#[test]
fn transform_stream_start_rejection_rejects_pending_close_without_flush() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformStartRejectCloseEvents = [];
                const error = new Error("start-close-boom");
                const stream = new TransformStream({
                    start() {
                        return Promise.reject(error);
                    },
                    flush() {
                        globalThis.__transformStartRejectCloseEvents.push("flush");
                    }
                });
                const writer = stream.writable.getWriter();
                writer.close().then(
                    () => globalThis.__transformStartRejectCloseEvents.push("close:resolved"),
                    error => globalThis.__transformStartRejectCloseEvents.push(`close:${error.message}`)
                );
                stream.readable.getReader().closed.catch(error => {
                    globalThis.__transformStartRejectCloseEvents.push(`readable:${error.message}`);
                });
                return JSON.stringify(globalThis.__transformStartRejectCloseEvents);
            })()
            "#,
        )
        .expect("TransformStream start rejection close setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformStartRejectCloseEvents.sort())")
        .expect("TransformStream start rejection should reject queued close");
    assert_eq!(
        settled,
        r#"["close:start-close-boom","readable:start-close-boom"]"#
    );
}

#[test]
fn transform_stream_start_controller_error_beats_later_rejection() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformStartFirstErrorEvents = [];
                const controllerError = new Error("controller-start-error");
                const ignoredError = new Error("ignored-start-error");
                const stream = new TransformStream({
                    start(controller) {
                        return Promise.resolve().then(() => {
                            controller.error(controllerError);
                            throw ignoredError;
                        });
                    },
                    transform() {
                        globalThis.__transformStartFirstErrorEvents.push("transform");
                    },
                    flush() {
                        globalThis.__transformStartFirstErrorEvents.push("flush");
                    }
                }, undefined, { highWaterMark: 1 });
                const writer = stream.writable.getWriter();
                writer.write("a").then(
                    () => globalThis.__transformStartFirstErrorEvents.push("write:resolved"),
                    error => globalThis.__transformStartFirstErrorEvents.push(`write:${error.message}`)
                );
                writer.close().then(
                    () => globalThis.__transformStartFirstErrorEvents.push("close:resolved"),
                    error => globalThis.__transformStartFirstErrorEvents.push(`close:${error.message}`)
                );
                stream.readable.getReader().read().catch(error => {
                    globalThis.__transformStartFirstErrorEvents.push(`read:${error.message}`);
                });
                return JSON.stringify(globalThis.__transformStartFirstErrorEvents);
            })()
            "#,
        )
        .expect("TransformStream start first-error setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformStartFirstErrorEvents.sort())")
        .expect("TransformStream controller.error should beat later start rejection");
    assert_eq!(
        settled,
        r#"["close:controller-start-error","read:controller-start-error","write:controller-start-error"]"#
    );
}

#[test]
fn writable_stream_writer_closed_stays_pending_until_transform_closes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__writerClosedPendingEvents = [];
                const stream = new TransformStream();
                globalThis.__writerClosedPendingWriter = stream.writable.getWriter();
                globalThis.__writerClosedPendingClosed =
                    globalThis.__writerClosedPendingWriter.closed;
                globalThis.__writerClosedPendingClosed.then(() => {
                    globalThis.__writerClosedPendingEvents.push("closed");
                });
                return JSON.stringify({
                    same: globalThis.__writerClosedPendingWriter.closed ===
                        globalThis.__writerClosedPendingClosed,
                    events: globalThis.__writerClosedPendingEvents
                });
            })()
            "#,
        )
        .expect("Writable writer closed pending setup should evaluate");
    assert_eq!(initial, r#"{"same":true,"events":[]}"#);

    let before_close = vm
        .eval("JSON.stringify(globalThis.__writerClosedPendingEvents)")
        .expect("Writable writer closed should stay pending before close");
    assert_eq!(before_close, "[]");

    let close_started = vm
        .eval(
            r#"
            (() => {
                globalThis.__writerClosedPendingWriter.close().then(() => {
                    globalThis.__writerClosedPendingEvents.push("close");
                });
                return JSON.stringify({
                    same: globalThis.__writerClosedPendingWriter.closed ===
                        globalThis.__writerClosedPendingClosed,
                    events: globalThis.__writerClosedPendingEvents
                });
            })()
            "#,
        )
        .expect("Writable writer close should evaluate");
    assert_eq!(close_started, r#"{"same":true,"events":[]}"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__writerClosedPendingEvents.sort())")
        .expect("Writable writer closed should resolve after close");
    assert_eq!(settled, r#"["close","closed"]"#);
}

#[test]
fn writable_stream_writer_closed_rejects_after_transform_controller_error() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__writerClosedErrorEvents = [];
                let savedController;
                const stream = new TransformStream({
                    start(controller) {
                        savedController = controller;
                    }
                });
                const writer = stream.writable.getWriter();
                const closed = writer.closed;
                globalThis.__writerClosedErrorWriter = writer;
                globalThis.__writerClosedErrorClosed = closed;
                closed.then(
                    () => globalThis.__writerClosedErrorEvents.push("closed:resolved"),
                    error => globalThis.__writerClosedErrorEvents.push(`closed:${error.message}`)
                );
                globalThis.__errorWriterClosedTransform = () => {
                    savedController.error(new Error("writer-closed-boom"));
                };
                return JSON.stringify(globalThis.__writerClosedErrorEvents);
            })()
            "#,
        )
        .expect("Writable writer closed error setup should evaluate");
    assert_eq!(initial, "[]");

    let before_error = vm
        .eval("JSON.stringify(globalThis.__writerClosedErrorEvents)")
        .expect("Writable writer closed should stay pending before controller error");
    assert_eq!(before_error, "[]");

    let error_started = vm
        .eval(
            r#"
            (() => {
                globalThis.__errorWriterClosedTransform();
                return JSON.stringify({
                    same: globalThis.__writerClosedErrorWriter.closed ===
                        globalThis.__writerClosedErrorClosed,
                    events: globalThis.__writerClosedErrorEvents
                });
            })()
            "#,
        )
        .expect("Writable writer closed controller error should evaluate");
    assert_eq!(error_started, r#"{"same":true,"events":[]}"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__writerClosedErrorEvents)")
        .expect("Writable writer closed should reject after controller error");
    assert_eq!(settled, r#"["closed:writer-closed-boom"]"#);
}

#[test]
fn transform_stream_controller_terminate_closes_readable_and_errors_writable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformTerminateEvents = [];
                let controller;
                const stream = new TransformStream({
                    start(c) {
                        controller = c;
                    }
                });
                const writer = stream.writable.getWriter();
                const reader = stream.readable.getReader();
                writer.closed.then(
                    () => globalThis.__transformTerminateEvents.push("writer:resolved"),
                    error => globalThis.__transformTerminateEvents.push(
                        `writer:${error.name}:${error instanceof TypeError}:${error.message}`
                    )
                );
                reader.closed.then(
                    () => globalThis.__transformTerminateEvents.push("reader:closed"),
                    error => globalThis.__transformTerminateEvents.push(`reader:${error.name}`)
                );
                controller.terminate();
                controller.terminate();
                let enqueueResult = "no-throw";
                try {
                    controller.enqueue("after");
                } catch (error) {
                    enqueueResult = `${error.name}:${error instanceof TypeError}`;
                }
                return JSON.stringify({
                    enqueueResult,
                    events: globalThis.__transformTerminateEvents
                });
            })()
            "#,
        )
        .expect("TransformStream controller terminate setup should evaluate");
    assert_eq!(initial, r#"{"enqueueResult":"TypeError:true","events":[]}"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformTerminateEvents.sort())")
        .expect("TransformStream controller terminate promises should settle");
    assert_eq!(
        settled,
        r#"["reader:closed","writer:TypeError:true:The transform stream has been terminated"]"#
    );
}

#[test]
fn transform_stream_controller_terminate_after_readable_cancel_preserves_cancel_result() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformTerminateCancelEvents = [];
                let controller;
                const stream = new TransformStream({
                    start(c) {
                        controller = c;
                    }
                });
                const cancelReason = { name: "cancelReason" };
                stream.readable.cancel(cancelReason).then(
                    () => globalThis.__transformTerminateCancelEvents.push("cancel:resolved"),
                    error => globalThis.__transformTerminateCancelEvents.push(
                        `cancel:${error && error.name}`
                    )
                );
                stream.writable.getWriter().closed.then(
                    () => globalThis.__transformTerminateCancelEvents.push("writer:resolved"),
                    error => globalThis.__transformTerminateCancelEvents.push(
                        `writer:${error.name}:${error instanceof TypeError}:${error.message}`
                    )
                );
                controller.terminate();
                return JSON.stringify(globalThis.__transformTerminateCancelEvents);
            })()
            "#,
        )
        .expect("TransformStream terminate after cancel setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformTerminateCancelEvents.sort())")
        .expect("TransformStream terminate after cancel promises should settle");
    assert_eq!(
        settled,
        r#"["cancel:resolved","writer:TypeError:true:The transform stream has been terminated"]"#
    );
}

#[test]
fn transform_stream_identity_write_restores_writer_desired_size_after_read() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__identityDesiredSizeEvents = [];
                const stream = new TransformStream();
                const writer = stream.writable.getWriter();
                globalThis.__identityDesiredSizeEvents.push(`initial:${writer.desiredSize}`);
                writer.write("a").then(() => {
                    globalThis.__identityDesiredSizeEvents.push(`write:${writer.desiredSize}`);
                });
                globalThis.__identityDesiredSizeEvents.push(`after-write:${writer.desiredSize}`);
                stream.readable.getReader().read().then(result => {
                    globalThis.__identityDesiredSizeEvents.push(
                        `read:${result.value}:${result.done}:${writer.desiredSize}`
                    );
                });
                return JSON.stringify(globalThis.__identityDesiredSizeEvents);
            })()
            "#,
        )
        .expect("TransformStream identity desiredSize setup should evaluate");
    assert_eq!(initial, r#"["initial:1","after-write:0"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__identityDesiredSizeEvents)")
        .expect("TransformStream identity desiredSize should settle");
    assert_eq!(
        settled,
        r#"["initial:1","after-write:0","read:a:false:0","write:1"]"#
    );
}

#[test]
fn transform_stream_backpressured_write_and_close_wait_for_readable_demand() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformBackpressureOrderEvents = [];
                const stream = new TransformStream({}, undefined, { highWaterMark: 1 });
                globalThis.__transformBackpressureOrderWriter = stream.writable.getWriter();
                globalThis.__transformBackpressureOrderReader = stream.readable.getReader();
                globalThis.__transformBackpressureOrderWriter.write("a").then(() => {
                    globalThis.__transformBackpressureOrderEvents.push("write:a");
                });
                globalThis.__transformBackpressureOrderWriter.write("b").then(() => {
                    globalThis.__transformBackpressureOrderEvents.push("write:b");
                });
                globalThis.__transformBackpressureOrderWriter.close().then(() => {
                    globalThis.__transformBackpressureOrderEvents.push("close");
                });
                return JSON.stringify(globalThis.__transformBackpressureOrderEvents);
            })()
            "#,
        )
        .expect("TransformStream backpressured write setup should evaluate");
    assert_eq!(initial, "[]");

    let before_read = vm
        .eval("JSON.stringify(globalThis.__transformBackpressureOrderEvents)")
        .expect("TransformStream first write should settle before readable drain");
    assert_eq!(before_read, r#"["write:a"]"#);

    let read_started = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformBackpressureOrderReader.read().then(result => {
                    globalThis.__transformBackpressureOrderEvents.push(
                        `read:${result.value}:${result.done}`
                    );
                });
                return JSON.stringify(globalThis.__transformBackpressureOrderEvents);
            })()
            "#,
        )
        .expect("TransformStream backpressured read should evaluate");
    assert_eq!(read_started, r#"["write:a"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformBackpressureOrderEvents)")
        .expect("TransformStream backpressured writes should settle after read");
    assert_eq!(settled, r#"["write:a","read:a:false","write:b","close"]"#);
}

#[test]
fn transform_stream_readable_cancel_rejects_pipe_to_destination() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformPipeAbortEvents = [];
                globalThis.__transformPipeAbortError = new Error("pipe-abort");
                globalThis.__transformPipeAbortStream = new TransformStream();
                new ReadableStream().pipeTo(globalThis.__transformPipeAbortStream.writable).then(
                    () => globalThis.__transformPipeAbortEvents.push("pipe:resolved"),
                    error => globalThis.__transformPipeAbortEvents.push(
                        `pipe:${error === globalThis.__transformPipeAbortError}:${error.message}`
                    )
                );
                return JSON.stringify(globalThis.__transformPipeAbortEvents);
            })()
            "#,
        )
        .expect("TransformStream pipeTo abort setup should evaluate");
    assert_eq!(initial, "[]");

    let cancel_started = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformPipeAbortStream.readable.cancel(
                    globalThis.__transformPipeAbortError
                );
                return JSON.stringify(globalThis.__transformPipeAbortEvents);
            })()
            "#,
        )
        .expect("TransformStream readable cancel should evaluate");
    assert_eq!(cancel_started, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformPipeAbortEvents)")
        .expect("TransformStream pipeTo promise should reject after readable cancel");
    assert_eq!(settled, r#"["pipe:true:pipe-abort"]"#);
}

#[test]
fn readable_stream_bad_strategy_size_errors_stream() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__badStrategySizeEvents = [];
                const getterError = new Error("getter");
                const methodError = new Error("method");
                const construction = [];
                try {
                    new ReadableStream({}, { get size() { throw getterError; }, highWaterMark: 1 });
                } catch (error) {
                    construction.push(error === getterError ? "getter" : error.name);
                }
                try {
                    new ReadableStream({}, { size: 1, highWaterMark: 1 });
                } catch (error) {
                    construction.push(error.name);
                }
                try {
                    new ReadableStream({}, {});
                    construction.push("empty-ok");
                } catch (error) {
                    construction.push(`empty:${error.name}`);
                }
                try {
                    new ReadableStream({}, { highWaterMark: NaN });
                } catch (error) {
                    construction.push(`nan:${error.name}`);
                }
                try {
                    new ReadableStream({}, { highWaterMark: -1 });
                } catch (error) {
                    construction.push(`negative:${error.name}`);
                }

                let throwController;
                const throwing = new ReadableStream({
                    start(c) {
                        throwController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        throw methodError;
                    }
                });
                throwing.getReader().closed.catch(error => {
                    globalThis.__badStrategySizeEvents.push(`throw-closed:${error === methodError}`);
                });
                try {
                    throwController.enqueue("x");
                } catch (error) {
                    globalThis.__badStrategySizeEvents.push(`throw-enqueue:${error === methodError}`);
                }

                let rangeController;
                const range = new ReadableStream({
                    start(c) {
                        rangeController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        return Infinity;
                    }
                });
                range.getReader().closed.catch(error => {
                    globalThis.__badStrategySizeEvents.push(`range-closed:${error.name}`);
                });
                try {
                    rangeController.enqueue("y");
                } catch (error) {
                    globalThis.__badStrategySizeEvents.push(`range-enqueue:${error.name}`);
                }

                const controllerError = { name: "controller error" };
                const thrownError = { name: "thrown error" };
                let priorityController;
                const priority = new ReadableStream({
                    start(c) {
                        priorityController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        priorityController.error(controllerError);
                        throw thrownError;
                    }
                });
                priority.getReader().closed.catch(error => {
                    globalThis.__badStrategySizeEvents.push(`priority-closed:${error === controllerError}`);
                });
                try {
                    priorityController.enqueue("z");
                } catch (error) {
                    globalThis.__badStrategySizeEvents.push(`priority-enqueue:${error === thrownError}`);
                }

                const rangeControllerError = { name: "range controller error" };
                let priorityRangeController;
                const priorityRange = new ReadableStream({
                    start(c) {
                        priorityRangeController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        priorityRangeController.error(rangeControllerError);
                        return Infinity;
                    }
                });
                priorityRange.getReader().closed.catch(error => {
                    globalThis.__badStrategySizeEvents.push(
                        `priority-range-closed:${error === rangeControllerError}`
                    );
                });
                try {
                    priorityRangeController.enqueue("q");
                } catch (error) {
                    globalThis.__badStrategySizeEvents.push(`priority-range-enqueue:${error.name}`);
                }

                let closeController;
                const closeInsideSize = new ReadableStream({
                    start(c) {
                        closeController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        closeController.close();
                        return 1;
                    }
                });
                closeController.enqueue("closed-chunk");
                closeInsideSize.getReader().read().then(({ value, done }) => {
                    globalThis.__badStrategySizeEvents.push(`close-read:${done}:${String(value)}`);
                });

                return JSON.stringify({ construction, events: globalThis.__badStrategySizeEvents });
            })()
            "#,
        )
        .expect("ReadableStream bad strategy size setup should evaluate");
    assert_eq!(
        initial,
        r#"{"construction":["getter","TypeError","empty-ok","nan:RangeError","negative:RangeError"],"events":["throw-enqueue:true","range-enqueue:RangeError","priority-enqueue:true","priority-range-enqueue:RangeError"]}"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__badStrategySizeEvents.sort())")
        .expect("ReadableStream bad strategy size closed promises should settle");
    assert_eq!(
        settled,
        r#"["close-read:true:undefined","priority-closed:true","priority-enqueue:true","priority-range-closed:true","priority-range-enqueue:RangeError","range-closed:RangeError","range-enqueue:RangeError","throw-closed:true","throw-enqueue:true"]"#
    );
}

#[test]
fn queuing_strategy_constructors_expose_high_water_mark_and_size() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const internalNames = () => Object.getOwnPropertyNames(globalThis)
                    .filter(name => name === "__moliCountQueuingStrategySizeFunction" ||
                        name === "__moliByteLengthQueuingStrategySizeFunction")
                    .sort()
                    .join(",");
                const internalNamesBefore = internalNames();
                const count = new CountQueuingStrategy({ highWaterMark: 5 });
                const count2 = new CountQueuingStrategy({ highWaterMark: 10 });
                const byteLength = new ByteLengthQueuingStrategy({ highWaterMark: 7 });
                const byteLength2 = new ByteLengthQueuingStrategy({ highWaterMark: 11 });
                const countSize = count.size;
                const countSize2 = count2.size;
                const byteLengthSize = byteLength.size;
                const byteLengthSize2 = byteLength2.size;
                const internalNamesAfterCache = internalNames();
                Object.defineProperties(globalThis, {
                    __moliCountQueuingStrategySizeFunction: {
                        configurable: true,
                        value() { return 99; }
                    },
                    __moliByteLengthQueuingStrategySizeFunction: {
                        configurable: true,
                        value() { return 99; }
                    }
                });
                const internalNamesAfterSpoof = internalNames();
                const countSizeAfterSpoof = count.size;
                const byteLengthSizeAfterSpoof = byteLength.size;
                const getterError = new Error("byteLength");
                const throws = callback => {
                    try {
                        callback();
                        return "no-throw";
                    } catch (error) {
                        return error === getterError ? "getter-error" : error.name;
                    }
                };
                return [
                    internalNamesBefore,
                    internalNamesAfterCache,
                    internalNamesAfterSpoof,
                    count.highWaterMark,
                    byteLength.highWaterMark,
                    countSize.name,
                    countSize.length,
                    countSize("ignored"),
                    countSize === countSize2,
                    countSize === countSizeAfterSpoof,
                    "prototype" in countSize,
                    throws(() => new countSize()),
                    byteLengthSize.name,
                    byteLengthSize.length,
                    byteLengthSize({ byteLength: 9 }),
                    byteLengthSize === byteLengthSize2,
                    byteLengthSize === byteLengthSizeAfterSpoof,
                    "prototype" in byteLengthSize,
                    throws(() => new byteLengthSize({ byteLength: 1 })),
                    throws(() => byteLengthSize()),
                    throws(() => byteLengthSize(null)),
                    byteLengthSize("potato"),
                    byteLengthSize({}),
                    byteLengthSize({ get byteLength() { return 13; } }),
                    throws(() => byteLengthSize({ get byteLength() { throw getterError; } })),
                    new ReadableStream({}, count) instanceof ReadableStream,
                ].join("|");
            })()
            "#,
        )
        .expect("QueuingStrategy constructors should evaluate");

    assert_eq!(
        result,
        "||__moliByteLengthQueuingStrategySizeFunction,__moliCountQueuingStrategySizeFunction|5|7|size|0|1|true|true|false|TypeError|size|1|9|true|true|false|TypeError|TypeError|TypeError|||13|getter-error|true"
    );
}

#[test]
fn stream_prototype_methods_preserve_declared_descriptors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const methodDescriptor = (prototype, key, label = key) => {
                    const descriptor = Object.getOwnPropertyDescriptor(prototype, key);
                    return [
                        label,
                        typeof descriptor?.value,
                        descriptor?.value?.name,
                        descriptor?.value?.length,
                        descriptor?.enumerable,
                        descriptor?.writable,
                        descriptor?.configurable,
                    ].join(":");
                };
                const accessorDescriptor = (prototype, key, label = key) => {
                    const descriptor = Object.getOwnPropertyDescriptor(prototype, key);
                    return [
                        label,
                        typeof descriptor?.get,
                        descriptor?.get?.name,
                        descriptor?.get?.length,
                        descriptor?.enumerable,
                        Boolean(descriptor?.set),
                        descriptor?.configurable,
                    ].join(":");
                };
                const readable = new ReadableStream({
                    start(controller) {
                        controller.enqueue("queued");
                    }
                });
                const writable = new WritableStream({
                    write() {}
                });
                const reader = readable.getReader();
                const readableLockedAfterGet = readable.locked;
                reader.releaseLock();
                const readableLockedAfterRelease = readable.locked;
                const writer = writable.getWriter();
                const writableLockedAfterGet = writable.locked;
                writer.releaseLock();
                const writableLockedAfterRelease = writable.locked;
                return JSON.stringify({
                    readableMethods: [
                        methodDescriptor(ReadableStream.prototype, "getReader"),
                        methodDescriptor(ReadableStream.prototype, "cancel"),
                        methodDescriptor(ReadableStream.prototype, "pipeThrough"),
                        methodDescriptor(ReadableStream.prototype, "pipeTo"),
                        methodDescriptor(ReadableStream.prototype, "tee"),
                        methodDescriptor(ReadableStream.prototype, "values"),
                        methodDescriptor(
                            ReadableStream.prototype,
                            Symbol.asyncIterator,
                            "Symbol.asyncIterator"
                        ),
                    ],
                    readableLocked: accessorDescriptor(ReadableStream.prototype, "locked"),
                    readerMethods: [
                        methodDescriptor(ReadableStreamDefaultReader.prototype, "read"),
                        methodDescriptor(ReadableStreamDefaultReader.prototype, "releaseLock"),
                        methodDescriptor(ReadableStreamDefaultReader.prototype, "cancel"),
                    ],
                    readerAccessors: [
                        accessorDescriptor(ReadableStreamDefaultReader.prototype, "closed"),
                    ],
                    readableIteratorAlias:
                        ReadableStream.prototype.values ===
                        ReadableStream.prototype[Symbol.asyncIterator],
                    readableOwnNames: Object.getOwnPropertyNames(readable),
                    writableMethods: [
                        methodDescriptor(WritableStream.prototype, "getWriter"),
                        methodDescriptor(WritableStream.prototype, "abort"),
                        methodDescriptor(WritableStream.prototype, "close"),
                    ],
                    writableLocked: accessorDescriptor(WritableStream.prototype, "locked"),
                    writerMethods: [
                        methodDescriptor(WritableStreamDefaultWriter.prototype, "write"),
                        methodDescriptor(WritableStreamDefaultWriter.prototype, "close"),
                        methodDescriptor(WritableStreamDefaultWriter.prototype, "abort"),
                        methodDescriptor(WritableStreamDefaultWriter.prototype, "releaseLock"),
                    ],
                    writableOwnNames: Object.getOwnPropertyNames(writable),
                    lockStates: [
                        readableLockedAfterGet,
                        readableLockedAfterRelease,
                        writableLockedAfterGet,
                        writableLockedAfterRelease,
                    ],
                });
            })()
            "#,
        )
        .expect("stream prototype descriptors should evaluate");

    assert_eq!(
        result,
        "{\"readableMethods\":[\"getReader:function:getReader:0:true:true:true\",\"cancel:function:cancel:0:true:true:true\",\"pipeThrough:function:pipeThrough:1:true:true:true\",\"pipeTo:function:pipeTo:1:true:true:true\",\"tee:function:tee:0:true:true:true\",\"values:function:values:0:true:true:true\",\"Symbol.asyncIterator:function:values:0:false:true:true\"],\"readableLocked\":\"locked:function:get locked:0:true:false:true\",\"readerMethods\":[\"read:function:read:0:true:true:true\",\"releaseLock:function:releaseLock:0:true:true:true\",\"cancel:function:cancel:0:true:true:true\"],\"readerAccessors\":[\"closed:function:get closed:0:true:false:true\"],\"readableIteratorAlias\":true,\"readableOwnNames\":[],\"writableMethods\":[\"getWriter:function:getWriter:0:true:true:true\",\"abort:function:abort:0:true:true:true\",\"close:function:close:0:true:true:true\"],\"writableLocked\":\"locked:function:get locked:0:true:false:true\",\"writerMethods\":[\"write:function:write:0:true:true:true\",\"close:function:close:0:true:true:true\",\"abort:function:abort:0:true:true:true\",\"releaseLock:function:releaseLock:0:true:true:true\"],\"writableOwnNames\":[],\"lockStates\":[true,false,true,false]}"
    );
}

#[test]
fn readable_stream_default_reader_closed_tracks_close_and_error() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__readerClosedEvents = [];

                const closing = new ReadableStream({
                    start(controller) {
                        globalThis.__readerClosedCloseController = controller;
                    }
                });
                const closingReader = closing.getReader();
                const closingClosed = closingReader.closed;
                globalThis.__readerClosedEvents.push(
                    "close-same:" + (closingReader.closed === closingClosed) + ":" +
                    (closingClosed instanceof Promise)
                );
                closingClosed.then(
                    () => globalThis.__readerClosedEvents.push("close:resolved"),
                    error => globalThis.__readerClosedEvents.push("close:" + error.message)
                );
                globalThis.__readerClosedCloseController.close();

                const erroring = new ReadableStream({
                    start(controller) {
                        globalThis.__readerClosedErrorController = controller;
                    }
                });
                const erroringReader = erroring.getReader();
                const erroringClosed = erroringReader.closed;
                globalThis.__readerClosedEvents.push(
                    "error-same:" + (erroringReader.closed === erroringClosed)
                );
                erroringClosed.then(
                    () => globalThis.__readerClosedEvents.push("error:resolved"),
                    error => globalThis.__readerClosedEvents.push(
                        "error:" + error.name + ":" + error.message
                    )
                );
                globalThis.__readerClosedErrorController.error(new Error("boom"));

                const lateError = new ReadableStream({
                    start(controller) {
                        controller.error(new Error("late"));
                    }
                });
                lateError.getReader().closed.then(
                    () => globalThis.__readerClosedEvents.push("late:resolved"),
                    error => globalThis.__readerClosedEvents.push(
                        "late:" + error.name + ":" + error.message
                    )
                );

                return JSON.stringify(globalThis.__readerClosedEvents);
            })()
            "#,
        )
        .expect("ReadableStreamDefaultReader.closed setup should evaluate");
    assert_eq!(initial, r#"["close-same:true:true","error-same:true"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__readerClosedEvents.sort())")
        .expect("ReadableStreamDefaultReader.closed promises should settle");
    assert_eq!(
        settled,
        r#"["close-same:true:true","close:resolved","error-same:true","error:Error:boom","late:Error:late"]"#
    );
}

#[test]
fn readable_stream_default_reader_release_lock_rejects_closed_and_reads() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__readerReleaseEvents = [];
                const releasedPending = new ReadableStream({
                    start(controller) {
                        globalThis.__releasedPendingController = controller;
                    }
                });
                const pendingReader = releasedPending.getReader();
                const pendingRead = pendingReader.read();
                const pendingClosedBefore = pendingReader.closed;
                pendingClosedBefore.then(
                    () => globalThis.__readerReleaseEvents.push("pending-closed:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "pending-closed:" + error.name + ":" + (error instanceof TypeError)
                    )
                );
                pendingRead.then(
                    () => globalThis.__readerReleaseEvents.push("pending-read:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "pending-read:" + error.name + ":" + (error instanceof TypeError)
                    )
                );
                pendingReader.releaseLock();
                const pendingClosedAfter = pendingReader.closed;
                pendingClosedAfter.catch(() => {});
                pendingReader.read().then(
                    () => globalThis.__readerReleaseEvents.push("future-read:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "future-read:" + error.name + ":" + (error instanceof TypeError)
                    )
                );
                pendingReader.cancel("released").then(
                    () => globalThis.__readerReleaseEvents.push("future-cancel:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "future-cancel:" + error.name + ":" + (error instanceof TypeError)
                    )
                );

                const closedStream = new ReadableStream({
                    start(controller) {
                        controller.close();
                    }
                });
                const closedReader = closedStream.getReader();
                const closedBefore = closedReader.closed;
                closedBefore.then(
                    () => globalThis.__readerReleaseEvents.push("closed-before:resolved"),
                    error => globalThis.__readerReleaseEvents.push("closed-before:" + error.name)
                );
                closedReader.releaseLock();
                const closedAfter = closedReader.closed;
                closedAfter.then(
                    () => globalThis.__readerReleaseEvents.push("closed-after:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "closed-after:" + error.name + ":" + (error instanceof TypeError)
                    )
                );

                return JSON.stringify({
                    locked: releasedPending.locked,
                    pendingClosedSame: pendingClosedBefore === pendingClosedAfter,
                    closedReplaced: closedBefore !== closedAfter,
                    events: globalThis.__readerReleaseEvents
                });
            })()
            "#,
        )
        .expect("ReadableStreamDefaultReader.releaseLock setup should evaluate");

    assert_eq!(
        initial,
        r#"{"locked":false,"pendingClosedSame":true,"closedReplaced":true,"events":[]}"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__readerReleaseEvents.sort())")
        .expect("ReadableStreamDefaultReader.releaseLock promises should settle");
    assert_eq!(
        settled,
        r#"["closed-after:TypeError:true","closed-before:resolved","future-cancel:TypeError:true","future-read:TypeError:true","pending-closed:TypeError:true","pending-read:TypeError:true"]"#
    );
}

#[test]
fn readable_stream_default_reader_release_lock_suppresses_internal_closed_rejection() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__readerReleaseUnhandled = [];
                window.addEventListener("unhandledrejection", event => {
                    globalThis.__readerReleaseUnhandled.push(
                        "unhandled:" + event.reason.name
                    );
                });
                const stream = new ReadableStream({
                    start(controller) {
                        globalThis.__readerReleaseSuppressController = controller;
                    }
                });
                const reader = stream.getReader();
                const savedClosed = reader.closed;
                reader.releaseLock();
                return JSON.stringify({
                    locked: stream.locked,
                    closedSame: reader.closed === savedClosed
                });
            })()
            "#,
        )
        .expect("ReadableStreamDefaultReader.releaseLock suppress setup should evaluate");
    assert_eq!(initial, r#"{"locked":false,"closedSame":true}"#);

    for _ in 0..4 {
        vm.eval("JSON.stringify(globalThis.__readerReleaseUnhandled)")
            .expect("ReadableStreamDefaultReader.releaseLock suppress promises should drain");
    }

    let unhandled = vm
        .eval("JSON.stringify(globalThis.__readerReleaseUnhandled)")
        .expect("ReadableStreamDefaultReader.releaseLock suppress events should evaluate");
    assert_eq!(unhandled, "[]");
}

#[test]
fn readable_stream_default_reader_constructor_and_get_reader_mode_match_wpt() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const events = [];
                const nameOf = callback => {
                    try {
                        callback();
                        return "ok";
                    } catch (error) {
                        return error && error.name;
                    }
                };

                events.push(
                    "missing:" + nameOf(() => new ReadableStreamDefaultReader())
                );
                events.push(
                    "non-stream:" + nameOf(() => new ReadableStreamDefaultReader({}))
                );

                const directStream = new ReadableStream();
                const directReader = new ReadableStreamDefaultReader(directStream);
                const directClosed = directReader.closed;
                events.push(
                    "direct:" +
                    (directReader instanceof ReadableStreamDefaultReader) + ":" +
                    directStream.locked + ":" +
                    (directReader.closed === directClosed)
                );
                events.push(
                    "direct-locked:" +
                    nameOf(() => new ReadableStreamDefaultReader(directStream))
                );
                events.push(
                    "get-reader-locked:" + nameOf(() => directStream.getReader())
                );
                directReader.releaseLock();

                const closedStream = new ReadableStream({
                    start(controller) {
                        controller.close();
                    }
                });
                const closedReader = new ReadableStreamDefaultReader(closedStream);
                events.push("closed-direct:" + closedStream.locked);
                closedReader.releaseLock();

                const erroredStream = new ReadableStream({
                    start(controller) {
                        controller.error(new Error("stream-error"));
                    }
                });
                const erroredReader = new ReadableStreamDefaultReader(erroredStream);
                erroredReader.closed.catch(() => {});
                events.push("errored-direct:" + erroredStream.locked);
                erroredReader.releaseLock();

                let toStringCalled = false;
                const modeStream = new ReadableStream();
                const modeError = nameOf(() => modeStream.getReader({
                    mode: {
                        toString() {
                            toStringCalled = true;
                            return "";
                        }
                    }
                }));
                events.push("mode:" + modeError + ":" + toStringCalled);

                return events.join("|");
            })()
            "#,
        )
        .expect("ReadableStreamDefaultReader constructor probe should evaluate");

    assert_eq!(
        result,
        "missing:TypeError|non-stream:TypeError|direct:true:true:true|direct-locked:TypeError|get-reader-locked:TypeError|closed-direct:true|errored-direct:true|mode:TypeError:true"
    );
}

#[test]
fn readable_stream_start_rejected_promise_errors_stream_with_reason() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__startRejectEvents = [];
                const stream = new ReadableStream({
                    start() {
                        return Promise.reject();
                    }
                });
                const reader = stream.getReader();
                reader.read().then(
                    () => globalThis.__startRejectEvents.push("read:resolved"),
                    error => globalThis.__startRejectEvents.push(
                        "read:" + String(error) + ":" + (error === undefined)
                    )
                );
                reader.closed.then(
                    () => globalThis.__startRejectEvents.push("closed:resolved"),
                    error => globalThis.__startRejectEvents.push(
                        "closed:" + String(error) + ":" + (error === undefined)
                    )
                );
                return JSON.stringify(globalThis.__startRejectEvents);
            })()
            "#,
        )
        .expect("ReadableStream start reject setup should evaluate");
    assert_eq!(initial, "[]");

    vm.eval("undefined")
        .expect("ReadableStream start reject should advance one microtask turn");
    let settled = vm
        .eval("JSON.stringify(globalThis.__startRejectEvents.sort())")
        .expect("ReadableStream start reject promises should settle");
    assert_eq!(
        settled,
        r#"["closed:undefined:true","read:undefined:true"]"#
    );
}

#[test]
fn readable_stream_cancel_rejects_locked_stream_without_calling_source_cancel() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__lockedCancelEvents = [];
                const stream = new ReadableStream({
                    start(controller) {
                        controller.enqueue("a");
                        controller.close();
                    },
                    cancel() {
                        globalThis.__lockedCancelEvents.push("source-cancel");
                    }
                });
                const reader = stream.getReader();
                stream.cancel().then(
                    () => globalThis.__lockedCancelEvents.push("cancel:resolved"),
                    error => globalThis.__lockedCancelEvents.push("cancel:" + error.name)
                );
                reader.read().then(({ value, done }) => {
                    globalThis.__lockedCancelEvents.push(`read:${value}:${done}`);
                });
                reader.closed.then(() => {
                    globalThis.__lockedCancelEvents.push("closed");
                });
                return JSON.stringify(globalThis.__lockedCancelEvents);
            })()
            "#,
        )
        .expect("ReadableStream locked cancel setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__lockedCancelEvents.sort())")
        .expect("ReadableStream locked cancel promises should settle");
    assert_eq!(settled, r#"["cancel:TypeError","closed","read:a:false"]"#);
}

#[test]
fn readable_stream_cancel_follows_underlying_source_result() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__cancelEvents = [];
                const thrown = new Error("thrown");
                const rejectReason = new Error("reject");

                new ReadableStream({
                    cancel(reason) {
                        globalThis.__cancelEvents.push("throw-called:" + reason);
                        throw thrown;
                    }
                }).cancel("sync").then(
                    () => globalThis.__cancelEvents.push("throw:resolved"),
                    error => globalThis.__cancelEvents.push("throw:" + (error === thrown))
                );

                let resolveSource;
                let sourceFulfilled = false;
                const resolved = new ReadableStream({
                    cancel() {
                        const promise = new Promise(resolve => { resolveSource = resolve; });
                        promise.then(() => { sourceFulfilled = true; });
                        return promise;
                    }
                });
                resolved.cancel().then(value => {
                    globalThis.__cancelEvents.push(
                        "resolve:" + sourceFulfilled + ":" + (value === undefined)
                    );
                });
                globalThis.__resolveCancel = () => resolveSource("ignored");

                let rejectSource;
                let sourceRejected = false;
                const rejected = new ReadableStream({
                    cancel() {
                        const promise = new Promise((_, reject) => { rejectSource = reject; });
                        promise.catch(() => { sourceRejected = true; });
                        return promise;
                    }
                });
                rejected.cancel().then(
                    () => globalThis.__cancelEvents.push("reject:resolved"),
                    error => globalThis.__cancelEvents.push(
                        "reject:" + sourceRejected + ":" + (error === rejectReason)
                    )
                );
                globalThis.__rejectCancel = () => rejectSource(rejectReason);
                return JSON.stringify(globalThis.__cancelEvents);
            })()
            "#,
        )
        .expect("ReadableStream cancel source result setup should evaluate");
    assert_eq!(initial, r#"["throw-called:sync"]"#);

    let after_throw = vm
        .eval("JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel sync throw should settle");
    assert_eq!(after_throw, r#"["throw-called:sync","throw:true"]"#);

    let after_resolve = vm
        .eval("globalThis.__resolveCancel(); JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel resolve should be scheduled");
    assert_eq!(after_resolve, r#"["throw-called:sync","throw:true"]"#);
    let after_resolve_settled = vm
        .eval("JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel resolve should settle");
    assert_eq!(
        after_resolve_settled,
        r#"["throw-called:sync","throw:true","resolve:true:true"]"#
    );

    let after_reject = vm
        .eval("globalThis.__rejectCancel(); JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel reject should be scheduled");
    assert_eq!(
        after_reject,
        r#"["throw-called:sync","throw:true","resolve:true:true"]"#
    );
    let after_reject_settled = vm
        .eval("JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel reject should settle");
    assert_eq!(
        after_reject_settled,
        r#"["throw-called:sync","throw:true","resolve:true:true","reject:true:true"]"#
    );
}
