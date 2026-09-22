use super::*;

fn new_broadcast_channel_test_vm(url: &str) -> crate::runtime::PageVmTaskExecutorTestHarness {
    new_broadcast_channel_page_test_vm(url)
}

async fn apply_page_broadcast_channel_deliveries(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("BroadcastChannel test loader");
    page.apply_pending_broadcast_channel_delivery_tasks(&loader, 64)
        .await
        .expect("production BroadcastChannel executor tasks should apply");
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_postmessage_delivers_structured_data() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast");

    let result = vm
        .eval(
            r#"
            (() => {
              const sender = new BroadcastChannel("plain");
              const receiver = new BroadcastChannel("plain");
              globalThis.__broadcastEvents = [];
              sender.onmessage = () => __broadcastEvents.push("sender");
              receiver.onmessage = event => {
                __broadcastEvents.push([
                  event.data.marker,
                  event.data.nested.text,
                  event.data.list.join(","),
                  event.origin,
                  event.ports.length,
                  receiver.name
                ].join("|"));
              };
              sender.postMessage({
                marker: 17,
                nested: { text: "ok" },
                list: [1, 2]
              });
              return __broadcastEvents.length;
            })()
            "#,
        )
        .expect("BroadcastChannel setup should evaluate");

    assert_eq!(result, "0");
    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__broadcastEvents.join(';')")
            .expect("BroadcastChannel result should evaluate"),
        "17|ok|1,2|https://example.com|0|plain"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn detached_child_broadcast_channel_disconnects_child_owned_channels() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-detached-child");

    let setup = vm
        .eval(
            r#"
            (() => {
              globalThis.__detachedChildBroadcastChannelParentEvents = [];
              globalThis.__detachedChildBroadcastChannelChildEvents = [];
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const ChildBroadcastChannel = frame.contentWindow.BroadcastChannel;
              const parentReceiver = new BroadcastChannel("detached-child-channel");
              parentReceiver.onmessage = event => {
                __detachedChildBroadcastChannelParentEvents.push(event.data);
              };
              const childReceiver = new ChildBroadcastChannel("detached-child-channel");
              childReceiver.onmessage = event => {
                __detachedChildBroadcastChannelChildEvents.push(event.data);
              };
              const childSender = new ChildBroadcastChannel("detached-child-channel");
              frame.remove();
              childSender.postMessage("old-child-source");
              const detachedChildSender = new ChildBroadcastChannel("detached-child-channel");
              detachedChildSender.close();
              detachedChildSender.close();
              let detachedPost;
              try {
                detachedPost = String(detachedChildSender.postMessage("new-detached-source"));
              } catch (error) {
                detachedPost = error.name;
              }
              const topSender = new BroadcastChannel("detached-child-channel");
              topSender.postMessage("top-source");
              return [
                typeof ChildBroadcastChannel,
                detachedChildSender.name,
                detachedPost
              ].join("|");
            })()
            "#,
        )
        .expect("detached child BroadcastChannel setup should evaluate");

    assert_eq!(setup, "function|detached-child-channel|InvalidStateError");
    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
              parent: globalThis.__detachedChildBroadcastChannelParentEvents,
              child: globalThis.__detachedChildBroadcastChannelChildEvents
            })"#,
        )
        .expect("detached child BroadcastChannel events should evaluate"),
        r#"{"parent":["top-source"],"child":[]}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn main_document_open_preserves_broadcast_channel_execution_context() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-main-replacement");

    vm.eval(
        r#"
        (() => {
          globalThis.__mainReplacementBroadcastEvents = [];
          globalThis.__mainReplacementReceiver =
            new BroadcastChannel("main-replacement-owner");
          globalThis.__mainReplacementReceiver.onmessage = event => {
            globalThis.__mainReplacementBroadcastEvents.push(event.data);
          };
        })()
        "#,
    )
    .expect("main replacement BroadcastChannel setup should evaluate");
    let retired_owner = vm
        .current_main_document_task_owner()
        .expect("main BroadcastChannel owner should exist");

    vm.eval(
        r#"
        document.open();
        document.close();
        new BroadcastChannel("main-replacement-owner")
          .postMessage("replacement-message");
        "replaced";
        "#,
    )
    .expect("document.open should replace the main document and preserve the same-realm channel");
    let current_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main BroadcastChannel owner should exist");
    assert_ne!(current_owner, retired_owner);
    assert_eq!(
        current_owner.local_window_id, retired_owner.local_window_id,
        "document.open must preserve the BroadcastChannel-owning LocalWindow"
    );

    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__mainReplacementBroadcastEvents.join('|')")
            .expect("replacement main BroadcastChannel event should evaluate"),
        "replacement-message",
        "same-execution-context document.open must preserve the existing channel without owner rebind"
    );
}

