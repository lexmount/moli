use super::*;

#[tokio::test]
async fn idle_callback_timeout_is_a_deadline_not_a_dispatch_delay() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm("https://idle-callback-timeout.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__idleCallbackTimeoutProbe = {};
  requestIdleCallback(deadline => {
    globalThis.__idleCallbackTimeoutProbe.expired = {
      didTimeout: deadline.didTimeout,
      timeRemaining: deadline.timeRemaining()
    };
  }, { timeout: 0 });
  requestIdleCallback(deadline => {
    globalThis.__idleCallbackTimeoutProbe.long = {
      didTimeout: deadline.didTimeout,
      hasTimeRemaining: deadline.timeRemaining() > 0
    };
  }, { timeout: 100000 });
  return "scheduled";
})()
"#,
    )
    .expect("idle callback timeout probe should schedule");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("idle callback timeout probe should drain");

    let result = vm
        .eval("JSON.stringify(globalThis.__idleCallbackTimeoutProbe)")
        .expect("idle callback timeout probe result should evaluate");

    assert_eq!(
        result,
        r#"{"expired":{"didTimeout":true,"timeRemaining":0},"long":{"didTimeout":false,"hasTimeRemaining":true}}"#
    );
}

#[tokio::test]
async fn idle_callback_does_not_overtake_queued_javascript_url_navigation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://idle-after-javascript-url.test/",
        &loader,
    );
    vm.eval(
        r#"
globalThis.__javascriptUrlIdleOrder = [];
const frame = document.createElement("iframe");
(document.body || document.documentElement || document).appendChild(frame);
const child = frame.contentWindow;
child.location.href =
  "javascript:parent.__javascriptUrlIdleOrder.push('javascript-url')";
child.requestIdleCallback(() => __javascriptUrlIdleOrder.push("idle"));
"#,
    )
    .expect("javascript URL and idle callback should queue");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__javascriptUrlIdleOrder.length)",
        "2",
        "javascript URL must run before the page becomes idle",
    )
    .await;
    assert_eq!(
        vm.eval("__javascriptUrlIdleOrder.join('|')")
            .expect("javascript URL idle order should evaluate"),
        "javascript-url|idle"
    );
}

#[tokio::test]
async fn idle_deadline_declared_shape_keeps_state_private() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm("https://idle-deadline-shape.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__idleDeadlineProbe = null;
  requestIdleCallback(deadline => {
    const internalOwnNames = object => Object.getOwnPropertyNames(object)
      .filter(name => name.startsWith("__moliIdleDeadline"))
      .sort()
      .join(",");
    const keys = Object.keys(deadline);
    const ownNames = Object.getOwnPropertyNames(deadline);
    const initialInternalOwnNames = internalOwnNames(deadline);
    const leaksInternalSlotNameBefore = "__moliIdleDeadlineMs" in deadline;
    IdleDeadline.prototype.__moliIdleDeadlineMs = Number.POSITIVE_INFINITY;
    deadline.__moliIdleDeadlineMs = Number.POSITIVE_INFINITY;
    const firstRemaining = deadline.timeRemaining();
    deadline.__moliIdleDeadlineMs = -1000000;
    const secondRemaining = deadline.timeRemaining();
    globalThis.__idleDeadlineProbe = JSON.stringify({
      ctor: deadline.constructor && deadline.constructor.name,
      didTimeout: deadline.didTimeout,
      typeofTimeRemaining: typeof deadline.timeRemaining,
      firstFinite: Number.isFinite(firstRemaining),
      secondFinite: Number.isFinite(secondRemaining),
      spoofIgnored: firstRemaining < 1000 && secondRemaining < 1000,
      leaksStateField: "deadlineState" in deadline,
      leaksInternalSlotNameBefore,
      initialInternalOwnNames,
      spoofedInternalOwnNames: internalOwnNames(deadline),
      keys,
      ownNames
    });
  });
  return "scheduled";
})()
"#,
    )
    .expect("IdleDeadline probe should schedule");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("IdleDeadline callback should drain");

    let result = vm
        .eval("globalThis.__idleDeadlineProbe")
        .expect("IdleDeadline probe result should evaluate");

    assert_eq!(
        result,
        r#"{"ctor":"IdleDeadline","didTimeout":false,"typeofTimeRemaining":"function","firstFinite":true,"secondFinite":true,"spoofIgnored":true,"leaksStateField":false,"leaksInternalSlotNameBefore":false,"initialInternalOwnNames":"","spoofedInternalOwnNames":"__moliIdleDeadlineMs","keys":[],"ownNames":[]}"#
    );
}

