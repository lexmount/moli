use super::*;

#[test]
fn history_api_url_updates_preserve_scroll_and_focus() {
    for method in ["pushState", "replaceState"] {
        for destination in ["/more?month=8", "#destination"] {
            let mut vm = new_parsed_test_vm(
                "https://example.com/profile",
                r#"<!doctype html><body style="margin:0;width:3000px;height:5000px">
                <input id="focused">
                <div id="destination" style="position:absolute;top:3500px">month</div>"#,
            );
            refresh_layout_for_test(&mut vm);
            vm.eval(
                r#"focused.focus({preventScroll:true}); scrollTo(120,900);
                window.events=[];
                for (const type of ['navigate','currententrychange','navigatesuccess'])
                    navigation.addEventListener(type,()=>events.push(type));
                window.position=()=>[scrollX,scrollY,document.scrollingElement.scrollLeft,
                    document.scrollingElement.scrollTop,document.activeElement.id].join('|');"#,
            )
            .unwrap();
            refresh_layout_for_test(&mut vm);
            let layouts = vm.layout_pass_observability_for_test().1;
            assert_eq!(
                vm.eval(&format!(
                    "history.{method}({{month:8}},'',{destination:?}); position()+'|'+events.join(',')"
                ))
                .unwrap(),
                "120|900|120|900|focused|navigate,currententrychange",
                "{method}({destination}) before completion"
            );
            // History API completion still emits navigatesuccess, but must not
            // apply the fragment/top scrolling used by an actual navigation.
            assert_eq!(
                vm.eval("position()+'|'+events.join(',')").unwrap(),
                "120|900|120|900|focused|navigate,currententrychange,navigatesuccess",
                "{method}({destination}) after completion"
            );
            assert_eq!(vm.layout_pass_observability_for_test().1, layouts);
            refresh_layout_for_test(&mut vm);
            assert_eq!(vm.eval("position()").unwrap(), "120|900|120|900|focused");
        }
    }
}

#[test]
fn same_document_navigation_dispatches_navigatesuccess() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmSameDocumentSuccessLog = [];
              navigation.onnavigatesuccess = () => {
                globalThis.__lmSameDocumentSuccessLog.push(`success:${location.hash}`);
              };
              navigation.navigate("#one");
              return globalThis.__lmSameDocumentSuccessLog.join("|");
            })()
            "##,
        )
        .expect("same-document navigatesuccess probe should evaluate");

    assert_eq!(result, "");
    let after_microtask = vm
        .eval("globalThis.__lmSameDocumentSuccessLog.join('|')")
        .expect("same-document navigatesuccess log should evaluate after microtask");
    assert_eq!(after_microtask, "success:#one");
}

#[test]
fn same_document_navigation_precommit_redirect_updates_destination() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let before_commit = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmPrecommitRedirectLog = [];
              globalThis.__lmPrecommitRedirectStartLength = navigation.entries().length;
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: controller => {
                    __lmPrecommitRedirectLog.push(`before:${location.hash}:${event.info.flag}:${event.destination.getState().value}`);
                    controller.redirect("#redirected", {
                      history: "replace",
                      info: { flag: "redirected" },
                      state: { value: 2 }
                    });
                    __lmPrecommitRedirectLog.push(`after:${location.hash}:${new URL(event.destination.url).hash}:${event.info.flag}:${event.destination.getState().value}`);
                  }
                });
              };
              navigation.navigate("#push", {
                history: "push",
                info: { flag: "initial" },
                state: { value: 1 }
              });
              return `${__lmPrecommitRedirectLog.join("|")}|pending:${location.hash}`;
            })()
            "##,
        )
        .expect("precommit redirect probe should evaluate");

    assert_eq!(
        before_commit,
        "before::initial:1|after::#redirected:redirected:2|pending:"
    );
    assert_eq!(
        vm.eval(
            r##"`final:${location.hash}:${navigation.entries().length - __lmPrecommitRedirectStartLength}:${navigation.currentEntry.getState().value}`"##
        )
        .expect("precommit redirect should commit after its Promise boundary"),
        "final:#redirected:0:2"
    );
}