#[tokio::test]
async fn child_document_replacement_closes_old_broadcast_channels() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-child-replacement");

    vm.eval(
        r#"
        (() => {
          globalThis.__childReplacementBroadcastEvents = [];
          const root = document.documentElement || document.appendChild(document.createElement("html"));
          const body = document.body || root.appendChild(document.createElement("body"));
          const frame = document.createElement("iframe");
          globalThis.__childReplacementBroadcastFrame = frame;
          body.appendChild(frame);
        })()
        "#,
    )
    .expect("child replacement BroadcastChannel frame should evaluate");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        None,
        "child BroadcastChannel initial about:blank should complete synchronously without Page-owned child work"
    );
    let initial_owner = current_single_child_document_owner_for_test(
        &vm,
        "child BroadcastChannel initial-empty document",
    );
    vm.eval("__childReplacementBroadcastFrame.srcdoc = '<p>committed child</p>'; 'queued'")
        .expect("first child BroadcastChannel document should queue");
    for expected in [
        ChildFrameSemanticTurnKind::NavigationCommit,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        ChildFrameSemanticTurnKind::HostLoad,
    ] {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn().await,
            Some(expected),
            "first child BroadcastChannel document should advance {expected:?} through the real Page-owned realm prerequisite"
        );
    }
    let committed_owner = current_single_child_document_owner_for_test(
        &vm,
        "committed child BroadcastChannel document",
    );
    assert_eq!(
        committed_owner.local_window_id, initial_owner.local_window_id,
        "the first secure commit must reuse the initial-empty LocalWindow"
    );
    assert_ne!(committed_owner.document_id, initial_owner.document_id);

    vm.eval(
        r#"
        (() => {
          const frame = globalThis.__childReplacementBroadcastFrame;
          const OldChildBroadcastChannel = frame.contentWindow.BroadcastChannel;
          globalThis.__oldChildBroadcastChannelConstructor = OldChildBroadcastChannel;
          globalThis.__oldChildBroadcastReceiver =
            new OldChildBroadcastChannel("child-replacement-owner");
          __oldChildBroadcastReceiver.onmessage = event => {
            globalThis.__childReplacementBroadcastEvents.push("old:" + event.data);
          };
          frame.srcdoc = "<p>replacement child</p>";
        })()
        "#,
    )
    .expect("child replacement BroadcastChannel setup should evaluate");
    let child_handle = vm
        ._context_host
        .borrow()
        .top_level_child_browsing_context_handles_in_document_order()
        .into_iter()
        .next()
        .expect("child BroadcastChannel frame should exist");
    let retired_local_window_id = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("child BroadcastChannel owner should exist")
        .local_window_id;

    assert_eq!(
        vm.run_next_child_frame_semantic_turn().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "child srcdoc replacement should rotate its document owner"
    );
    let current_local_window_id = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("replacement child BroadcastChannel owner should exist")
        .local_window_id;
    assert_ne!(current_local_window_id, retired_local_window_id);

    vm.eval(
        r#"
        (() => {
          globalThis.__retiredConstructorBroadcastReceiver =
            new __oldChildBroadcastChannelConstructor("child-replacement-owner");
          __retiredConstructorBroadcastReceiver.onmessage = event => {
            globalThis.__childReplacementBroadcastEvents.push("retired-constructor:" + event.data);
          };
          const NewChildBroadcastChannel =
            __childReplacementBroadcastFrame.contentWindow.BroadcastChannel;
          globalThis.__newChildBroadcastReceiver =
            new NewChildBroadcastChannel("child-replacement-owner");
          __newChildBroadcastReceiver.onmessage = event => {
            globalThis.__childReplacementBroadcastEvents.push("new:" + event.data);
          };
          const sender = new BroadcastChannel("child-replacement-owner");
          sender.postMessage("after-navigation");
        })()
        "#,
    )
    .expect("replacement child BroadcastChannel should evaluate");
    // The exposed replacement Window can accept a channel before its realm
    // turn. Advance earlier child work through the Page executor: lifecycle
    // tasks share the DOM-manipulation FIFO with BroadcastChannel deliveries.
    for _ in 0..5 {
        let Some(turn) = vm.run_next_child_frame_semantic_turn().await else {
            break;
        };
        assert!(
            matches!(
                turn,
                ChildFrameSemanticTurnKind::RealmMaterialization
                    | ChildFrameSemanticTurnKind::DocumentLifecycle
                    | ChildFrameSemanticTurnKind::HostLoad
            ),
            "replacement child should only have realm and lifecycle work before delivery: {turn:?}"
        );
    }
    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__childReplacementBroadcastEvents.join('|')")
            .expect("replacement child BroadcastChannel event should evaluate"),
        "new:after-navigation",
        "old child channel must not be rebound through the stable iframe handle"
    );
}

