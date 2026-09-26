//! Behavioral regression tests for the dom-heavy / dom-* synthetic benchmark
//! cases (see moli-benchmark/synthetic_case_groups/basic.py).
//!
//! The Python cases live in the benchmark harness and are timed; these tests
//! exercise the same code paths inside the renderer-v8 unit-test VM so that
//! `cargo nextest run` catches behavioral regressions in the hot paths we've
//! optimized (mark_subtree_tree_scope alloc-free walk, the no-MO mutation
//! record gate, the no-shadow slot short-circuit, the iframe-subtree
//! shortcut, the css-state sync gate, etc.) without needing to invoke the
//! external benchmark binary. Iteration counts are scaled down from the perf
//! cases so each test runs in well under a second.

use super::ScriptVm;
use super::ScriptVmDefaultWorldBootstrap;
use super::StandaloneScriptVmHarness;
use crate::dom::native::{DomHost, NativeDom};

fn new_dom_regression_vm() -> StandaloneScriptVmHarness {
    let _js_runtime = crate::JsRuntime::initialize();
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
    let page_runtime_task_source = page_task_queue.residence();
    ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(NativeDom::new(
            url::Url::parse("https://dom-heavy-regression.test/").expect("test url"),
        )),
        post_domcontentloaded_page_task_sender,
        page_task_front_injection_tx,
    )
    .expect("script vm bootstrap should succeed")
    .finish()
    .map(|mut vm| {
        vm.install_page_task_residence_for_executor_test(page_runtime_task_source);
        vm
    })
    .expect("script vm finish should succeed")
}

/// Wraps the case script with the same DOM scaffolding the synthetic
/// benchmark fixture provides (`<html><body><main></main></body></html>`) and
/// returns the value the case writes into `document.body.dataset.benchmarkStatus`.
fn run_dom_case(case_script: &str) -> String {
    let mut vm = new_dom_regression_vm();
    let script = format!(
        r#"
        (() => {{
            if (!document.documentElement) {{
                document.appendChild(document.createElement('html'));
            }}
            if (!document.body) {{
                document.documentElement.appendChild(document.createElement('body'));
            }}
            const __main = document.createElement('main');
            document.body.appendChild(__main);
            {case_script}
            return document.body.dataset.benchmarkStatus;
        }})()
        "#
    );
    vm.eval(&script).expect("case script should evaluate")
}

#[test]
fn dom_heavy_appends_2000_buttons_with_text_and_dataset() {
    // Mirrors the dom-heavy benchmark fixture: 2000 button appends with
    // textContent + dataset.index, exercising the live-ranges short-circuit
    // and the connected-script subtree skip for leaves.
    let status = run_dom_case(
        r#"
        const root = document.querySelector('main');
        const COUNT = 500;
        for (let i = 0; i < COUNT; i++) {
            const item = document.createElement('button');
            item.textContent = 'item-' + i;
            item.dataset.index = String(i);
            root.appendChild(item);
        }
        document.body.dataset.benchmarkStatus =
            root.children.length === COUNT ? 'ok' : 'bad';
        "#,
    );
    assert_eq!(status, "ok");
}

#[test]
fn live_html_collection_for_in_does_not_abort_v8() {
    // V8 expects indexed interceptor enumerators to return integer keys.
    // Returning string keys here trips a fatal Object::ToUint32 check during for-in.
    let status = run_dom_case(
        r#"
        const root = document.querySelector('main');
        root.appendChild(document.createElement('span'));
        root.appendChild(document.createElement('span'));
        const keys = [];
        for (const key in root.getElementsByTagName('span')) {
            keys.push(key);
        }
        document.body.dataset.benchmarkStatus =
            keys.includes('0') && keys.includes('1') ? 'ok' : keys.join(',');
        "#,
    );
    assert_eq!(status, "ok");
}

