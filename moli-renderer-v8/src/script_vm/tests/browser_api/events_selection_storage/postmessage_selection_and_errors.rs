use super::*;

#[test]
fn document_domain_setter_rejects_ip_suffix_relaxation() {
    let mut vm = new_storage_test_vm("http://127.0.0.1/path");

    let result = vm
        .eval(
            r#"
(() => {
  const initial = document.domain;
  document.domain = "127.0.0.1";
  const exact = document.domain;
  let suffix;
  try {
    document.domain = "0.0.1";
    suffix = "no-throw";
  } catch (error) {
    suffix = `${error.name}:${error instanceof DOMException}:${error.code}:${document.domain}`;
  }
  return `${initial}|${exact}|${suffix}`;
})()
"#,
        )
        .expect("document.domain IP setter probe should evaluate");

    assert_eq!(
        result,
        "127.0.0.1|127.0.0.1|SecurityError:true:18:127.0.0.1"
    );
}

#[test]
fn document_domain_setter_rejects_public_suffix_relaxation() {
    let mut vm = new_storage_test_vm("https://www.co.uk/path");

    let result = vm
        .eval(
            r#"
(() => {
  const initial = document.domain;
  let suffix;
  try {
    document.domain = "co.uk";
    suffix = "no-throw";
  } catch (error) {
    suffix = `${error.name}:${error instanceof DOMException}:${error.code}:${document.domain}`;
  }
  document.domain = "www.co.uk";
  const exact = document.domain;
  return `${initial}|${suffix}|${exact}`;
})()
"#,
        )
        .expect("document.domain public suffix setter probe should evaluate");

    assert_eq!(
        result,
        "www.co.uk|SecurityError:true:18:www.co.uk|www.co.uk"
    );
}

#[test]
fn message_port_is_not_constructible_and_channel_ports_keep_declared_state() {
    let mut vm = new_storage_test_vm("https://message-port-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methodDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(":");
  };
  const accessorDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      descriptor?.enumerable,
      typeof descriptor?.set,
      descriptor?.configurable
    ].join(":");
  };
  let constructorResult;
  try {
    new MessagePort();
    constructorResult = "no-throw";
  } catch (error) {
    constructorResult = `${error.name}:${error instanceof TypeError}`;
  }
  const standalone = new MessageChannel().port1;
  const channel = new MessageChannel();
  const originalPort1 = channel.port1;
  const originalPort2 = channel.port2;
  const messagePortOwnSlots = Object.getOwnPropertyNames(standalone)
    .filter(name => name.startsWith("__lmMessagePort") ||
      name.startsWith("__moliMessagePort"))
    .sort();
  const messageChannelOwnSlots = Object.getOwnPropertyNames(channel)
    .filter(name => name.startsWith("__moliMessageChannel"))
    .sort();
  Object.defineProperties(MessagePort.prototype, {
    __lmMessagePortOnmessageHandler: { value: () => "proto-message", configurable: true },
    __lmMessagePortOnmessageerrorHandler: { value: () => "proto-error", configurable: true },
    __lmMessagePortOncloseHandler: { value: () => "proto-close", configurable: true },
    __moliMessagePortEventListeners: { value: [], configurable: true }
  });
  Object.defineProperties(standalone, {
    __lmMessagePortOnmessageHandler: { value: () => "own-message", configurable: true },
    __lmMessagePortOnmessageerrorHandler: { value: () => "own-error", configurable: true },
    __lmMessagePortOncloseHandler: { value: () => "own-close", configurable: true },
    __moliMessagePortEventListeners: { value: [], configurable: true }
  });
  Object.defineProperties(MessageChannel.prototype, {
    __moliMessageChannelPort1: { value: "proto-port1", configurable: true },
    __moliMessageChannelPort2: { value: "proto-port2", configurable: true }
  });
  Object.defineProperties(channel, {
    __moliMessageChannelPort1: { value: "own-port1", configurable: true },
    __moliMessageChannelPort2: { value: "own-port2", configurable: true }
  });
  standalone.onmessage = () => "real-message";
  standalone.onmessageerror = () => "real-error";
  standalone.onclose = () => "real-close";
  return JSON.stringify({
    constructorResult,
    standaloneTag: Object.prototype.toString.call(standalone),
    standaloneCtor: standalone.constructor && standalone.constructor.name,
    standaloneProtoCtor: Object.getPrototypeOf(standalone)?.constructor?.name ?? null,
    standaloneKeys: Object.keys(standalone).join(","),
    eventTargetMethodsInherited: ["addEventListener", "removeEventListener", "dispatchEvent"].every(name =>
      !Object.hasOwn(MessagePort.prototype, name) && standalone[name] === EventTarget.prototype[name]),
    messagePortMethods: [
      methodDescriptor(MessagePort.prototype, "postMessage"),
      methodDescriptor(MessagePort.prototype, "start"),
      methodDescriptor(MessagePort.prototype, "close"),
      methodDescriptor(EventTarget.prototype, "addEventListener"),
      methodDescriptor(EventTarget.prototype, "removeEventListener")
    ],
    messagePortAccessors: [
      accessorDescriptor(MessagePort.prototype, "onmessage"),
      accessorDescriptor(MessagePort.prototype, "onmessageerror"),
      accessorDescriptor(MessagePort.prototype, "onclose")
    ],
    messageChannelAccessors: [
      accessorDescriptor(MessageChannel.prototype, "port1"),
      accessorDescriptor(MessageChannel.prototype, "port2")
    ],
    messagePortOwnSlots,
    messageChannelOwnSlots,
    standaloneOnmessage: typeof standalone.onmessage,
    standaloneOnmessageIsSpoof: standalone.onmessage === standalone.__lmMessagePortOnmessageHandler,
    standaloneOnmessageerror: typeof standalone.onmessageerror,
    standaloneOnmessageerrorIsSpoof: standalone.onmessageerror === standalone.__lmMessagePortOnmessageerrorHandler,
    standaloneOnclose: typeof standalone.onclose,
    standaloneOncloseIsSpoof: standalone.onclose === standalone.__lmMessagePortOncloseHandler,
    portTag: Object.prototype.toString.call(channel.port1),
    portCtor: channel.port1.constructor && channel.port1.constructor.name,
    portKeys: Object.keys(channel.port1).join(","),
    portOnmessage: channel.port1.onmessage,
    channelKeys: Object.keys(channel).join(","),
    stablePortAccessor: channel.port1 === originalPort1 && channel.port2 === originalPort2,
    channelPortSpoofed: channel.port1 === channel.__moliMessageChannelPort1 ||
      channel.port2 === channel.__moliMessageChannelPort2
  });
})()
"#,
        )
        .expect("MessagePort surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructorResult":"TypeError:true","standaloneTag":"[object MessagePort]","standaloneCtor":"MessagePort","standaloneProtoCtor":"MessagePort","standaloneKeys":"","eventTargetMethodsInherited":true,"messagePortMethods":["postMessage:function:postMessage:1:true:true:true","start:function:start:0:true:true:true","close:function:close:0:true:true:true","addEventListener:function:addEventListener:2:true:true:true","removeEventListener:function:removeEventListener:2:true:true:true"],"messagePortAccessors":["onmessage:function:get onmessage:0:true:function:true","onmessageerror:function:get onmessageerror:0:true:function:true","onclose:function:get onclose:0:true:function:true"],"messageChannelAccessors":["port1:function:get port1:0:true:undefined:true","port2:function:get port2:0:true:undefined:true"],"messagePortOwnSlots":[],"messageChannelOwnSlots":[],"standaloneOnmessage":"function","standaloneOnmessageIsSpoof":false,"standaloneOnmessageerror":"function","standaloneOnmessageerrorIsSpoof":false,"standaloneOnclose":"function","standaloneOncloseIsSpoof":false,"portTag":"[object MessagePort]","portCtor":"MessagePort","portKeys":"","portOnmessage":null,"channelKeys":"","stablePortAccessor":true,"channelPortSpoofed":false}"#
    );
}