#[tokio::test]
async fn idle_callback_uses_webidl_callback_function_semantics() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm("https://idle-callback-webidl.test/");
    vm.eval(
        r#"
        const frame = document.createElement("iframe");
        (document.body || document.documentElement || document).appendChild(frame);
        globalThis.__idleCallbackFrame = frame;
        "#,
    )
    .expect("idle callback child-realm setup");
    materialize_single_child_default_realm_for_test(&mut vm, "idle callback child-realm setup");

    vm.eval(
        r#"
        (() => {
          const child = __idleCallbackFrame.contentWindow;
          globalThis.__idleCallbackFacts = null;
          globalThis.__idleCallbackProxyCalls = 0;
          const callback = child.Function(`
            return new Proxy(
              function(deadline) {
                "use strict";
                const ownRemaining = deadline.timeRemaining();
                const borrowedRemaining =
                  IdleDeadline.prototype.timeRemaining.call(deadline);
                parent.__idleCallbackFacts = {
                  callbackRealm:
                    globalThis === parent.__idleCallbackFrame.contentWindow,
                  receiverUndefined: this === undefined,
                  argumentCount: arguments.length,
                  deadlineRealm: deadline instanceof parent.IdleDeadline,
                  hasTimeRemaining: typeof deadline.timeRemaining === "function",
                  crossRealmTimeRemaining:
                    Math.abs(ownRemaining - borrowedRemaining) <= 5,
                  proxyCalls: parent.__idleCallbackProxyCalls
                };
              },
              {
                apply(target, receiver, argumentsList) {
                  parent.__idleCallbackProxyCalls++;
                  if (receiver !== undefined)
                    throw new Error("idle callback receiver was not undefined");
                  return Reflect.apply(target, receiver, argumentsList);
                }
              }
            );
          `)();
          requestIdleCallback(callback);
          return "scheduled";
        })()
        "#,
    )
    .expect("idle Web IDL callback should schedule");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("idle Web IDL callback should drain");
    assert_eq!(
        vm.eval("JSON.stringify(__idleCallbackFacts)")
            .expect("idle Web IDL callback facts"),
        r#"{"callbackRealm":true,"receiverUndefined":true,"argumentCount":1,"deadlineRealm":true,"hasTimeRemaining":true,"crossRealmTimeRemaining":true,"proxyCalls":1}"#
    );
}