#[test]
fn broadcast_channel_declared_methods_preserve_descriptors_and_close_behavior() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-declared-methods");

    let result = vm
        .eval(
            r#"
            (() => {
              const describe = name => {
                const descriptor = Object.getOwnPropertyDescriptor(BroadcastChannel.prototype, name);
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
              const describeAccessor = name => {
                const descriptor = Object.getOwnPropertyDescriptor(BroadcastChannel.prototype, name);
                return [
                  name,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.set?.name,
                  descriptor?.set?.length,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              };
              const channel = new BroadcastChannel("declared-methods");
              channel.onmessage = function declaredHandler() {};
              const postResult = channel.postMessage("ignored");
              const closeResult = channel.close();
              let closedPost;
              try {
                channel.postMessage("after-close");
                closedPost = "ok";
              } catch (error) {
                closedPost = error && error.name;
              }
              return JSON.stringify({
                descriptors: [describe("postMessage"), describe("close")],
                accessors: [
                  describeAccessor("name"),
                  describeAccessor("onmessage"),
                  describeAccessor("onmessageerror")
                ],
                nameValue: channel.name,
                handlerType: typeof channel.onmessage,
                messageErrorValue: channel.onmessageerror,
                postUndefined: postResult === undefined,
                closeUndefined: closeResult === undefined,
                closedPost
              });
            })()
            "#,
        )
        .expect("BroadcastChannel declared method probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["postMessage:function:postMessage:1:true:true:true","close:function:close:0:true:true:true"],"accessors":["name:function:get name:0:undefined:::true:true","onmessage:function:get onmessage:0:function:set onmessage:1:true:true","onmessageerror:function:get onmessageerror:0:function:set onmessageerror:1:true:true"],"nameValue":"declared-methods","handlerType":"function","messageErrorValue":null,"postUndefined":true,"closeUndefined":true,"closedPost":"InvalidStateError"}"#
    );
}

#[test]
fn broadcast_channel_constructor_requires_new() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-constructor-new");

    let result = vm
        .eval(
            r#"
            (() => {
              try {
                BroadcastChannel("");
                return "ok";
              } catch (error) {
                return `${error.name}:${error instanceof TypeError}`;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel constructor new probe should evaluate");

    assert_eq!(result, "TypeError:true");
}
#[test]
fn broadcast_channel_constructor_requires_name() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-name-required");

    let result = vm
        .eval(
            r#"
            (() => {
              try {
                new BroadcastChannel();
                return "ok";
              } catch (error) {
                return `${error.name}:${error instanceof TypeError}`;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel missing name probe should evaluate");

    assert_eq!(result, "TypeError:true");
}
#[test]
fn broadcast_channel_null_name_stringifies() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-null-name");

    let result = vm
        .eval("new BroadcastChannel(null).name")
        .expect("BroadcastChannel null name probe should evaluate");

    assert_eq!(result, "null");
}
#[test]
fn broadcast_channel_undefined_name_stringifies() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-undefined-name");

    let result = vm
        .eval("new BroadcastChannel(undefined).name")
        .expect("BroadcastChannel undefined name probe should evaluate");

    assert_eq!(result, "undefined");
}
#[test]
fn broadcast_channel_number_name_stringifies() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-number-name");

    let result = vm
        .eval("new BroadcastChannel(123).name")
        .expect("BroadcastChannel number name probe should evaluate");

    assert_eq!(result, "123");
}
#[test]
fn broadcast_channel_empty_name_is_allowed() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-empty-name");

    let result = vm
        .eval(
            r#"
            (() => {
              try {
                return new BroadcastChannel("").name;
              } catch (error) {
                return error.name;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel empty name probe should evaluate");

    assert_eq!(result, "");
}
#[test]
fn broadcast_channel_postmessage_requires_argument() {
    let mut vm =
        new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-message-required");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("wpt-message-required");
              try {
                channel.postMessage();
                return "ok";
              } catch (error) {
                return `${error.name}:${error instanceof TypeError}`;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel missing message probe should evaluate");

    assert_eq!(result, "TypeError:true");
}
#[test]
fn broadcast_channel_postmessage_accepts_null() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-null-message");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("wpt-null-message");
              try {
                channel.postMessage(null);
                return "ok";
              } catch (error) {
                return error.name;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel null message probe should evaluate");

    assert_eq!(result, "ok");
}
#[test]
fn broadcast_channel_close_is_idempotent() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-close");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("wpt-close");
              try {
                channel.close();
                channel.close();
                return "ok";
              } catch (error) {
                return error.name;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel close probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn broadcast_channel_repeated_close_does_not_close_first_channel_id() {
    let mut vm =
        new_broadcast_channel_test_vm("https://example.com/broadcast-detached-close-sentinel");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const ChildBroadcastChannel = frame.contentWindow.BroadcastChannel;
              const ChildDOMException = frame.contentWindow.DOMException;
              frame.remove();
              const detached = new ChildBroadcastChannel("detached-close-sentinel");
              detached.close();
              detached.close();
              try {
                detached.postMessage("after-close");
                return "posted";
              } catch (error) {
                return `${error.name}:${error instanceof DOMException}:${error instanceof ChildDOMException}:${error.code}`;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel detached close sentinel probe should evaluate");

    assert_eq!(result, "InvalidStateError:false:true:11");
}

#[test]
fn broadcast_channel_postmessage_after_close_throws_invalid_state() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-closed-post");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("wpt-closed-post");
              channel.close();
              try {
                channel.postMessage("");
                return "ok";
              } catch (error) {
                return `${error.name}:${error instanceof DOMException}:${error.code}`;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel closed postMessage probe should evaluate");

    assert_eq!(result, "InvalidStateError:true:11");
}
#[test]
fn broadcast_channel_onmessage_property_exists() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-onmessage");

    let result = vm
        .eval("String(new BroadcastChannel('wpt-onmessage').onmessage !== undefined)")
        .expect("BroadcastChannel onmessage probe should evaluate");

    assert_eq!(result, "true");
}

#[test]
fn broadcast_channel_declared_slots_ignore_string_property_spoofing() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-declared-slots");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("real");
              const initialSlots = Object.getOwnPropertyNames(channel)
                .filter(name => name.startsWith("__lmBroadcastChannel"))
                .sort();

              BroadcastChannel.prototype.__lmBroadcastChannelName = "proto-poison";
              BroadcastChannel.prototype.__lmBroadcastChannelClosed = true;
              BroadcastChannel.prototype.__lmBroadcastChannelOnmessage = () => "proto";
              BroadcastChannel.prototype.__lmBroadcastChannelOnmessageerror = () => "proto";
              Object.assign(channel, {
                __lmBroadcastChannelName: "own-poison",
                __lmBroadcastChannelClosed: true,
                __lmBroadcastChannelOnmessage: () => "own",
                __lmBroadcastChannelOnmessageerror: () => "own"
              });

              const before = [
                channel.name,
                channel.onmessage === null,
                channel.onmessageerror === null
              ].join("|");
              let postBefore;
              try {
                channel.postMessage("ok");
                postBefore = "posted";
              } catch (error) {
                postBefore = error.name;
              }

              channel.onmessage = function realHandler() {};
              const handlerAfterSet = [
                typeof channel.onmessage,
                channel.onmessage === channel.__lmBroadcastChannelOnmessage
              ].join(":");

              channel.close();
              channel.__lmBroadcastChannelClosed = false;
              let postAfter;
              try {
                channel.postMessage("after-close");
                postAfter = "posted";
              } catch (error) {
                postAfter = error.name;
              }

              return JSON.stringify({
                initialSlots,
                before,
                postBefore,
                handlerAfterSet,
                postAfter
              });
            })()
            "#,
        )
        .expect("BroadcastChannel declared slots spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialSlots":[],"before":"real|true|true","postBefore":"posted","handlerAfterSet":"function:false","postAfter":"InvalidStateError"}"#
    );
}

#[test]
fn broadcast_channel_prototype_callbacks_reject_unbranded_receivers() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-receiver-brand");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("receiver-brand");
              const fake = Object.create(channel);
              Object.assign(fake, {
                __lmBroadcastChannelBrand: true,
                __lmBroadcastChannelId: 1n,
                __lmBroadcastChannelName: "fake",
                __lmBroadcastChannelClosed: false,
                __lmBroadcastChannelOnmessage: () => "fake",
                __lmBroadcastChannelOnmessageerror: () => "fake"
              });
              const name = Object.getOwnPropertyDescriptor(BroadcastChannel.prototype, "name");
              const onmessage = Object.getOwnPropertyDescriptor(BroadcastChannel.prototype, "onmessage");
              const onmessageerror = Object.getOwnPropertyDescriptor(BroadcastChannel.prototype, "onmessageerror");
              const outcome = callback => {
                try {
                  const value = callback();
                  return value === undefined ? "undefined" : String(value);
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };

              return JSON.stringify({
                outcomes: [
                  outcome(() => name.get.call(fake)),
                  outcome(() => onmessage.get.call(fake)),
                  outcome(() => onmessage.set.call(fake, () => {})),
                  outcome(() => onmessageerror.get.call(fake)),
                  outcome(() => onmessageerror.set.call(fake, () => {})),
                  outcome(() => channel.postMessage.call(fake, "x")),
                  outcome(() => channel.close.call(fake)),
                  outcome(() => channel.addEventListener.call(fake, "message", () => {})),
                  outcome(() => channel.removeEventListener.call(fake, "message", () => {})),
                  outcome(() => channel.dispatchEvent.call(fake, new Event("message")))
                ],
                realSlots: Object.getOwnPropertyNames(channel)
                  .filter(name => name.startsWith("__lmBroadcastChannel"))
                  .sort(),
                fakeSlots: Object.getOwnPropertyNames(fake)
                  .filter(name => name.startsWith("__lmBroadcastChannel"))
                  .sort(),
                realName: channel.name,
                realOnmessage: channel.onmessage
              });
            })()
            "#,
        )
        .expect("BroadcastChannel receiver brand probe should evaluate");

    assert_eq!(
        result,
        r#"{"outcomes":["throw:TypeError","throw:TypeError","throw:TypeError","throw:TypeError","throw:TypeError","throw:TypeError","throw:TypeError","throw:TypeError","throw:TypeError","throw:TypeError"],"realSlots":[],"fakeSlots":["__lmBroadcastChannelBrand","__lmBroadcastChannelClosed","__lmBroadcastChannelId","__lmBroadcastChannelName","__lmBroadcastChannelOnmessage","__lmBroadcastChannelOnmessageerror"],"realName":"receiver-brand","realOnmessage":null}"#
    );
}

#[test]
fn broadcast_channel_ordered_handler_slot_ignores_reflection_and_spoofing() {
    let mut vm =
        new_broadcast_channel_test_vm("https://example.com/broadcast-ordered-handler-slot");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("ordered-handler-slot");
              const ownBefore = Object.getOwnPropertyNames(channel)
                .filter(name =>
                  name === "__moliEventTargetSlot" ||
                  name === "__moliSimpleEventTargetOrderedHandlers"
                );
              const summarize = name => {
                const descriptor = Object.getOwnPropertyDescriptor(channel, name);
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value,
                  descriptor && descriptor.value.length,
                  descriptor && descriptor.value.name
                ].join(":");
              };
              const calls = [];
              channel.addEventListener("message", () => calls.push("listener"));
              channel.onmessage = () => calls.push("handler");

              channel.__moliEventTargetSlot = "spoofedListeners";
              channel.__moliSimpleEventTargetOrderedHandlers = false;
              const publicSpoof = [
                channel.__moliEventTargetSlot,
                channel.__moliSimpleEventTargetOrderedHandlers
              ];
              channel.dispatchEvent(new MessageEvent("message", { data: "direct" }));
              channel.close();

              return JSON.stringify({
                ownBefore,
                publicSpoof,
                add: summarize("addEventListener"),
                remove: summarize("removeEventListener"),
                dispatch: summarize("dispatchEvent"),
                calls
              });
            })()
            "#,
        )
        .expect("BroadcastChannel ordered handler private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"ownBefore":[],"publicSpoof":["spoofedListeners",false],"add":"true:true:true:true:function:0:addEventListener","remove":"true:true:true:true:function:0:removeEventListener","dispatch":"true:true:true:true:function:0:dispatchEvent","calls":["listener","handler"]}"#
    );
}