#[test]
fn message_channel_construction_keeps_popup_realm_with_ambient_child_marker() {
    let mut vm = new_storage_test_vm("https://message-channel-realm-binding.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc = "<!doctype html><body>child</body>";
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__messageChannelRealmPopup = open(
    "about:blank",
    "message-channel-realm-popup"
  );
  return "ready";
})()
"#,
    )
    .expect("MessageChannel realm-binding setup should evaluate");

    let (child_handle, popup_id, popup_owner) = {
        let host = vm._context_host.borrow();
        let child_handle = host.child_browsing_context_handles_in_document_order()[0];
        let popup_id = host.open_lightweight_popup_ids()[0];
        let local_window_id = host
            .current_lightweight_popup_local_window_id(popup_id)
            .expect("popup LocalWindow should exist");
        (
            child_handle,
            popup_id,
            crate::native_bridge::WindowExecutionContextOwner::LightweightPopup {
                popup_id,
                local_window_id,
            },
        )
    };

    let top_context_ptr = &vm.page_default_runtime.context as *const v8::Global<v8::Context>;
    vm.with_context_scope_by_ptr(top_context_ptr, |scope, _host_ptr| {
        let _previous_child =
            crate::native_bridge::enter_active_child_window_scope(scope, Some(child_handle));
        let _previous_popup =
            crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id);
        Ok(())
    })
    .expect("overlapping ambient owner markers should install");

    let result = vm
        .eval(
            r#"
(() => {
  const channel = new MessageChannel();
  return [
    channel.port1 instanceof MessagePort,
    channel.port2 instanceof MessagePort
  ].join("|");
})()
"#,
        )
        .expect("MessageChannel should construct in the popup realm");

    vm.with_context_scope_by_ptr(top_context_ptr, |scope, _host_ptr| {
        let _previous_child = crate::native_bridge::enter_active_child_window_scope(scope, None);
        let _previous_popup = crate::native_bridge::enter_top_level_lightweight_popup_scope(scope);
        Ok(())
    })
    .expect("ambient owner markers should clear");

    assert_eq!(result, "true|true");
    let owners = vm
        ._context_host
        .borrow()
        .message_port_execution_context_owners_for_test();
    assert_eq!(owners.len(), 2);
    assert!(
        owners.iter().all(|(_, owner, _)| *owner == popup_owner),
        "both MessagePorts must retain the constructor-entry popup realm"
    );
}

#[tokio::test]
async fn window_post_message_structured_clones_data_and_ports() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://message-clone.test/", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageEvents = [];
  const original = [];
  const payload = [original, original];
  const channel = new MessageChannel();
  onmessage = event => {
    __messageEvents.push({
      sharedReference: event.data[0] === event.data[1],
      notOriginal: event.data[0] !== original,
      portCount: event.ports.length,
      portType: Object.prototype.toString.call(event.ports[0])
    });
  };
  postMessage(payload, "*", [channel.port1, channel.port2]);
  return __messageEvents.length;
})()
"#,
        )
        .expect("window postMessage clone setup should evaluate");

    assert_eq!(result, "0");
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("wait driver should drain cloned window message");
    assert_eq!(
        vm.eval("JSON.stringify(__messageEvents)")
            .expect("cloned window message should evaluate"),
        r#"[{"sharedReference":true,"notOriginal":true,"portCount":2,"portType":"[object MessagePort]"}]"#
    );
}

#[tokio::test]
async fn window_post_message_options_transfer_list_transfers_ports() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-options-transfer.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageEvents = [];
  const original = [];
  const payload = [original, original];
  const channel = new MessageChannel();
  onmessage = event => {
    __messageEvents.push({
      sharedReference: event.data[0] === event.data[1],
      notOriginal: event.data[0] !== original,
      portCount: event.ports.length,
      portType: Object.prototype.toString.call(event.ports[0])
    });
  };
  postMessage(payload, {targetOrigin: "*", transfer: [channel.port1, channel.port2]});
  return String(__messageEvents.length);
})()
"#,
        )
        .expect("window postMessage options transfer setup should evaluate");

    assert_eq!(result, "0");
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("wait driver should drain options-transfer window message");
    assert_eq!(
        vm.eval("JSON.stringify(__messageEvents)")
            .expect("options-transfer window message should evaluate"),
        r#"[{"sharedReference":true,"notOriginal":true,"portCount":2,"portType":"[object MessagePort]"}]"#
    );
}

#[tokio::test]
async fn message_port_dispatch_uses_lightweight_popup_owner_scope() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-port-popup-owner.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__messagePortPopupOwnerMessages = [];
  onmessage = event => {
    __messagePortPopupOwnerMessages.push("window:" + event.data + ":" + event.origin);
  };

  const popup = open("https://message-port-popup-child.test/page.html");
  popup.onmessage = event => {
    if (event.data !== "setup") {
      return;
    }
    const channel = new MessageChannel();
    channel.port2.onmessage = () => {
      event.source.postMessage("popup-port-handler-ran", event.origin);
    };
    channel.port1.postMessage("start");
  };
  popup.postMessage("setup", "*");
  return "scheduled";
})()
"#,
        )
        .expect("popup-owned MessagePort setup should evaluate");
    assert_eq!(setup, "scheduled");

    for _ in 0..12 {
        if vm
            .eval(
                r#"String(globalThis.__messagePortPopupOwnerMessages.some(
  message => message.startsWith("window:popup-port-handler-ran:")
))"#,
            )
            .expect("popup-owned MessagePort completion should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance popup-owned MessagePort");
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__messagePortPopupOwnerMessages)")
            .expect("popup-owned MessagePort messages should evaluate"),
        r#"["window:popup-port-handler-ran:https://message-port-popup-child.test"]"#
    );
}

#[tokio::test]
async fn window_message_handler_broadcast_channel_stays_in_lightweight_popup_owner() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_broadcast_channel_page_test_vm_with_loader(
        "https://window-message-popup-broadcast-channel-owner.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__windowMessagePopupBroadcastChannelMessages = [];
  const topChannel = new BroadcastChannel("window-message-popup-broadcast-channel-owner");
  topChannel.onmessage = event => {
    __windowMessagePopupBroadcastChannelMessages.push("top-bc:" + event.data + ":" + event.origin);
  };
  onmessage = event => {
    __windowMessagePopupBroadcastChannelMessages.push("window:" + event.data + ":" + event.origin);
  };

  const popup = open("https://window-message-popup-broadcast-channel-child.test/page.html");
  popup.onmessage = event => {
    if (event.data !== "probe") {
      return;
    }
    const popupChannel = new BroadcastChannel("window-message-popup-broadcast-channel-owner");
    popupChannel.postMessage("from-popup-window-message");
    event.source.postMessage("done", event.origin);
  };
  popup.postMessage("probe", "*");
  return "scheduled";
})()
"#,
        )
        .expect("popup window-message BroadcastChannel owner workflow should schedule");
    assert_eq!(setup, "scheduled");

    for _ in 0..12 {
        if vm
            .eval(
                r#"String(globalThis.__windowMessagePopupBroadcastChannelMessages.some(
  message => message.startsWith("window:")
))"#,
            )
            .expect("popup window-message BroadcastChannel completion should evaluate")
            == "true"
        {
            break;
        }
        let outcome = vm
            .run_one_window_message_executor_turn(&loader)
            .await
            .expect("typed popup Window.postMessage turn should apply");
        assert!(
            outcome,
            "popup window-message workflow should retain a scheduler-visible task"
        );
    }

    vm.apply_pending_broadcast_channel_delivery_tasks(&loader, 4)
        .await
        .expect("any admitted BroadcastChannel executor tasks should apply");

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__windowMessagePopupBroadcastChannelMessages)")
            .expect("popup window-message BroadcastChannel messages should evaluate"),
        r#"["window:done:https://window-message-popup-broadcast-channel-child.test"]"#
    );
}

