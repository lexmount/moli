mod support;

use moli_v8_test_util::ensure_v8;
use moli_webapi::{headers, interfaces};
use std::pin::pin;
use support::{eval, finish_template, install_errors, publish};

fn with_headers(test: impl FnOnce(&mut v8::PinScope<'_, '_>)) {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_errors(scope);
    let template = v8::FunctionTemplate::new(scope, headers::headers_constructor_callback);
    headers::install_headers_template_bindings(scope, template);
    finish_template(scope, interfaces::Headers::DESCRIPTOR, template);
    test(scope);
}

#[test]
fn standalone_headers_keep_live_iteration_and_callback_exceptions() {
    with_headers(|scope| {
        eval(
            scope,
            r#"
        const headers = new Headers([['X-Test','one'],['x-test','two'],['Set-Cookie','a=1'],['Set-Cookie','b=2']]);
        if(headers.get('x-test')!=='one, two' || headers.getSetCookie().join('|')!=='a=1|b=2') throw 'projection';
        const iterator = headers.entries();
        if(iterator.next().value.join()!=='set-cookie,a=1') throw 'first';
        headers.append('A','live');
        if(iterator.next().value.join()!=='set-cookie,a=1') throw 'live order';
        if(Object.getPrototypeOf(iterator).hasOwnProperty('constructor')) throw 'leaked constructor';
        const expected={};
        try {headers.forEach(()=>{throw expected});throw 'missing exception'}
        catch(error) {if(error!==expected) throw error}
        try {Headers.prototype.get.call(Object.create(Headers.prototype),'x');throw 'missing brand check'}
        catch(error) {if(!(error instanceof TypeError)) throw error}
        "#,
        );
    });
}

#[test]
fn standalone_headers_guarded_native_factories_survive_global_replacement() {
    with_headers(|scope| {
        let guarded = headers::build_headers_object_with_state(
            scope,
            &[("X-Test".into(), "one".into())],
            headers::HeadersGuard::Request,
            false,
        );
        let immutable = headers::build_headers_object_with_state(
            scope,
            &[("X-Test".into(), "fixed".into())],
            headers::HeadersGuard::None,
            true,
        );
        publish(scope, "guarded", guarded);
        publish(scope, "immutable", immutable);
        eval(
            scope,
            r#"
        const NativeHeaders=Headers;
        globalThis.Headers=function(){throw 'public constructor'};
        guarded.append('Cookie','blocked'); guarded.set('X-Test','two');
        if(!(guarded instanceof NativeHeaders) || guarded.has('Cookie') || guarded.get('x-test')!=='two') throw 'guard';
        try {immutable.set('X-Test','changed');throw 'mutable response'}
        catch(error) {if(!(error instanceof TypeError)) throw error}
        if(immutable.get('x-test')!=='fixed') throw 'changed immutable';
        "#,
        );
        assert_eq!(
            headers::headers_entries(scope, guarded),
            vec![("x-test".into(), "two".into())]
        );
    });
}
