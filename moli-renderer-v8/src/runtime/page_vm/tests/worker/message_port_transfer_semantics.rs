use super::*;

#[tokio::test]
async fn messageport_dispatch_uses_registration_order_for_onmessage_and_listeners() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__messagePortEventOrderResult = [];
                            globalThis.__messagePortEventOrderDone = false;
                            let remaining = 2;
                            function poisonPortInternals(port) {
                                Object.defineProperties(port, {
                                    __lmMessagePortOnmessageHandler: {
                                        value: () => {
                                            throw new Error('own onmessage spoof called');
                                        },
                                        configurable: true
                                    },
                                    __lmMessagePortOnmessageOrder: {
                                        value: -1000,
                                        configurable: true
                                    },
                                    __lmMessagePortNextListenerOrder: {
                                        value: 1000,
                                        configurable: true
                                    },
                                    __moliMessagePortStarted: {
                                        value: false,
                                        configurable: true
                                    },
                                    __moliMessagePortClosed: {
                                        value: true,
                                        configurable: true
                                    },
                                    __moliMessagePortListeners: {
                                        value: [],
                                        configurable: true
                                    }
                                });
                            }
                            function finish(label, order, channel) {
                                if (order.length !== 2) {
                                    return;
                                }
                                globalThis.__messagePortEventOrderResult.push(
                                    `${label}:${order.join(',')}`
                                );
                                channel.port1.close();
                                channel.port2.close();
                                remaining -= 1;
                                if (remaining === 0) {
                                    globalThis.__messagePortEventOrderDone = true;
                                }
                            }

                            const listenerFirst = new MessageChannel();
                            const listenerFirstOrder = [];
                            listenerFirst.port2.addEventListener('message', () => {
                                listenerFirstOrder.push('listener');
                                finish('listener-first', listenerFirstOrder, listenerFirst);
                            });
                            listenerFirst.port2.onmessage = () => {
                                listenerFirstOrder.push('onmessage');
                                finish('listener-first', listenerFirstOrder, listenerFirst);
                            };
                            listenerFirst.port2.start();
                            poisonPortInternals(listenerFirst.port2);
                            listenerFirst.port1.postMessage('go');

                            const onmessageFirst = new MessageChannel();
                            const onmessageFirstOrder = [];
                            onmessageFirst.port2.onmessage = () => {
                                onmessageFirstOrder.push('onmessage');
                                finish('onmessage-first', onmessageFirstOrder, onmessageFirst);
                            };
                            onmessageFirst.port2.addEventListener('message', () => {
                                onmessageFirstOrder.push('listener');
                                finish('onmessage-first', onmessageFirstOrder, onmessageFirst);
                            });
                            onmessageFirst.port2.start();
                            poisonPortInternals(onmessageFirst.port2);
                            onmessageFirst.port1.postMessage('go');
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__messagePortEventOrderDone === true)",
                    "MessagePort event order should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__messagePortEventOrderResult.sort().join('|')")
            })
            .await
            .expect("MessagePort event order test should run on owner lane");

        assert_eq!(
            result,
            "listener-first:listener,onmessage|onmessage-first:onmessage,listener"
        );
    })
    .await;
}

#[tokio::test]
async fn messageport_listener_options_dedupe_once_and_capture_removal() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__messagePortListenerOptionsResult = [];
                            globalThis.__messagePortListenerOptionsDone = false;
                            let remaining = 3;
                            function finish(label, value, channel) {
                                globalThis.__messagePortListenerOptionsResult.push(
                                    `${label}:${value}`
                                );
                                channel.port1.close();
                                channel.port2.close();
                                remaining -= 1;
                                if (remaining === 0) {
                                    globalThis.__messagePortListenerOptionsDone = true;
                                }
                            }

                            const duplicate = new MessageChannel();
                            let duplicateCalls = 0;
                            function duplicateListener() {
                                duplicateCalls += 1;
                            }
                            duplicate.port2.addEventListener('message', duplicateListener);
                            duplicate.port2.addEventListener('message', duplicateListener);
                            duplicate.port2.addEventListener('message', () => {
                                finish('duplicate', String(duplicateCalls), duplicate);
                            }, { once: true });
                            duplicate.port2.start();
                            duplicate.port1.postMessage('go');

                            const once = new MessageChannel();
                            const onceEvents = [];
                            once.port2.addEventListener('message', (event) => {
                                onceEvents.push(event.data);
                            }, { once: true });
                            once.port2.addEventListener('message', (event) => {
                                if (event.data === 'second') {
                                    setTimeout(() => {
                                        finish('once', onceEvents.join(','), once);
                                    }, 0);
                                }
                            });
                            once.port2.start();
                            once.port1.postMessage('first');
                            once.port1.postMessage('second');

                            const capture = new MessageChannel();
                            let captureCalls = 0;
                            function captureListener() {
                                captureCalls += 1;
                            }
                            capture.port2.addEventListener('message', captureListener);
                            capture.port2.addEventListener('message', captureListener, true);
                            capture.port2.removeEventListener('message', captureListener);
                            capture.port2.addEventListener('message', () => {
                                finish('capture', String(captureCalls), capture);
                            }, { once: true });
                            capture.port2.start();
                            capture.port1.postMessage('go');
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__messagePortListenerOptionsDone === true)",
                    "MessagePort listener options should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__messagePortListenerOptionsResult.sort().join('|')")
            })
            .await
            .expect("MessagePort listener options test should run on owner lane");

        assert_eq!(result, "capture:1|duplicate:1|once:first");
    })
    .await;
}