#[test]
fn window_post_message_requires_message_argument() {
    let mut vm = new_storage_test_vm("https://window-post-message-required-argument.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const outcome = args => {
    try {
      postMessage(...args);
      return "ok";
    } catch (error) {
      return `${error.name}:${error instanceof TypeError}`;
    }
  };
  return [outcome([]), outcome([undefined])].join("|");
})()
"#,
        )
        .expect("Window.postMessage required argument conversion should evaluate");

    assert_eq!(result, "TypeError:true|ok");
}

#[test]
fn body_onmessageerror_content_attribute_reflects_window_handler() {
    let mut vm = new_storage_test_vm("https://body-window-messageerror-handler.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageErrorRuns = 0;
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const initial = [body.onmessageerror, window.onmessageerror];
  body.setAttribute(
    "onmessageerror",
    "globalThis.__messageErrorRuns += 1;"
  );
  const compiled = body.onmessageerror;
  compiled();
  const reflected = window.onmessageerror === compiled;

  body.removeAttribute("onmessageerror");
  const removed = body.onmessageerror === null && window.onmessageerror === null;
  body.setAttribute(
    "onmessageerror",
    "globalThis.__messageErrorRuns += 10;"
  );
  window.dispatchEvent(new Event("messageerror"));

  return JSON.stringify({
    initial: initial.map(value => value === null),
    compiledType: typeof compiled,
    reflected,
    removed,
    runs: globalThis.__messageErrorRuns
  });
})()
"#,
        )
        .expect("body WindowEventHandlers onmessageerror probe should evaluate");

    assert_eq!(
        result,
        r#"{"initial":[true,true],"compiledType":"function","reflected":true,"removed":true,"runs":11}"#
    );
}

#[test]
fn window_post_message_legacy_transfer_argument_must_be_iterable() {
    let mut vm = new_storage_test_vm("https://message-transfer-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = value => {
    try {
      postMessage("", "*", value);
      return "no-throw";
    } catch (error) {
      return `${error.name}:${error instanceof TypeError}`;
    }
  };
  const channel = new MessageChannel();
  channel[0] = channel.port1;
  channel[1] = channel.port2;
  channel.length = 2;
  return [
    probe(null),
    probe(undefined),
    probe(1),
    probe({length: 1}),
    probe(channel)
  ].join("|");
})()
"#,
        )
        .expect("window postMessage transfer validation should evaluate");

    assert_eq!(
        result,
        "TypeError:true|no-throw|TypeError:true|TypeError:true|TypeError:true"
    );
}

#[test]
fn window_post_message_options_transfer_null_throws_type_error() {
    let mut vm = new_storage_test_vm("https://window-post-message-options.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const outcome = options => {
    try {
      postMessage("payload", options);
      return "ok";
    } catch (error) {
      return `${error.name}:${error instanceof TypeError}`;
    }
  };
  const arrayFrom = Array.from;
  Array.from = () => { throw new Error("postMessage transfer must not use Array.from"); };
  const iterableOutcome = outcome({
    transfer: {
      *[Symbol.iterator]() {}
    }
  });
  Array.from = arrayFrom;
  return [
    outcome({}),
    outcome({ transfer: undefined }),
    outcome({ transfer: null }),
    iterableOutcome
  ].join("|");
})()
"#,
        )
        .expect("Window.postMessage options transfer conversion should evaluate");

    assert_eq!(result, "ok|ok|TypeError:true|ok");
}

#[test]
fn window_post_message_wasm_memory_buffer_transfer_throws_type_error() {
    let mut vm = new_storage_test_vm("https://wasm-memory-transfer-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const buffer = new WebAssembly.Memory({ initial: 1 }).buffer;
  try {
    postMessage("payload", "*", [buffer]);
    return "no-throw";
  } catch (error) {
    return `${error.name}:${error instanceof TypeError}:${error instanceof DOMException}`;
  }
})()
"#,
        )
        .expect("wasm memory buffer transfer validation should evaluate");

    assert_eq!(result, "TypeError:true:false");
}

#[tokio::test]
async fn queued_selectionchange_ignores_page_tampered_document_dispatch_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://selectionchange-dispatch-guard.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              globalThis.__selectionTamperedDispatch = "no";
              globalThis.__selectionChangeFired = "no";
              document.addEventListener("selectionchange", () => {
                globalThis.__selectionChangeFired = "yes";
              });
              document.dispatchEvent = () => {
                globalThis.__selectionTamperedDispatch = "yes";
                throw new Error("host must not call document.dispatchEvent");
              };
              getSelection().collapse(text, 1);
              return `${globalThis.__selectionTamperedDispatch}|${globalThis.__selectionChangeFired}|${getSelection().anchorOffset}`;
            })()
            "#,
        )
        .expect("queued selectionchange setup should evaluate");

    assert_eq!(result, "no|no|1");
    assert!(
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance queued selectionchange dispatch")
    );
    assert_eq!(
        vm.eval("`${globalThis.__selectionTamperedDispatch}|${globalThis.__selectionChangeFired}|${getSelection().anchorOffset}`")
            .expect("queued selectionchange dispatch result should evaluate"),
        "no|yes|1"
    );
}

#[tokio::test]
async fn selectionchange_mutation_inside_listener_queues_a_later_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://selectionchange-reentrant-task.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              const selection = getSelection();
              globalThis.__selectionReentrantCount = 0;
              globalThis.__selectionReentrantLog = [];
              document.addEventListener("selectionchange", () => {
                if (globalThis.__selectionReentrantCount === 0) {
                  selection.setPosition(text, 2);
                  selection.setPosition(text, 0);
                }
                globalThis.__selectionReentrantCount += 1;
                globalThis.__selectionReentrantLog.push(
                  `event:${globalThis.__selectionReentrantCount}:${selection.anchorOffset}`
                );
              });
              selection.setPosition(text, 1);
              return `${globalThis.__selectionReentrantLog.join("|")}|count:${globalThis.__selectionReentrantCount}`;
            })()
            "#,
        )
        .expect("reentrant selectionchange setup should evaluate");

    assert_eq!(result, "|count:0");
    assert!(
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance first selectionchange task")
    );
    assert_eq!(
        vm.eval("`${globalThis.__selectionReentrantLog.join('|')}|count:${globalThis.__selectionReentrantCount}`")
            .expect("first reentrant selectionchange result should evaluate"),
        "event:1:0|count:1"
    );
    assert!(
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance reentrant selectionchange task")
    );
    assert_eq!(
        vm.eval("`${globalThis.__selectionReentrantLog.join('|')}|count:${globalThis.__selectionReentrantCount}`")
            .expect("second reentrant selectionchange result should evaluate"),
        "event:1:0|event:2:0|count:2"
    );
}

#[tokio::test]
async fn selectionchange_without_document_listener_does_not_schedule_host_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://selectionchange-no-listener.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              getSelection().setBaseAndExtent(text, 0, text, 2);
              return `${getSelection().anchorOffset}|${getSelection().focusOffset}`;
            })()
            "#,
        )
        .expect("selection mutation without listeners should evaluate");

    assert_eq!(result, "0|2");
    assert!(
        !vm.run_one_user_interaction_executor_turn(&loader)
            .await
            .expect("exact UserInteraction source should remain empty"),
        "a selection mutation without a Document listener must not enqueue a selectionchange task"
    );
}

