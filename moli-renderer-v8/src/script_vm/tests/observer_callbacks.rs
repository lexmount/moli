use super::*;

fn assert_mutation_record_attribute_checks(json: &str, total: usize) {
    let facts: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(facts["complete"], true);
    assert_eq!(facts["total"], total);
    let checks = facts["checks"].as_array().unwrap();
    let failures: Vec<_> = checks.iter().filter(|row| row["passed"] != true).collect();
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(checks.len(), total);
    let names: std::collections::BTreeSet<_> = checks
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), total);
}

fn run_mutation_record_attribute_checks(group: &str, total: usize) {
    let mut vm = new_storage_html_test_vm("https://mutation-record-attributes.test/");
    vm.eval(&format!(
        "document.body.innerHTML='<iframe id=child></iframe>'; \
         globalThis.__mutationRecordGroup={group:?};"
    ))
    .unwrap();
    vm.eval(include_str!(
        "../../../tests/fixtures/mutation-record-attributes.js"
    ))
    .unwrap();
    assert_mutation_record_attribute_checks(
        &vm.eval("JSON.stringify(__uiEventResults)").unwrap(),
        total,
    );
}

#[test]
fn mutation_record_attributes_are_readonly_prototype_snapshots() {
    run_mutation_record_attribute_checks("snapshots", 324);
}

#[test]
fn mutation_record_getters_validate_native_receivers_before_public_lookup() {
    run_mutation_record_attribute_checks("brands", 228);
}

#[tokio::test(flavor = "current_thread")]
async fn mutation_record_delivery_uses_callback_realm_and_retains_snapshot_values() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://mutation-record-delivery.test/",
        &loader,
    );
    vm.exec(
        "document.body.innerHTML='<iframe id=child></iframe>'; \
         globalThis.__mutationRecordGroup='delivery';",
        None,
    )
    .unwrap();
    vm.exec(
        include_str!("../../../tests/fixtures/mutation-record-attributes.js"),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "MutationRecord callback conversion and retained snapshots",
    )
    .await;
    assert_mutation_record_attribute_checks(
        &vm.eval("JSON.stringify(__uiEventResults)").unwrap(),
        12,
    );
}

#[test]
fn mutation_record_getters_read_registered_native_proxy_payloads() {
    fn register_proxy<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        args: v8::FunctionCallbackArguments<'s>,
        mut rv: v8::ReturnValue<'s>,
    ) {
        let target = v8::Local::<v8::Object>::try_from(args.get(0)).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert!(crate::web_api_interfaces::MutationRecord::is_instance(
            scope,
            proxy.into()
        ));
        rv.set(proxy.into());
    }
    let mut vm = new_storage_html_test_vm("https://mutation-record-native-proxy.test/");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let function = v8::Function::new(scope, register_proxy).unwrap();
        let key = crate::util::v8str(scope, "registerNativeRecordProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), function.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        document.body.innerHTML='<iframe id=child></iframe>';
        const child=document.getElementById('child').contentWindow;
        const target=document.createElement('item');
        target.setAttribute('keep','before\ud800');
        const observer=new MutationObserver(()=>{});
        observer.observe(target,{attributeOldValue:true});
        target.setAttribute('keep','after');
        const record=observer.takeRecords()[0]; observer.disconnect();
        const proxy=registerNativeRecordProxy(record);
        const revoked=Proxy.revocable(proxy,{}); revoked.revoke();
        let traps=0;
        const author=new Proxy(proxy,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 43;}});
        for(const callee of [globalThis,child]) {
            for(const key of ['type','target','addedNodes','removedNodes','previousSibling',
                'nextSibling','attributeName','attributeNamespace','oldValue']) {
                const get=Object.getOwnPropertyDescriptor(callee.MutationRecord.prototype,key).get;
                if(get.call(proxy)!==record[key])throw Error('native payload '+key);
                for(const invalid of [author,revoked.proxy,Object.create(proxy)]) {
                    let error;try{get.call(invalid);}catch(caught){error=caught;}
                    if(!(error instanceof callee.TypeError))throw Error('author receiver '+key);
                }
            }
        }
        return traps===0;
    })()"#).unwrap(), "true");
}

fn run_mutation_observer_init_checks(group: &str, total: usize) {
    let mut vm = new_storage_html_test_vm("https://mutation-observer-init.test/");
    vm.eval(&format!(
        "document.body.innerHTML='<iframe id=child></iframe>'; \
         globalThis.__mutationObserverGroup={group:?};"
    ))
    .unwrap();
    vm.eval(include_str!(
        "../../../tests/fixtures/mutation-observer-init.js"
    ))
    .unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__uiEventResults.total").unwrap(),
        total.to_string()
    );
    let names: Vec<String> = serde_json::from_str(
        &vm.eval("JSON.stringify(__uiEventResults.checks.map(row=>row.name))")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(names.len(), total);
    assert_eq!(
        names
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        total
    );
}