#[test]
fn broadcast_channel_event_handler_preserves_objects_and_clears_primitives() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-handler-object");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("wpt-handler-object");
              const handlerObject = { handleEvent() {} };
              channel.onmessage = handlerObject;
              const messagePreserved = channel.onmessage === handlerObject;
              channel.onmessageerror = handlerObject;
              const messageErrorPreserved = channel.onmessageerror === handlerObject;
              channel.onmessage = 42;
              channel.onmessageerror = false;
              const primitivesCleared = channel.onmessage === null && channel.onmessageerror === null;
              channel.close();
              return `${messagePreserved}:${messageErrorPreserved}:${primitivesCleared}`;
            })()
            "#,
        )
        .expect("BroadcastChannel non-callable handler probe should evaluate");

    assert_eq!(result, "true:true:true");
}
#[test]
fn broadcast_channel_postmessage_uncloneable_data_throws_data_clone_error() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-uncloneable");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("wpt-uncloneable");
              try {
                channel.postMessage(Symbol());
                return "ok";
              } catch (error) {
                return `${error.name}:${error instanceof DOMException}:${error.code}`;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel uncloneable message probe should evaluate");

    assert_eq!(result, "DataCloneError:true:25");
}
#[test]
fn broadcast_channel_postmessage_after_close_prefers_invalid_state() {
    let mut vm =
        new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-closed-uncloneable");

    let result = vm
        .eval(
            r#"
            (() => {
              const channel = new BroadcastChannel("wpt-closed-uncloneable");
              channel.close();
              try {
                channel.postMessage(Symbol());
                return "ok";
              } catch (error) {
                return `${error.name}:${error instanceof DOMException}:${error.code}`;
              }
            })()
            "#,
        )
        .expect("BroadcastChannel closed uncloneable probe should evaluate");

    assert_eq!(result, "InvalidStateError:true:11");
}
#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_postmessage_dispatches_message_event_shape() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-event-shape");

    let scheduled = vm
        .eval(
            r#"
            (() => {
              const sender = new BroadcastChannel("wpt-event-shape");
              const receiver = new BroadcastChannel("wpt-event-shape");
              globalThis.__bcEventShape = [];
              receiver.onmessage = event => {
                __bcEventShape.push([
                  event instanceof MessageEvent,
                  event.target === receiver,
                  event.type,
                  event.origin,
                  event.data,
                  event.source === null,
                  event.ports.length
                ].join("|"));
              };
              sender.postMessage("hello world");
              return __bcEventShape.length;
            })()
            "#,
        )
        .expect("BroadcastChannel event shape setup should evaluate");

    assert_eq!(scheduled, "0");
    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__bcEventShape.join(';')")
            .expect("BroadcastChannel event shape result should evaluate"),
        "true|true|message|https://example.com|hello world|true|0"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_messages_are_delivered_in_creation_order() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-order");

    vm.eval(
        r#"
        (() => {
          const c1 = new BroadcastChannel("wpt-order");
          const c2 = new BroadcastChannel("wpt-order");
          const c3 = new BroadcastChannel("wpt-order");
          globalThis.__bcOrderEvents = [];
          const label = event => {
            if (event.target === c1) return "c1";
            if (event.target === c2) return "c2";
            if (event.target === c3) return "c3";
            return "unknown";
          };
          const handler = event => __bcOrderEvents.push(`${label(event)}:${event.data}`);
          c1.onmessage = handler;
          c2.onmessage = handler;
          c3.onmessage = handler;
          c1.postMessage("from c1");
          c3.postMessage("from c3");
          c2.postMessage("done");
        })()
        "#,
    )
    .expect("BroadcastChannel order setup should evaluate");

    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__bcOrderEvents.join('|')")
            .expect("BroadcastChannel order result should evaluate"),
        "c2:from c1|c3:from c1|c1:from c3|c2:from c3|c1:done|c3:done"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_does_not_deliver_to_channel_closed_before_post() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-closed-before");

    vm.eval(
        r#"
        (() => {
          const c1 = new BroadcastChannel("wpt-closed-before");
          const c2 = new BroadcastChannel("wpt-closed-before");
          const c3 = new BroadcastChannel("wpt-closed-before");
          globalThis.__bcClosedBefore = [];
          c2.onmessage = () => __bcClosedBefore.push("c2");
          c2.close();
          c3.onmessage = event => __bcClosedBefore.push(`c3:${event.data}`);
          c1.postMessage("test");
        })()
        "#,
    )
    .expect("BroadcastChannel closed-before setup should evaluate");

    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__bcClosedBefore.join('|')")
            .expect("BroadcastChannel closed-before result should evaluate"),
        "c3:test"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_does_not_deliver_to_channel_closed_after_post() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-closed-after");

    vm.eval(
        r#"
        (() => {
          const c1 = new BroadcastChannel("wpt-closed-after");
          const c2 = new BroadcastChannel("wpt-closed-after");
          const c3 = new BroadcastChannel("wpt-closed-after");
          globalThis.__bcClosedAfter = [];
          c2.onmessage = () => __bcClosedAfter.push("c2");
          c3.onmessage = event => __bcClosedAfter.push(`c3:${event.data}`);
          c1.postMessage("test");
          c2.close();
        })()
        "#,
    )
    .expect("BroadcastChannel closed-after setup should evaluate");

    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__bcClosedAfter.join('|')")
            .expect("BroadcastChannel closed-after result should evaluate"),
        "c3:test"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_closing_and_creating_during_delivery_works() {
    let mut vm =
        new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-create-in-message");

    vm.eval(
        r#"
        (() => {
          const c1 = new BroadcastChannel("wpt-create-in-message");
          const c2 = new BroadcastChannel("wpt-create-in-message");
          globalThis.__bcCreateDuringDelivery = [];
          c2.onmessage = event => {
            __bcCreateDuringDelivery.push(`c2:${event.data}`);
            c2.close();
            const c3 = new BroadcastChannel("wpt-create-in-message");
            c3.onmessage = event => __bcCreateDuringDelivery.push(`c3:${event.data}`);
            c1.postMessage("done");
          };
          c1.postMessage("first");
          c2.postMessage("second");
        })()
        "#,
    )
    .expect("BroadcastChannel create-during-delivery setup should evaluate");

    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__bcCreateDuringDelivery.join('|')")
            .expect("BroadcastChannel create-during-delivery result should evaluate"),
        "c2:first|c3:done"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_close_in_onmessage_suppresses_queued_events() {
    let mut vm =
        new_broadcast_channel_test_vm("https://example.com/broadcast-wpt-close-in-message");

    vm.eval(
        r#"
        (() => {
          const c1 = new BroadcastChannel("wpt-close-in-message");
          const c2 = new BroadcastChannel("wpt-close-in-message");
          const c3 = new BroadcastChannel("wpt-close-in-message");
          globalThis.__bcCloseInMessage = [];
          c1.onmessage = event => __bcCloseInMessage.push(`c1:${event.data}`);
          c2.onmessage = event => __bcCloseInMessage.push(`c2:${event.data}`);
          c3.onmessage = event => __bcCloseInMessage.push(`c3:${event.data}`);
          c2.addEventListener("message", () => c2.close());
          c1.postMessage("first");
          c1.postMessage("done");
        })()
        "#,
    )
    .expect("BroadcastChannel close-in-message setup should evaluate");

    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__bcCloseInMessage.join('|')")
            .expect("BroadcastChannel close-in-message result should evaluate"),
        "c2:first|c3:first|c3:done"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn broadcast_channel_postmessage_preserves_webassembly_module() {
    let mut vm = new_broadcast_channel_test_vm("https://example.com/broadcast-wasm-module");

    let scheduled = vm
        .eval(
            r#"
            (() => {
              const sender = new BroadcastChannel("wpt-wasm-module");
              const receiver = new BroadcastChannel("wpt-wasm-module");
              const module = new WebAssembly.Module(
                new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
              );
              globalThis.__bcWasmModuleEvents = [];
              receiver.onmessage = event => {
                const instance = new WebAssembly.Instance(event.data.module, {});
                __bcWasmModuleEvents.push([
                  event instanceof MessageEvent,
                  event.data.module instanceof WebAssembly.Module,
                  event.data.module === module,
                  Object.keys(instance.exports).length,
                  event.data.label
                ].join("|"));
              };
              sender.postMessage({ label: "wasm", module });
              return __bcWasmModuleEvents.length;
            })()
            "#,
        )
        .expect("BroadcastChannel WebAssembly.Module setup should evaluate");

    assert_eq!(scheduled, "0");
    apply_page_broadcast_channel_deliveries(&mut vm).await;
    assert_eq!(
        vm.eval("__bcWasmModuleEvents.join(';')")
            .expect("BroadcastChannel WebAssembly.Module result should evaluate"),
        "true|true|false|0|wasm"
    );
}

#[tokio::test]
async fn broadcast_channel_data_iframe_uses_opaque_storage_key() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_broadcast_channel_page_test_vm_with_loader(
        "https://broadcast-channel-opaque-child.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__opaqueBroadcastChannelMessages = [];
  const topChannel = new BroadcastChannel("opaque-child-owner");
  topChannel.onmessage = event => {
    __opaqueBroadcastChannelMessages.push("top:" + event.data + ":" + event.origin);
  };
  addEventListener("message", event => {
    __opaqueBroadcastChannelMessages.push(String(event.data));
  });

  const frame = document.createElement("iframe");
  frame.src = "data:text/html," + encodeURIComponent(`
    <!doctype html>
    <script>
      const receiver = new BroadcastChannel("opaque-child-owner");
      receiver.onmessage = event => {
        parent.postMessage("child:" + event.data + ":" + event.origin, "*");
      };
      const sender = new BroadcastChannel("opaque-child-owner");
      sender.postMessage("ping");
    <\/script>
  `);
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__opaqueBroadcastChannelFrame = frame;
  return "queued";
})()
"#,
    )
    .expect("opaque BroadcastChannel frame setup should evaluate");

    // NavigationCommit, realm admission, child lifecycle, BroadcastChannel
    // delivery and the parent WindowMessage are distinct production tasks.
    for _ in 0..16 {
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("opaque BroadcastChannel Page task executor should advance");
        apply_page_broadcast_channel_deliveries(&mut vm).await;
        if vm
            .eval("String(globalThis.__opaqueBroadcastChannelMessages.length)")
            .expect("opaque BroadcastChannel message length should evaluate")
            == "1"
        {
            break;
        }
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__opaqueBroadcastChannelMessages)")
            .expect("opaque BroadcastChannel messages should evaluate"),
        r#"["child:ping:null"]"#
    );
}
