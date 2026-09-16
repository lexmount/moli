use super::*;

use crate::page_task_queue::PageMessagePortDeliveryTargetEffect;

#[tokio::test(flavor = "current_thread")]
async fn message_port_callback_cleanup_runs_before_dispatch_and_selected_task_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/message-port-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__messagePortBodyBoundary = [];
globalThis.__messagePortBodyBoundaryChannel = new MessageChannel();
const port = __messagePortBodyBoundaryChannel.port1;
const later = () => __messagePortBodyBoundary.push("unexpected later listener");
port.onmessage = event => {
  __messagePortBodyBoundary.push("message:" + event.data);
  Promise.resolve().then(() => {
    __messagePortBodyBoundary.push("microtask:" + event.data);
    if (event.currentTarget !== port || event.eventPhase !== Event.AT_TARGET) {
      __messagePortBodyBoundary.push("callback state cleared before cleanup");
    }
    port.removeEventListener("message", later);
  });
};
port.addEventListener("message", later);
__messagePortBodyBoundaryChannel.port2.postMessage("one");
"queued"
"#,
        )?;

        let task_sources = page_vm.page_task_executor_sources_for_test();
        let (task, same_attachment_task_is_ready) = task_sources
            .take_message_port_delivery_for_executor_test()
            .expect("one exact MessagePort task should be ready");
        let body = page_vm
            .apply_selected_page_message_port_delivery_turn(task, same_attachment_task_is_ready)?;
        assert_eq!(
            body.action.target_effect,
            PageMessagePortDeliveryTargetEffect::ConsumedByCurrentOwner {
                callback_dispatched: true,
            }
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__messagePortBodyBoundary.join('|')")?,
            "message:one|microtask:one",
            "Web IDL callback cleanup precedes later event listeners"
        );

        page_vm.finish_selected_page_callback_task(&loader).await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__messagePortBodyBoundary.join('|')")?,
            "message:one|microtask:one",
            "the task checkpoint must not repeat the callback cleanup microtask"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("MessagePort body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn message_port_delivery_runs_one_event_and_checkpoint_per_typed_turn() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/message-port-owner-turn").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__typedMessagePortEvents = [];
  globalThis.__typedMessagePortChannel = new MessageChannel();
  const { port1, port2 } = __typedMessagePortChannel;
  port1.onmessage = event => {
    __typedMessagePortEvents.push("message:" + event.data);
    Promise.resolve().then(() => {
      __typedMessagePortEvents.push("microtask:" + event.data);
    });
  };
  port2.postMessage("first");
  port2.postMessage("second");
})()
"#,
        )?;

        assert_eq!(page_vm.vm().ms_to_next_timeout(), None);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedMessagePortEvents.join('|')")?,
            "",
            "a timer-deadline observation must not consume a migrated MessagePort task"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader
                )
                .await?,
            "first MessagePort event should consume one selected typed turn"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedMessagePortEvents.join('|')")?,
            "message:first|microtask:first",
            "the first turn must checkpoint its microtasks without draining the second event"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader
                )
                .await?,
            "second MessagePort event should remain for the next selected turn"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedMessagePortEvents.join('|')")?,
            "message:first|microtask:first|message:second|microtask:second"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader
                )
                .await?,
            "one producer task per queued event must not leave a duplicate no-op turn"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("MessagePort typed one-turn test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn message_port_listeners_use_event_listener_callback_interface_semantics() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/message-port-callback-interface").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__messagePortCallbackInterfaceEvents = [];
  globalThis.__messagePortCallbackInterfaceChannel = new MessageChannel();
  const { port1, port2 } = __messagePortCallbackInterfaceChannel;

  const objectListener = {
    handleEvent() {
      throw new Error("the replacement handleEvent operation must be resolved dynamically");
    }
  };
  port1.addEventListener("message", objectListener);
  // A duplicate registration must not replace the first record's options.
  port1.addEventListener("message", objectListener, { once: true });
  objectListener.handleEvent = function(event) {
    __messagePortCallbackInterfaceEvents.push(
      `object:${event.data}:${this === objectListener}:${window.event === event}`
    );
  };

  let callableHandleEventLookups = 0;
  function callable(event) {
    "use strict";
    __messagePortCallbackInterfaceEvents.push(
      `callable:${event.data}:${this === port1}:${event.currentTarget === port1}`
    );
  }
  Object.defineProperty(callable, "handleEvent", {
    get() {
      callableHandleEventLookups += 1;
      throw new Error("callable listeners must not resolve handleEvent");
    }
  });
  port1.addEventListener("message", callable);

  port1.addEventListener("message", event => {
    __messagePortCallbackInterfaceEvents.push(`once:${event.data}`);
  }, { once: true });

  function removedBeforeTurn(event) {
    __messagePortCallbackInterfaceEvents.push(`removed:${event.data}`);
  }
  port1.addEventListener("message", event => {
    __messagePortCallbackInterfaceEvents.push(`remove:${event.data}`);
    port1.removeEventListener("message", removedBeforeTurn);
  });
  port1.addEventListener("message", removedBeforeTurn);

  const late = event => {
    __messagePortCallbackInterfaceEvents.push(`late:${event.data}`);
  };
  port1.addEventListener("message", event => {
    __messagePortCallbackInterfaceEvents.push(`add:${event.data}`);
    port1.addEventListener("message", late);
  });
  port1.onmessage = event => {
    __messagePortCallbackInterfaceEvents.push(`handler:${event.data}`);
  };
  port1.start();
  port2.postMessage("first");
  port2.postMessage("second");
  globalThis.__messagePortCallableHandleEventLookups =
    () => callableHandleEventLookups;
})()
"#,
        )?;

        for _ in 0..2 {
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::MessagePortDelivery,
                        &loader,
                    )
                    .await?,
                "each queued MessagePort event must run through one selected task"
            );
        }

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__messagePortCallbackInterfaceEvents.join('|')")?,
            concat!(
                "object:first:true:true|callable:first:true:true|once:first|remove:first|",
                "add:first|handler:first|object:second:true:true|callable:second:true:true|",
                "remove:second|add:second|handler:second|late:second"
            )
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(__messagePortCallableHandleEventLookups())")?,
            "0",
            "the callable callback-interface branch must never resolve handleEvent"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("MessagePort callback-interface semantics test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn message_port_listener_signal_controls_the_exact_registration() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/message-port-listener-signal").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__messagePortSignalEvents = [];
  globalThis.__messagePortSignalChannel = new MessageChannel();
  const { port1, port2 } = __messagePortSignalChannel;
  globalThis.__messagePortSignalPrimary = new AbortController();
  globalThis.__messagePortSignalDuplicate = new AbortController();
  globalThis.__messagePortSignalOnce = new AbortController();

  const signaled = event => {
    __messagePortSignalEvents.push(`signal:${event.data}`);
  };
  globalThis.__messagePortSignalOnceListener = event => {
    __messagePortSignalEvents.push(`once:${event.data}`);
  };

  // Listener-option linkage is an internal abort algorithm. It must not call
  // an overridable public AbortSignal method.
  __messagePortSignalPrimary.signal.addEventListener = () => {
    throw new Error("public AbortSignal.addEventListener must not be consulted");
  };
  port1.addEventListener("message", signaled, {
    signal: __messagePortSignalPrimary.signal
  });
  // Duplicate registration keeps the first record and therefore its signal.
  port1.addEventListener("message", signaled, {
    signal: __messagePortSignalDuplicate.signal
  });
  port1.addEventListener("message", __messagePortSignalOnceListener, {
    once: true,
    signal: __messagePortSignalOnce.signal
  });

  const alreadyAborted = new AbortController();
  alreadyAborted.abort();
  port1.addEventListener("message", () => {
    __messagePortSignalEvents.push("already-aborted");
  }, { signal: alreadyAborted.signal });

  let invalidSignalThrew = false;
  try {
    port1.addEventListener("message", () => {}, { signal: {} });
  } catch (error) {
    invalidSignalThrew = error instanceof TypeError;
  }
  globalThis.__messagePortInvalidSignalThrew = invalidSignalThrew;

  port1.addEventListener("message", event => {
    __messagePortSignalEvents.push(`base:${event.data}`);
  });
  port1.start();
  port2.postMessage("first");
})()
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader,
                )
                .await?,
            "the first MessagePort event should be selected"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "__messagePortInvalidSignalThrew + ':' + __messagePortSignalEvents.join('|')"
            )?,
            "true:signal:first|once:first|base:first"
        );

        page_vm.vm_mut().eval(
            r#"
__messagePortSignalDuplicate.abort();
__messagePortSignalOnce.abort();
globalThis.__messagePortSignalReplacement = new AbortController();
__messagePortSignalChannel.port1.addEventListener(
  "message",
  __messagePortSignalOnceListener,
  { once: true, signal: __messagePortSignalReplacement.signal }
);
__messagePortSignalChannel.port2.postMessage("second");
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader,
                )
                .await?,
            "aborting the ignored duplicate signal must preserve the original registration"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__messagePortSignalEvents.join('|')")?,
            "signal:first|once:first|base:first|signal:second|base:second|once:second"
        );

        page_vm.vm_mut().eval(
            r#"
__messagePortSignalPrimary.abort();
__messagePortSignalReplacement.abort();
__messagePortSignalChannel.port2.postMessage("third");
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader,
                )
                .await?,
            "the baseline listener should keep the port delivery observable"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__messagePortSignalEvents.join('|')")?,
            concat!(
                "signal:first|once:first|base:first|signal:second|base:second|",
                "once:second|base:third"
            )
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("MessagePort signal registration test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn message_port_listener_uses_callback_realm_and_retires_with_it() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/message-port-callback-realm").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "message-port-callback-realm";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "message-port-callback-realm",
        )?;
        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.getElementById("message-port-callback-realm");
  const other = frame.contentWindow;
  globalThis.__messagePortRealmFacts = [];
  globalThis.__messagePortRealmTopEvents = [];
  globalThis.__messagePortRealmReported = null;
  globalThis.__messagePortRealmExpected = other;
  globalThis.__messagePortRealmChannel = new MessageChannel();
  const { port1, port2 } = __messagePortRealmChannel;
  globalThis.__messagePortRealmPort = port1;
  const callback = other.Function(
    "event",
    `"use strict";
     parent.__messagePortRealmFacts.push([
       this === parent.__messagePortRealmPort,
       globalThis === parent.__messagePortRealmExpected,
       window.event === event,
       event.currentTarget === parent.__messagePortRealmPort
     ]);`
  );
  port1.addEventListener("message", callback);
  const missingOperation = new other.Object();
  const onError = event => {
    __messagePortRealmReported = {
      relevantTypeError:
        event.error instanceof other.TypeError &&
        !(event.error instanceof TypeError),
      targetIsCallbackWindow: event.currentTarget === other
    };
    event.preventDefault();
  };
  other.addEventListener("error", onError);
  port1.addEventListener("message", missingOperation, { once: true });
  port1.onmessage = event => __messagePortRealmTopEvents.push(event.data);
  port1.start();
  port2.postMessage("before-retirement");
})()
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "JSON.stringify({ facts: __messagePortRealmFacts, reported: __messagePortRealmReported, top: __messagePortRealmTopEvents })"
            )?,
            r#"{"facts":[[true,true,true,true]],"reported":{"relevantTypeError":true,"targetIsCallbackWindow":true},"top":["before-retirement"]}"#
        );

        page_vm.vm_mut().eval(
            r#"
document.getElementById("message-port-callback-realm").remove();
__messagePortRealmChannel.port2.postMessage("after-retirement");
"queued"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader,
                )
                .await?,
            "retiring the callback Window must not retire the top-owned port"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "JSON.stringify({ facts: __messagePortRealmFacts, reported: __messagePortRealmReported, top: __messagePortRealmTopEvents })"
            )?,
            r#"{"facts":[[true,true,true,true]],"reported":{"relevantTypeError":true,"targetIsCallbackWindow":true},"top":["before-retirement","after-retirement"]}"#,
            "the retired callback must be removed while the exact top-owned wrapper keeps delivering"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("MessagePort callback Realm/lifetime test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn message_port_completion_syncs_a_microtask_created_child_after_the_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/message-port-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__messagePortChildOrder = [];
globalThis.__messagePortChildChannel = new MessageChannel();
__messagePortChildChannel.port1.onmessage = () => {
  __messagePortChildOrder.push("callback");
  Promise.resolve().then(() => {
    __messagePortChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "message-port-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
};
__messagePortChildChannel.port2.postMessage("create-child");
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MessagePortDelivery, &loader)
                .await?,
            "the exact MessagePort task should run through the selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__messagePortChildOrder.join('|')")?,
            "callback|microtask",
            "the agent checkpoint must precede callback child-record synchronization"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a reaction-created srcdoc frame must publish a typed navigation commit during callback completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit),
            "the microtask-created frame must remain a concrete later Page task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("MessagePort post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn message_port_close_during_dispatch_preserves_already_queued_delivery_tasks() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/message-port-close-queue").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__typedMessagePortCloseEvents = [];
  globalThis.__typedMessagePortCloseChannel = new MessageChannel();
  const { port1, port2 } = __typedMessagePortCloseChannel;
  port1.onmessage = event => {
    __typedMessagePortCloseEvents.push(event.data);
    if (event.data === "first") {
      port1.close();
    }
  };
  port2.postMessage("first");
  port2.postMessage("second");
})()
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MessagePortDelivery, &loader)
                .await?,
            "the first queued event should consume one selected typed turn"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedMessagePortCloseEvents.join('|')")?,
            "first"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MessagePortDelivery, &loader)
                .await?,
            "close must not cancel a delivery task accepted before the callback"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedMessagePortCloseEvents.join('|')")?,
            "first|second"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MessagePortDelivery, &loader)
                .await?,
            "closing the receiving endpoint should queue one peer close event"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__typedMessagePortCloseEvents.join('|')")?,
            "first|second",
            "a peer close without a close listener must consume its task without dispatching a callback"
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("MessagePort close queue test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn message_port_unstarted_event_is_retained_until_handler_activation_rewakes_it() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/message-port-activation").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__activationEvents = [];
globalThis.__activationChannel = new MessageChannel();
__activationChannel.port2.postMessage("retained");
"queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader
                )
                .await?,
            "the unstarted port still owns one bounded selected delivery opportunity"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__activationEvents.join('|')")?,
            "",
            "an unstarted port must retain its registry payload"
        );

        page_vm.vm_mut().eval(
            r#"
__activationChannel.port1.onmessage = event => __activationEvents.push(event.data);
"activated"
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader
                )
                .await?,
            "installing onmessage must re-admit the retained registry payload"
        );

        assert_eq!(
            page_vm.vm_mut().eval("__activationEvents.join('|')")?,
            "retained"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("MessagePort activation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn message_port_transfer_rejects_old_attachment_before_delivering_to_new_realm() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/message-port-transfer").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "message-port-transfer-target";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "message-port-transfer-target",
        )?;
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__transferredPortEvents = [];
  const frame = document.getElementById("message-port-transfer-target");
  frame.contentWindow.onmessage = event => {
    const port = event.ports[0];
    port.addEventListener(
      "message",
      message => parent.__transferredPortEvents.push(`first:${message.data}`)
    );
    port.addEventListener(
      "message",
      message => parent.__transferredPortEvents.push(`second:${message.data}`)
    );
    port.start();
    // Registrations stay on their original JS object across transfer.
    // Neither an old removed listener's signal nor an old retained listener's
    // signal may remove listeners from this new wrapper.
    parent.__manualSignalController.abort();
    parent.__transferSignalController.abort();
  };
  globalThis.__transferChannel = new MessageChannel();
  globalThis.__manualSignalController = new AbortController();
  globalThis.__transferSignalController = new AbortController();
  const manuallyRemoved = () => {
    throw new Error("a manually removed listener must not dispatch");
  };
  __transferChannel.port1.addEventListener("message", manuallyRemoved, {
    signal: __manualSignalController.signal
  });
  __transferChannel.port1.removeEventListener("message", manuallyRemoved);
  __transferChannel.port1.addEventListener("message", () => {
    throw new Error("the retired top-realm attachment must not dispatch");
  }, { signal: __transferSignalController.signal });
  __transferChannel.port1.start();
  __transferChannel.port2.postMessage("preserved-across-transfer");
  frame.contentWindow.postMessage("attach", "*", [__transferChannel.port1]);
})()
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader
                )
                .await?,
            "the transfer Window.postMessage task should install the child wrapper"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader
                )
                .await?,
            "the old attachment task should consume one stale selected turn"
        );

        assert_eq!(
            page_vm.vm_mut().eval("__transferredPortEvents.join('|')")?,
            ""
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader
                )
                .await?,
            "the new attachment task should remain behind the stale task"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__transferredPortEvents.join('|')")?,
            concat!(
                "first:preserved-across-transfer|",
                "second:preserved-across-transfer"
            ),
            "discarding the old attachment task must not consume the registry payload"
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MessagePortDelivery,
                    &loader
                )
                .await?,
            "materializing one transferred wrapper must not enqueue a duplicate delivery task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("MessagePort exact attachment transfer test should run");
}

