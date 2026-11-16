mod support;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{set_private_value, v8str};
use moli_webapi::{
    interfaces,
    performance::{self as perf, entries, entry_list, legacy, memory},
};
use moli_webidl::WebIdlConverter;
use std::pin::pin;
use support::{eval, finish_template, illegal_constructor, install_errors, publish};

fn install<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    install_errors(scope);
    let performance = v8::FunctionTemplate::new(scope, illegal_constructor);
    let getter = v8::FunctionTemplate::new(scope, memory::performance_memory_getter);
    performance.prototype_template(scope).set_accessor_property(
        v8str(scope, "memory").into(),
        Some(getter),
        None,
        Default::default(),
    );
    finish_template(scope, interfaces::Performance::DESCRIPTOR, performance);
    let entry = v8::FunctionTemplate::new(scope, illegal_constructor);
    entries::install_performance_entry_template_bindings(scope, entry, "PerformanceEntry");
    finish_template(scope, interfaces::PerformanceEntry::DESCRIPTOR, entry);
    let resource = v8::FunctionTemplate::new(scope, illegal_constructor);
    resource.inherit(entry);
    entries::install_performance_entry_template_bindings(
        scope,
        resource,
        "PerformanceResourceTiming",
    );
    finish_template(
        scope,
        interfaces::PerformanceResourceTiming::DESCRIPTOR,
        resource,
    );
    for interface in [
        interfaces::PerformanceMark::DESCRIPTOR,
        interfaces::PerformanceMeasure::DESCRIPTOR,
        interfaces::PerformanceNavigationTiming::DESCRIPTOR,
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        template.inherit(if interface.name() == "PerformanceNavigationTiming" {
            resource
        } else {
            entry
        });
        entries::install_performance_entry_template_bindings(scope, template, interface.name());
        legacy::install(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    for interface in [
        interfaces::PerformanceNavigation::DESCRIPTOR,
        interfaces::PerformanceTiming::DESCRIPTOR,
        interfaces::PerformanceObserverEntryList::DESCRIPTOR,
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        legacy::install(scope, template, interface.name());
        if interface.name() == "PerformanceObserverEntryList" {
            entry_list::install(scope, template);
        }
        finish_template(scope, interface, template);
    }
    let object = v8::Object::new(scope);
    let prototype = moli_v8_util::global_constructor_prototype(scope, "Performance").unwrap();
    object.set_prototype(scope, prototype.into()).unwrap();
    interfaces::initialize(scope, object, "Performance").unwrap();
    publish(scope, "performance", object);
    eval(
        scope,
        "function check(ok, label) { if (!ok) throw Error(label); }",
    );
    object
}

#[test]
fn performance_names_and_entry_lists_keep_utf16_identity_and_stable_order() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let owner = install(scope);
    let mut values = Vec::new();
    for (name, kind, start) in [
        ("'\u{fffd}'", "mark", 5.0),
        ("'\\ud800'", "mark", 1.0),
        ("'\\ud800'", "measure", 1.0),
        ("'😀'", "mark", 3.0),
    ] {
        let value = eval(scope, name);
        let name = perf::PerformanceEntryName::convert(
            scope,
            value,
            moli_webidl::Context::argument("test", 1),
            &Default::default(),
        )
        .unwrap();
        if start == 3.0 {
            assert!(name == perf::PerformanceEntryName::from("😀"));
            assert_eq!(name.as_str(), Some("😀"));
        } else if start == 1.0 {
            assert_eq!(name.as_str(), None);
        }
        let entry = entries::create_performance_entry(scope, owner, kind, name, start, 2.0, None);
        values.push(entry.into());
    }
    let array = v8::Array::new_with_elements(scope, &values);
    let list = entry_list::build(scope, array).unwrap();
    publish(scope, "list", list);
    eval(
        scope,
        r#"
      const entries = list.getEntries();
      check(entries.map(e => e.startTime).join(',') === '1,1,3,5', 'stable time order');
      check(entries[0] instanceof PerformanceMark && entries[1] instanceof PerformanceMeasure, 'tie order and brands');
      check(list.getEntriesByName('\ud800').length === 2, 'lone surrogate lookup');
      check(list.getEntriesByName('\ufffd').length === 1, 'replacement char is distinct');
      check(list.getEntriesByName('\ud800', 'mark').length === 1, 'combined filter');
      check(entries[0].toJSON().name.charCodeAt(0) === 0xd800, 'lossless JSON');
      entries.pop();
      check(list.getEntries().length === 4, 'fresh result arrays');
      let failed = false;
      try { PerformanceObserverEntryList.prototype.getEntries.call({}); } catch (e) { failed = e instanceof TypeError; }
      check(failed, 'native receiver check');
    "#,
    );
}