#[tokio::test]
async fn same_document_navigation_precommit_added_handler_delays_finished() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmPrecommitAddHandlerLog = [];
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: controller => {
                    controller.addHandler(() => new Promise(resolve => {
                      setTimeout(() => {
                        globalThis.__lmPrecommitAddHandlerLog.push("added");
                        resolve();
                      }, 0);
                    }));
                  },
                  handler: () => {
                    globalThis.__lmPrecommitAddHandlerLog.push("handler");
                  }
                });
              };
              navigation.navigate("#one").finished.then(() => {
                globalThis.__lmPrecommitAddHandlerLog.push("finished");
              });
              return `${location.hash}:${globalThis.__lmPrecommitAddHandlerLog.join("|")}`;
            })()
            "##,
        )
        .expect("precommit addHandler setup should evaluate");

    assert_eq!(setup, ":");
    assert_eq!(
        vm.eval("`${location.hash}:${globalThis.__lmPrecommitAddHandlerLog.join('|')}`")
            .expect("precommit completion should commit before the added timer"),
        "#one:handler"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("precommit added handler timer should drain");
    let after_timeout = vm
        .eval("globalThis.__lmPrecommitAddHandlerLog.join('|')")
        .expect("precommit addHandler log should evaluate");

    assert_eq!(after_timeout, "handler|added|finished");
}
#[tokio::test]
async fn window_stop_cancels_child_location_navigation_without_committing_history() {
    for action in [
        "child.location.search = '?blocked'",
        "child.location.assign('/blocked')",
        "child.location.replace('/blocked')",
        "child.location.reload()",
        "const result = child.navigation.navigate('/blocked'); result.committed.catch(() => {}); result.finished.catch(() => {})",
    ] {
        let server = StaticHttpServer::spawn(3).await;
        let parent_url = server.url_for_host("window-stop.test", "/page.html");
        let loader = static_http_loader([server.resolve_entry("window-stop.test")]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
        vm.eval(
            r#"
globalThis.__stopFrame = document.createElement('iframe');
globalThis.__stopLoads = 0;
__stopFrame.onload = () => ++__stopLoads;
__stopFrame.src = '/child.html';
(document.body || document.documentElement || document).appendChild(__stopFrame);
"#,
        )
        .expect("window.stop child setup should evaluate");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__stopLoads)",
            "1",
            "initial child should load before the canceled navigation",
        )
        .await;

        let snapshot = vm
            .eval(&format!(
                r#"
(() => {{
  globalThis.__stopSibling = document.createElement('iframe');
  globalThis.__stopSiblingLoaded = false;
  __stopSibling.onload = () => {{ __stopSiblingLoaded = true; }};
  __stopSibling.src = '/sibling.html';
  (document.body || document.documentElement || document).appendChild(__stopSibling);
  const child = __stopFrame.contentWindow;
  child.history.replaceState({{ retained: true }}, '', '#original');
  const documentBefore = child.document;
  const entryBefore = child.navigation.currentEntry;
  const lengthBefore = child.history.length;
  const hrefBefore = child.location.href;
  const changes = [];
  child.navigation.oncurrententrychange = () => changes.push('change');
  globalThis.__stopSnapshot = () => [
    child.document === documentBefore,
    child.location.href === hrefBefore,
    child.document.URL === hrefBefore,
    child.navigation.currentEntry === entryBefore,
    child.history.length === lengthBefore,
    child.history.state?.retained === true,
    changes.length,
    __stopLoads
  ].join('|');
  {action};
  const beforeStop = __stopSnapshot();
  child.stop();
  child.stop();
  return beforeStop + ';' + __stopSnapshot();
}})()
"#,
            ))
            .expect("stopping a child navigation should evaluate");
        const UNCHANGED: &str = "true|true|true|true|true|true|0|1";
        assert_eq!(snapshot, format!("{UNCHANGED};{UNCHANGED}"), "{action}");

        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__stopSiblingLoaded)",
            "true",
            "a sibling should load while the stopped child remains unchanged",
        )
        .await;
        assert_eq!(
            vm.eval("__stopSnapshot()")
                .expect("stopped child snapshot should evaluate"),
            UNCHANGED,
            "{action} must not commit from a queued navigation task"
        );

        vm.eval("__stopFrame.contentWindow.location.assign('/after-stop.html')")
            .expect("navigation after stop should schedule");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__stopLoads)",
            "2",
            "stopping a navigation must not disable later navigations",
        )
        .await;
        assert_eq!(
            vm.eval("__stopFrame.contentWindow.location.pathname")
                .expect("resumed child URL should evaluate"),
            "/after-stop.html"
        );
        assert_eq!(
            server.finish_targets().await,
            ["/child.html", "/sibling.html", "/after-stop.html"],
            "{action} must not start the canceled request"
        );
    }
}

#[test]
fn window_stop_in_child_does_not_cancel_the_top_level_navigation() {
    let mut vm = new_storage_test_vm("https://window-stop.test/page.html");
    vm.eval(
        r#"
globalThis.__stopFrame = document.createElement('iframe');
__stopFrame.srcdoc = '<body>child</body>';
(document.body || document.documentElement || document).appendChild(__stopFrame);
"#,
    )
    .expect("child setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.eval(
        r#"
const child = __stopFrame.contentWindow;
const childResult = child.navigation.navigate('/child-next.html');
childResult.committed.catch(() => {});
childResult.finished.catch(() => {});
const topResult = navigation.navigate('/top-next.html');
topResult.committed.catch(() => {});
topResult.finished.catch(() => {});
child.stop();
"#,
    )
    .expect("stopping the child should leave the parent navigation alone");
    assert_eq!(
        vm.take_pending_location_navigation_with_seed()
            .expect("the top-level navigation must remain pending")
            .url
            .as_str(),
        "https://window-stop.test/top-next.html"
    );
}

