use super::*;

#[test]
fn page_reveal_event_converts_typed_dictionary_and_guards_its_getter() {
    let mut vm = new_storage_html_test_vm("https://page-reveal-event.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/page-reveal-event.js"
        ))
        .unwrap(),
        "true"
    );
}

#[tokio::test]
async fn page_reveal_precedes_animation_snapshot_and_survives_public_constructor_replacement() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        "https://page-reveal-order.test/",
        &loader,
    );
    page.eval(
        r#"
        globalThis.revealLog = [];
        const RevealEvent = PageRevealEvent;
        onpagereveal = event => {
          revealLog.push([
            'reveal', event instanceof RevealEvent, event.isTrusted,
            event.target === window, event.currentTarget === window,
            event.viewTransition === null, event.bubbles, event.cancelable, event.composed
          ]);
          requestAnimationFrame(() => revealLog.push('event-rAF'));
        };
        requestAnimationFrame(() => revealLog.push('first-rAF'));
        window.PageRevealEvent = function() { throw new Error('author constructor'); };
        "#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(revealLog.length)",
        "3",
        "Document reveal before animation callbacks",
    )
    .await;
    assert_eq!(
        page.eval("JSON.stringify(revealLog)").unwrap(),
        r#"[["reveal",true,true,true,true,true,false,false,false],"first-rAF","event-rAF"]"#
    );
}

#[tokio::test]
async fn page_reveal_document_open_retires_the_turn_and_preserves_window_callbacks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        "https://page-reveal-open.test/",
        &loader,
    );
    page.eval(
        r#"
        globalThis.revealLog = [];
        onpagereveal = () => {
          revealLog.push('reveal');
          document.open(); document.write('<!doctype html><body>replacement'); document.close();
          onpagereveal = () => revealLog.push('duplicate');
          requestAnimationFrame(() => revealLog.push('new-rAF'));
        };
        requestAnimationFrame(() => revealLog.push('retained-rAF'));
        "#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(revealLog.includes('new-rAF'))",
        "true",
        "Document.open from reveal listener",
    )
    .await;
    assert_eq!(
        page.eval("JSON.stringify(revealLog)").unwrap(),
        r#"["reveal","retained-rAF","new-rAF"]"#
    );
}

#[tokio::test]
async fn page_reveal_revalidates_removed_child_before_its_animation_callbacks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        "https://page-reveal-child.test/",
        &loader,
    );
    page.eval(
        r#"
        document.open(); document.write('<!doctype html><body><iframe></iframe>'); document.close();
        globalThis.childLog = [];
        const frame = document.querySelector('iframe');
        const child = frame.contentWindow;
        child.onpagereveal = event => {
          childLog.push(event.target === child && event instanceof child.PageRevealEvent);
          frame.remove();
        };
        child.requestAnimationFrame(() => childLog.push('stale-rAF'));
        globalThis.frameDone = false;
        requestAnimationFrame(() => requestAnimationFrame(() => { frameDone = true; }));
        "#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(frameDone)",
        "true",
        "Removed child reveal owner",
    )
    .await;
    assert_eq!(page.eval("JSON.stringify(childLog)").unwrap(), "[true]");
}

#[tokio::test]
async fn page_reveal_orders_nested_documents_before_animation_callbacks() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        "https://page-reveal-tree.test/",
        &loader,
    );
    page.eval(
        r#"
        document.body.innerHTML = '<iframe id=left></iframe><iframe id=right></iframe>';
        const left = document.getElementById('left').contentWindow;
        const right = document.getElementById('right').contentWindow;
        left.document.body.innerHTML = '<iframe id=nested></iframe>';
        const nested = left.document.getElementById('nested').contentWindow;
        globalThis.treeLog = [];
        globalThis.frameLogs = [];
        globalThis.revealOwners = [];
        const views = [[window, 'top'], [left, 'left'], [nested, 'nested'], [right, 'right']];
        for (const [view, name] of views) {
          const RevealEvent = view.PageRevealEvent;
          view.onpagereveal = event => {
            treeLog.push(name);
            revealOwners.push(event instanceof RevealEvent && event.isTrusted &&
                event.target === view && event.currentTarget === view &&
                event.viewTransition === null);
          };
        }
        for (const [view, name] of [...views].reverse()) {
          view.requestAnimationFrame(() => frameLogs.push(name + ':' + treeLog.join(',')));
        }
        'installed'
        "#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(frameLogs.length)",
        "4",
        "All eligible Documents reveal before animation callbacks",
    )
    .await;
    assert_eq!(
        page.eval("treeLog.join(',')").unwrap(),
        "top,left,nested,right"
    );
    assert_eq!(
        page.eval("String(frameLogs.every(log => log.endsWith(':top,left,nested,right')))")
            .unwrap(),
        "true",
    );
    assert_eq!(
        page.eval("JSON.stringify(revealOwners)").unwrap(),
        "[true,true,true,true]"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn page_reveal_watchdog_bounds_listeners_and_their_microtasks() {
    use crate::v8_execution_watchdog::{V8ExecutionWatchdog, V8ExecutionWatchdogKind};
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for runaway in [
        "while (true) {}",
        "queueMicrotask(() => { while (true) {} })",
    ] {
        let mut page = new_storage_page_task_executor_test_vm_with_loader(
            "https://page-reveal-watchdog.test/",
            &loader,
        );
        page.eval(&format!(
            r#"
            globalThis.revealCalls = 0;
            globalThis.frameDone = false;
            onpagereveal = () => {{ revealCalls++; {runaway} }};
            requestAnimationFrame(() => {{ frameDone = true; }});
            'installed'
        "#
        ))
        .unwrap();
        let _budget = V8ExecutionWatchdog::override_timeout_for_test(
            V8ExecutionWatchdogKind::LifecycleEvent,
            std::time::Duration::from_millis(100),
        );
        advance_page_task_executor_until_eval_equals(
            &mut page,
            &loader,
            "String(frameDone)",
            "true",
            "A runaway reveal listener must return control to rendering",
        )
        .await;
        assert_eq!(
            page.eval("[revealCalls, frameDone, 6 * 7].join('|')")
                .unwrap(),
            "1|true|42",
            "{runaway}"
        );
    }
}