#[test]
fn mutation_observer_init_converts_iterables_and_preserves_dictionary_order_and_registration() {
    run_mutation_observer_init_checks("conversion", 128);
}

#[test]
fn mutation_observer_filters_match_exact_names_namespaces_and_utf16_identity() {
    run_mutation_observer_init_checks("filters", 52);
}

#[test]
fn same_value_meta_set_attribute_records_preserve_xml_case_and_namespace() {
    let mut vm = new_storage_html_test_vm("https://meta-attribute-identity.test/");
    let result = vm
        .eval(
            r#"(() => {
        const xml = document.implementation.createDocument(
            'http://www.w3.org/1999/xhtml', 'meta', null);
        const cases = [
            [xml.documentElement, 'CONTENT', null, 'CONTENT'],
            [xml.documentElement, 'HTTP-EQUIV', null, 'HTTP-EQUIV'],
            [document.createElement('meta'), 'CONTENT', null, 'content'],
            [document.createElement('meta'), 'content', 'urn:test', 'content'],
            [document.createElement('meta'), 'p:content', 'urn:test', 'content'],
        ];
        for (const [meta, setter, namespace, local] of cases) {
            if (namespace === null) meta.setAttribute(setter, 'x');
            else meta.setAttributeNS(namespace, setter, 'x');
            const all = new MutationObserver(() => {});
            const filtered = new MutationObserver(() => {});
            all.observe(meta, {attributes: true, attributeOldValue: true});
            filtered.observe(meta, {attributes: true, attributeFilter: [local]});
            meta.setAttribute(setter, 'x');
            const records = all.takeRecords();
            if (records.length !== 1 || records[0].attributeName !== local ||
                records[0].attributeNamespace !== namespace || records[0].oldValue !== 'x')
                throw Error('wrong attribute identity: ' + setter);
            const expected = namespace === null ? 1 : 0;
            if (filtered.takeRecords().length !== expected)
                throw Error('wrong filter result: ' + setter);
            all.disconnect(); filtered.disconnect();
        }
        return true;
    })()"#,
        )
        .unwrap();
    assert_eq!(result, "true");
}

#[test]
fn same_value_namespace_and_dataset_sets_deliver_one_attribute_record() {
    let mut vm = new_storage_html_test_vm("https://same-value-attribute-records.test/");
    assert_eq!(vm.eval(r#"(() => {
        const cases = [
            [null, 'data-x', element => element.setAttributeNS(null, 'data-x', 'a')],
            [null, 'data-x', element => element.setAttributeNS('', 'data-x', 'a')],
            [null, 'data-x', element => { element.dataset.x = 'a'; }],
            [null, 'data-x', element => element.setAttribute('DATA-X', 'a')],
            ['urn:test', 'p:x', element => element.setAttributeNS('urn:test', 'p:x', 'a')],
            ['urn:test', 'p:x', element => element.setAttribute('p:x', 'a')],
        ];
        for (const tag of ['div', 'meta']) for (const [namespace, qualified, set] of cases) {
            const element = document.createElement(tag);
            element.setAttributeNS(namespace, qualified, 'a');
            const local = qualified.split(':').pop();
            const observer = new MutationObserver(() => {});
            const filtered = new MutationObserver(() => {});
            observer.observe(element, {attributes: true, attributeOldValue: true});
            filtered.observe(element, {attributes: true, attributeFilter: [local]});
            for (let i = 0; i < 2; i++) {
                set(element);
                const records = observer.takeRecords();
                if (records.length !== 1 || records[0].target !== element ||
                    records[0].attributeName !== local || records[0].attributeNamespace !== namespace ||
                    records[0].oldValue !== 'a') throw Error(tag + ':' + qualified + ':same-value record');
                if (filtered.takeRecords().length !== (namespace === null ? 1 : 0))
                    throw Error('attributeFilter: ' + qualified);
            }
            observer.disconnect(); filtered.disconnect();
        }
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn mutation_observer_observe_accepts_registered_native_proxy_targets_and_receivers() {
    fn register_proxy<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        args: v8::FunctionCallbackArguments<'s>,
        mut rv: v8::ReturnValue<'s>,
    ) {
        let target = v8::Local::<v8::Object>::try_from(args.get(0)).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        rv.set(proxy.into());
    }
    let mut vm = new_storage_html_test_vm("https://mutation-observer-native-proxy.test/");
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let function = v8::Function::new(scope, register_proxy).unwrap();
        let key = crate::util::v8str(scope, "registerNativeProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), function.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const observer = new MutationObserver(() => {});
        const target = document.createElement('div');
        const observerProxy = registerNativeProxy(observer);
        const targetProxy = registerNativeProxy(target);
        MutationObserver.prototype.observe.call(observerProxy, targetProxy, {
            attributeFilter: new Set(['keep'])
        });
        target.setAttribute('drop', 'x'); target.setAttribute('keep', 'y');
        const records = MutationObserver.prototype.takeRecords.call(observerProxy);
        if (records.length !== 1 || records[0].target !== target || records[0].attributeName !== 'keep')
            throw Error('registered native identity');
        let gets = 0, traps = 0;
        for (const [receiver, node] of [
            [new Proxy(observerProxy, {get(){traps++;}}), targetProxy],
            [observerProxy, new Proxy(targetProxy, {get(){traps++;}})]
        ]) {
            let error;
            try {MutationObserver.prototype.observe.call(receiver, node, {
                get childList(){gets++;return true;}
            });} catch(caught) {error = caught;}
            if (!(error instanceof TypeError)) throw Error('author Proxy was accepted');
        }
        MutationObserver.prototype.disconnect.call(observerProxy);
        return gets === 0 && traps === 0;
    })()"#).unwrap(), "true");
}

#[tokio::test(flavor = "current_thread")]
async fn xml_serialization_preserves_utf16_and_uses_native_node_identity() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://xml-serialization-utf16.test/",
        &loader,
    );
    // Full popup Pages execute the same fixture over CDP.
    vm.exec("globalThis.__xmlSerializationIncludePopup = false;", None)
        .unwrap();
    vm.exec(include_str!("xml_serialization_utf16.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "XML native DOMString serialization",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
    assert_eq!(vm.eval("__uiEventResults.includePopup").unwrap(), "false");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "3450");
}