#[tokio::test]
async fn window_stop_discards_in_flight_child_navigation_completions() {
    let (child_url, request_rx, release_tx, server) =
        spawn_gated_child_document_resource_server(200).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &child_url.replace("/child.html", "/page.html"),
        &loader,
    );
    vm.eval(
        r#"
globalThis.__stopFrame = document.createElement('iframe');
globalThis.__stopLoads = 0;
__stopFrame.onload = () => ++__stopLoads;
__stopFrame.srcdoc = '<body>original</body>';
(document.body || document.documentElement || document).appendChild(__stopFrame);
"#,
    )
    .expect("in-flight navigation child setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__stopLoads)",
        "1",
        "srcdoc should finish before starting the blocked response",
    )
    .await;
    vm.eval(&format!(
        "globalThis.__stopDocument = __stopFrame.contentDocument; __stopFrame.contentWindow.location.assign({child_url:?});"
    ))
    .expect("child navigation should queue");
    run_page_realm_prerequisite_then_expected_child_frame_semantic_turn(
        &mut vm,
        &loader,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "navigation commit task should start the child request",
    )
    .await;
    request_rx.await.expect("the child request should arrive");
    assert_eq!(
        vm.eval("__stopFrame.contentWindow.location.href")
            .expect("in-flight child URL should evaluate"),
        "about:srcdoc"
    );
    vm.eval("__stopFrame.contentWindow.stop()")
        .expect("in-flight child navigation should stop");
    release_tx.send(()).expect("release the canceled response");
    server.await.expect("the gated server should finish");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "late navigation completion should be discarded",
    )
    .await;
    assert_eq!(
        vm.eval(
            "[__stopFrame.contentDocument === __stopDocument, __stopFrame.contentWindow.location.href, __stopLoads].join('|')"
        )
        .expect("late response must not replace the stopped child"),
        "true|about:srcdoc|1"
    );
}

#[tokio::test]
async fn window_stop_preserves_child_cross_document_history_traversal() {
    let server = StaticHttpServer::spawn(3).await;
    let parent_url = server.url_for_host("window-stop-traversal.test", "/page.html");
    let loader = static_http_loader([server.resolve_entry("window-stop-traversal.test")]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(
        r#"
globalThis.__stopFrame = document.createElement('iframe');
globalThis.__stopLoads = 0;
__stopFrame.onload = () => ++__stopLoads;
__stopFrame.src = '/one.html';
(document.body || document.documentElement || document).appendChild(__stopFrame);
"#,
    )
    .expect("traversal child setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__stopLoads)",
        "1",
        "first child document should load",
    )
    .await;
    vm.eval("__stopFrame.contentWindow.location.assign('/two.html')")
        .expect("second child navigation should schedule");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__stopLoads)",
        "2",
        "second child document should load",
    )
    .await;
    vm.eval("__stopFrame.contentWindow.history.back()")
        .expect("child history traversal should schedule");
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("the history task should prepare the cross-document traversal")
    );
    vm.eval("__stopFrame.contentWindow.stop()")
        .expect("stop during traversal should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__stopLoads)",
        "3",
        "window.stop must not cancel a session history traversal",
    )
    .await;
    assert_eq!(
        vm.eval("__stopFrame.contentWindow.location.pathname")
            .expect("traversed URL should evaluate"),
        "/one.html"
    );
    assert_eq!(
        server.finish_targets().await,
        ["/one.html", "/two.html", "/one.html"]
    );
}