#[tokio::test]
async fn messageport_init_event_is_ignored_during_dispatch() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            globalThis.__messagePortInitEventResult = null;
                            globalThis.__messagePortInitEventDone = false;
                            const channel = new MessageChannel();
                            channel.port2.onmessage = (event) => {
                                const before = [
                                    event.type,
                                    event.bubbles,
                                    event.cancelable,
                                    event.target === channel.port2,
                                    event.currentTarget === channel.port2,
                                    event.eventPhase,
                                    event.srcElement === channel.port2,
                                ].join('|');
                                event.initEvent('mutated', true, true);
                                const after = [
                                    event.type,
                                    event.bubbles,
                                    event.cancelable,
                                    event.target === channel.port2,
                                    event.currentTarget === channel.port2,
                                    event.eventPhase,
                                    event.srcElement === channel.port2,
                                ].join('|');
                                globalThis.__messagePortInitEventResult = `${before}->${after}`;
                                channel.port1.close();
                                channel.port2.close();
                                globalThis.__messagePortInitEventDone = true;
                            };
                            channel.port1.postMessage('go');
                        })()
                        "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__messagePortInitEventDone === true)",
                    "MessagePort initEvent dispatch suppression should complete",
                )
                .await?;
                page_vm
                    .vm_mut()
                    .eval("globalThis.__messagePortInitEventResult")
            })
            .await
            .expect("MessagePort initEvent suppression test should run on owner lane");

        assert_eq!(
            result,
            "message|false|false|true|true|2|true->message|false|false|true|true|2|true"
        );
    })
    .await;
}

#[tokio::test]
async fn worker_messageport_transfer_from_worker_to_window_round_trips_messages() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__workerCreatedPortResult = null;
                            globalThis.__workerCreatedPortDone = false;
                            const worker = new Worker(
                                "data:text/javascript,const channel = new MessageChannel(); channel.port1.onmessage = (event) => { channel.port1.postMessage(`worker:${event.data}`); }; postMessage('port-ready', [channel.port2]);"
                            );
                            worker.onmessage = (event) => {
                                if (event.data !== 'port-ready') {
                                    return;
                                }
                                const port = event.ports[0];
                                port.onmessage = (messageEvent) => {
                                    globalThis.__workerCreatedPortResult = [
                                        messageEvent.data,
                                        String(event.ports.length),
                                    ].join('|');
                                    globalThis.__workerCreatedPortDone = true;
                                };
                                port.postMessage('ping');
                            };
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerCreatedPortDone === true)",
                        "worker MessagePort transfer from worker should complete",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__workerCreatedPortResult")
                })
                .await
                .expect("worker-created MessagePort transfer test should run on owner lane");

            assert_eq!(result, "worker:ping|1");
        })
        .await;
}

#[tokio::test]
async fn worker_postmessage_rejects_messageport_without_transfer_list_in_page_vm() {
    let _ = run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            const worker = new Worker("data:text/javascript,postMessage('ready')");
                            const channel = new MessageChannel();
                            try {
                                worker.postMessage(channel.port1);
                                return "unexpected";
                            } catch (error) {
                                return error.name;
                            }
                        })()
                        "#,
                )
            })
            .await
            .expect("worker MessagePort rejection test should run on owner lane");

        assert_eq!(result, "DataCloneError");
        anyhow::Ok(())
    })
    .await;
}