#[tokio::test(flavor = "current_thread")]
async fn html_serialization_preserves_utf16_and_custom_element_is_identity() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://html-serialization-utf16.test/",
        &loader,
    );
    // Actual popup Pages run the same fixture over CDP.
    vm.exec("globalThis.__htmlSerializationIncludePopup = false;", None)
        .unwrap();
    vm.exec(include_str!("html_serialization_utf16.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "HTML DOMString serialization",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
    assert_eq!(vm.eval("__uiEventResults.includePopup").unwrap(), "false");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "3196");
}

#[tokio::test(flavor = "current_thread")]
async fn attribute_argument_conversion_precedes_dom_validation_and_policy_lookup() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://attribute-argument-order.test/",
        &loader,
    );
    // Actual popup Pages are covered by the same fixture over CDP.
    vm.exec("globalThis.__attributeOrderIncludePopup = false;", None)
        .unwrap();
    vm.exec(include_str!("attribute_argument_order.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "attribute argument conversion order",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
    assert_eq!(vm.eval("__uiEventResults.includePopup").unwrap(), "false");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "532");
}

#[tokio::test(flavor = "current_thread")]
async fn attribute_mutation_and_custom_element_reaction_snapshots_preserve_utf16() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://attribute-mutation-values.test/",
        &loader,
    );
    // This harness has no owner-local Page slot for activating real popup
    // Pages. The full three-realm fixture runs over CDP; this native test
    // covers main and iframe Documents on the production selected-task path.
    vm.exec("globalThis.__attributeMutationIncludePopup = false;", None)
        .unwrap();
    vm.exec(include_str!("attribute_mutation_values.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "attribute mutation UTF-16 snapshots",
    )
    .await;
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "1796");
    assert_eq!(vm.eval("__uiEventResults.includePopup").unwrap(), "false");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
}

async fn observer_callback_test_vm(url: &str) -> crate::runtime::PageVmTaskExecutorTestHarness {
    let mut vm = new_storage_page_task_executor_test_vm(url);
    vm.eval(
        r#"
(() => {
  const root =
    document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__observerCallbackFrame = frame;
})()
"#,
    )
    .expect("observer callback child-realm setup should evaluate");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("materialize observer callback child realm");
    vm
}