#[tokio::test]
async fn window_stop_cancels_pending_precommit_before_commit() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmStopPrecommitLog = [];
              navigation.onnavigate = event => {
                event.signal.addEventListener("abort", () => {
                  globalThis.__lmStopPrecommitLog.push(`abort:${event.signal.reason.name}:${location.search}`);
                });
                event.intercept({
                  precommitHandler: () => new Promise(() => {})
                });
              };
              navigation.onnavigateerror = () => {
                globalThis.__lmStopPrecommitLog.push(`error:${location.search}`);
              };
              navigation.onnavigatesuccess = () => {
                globalThis.__lmStopPrecommitLog.push("success");
              };
              const result = navigation.navigate("?blocked");
              result.committed.then(
                () => globalThis.__lmStopPrecommitLog.push("committed"),
                error => globalThis.__lmStopPrecommitLog.push(`committed-rejected:${error.name}`)
              );
              result.finished.then(
                () => globalThis.__lmStopPrecommitLog.push("finished"),
                error => globalThis.__lmStopPrecommitLog.push(`finished-rejected:${error.name}`)
              );
              window.stop();
              return `${location.search}:${globalThis.__lmStopPrecommitLog.join("|")}`;
            })()
            "##,
        )
        .expect("window.stop pending precommit setup should evaluate");

    assert_eq!(setup, ":abort:AbortError:|error:");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("window.stop pending precommit should drain");
    let settled = vm
        .eval("globalThis.__lmStopPrecommitLog.join('|')")
        .expect("window.stop pending precommit log should evaluate");
    assert_eq!(
        settled,
        "abort:AbortError:|error:|committed-rejected:AbortError|finished-rejected:AbortError"
    );
}

#[tokio::test]
async fn same_document_navigation_async_precommit_waits_to_commit() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmAsyncPrecommitLog = [];
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: () => new Promise(resolve => {
                    setTimeout(() => {
                      globalThis.__lmAsyncPrecommitLog.push(`precommit:${location.hash}`);
                      resolve();
                    }, 0);
                  }),
                  handler: () => {
                    globalThis.__lmAsyncPrecommitLog.push(`handler:${location.hash}`);
                  }
                });
              };
              const result = navigation.navigate("#one");
              result.committed.then(() => globalThis.__lmAsyncPrecommitLog.push(`committed:${location.hash}`));
              result.finished.then(() => globalThis.__lmAsyncPrecommitLog.push(`finished:${location.hash}`));
              return `${location.hash}:${globalThis.__lmAsyncPrecommitLog.join("|")}`;
            })()
            "##,
        )
        .expect("async precommit setup should evaluate");

    assert_eq!(setup, ":");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("async precommit timer should drain");
    let after_timeout = vm
        .eval("`${location.hash}:${globalThis.__lmAsyncPrecommitLog.join('|')}`")
        .expect("async precommit log should evaluate");

    assert_eq!(
        after_timeout,
        "#one:precommit:|handler:#one|committed:#one|finished:#one"
    );
}

#[tokio::test]
async fn same_document_navigation_async_precommit_reject_blocks_commit() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmAsyncPrecommitRejectLog = [];
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: () => new Promise((_, reject) => {
                    setTimeout(() => reject(new Error("blocked")), 0);
                  }),
                  handler: () => {
                    globalThis.__lmAsyncPrecommitRejectLog.push("handler");
                  }
                });
              };
              navigation.onnavigateerror = () => {
                globalThis.__lmAsyncPrecommitRejectLog.push(`error:${location.hash}`);
              };
              const result = navigation.navigate("#one");
              result.committed.catch(error => globalThis.__lmAsyncPrecommitRejectLog.push(`committed:${error.message}:${location.hash}`));
              result.finished.catch(error => globalThis.__lmAsyncPrecommitRejectLog.push(`finished:${error.message}:${location.hash}`));
              return `${location.hash}:${globalThis.__lmAsyncPrecommitRejectLog.join("|")}`;
            })()
            "##,
        )
        .expect("async precommit rejection setup should evaluate");

    assert_eq!(setup, ":");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("async precommit rejection timer should drain");
    let after_timeout = vm
        .eval("`${location.hash}:${globalThis.__lmAsyncPrecommitRejectLog.join('|')}`")
        .expect("async precommit rejection log should evaluate");

    assert_eq!(
        after_timeout,
        ":error:|committed:blocked:|finished:blocked:"
    );
}

#[tokio::test]
async fn same_document_navigation_finished_resolves_after_microtask() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let before_microtask_checkpoint = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmNavigationFinishedOrder = [];
              navigation.onnavigatesuccess = () => {
                globalThis.__lmNavigationFinishedOrder.push(`success:${location.hash}`);
              };
              const result = navigation.navigate("#one");
              result.committed.then(() => globalThis.__lmNavigationFinishedOrder.push("committed"));
              result.finished.then(() => globalThis.__lmNavigationFinishedOrder.push("finished"));
              Promise.resolve().then(() => globalThis.__lmNavigationFinishedOrder.push("microtask"));
              return globalThis.__lmNavigationFinishedOrder.join("|");
            })()
            "##,
        )
        .expect("same-document navigation ordering setup should evaluate");

    assert_eq!(before_microtask_checkpoint, "");

    let after_microtasks = vm
        .eval("globalThis.__lmNavigationFinishedOrder.join('|')")
        .expect("same-document navigation ordering log should evaluate after microtasks");

    assert_eq!(
        after_microtasks,
        "success:#one|committed|microtask|finished"
    );
}