#[test]
fn selection_range_methods_require_actual_range_objects() {
    let mut vm = new_storage_test_vm("https://selection-range-brand.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              const selection = getSelection();
              const range = document.createRange();
              range.setStart(text, 1);
              range.setEnd(text, 3);
              const equivalentRange = document.createRange();
              equivalentRange.setStart(text, 1);
              equivalentRange.setEnd(text, 3);
              const probe = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return error && error.name;
                }
              };
              const addSelection = probe(() => selection.addRange(selection));
              const addPlainObject = probe(() => selection.addRange({}));
              selection.addRange(range);
              const removeSelection = probe(() => selection.removeRange(selection));
              const removeEquivalent = probe(() => selection.removeRange(equivalentRange));
              const rangeStillSelected = selection.rangeCount;
              selection.removeRange(range);
              return [
                addSelection,
                addPlainObject,
                removeSelection,
                removeEquivalent,
                rangeStillSelected,
                selection.rangeCount,
                selection.anchorNode === null,
                selection.focusNode === null
              ].join("|");
            })()
            "#,
        )
        .expect("Selection Range argument brand checks should evaluate");

    assert_eq!(
        result,
        "TypeError|TypeError|TypeError|NotFoundError|1|0|true|true"
    );
}

#[test]
fn selection_empty_state_operations_throw_invalid_state_error() {
    let mut vm = new_storage_test_vm("https://selection-empty-state.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const selection = getSelection();
              selection.removeAllRanges();
              const div = document.createElement("div");
              (document.body || document.documentElement || document).appendChild(div);
              const probe = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return `${error && error.name}:${error && error.code}:${error instanceof DOMException}`;
                }
              };
              return [
                probe(() => selection.collapseToStart()),
                probe(() => selection.collapseToEnd()),
                probe(() => selection.extend(div))
              ].join("|");
            })()
            "#,
        )
        .expect("empty Selection operation checks should evaluate");

    assert_eq!(
        result,
        "InvalidStateError:11:true|InvalidStateError:11:true|InvalidStateError:11:true"
    );
}

#[test]
fn selection_add_range_ignores_detached_and_foreign_ranges() {
    let mut vm = new_storage_test_vm("https://selection-add-range-root.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const selection = getSelection();
              selection.removeAllRanges();

              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);

              const detachedText = document.createTextNode("detached");
              const detachedRange = document.createRange();
              detachedRange.setStart(detachedText, 1);
              detachedRange.setEnd(detachedText, 3);
              selection.addRange(detachedRange);
              const detachedState = [
                selection.rangeCount,
                selection.anchorNode === null,
                detachedRange.startContainer === detachedText,
                detachedRange.startOffset,
                detachedRange.endContainer === detachedText,
                detachedRange.endOffset
              ].join(":");

              const foreignDocument = document.implementation.createHTMLDocument("");
              const foreignText = foreignDocument.createTextNode("foreign");
              foreignDocument.body.appendChild(foreignText);
              const foreignRange = foreignDocument.createRange();
              foreignRange.setStart(foreignText, 1);
              foreignRange.setEnd(foreignText, 4);
              selection.addRange(foreignRange);
              const foreignState = [
                selection.rangeCount,
                selection.anchorNode === null,
                foreignRange.startContainer === foreignText,
                foreignRange.startOffset,
                foreignRange.endContainer === foreignText,
                foreignRange.endOffset
              ].join(":");

              const selectedRange = document.createRange();
              selectedRange.setStart(text, 1);
              selectedRange.setEnd(text, 3);
              selection.addRange(selectedRange);
              const selectedBefore = selection.getRangeAt(0);
              selection.addRange(detachedRange);
              const secondDetachedState = [
                selection.rangeCount,
                selection.getRangeAt(0) === selectedBefore,
                selection.anchorNode === text,
                selection.anchorOffset,
                selection.focusNode === text,
                selection.focusOffset
              ].join(":");

              const iframe = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(iframe);
              const childSelection = iframe.contentWindow.getSelection();
              const childOriginalSelection = childSelection;
              const childText = iframe.contentDocument.createTextNode("child");
              iframe.contentDocument.body.appendChild(childText);
              const childRange = iframe.contentDocument.createRange();
              childRange.selectNodeContents(iframe.contentDocument.body);
              childSelection.removeAllRanges();
              childSelection.addRange(childRange);
              const childSelectedRange = (() => {
                try {
                  return childSelection.getRangeAt(0);
                } catch (error) {
                  return error && error.name;
                }
              })();
              const childState = [
                iframe.contentWindow.getSelection() === childOriginalSelection,
                childSelection.rangeCount,
                childSelectedRange === childRange ? "same" : String(childSelectedRange),
                childSelection.anchorNode === iframe.contentDocument.body,
                childSelection.anchorOffset,
                childSelection.focusNode === iframe.contentDocument.body,
                childSelection.focusOffset
              ].join(":");

              return `${detachedState}|${foreignState}|${secondDetachedState}|${childState}`;
            })()
            "#,
        )
        .expect("Selection.addRange root checks should evaluate");

    assert_eq!(
        result,
        "0:true:true:1:true:3|0:true:true:1:true:4|1:true:true:1:true:3|true:1:same:true:0:true:1"
    );
}

#[test]
fn window_selection_rejects_child_document_shadow_ranges() {
    let mut vm = new_storage_test_vm("https://selection-child-shadow-range.test/");

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
            (function* () {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const childDocument = frame.contentWindow.document;
              const parentSelection = window.getSelection();
              parentSelection.removeAllRanges();

              const host = childDocument.createElement("div");
              childDocument.body.appendChild(host);
              const shadow = host.attachShadow({ mode: "open" });
              const span = childDocument.createElement("span");
              span.textContent = "Some text";
              shadow.appendChild(span);
              const shadowRange = childDocument.createRange();
              shadowRange.setStart(span.firstChild, 0);
              shadowRange.setEnd(span.firstChild, 3);
              parentSelection.addRange(shadowRange);
              const shadowState = [
                parentSelection.rangeCount,
                parentSelection.toString()
              ].join(":");

              const slottedHost = childDocument.createElement("div");
              childDocument.body.appendChild(slottedHost);
              const slottedSpan = childDocument.createElement("span");
              slottedSpan.textContent = "More text";
              slottedSpan.slot = "span";
              slottedHost.appendChild(slottedSpan);
              const slottedShadow = slottedHost.attachShadow({ mode: "open" });
              slottedShadow.innerHTML = '<slot name="span"></slot>';
              const slottedRange = childDocument.createRange();
              slottedRange.setStart(slottedSpan.firstChild, 0);
              slottedRange.setEnd(slottedSpan.firstChild, 4);
              parentSelection.addRange(slottedRange);
              const slottedState = [
                parentSelection.rangeCount,
                parentSelection.toString()
              ].join(":");

              yield; // Publish this scene before reading its geometry.
const childSelection = frame.contentWindow.getSelection();
              childSelection.removeAllRanges();
              childSelection.addRange(shadowRange);
              const childState = [
                childSelection.rangeCount,
                childSelection.toString()
              ].join(":");

              return `${shadowState}|${slottedState}|${childState}`;
            })()
            "#,
    )
    .expect("window Selection should reject child document shadow ranges");

    assert_eq!(result, "0:|0:|1:Som");
}

