use super::*;

use crate::page_task_queue::{PageDomManipulationTurnAction, PageViewTransitionUpdateTargetEffect};

async fn run_next_view_transition_update_task_through_selected_dispatcher_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<bool> {
    let Some(claimed) = page_vm.claim_exact_selected_page_task_for_test(
        PageSelectedTaskTestSelector::DomManipulation(
            PageDomManipulationTestFamily::ViewTransitionUpdate,
        ),
    ) else {
        return Ok(false);
    };
    page_vm
        .run_claimed_selected_page_task_for_test(claimed, loader)
        .await?;
    Ok(true)
}

#[tokio::test(flavor = "current_thread")]
async fn view_transition_update_callback_runs_on_window_owned_platform_task() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("resource request client");
        let document_url = Url::parse("https://example.com/view-transition-task").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let initial_materialization = [
            page_vm
                .vm_mut()
                .lazy_constructor_materialization_count_for_test("ViewTransition")?,
            page_vm
                .vm_mut()
                .lazy_constructor_materialization_count_for_test("ViewTransitionTypeSet")?,
        ];
        assert_eq!(
            initial_materialization,
            [0, 0],
            "view-transition constructors must remain lazy during Window bootstrap"
        );

        page_vm.vm_mut().eval(
            r#"
globalThis.__viewTransitionEvents = [];
const __viewTransitionGetPrototypeOf = Reflect.getPrototypeOf;
const __viewTransitionSetIteratorPrototype =
  __viewTransitionGetPrototypeOf(new Set().values());
const __originalSetMethods = {};
for (const name of ["entries", "forEach", "keys", "values"]) {
  __originalSetMethods[name] = Set.prototype[name];
  Set.prototype[name] = () => {
    throw new Error(`poisoned Set.prototype.${name}`);
  };
}
globalThis.__viewTransitionOptionGetterOrder = [];
const transition = document.startViewTransition({
  get update() {
    __viewTransitionOptionGetterOrder.push("update");
    return () => {
      __viewTransitionEvents.push("callback");
      queueMicrotask(() => __viewTransitionEvents.push("callback-microtask"));
    };
  },
  get types() {
    __viewTransitionOptionGetterOrder.push("types");
    return ["initial", "removed", "initial"];
  }
});
globalThis.__viewTransition = transition;
__viewTransitionEvents.push("after-start");
Promise.resolve().then(() => __viewTransitionEvents.push("initial-microtask"));
transition.updateCallbackDone.then(() => __viewTransitionEvents.push("update-done"));
transition.ready.then(() => __viewTransitionEvents.push("ready"));
transition.finished.then(() => __viewTransitionEvents.push("finished"));
const types = transition.types;
types.add("added").add("added");
types.delete("removed");
types.add("").add(".").add(123);
globalThis.__viewTransitionForEach = [];
const __viewTransitionThisArg = {};
types.forEach(function(value, key, owner) {
  __viewTransitionForEach.push([
    value,
    key,
    owner === types,
    this === __viewTransitionThisArg
  ]);
}, __viewTransitionThisArg);
const __viewTransitionTypeIterator = types.values();
for (const name of ["entries", "forEach", "keys", "values"]) {
  Set.prototype[name] = __originalSetMethods[name];
}
"queued"
"#,
        )?;
        let initial = page_vm.vm_mut().eval(
            r#"
JSON.stringify({
  events: __viewTransitionEvents,
  optionGetterOrder: __viewTransitionOptionGetterOrder,
  active: document.activeViewTransition === transition,
  transitionBrand:
    transition instanceof ViewTransition &&
    Object.prototype.toString.call(transition) === "[object ViewTransition]",
  typesBrand:
    types instanceof ViewTransitionTypeSet &&
    !(types instanceof Set) &&
    Object.prototype.toString.call(types) === "[object ViewTransitionTypeSet]",
  samePromises:
    transition.ready === transition.ready &&
    transition.finished === transition.finished &&
    transition.updateCallbackDone === transition.updateCallbackDone,
  sameTypes: transition.types === types,
  types: [...types],
  entries: [...types.entries()],
  forEach: __viewTransitionForEach,
  setlikeSurface:
    ViewTransitionTypeSet.prototype.keys !==
      ViewTransitionTypeSet.prototype.values &&
    ViewTransitionTypeSet.prototype[Symbol.iterator] ===
      ViewTransitionTypeSet.prototype.values,
  iteratorPrototypeShape: (() => {
    const prototype = __viewTransitionGetPrototypeOf(__viewTransitionTypeIterator);
    const next = Object.getOwnPropertyDescriptor(prototype, "next");
    const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
    return (
      __viewTransitionGetPrototypeOf(prototype) ===
        __viewTransitionSetIteratorPrototype &&
      __viewTransitionTypeIterator[Symbol.iterator]() ===
        __viewTransitionTypeIterator &&
      !Object.hasOwn(__viewTransitionTypeIterator, "next") &&
      !Object.hasOwn(__viewTransitionTypeIterator, Symbol.iterator) &&
      !Object.hasOwn(prototype, "constructor") &&
      next?.enumerable === true &&
      next?.writable === true &&
      next?.configurable === true &&
      tag?.value === "ViewTransitionTypeSet Iterator" &&
      tag?.enumerable === false &&
      tag?.writable === false &&
      tag?.configurable === true
    );
  })(),
  prototypeShape:
    Object.getPrototypeOf(ViewTransition.prototype) === Object.prototype &&
    Object.getPrototypeOf(ViewTransitionTypeSet.prototype) === Object.prototype,
  illegalInvocationNames: [
    () => new ViewTransition(),
    () => new ViewTransitionTypeSet(),
    () => ViewTransition.prototype.skipTransition.call({}),
    () => ViewTransitionTypeSet.prototype.add.call(new Set(), "x"),
    () => Set.prototype.has.call(types, "initial"),
    () => Set.prototype.values.call(types),
    () => Set.prototype.forEach.call(types, () => {}),
    () => Document.prototype.startViewTransition.call({}),
    () => document.startViewTransition(1),
    () => document.startViewTransition("callback"),
    () => document.startViewTransition(Symbol("callback")),
    () => types.add(Symbol("type"))
  ].map(callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.name;
    }
  }),
  lengths: [
    document.startViewTransition.length,
    transition.skipTransition.length,
    transition.waitUntil.length,
    types.add.length
  ]
})
"#,
        )?;
        assert_eq!(
            initial,
            r#"{"events":["after-start","initial-microtask"],"optionGetterOrder":["types","update"],"active":true,"transitionBrand":true,"typesBrand":true,"samePromises":true,"sameTypes":true,"types":["initial","added","",".","123"],"entries":[["initial","initial"],["added","added"],["",""],[".","."],["123","123"]],"forEach":[["initial","initial",true,true],["added","added",true,true],["","",true,true],[".",".",true,true],["123","123",true,true]],"setlikeSurface":true,"iteratorPrototypeShape":true,"prototypeShape":true,"illegalInvocationNames":["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"],"lengths":[0,0,1,1]}"#
        );
        let live_iterator = page_vm.vm_mut().eval(
            r#"
(() => {
  const iterator = types.values();
  const first = iterator.next();
  types.delete("initial");
  types.add("live-added");
  const remaining = [...iterator];
  types.delete("live-added");
  types.add("initial");
  return JSON.stringify([first, remaining]);
})()
"#,
        )?;
        assert_eq!(
            live_iterator,
            r#"[{"value":"initial","done":false},["added","",".","123","live-added"]]"#
        );
        let materialized = [
            page_vm
                .vm_mut()
                .lazy_constructor_materialization_count_for_test("ViewTransition")?,
            page_vm
                .vm_mut()
                .lazy_constructor_materialization_count_for_test("ViewTransitionTypeSet")?,
        ];
        assert_eq!(materialized, [1, 1]);
        assert!(!page_vm.vm().has_ready_timeout());

        let task = page_vm
            .take_dom_manipulation_body_task_for_test(
                PageDomManipulationTestFamily::ViewTransitionUpdate,
            )
            .expect("view-transition update callback should queue one platform task");
        let body = page_vm.apply_selected_page_dom_manipulation_turn(task).await?;
        let PageDomManipulationTurnAction::ViewTransitionUpdate(action) = body.action else {
            unreachable!("the selected view-transition task must retain its typed action")
        };
        assert_eq!(
            action.target_effect,
            PageViewTransitionUpdateTargetEffect::ProcessedForCurrentOwner
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionEvents.join('|')")?,
            "after-start|initial-microtask|callback",
            "the task body must leave callback and promise reactions for task completion"
        );

        page_vm
            .finish_selected_page_task_completion(action.into_page_task_completion(), &loader)
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionEvents.join('|')")?,
            "after-start|initial-microtask|callback|callback-microtask|update-done|ready|finished"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "String(document.activeViewTransition === null && \
                 __viewTransition.ready === __viewTransition.ready)"
            )?,
            "true"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("view-transition rendering-task test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn view_transition_update_uses_webidl_callback_function_semantics() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("resource request client");
        let document_url =
            Url::parse("https://example.com/view-transition-callback-function").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "view-transition-callback-frame";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "view-transition-callback-frame",
        )?;
        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.getElementById("view-transition-callback-frame");
  const child = frame.contentWindow;
  globalThis.__viewTransitionCallbackFrame = frame;
  globalThis.__viewTransitionCallbackFacts = null;
  globalThis.__viewTransitionCallbackEvents = [];
  globalThis.__viewTransitionProxyCalls = 0;
  const callback = child.Function(`
    return new Proxy(
      function() {
        "use strict";
        parent.__viewTransitionCallbackFacts = {
          callbackRealm:
            globalThis === parent.__viewTransitionCallbackFrame.contentWindow,
          receiverUndefined: this === undefined,
          argumentCount: arguments.length,
          proxyCalls: parent.__viewTransitionProxyCalls
        };
        return Promise.resolve();
      },
      {
        apply(target, receiver, argumentsList) {
          parent.__viewTransitionProxyCalls++;
          return Reflect.apply(target, receiver, argumentsList);
        }
      }
    );
  `)();
  globalThis.__viewTransitionCallbackTransition =
    document.startViewTransition(callback);
  __viewTransitionCallbackTransition.updateCallbackDone.then(
    () => __viewTransitionCallbackEvents.push("update")
  );
  __viewTransitionCallbackTransition.ready.then(
    () => __viewTransitionCallbackEvents.push("ready")
  );
  __viewTransitionCallbackTransition.finished.then(
    () => __viewTransitionCallbackEvents.push("finished")
  );
  return "queued";
})()
"#,
        )?;

        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__viewTransitionCallbackFacts)")?,
            r#"{"callbackRealm":true,"receiverUndefined":true,"argumentCount":0,"proxyCalls":1}"#
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionCallbackEvents.join('|')")?,
            "update|ready|finished"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("view-transition callback-function semantics test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn view_transition_update_rejection_preserves_callback_realm_exception() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("resource request client");
        let document_url =
            Url::parse("https://example.com/view-transition-callback-error-realm").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "view-transition-error-frame";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "view-transition-error-frame",
        )?;
        page_vm.vm_mut().eval(
            r#"
(() => {
  const child = document.getElementById("view-transition-error-frame").contentWindow;
  globalThis.__viewTransitionErrorFrame = child;
  globalThis.__viewTransitionErrorFacts = null;
  const callback = child.Function(`
    return function() {
      throw new Error("callback-realm-error");
    };
  `)();
  const transition = document.startViewTransition({ update: callback });
  transition.updateCallbackDone.catch(error => {
    __viewTransitionErrorFacts = {
      callbackRealmError:
        error instanceof __viewTransitionErrorFrame.Error &&
        !(error instanceof Error),
      message: error.message
    };
  });
  return "queued";
})()
"#,
        )?;

        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(__viewTransitionErrorFacts)")?,
            r#"{"callbackRealmError":true,"message":"callback-realm-error"}"#
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("view-transition callback error Realm test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn view_transition_update_rejects_when_callback_realm_retires() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("resource request client");
        let document_url =
            Url::parse("https://example.com/view-transition-callback-retirement").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement("iframe");
frame.id = "view-transition-retired-callback-frame";
document.body.appendChild(frame);
"created"
"#,
        )?;
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "view-transition-retired-callback-frame",
        )?;
        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.getElementById("view-transition-retired-callback-frame");
  const child = frame.contentWindow;
  globalThis.__retiredViewTransitionCallbackRan = false;
  globalThis.__retiredViewTransitionEvents = [];
  const callback = child.Function(`
    parent.__retiredViewTransitionCallbackRan = true;
  `);
  const transition = document.startViewTransition(callback);
  transition.updateCallbackDone.catch(error => {
    __retiredViewTransitionEvents.push(`update:${error.name}`);
  });
  transition.ready.catch(error => {
    __retiredViewTransitionEvents.push(`ready:${error.name}`);
  });
  transition.finished.catch(error => {
    __retiredViewTransitionEvents.push(`finished:${error.name}`);
  });
  frame.remove();
  return "queued";
})()
"#,
        )?;

        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "JSON.stringify({\
                   callbackRan: __retiredViewTransitionCallbackRan, \
                   events: __retiredViewTransitionEvents, \
                   active: document.activeViewTransition === null\
                 })"
            )?,
            r#"{"callbackRan":false,"events":["update:AbortError","ready:AbortError","finished:AbortError"],"active":true}"#
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("view-transition callback retirement test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn view_transition_update_callback_survives_document_open_in_same_window() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("resource request client");
        let document_url = Url::parse("https://example.com/view-transition-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let before_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("the initial Document should have an owner");

        page_vm.vm_mut().eval(
            r#"
globalThis.__viewTransitionDocumentOpenEvents = [];
globalThis.__viewTransitionAcrossDocumentOpen =
  document.startViewTransition(() => {
    __viewTransitionDocumentOpenEvents.push("callback");
  });
__viewTransitionAcrossDocumentOpen.updateCallbackDone.then(
  () => __viewTransitionDocumentOpenEvents.push("update")
);
__viewTransitionAcrossDocumentOpen.ready.then(
  () => __viewTransitionDocumentOpenEvents.push("ready")
);
__viewTransitionAcrossDocumentOpen.finished.then(
  () => __viewTransitionDocumentOpenEvents.push("finished")
);
document.open();
document.write("<!doctype html><p id=replacement>replacement</p>");
document.close();
__viewTransitionDocumentOpenEvents.push("after-open");
"queued"
"#,
        )?;
        let after_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("document.open should install a replacement Document owner");
        assert_ne!(before_owner, after_owner);
        assert_eq!(
            before_owner.local_window_id, after_owner.local_window_id,
            "document.open must preserve the LocalWindow task owner"
        );
        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?,
            "the pre-document.open callback must remain queued on its LocalWindow"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionDocumentOpenEvents.join('|')")?,
            "after-open|callback|update|ready|finished"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "String(document.activeViewTransition === null && \
                 document.getElementById('replacement') !== null)"
            )?,
            "true"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("view-transition document.open ownership test should run");
}