#[tokio::test]
async fn closed_navigation_source_rejects_task_finished_without_timer_fallback() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    vm.eval(
        r#"
globalThis.__lmClosedNavigationTaskRoute = [];
navigation.navigate("/next-document");
"queued"
"#,
    )
    .expect("cross-document navigation should run its unload lifecycle step");
    assert!(
        !vm.has_ready_timeout(),
        "cross-document unload lifecycle must not leave a PageTimer before route retirement"
    );

    drop(
        vm._page_task_residence_for_executor_test
            .take()
            .expect("Navigation API route-retirement fixture should own one production consumer"),
    );
    vm.eval(
        r##"
navigation.onnavigatesuccess = () => __lmClosedNavigationTaskRoute.push("success");
navigation.onnavigateerror = event => {
  __lmClosedNavigationTaskRoute.push("error:" + event.error.name);
};
const result = navigation.navigate("#replacement");
result.committed.then(
  () => __lmClosedNavigationTaskRoute.push("committed"),
  error => __lmClosedNavigationTaskRoute.push("committed-error:" + error.name),
);
result.finished.then(
  () => __lmClosedNavigationTaskRoute.push("finished"),
  error => __lmClosedNavigationTaskRoute.push("finished-error:" + error.name),
);
"queued"
"##,
    )
    .expect("closed Navigation API route should reject instead of falling back");

    assert_eq!(
        vm.eval("globalThis.__lmClosedNavigationTaskRoute.join('|')")
            .expect("closed Navigation API route settlement should be observable"),
        "error:AbortError|error:AbortError|committed|finished-error:AbortError",
        "the retired cross-document attempt and the rejected replacement attempt should each dispatch one navigateerror"
    );
    assert!(
        !vm.has_ready_timeout(),
        "a closed navigation-and-traversal source must not recreate the removed timer transport"
    );
}

#[test]
fn same_document_intercept_rejects_finished_and_dispatches_navigateerror() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let setup = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const err = new Error("boom");
              navigation.onnavigatesuccess = () => log.push("success");
              navigation.onnavigateerror = event => {
                log.push("error:" + String(event.error === err));
              };
              navigation.onnavigate = event => {
                event.intercept({ handler: () => Promise.reject(err) });
              };
              const result = navigation.navigate("#one");
              result.committed.then(() => log.push("committed"));
              result.finished.then(
                () => log.push("finished:fulfilled"),
                error => log.push("finished:" + String(error === err))
              );
              Promise.resolve().then(() => log.push("microtask"));
              globalThis.__lmInterceptRejectLog = log;
              return log.join("|");
            })()
            "##,
        )
        .expect("intercept reject setup should evaluate");
    assert_eq!(setup, "");

    let after_microtasks = vm
        .eval("globalThis.__lmInterceptRejectLog.join('|')")
        .expect("intercept reject microtasks should evaluate");
    assert_eq!(
        after_microtasks,
        "error:true|committed|microtask|finished:true"
    );
}

#[test]
fn same_document_multiple_intercept_handlers_reject_if_any_handler_rejects() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let setup = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const err = new TypeError("sentinel");
              navigation.onnavigatesuccess = () => log.push("success");
              navigation.onnavigateerror = event => {
                log.push("error:" + String(event.error === err));
              };
              navigation.onnavigate = event => {
                event.intercept();
                event.intercept({ handler: () => Promise.reject(err) });
                event.intercept({ handler: () => Promise.resolve("ignored") });
              };
              const result = navigation.navigate("#one");
              result.finished.then(
                () => log.push("finished:fulfilled"),
                error => log.push("finished:" + String(error === err))
              );
              globalThis.__lmMultipleInterceptRejectLog = log;
              return log.join("|");
            })()
            "##,
        )
        .expect("multiple intercept rejection setup should evaluate");
    assert_eq!(setup, "");

    let after_microtasks = vm
        .eval("globalThis.__lmMultipleInterceptRejectLog.join('|')")
        .expect("multiple intercept rejection microtasks should evaluate");
    assert_eq!(after_microtasks, "error:true|finished:true");
}