#[test]
fn performance_resource_and_navigation_json_read_native_state() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let owner = install(scope);
    let navigation = entries::create_performance_entry(
        scope,
        owner,
        "navigation",
        "https://example.test/".into(),
        0.0,
        8.0,
        None,
    );
    entries::initialize_resource_timing_slots(
        scope,
        navigation,
        "navigation",
        100.0,
        80.0,
        120.0,
        "blocking",
        200.0,
        "text/html",
    );
    legacy::PerformanceNavigationTimingSlotDeclaration::new("reload")
        .initialize(scope, navigation)
        .unwrap();
    perf::set_performance_entry_slot_number(
        scope,
        navigation,
        legacy::NAV_LOAD_EVENT_END_SLOT,
        8.0,
    );
    publish(scope, "navigation", navigation);
    eval(
        scope,
        r#"
      for (const key of ['name', 'duration', 'responseStatus', 'loadEventEnd', 'type']) {
        Object.defineProperty(navigation, key, {get() {throw Error('author shadow ' + key);}});
      }
      const json = navigation.toJSON();
      check(json.name === 'https://example.test/' && json.duration === 8, 'entry state');
      check(json.responseStatus === 200 && json.transferSize === 100 && json.decodedBodySize === 120, 'resource state');
      check(json.type === 'reload' && json.loadEventEnd === 8, 'navigation state');
      check(navigation instanceof PerformanceResourceTiming && navigation instanceof PerformanceEntry, 'inheritance');
    "#,
    );
}

#[test]
fn legacy_timing_and_memory_keep_snapshot_semantics_without_a_page() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    let mut snapshot = legacy::PerformanceTimingSnapshot::new(100.75);
    snapshot.dom_interactive = 105;
    snapshot.load_event_end = 109;
    let timing = legacy::PerformanceTimingObjectDeclaration::from(snapshot)
        .bind(scope)
        .unwrap();
    publish(scope, "timing", timing);
    let navigation = legacy::PerformanceNavigationObjectDeclaration::new(2.0, 3.0)
        .bind(scope)
        .unwrap();
    publish(scope, "legacyNavigation", navigation);
    eval(
        scope,
        r#"
      check(timing instanceof PerformanceTiming, 'legacy timing prototype');
      const json = timing.toJSON();
      check(json.navigationStart === 100 && json.domInteractive === 105 && json.loadEventEnd === 109, 'timing projection');
      check(PerformanceNavigation.TYPE_BACK_FORWARD === 2 && legacyNavigation.type === 2, 'legacy constants');
      check(legacyNavigation.toJSON().redirectCount === 3, 'legacy native state');
      const first = performance.memory, second = performance.memory;
      check(first !== second && Object.getPrototypeOf(first) === Object.getPrototypeOf(second), 'memory snapshot identity');
      check(Object.prototype.toString.call(first) === '[object MemoryInfo]', 'memory tag');
      check(first.totalJSHeapSize === second.totalJSHeapSize && first.usedJSHeapSize === second.usedJSHeapSize, 'coarse cached sample');
      check(first.totalJSHeapSize >= first.usedJSHeapSize && first.jsHeapSizeLimit > 0, 'heap bounds');
    "#,
    );
}

#[test]
fn performance_factory_uses_owners_realm_and_immutable_intrinsics() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let first = v8::Context::new(scope, Default::default());
    let token = v8str(scope, "same origin");
    first.set_security_token(token.into());
    let owner = {
        let scope = &mut v8::ContextScope::new(scope, first);
        let owner = install(scope);
        let prototype =
            moli_v8_util::global_constructor_prototype(scope, "PerformanceMark").unwrap();
        publish(scope, "originalMarkPrototype", prototype);
        eval(
            scope,
            "Object.defineProperty(globalThis, 'PerformanceMark', {get() {throw Error('author constructor');}})",
        );
        owner
    };
    let second = v8::Context::new(scope, Default::default());
    second.set_security_token(token.into());
    {
        let scope = &mut v8::ContextScope::new(scope, second);
        install(scope);
        let mark = entries::create_performance_entry(
            scope,
            owner,
            "mark",
            "cross-realm".into(),
            2.0,
            0.0,
            None,
        );
        assert!(mark.get_creation_context(scope).unwrap() == first);
        set_private_value(scope, owner, "testMark", mark.into());
    }
    let scope = &mut v8::ContextScope::new(scope, first);
    let mark = moli_v8_util::get_private_value(scope, owner, "testMark").unwrap();
    let mark = v8::Local::<v8::Object>::try_from(mark).unwrap();
    publish(scope, "mark", mark);
    eval(
        scope,
        "check(Object.getPrototypeOf(mark) === originalMarkPrototype && mark.name === 'cross-realm', 'owner realm and canonical prototype')",
    );
}