#[tokio::test(flavor = "current_thread")]
async fn borrowed_performance_methods_use_receiver_timeline_and_entry_realms() {
    let mut vm = observer_callback_test_vm("https://performance-borrowed-method-realms.test/").await;
    let source = format!(
        "({})(window, __observerCallbackFrame.contentWindow).then(result => globalThis.__borrowedTimeline = result, error => globalThis.__borrowedTimeline = {{ error: String(error) }});",
        include_str!("performance_timeline_borrowed_methods.js"),
    );
    vm.eval(&source)
        .expect("queue borrowed mark and measure deliveries");
    assert_eq!(
        vm.eval("JSON.stringify(__borrowedTimeline)").unwrap(),
        r#"{"received":{"parent":["parent-mark","parent-measure"],"child":["child-mark","child-measure"]},"failures":[]}"#,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn borrowed_performance_observe_reads_and_delivers_the_observers_own_buffer() {
    let mut vm = observer_callback_test_vm("https://performance-borrowed-buffered-observe.test/").await;
    vm.eval(
        r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  globalThis.__borrowedBuffered = { parent: [], child: [] };
  performance.mark('parent-buffered');
  child.performance.mark('child-buffered');
  const parentObserver = new PerformanceObserver(list => {
    __borrowedBuffered.parent.push(...list.getEntries().map(entry => entry.name));
  });
  const childObserver = new child.PerformanceObserver(list => {
    __borrowedBuffered.child.push(...list.getEntries().map(entry => entry.name));
  });
  child.PerformanceObserver.prototype.observe.call(parentObserver, { type: 'mark', buffered: true });
  PerformanceObserver.prototype.observe.call(childObserver, { type: 'mark', buffered: true });
})()
"#,
    )
    .expect("subscribe to buffers using the other Realm's observe method");
    assert_eq!(
        vm.eval("JSON.stringify(__borrowedBuffered)").unwrap(),
        r#"{"parent":["parent-buffered"],"child":["child-buffered"]}"#,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn borrowed_performance_delivery_survives_retirement_of_the_method_realm() {
    let mut vm = observer_callback_test_vm("https://performance-method-realm-retirement.test/").await;
    vm.eval(
        r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  globalThis.__survivingTimeline = [];
  new PerformanceObserver(list => {
    __survivingTimeline.push(...list.getEntries().map(entry => entry.name));
  }).observe({ type: 'mark' });
  const mark = child.Performance.prototype.mark;
  mark.call(performance, 'before-method-realm-retirement');
  __observerCallbackFrame.remove();
  mark.call(performance, 'after-method-realm-retirement');
})()
"#,
    )
    .expect("retire the method Realm after queuing to the surviving timeline");
    assert_eq!(
        vm.eval("JSON.stringify(__survivingTimeline)").unwrap(),
        r#"["before-method-realm-retirement","after-method-realm-retirement"]"#,
    );
}