#[test]
fn dom_subtree_mark_thrash_preserves_descendant_count_across_detach_cycles() {
    // Mirrors the dom-subtree-mark-thrash benchmark: detach + reattach a
    // ~300-node subtree repeatedly, asserting the tree-scope marker walk
    // leaves descendant counts intact.
    let status = run_dom_case(
        r#"
        const root = document.querySelector('main');
        const subtree = document.createElement('section');
        subtree.id = 'subtree-payload';
        for (let i = 0; i < 60; i++) {
            const row = document.createElement('div');
            for (let j = 0; j < 4; j++) {
                const cell = document.createElement('span');
                cell.appendChild(document.createTextNode('c'));
                row.appendChild(cell);
            }
            subtree.appendChild(row);
        }
        root.appendChild(subtree);
        const expectedDescendants = subtree.getElementsByTagName('*').length;
        for (let i = 0; i < 50; i++) {
            root.removeChild(subtree);
            root.appendChild(subtree);
        }
        const finalDescendants = subtree.getElementsByTagName('*').length;
        document.body.dataset.benchmarkStatus =
            subtree.parentNode === root && finalDescendants === expectedDescendants
                ? 'ok' : 'bad';
        "#,
    );
    assert_eq!(status, "ok");
}

#[test]
fn dom_append_leaf_flood_appends_bare_spans_in_order() {
    // Mirrors dom-append-leaf-flood: validates the no-MO mutation record
    // gate, the iframe-subtree shortcut, the slot short-circuit, the
    // insertion-roots stack-slice path, and the css-state sync gate all
    // preserve correctness when the inserted node is a bare leaf.
    let status = run_dom_case(
        r#"
        const root = document.querySelector('main');
        const COUNT = 1000;
        for (let i = 0; i < COUNT; i++) {
            root.appendChild(document.createElement('span'));
        }
        document.body.dataset.benchmarkStatus =
            root.children.length === COUNT ? 'ok' : 'bad';
        "#,
    );
    assert_eq!(status, "ok");
}

#[test]
fn dom_attribute_flood_round_trips_last_setattribute_value() {
    // Mirrors dom-attribute-flood: validates set_attribute_effects round
    // trips through getAttribute after a long run of mutations on the same
    // element. The DOM mutation owner captures the old value once and shares
    // it with typed style/observer payloads when those consumers are present.
    let status = run_dom_case(
        r#"
        const root = document.querySelector('main');
        const node = document.createElement('div');
        root.appendChild(node);
        const COUNT = 2000;
        for (let i = 0; i < COUNT; i++) {
            node.setAttribute('data-x', String(i));
        }
        document.body.dataset.benchmarkStatus =
            node.getAttribute('data-x') === String(COUNT - 1) ? 'ok' : 'bad';
        "#,
    );
    assert_eq!(status, "ok");
}

#[test]
fn live_range_offsets_shift_by_inserted_count_for_document_fragment_insert() {
    // Regression for the fragment-insertion live-range offset adjustment:
    // inserting a DocumentFragment hoists all N children into the parent,
    // so live-range offsets that sit *past* the insertion point each need to
    // shift by N — one bump per inserted child. The prior mutation pipeline
    // fired a single +1 regardless of fragment size, leaving ranges
    // pointing into the inserted children instead of the original siblings
    // they used to bracket.
    let mut vm = new_dom_regression_vm();
    let result = vm
        .eval(
            r#"
            (() => {
                if (!document.documentElement) {
                    document.appendChild(document.createElement('html'));
                }
                if (!document.body) {
                    document.documentElement.appendChild(document.createElement('body'));
                }
                const parent = document.createElement('div');
                document.body.appendChild(parent);
                const a = document.createElement('span'); a.id = 'a';
                const b = document.createElement('span'); b.id = 'b';
                const c = document.createElement('span'); c.id = 'c';
                parent.appendChild(a);
                parent.appendChild(b);
                parent.appendChild(c);
                // Live range that brackets exactly child `c` (offsets 2..3).
                // We intentionally pick a range whose start is strictly past
                // the upcoming insertion point so that *both* endpoints have
                // to shift by the fragment child count.
                const range = document.createRange();
                range.setStart(parent, 2);
                range.setEnd(parent, 3);
                // Hoist 3 new children before `b` (at index 1) via a
                // DocumentFragment.
                const frag = document.createDocumentFragment();
                frag.appendChild(document.createElement('em'));
                frag.appendChild(document.createElement('em'));
                frag.appendChild(document.createElement('em'));
                parent.insertBefore(frag, b);
                // After: parent.children = [a, em, em, em, b, c]. The range
                // should still bracket `c`, so its offsets are 5..6.
                return [
                    range.startOffset,
                    range.endOffset,
                    parent.children[range.startOffset].id,
                ].join('|');
            })()
            "#,
        )
        .expect("fragment insertion should preserve live range bracketing");
    assert_eq!(result, "5|6|c");
}