#[test]
fn message_port_delivery_rejects_a_real_page_vm_replacement_task() {
    run_page_vm_large_stack_async_test("message-port-page-vm-replacement", || async {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/replacement.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><body>replacement</body>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
        let (page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
globalThis.__retiredMessagePortChannel = new MessageChannel();
__retiredMessagePortChannel.port1.onmessage = () => {
  throw new Error("a retired PageVm must not receive its queued MessagePort event");
};
__retiredMessagePortChannel.port2.postMessage("retired");
"queued"
"#,
                )?;
                let retired_root = page_vm.document_lifecycle.identity().document;

                let replacement_url = format!("{base_url}/replacement.html");
                page_vm
                    .vm_mut()
                    .eval(&format!("location.href = {replacement_url:?}; 'queued'"))?;
                let mut pending_document_lifecycle_turn = None;
                let navigation = page_vm
                    .follow_pending_location_navigation_one_turn_async(
                        &mut pending_document_lifecycle_turn,
                        PageVmInitStage::Load,
                    )
                    .await?;
                assert!(matches!(
                    navigation,
                    crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                        | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                            ..
                        }
                ));

                let current_root = page_vm.document_lifecycle.identity().document;
                assert_ne!(retired_root, current_root);
                page_vm.vm_mut().eval(
                    r#"
globalThis.__replacementMessagePortEvents = [];
globalThis.__replacementMessagePortChannel = new MessageChannel();
__replacementMessagePortChannel.port1.onmessage = event => {
  __replacementMessagePortEvents.push(event.data);
};
__replacementMessagePortChannel.port2.postMessage("current");
"queued"
"#,
                )?;

                let mut stale_turns = 0;
                loop {
                    assert!(
                        page_vm
                            .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MessagePortDelivery, &loader)
                            .await?,
                        "the replacement task must remain behind retired PageVm tasks"
                    );
                    let events = page_vm
                        .vm_mut()
                        .eval("__replacementMessagePortEvents.join('|')")?;
                    if events == "current" {
                        break;
                    }
                    assert_eq!(
                        events, "",
                        "retired message/close tasks must not dispatch through or remove the replacement wrapper payload"
                    );
                    stale_turns += 1;
                    assert!(
                        stale_turns <= 2,
                        "retiring one entangled pair should leave only its queued message and close opportunities"
                    );
                }
                assert!(
                    stale_turns >= 1,
                    "the initial PageVm must leave at least its queued message task"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__replacementMessagePortEvents.join('|')")?,
                    "current"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("MessagePort PageVm replacement should run through the typed executor");
        server
            .await
            .expect("MessagePort PageVm replacement server should finish");
    });
}