#[test]
fn stale_view_transition_task_does_not_steal_a_replacement_page_vm_callback() {
    run_page_vm_large_stack_async_test(
        "view-transition-page-vm-replacement-id-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><body>replacement</body>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("resource request client");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let retired_root = page_vm.document_lifecycle.identity().document;
                    page_vm.vm_mut().eval(
                        "document.startViewTransition(() => { \
                         globalThis.__retiredViewTransitionRan = true; \
                         }); 'queued-retired'",
                    )?;

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm
                        .vm_mut()
                        .eval(&format!("location.href = {replacement_url:?}; 'navigating'"))?;
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
globalThis.__replacementViewTransitionEvents = [];
globalThis.__replacementViewTransition = document.startViewTransition(() => {
  __replacementViewTransitionEvents.push("callback");
});
__replacementViewTransition.finished.then(() => {
  __replacementViewTransitionEvents.push("finished");
});
"queued-current"
"#,
                    )?;

                    let stale = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::DomManipulation(
                                PageDomManipulationTestFamily::ViewTransitionUpdate,
                            ),
                        )
                        .expect("the retired PageVm callback should consume one stale turn");
                    let (stale_owner, stale_task_id) = stale
                        .view_transition_update_owner_and_task_id()
                        .expect("the exact selector must retain the retired task identity");
                    assert_eq!(
                        stale_owner.root_document(),
                        retired_root,
                        "the first selected task must remain bound to the retired PageVm"
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(stale, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("String(globalThis.__retiredViewTransitionRan === true)")?,
                        "false",
                        "the production dispatcher must reject the retired callback"
                    );

                    let current = page_vm
                        .claim_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::DomManipulation(
                                PageDomManipulationTestFamily::ViewTransitionUpdate,
                            ),
                        )
                        .expect("the replacement callback must survive stale-head settlement");
                    let (current_owner, current_task_id) = current
                        .view_transition_update_owner_and_task_id()
                        .expect("the exact selector must retain the replacement task identity");
                    assert_eq!(
                        current_owner.root_document(),
                        current_root,
                        "the second selected task must belong to the replacement PageVm"
                    );
                    assert_ne!(stale_owner, current_owner);
                    assert_eq!(
                        stale_owner.target(),
                        current_owner.target(),
                        "fresh PageVm-local Window ids should exercise a real owner-key collision"
                    );
                    assert_eq!(
                        stale_task_id, current_task_id,
                        "fresh PageVm-local ledgers should reuse the local task id"
                    );
                    page_vm
                        .run_claimed_selected_page_task_for_test(current, &loader)
                        .await?;
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__replacementViewTransitionEvents.join('|')")?,
                        "callback|finished"
                    );
                    assert!(!page_vm.vm().has_ready_timeout());
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("view-transition replacement should use the full owner key");
            server.abort();
        },
    );
}

