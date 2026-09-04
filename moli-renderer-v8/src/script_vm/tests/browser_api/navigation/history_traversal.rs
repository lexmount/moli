use super::*;

#[test]
fn reset_navigation_history_preserves_current_entry_and_disposes_pruned_entries() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    vm.eval(
        r##"
(() => {
  history.pushState({ step: 1 }, "", "#one");
  history.pushState({ step: 2 }, "", "#two");
  globalThis.__lmResetHistoryEntries = navigation.entries();
  globalThis.__lmResetHistoryCurrent = navigation.currentEntry;
  globalThis.__lmResetHistoryDisposed = [];
  __lmResetHistoryEntries.forEach((entry, index) => {
    entry.addEventListener("dispose", () => __lmResetHistoryDisposed.push(index));
  });
})()
"##,
    )
    .expect("reset history setup should evaluate");

    assert!(
        vm.reset_navigation_history()
            .expect("reset history command should execute")
    );
    assert_eq!(
        vm.eval(
            r##"
JSON.stringify({
  historyLength: history.length,
  navigationLength: navigation.entries().length,
  sameCurrent: navigation.currentEntry === __lmResetHistoryCurrent,
  sameArrayEntry: navigation.entries()[0] === __lmResetHistoryCurrent,
  currentIndex: navigation.currentEntry.index,
  currentUrl: navigation.currentEntry.url,
  historyState: history.state.step,
  disposed: __lmResetHistoryDisposed
})
"##,
        )
        .expect("reset history result should evaluate"),
        r##"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"currentUrl":"https://example.com/base#two","historyState":2,"disposed":[1,0]}"##
    );
}