#[test]
fn selection_get_composed_ranges_rescopes_shadow_boundaries() {
    let mut vm = new_storage_html_test_vm("https://selection-composed-ranges.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const start = document.createElement("span");
              start.id = "start";
              start.textContent = "Start";
              const host = document.createElement("div");
              const end = document.createElement("span");
              end.id = "end";
              end.textContent = "End";
              const container = document.body || document.documentElement || document;
              container.appendChild(start);
              container.appendChild(host);
              container.appendChild(end);

              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = '<span id="inner1">Inner1</span><span id="inner2">Inner2</span>';
              const inner2 = root.getElementById("inner2");
              const selection = getSelection();
              selection.removeAllRanges();
              selection.setBaseAndExtent(start.firstChild, 3, inner2.firstChild, 3);

              const exposed = selection.getComposedRanges({ shadowRoots: [root] })[0];
              const rescoped = selection.getComposedRanges()[0];
              return [
                typeof selection.getComposedRanges,
                selection.rangeCount,
                selection.isCollapsed,
                exposed instanceof StaticRange,
                exposed.startContainer === start.firstChild,
                exposed.startOffset,
                exposed.endContainer === inner2.firstChild,
                exposed.endOffset,
                rescoped.startContainer === start.firstChild,
                rescoped.startOffset,
                rescoped.endContainer === host.parentNode,
                rescoped.endOffset,
                Array.isArray(selection.getComposedRanges())
              ].join("|");
            })()
            "#,
        )
        .expect("Selection.getComposedRanges shadow rescope checks should evaluate");

    assert_eq!(
        result,
        "function|1|true|true|true|3|true|3|true|3|true|2|true"
    );
}

#[test]
fn selection_get_composed_ranges_static_range_init_ignores_prototype_setters() {
    let mut vm = new_storage_html_test_vm("https://selection-composed-ranges-init.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const setterHits = [];
              for (const name of ["startContainer", "startOffset", "endContainer", "endOffset"]) {
                Object.defineProperty(Object.prototype, name, {
                  configurable: true,
                  get() { return undefined; },
                  set(value) {
                    const receiverKind = this instanceof StaticRange ? "range" : "plain";
                    setterHits.push(`${receiverKind}:${name}`);
                    Object.defineProperty(this, name, {
                      configurable: true,
                      enumerable: true,
                      writable: true,
                      value
                    });
                  }
                });
              }

              const start = document.createElement("span");
              start.textContent = "Start";
              const host = document.createElement("div");
              const container = document.body || document.documentElement || document;
              container.appendChild(start);
              container.appendChild(host);
              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = '<span id="inner">Inner</span>';
              const inner = root.getElementById("inner");
              const selection = getSelection();
              selection.removeAllRanges();
              selection.setBaseAndExtent(start.firstChild, 2, inner.firstChild, 4);

              const exposed = selection.getComposedRanges({ shadowRoots: [root] })[0];
              const rescoped = selection.getComposedRanges()[0];
              return JSON.stringify({
                exposed: [
                  exposed instanceof StaticRange,
                  exposed.startContainer === start.firstChild,
                  exposed.startOffset,
                  exposed.endContainer === inner.firstChild,
                  exposed.endOffset
                ],
                rescoped: [
                  rescoped instanceof StaticRange,
                  rescoped.startContainer === start.firstChild,
                  rescoped.startOffset,
                  rescoped.endContainer === host.parentNode,
                  rescoped.endOffset
                ],
                plainSetterHits: setterHits.filter(hit => hit.startsWith("plain:"))
              });
            })()
            "#,
        )
        .expect("Selection.getComposedRanges StaticRange init setter probe should evaluate");

    assert_eq!(
        result,
        r#"{"exposed":[true,true,2,true,4],"rescoped":[true,true,2,true,2],"plainSetterHits":[]}"#
    );
}

#[test]
fn selection_get_range_at_returns_shadow_collapsed_range_for_cross_root_selection() {
    let mut vm = new_storage_html_test_vm("https://selection-cross-root-range-at.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const start = document.createElement("span");
              start.textContent = "Start";
              const host = document.createElement("div");
              const container = document.body || document.documentElement || document;
              container.appendChild(start);
              container.appendChild(host);

              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = '<span id="inner1">Inner1</span><span id="inner2">Inner2</span>';
              const inner1 = root.getElementById("inner1");
              const inner2 = root.getElementById("inner2");
              const selection = getSelection();
              selection.removeAllRanges();
              selection.setBaseAndExtent(start.firstChild, 3, inner2.firstChild, 3);

              const composed = selection.getComposedRanges({ shadowRoots: [root] })[0];
              const range = selection.getRangeAt(0);
              return [
                selection.isCollapsed,
                selection.anchorNode === inner2.firstChild,
                selection.anchorOffset,
                composed.startContainer === start.firstChild,
                composed.startOffset,
                composed.endContainer === inner2.firstChild,
                composed.endOffset,
                range.collapsed,
                range.startContainer === inner2.firstChild,
                range.startOffset,
                range.endContainer === inner2.firstChild,
                range.endOffset,
                range.isPointInRange(inner1, 0),
                range.comparePoint(inner1, 0)
              ].join("|");
            })()
            "#,
        )
        .expect("cross-root Selection.getRangeAt probe should evaluate");

    assert_eq!(
        result,
        "true|true|3|true|3|true|3|true|true|3|true|3|false|-1"
    );
}

#[test]
fn selection_cross_root_set_base_and_extent_collapses_legacy_to_focus() {
    let mut vm = new_storage_test_vm("https://selection-cross-root-collapse.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const container = document.createElement("div");
              const host = document.createElement("div");
              const outText = document.createElement("p");
              outText.textContent = "Outside shadow tree.";
              const host2 = document.createElement("div");
              container.appendChild(host);
              container.appendChild(outText);
              container.appendChild(host2);
              (document.body || document.documentElement || document).appendChild(container);

              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = "<p>Inside shadow tree 1.</p>";
              const root2 = host2.attachShadow({ mode: "open" });
              root2.innerHTML = "<p>Inside shadow tree 2.</p>";
              const inText = root.querySelector("p").firstChild;
              const inText2 = root2.querySelector("p").firstChild;
              const out = outText.firstChild;
              const selection = getSelection();
              const roots = { shadowRoots: [root, root2] };
              const records = [];
              const label = node =>
                node === inText ? "in1" :
                node === inText2 ? "in2" :
                node === out ? "out" :
                node === null ? "null" : "other";
              const state = name => {
                const composed = selection.getComposedRanges(roots)[0];
                records.push([
                  name,
                  selection.isCollapsed,
                  label(selection.anchorNode),
                  selection.anchorOffset,
                  label(selection.focusNode),
                  selection.focusOffset,
                  composed.collapsed,
                  label(composed.startContainer),
                  composed.startOffset,
                  label(composed.endContainer),
                  composed.endOffset
                ].join(":"));
              };

              selection.setBaseAndExtent(inText, 0, out, 1);
              state("shadow-to-light");
              selection.setBaseAndExtent(inText, 0, inText2, 1);
              state("shadow-to-shadow");
              selection.setBaseAndExtent(out, 1, inText, 0);
              state("light-to-shadow-backward");
              return records.join("|");
            })()
            "#,
        )
        .expect("cross-root Selection.setBaseAndExtent collapse probe should evaluate");

    assert_eq!(
        result,
        "shadow-to-light:true:out:1:out:1:false:in1:0:out:1|shadow-to-shadow:true:in2:1:in2:1:false:in1:0:in2:1|light-to-shadow-backward:true:out:1:out:1:false:in1:0:out:1"
    );
}

#[test]
fn selection_set_base_and_extent_to_earlier_shadow_root_collapses_to_anchor() {
    let mut vm = new_storage_html_test_vm("https://selection-cross-root-backward-collapse.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const anchor = document.createElement("div");
              const parent = document.body || document.documentElement || document;
              parent.appendChild(host);
              parent.appendChild(anchor);
              const root = host.attachShadow({ mode: "open" });
              root.textContent = "A";

              const selection = getSelection();
              selection.setBaseAndExtent(anchor, 0, root, 0);

              return [
                selection.anchorNode === anchor,
                selection.anchorOffset,
                selection.focusNode === anchor,
                selection.focusOffset,
                selection.isCollapsed,
                selection.getRangeAt(0).startContainer === anchor,
                selection.getRangeAt(0).startOffset,
                selection.getRangeAt(0).endContainer === anchor,
                selection.getRangeAt(0).endOffset
              ].join(":");
            })()
            "#,
        )
        .expect("cross-root backward Selection.setBaseAndExtent probe should evaluate");

    assert_eq!(result, "true:0:true:0:true:true:0:true:0");
}