#[tokio::test]
async fn idle_callback_exception_and_retirement_use_the_callback_realm() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm("https://idle-callback-lifetime.test/");
    vm.eval(
        r#"
        const errorFrame = document.createElement("iframe");
        const retiredFrame = document.createElement("iframe");
        const parent = document.body || document.documentElement || document;
        parent.appendChild(errorFrame);
        parent.appendChild(retiredFrame);
        globalThis.__idleCallbackErrorFrame = errorFrame;
        globalThis.__idleCallbackRetiredFrame = retiredFrame;
        "#,
    )
    .expect("idle callback lifetime setup");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval(
        r#"
        (() => {
          const errorChild = __idleCallbackErrorFrame.contentWindow;
          const retiredChild = __idleCallbackRetiredFrame.contentWindow;
          globalThis.__idleCallbackErrors = [];
          globalThis.__retiredIdleCallbackRan = false;
          errorChild.onerror = errorChild.Function(
            "message",
            `parent.__idleCallbackErrors.push(message); return true;`
          );
          requestIdleCallback(errorChild.Function(
            `throw new Error("idle-callback-error");`
          ));
          requestIdleCallback(retiredChild.Function(
            `parent.__retiredIdleCallbackRan = true;`
          ));
          __idleCallbackRetiredFrame.remove();
          return "scheduled";
        })()
        "#,
    )
    .expect("idle callback exception and retirement should schedule");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("idle callback exception and retirement should drain");
    assert_eq!(
        vm.eval(
            "JSON.stringify({ errors: __idleCallbackErrors, retired: __retiredIdleCallbackRan })"
        )
        .expect("idle callback lifetime result"),
        r#"{"errors":["Uncaught Error: idle-callback-error"],"retired":false}"#
    );
}

#[tokio::test]
async fn animation_frame_and_idle_callback_cancellation_retire_the_exact_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm("https://window-scheduled-callback-cancel.test/");

    assert_eq!(
        vm.eval(
            r#"
            globalThis.__cancelledWindowCallbackRuns = [];
            const animationFrameId = requestAnimationFrame(
              () => __cancelledWindowCallbackRuns.push("animation-frame")
            );
            const idleCallbackId = requestIdleCallback(
              () => __cancelledWindowCallbackRuns.push("idle")
            );
            cancelAnimationFrame(animationFrameId);
            cancelIdleCallback(idleCallbackId);
            String(animationFrameId > 0 && idleCallbackId > 0)
            "#,
        )
        .expect("Window scheduled callback cancellation setup"),
        "true"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("cancelled Window callbacks should leave the timer source quiescent");
    assert_eq!(
        vm.eval("__cancelledWindowCallbackRuns.join('|')")
            .expect("cancelled Window callback result"),
        ""
    );
}

#[test]
fn animation_frame_and_idle_callbacks_require_callable_webidl_values() {
    let mut vm = new_storage_test_vm("https://window-scheduled-callback-conversion.test/");
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const names = [];
              for (const schedule of [requestAnimationFrame, requestIdleCallback]) {
                try {
                  schedule({});
                  names.push("accepted");
                } catch (error) {
                  names.push(error.name);
                }
              }
              return names.join("|");
            })()
            "#,
        )
        .expect("Window scheduled callback conversion result"),
        "TypeError|TypeError"
    );
}

#[test]
fn animation_frame_and_idle_cancellation_use_required_unsigned_long_handles() {
    let mut vm = new_storage_test_vm("https://window-cancel-callback-conversion.test/");
    assert_eq!(
        vm.eval(
            r#"
            (() => {
              const probe = callback => {
                try {
                  return `return:${String(callback())}`;
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const facts = {};
              for (const [label, cancel] of [
                ["animation", cancelAnimationFrame],
                ["idle", cancelIdleCallback]
              ]) {
                let conversions = 0;
                facts[label] = {
                  missing: probe(() => cancel.call(window)),
                  wrongReceiver: probe(() => cancel.call({}, 0)),
                  symbol: probe(() => cancel.call(window, Symbol("handle"))),
                  object: probe(() => cancel.call(window, {
                    valueOf() {
                      conversions++;
                      return 0;
                    }
                  })),
                  conversions
                };
              }
              return JSON.stringify(facts);
            })()
            "#,
        )
        .expect("Window cancellation callback Web IDL conversion result"),
        r#"{"animation":{"missing":"throw:TypeError","wrongReceiver":"throw:TypeError","symbol":"throw:TypeError","object":"return:undefined","conversions":1},"idle":{"missing":"throw:TypeError","wrongReceiver":"throw:TypeError","symbol":"throw:TypeError","object":"return:undefined","conversions":1}}"#
    );
}