#[tokio::test]
async fn reset_navigation_history_updates_all_live_window_realms() {
    let mut vm = new_storage_test_vm("https://reset-history-realms.test/page.html");

    vm.eval(
        r##"
(() => {
  history.pushState({ realm: "top-default" }, "", "#top-default");
  const frame = document.createElement("iframe");
  frame.srcdoc = "<!doctype html><p>child</p>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"##,
    )
    .expect("reset history frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("reset history child realm should be materialized")
        .context_id;
    let child_frame_id = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("reset history child realm should exist")
        .frame_id
        .clone();
    vm.eval_in_child_default_context(
        child_context_id,
        r##"history.pushState({ realm: "child-default" }, "", "about:srcdoc#child-default")"##,
    )
    .expect("child default history setup should evaluate");

    let top_isolated_context_id = vm
        .create_isolated_world("reset-history-top-isolated", false)
        .expect("top isolated world should be created");
    let child_isolated_context_id = vm
        .create_isolated_world_for_frame(&child_frame_id, "reset-history-child-isolated", false)
        .expect("child isolated world should be created");
    vm.eval_in_isolated_context(
        top_isolated_context_id,
        r##"history.pushState({ realm: "top-isolated" }, "", "#top-isolated")"##,
    )
    .expect("top isolated history setup should evaluate");
    vm.eval_in_isolated_context(
        child_isolated_context_id,
        r##"history.pushState({ realm: "child-isolated" }, "", "about:srcdoc#child-isolated")"##,
    )
    .expect("child isolated history setup should evaluate");

    const INSTALL_ENTRY_OBSERVERS: &str = r#"
(() => {
  globalThis.__lmResetRealmEntries = navigation.entries();
  globalThis.__lmResetRealmCurrent = navigation.currentEntry;
  globalThis.__lmResetRealmDisposed = [];
  __lmResetRealmEntries.forEach((entry, index) => {
    entry.addEventListener("dispose", () => __lmResetRealmDisposed.push(index));
  });
})()
"#;
    vm.eval(INSTALL_ENTRY_OBSERVERS)
        .expect("top default reset observers should install");
    vm.eval_in_child_default_context(child_context_id, INSTALL_ENTRY_OBSERVERS)
        .expect("child default reset observers should install");
    vm.eval_in_isolated_context(top_isolated_context_id, INSTALL_ENTRY_OBSERVERS)
        .expect("top isolated reset observers should install");
    vm.eval_in_isolated_context(child_isolated_context_id, INSTALL_ENTRY_OBSERVERS)
        .expect("child isolated reset observers should install");

    vm.eval(
        r#"
(() => {
  const child = document.querySelector("iframe").contentWindow;
  globalThis.__lmResetRealmOrder = [];
  __lmResetRealmEntries[0].addEventListener("dispose", () => {
    __lmResetRealmOrder.push({
      listener: "top",
      sharedLengthPruned: history.length === 1 && child.history.length === 1,
      topEntries: navigation.entries().length,
      childEntries: child.navigation.entries().length
    });
  });
})()
"#,
    )
    .expect("top default reset order observer should install");
    vm.eval_in_child_default_context(
        child_context_id,
        r#"
__lmResetRealmEntries[0].addEventListener("dispose", () => {
  parent.__lmResetRealmOrder.push({
    listener: "child",
    sharedLengthPruned: history.length === 1 && parent.history.length === 1,
    topEntries: parent.navigation.entries().length,
    childEntries: navigation.entries().length
  });
});
"#,
    )
    .expect("child default reset order observer should install");

    assert!(
        vm.reset_navigation_history()
            .expect("multi-realm reset history command should execute")
    );

    const RESET_REALM_STATE: &str = r#"
JSON.stringify({
  historyLength: history.length,
  navigationLength: navigation.entries().length,
  sameCurrent: navigation.currentEntry === __lmResetRealmCurrent,
  sameArrayEntry: navigation.entries()[0] === __lmResetRealmCurrent,
  currentIndex: navigation.currentEntry.index,
  stateRealm: history.state.realm,
  disposedAll:
    __lmResetRealmDisposed.length === __lmResetRealmEntries.length - 1,
  disposedInReverseOrder:
    __lmResetRealmDisposed.every(
      (entryIndex, index, disposed) =>
        index === 0 || disposed[index - 1] > entryIndex
    )
})
"#;
    assert_eq!(
        vm.eval(RESET_REALM_STATE)
            .expect("top default reset state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"stateRealm":"top-isolated","disposedAll":true,"disposedInReverseOrder":true}"#
    );
    assert_eq!(
        vm.eval_in_child_default_context(child_context_id, RESET_REALM_STATE)
            .expect("child default reset state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"stateRealm":"child-isolated","disposedAll":true,"disposedInReverseOrder":true}"#
    );
    assert_eq!(
        vm.eval_in_isolated_context(top_isolated_context_id, RESET_REALM_STATE)
            .expect("top isolated reset state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"stateRealm":"top-isolated","disposedAll":true,"disposedInReverseOrder":true}"#
    );
    assert_eq!(
        vm.eval_in_isolated_context(child_isolated_context_id, RESET_REALM_STATE)
            .expect("child isolated reset state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"sameCurrent":true,"sameArrayEntry":true,"currentIndex":0,"stateRealm":"child-isolated","disposedAll":true,"disposedInReverseOrder":true}"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(__lmResetRealmOrder)")
            .expect("cross-realm reset order should evaluate"),
        r#"[{"listener":"top","sharedLengthPruned":true,"topEntries":1,"childEntries":3},{"listener":"child","sharedLengthPruned":true,"topEntries":1,"childEntries":1}]"#
    );
}

#[tokio::test]
async fn reset_navigation_history_preserves_child_entry_created_by_top_dispose_listener() {
    let mut vm = new_storage_test_vm("https://reset-history-reentrant-frame.test/page.html");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc = "<!doctype html><p>child</p>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("reentrant frame reset setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .next()
        .expect("reentrant reset child realm should be materialized")
        .context_id;

    vm.eval(r##"history.pushState({ realm: "top" }, "", "#top")"##)
        .expect("top reentrant reset history setup should evaluate");
    vm.eval_in_child_default_context(
        child_context_id,
        r##"
(() => {
  history.pushState(
    { realm: "child-before-reset" },
    "",
    "about:srcdoc#child-before-reset"
  );
  globalThis.__lmChildCurrentBeforeReset = navigation.currentEntry;
  globalThis.__lmChildEntriesBeforeReset = navigation.entries();
  globalThis.__lmChildDisposed = [];
  __lmChildEntriesBeforeReset.forEach((entry, index) => {
    entry.addEventListener("dispose", () => __lmChildDisposed.push(index));
  });
})()
"##,
    )
    .expect("child reentrant reset history setup should evaluate");
    vm.eval(
        r##"
navigation.entries()[0].addEventListener("dispose", () => {
  document.querySelector("iframe").contentWindow.history.pushState(
    { realm: "child-during-top-dispose" },
    "",
    "about:srcdoc#child-during-top-dispose"
  );
});
"##,
    )
    .expect("top dispose reentrant child navigation should install");

    assert!(
        vm.reset_navigation_history()
            .expect("reentrant frame reset history command should execute")
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            r#"
JSON.stringify({
  historyLength: history.length,
  navigationLength: navigation.entries().length,
  currentIndex: navigation.currentEntry.index,
  retainedPreviousCurrent:
    navigation.entries()[0] === __lmChildCurrentBeforeReset,
  appendedCurrent:
    navigation.entries()[1] === navigation.currentEntry,
  stateRealm: history.state.realm,
  disposed: __lmChildDisposed
})
"#,
        )
        .expect("reentrant child reset history state should evaluate"),
        r#"{"historyLength":2,"navigationLength":2,"currentIndex":1,"retainedPreviousCurrent":true,"appendedCurrent":true,"stateRealm":"child-during-top-dispose","disposed":[0]}"#
    );
}

#[tokio::test]
async fn reset_navigation_history_updates_prebootstrapped_child_default_realm() {
    let mut vm = new_storage_test_vm("https://reset-history-prebootstrap.test/page.html");

    vm.eval(
        r##"
(() => {
  history.pushState({ step: 1 }, "", "#one");
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  void frame.contentWindow;
})()
"##,
    )
    .expect("prebootstrapped child reset setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "prebootstrapped child reset setup",
    )
    .await;
    assert_eq!(
        vm.prebootstrapped_child_default_contexts.borrow().len(),
        1,
        "contentWindow access should prebootstrap one child default context"
    );
    assert_eq!(
        vm.child_frame_realm_store.len(),
        0,
        "the child default context should not be materialized before reset"
    );

    assert!(
        vm.reset_navigation_history()
            .expect("prebootstrapped child reset history command should execute")
    );
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "prebootstrapped child after reset",
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            "JSON.stringify({ historyLength: history.length, navigationLength: navigation.entries().length, currentIndex: navigation.currentEntry.index })",
        )
        .expect("materialized child reset history state should evaluate"),
        r#"{"historyLength":1,"navigationLength":1,"currentIndex":0}"#
    );
}

#[tokio::test]
async fn history_traversal_uses_active_document_sandbox_until_navigation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let restricted = "allow-scripts allow-same-origin";
    let permitted = "allow-scripts allow-same-origin allow-top-navigation";
    for (initial, changed, initially_allowed, subsequently_allowed) in [
        (None, Some(restricted), true, false),
        (Some(permitted), Some(restricted), true, false),
        (Some(restricted), None, false, true),
        (Some(restricted), Some(permitted), false, true),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            "https://history-active-sandbox.test/parent",
            &loader,
        );
        let initial = serde_json::to_string(&initial).unwrap();
        let changed = serde_json::to_string(&changed).unwrap();
        vm.eval(&format!(
            r#"
globalThis.frame = document.createElement('iframe');
if ({initial} !== null) frame.setAttribute('sandbox', {initial});
frame.srcdoc = '<p>original document</p>';
(document.body || document.documentElement || document).appendChild(frame);
"#
        ))
        .unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .unwrap();
        vm.eval(&format!(
            r#"
globalThis.originalDocument = frame.contentDocument;
if ({changed} === null) frame.removeAttribute('sandbox');
else frame.setAttribute('sandbox', {changed});
"#
        ))
        .unwrap();
        assert_eq!(
            vm.eval("frame.contentDocument === originalDocument")
                .unwrap(),
            "true"
        );

        for (navigate, allowed) in [(false, initially_allowed), (true, subsequently_allowed)] {
            if navigate {
                vm.eval("frame.srcdoc = '<p>replacement document</p>';")
                    .unwrap();
                vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
                    .await
                    .unwrap();
                assert_eq!(
                    vm.eval("frame.contentDocument === originalDocument")
                        .unwrap(),
                    "false"
                );
            }
            vm.eval(
                "history.replaceState(0, ''); history.pushState(1, ''); frame.contentWindow.eval('history.back()');",
            )
            .unwrap();
            assert_eq!(
                vm.run_one_history_traversal_executor_turn(&loader)
                    .await
                    .unwrap(),
                allowed,
                "initial={initial}, changed={changed}, navigated={navigate}"
            );
            assert_eq!(
                vm.eval("String(history.state)").unwrap(),
                if allowed { "0" } else { "1" },
                "initial={initial}, changed={changed}, navigated={navigate}"
            );
            assert!(vm.take_pending_top_level_history_traversal().is_none());
        }
    }
}

#[tokio::test]
async fn history_methods_from_child_realm_traverse_receiver_history() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://history-cross-realm.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts allow-same-origin");
  frame.srcdoc = "<p>child</p>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("cross-realm History frame setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");

    let setup = vm
        .eval(
            r##"
(() => {
  globalThis.__lmCrossRealmHistoryEvents = [];
  onpopstate = () => __lmCrossRealmHistoryEvents.push(location.hash || "initial");
  history.pushState({ step: 1 }, "", "#one");
  history.pushState({ step: 2 }, "", "#two");
  document.querySelector("iframe").contentWindow.history.back.call(history);
  return location.hash;
})()
"##,
        )
        .expect("borrowed child History.back should queue against the receiver");
    assert_eq!(setup, "#two");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`",
        "#one|#one",
        "borrowed child History.back should traverse the receiver history",
    )
    .await;
    assert_eq!(
        vm.eval("`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`")
            .expect("borrowed child History.back result should evaluate"),
        "#one|#one"
    );

    vm.eval(
        r#"
document.querySelector("iframe").contentWindow.history.forward.call(history);
"#,
    )
    .expect("borrowed child History.forward should queue against the receiver");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`",
        "#two|#one,#two",
        "borrowed child History.forward should traverse the receiver history",
    )
    .await;
    assert_eq!(
        vm.eval("`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`")
            .expect("borrowed child History.forward result should evaluate"),
        "#two|#one,#two"
    );

    vm.eval(
        r#"
document.querySelector("iframe").contentWindow.history.go.call(history, -2);
"#,
    )
    .expect("borrowed child History.go should queue against the receiver");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`",
        "|#one,#two,initial",
        "borrowed child History.go should traverse the receiver history",
    )
    .await;
    assert_eq!(
        vm.eval("`${location.hash}|${__lmCrossRealmHistoryEvents.join(',')}`")
            .expect("borrowed child History.go result should evaluate"),
        "|#one,#two,initial"
    );
}

#[tokio::test]
async fn traversal_navigate_destination_index_tracks_entry_identity() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);

    let before_back = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmTraversalDestinationProbe = { log: [] };
              const startIndex = navigation.currentEntry.index;
              navigation.navigate("#1");
              navigation.addEventListener("navigate", e => {
                globalThis.__lmTraversalDestinationProbe.backDestination = e.destination;
                globalThis.__lmTraversalDestinationProbe.log.push(`back:${startIndex}:${e.destination.index}`);
              }, { once: true });
              navigation.back().finished.then(
                () => globalThis.__lmTraversalDestinationProbe.log.push("backFinished"),
                error => globalThis.__lmTraversalDestinationProbe.log.push(`backRejected:${error.name}`)
              );
              return globalThis.__lmTraversalDestinationProbe.log.join("|");
            })()
            "##,
        )
        .expect("traversal destination setup should evaluate");
    assert_eq!(before_back, "");

    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("queued history traversal should run")
    );
    let back_finished = vm
        .eval("globalThis.__lmTraversalDestinationProbe.log.join('|')")
        .expect("back traversal promise should be inspectable");
    assert_eq!(back_finished, "back:0:0|backFinished");

    let after_replace = vm
        .eval(
            r##"
            (() => {
              navigation.navigate("#clobber_back", { history: "replace" });
              return `${location.hash}:${globalThis.__lmTraversalDestinationProbe.backDestination.index}`;
            })()
            "##,
        )
        .expect("replace after traversal should evaluate");
    assert_eq!(after_replace, "#clobber_back:-1");

    let forward = vm
        .eval(
            r##"
            (() => {
              const probe = globalThis.__lmTraversalDestinationProbe;
              navigation.addEventListener("navigate", e => {
                probe.forwardInitial = e.destination.index;
                navigation.navigate("#clobber_forward");
                probe.forwardAfterNestedNavigate = e.destination.index;
              }, { once: true });
              const result = navigation.forward();
              result.committed.catch(error => { probe.forwardCommittedRejected = error.name; });
              result.finished.catch(error => { probe.forwardFinishedRejected = error.name; });
              return JSON.stringify({
                initial: probe.forwardInitial,
                after: probe.forwardAfterNestedNavigate,
                committedRejected: probe.forwardCommittedRejected || null,
                finishedRejected: probe.forwardFinishedRejected || null
              });
            })()
            "##,
        )
        .expect("forward traversal destination should evaluate");
    assert_eq!(
        forward,
        r#"{"committedRejected":null,"finishedRejected":null}"#
    );
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("queued forward traversal should run")
    );
    let forward_rejections = vm
        .eval(
            r##"
            JSON.stringify({
              initial: globalThis.__lmTraversalDestinationProbe.forwardInitial,
              after: globalThis.__lmTraversalDestinationProbe.forwardAfterNestedNavigate,
              committedRejected: globalThis.__lmTraversalDestinationProbe.forwardCommittedRejected || null,
              finishedRejected: globalThis.__lmTraversalDestinationProbe.forwardFinishedRejected || null
            })
            "##,
        )
        .expect("forward traversal rejection microtasks should settle");
    assert_eq!(
        forward_rejections,
        r#"{"initial":1,"after":-1,"committedRejected":"AbortError","finishedRejected":"AbortError"}"#
    );

    let helper_style_wait = vm
        .eval(
            r##"
            (() => {
              const probe = globalThis.__lmTraversalDestinationProbe;
              const { promise: all, resolve, reject } = Promise.withResolvers();
              let remaining = 0;
              const result = navigation.forward();
              for (const promise of [
                result.committed.then(
                  () => { probe.secondCommitted = "fulfilled"; },
                  error => { probe.secondCommitted = error.name; }
                ),
                result.finished.then(
                  () => { probe.secondFinished = "fulfilled"; },
                  error => { probe.secondFinished = error.name; }
                )
              ]) {
                remaining++;
                promise.then(() => {
                  --remaining;
                  if (!remaining) resolve("done");
                }, error => reject(error));
              }
              all.then(value => { probe.secondAll = value; }, error => { probe.secondAll = error.name; });
              return JSON.stringify({
                committed: probe.secondCommitted || null,
                finished: probe.secondFinished || null,
                all: probe.secondAll || null
              });
            })()
            "##,
        )
        .expect("helper-style wait probe should evaluate");
    assert_eq!(
        helper_style_wait,
        r#"{"committed":null,"finished":null,"all":null}"#
    );
    let helper_style_wait_after_microtasks = vm
        .eval(
            r##"
            JSON.stringify({
              committed: globalThis.__lmTraversalDestinationProbe.secondCommitted || null,
              finished: globalThis.__lmTraversalDestinationProbe.secondFinished || null,
              all: globalThis.__lmTraversalDestinationProbe.secondAll || null
            })
            "##,
        )
        .expect("helper-style wait microtasks should settle");
    assert_eq!(
        helper_style_wait_after_microtasks,
        r#"{"committed":"InvalidStateError","finished":"InvalidStateError","all":"done"}"#
    );
}

#[tokio::test]
async fn traverse_to_preserves_intervening_history_back() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              history.replaceState(null, "", "#0");
              const keys = [navigation.currentEntry.key];
              for (let i = 1; i <= 3; i++) {
                history.pushState(null, "", `#${i}`);
                keys.push(navigation.currentEntry.key);
              }
              const probe = globalThis.__lmMixedTraversals = {
                popstate: [], committed: [], finished: []
              };
              onpopstate = () => probe.popstate.push(location.hash);
              const observe = (label, result) => {
                result.committed.then(
                  entry => probe.committed.push(`${label}:${new URL(entry.url).hash}:${location.hash}`),
                  error => probe.committed.push(`${label}:rejected:${error.name}`)
                );
                result.finished.then(
                  entry => probe.finished.push(`${label}:${new URL(entry.url).hash}`),
                  error => probe.finished.push(`${label}:rejected:${error.name}`)
                );
              };
              const first = navigation.traverseTo(keys[2]);
              observe("first", first);
              history.back();
              const last = navigation.traverseTo(keys[0]);
              observe("last", last);
              return [location.hash, first.committed !== last.committed,
                first.finished !== last.finished].join("|");
            })()
            "##,
        )
        .expect("mixed traversal requests should queue in one script turn");
    assert_eq!(setup, "#3|true|true");

    let mut executed = Vec::new();
    for _ in 0..4 {
        executed.push(
            vm.run_one_history_traversal_executor_turn(&loader)
                .await
                .expect("mixed history traversal should execute"),
        );
    }
    let settled = vm
        .eval("JSON.stringify({hash: location.hash, ...__lmMixedTraversals})")
        .expect("mixed traversal results should be inspectable");
    assert_eq!(
        settled,
        r##"{"hash":"#0","popstate":["#2","#1","#0"],"committed":["first:#2:#2","last:#0:#0"],"finished":["first:#2","last:#0"]}"##
    );
    assert_eq!(executed, [true, true, true, false]);
}