/// Bootstraps `<html><body>` on the VM and then runs the caller-provided
/// JS expression, returning whatever it evaluates to. Used by the
/// Synchronous setup and state reads. Rendering-dependent tests explicitly
/// use `eval_with_rendering` to complete production Page tasks.
fn eval_with_body(vm: &mut ScriptVm, expr: &str) -> String {
    let script = format!(
        r#"
        (() => {{
            if (!document.documentElement) {{
                document.appendChild(document.createElement('html'));
            }}
            if (!document.body) {{
                document.documentElement.appendChild(document.createElement('body'));
            }}
            return {expr};
        }})()
        "#
    );
    vm.eval(&script).expect("expression should evaluate")
}

fn new_observer_regression_vm() -> crate::runtime::PageVmTaskExecutorTestHarness {
    let loader =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    crate::runtime::PageVmTaskExecutorTestHarness::new(
        url::Url::parse("https://dom-heavy-regression.test/").unwrap(),
        &loader,
    )
}

/// Complete a rendering opportunity and its subsequent observer delivery task
/// using the same dispatcher as production, between geometry-test phases.
async fn eval_with_rendering(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    expr: &str,
) -> String {
    let result = eval_with_body(vm, expr);
    let loader =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("render observer scene");
    result
}

#[tokio::test(flavor = "current_thread")]
async fn intersection_observer_callback_fires_after_dom_mutation_without_mutation_observer() {
    // Regression for the queue_intersection_checks → records.is_empty()
    // implicit invariant break: with only an IntersectionObserver registered
    // (no MutationObserver), the mutation_records_enabled flag suppresses
    // record allocation, which used to mask the "DOM mutated" signal that
    // queue_mutation_records relied on to schedule intersection checks.
    //
    // Rendering samples the final state of each script, then a separate Page
    // task delivers changes. Cover both a transient detach/reattach and the
    // same mutations separated by actual rendering opportunities.
    let mut vm = new_observer_regression_vm();
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoCallbacks = 0;
            window.__lmIoEntryCount = 0;
            window.__lmIoTarget = document.createElement('div');
            document.body.appendChild(window.__lmIoTarget);
            window.__lmIoObserver = new IntersectionObserver((entries) => {
                window.__lmIoCallbacks += 1;
                window.__lmIoEntryCount += entries.length;
            });
            window.__startPreparedObserver = () => window.__lmIoObserver.observe(window.__lmIoTarget);
            return 'observed';
        })()"#,
    ).await;
    vm.publish_layout_for_test()
        .expect("publish observer fixture");
    eval_with_rendering(&mut vm, "window.__startPreparedObserver()").await;
    let after_initial = eval_with_rendering(&mut vm, "String(window.__lmIoCallbacks)").await;
    assert_eq!(
        after_initial, "1",
        "initial-intersection delivery should bring callback count to 1",
    );
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoTarget.remove();
            document.body.appendChild(window.__lmIoTarget);
            return 'mutated';
        })()"#,
    )
    .await;
    assert_eq!(
        eval_with_rendering(&mut vm, "String(window.__lmIoCallbacks)").await,
        "1",
        "detach/reattach within one script does not change the rendered intersection"
    );
    eval_with_rendering(&mut vm, "window.__lmIoTarget.remove()").await;
    eval_with_rendering(&mut vm, "document.body.appendChild(window.__lmIoTarget)").await;
    let after_mutation = eval_with_rendering(&mut vm, "String(window.__lmIoCallbacks)").await;
    let after_mutation: i64 = after_mutation
        .parse()
        .expect("callback counter must parse as an integer");
    assert!(
        after_mutation > 1,
        "IntersectionObserver callback must fire on DOM mutation without a registered \
         MutationObserver (got callback count {after_mutation})",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn intersection_observer_callback_can_replace_its_observation_reentrantly() {
    let mut vm = new_observer_regression_vm();
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoReentrantLog = [];
            const first = document.createElement('div');
            first.id = 'first';
            const second = document.createElement('div');
            second.id = 'second';
            document.body.append(first, second);

            const observer = new IntersectionObserver((entries) => {
                window.__lmIoReentrantLog.push(
                    entries.map((entry) => entry.target.id).join(',')
                );
                if (window.__lmIoReentrantLog.length === 1) {
                    observer.disconnect();
                    first.remove();
                    observer.observe(second);
                    window.__lmIoTakeRecordsDuringCallback = observer.takeRecords().length;
                }
            });
            window.__lmIoReentrantObserver = observer;
            observer.observe(first);
            return 'observed';
        })()"#,
    )
    .await;

    assert_eq!(
        eval_with_rendering(
            &mut vm,
            "JSON.stringify([window.__lmIoReentrantLog, window.__lmIoTakeRecordsDuringCallback])",
        )
        .await,
        r#"[["first","second"],0]"#,
        "delivery must release observer owner state before callback reentrant disconnect/observe",
    );
}