#[test]
fn navigation_reload_intercept_rejects_finished_and_dispatches_navigateerror() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let setup = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const err = new Error("reload blocked");
              const from = navigation.currentEntry;
              navigation.oncurrententrychange = event => {
                log.push(`change:${event.navigationType}:${event.from === from}`);
              };
              navigation.onnavigatesuccess = () => log.push("success");
              navigation.onnavigateerror = event => {
                log.push("error:" + String(event.error === err));
              };
              navigation.onnavigate = event => {
                log.push(`navigate:${event.navigationType}`);
                event.intercept({ handler: () => Promise.reject(err) });
              };
              const result = navigation.reload();
              result.committed.then(
                value => log.push("committed:" + String(value === navigation.currentEntry)),
                () => log.push("committed:rejected")
              );
              result.finished.then(
                () => log.push("finished:fulfilled"),
                error => log.push("finished:" + String(error === err))
              );
              Promise.resolve().then(() => log.push("microtask"));
              globalThis.__lmReloadInterceptRejectLog = log;
              return log.join("|");
            })()
            "##,
        )
        .expect("reload intercept rejection setup should evaluate");
    assert_eq!(setup, "navigate:reload|change:reload:true");

    let after_microtasks = vm
        .eval("globalThis.__lmReloadInterceptRejectLog.join('|')")
        .expect("reload intercept rejection microtasks should evaluate");
    assert_eq!(
        after_microtasks,
        "navigate:reload|change:reload:true|error:true|committed:true|microtask|finished:true"
    );
}

#[test]
fn navigation_reload_intercept_exposes_transition_during_events() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let result = vm
        .eval(
            r##"
            (() => {
              const from = navigation.currentEntry;
              globalThis.__lmReloadTransitionLog = [];
              const record = name => globalThis.__lmReloadTransitionLog.push({
                name,
                transitionObject: navigation.transition !== null,
                transitionBrand: navigation.transition instanceof NavigationTransition,
                fromMatches: navigation.transition?.from === from,
                navigationType: navigation.transition?.navigationType ?? null
              });
              navigation.addEventListener("navigate", () => record("navigate"));
              navigation.addEventListener("currententrychange", () => record("currententrychange"));
              navigation.addEventListener("navigatesuccess", () => record("navigatesuccess"));
              navigation.onnavigate = event => event.intercept({
                handler() { record("handler"); }
              });
              const reloadResult = navigation.reload();
              return JSON.stringify({
                log: globalThis.__lmReloadTransitionLog,
                transitionAfterReload: navigation.transition !== null,
                committedPromise: typeof reloadResult.committed.then === "function",
                finishedPromise: typeof reloadResult.finished.then === "function"
              });
            })()
            "##,
        )
        .expect("reload intercepted transition probe should evaluate");

    assert_eq!(
        result,
        r##"{"log":[{"name":"navigate","transitionObject":false,"transitionBrand":false,"fromMatches":false,"navigationType":null},{"name":"currententrychange","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"},{"name":"handler","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"}],"transitionAfterReload":true,"committedPromise":true,"finishedPromise":true}"##
    );
    let after_microtask = vm
        .eval(
            r##"JSON.stringify({
              log: globalThis.__lmReloadTransitionLog,
              transitionAfterReload: navigation.transition
            })"##,
        )
        .expect("reload intercepted transition microtask probe should evaluate");
    assert_eq!(
        after_microtask,
        r##"{"log":[{"name":"navigate","transitionObject":false,"transitionBrand":false,"fromMatches":false,"navigationType":null},{"name":"currententrychange","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"},{"name":"handler","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"},{"name":"navigatesuccess","transitionObject":true,"transitionBrand":true,"fromMatches":true,"navigationType":"reload"}],"transitionAfterReload":null}"##
    );
}

#[test]
fn navigation_state_clone_rejects_dom_nodes_before_document_state() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    vm.eval(
        r##"
        (() => {
          globalThis.__lmNavigationStateCloneLog = [];
          const record = (label, result) => {
            result.committed.then(
              () => __lmNavigationStateCloneLog.push(`${label}:committed:fulfilled`),
              error => __lmNavigationStateCloneLog.push(`${label}:committed:${error && error.name}`)
            );
            result.finished.then(
              () => __lmNavigationStateCloneLog.push(`${label}:finished:fulfilled`),
              error => __lmNavigationStateCloneLog.push(`${label}:finished:${error && error.name}`)
            );
          };
          const stateNode = document.createElement("div");

          record("active-navigate", navigation.navigate("?node-state", { state: stateNode }));
          record("active-reload", navigation.reload({ state: stateNode }));

          const host = document.documentElement || document.appendChild(document.createElement("html"));
          const frame = document.createElement("iframe");
          host.appendChild(frame);
          const child = frame.contentWindow;
          frame.remove();
          record("detached-navigate", child.navigation.navigate("/next", { state: stateNode }));
          record("detached-reload", child.navigation.reload({ state: stateNode }));
          return "queued";
        })()
        "##,
    )
    .expect("navigation state clone rejection setup should evaluate");

    let result = vm
        .eval("globalThis.__lmNavigationStateCloneLog.join('|')")
        .expect("navigation state clone rejection log should evaluate");

    assert_eq!(
        result,
        "active-navigate:committed:DataCloneError|active-navigate:finished:DataCloneError|active-reload:committed:DataCloneError|active-reload:finished:DataCloneError|detached-navigate:committed:DataCloneError|detached-navigate:finished:DataCloneError|detached-reload:committed:DataCloneError|detached-reload:finished:DataCloneError"
    );
}