#[tokio::test]
async fn worker_postmessage_rejects_nested_messageport_without_transfer_list_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__workerNestedPortDone = false;
                            globalThis.__workerNestedPortResult = null;
                            const worker = new Worker("data:text/javascript,postMessage('ready')");
                            const probe = new MessageChannel();
                            let errorName = "unexpected";
                            try {
                                worker.postMessage({ port: probe.port1 });
                            } catch (error) {
                                errorName = error.name;
                            }
                            probe.port2.onmessage = (event) => {
                                globalThis.__workerNestedPortResult = `${errorName}|${String(event.data)}`;
                                globalThis.__workerNestedPortDone = true;
                            };
                            probe.port1.postMessage("still-live");
                            worker.terminate();
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerNestedPortDone === true)",
                        "worker nested MessagePort rejection should preserve the port",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__workerNestedPortResult")
                })
                .await
                .expect("worker nested MessagePort rejection test should run on owner lane");

            assert_eq!(result, "DataCloneError|still-live");
        })
        .await;
}

#[tokio::test]
async fn worker_postmessage_rejects_duplicate_messageport_options_transfer_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__workerDuplicateOptionsPortDone = false;
                            globalThis.__workerDuplicateOptionsPortResult = null;
                            const worker = new Worker("data:text/javascript,postMessage('ready')");
                            const probe = new MessageChannel();
                            let errorName = "unexpected";
                            try {
                                worker.postMessage("payload", { transfer: [probe.port1, probe.port1] });
                            } catch (error) {
                                errorName = error.name;
                            }
                            probe.port2.onmessage = (event) => {
                                globalThis.__workerDuplicateOptionsPortResult = `${errorName}|${String(event.data)}`;
                                globalThis.__workerDuplicateOptionsPortDone = true;
                                worker.terminate();
                                probe.port1.close();
                                probe.port2.close();
                            };
                            probe.port1.postMessage("still-live");
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__workerDuplicateOptionsPortDone === true)",
                        "worker duplicate MessagePort options.transfer rejection should preserve the port",
                    )
                    .await?;
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__workerDuplicateOptionsPortResult")
                })
                .await
                .expect("worker duplicate MessagePort options.transfer test should run on owner lane");

            assert_eq!(result, "DataCloneError|still-live");
        })
        .await;
}

#[tokio::test]
async fn worker_postmessage_rejects_detached_arraybuffer_transfer_in_page_vm() {
    let _ = run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            const worker = new Worker("data:text/javascript,onmessage = function () {}");
                            try {
                                const buffer = new ArrayBuffer(4);
                                worker.postMessage(buffer, [buffer]);
                                let errorName = "unexpected";
                                try {
                                    worker.postMessage(buffer, [buffer]);
                                } catch (error) {
                                    errorName = error.name;
                                }
                                return `${buffer.byteLength}|${errorName}`;
                            } finally {
                                worker.terminate();
                            }
                        })()
                        "#,
                    )
                })
                .await
                .expect("worker detached ArrayBuffer rejection test should run on owner lane");

            assert_eq!(result, "0|DataCloneError");
            anyhow::Ok(())
        })
        .await;
}

#[tokio::test]
async fn messageport_postmessage_rejects_nested_messageport_without_transfer_list_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__portNestedPortDone = false;
                            globalThis.__portNestedPortResult = null;
                            const channel = new MessageChannel();
                            const probe = new MessageChannel();
                            let errorName = "unexpected";
                            try {
                                channel.port1.postMessage({ port: probe.port1 });
                            } catch (error) {
                                errorName = error.name;
                            }
                            probe.port2.onmessage = (event) => {
                                globalThis.__portNestedPortResult = `${errorName}|${String(event.data)}`;
                                globalThis.__portNestedPortDone = true;
                                channel.port1.close();
                                channel.port2.close();
                                probe.port1.close();
                                probe.port2.close();
                            };
                            probe.port1.postMessage("still-live");
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__portNestedPortDone === true)",
                        "MessagePort nested MessagePort rejection should preserve the port",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__portNestedPortResult")
                })
                .await
                .expect("MessagePort nested MessagePort rejection test should run on owner lane");

            assert_eq!(result, "DataCloneError|still-live");
        })
        .await;
}

#[tokio::test]
async fn messageport_postmessage_rejects_source_port_transfer_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__portSelfTransferDone = false;
                            globalThis.__portSelfTransferResult = null;
                            const channel = new MessageChannel();
                            let errorName = "unexpected";
                            try {
                                channel.port1.postMessage("ports", [channel.port1]);
                            } catch (error) {
                                errorName = error.name;
                            }
                            channel.port2.onmessage = (event) => {
                                globalThis.__portSelfTransferResult = `${errorName}|${String(event.data)}`;
                                globalThis.__portSelfTransferDone = true;
                                channel.port1.close();
                                channel.port2.close();
                            };
                            channel.port1.postMessage("still-live");
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__portSelfTransferDone === true)",
                        "MessagePort source-port transfer rejection should preserve the port",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__portSelfTransferResult")
                })
                .await
                .expect("MessagePort source-port transfer test should run on owner lane");

            assert_eq!(result, "DataCloneError|still-live");
        })
        .await;
}