#[test]
fn intersection_observer_option_getter_can_reenter_observer_and_dom_owners() {
    let mut vm = new_dom_regression_vm();
    let result = eval_with_body(
        &mut vm,
        r#"(() => {
            const target = document.createElement('div');
            document.body.appendChild(target);
            let innerObserver = null;
            let getterCalls = 0;
            const options = {
                get root() {
                    getterCalls += 1;
                    innerObserver = new IntersectionObserver(() => {});
                    const reentrantNode = document.createElement('span');
                    reentrantNode.id = 'from-root-getter';
                    document.body.appendChild(reentrantNode);
                    return document;
                },
                threshold: [0]
            };
            const outerObserver = new IntersectionObserver(() => {}, options);
            innerObserver.observe(target);
            outerObserver.observe(target);
            return JSON.stringify([
                getterCalls,
                document.getElementById('from-root-getter') !== null,
                outerObserver.root === document
            ]);
        })()"#,
    );

    assert_eq!(
        result, "[1,true,true]",
        "WebIDL option getters must run before any live host/DOM borrow phase",
    );
}

#[test]
fn mutation_observer_callback_can_replace_registration_and_mutate_reentrantly() {
    let mut vm = new_dom_regression_vm();
    eval_with_body(
        &mut vm,
        r#"(() => {
            window.__lmMoReentrantLog = [];
            const observer = new MutationObserver((records) => {
                window.__lmMoReentrantLog.push(
                    records.flatMap((record) =>
                        Array.from(record.addedNodes, (node) => node.id)
                    ).join(',')
                );
                if (window.__lmMoReentrantLog.length === 1) {
                    observer.disconnect();
                    observer.observe(document.body, { childList: true });
                    const second = document.createElement('div');
                    second.id = 'second';
                    document.body.appendChild(second);
                }
            });
            window.__lmMoReentrantObserver = observer;
            observer.observe(document.body, { childList: true });
            const first = document.createElement('div');
            first.id = 'first';
            document.body.appendChild(first);
            return 'observed';
        })()"#,
    );

    assert_eq!(
        eval_with_body(&mut vm, "JSON.stringify(window.__lmMoReentrantLog)"),
        r#"["first","second"]"#,
        "delivery must release observer owner state before callback reentrant observe/mutation",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn rootless_intersection_observer_keeps_plain_deep_spa_targets_viewport_visible() {
    // Real block layout gives empty wrappers zero height, so a deep plain SPA
    // sentinel can remain in the first viewport and trigger the same lazy-chunk
    // callback as Chromium. The explicit Mock provider retains its older
    // bounded compatibility heuristic separately.
    let mut vm = new_observer_regression_vm();
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoEntries = [];
            for (let i = 0; i < 400; i++) {
                document.body.appendChild(document.createElement('div'));
            }
            const target = document.createElement('div');
            document.body.appendChild(target);
            const observer = new IntersectionObserver((entries) => {
                window.__lmIoEntries.push(...entries.map((entry) => ({
                    isIntersecting: entry.isIntersecting,
                    ratio: entry.intersectionRatio,
                    top: entry.boundingClientRect.top
                })));
            }, { root: null, rootMargin: '0px', threshold: [0, 1] });
            window.__startPreparedObserver = () => observer.observe(target);
            return 'observed';
        })()"#,
    )
    .await;
    vm.publish_layout_for_test()
        .expect("publish observer fixture");
    eval_with_rendering(&mut vm, "window.__startPreparedObserver()").await;

    let result = eval_with_rendering(
        &mut vm,
        r#"JSON.stringify(window.__lmIoEntries.map((entry) => ({
            isIntersecting: entry.isIntersecting,
            ratio: entry.ratio,
            top: entry.top
        })))"#,
    )
    .await;
    let entries: serde_json::Value =
        serde_json::from_str(&result).expect("IO entries should be JSON");
    let first = entries
        .as_array()
        .and_then(|entries| entries.first())
        .expect("rootless IO should deliver an initial entry");
    assert_eq!(first["isIntersecting"], true);
    assert_eq!(first["ratio"], 1.0);
    assert!(
        first["top"].as_f64().is_some_and(|top| top < 1080.0),
        "compressed rootless IO mock flow should place the target in the viewport: {first:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn rootless_intersection_observer_does_not_promote_display_none_targets() {
    // Class names have no layout meaning on their own. The stylesheet result,
    // not a site-specific `hidden` token heuristic, must remove the target's
    // box from IntersectionObserver geometry.
    let mut vm = new_observer_regression_vm();
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoEntries = [];
            for (let i = 0; i < 400; i++) {
                document.body.appendChild(document.createElement('div'));
            }
            const target = document.createElement('li');
            target.className = 'no-hover hidden';
            target.style.display = 'none';
            document.body.appendChild(target);
            const observer = new IntersectionObserver((entries) => {
                window.__lmIoEntries.push(...entries.map((entry) => ({
                    isIntersecting: entry.isIntersecting,
                    ratio: entry.intersectionRatio,
                    top: entry.boundingClientRect.top,
                    left: entry.boundingClientRect.left,
                    width: entry.boundingClientRect.width,
                    height: entry.boundingClientRect.height
                })));
            }, { root: null, rootMargin: '0px', threshold: [0, 1] });
            observer.observe(target);
            return 'observed';
        })()"#,
    )
    .await;

    let result = eval_with_rendering(&mut vm, "JSON.stringify(window.__lmIoEntries)").await;
    let entries: serde_json::Value =
        serde_json::from_str(&result).expect("IO entries should be JSON");
    let first = entries
        .as_array()
        .and_then(|entries| entries.first())
        .expect("rootless IO should deliver an initial entry");
    assert_eq!(first["isIntersecting"], false);
    assert_eq!(first["ratio"], 0.0);
    assert_eq!(first["width"], 0.0);
    assert_eq!(first["height"], 0.0);
    assert_eq!(first["top"], 0.0);
    assert_eq!(first["left"], 0.0);
}