#[tokio::test]
async fn repeated_traverse_to_reuses_promises_after_history_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              history.pushState(null, "", "#1");
              const key = navigation.currentEntry.key;
              history.pushState(null, "", "#2");
              history.back();
              const first = navigation.traverseTo(key);
              const second = navigation.traverseTo(key);
              globalThis.__lmRepeatedAfterHistory = [];
              for (const [label, result] of [["first", first], ["second", second]]) {
                result.finished.then(
                  entry => __lmRepeatedAfterHistory.push(`${label}:${new URL(entry.url).hash}`),
                  error => __lmRepeatedAfterHistory.push(`${label}:rejected:${error.name}`)
                );
              }
              return [first !== second, first.committed === second.committed,
                first.finished === second.finished].join("|");
            })()
            "##,
        )
        .expect("a History request should not hide a matching Navigation request");
    assert_eq!(setup, "true|true|true");

    for _ in 0..2 {
        assert!(
            vm.run_one_history_traversal_executor_turn(&loader)
                .await
                .expect("History and Navigation requests should execute separately")
        );
    }
    assert_eq!(
        vm.eval("[location.hash, ...__lmRepeatedAfterHistory].join('|')")
            .expect("both callers should finish at their shared destination"),
        "#1|first:#1|second:#1"
    );
    assert!(
        !vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("the repeated Navigation request should not add another task")
    );
}