#[test]
fn selection_set_base_and_extent_orders_descendant_boundary_before_ancestor_after_child() {
    let mut vm = new_storage_test_vm("https://selection-ancestor-boundary-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const parent = document.createElement("div");
              parent.innerHTML = "<span><b></b></span>";
              (document.body || document.documentElement || document).appendChild(parent);
              const span = parent.firstChild;
              const child = span.firstChild;
              const selection = getSelection();
              const record = label => {
                const range = selection.getRangeAt(0);
                return [
                  label,
                  range.startContainer === child,
                  range.startOffset,
                  range.endContainer === span,
                  range.endOffset,
                ].join(":");
              };
              selection.setBaseAndExtent(child, 0, span, 1);
              const forward = record("forward");
              selection.setBaseAndExtent(span, 1, child, 0);
              const backward = record("backward");
              return `${forward}|${backward}`;
            })()
            "#,
        )
        .expect("Selection.setBaseAndExtent descendant/ancestor order probe should evaluate");

    assert_eq!(result, "forward:true:0:true:1|backward:true:0:true:1");
}

#[test]
fn selection_get_composed_ranges_orders_slotted_boundary_like_chromium() {
    let mut vm = new_storage_test_vm("https://selection-composed-ranges-slot.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const container = document.createElement("div");
              const host = document.createElement("div");
              host.textContent = "Second";
              container.appendChild(host);
              (document.body || document.documentElement || document).appendChild(container);

              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = 'First <slot></slot> Third';
              const second = host.firstChild;
              const third = root.querySelector("slot").nextSibling;
              const selection = getSelection();

              selection.removeAllRanges();
              selection.setBaseAndExtent(second, 3, third, 4);
              const rescoped = selection.getComposedRanges()[0];
              const exposed = selection.getComposedRanges({ shadowRoots: [root] })[0];

              selection.setBaseAndExtent(third, 4, second, 3);
              const reversed = selection.getComposedRanges({ shadowRoots: [root] })[0];

              return [
                selection.isCollapsed,
                selection.anchorNode === second,
                selection.anchorOffset,
                selection.focusNode === second,
                selection.focusOffset,
                rescoped.startContainer === container,
                rescoped.startOffset,
                rescoped.endContainer === second,
                rescoped.endOffset,
                exposed.startContainer === third,
                exposed.startOffset,
                exposed.endContainer === second,
                exposed.endOffset,
                reversed.startContainer === third,
                reversed.startOffset,
                reversed.endContainer === second,
                reversed.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("Selection.getComposedRanges slot ordering checks should evaluate");

    assert_eq!(
        result,
        "true|true|3|true|3|true|0|true|3|true|4|true|3|true|4|true|3"
    );
}

#[test]
fn selection_get_composed_ranges_tracks_associated_range_updates() {
    let mut vm = new_storage_html_test_vm("https://selection-composed-range-update.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const container = document.body || document.documentElement || document;
              const light = document.createElement("div");
              light.textContent = "Start outside shadow DOM";
              const outerHost = document.createElement("div");
              outerHost.textContent = "outerHost";
              const lightEnd = document.createElement("div");
              lightEnd.textContent = "End outside shadow DOM";
              container.appendChild(light);
              container.appendChild(outerHost);
              container.appendChild(lightEnd);

              const outerRoot = outerHost.attachShadow({ mode: "open" });
              outerRoot.appendChild(document.createElement("slot"));
              const innerHost = document.createElement("div");
              innerHost.textContent = "innerHost";
              outerRoot.appendChild(innerHost);
              const innerRoot = innerHost.attachShadow({ mode: "open" });
              innerRoot.appendChild(document.createElement("slot"));

              const selection = getSelection();
              const lightText = light.firstChild;
              const lightEndText = lightEnd.firstChild;
              const innerText = innerHost.firstChild;
              const roots = { shadowRoots: [outerRoot, innerRoot] };
              const records = [];
              const thrown = (fn) => {
                try { fn(); return false; } catch (_) { return true; }
              };
              const state = (label, liveRange) => {
                const ranges = selection.getComposedRanges(roots);
                const composed = ranges[0];
                records.push([
                  label,
                  liveRange.collapsed,
                  liveRange.startContainer === innerText ? "inner" :
                    liveRange.startContainer === lightText ? "light" :
                    liveRange.startContainer === lightEndText ? "end" :
                    liveRange.startContainer === outerRoot ? "outerRoot" : "other",
                  liveRange.startOffset,
                  selection.isCollapsed,
                  selection.anchorNode === innerText ? "inner" :
                    selection.anchorNode === lightText ? "light" :
                    selection.anchorNode === lightEndText ? "end" :
                    selection.anchorNode === outerRoot ? "outerRoot" :
                    selection.anchorNode === null ? "null" : "other",
                  selection.anchorOffset,
                  ranges.length,
                  composed ? (
                    (composed.startContainer === innerText ? "inner" :
                     composed.startContainer === lightText ? "light" :
                     composed.startContainer === lightEndText ? "end" :
                     composed.startContainer === outerRoot ? "outerRoot" :
                     composed.startContainer === document ? "document" : "other") +
                    ":" + composed.startOffset + ">" +
                    (composed.endContainer === innerText ? "inner" :
                     composed.endContainer === lightText ? "light" :
                     composed.endContainer === lightEndText ? "end" :
                     composed.endContainer === outerRoot ? "outerRoot" :
                     composed.endContainer === document ? "document" : "other") +
                    ":" + composed.endOffset
                  ) : "none"
                ].join(":"));
              };

              selection.setBaseAndExtent(lightText, 10, innerText, 5);
              records.push("cross-getRangeAt:" + (() => {
                const range = selection.getRangeAt(0);
                return [
                  range.collapsed,
                  range.startContainer === innerText ? "inner" :
                    range.startContainer === lightText ? "light" :
                    range.startContainer === lightEndText ? "end" :
                    range.startContainer === outerRoot ? "outerRoot" : "other",
                  range.startOffset,
                  range.endContainer === innerText ? "inner" :
                    range.endContainer === lightText ? "light" :
                    range.endContainer === lightEndText ? "end" :
                    range.endContainer === outerRoot ? "outerRoot" : "other",
                  range.endOffset
                ].join(":");
              })());

              selection.setBaseAndExtent(lightText, 10, lightText, 20);
              let liveRange = selection.getRangeAt(0);
              liveRange.setEnd(innerText, 5);
              state("setEnd-cross-keeps-composed-start", liveRange);

              selection.setBaseAndExtent(lightEndText, 10, lightEndText, 20);
              liveRange = selection.getRangeAt(0);
              liveRange.setStart(innerText, 5);
              state("setStart-cross-keeps-composed-end", liveRange);

              selection.setBaseAndExtent(lightText, 10, lightText, 20);
              liveRange = selection.getRangeAt(0);
              liveRange.setStart(innerText, 5);
              state("setStart-cross-collapses-composed", liveRange);

              selection.setBaseAndExtent(lightText, 10, lightEndText, 20);
              liveRange = selection.getRangeAt(0);
              liveRange.selectNode(innerHost);
              state("selectNode-syncs-all", liveRange);

              selection.setBaseAndExtent(lightText, 10, lightEndText, 20);
              liveRange = selection.getRangeAt(0);
              liveRange.collapse();
              state("collapse-syncs-all", liveRange);

              selection.removeAllRanges();
              liveRange = document.createRange();
              selection.addRange(liveRange);
              liveRange.setEnd(innerText, 5);
              state("addRange-before-setEnd", liveRange);
              liveRange.setStart(lightText, 10);
              state("setStart-after-addRange-setEnd", liveRange);

              selection.setBaseAndExtent(lightText, 10, lightEndText, 20);
              liveRange = selection.getRangeAt(0);
              const detached = document.createElement("span");
              liveRange.setStart(detached, 0);
              records.push([
                "detached-clears-selection",
                liveRange.collapsed,
                selection.rangeCount,
                selection.anchorNode === null,
                selection.getComposedRanges(roots).length,
                thrown(() => selection.getRangeAt(0))
              ].join(":"));

              return records.join("|");
            })()
            "#,
        )
        .expect("Selection.getComposedRanges associated Range updates should evaluate");

    assert_eq!(
        result,
        "cross-getRangeAt:true:inner:5:inner:5|setEnd-cross-keeps-composed-start:true:inner:5:true:inner:5:1:light:10>inner:5|setStart-cross-keeps-composed-end:true:inner:5:true:inner:5:1:inner:5>end:20|setStart-cross-collapses-composed:true:inner:5:true:inner:5:1:inner:5>inner:5|selectNode-syncs-all:false:outerRoot:1:false:outerRoot:1:1:outerRoot:1>outerRoot:2|collapse-syncs-all:true:end:20:true:end:20:1:end:20>end:20|addRange-before-setEnd:true:inner:5:true:inner:5:1:document:0>inner:5|setStart-after-addRange-setEnd:true:light:10:true:light:10:1:light:10>inner:5|detached-clears-selection:true:0:true:0:true"
    );
}