#[tokio::test(flavor = "current_thread")]
async fn rootless_intersection_observer_keeps_inline_sized_deep_targets_outside_viewport() {
    // Chromium GUI keeps a sentinel below the viewport after preceding siblings
    // with real authored height. The rootless IO correction should only erase
    // mock height from empty wrapper trees; inline geometry hints still
    // contribute to flow distance.
    let mut vm = new_observer_regression_vm();
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoEntries = [];
            for (let i = 0; i < 400; i++) {
                const spacer = document.createElement('div');
                spacer.style.height = '24px';
                document.body.appendChild(spacer);
            }
            const target = document.createElement('div');
            document.body.appendChild(target);
            const observer = new IntersectionObserver((entries) => {
                window.__lmIoEntries.push(...entries.map((entry) => ({
                    isIntersecting: entry.isIntersecting,
                    ratio: entry.intersectionRatio,
                    top: entry.boundingClientRect.top
                })));
            }, { root: null, rootMargin: '0px', threshold: [0, 1] });
            window.__startPreparedObserver = () => observer.observe(target);
            return 'observed';
        })()"#,
    )
    .await;
    vm.publish_layout_for_test()
        .expect("publish observer fixture");
    eval_with_rendering(&mut vm, "window.__startPreparedObserver()").await;

    let result = eval_with_rendering(&mut vm, "JSON.stringify(window.__lmIoEntries)").await;
    let entries: serde_json::Value =
        serde_json::from_str(&result).expect("IO entries should be JSON");
    let first = entries
        .as_array()
        .and_then(|entries| entries.first())
        .expect("rootless IO should deliver an initial entry");
    assert_eq!(first["isIntersecting"], false);
    assert_eq!(first["ratio"], 0.0);
    assert!(
        first["top"].as_f64().is_some_and(|top| top > 1080.0),
        "inline-sized preceding content should keep the target below the viewport: {first:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn rootless_intersection_observer_counts_text_content_flow_units() {
    // Flight result cards and similar list rows often get most of their real
    // height from text/content rather than inline styles. Rootless IO should
    // not collapse those content-bearing rows as if they were empty wrappers,
    // otherwise infinite-list sentinels become visible too early.
    let mut vm = new_observer_regression_vm();
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoEntries = [];
            for (let i = 0; i < 80; i++) {
                const row = document.createElement('div');
                row.textContent = 'flight row ' + i;
                document.body.appendChild(row);
            }
            const target = document.createElement('div');
            document.body.appendChild(target);
            const observer = new IntersectionObserver((entries) => {
                window.__lmIoEntries.push(...entries.map((entry) => ({
                    isIntersecting: entry.isIntersecting,
                    ratio: entry.intersectionRatio,
                    top: entry.boundingClientRect.top
                })));
            }, { root: null, rootMargin: '0px', threshold: [0, 1] });
            window.__startPreparedObserver = () => observer.observe(target);
            return 'observed';
        })()"#,
    )
    .await;
    vm.publish_layout_for_test()
        .expect("publish observer fixture");
    eval_with_rendering(&mut vm, "window.__startPreparedObserver()").await;

    let result = eval_with_rendering(&mut vm, "JSON.stringify(window.__lmIoEntries)").await;
    let entries: serde_json::Value =
        serde_json::from_str(&result).expect("IO entries should be JSON");
    let first = entries
        .as_array()
        .and_then(|entries| entries.first())
        .expect("rootless IO should deliver an initial entry");
    assert_eq!(first["isIntersecting"], false);
    assert_eq!(first["ratio"], 0.0);
    assert!(
        first["top"].as_f64().is_some_and(|top| top > 1080.0),
        "text-bearing rows should keep later sentinels below the viewport: {first:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn intersection_observer_and_mutation_observer_coexist_during_dom_mutations() {
    // Regression guard for the fix to queue_mutation_records: the IO
    // scheduling must run whether or not records actually got pushed, but
    // MutationObserver delivery must continue to work when records *are*
    // present. Register both, mutate the DOM, and verify both pipelines
    // fire.
    let mut vm = new_observer_regression_vm();
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoTarget = document.createElement('div');
            document.body.appendChild(window.__lmIoTarget);
            window.__lmIoCount = 0;
            window.__lmMoCount = 0;
            window.__lmIoObserver = new IntersectionObserver(() => {
                window.__lmIoCount += 1;
            });
            window.__lmMoObserver = new MutationObserver((records) => {
                window.__lmMoCount += records.length;
            });
            window.__startPreparedObserver = () => window.__lmIoObserver.observe(window.__lmIoTarget);
            window.__lmMoObserver.observe(document.body, { childList: true, subtree: true });
            return 'observed';
        })()"#,
    ).await;
    vm.publish_layout_for_test()
        .expect("publish observer fixture");
    eval_with_rendering(&mut vm, "window.__startPreparedObserver()").await;
    eval_with_rendering(&mut vm, "String(window.__lmIoCount)").await;
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoTarget.remove();
            document.body.appendChild(window.__lmIoTarget);
            return 'mutated';
        })()"#,
    )
    .await;
    assert_eq!(
        eval_with_rendering(&mut vm, "String(window.__lmIoCount)").await,
        "1",
        "detach/reattach within one script does not change the rendered intersection"
    );
    eval_with_rendering(&mut vm, "window.__lmIoTarget.remove()").await;
    eval_with_rendering(&mut vm, "document.body.appendChild(window.__lmIoTarget)").await;
    let io_count: i64 = eval_with_rendering(&mut vm, "String(window.__lmIoCount)")
        .await
        .parse()
        .expect("IO callback counter must parse");
    let mo_count: i64 = eval_with_rendering(&mut vm, "String(window.__lmMoCount)")
        .await
        .parse()
        .expect("MO records counter must parse");
    assert!(
        io_count > 1,
        "IntersectionObserver should still see post-mutation entries while MO is active \
         (got IO callbacks {io_count})",
    );
    assert!(
        mo_count >= 2,
        "MutationObserver should receive at least one record per detach/reattach pair \
         (got MO records {mo_count})",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn mutation_observer_disconnect_does_not_starve_intersection_observer() {
    // The mutation_records_enabled flag flips off when the last
    // MutationObserver disconnects. If queue_intersection_checks were
    // gated on the flag (or on records being non-empty), an IO registered
    // *after* an MO is observed-then-disconnected would silently stop
    // receiving mutation-driven entries. This test pins the lifecycle:
    // attach MO → disconnect MO → register IO → mutate → IO must still
    // see deliveries.
    let mut vm = new_observer_regression_vm();
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            const transient = new MutationObserver(() => {});
            transient.observe(document.body, { childList: true });
            transient.disconnect();
            window.__lmIoTarget = document.createElement('div');
            document.body.appendChild(window.__lmIoTarget);
            window.__lmIoCount = 0;
            window.__lmIoObserver = new IntersectionObserver(() => {
                window.__lmIoCount += 1;
            });
            window.__startPreparedObserver = () => window.__lmIoObserver.observe(window.__lmIoTarget);
            return 'observed';
        })()"#,
    ).await;
    vm.publish_layout_for_test()
        .expect("publish observer fixture");
    eval_with_rendering(&mut vm, "window.__startPreparedObserver()").await;
    eval_with_rendering(&mut vm, "String(window.__lmIoCount)").await;
    eval_with_rendering(
        &mut vm,
        r#"(() => {
            window.__lmIoTarget.remove();
            document.body.appendChild(window.__lmIoTarget);
            return 'mutated';
        })()"#,
    )
    .await;
    assert_eq!(
        eval_with_rendering(&mut vm, "String(window.__lmIoCount)").await,
        "1",
        "detach/reattach within one script does not change the rendered intersection"
    );
    eval_with_rendering(&mut vm, "window.__lmIoTarget.remove()").await;
    eval_with_rendering(&mut vm, "document.body.appendChild(window.__lmIoTarget)").await;
    let count: i64 = eval_with_rendering(&mut vm, "String(window.__lmIoCount)")
        .await
        .parse()
        .expect("IO callback counter must parse");
    assert!(
        count > 1,
        "IntersectionObserver must keep seeing DOM mutations after an MO has \
         registered-then-disconnected (got callbacks {count})",
    );
}