#[tokio::test(flavor = "current_thread")]
async fn detached_document_view_transition_skips_visual_phase_but_runs_update_callback() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("resource request client");
        let document_url = Url::parse("https://example.com/view-transition-detached").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__detachedViewTransitionEvents = [];
globalThis.__detachedViewTransitionDocument =
  new DOMParser().parseFromString("<!doctype html><p>detached</p>", "text/html");
globalThis.__detachedViewTransition =
  __detachedViewTransitionDocument.startViewTransition(() => {
    __detachedViewTransitionEvents.push("callback");
  });
__detachedViewTransition.updateCallbackDone.then(
  () => __detachedViewTransitionEvents.push("update")
);
__detachedViewTransition.ready.then(
  () => __detachedViewTransitionEvents.push("ready"),
  error => __detachedViewTransitionEvents.push(`ready:${error.name}:${error.message}`)
);
__detachedViewTransition.finished.then(
  () => __detachedViewTransitionEvents.push("finished")
);
__detachedViewTransitionEvents.push("after-start");
"queued"
"#,
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "JSON.stringify([\
                   __detachedViewTransitionEvents.join('|'), \
                   __detachedViewTransitionDocument.activeViewTransition === null\
                 ])"
            )?,
            r#"["after-start|ready:AbortError:Transition was skipped",true]"#
        );

        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?,
            "the detached Document callback must project onto the incumbent Window"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__detachedViewTransitionEvents.join('|')")?,
            "after-start|ready:AbortError:Transition was skipped|callback|update|finished"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("detached Document view-transition test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn replacing_view_transition_aborts_ready_but_preserves_callback_fifo() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("resource request client");
        let document_url = Url::parse("https://example.com/view-transition-replace").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__viewTransitionReplaceEvents = [];
globalThis.__firstViewTransition = document.startViewTransition(() => {
  __viewTransitionReplaceEvents.push("first-callback");
});
__firstViewTransition.ready.then(
  () => __viewTransitionReplaceEvents.push("first-ready"),
  error => __viewTransitionReplaceEvents.push(`first-ready:${error.name}`)
);
__firstViewTransition.updateCallbackDone.then(
  () => __viewTransitionReplaceEvents.push("first-update")
);
__firstViewTransition.finished.then(
  () => __viewTransitionReplaceEvents.push("first-finished")
);
__viewTransitionReplaceEvents.push("after-first");
globalThis.__secondViewTransition = document.startViewTransition(() => {
  __viewTransitionReplaceEvents.push("second-callback");
});
__secondViewTransition.updateCallbackDone.then(
  () => __viewTransitionReplaceEvents.push("second-update")
);
__secondViewTransition.ready.then(
  () => __viewTransitionReplaceEvents.push("second-ready")
);
__secondViewTransition.finished.then(
  () => __viewTransitionReplaceEvents.push("second-finished")
);
__viewTransitionReplaceEvents.push("after-second");
"queued"
"#,
        )?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionReplaceEvents.join('|')")?,
            "after-first|after-second|first-ready:AbortError"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "String(document.activeViewTransition === __secondViewTransition)"
            )?,
            "true"
        );

        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?,
            "the replaced transition callback should remain queued"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionReplaceEvents.join('|')")?,
            "after-first|after-second|first-ready:AbortError|first-callback|first-update|first-finished"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "String(document.activeViewTransition === __secondViewTransition)"
            )?,
            "true",
            "finishing an aborted predecessor must not clear its replacement"
        );

        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?,
            "the replacement transition callback should remain queued"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionReplaceEvents.join('|')")?,
            "after-first|after-second|first-ready:AbortError|first-callback|first-update|first-finished|second-callback|second-update|second-ready|second-finished"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.activeViewTransition === null)")?,
            "true"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("view-transition replacement test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn view_transition_waits_for_the_update_callback_promise() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("resource request client");
        let document_url =
            Url::parse("https://example.com/view-transition-async-callback").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__asyncViewTransitionEvents = [];
globalThis.__resolveAsyncViewTransition = undefined;
globalThis.__asyncViewTransition = document.startViewTransition(() => {
  __asyncViewTransitionEvents.push("callback");
  return new Promise(resolve => {
    __resolveAsyncViewTransition = resolve;
  });
});
__asyncViewTransition.updateCallbackDone.then(
  () => __asyncViewTransitionEvents.push("update")
);
__asyncViewTransition.ready.then(
  () => __asyncViewTransitionEvents.push("ready")
);
__asyncViewTransition.finished.then(
  () => __asyncViewTransitionEvents.push("finished")
);
"queued"
"#,
        )?;
        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?,
            "the asynchronous update callback should queue"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__asyncViewTransitionEvents.join('|')")?,
            "callback",
            "the transition promises must wait for the callback's returned Promise"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.activeViewTransition === __asyncViewTransition)")?,
            "true"
        );

        page_vm
            .vm_mut()
            .eval("__resolveAsyncViewTransition(); 'resolved'")?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__asyncViewTransitionEvents.join('|')")?,
            "callback|update|ready|finished"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.activeViewTransition === null)")?,
            "true"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("asynchronous view-transition callback test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn view_transition_rejection_and_wait_until_follow_chromium_promise_semantics() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
            .expect("resource request client");
        let document_url = Url::parse("https://example.com/view-transition-promises").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
globalThis.__viewTransitionRejectEvents = [];
globalThis.__viewTransitionError = new Error("callback failed");
globalThis.__rejectedViewTransition = document.startViewTransition(() => {
  __viewTransitionRejectEvents.push("callback");
  throw __viewTransitionError;
});
__rejectedViewTransition.updateCallbackDone.then(
  () => __viewTransitionRejectEvents.push("update-fulfilled"),
  error => __viewTransitionRejectEvents.push(`update:${error === __viewTransitionError}`)
);
__rejectedViewTransition.ready.then(
  () => __viewTransitionRejectEvents.push("ready-fulfilled"),
  error => __viewTransitionRejectEvents.push(`ready:${error === __viewTransitionError}`)
);
__rejectedViewTransition.finished.then(
  () => __viewTransitionRejectEvents.push("finished-fulfilled"),
  error => __viewTransitionRejectEvents.push(`finished:${error === __viewTransitionError}`)
);
"queued"
"#,
        )?;
        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?,
            "the rejecting transition callback should queue"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionRejectEvents.join('|')")?,
            "callback|update:true|ready:true|finished:true"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.activeViewTransition === null)")?,
            "true"
        );

        page_vm.vm_mut().eval(
            r#"
globalThis.__viewTransitionWaitEvents = [];
globalThis.__releaseViewTransitionWait = undefined;
const blocker = new Promise(resolve => {
  __releaseViewTransitionWait = resolve;
});
globalThis.__waitingViewTransition = document.startViewTransition(() => {
  __viewTransitionWaitEvents.push("callback");
});
__waitingViewTransition.waitUntil(blocker);
__waitingViewTransition.updateCallbackDone.then(
  () => __viewTransitionWaitEvents.push("update")
);
__waitingViewTransition.ready.then(
  () => __viewTransitionWaitEvents.push("ready")
);
__waitingViewTransition.finished.then(
  () => __viewTransitionWaitEvents.push("finished")
);
"queued"
"#,
        )?;
        assert!(
            run_next_view_transition_update_task_through_selected_dispatcher_for_test(
                &mut page_vm,
                &loader,
            )
            .await?,
            "the waiting transition callback should queue"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionWaitEvents.join('|')")?,
            "callback|update|ready"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.activeViewTransition === __waitingViewTransition)")?,
            "true"
        );

        page_vm
            .vm_mut()
            .eval("__releaseViewTransitionWait(); 'released'")?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__viewTransitionWaitEvents.join('|')")?,
            "callback|update|ready|finished"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.activeViewTransition === null)")?,
            "true"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("view-transition promise semantics test should run");
}