#[test]
fn selection_get_composed_ranges_rescopes_after_dom_removals() {
    let mut vm = new_storage_test_vm("https://selection-composed-dom-removal.test/");

    let result = vm
        .eval(
            r##"
            (() => {
              const sel = getSelection();
              const container = document.createElement("div");
              (document.body || document.documentElement || document).appendChild(container);
              const failures = [];
              const expectBoundary = (name, range, side, node, offset) => {
                const actualNode = range[`${side}Container`];
                const actualOffset = range[`${side}Offset`];
                if (actualNode !== node || actualOffset !== offset) {
                  failures.push(`${name}:${side}:${actualNode?.nodeName}:${actualOffset}`);
                }
              };
              const composed = (...roots) =>
                sel.getComposedRanges({ shadowRoots: roots })[0];
              const reset = (html) => {
                sel.removeAllRanges();
                container.innerHTML = html;
              };

              for (const mode of ["open", "closed"]) {
                reset('a<div id="host"></div>b');
                let host = container.querySelector("#host");
                let root = host.attachShadow({ mode });
                root.innerHTML = "hello, world";
                sel.setBaseAndExtent(root.firstChild, 7, container, 2);
                host.remove();
                let range = composed(root);
                expectBoundary(`${mode}:host-remove`, range, "start", container, 1);
                expectBoundary(`${mode}:host-remove`, range, "end", container, 1);

                reset('<div id="wrapper">a<div id="host"></div>b</div>');
                const wrapper = container.querySelector("#wrapper");
                host = container.querySelector("#host");
                root = host.attachShadow({ mode });
                root.innerHTML = "hello, world";
                sel.setBaseAndExtent(root.firstChild, 4, root.firstChild, 7);
                wrapper.remove();
                range = composed(root);
                expectBoundary(`${mode}:wrapper-remove`, range, "start", container, 0);
                expectBoundary(`${mode}:wrapper-remove`, range, "end", container, 0);

                reset('<div id="hello">Hello,</div><div id="world"> World</div>');
                const hello = container.querySelector("#hello");
                const world = container.querySelector("#world");
                sel.setBaseAndExtent(hello.firstChild, 1, world.firstChild, 3);
                hello.firstChild.remove();
                range = sel.getComposedRanges()[0];
                expectBoundary(`${mode}:light-text-remove`, range, "start", hello, 0);
                expectBoundary(`${mode}:light-text-remove`, range, "end", world.firstChild, 3);

                reset('a<div id="host"></div>b');
                host = container.querySelector("#host");
                root = host.attachShadow({ mode });
                root.innerHTML = "hello, world";
                sel.setBaseAndExtent(root.firstChild, 7, container, 2);
                root.innerHTML = "";
                range = composed(root);
                expectBoundary(`${mode}:shadow-content-clear`, range, "start", root, 0);
                expectBoundary(`${mode}:shadow-content-clear`, range, "end", container, 2);

                reset('a<div id="outerhost"></div>b');
                const outerHost = container.querySelector("#outerhost");
                const outerRoot = outerHost.attachShadow({ mode });
                outerRoot.innerHTML = 'c<div id="innerHost"></div>d';
                const innerHost = outerRoot.querySelector("#innerHost");
                const innerRoot = innerHost.attachShadow({ mode });
                innerRoot.innerHTML = "hello, world";
                sel.setBaseAndExtent(container.firstChild, 0, innerRoot.firstChild, 4);
                outerHost.remove();
                range = composed(innerRoot, outerRoot);
                expectBoundary(`${mode}:outer-host-remove`, range, "start", container.firstChild, 0);
                expectBoundary(`${mode}:outer-host-remove`, range, "end", container, 1);
              }

              reset([
                '<div id=host>',
                '<div id=div1 slot=slot2>slotted content 1</div>',
                '<div id=div2 slot=slot1>slotted content 2</div>',
                '</div>'
              ].join(""));
              const host = container.querySelector("#host");
              const div1 = container.querySelector("#div1");
              const div2 = container.querySelector("#div2");
              const shadowRoot = host.attachShadow({ mode: "open" });
              shadowRoot.innerHTML = [
                '<span>before</span>',
                '<slot name=slot1></slot>',
                '<span>between</span>',
                '<slot name=slot2></slot>',
                '<span>after</span>',
              ].join("");
              sel.setBaseAndExtent(div1.firstChild, 2, div2.firstChild, 2);
              div1.remove();
              const range = composed(shadowRoot);
              expectBoundary("slot-start-remove", range, "start", host, 0);
              expectBoundary("slot-start-remove", range, "end", div2.firstChild, 2);

              return failures.join("|") || "ok";
            })()
            "##,
        )
        .expect("Selection.getComposedRanges removal rescope checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn document_get_selection_uses_associated_window_selection() {
    let mut vm = new_storage_test_vm("https://document-get-selection.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const topWindowSelection = window.getSelection();
              const topDocumentSelection = document.getSelection();
              const internalSlotName = "__moliWindowSelection";
              const topWindowInternalBefore = Object.getOwnPropertyNames(window)
                .includes(internalSlotName);
              window[internalSlotName] = "top-spoof";

              const htmlDocument = document.implementation.createHTMLDocument("");
              const xmlDocument = document.implementation.createDocument(null, "", null);

              const iframe = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(iframe);
              const childWindowSelection = iframe.contentWindow.getSelection();
              const childDocumentSelection = iframe.contentDocument.getSelection();
              const childWindowInternalBefore = Object.getOwnPropertyNames(iframe.contentWindow)
                .includes(internalSlotName);
              iframe.contentWindow[internalSlotName] = "child-spoof";

              return [
                topDocumentSelection === topWindowSelection,
                topWindowSelection === window.getSelection(),
                window[internalSlotName] === "top-spoof",
                topWindowInternalBefore,
                topDocumentSelection instanceof Selection,
                Object.prototype.toString.call(topDocumentSelection),
                Object.keys(topDocumentSelection).join(","),
                Object.getOwnPropertyNames(topDocumentSelection)
                  .filter((name) => name.startsWith("__moli")).length,
                htmlDocument.defaultView === null,
                htmlDocument.getSelection() === null,
                "getSelection" in xmlDocument,
                xmlDocument.defaultView === null,
                xmlDocument.getSelection() === null,
                childWindowSelection === iframe.contentWindow.getSelection(),
                childDocumentSelection === childWindowSelection,
                iframe.contentWindow[internalSlotName] === "child-spoof",
                childWindowInternalBefore,
                childWindowSelection !== topWindowSelection,
                childWindowSelection instanceof iframe.contentWindow.Selection,
                Object.getOwnPropertyNames(childWindowSelection)
                  .filter((name) => name.startsWith("__moli")).length
              ].join("|");
            })()
            "#,
        )
        .expect("Document.getSelection checks should evaluate");

    assert_eq!(
        result,
        "true|true|true|false|true|[object Selection]||0|true|true|true|true|true|true|true|true|false|true|true|0"
    );
}