#[test]
fn intersection_observer_does_not_loop_on_stylesheet_heavy_page() {
    // Regression for cf798154 (2026-05-10): the IO geometry hot path used to
    // call into the cascade for `overflow*` lookups, which on stylesheet-heavy
    // pages (Feishu help center, jQuery/Sizzle init) re-parsed every
    // <style>/match every selector for every IO target on every mutation, so
    // mutation flood × IO ancestor walk × cascade became effectively an
    // infinite loop. Per docs/intersection-observer-no-layout-2026-05-12.md the
    // IO check now reads inline-style only.
    //
    // This test installs a non-trivial stylesheet, registers an IO with one
    // target, and runs many mutations in a single eval. The retained-style
    // rebuild assertion pins the intended invariant directly; the loose wall
    // clock budget still catches catastrophic regressions on non-test builds.
    let mut vm = new_dom_regression_vm();
    let document = vm.document_handle_for_test();
    eval_with_body(
        &mut vm,
        r#"(() => {
            const style = document.createElement('style');
            let css = '';
            for (let i = 0; i < 200; i++) {
                css += `.lm-rule-${i} { color: rgb(${i}, ${i}, ${i}); padding: ${i}px; width: ${100 + i}px; }\n`;
            }
            style.textContent = css;
            (document.head || document.documentElement).appendChild(style);
            window.__lmIoCallbacks = 0;
            window.__lmIoTarget = document.createElement('div');
            window.__lmIoTarget.className = 'lm-rule-7';
            document.body.appendChild(window.__lmIoTarget);
            window.__lmIoObserver = new IntersectionObserver(() => {
                window.__lmIoCallbacks += 1;
            });
            window.__lmIoObserver.observe(window.__lmIoTarget);
            return 'observed';
        })()"#,
    );
    let rebuilds_before = vm.retained_style_system_rebuild_count_for_document_for_test(document);
    let started = std::time::Instant::now();
    eval_with_body(
        &mut vm,
        r#"(() => {
            for (let i = 0; i < 500; i++) {
                const child = document.createElement('div');
                child.className = 'lm-rule-' + (i % 200);
                document.body.appendChild(child);
            }
            return 'flooded';
        })()"#,
    );
    let elapsed = started.elapsed();
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds_before,
        "IntersectionObserver mutation checks must stay cascade-free",
    );
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "stylesheet-heavy IO mutation flood must not loop on cascade reads \
         (took {elapsed:?})",
    );
}

