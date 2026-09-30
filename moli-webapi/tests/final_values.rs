mod support;
use moli_v8_test_util::ensure_v8;
use moli_webapi::{
    highlights, interfaces, performance::event_counts, push_values, webidl_iterator,
};
use std::pin::pin;
use support::{eval, finish_template, illegal_constructor, install_errors, publish};

fn with_values(test: impl FnOnce(&mut v8::PinScope<'_, '_>)) {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_errors(scope);
    for interface in [
        interfaces::EventCounts::DESCRIPTOR,
        interfaces::PushMessageData::DESCRIPTOR,
        interfaces::PushSubscriptionOptions::DESCRIPTOR,
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        match interface.name() {
            "EventCounts" => event_counts::install(scope, template),
            name => push_values::install_subscription_template_bindings(scope, template, name),
        }
        finish_template(scope, interface, template);
    }
    test(scope);
}

#[test]
fn standalone_event_counts_keep_native_values_snapshot_iterators_and_callback_calls() {
    with_values(|scope| {
        let mut counts = vec![0; event_counts::EVENT_COUNTS_TRACKED_TYPES.len()];
        let click = event_counts::event_counts_tracked_index("click").unwrap();
        counts[click] = 3;
        let counts = event_counts::new_event_counts(scope, counts).unwrap();
        publish(scope, "counts", counts);
        eval(
            scope,
            r#"
            if(!(counts instanceof EventCounts) || counts.get('click')!==3 || !counts.has('click'))throw 'count';
            if([...counts.keys()].length!==counts.size || counts[Symbol.iterator]!==counts.entries)throw 'iteration';
            globalThis.snapshot=counts.values();
            let calls=0;
            function callback(value,key,owner){if(this!==receiver || owner!==counts || typeof key!=='string')throw 'callback';calls++;}
            const receiver={};Object.defineProperty(callback,'call',{get(){throw 'public call'}});
            counts.forEach(callback,receiver);if(calls!==counts.size)throw 'forEach';
            const sentinel={};try{counts.forEach(()=>{throw sentinel});throw 'not propagated'}catch(e){if(e!==sentinel)throw e;}
        "#,
        );
        let values = event_counts::event_counts_values_array(scope, counts).unwrap();
        assert_eq!(
            values.set_index(scope, click as u32, v8::Integer::new(scope, 9).into()),
            Some(true)
        );
        eval(
            scope,
            "if(counts.get('click')!==9 || [...snapshot][1]!==3 || [...counts.values()][1]!==9)throw 'snapshot/live values';",
        );
    });
}

#[test]
fn standalone_highlights_keep_live_iterators_and_captured_callback_intrinsics() {
    with_values(|scope| {
        let global = scope.get_current_context().global(scope);
        webidl_iterator::install_webidl_collection_iterator_intrinsics(scope, global).unwrap();
        let state = highlights::build_highlight_runtime_state(scope).unwrap();
        publish(scope, "registry", state.registry);
        assert_eq!(
            global.set(
                scope,
                v8::String::new(scope, "Highlight").unwrap().into(),
                state.highlight_constructor
            ),
            Some(true)
        );
        for name in ["range1", "range2", "range3"] {
            let range = v8::Object::new(scope);
            interfaces::Range::DESCRIPTOR
                .initialize(scope, range)
                .unwrap();
            publish(scope, name, range);
        }
        eval(
            scope,
            r#"
            const highlight=new Highlight(range1,range2);registry.set('probe',highlight);
            if(!(highlight instanceof Highlight) || highlight.size!==2 || registry.get('probe')!==highlight)throw 'highlight values';
            const iterator=highlight.values();
            if(Object.prototype.toString.call(iterator)!=='[object Highlight Iterator]' || iterator.next().value!==range1)throw 'highlight iterator';
            highlight.delete(range2);highlight.add(range3);
            if(iterator.next().value!==range3 || !iterator.next().done)throw 'live records';
            const registryIterator=registry.entries();registry.delete('probe');registry.set('new',highlight);
            if(Object.prototype.toString.call(registryIterator)!=='[object HighlightRegistry Iterator]' || registryIterator.next().value[0]!=='new')throw 'registry iterator';
            const receiver={},seen=[];
            function callback(value,key,owner){if(this!==receiver || value!==key || owner!==highlight)throw 'callback';seen.push(value);}
            Object.defineProperty(callback,'call',{get(){throw 'public call'}});highlight.forEach(callback,receiver);
            if(seen[0]!==range1 || seen[1]!==range3)throw 'forEach';
            const sentinel={};try{highlight.forEach(()=>{throw sentinel});throw 'not propagated'}catch(e){if(e!==sentinel)throw e;}
            try{Object.getPrototypeOf(iterator).next.call({});throw 'forged receiver'}catch(e){if(!(e instanceof TypeError))throw e;}
        "#,
        );
    });
}

#[test]
fn standalone_push_values_preserve_byte_snapshots_and_need_no_worker_host() {
    with_values(|scope| {
        let message = push_values::new_push_message_data(scope, br#"{"value":42}"#).unwrap();
        publish(scope, "message", message);
        let invalid = push_values::new_push_message_data(scope, &[0xff]).unwrap();
        publish(scope, "invalid", invalid);
        let prototype =
            moli_v8_util::global_constructor_prototype(scope, "PushSubscriptionOptions").unwrap();
        let options = push_values::new_subscription_options(scope, prototype, true).unwrap();
        assert!(interfaces::PushSubscriptionOptions::is_instance(
            scope, options
        ));
        publish(scope, "options", options);
        eval(
            scope,
            r#"
            if(!(message instanceof PushMessageData) || message.json().value!==42)throw 'message';
            const bytes=message.bytes();bytes[0]=0;new Uint8Array(message.arrayBuffer())[0]=0;
            if(message.text()!=='{"value":42}' || message.bytes()[0]!==123)throw 'byte snapshot';
            if(invalid.text()!=='\ufffd')throw 'utf8';
            try{invalid.json();throw 'invalid json accepted'}catch(e){if(!(e instanceof SyntaxError))throw e;}
            const visibleOnly=Object.getOwnPropertyDescriptor(PushSubscriptionOptions.prototype,'userVisibleOnly');
            if(!options.userVisibleOnly || options.applicationServerKey!==null ||
                Object.hasOwn(options,'userVisibleOnly') || typeof visibleOnly.get!=='function' ||
                visibleOnly.set!==undefined)throw 'options';
        "#,
        );
    });
}