#[test]
fn modal_dialog_selection_inertness_is_scoped_to_its_document() {
    let mut vm = new_storage_test_vm("https://selection-modal-document-scope.test/");

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
            (function* () {
              const html = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body || html.appendChild(document.createElement("body"));
              body.textContent = "";
              const parentText = document.createElement("p");
              parentText.textContent = "parent outside";
              const parentDialog = document.createElement("dialog");
              parentDialog.textContent = "parent dialog";
              const frame = document.createElement("iframe");
              body.append(parentText, parentDialog, frame);

              const childDocument = frame.contentDocument;
              const childHtml = childDocument.documentElement ||
                childDocument.appendChild(childDocument.createElement("html"));
              const childBody = childDocument.body ||
                childHtml.appendChild(childDocument.createElement("body"));
              childBody.textContent = "";
              const childText = childDocument.createElement("p");
              childText.textContent = "child outside";
              const childDialog = childDocument.createElement("dialog");
              childDialog.textContent = "child dialog";
              childBody.append(childText, childDialog);

              const selectedText = (selection, root) => {
                selection.removeAllRanges();
                selection.selectAllChildren(root);
                return selection.toString();
              };
              const parentSelection = getSelection();
              const childSelection = frame.contentWindow.getSelection();

              parentDialog.showModal();
              yield; // Publish this scene before reading its geometry.
const parentWithParentModal = selectedText(parentSelection, body);
              const childWithParentModal = selectedText(childSelection, childBody);
              childSelection.removeAllRanges();
              const childCommandWithParentModal = childDocument.execCommand("selectAll");
              const childCommandTextWithParentModal = childSelection.toString();
              parentDialog.close();

              childDialog.showModal();
              yield; // Publish this scene before reading its geometry.
const parentWithChildModal = selectedText(parentSelection, body);
              const childWithChildModal = selectedText(childSelection, childBody);
              parentSelection.removeAllRanges();
              const parentCommandWithChildModal = document.execCommand("selectAll");
              const parentCommandTextWithChildModal = parentSelection.toString();

              return JSON.stringify({
                parentWithParentModal,
                childWithParentModal,
                childCommandWithParentModal,
                childCommandTextWithParentModal,
                parentWithChildModal,
                childWithChildModal,
                parentCommandWithChildModal,
                parentCommandTextWithChildModal
              });
            })()
            "#,
    )
    .expect("modal selection inertness should remain document-scoped");

    assert_eq!(
        result,
        r#"{"parentWithParentModal":"parent dialog","childWithParentModal":"child outside","childCommandWithParentModal":true,"childCommandTextWithParentModal":"child outside","parentWithChildModal":"parent outside","childWithChildModal":"child dialog","parentCommandWithChildModal":true,"parentCommandTextWithChildModal":"parent outside"}"#
    );
}

#[test]
fn report_error_uses_the_receiver_realm_without_observing_exception_getters() {
    let mut vm = new_storage_test_vm("https://report-error-receiver-realm.test/");

    vm.eval(
        r#"
        (() => {
          const frame = document.createElement("iframe");
          (document.body || document.documentElement || document).appendChild(frame);
          globalThis.__reportErrorRealmFrame = frame;
          return "ready";
        })()
        "#,
    )
    .expect("reportError receiver Realm setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const child = globalThis.__reportErrorRealmFrame.contentWindow;
              const childEvents = [];
              let topEvents = 0;
              let getterCalls = 0;
              window.addEventListener("error", () => topEvents++);
              child.addEventListener("error", event => childEvents.push(event));

              const error = new TypeError("receiver failure");
              window.reportError.call(child, error);

              const opaqueReason = {};
              for (const name of ["name", "message", "fileName", "lineNumber", "columnNumber"]) {
                Object.defineProperty(opaqueReason, name, {
                  get() {
                    getterCalls++;
                    throw new Error(`unexpected ${name} getter`);
                  }
                });
              }
              window.reportError.call(child, opaqueReason);

              let missingArgumentError = null;
              try {
                window.reportError();
              } catch (exception) {
                missingArgumentError = exception.name;
              }

              globalThis.__reportErrorRealmFrame.remove();
              child.reportError("detached failure");
              const detachedCallCompleted = true;

              return JSON.stringify({
                topEvents,
                eventCount: childEvents.length,
                exactErrors: [childEvents[0].error === error, childEvents[1].error === opaqueReason],
                receiverRealm: childEvents.every(event => event instanceof child.ErrorEvent),
                messagesAreNonEmpty: childEvents.every(event => event.message.length > 0),
                getterCalls,
                missingArgumentError,
                detachedCallCompleted
              });
            })()
            "#,
        )
        .expect("cross-Realm reportError probe should evaluate");

    assert_eq!(
        result,
        r#"{"topEvents":0,"eventCount":2,"exactErrors":[true,true],"receiverRealm":true,"messagesAreNonEmpty":true,"getterCalls":0,"missingArgumentError":"TypeError","detachedCallCompleted":true}"#,
    );
}

#[test]
fn selection_range_membership_uses_native_document_relationships() {
    let mut vm = new_storage_test_vm("https://selection-native-membership.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../tests/fixtures/selection-native-membership.js"
        ))
        .expect("selection membership should ignore author relationship properties"),
        ""
    );
}

#[test]
fn document_domain_setter_respects_the_original_hosts_public_suffix() {
    for (host, domain, allowed) in [
        ("www1.localhost", "localhost", false),
        ("localhost", "localhost", true),
        (
            "www1.web-platform.localhost",
            "web-platform.localhost",
            true,
        ),
        ("www.example.compute.amazonaws.com", "amazonaws.com", false),
        (
            "www.example.compute.amazonaws.com",
            "example.compute.amazonaws.com",
            false,
        ),
        ("test.amazonaws.com", "amazonaws.com", true),
        ("www.city.kawasaki.jp", "city.kawasaki.jp", true),
    ] {
        let mut vm = new_storage_test_vm(&format!("https://{host}/path"));
        let result = vm
            .eval(&format!(
                r#"((value) => {{
                const before = document.domain;
                let outcome;
                try {{ document.domain = value; outcome = "accepted"; }}
                catch (error) {{
                    outcome = `${{error.name}}:${{error instanceof DOMException}}:${{error.code}}`;
                }}
                return JSON.stringify([before, outcome, document.domain]);
            }})({})"#,
                serde_json::to_string(domain).unwrap(),
            ))
            .expect("document.domain boundary probe should evaluate");
        assert_eq!(
            result,
            serde_json::json!([
                host,
                if allowed {
                    "accepted"
                } else {
                    "SecurityError:true:18"
                },
                if allowed { domain } else { host },
            ])
            .to_string(),
            "{host} -> {domain}"
        );
    }
}