#[tokio::test]
async fn navigation_reload_async_precommit_reject_blocks_commit() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base", &loader);

    let setup = vm
        .eval(
            r##"
            (() => {
              globalThis.__lmReloadPrecommitRejectLog = [];
              navigation.onnavigate = event => {
                event.intercept({
                  precommitHandler: () => new Promise((_, reject) => {
                    setTimeout(() => reject(new Error("reload blocked")), 0);
                  }),
                  handler: () => {
                    globalThis.__lmReloadPrecommitRejectLog.push("handler");
                  }
                });
              };
              navigation.oncurrententrychange = () => {
                globalThis.__lmReloadPrecommitRejectLog.push("change");
              };
              navigation.onnavigateerror = () => {
                globalThis.__lmReloadPrecommitRejectLog.push("error");
              };
              const result = navigation.reload();
              result.committed.then(
                () => globalThis.__lmReloadPrecommitRejectLog.push("committed:fulfilled"),
                error => globalThis.__lmReloadPrecommitRejectLog.push(`committed:${error.message}`)
              );
              result.finished.then(
                () => globalThis.__lmReloadPrecommitRejectLog.push("finished:fulfilled"),
                error => globalThis.__lmReloadPrecommitRejectLog.push(`finished:${error.message}`)
              );
              return globalThis.__lmReloadPrecommitRejectLog.join("|");
            })()
            "##,
        )
        .expect("reload async precommit rejection setup should evaluate");

    assert_eq!(setup, "");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("reload async precommit rejection timer should drain");
    let after_timeout = vm
        .eval("globalThis.__lmReloadPrecommitRejectLog.join('|')")
        .expect("reload async precommit rejection log should evaluate");

    assert_eq!(
        after_timeout,
        "error|committed:reload blocked|finished:reload blocked"
    );
}

#[test]
fn same_document_location_intercept_reject_dispatches_navigateerror() {
    let mut vm = new_storage_test_vm("https://example.com/base");

    let setup = vm
        .eval(
            r##"
            (() => {
              const log = [];
              const err = new Error("boom");
              navigation.onnavigatesuccess = () => log.push("success");
              navigation.onnavigateerror = event => {
                log.push("error:" + String(event.error === err) + ":" + location.hash);
              };
              navigation.onnavigate = event => {
                event.intercept({ handler: () => Promise.reject(err) });
              };
              location.href = "#one";
              Promise.resolve().then(() => log.push("microtask"));
              globalThis.__lmLocationInterceptRejectLog = log;
              return log.join("|");
            })()
            "##,
        )
        .expect("location intercept reject setup should evaluate");
    assert_eq!(setup, "");

    let after_microtasks = vm
        .eval("globalThis.__lmLocationInterceptRejectLog.join('|')")
        .expect("location intercept reject microtasks should evaluate");
    assert_eq!(after_microtasks, "error:true:#one|microtask");
}

#[test]
fn same_url_navigation_classifies_fragments_before_interception() {
    for (fragment, remove_fragment, same_document) in [
        ("", false, false),
        ("#", false, true),
        ("#fragment", false, true),
        ("#fragment", true, false),
    ] {
        for history in ["auto", "push", "replace"] {
            for action in ["navigate", "intercept", "cancel"] {
                let mut vm = new_storage_test_vm(&format!(
                    "https://same-url-navigation.test/page{fragment}"
                ));
                vm.exec(
                    &format!(
                        r#"
const beforeLength = history.length;
const events = [], settled = [];
navigation.addEventListener('navigate', event => {{
  events.push([event.navigationType, event.destination.sameDocument, event.hashChange]);
  if ({action:?} === 'intercept') event.intercept();
  if ({action:?} === 'cancel') event.preventDefault();
}});
const destination = {remove_fragment} ? location.href.split('#')[0] : location.href;
const result = navigation.navigate(destination, {{history: {history:?}}});
for (const name of ['committed', 'finished'])
  result[name].then(() => settled.push(name), error => settled.push(name + ':' + error.name));
"#
                    ),
                    None,
                )
                .unwrap();
                let context = format!("{fragment}/{remove_fragment}/{history}/{action}");
                let pushes = history == "push" || (history == "auto" && remove_fragment);
                let navigation_type = if pushes { "push" } else { "replace" };
                let pending = vm.take_pending_location_navigation_with_seed();
                assert_eq!(
                    pending.is_some(),
                    !same_document && action == "navigate",
                    "{context}"
                );
                if let Some(pending) = pending {
                    assert_eq!(
                        pending.url.as_str(),
                        "https://same-url-navigation.test/page"
                    );
                    assert_eq!(
                        pending
                            .entry_seed
                            .unwrap()
                            .activation
                            .unwrap()
                            .navigation_type
                            .as_deref(),
                        Some(navigation_type),
                        "{context}"
                    );
                }
                let result = vm
                    .eval("JSON.stringify({events, settled, delta: history.length - beforeLength})")
                    .unwrap();
                let result: serde_json::Value = serde_json::from_str(&result).unwrap();
                let committed = action != "cancel" && (same_document || action == "intercept");
                let settled = if action == "cancel" {
                    serde_json::json!(["committed:AbortError", "finished:AbortError"])
                } else if committed {
                    serde_json::json!(["committed", "finished"])
                } else {
                    serde_json::json!([])
                };
                assert_eq!(
                    result,
                    serde_json::json!({
                        "events": [[navigation_type, same_document, false]],
                        "settled": settled,
                        "delta": i32::from(committed && pushes),
                    }),
                    "{context}"
                );
            }
        }
    }
}