#[tokio::test]
async fn messageport_postmessage_rejects_detached_arraybuffer_transfer_in_page_vm() {
    let _ = run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                        (() => {
                            const channel = new MessageChannel();
                            try {
                                const buffer = new ArrayBuffer(4);
                                channel.port1.postMessage(buffer, [buffer]);
                                let errorName = "unexpected";
                                try {
                                    channel.port1.postMessage(buffer, [buffer]);
                                } catch (error) {
                                    errorName = error.name;
                                }
                                return `${buffer.byteLength}|${errorName}`;
                            } finally {
                                channel.port1.close();
                                channel.port2.close();
                            }
                        })()
                        "#,
                )
            })
            .await
            .expect("MessagePort detached ArrayBuffer rejection test should run on owner lane");

        assert_eq!(result, "0|DataCloneError");
        anyhow::Ok(())
    })
    .await;
}

#[tokio::test]
async fn messageport_messageevent_ports_array_is_frozen_in_page_vm() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__portFrozenPortsDone = false;
                            globalThis.__portFrozenPortsResult = null;
                            const channel = new MessageChannel();
                            const transferred = new MessageChannel();
                            channel.port2.onmessage = (event) => {
                                let pushName = "no-throw";
                                try {
                                    event.ports.push("extra");
                                } catch (error) {
                                    pushName = error.name;
                                }
                                globalThis.__portFrozenPortsResult =
                                    `${Object.isFrozen(event.ports)}|${pushName}|${event.ports.length}`;
                                globalThis.__portFrozenPortsDone = true;
                                event.ports[0].close();
                                channel.port1.close();
                                channel.port2.close();
                            };
                            channel.port1.postMessage("payload", [transferred.port1]);
                            transferred.port2.close();
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__portFrozenPortsDone === true)",
                        "MessageEvent.ports should be frozen during MessagePort delivery",
                    )
                    .await?;
                    page_vm.vm_mut().eval("globalThis.__portFrozenPortsResult")
                })
                .await
                .expect("MessagePort frozen ports test should run on owner lane");

            assert_eq!(result, "true|TypeError|1");
        })
        .await;
}

#[tokio::test]
async fn messageport_postmessage_raw_iterable_transfer_moves_buffer_and_port() {
    run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
                        (() => {
                            globalThis.__portRawIterableResult = [];
                            globalThis.__portRawIterableDone = false;
                            let remaining = 2;
                            function finish(value) {
                                globalThis.__portRawIterableResult.push(value);
                                remaining -= 1;
                                if (remaining === 0) {
                                    globalThis.__portRawIterableDone = true;
                                }
                            }

                            const bufferChannel = new MessageChannel();
                            const transferred = new Uint8Array([31, 32]).buffer;
                            const bufferIterable = {
                                [Symbol.iterator]: function* () {
                                    yield transferred;
                                }
                            };
                            bufferChannel.port2.onmessage = (event) => {
                                finish(`buffer:${Array.from(new Uint8Array(event.data.buffer)).join(',')}:${event.ports.length}`);
                                bufferChannel.port1.close();
                                bufferChannel.port2.close();
                            };
                            bufferChannel.port1.postMessage({ buffer: transferred }, bufferIterable);
                            const detached = transferred.byteLength;

                            const control = new MessageChannel();
                            const inner = new MessageChannel();
                            const portIterable = {
                                [Symbol.iterator]: function* () {
                                    yield inner.port2;
                                }
                            };
                            control.port2.onmessage = (event) => {
                                const port = event.data.port;
                                const sameWrapper = port === event.ports[0];
                                port.onmessage = (innerEvent) => {
                                    port.postMessage(`raw:${innerEvent.data}`);
                                };
                                inner.port1.onmessage = (innerEvent) => {
                                    finish(`port:${sameWrapper}:${event.ports.length}:${innerEvent.data}:${detached}`);
                                    control.port1.close();
                                    control.port2.close();
                                    inner.port1.close();
                                    inner.port2.close();
                                };
                                inner.port1.postMessage('ping');
                            };
                            control.port1.postMessage({ port: inner.port2 }, portIterable);
                        })()
                        "#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "String(globalThis.__portRawIterableDone === true)",
                        "MessagePort raw iterable transfer should complete",
                    )
                    .await?;
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__portRawIterableResult.sort().join('|')")
                })
                .await
                .expect("MessagePort raw iterable transfer test should run on owner lane");

            assert_eq!(result, "buffer:31,32:0|port:true:1:raw:ping:0");
        })
        .await;
}