#[tokio::test]
async fn repeated_traverse_to_reuses_pending_navigation_promises() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              const key = navigation.currentEntry.key;
              navigation.navigate("#one");
              const first = navigation.traverseTo(key, { info: "first" });
              const second = navigation.traverseTo(key, { info: "second" });
              globalThis.__lmRepeatedTraverseTo = { first, second, log: [] };
              navigation.addEventListener("navigate", event => {
                __lmRepeatedTraverseTo.log.push(`info:${event.info}`);
              }, { once: true });
              first.finished.then(
                entry => globalThis.__lmRepeatedTraverseTo.log.push(`finished:${entry.url}:${location.hash}`),
                error => globalThis.__lmRepeatedTraverseTo.log.push(`rejected:${error.name}`)
              );
              return [
                first !== second,
                first.committed === second.committed,
                first.finished === second.finished
              ].join("|");
            })()
            "##,
        )
        .expect("repeated traverseTo setup should evaluate");

    assert_eq!(setup, "true|true|true");
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("queued repeated traverseTo should run")
    );
    let settled = vm
        .eval("globalThis.__lmRepeatedTraverseTo.log.join('|')")
        .expect("repeated traverseTo settlement should evaluate");
    assert_eq!(settled, "info:first|finished:https://example.com/base:");
    assert!(
        !vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("repeated traverseTo should share one traversal task")
    );
}