#[test]
fn common_callback_registry_keeps_active_observers_until_owner_teardown() {
    let mut vm = new_storage_test_vm("https://performance-observer-active-root.test/");
    let common = std::rc::Rc::clone(&vm._context_host.borrow().common);
    vm.eval(
        r#"
globalThis.__activeObserverEntries = [];
new PerformanceObserver(list => {
  __activeObserverEntries.push(...list.getEntries().map(entry => entry.name));
}).observe({ type: 'mark' });
"#,
    )
    .expect("register an observer with no independent JS root");
    vm.renderer_document_isolate_ops()
        .collect_renderer_document_isolate_garbage()
        .expect("collect unreachable observer wrappers");
    assert_eq!(common.callbacks.len(), 1);
    vm.eval("performance.mark('after-gc')")
        .expect("deliver to the active observer after GC");
    assert_eq!(
        vm.eval("JSON.stringify(__activeObserverEntries)").unwrap(),
        r#"["after-gc"]"#,
    );
    drop(vm);
    assert_eq!(common.callbacks.len(), 0);
    assert_eq!(common.finalizer_count(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn performance_observer_registration_uses_its_own_global_with_a_foreign_callback_realm() {
    let mut vm = observer_callback_test_vm("https://performance-observer-global-isolation.test/").await;
    vm.eval(
        r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  globalThis.__timelineEntries = { parent: [], child: [], callbackRealms: [] };
  globalThis.__parentTimelineObserver = new PerformanceObserver(child.Function(
    'list',
    `parent.__timelineEntries.parent.push(...list.getEntries().map(entry => entry.name));
     parent.__timelineEntries.callbackRealms.push(globalThis === parent.__observerCallbackFrame.contentWindow);`
  ));
  globalThis.__childTimelineObserver = new child.PerformanceObserver(list => {
    __timelineEntries.child.push(...list.getEntries().map(entry => entry.name));
    __timelineEntries.callbackRealms.push(globalThis === window);
  });
  __parentTimelineObserver.observe({ type: 'mark' });
  __childTimelineObserver.observe({ type: 'mark' });
  performance.mark('parent-only');
  child.performance.mark('child-only');
})()
"#,
    )
    .expect("queue separate global timelines with crossed callback realms");
    assert_eq!(
        vm.eval("JSON.stringify(__timelineEntries)").unwrap(),
        r#"{"parent":["parent-only"],"child":["child-only"],"callbackRealms":[true,true]}"#,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn child_node_conversion_preserves_order_and_mutation_boundaries() {
    let mut vm = observer_callback_test_vm("https://child-node-conversion.test/").await;
    assert_eq!(
        vm.eval(include_str!("child_node_conversion.js"))
            .expect("ChildNode conversion must preserve side effects and record boundaries"),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn parent_node_conversion_preserves_order_and_mutation_boundaries() {
    let mut vm = observer_callback_test_vm("https://parent-node-conversion.test/").await;
    assert_eq!(
        vm.eval(include_str!("parent_node_conversion.js"))
            .expect("ParentNode conversion must preserve side effects and record boundaries"),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn document_preinsert_validation_precedes_mutation_and_adoption() {
    let mut vm = observer_callback_test_vm("https://document-preinsert.test/").await;
    assert_eq!(
        vm.eval(include_str!("document_preinsert_validation.js"))
            .expect("Document pre-insert validation should precede mutation and adoption"),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn node_replacement_preserves_records_adoption_and_live_state() {
    let mut vm = observer_callback_test_vm("https://node-replacement.test/").await;
    assert_eq!(
        vm.eval(include_str!("node_replacement.js"))
            .expect("Node replacement must preserve records, adoption and live state"),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn windowless_fragment_insertions_preserve_native_mutation_records() {
    let mut vm = observer_callback_test_vm("https://windowless-fragments.test/").await;
    assert_eq!(
        vm.eval(include_str!("windowless_fragment_mutations.js"))
            .expect("fragment insertion should preserve native mutation records"),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn observer_callbacks_use_webidl_callback_function_realm_receiver_and_proxy_semantics() {
    let mut vm =
        observer_callback_test_vm("https://observer-callback-function-semantics.test/").await;

    let queued = vm
        .eval(
            r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  const target = document.body.appendChild(document.createElement("div"));
  globalThis.__observerCallbackFacts = [];
  globalThis.__observerProxyCalls = [];
  globalThis.__observerCallbackChild = child;

  const makeCallback = (label) => child.Function(
    "label",
    `return new Proxy(
      function(records, observer, options) {
        const recordCount =
          label === "performance" ? records.getEntries().length : records.length;
        parent.__observerCallbackFacts.push({
          label,
          callbackRealm: globalThis === parent.__observerCallbackChild,
          receiver: this === observer,
          observerArgument: observer === parent["__" + label + "Observer"],
          recordCount,
          argumentCount: arguments.length,
          optionsObject:
            label !== "performance" ||
            (options !== null && typeof options === "object")
        });
      },
      {
        apply(target, receiver, args) {
          parent.__observerProxyCalls.push(label);
          return Reflect.apply(target, receiver, args);
        }
      }
    );`
  )(label);

  const mutationObserver =
    new MutationObserver(makeCallback("mutation"));
  globalThis.__mutationObserver = mutationObserver;
  mutationObserver.observe(target, { attributes: true });
  target.setAttribute("data-observer-callback", "queued");

  const intersectionObserver =
    new IntersectionObserver(makeCallback("intersection"));
  globalThis.__intersectionObserver = intersectionObserver;
  intersectionObserver.observe(target);

  const resizeObserver =
    new ResizeObserver(makeCallback("resize"));
  globalThis.__resizeObserver = resizeObserver;
  resizeObserver.observe(target);

  const performanceObserver =
    new PerformanceObserver(makeCallback("performance"));
  globalThis.__performanceObserver = performanceObserver;
  performanceObserver.observe({ type: "mark" });
  performance.mark("observer-callback-semantics");

  return "queued";
})()
"#,
        )
        .expect("observer callback-function semantics should queue");
    assert_eq!(queued, "queued");

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("deliver rendering observers");
    let result = vm
        .eval(
            r#"
JSON.stringify({
  facts: globalThis.__observerCallbackFacts.sort((a, b) =>
    a.label.localeCompare(b.label)
  ),
  proxyCalls: globalThis.__observerProxyCalls.sort()
})
"#,
        )
        .expect("observer callback-function semantics should flush");
    assert_eq!(
        result,
        r#"{"facts":[{"label":"intersection","callbackRealm":true,"receiver":true,"observerArgument":true,"recordCount":1,"argumentCount":2,"optionsObject":true},{"label":"mutation","callbackRealm":true,"receiver":true,"observerArgument":true,"recordCount":1,"argumentCount":2,"optionsObject":true},{"label":"performance","callbackRealm":true,"receiver":true,"observerArgument":true,"recordCount":1,"argumentCount":3,"optionsObject":true},{"label":"resize","callbackRealm":true,"receiver":true,"observerArgument":true,"recordCount":1,"argumentCount":2,"optionsObject":true}],"proxyCalls":["intersection","mutation","performance","resize"]}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn intersection_entries_use_the_borrowed_callback_realm_in_both_layout_modes() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = observer_callback_test_vm("https://intersection-entry-realm.test/").await;
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            globalThis.rows = [];
            globalThis.child = __observerCallbackFrame.contentWindow;
            const target = document.body.appendChild(document.createElement('div'));
            target.id = 'intersection-target';
            target.style.cssText = 'width:100px;height:20px;padding:0;border:0';
        "#,
        )
        .unwrap();
        let before_ready = vm.layout_pass_observability_for_test().1;
        if policy == moli_page_types::LayoutPolicy::OnDemand {
            // Legacy observers sample an already-published tree. Establish the
            // initial sample after fixture setup, before observer registration.
            assert_eq!(
                vm.eval(
                    "document.getElementById('intersection-target').getBoundingClientRect().width"
                )
                .unwrap(),
                "100"
            );
        }
        let builds = vm.layout_pass_observability_for_test().1;
        assert_eq!(
            builds,
            before_ready + u64::from(policy == moli_page_types::LayoutPolicy::OnDemand),
            "readiness publishes one initial real layout and none for Mock: {policy:?}"
        );
        vm.eval(
            r#"
            globalThis.rootIO = new IntersectionObserver(child.Function('entries', 'observer', `
                const entry = entries[0];
                parent.rows.push({
                    callbackRealm: window === parent.child,
                    observerRealm: Object.getPrototypeOf(observer) === parent.IntersectionObserver.prototype,
                    receiver: this === observer && observer === parent.rootIO,
                    arrayRealm: Object.getPrototypeOf(entries) === Array.prototype,
                    entryRealm: Object.getPrototypeOf(entry) === IntersectionObserverEntry.prototype,
                    rectRealm: [entry.boundingClientRect, entry.intersectionRect, entry.rootBounds].every(
                        rect => rect !== null && DOMRectReadOnly.prototype.isPrototypeOf(rect)),
                    target: entry.target === parent.document.getElementById('intersection-target'),
                    count: entries.length,
                    argc: arguments.length
                });
                parent.entryWidth = entry.boundingClientRect.width;
            `));
            rootIO.observe(target);
        "#,
        )
        .unwrap();
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
        vm.advance_timers_until_deadline_for_test(&loader).await.expect("deliver Document intersection task");
        assert_eq!(
            vm.eval("JSON.stringify(rows)").unwrap(),
            r#"[{"callbackRealm":true,"observerRealm":true,"receiver":true,"arrayRealm":true,"entryRealm":true,"rectRealm":true,"target":true,"count":1,"argc":2}]"#,
            "{policy:?}"
        );
        if policy == moli_page_types::LayoutPolicy::OnDemand {
            assert_eq!(vm.eval("entryWidth").unwrap(), "100");
        }
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds,
            "observer registration, delivery and record reads must reuse the initial sample: {policy:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn intersection_entries_project_targets_into_the_isolated_callback_world() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://intersection-isolated-world.test/");
        vm.set_layout_policy(policy);
        vm.eval(
            r#"
            const root = document.documentElement || document.appendChild(document.createElement('html'));
            const body = document.body || root.appendChild(document.createElement('body'));
            const target = body.appendChild(document.createElement('div'));
            target.id = 'intersection-target';
            target.style.cssText = 'width:100px;height:20px;padding:0;border:0';
        "#,
        )
        .unwrap();
        let isolated = vm
            .create_isolated_world("intersection-entry-realm", false)
            .unwrap();
        vm.eval_in_isolated_context(
            isolated,
            r#"
            globalThis.rows = [];
            globalThis.observer = new IntersectionObserver(function(entries, observer) {
                const entry = entries[0];
                rows.push([
                    this === observer && observer === globalThis.observer,
                    Object.getPrototypeOf(entries) === Array.prototype,
                    Object.getPrototypeOf(entry) === IntersectionObserverEntry.prototype,
                    [entry.boundingClientRect, entry.intersectionRect, entry.rootBounds].every(
                        rect => rect !== null && DOMRectReadOnly.prototype.isPrototypeOf(rect)),
                    entry.target === document.getElementById('intersection-target'),
                    HTMLDivElement.prototype.isPrototypeOf(entry.target),
                    entries.length === 1 && arguments.length === 2
                ]);
            });
            observer.observe(document.getElementById('intersection-target'));
        "#,
        )
        .unwrap();
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
        vm.advance_timers_until_deadline_for_test(&loader).await.expect("deliver Document intersection task");
        assert_eq!(
            vm.eval_in_isolated_context(isolated, "JSON.stringify(rows)")
                .unwrap(),
            "[[true,true,true,true,true,true,true]]",
            "{policy:?}"
        );
        vm.destroy_isolated_world_context(isolated);
        assert_eq!(
            vm.eval("'default-realm-still-live'").unwrap(),
            "default-realm-still-live"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn observer_callback_exceptions_are_reported_to_the_callback_relevant_window() {
    let mut vm = observer_callback_test_vm("https://observer-callback-exception-realm.test/").await;

    vm.eval(
        r#"
(() => {
  const child = __observerCallbackFrame.contentWindow;
  const target = document.body.appendChild(document.createElement("div"));
  globalThis.__observerCallbackErrors = [];
  child.addEventListener("error", child.Function(
    "event",
    `parent.__observerCallbackErrors.push({
       message: event.error && event.error.message,
       errorRealm: event.error instanceof Error,
       targetRealm: event.currentTarget === globalThis
     });
     event.preventDefault();`
  ));

  const mutationObserver = new MutationObserver(
    child.Function(`throw new Error("mutation-observer-error");`)
  );
  mutationObserver.observe(target, { attributes: true });
  target.setAttribute("data-observer-error", "queued");

  const intersectionObserver = new IntersectionObserver(
    child.Function(`throw new Error("intersection-observer-error");`)
  );
  intersectionObserver.observe(target);

  const resizeObserver = new ResizeObserver(
    child.Function(`throw new Error("resize-observer-error");`)
  );
  resizeObserver.observe(target);

  const performanceObserver = new PerformanceObserver(
    child.Function(`throw new Error("performance-observer-error");`)
  );
  performanceObserver.observe({ type: "mark" });
  performance.mark("observer-callback-error");
  return "queued";
})()
"#,
    )
    .expect("observer exception projection should queue");

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("deliver rendering observers");
    let result = vm
        .eval(
            "JSON.stringify(globalThis.__observerCallbackErrors.sort((a, b) => a.message.localeCompare(b.message)))",
        )
        .expect("observer exception projection should flush");
    assert_eq!(
        result,
        r#"[{"message":"intersection-observer-error","errorRealm":true,"targetRealm":true},{"message":"mutation-observer-error","errorRealm":true,"targetRealm":true},{"message":"performance-observer-error","errorRealm":true,"targetRealm":true},{"message":"resize-observer-error","errorRealm":true,"targetRealm":true}]"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn observer_deliveries_retire_with_the_exact_observer_or_callback_realm() {
    let mut vm = observer_callback_test_vm("https://observer-callback-retirement.test/").await;

    let queued = vm
        .eval(
            r#"
(() => {
  const frame = __observerCallbackFrame;
  const child = frame.contentWindow;
  const target = document.body.appendChild(document.createElement("div"));
  globalThis.__retiredObserverCallbacks = [];
  const detachedCallback = child.Function(
    `parent.__retiredObserverCallbacks.push("already-retired-callback-realm");`
  );

  // The observer belongs to the parent Window while the callback's relevant
  // Realm belongs to the child Window.
  const mutationObserver = new MutationObserver(
    child.Function(
      `parent.__retiredObserverCallbacks.push("retired-callback-realm");`
    )
  );
  mutationObserver.observe(target, { attributes: true });
  target.setAttribute("data-retired-observer", "queued");

  // This observer object belongs to the child Window while its callback
  // belongs to the still-live parent Window.
  const intersectionObserver = new child.IntersectionObserver(() => {
    globalThis.__retiredObserverCallbacks.push("retired-observer-realm");
  });
  intersectionObserver.observe(target);

  const resizeObserver = new ResizeObserver(
    child.Function(
      `parent.__retiredObserverCallbacks.push("retired-resize-callback-realm");`
    )
  );
  resizeObserver.observe(target);

  const performanceObserver = new child.PerformanceObserver(() => {
    globalThis.__retiredObserverCallbacks.push("retired-performance-observer-realm");
  });
  performanceObserver.observe({ type: "mark" });
  child.performance.mark("retired-performance-observer");

  const performanceCallbackRealmObserver = new PerformanceObserver(
    child.Function(
      `parent.__retiredObserverCallbacks.push("retired-performance-callback-realm");`
    )
  );
  performanceCallbackRealmObserver.observe({ type: "mark" });
  performance.mark("retired-performance-callback");

  frame.remove();

  // Registration after retirement must not turn an unresolved Window
  // identity into an immortal callback residence.
  const observerWithDetachedCallback = new MutationObserver(detachedCallback);
  observerWithDetachedCallback.observe(target, { attributes: true });
  target.setAttribute("data-retired-observer", "detached-callback");

  const resizeObserverWithDetachedCallback = new ResizeObserver(detachedCallback);
  resizeObserverWithDetachedCallback.observe(target);

  return String(frame.contentWindow === null);
})()
"#,
        )
        .expect("observer retirement should queue");
    assert_eq!(queued, "true");

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("deliver rendering observers");
    let result = vm
        .eval("JSON.stringify(globalThis.__retiredObserverCallbacks)")
        .expect("retired observer callbacks should remain suppressed");
    assert_eq!(result, "[]");
}

#[test]
fn js_owned_observer_callback_cycles_remain_v8_collectable() {
    let mut vm = new_storage_test_vm("https://resize-observer-callback-gc.test/");

    let created = vm
        .eval(
            r#"
(() => {
  // Keep every observer reachable until Rust has observed the complete
  // callback-binding population. V8 may otherwise collect early loop
  // iterations before this evaluation returns, which conflates registration
  // with the unreachable-cycle collection phase exercised below.
  globalThis.__observerCallbackCycles = [];
  for (let index = 0; index < 128; index++) {
    let resizeObserver;
    resizeObserver = new ResizeObserver(() => resizeObserver.disconnect());
    const target = document.createElement('div');
    resizeObserver.observe(target);
    if (index % 2) resizeObserver.unobserve(target);
    else resizeObserver.disconnect();

    let performanceObserver;
    performanceObserver = new PerformanceObserver(
      () => performanceObserver.disconnect()
    );
    performanceObserver.observe({ type: "mark" });
    performanceObserver.disconnect();

    globalThis.__observerCallbackCycles.push(
      resizeObserver,
      performanceObserver
    );
  }
  return 256;
})()
"#,
        )
        .expect("JS-owned observer callback cycles should be created");
    assert_eq!(created, "256");
    assert_eq!(
        vm._context_host.borrow().common.callbacks.len(),
        256,
        "each unreachable observer must still start with one exact callback identity binding"
    );

    vm.eval("delete globalThis.__observerCallbackCycles")
        .expect("observer callback cycles should become unreachable");

    vm.renderer_document_isolate_ops()
        .collect_renderer_document_isolate_garbage()
        .expect("renderer isolate GC should complete");

    assert_eq!(
        vm._context_host.borrow().common.callbacks.len(),
        0,
        "a Rust callback registry must not root an unreachable callback-observer cycle"
    );
}

#[test]
fn performance_observer_disconnect_retires_and_reobserve_restores_delivery() {
    let mut vm = new_storage_test_vm("https://performance-observer-reobserve.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__performanceReobserveEntries = [];
  const observer = new PerformanceObserver(list => {
    __performanceReobserveEntries.push(
      ...list.getEntries().map(entry => entry.name)
    );
  });
  observer.observe({ type: "mark" });
  performance.mark("cleared-by-disconnect");
  observer.disconnect();
  performance.mark("ignored-while-disconnected");
  observer.observe({ type: "mark" });
  performance.mark("delivered-after-reobserve");
})()
"#,
    )
    .expect("PerformanceObserver re-observe workflow should queue");

    let delivered = vm
        .eval("JSON.stringify(globalThis.__performanceReobserveEntries)")
        .expect("PerformanceObserver re-observe workflow should flush");
    assert_eq!(delivered, r#"["delivered-after-reobserve"]"#);
}

#[tokio::test(flavor = "current_thread")]
async fn mutation_record_node_lists_are_native_readonly_snapshots() {
    let mut vm = observer_callback_test_vm("https://mutation-record-node-lists.test/").await;
    vm.eval(include_str!("mutation_record_node_lists.js"))
        .expect("MutationRecord NodeList fixture should evaluate");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("deliver mutation records");
    assert_eq!(
        vm.eval("String(globalThis.__mutationRecordNodeListResult)")
            .unwrap(),
        "true"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn dom_implementation_derives_receiver_and_argument_conversion_in_each_document_realm() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://dom-implementation-arguments.test/",
        &loader,
    );
    // Actual popup Pages run the same fixture over CDP.
    vm.exec("globalThis.__domImplementationIncludePopup = false;", None)
        .unwrap();
    vm.exec(include_str!("dom_implementation_arguments.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "DOMImplementation receiver and argument conversion",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
    assert_eq!(vm.eval("__uiEventResults.includePopup").unwrap(), "false");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "4344");
}

#[tokio::test(flavor = "current_thread")]
async fn element_creation_options_convert_before_dom_validation_in_each_document_realm() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://element-creation-options.test/",
        &loader,
    );
    // Real popup Pages run the same fixture over CDP.
    vm.exec("globalThis.__elementOptionsIncludePopup = false;", None)
        .unwrap();
    vm.exec(include_str!("element_creation_options.js"), None)
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__uiEventResults?.complete === true)",
        "true",
        "element creation options conversion",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]",
    );
    assert_eq!(vm.eval("__uiEventResults.includePopup").unwrap(), "false");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "3106");
}