#[test]
fn intersection_observer_inline_complex_width_stays_cascade_free() {
    // Microsoft's module-evaluation tail installs IO targets with complex
    // inline width values. IO geometry only needs the cheap mock box; resolving
    // those widths through the computed-style path reparses stylesheet-heavy
    // pages during every mutation-driven IO check.
    let mut vm = new_dom_regression_vm();
    let document = vm.document_handle_for_test();
    eval_with_body(
        &mut vm,
        r#"(() => {
            const style = document.createElement('style');
            let css = '';
            for (let i = 0; i < 200; i++) {
                css += `.lm-rule-${i} { color: rgb(${i}, ${i}, ${i}); padding: ${i}px; width: ${100 + i}px; }\n`;
            }
            style.textContent = css;
            (document.head || document.documentElement).appendChild(style);

            window.__lmIoCallbacks = 0;
            window.__lmIoParent = document.createElement('div');
            window.__lmIoParent.style.width = 'calc(100% - 1px)';
            document.body.appendChild(window.__lmIoParent);

            window.__lmIoTarget = document.createElement('div');
            window.__lmIoTarget.className = 'lm-rule-7';
            window.__lmIoTarget.style.width = 'calc(50% + 10px)';
            window.__lmIoParent.appendChild(window.__lmIoTarget);

            window.__lmIoObserver = new IntersectionObserver(() => {
                window.__lmIoCallbacks += 1;
            });
            window.__lmIoObserver.observe(window.__lmIoTarget);
            return 'observed';
        })()"#,
    );
    let rebuilds_before = vm.retained_style_system_rebuild_count_for_document_for_test(document);
    eval_with_body(
        &mut vm,
        r#"(() => {
            for (let i = 0; i < 100; i++) {
                const child = document.createElement('span');
                child.textContent = String(i);
                window.__lmIoParent.appendChild(child);
            }
            return 'mutated';
        })()"#,
    );
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        rebuilds_before,
        "IntersectionObserver complex inline width reads must not enter computed style",
    );
}

#[test]
fn dom_slot_attr_flood_round_trips_slot_attribute_on_non_shadow_host() {
    // Mirrors dom-slot-attr-flood: setAttribute('slot', ...) toggling on a
    // child whose parent has no shadow root. Validates that
    // record_slot_changes_for_host_child's no-shadow-root short-circuit
    // does not break the attribute round trip.
    let status = run_dom_case(
        r#"
        const root = document.querySelector('main');
        const node = document.createElement('div');
        root.appendChild(node);
        const COUNT = 1000;
        for (let i = 0; i < COUNT; i++) {
            node.setAttribute('slot', 'slot-' + (i & 1));
        }
        document.body.dataset.benchmarkStatus =
            node.getAttribute('slot') === ('slot-' + ((COUNT - 1) & 1)) ? 'ok' : 'bad';
        "#,
    );
    assert_eq!(status, "ok");
}