#[tokio::test]
async fn same_url_navigation_preserves_fragment_documents_in_iframes_and_popups() {
    for popup in [false, true] {
        for (fragment, remove_fragment, same_document) in [
            ("", false, false),
            ("#", false, true),
            ("#fragment", false, true),
            ("#fragment", true, false),
        ] {
            for history in ["auto", "push", "replace"] {
                let request_count = if same_document { 1 } else { 2 };
                let server = StaticHttpServer::spawn(request_count).await;
                let parent = server.base_url().join("parent").unwrap();
                let loader = static_http_loader([]);
                let mut vm =
                    new_storage_page_task_executor_test_vm_with_loader(parent.as_str(), &loader);
                let context = format!("popup={popup}/{fragment}/{remove_fragment}/{history}");
                vm.eval(&format!(
                    r#"
globalThis.child = null;
if ({popup}) {{
  child = open('/child', 'same-url-target');
}} else {{
  const frame = document.createElement('iframe');
  frame.src = '/child'; document.body.append(frame); child = frame.contentWindow;
}}
"#
                ))
                .unwrap();
                advance_page_task_executor_until_eval_equals(
                    &mut vm, &loader,
                    "(() => { try { return String(child.location.pathname === '/child' && child.document.readyState === 'complete'); } catch { return 'false'; } })()",
                    "true", &context,
                ).await;
                vm.exec(
                    &format!(
                        r#"
child.history.replaceState(null, '', child.location.pathname + {fragment:?});
const originalDocument = child.document;
const beforeLength = child.history.length;
const beforeEntries = child.navigation.entries().length;
const beforeIndex = child.navigation.currentEntry.index;
const events = [], settled = [];
child.navigation.addEventListener('navigate', event =>
  events.push([event.navigationType, event.destination.sameDocument, event.hashChange]));
const destination = {remove_fragment} ? child.location.href.split('#')[0] : child.location.href;
const result = child.navigation.navigate(destination, {{history: {history:?}}});
for (const name of ['committed', 'finished'])
  result[name].then(() => settled.push(name), error => settled.push(error.name));
"#
                    ),
                    None,
                )
                .unwrap();
                advance_page_task_executor_until_eval_equals(
                    &mut vm, &loader,
                    if same_document {
                        "String(settled.length === 2)"
                    } else {
                        "(() => { try { return String(child.document !== originalDocument && child.document.readyState === 'complete'); } catch { return 'false'; } })()"
                    },
                    "true", &context,
                ).await;
                let result = vm
                    .eval(
                        r#"JSON.stringify({
sameDocument: originalDocument === child.document,
historyDelta: child.history.length - beforeLength,
entriesDelta: child.navigation.entries().length - beforeEntries,
indexDelta: child.navigation.currentEntry.index - beforeIndex,
events, settled})"#,
                    )
                    .unwrap();
                let result: serde_json::Value = serde_json::from_str(&result).unwrap();
                let pushes = history == "push" || (history == "auto" && remove_fragment);
                assert_eq!(
                    result,
                    serde_json::json!({
                        "sameDocument": same_document,
                        "historyDelta": i32::from(pushes),
                        "entriesDelta": i32::from(pushes),
                        "indexDelta": i32::from(pushes),
                        "events": [[if pushes { "push" } else { "replace" }, same_document, false]],
                        "settled": if same_document { vec!["committed", "finished"] } else { vec![] },
                    }),
                    "{context}"
                );
                if popup {
                    vm.eval("child.close()").unwrap();
                }
                assert_eq!(
                    server.finish_targets().await,
                    vec!["/child"; request_count],
                    "{context}"
                );
            }
        }
    }
}
