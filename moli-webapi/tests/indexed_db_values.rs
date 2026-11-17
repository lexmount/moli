mod support;
use moli_indexeddb_types::{IndexedDbError, Key};
use moli_v8_test_util::ensure_v8;
use moli_webapi::{
    indexed_db_values::{self, errors, range, record},
    interfaces,
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
    let template = v8::FunctionTemplate::new(scope, illegal_constructor);
    range::install(scope, template);
    finish_template(scope, interfaces::IDBKeyRange::DESCRIPTOR, template);
    let template = v8::FunctionTemplate::new(scope, illegal_constructor);
    let prototype = template.prototype_template(scope);
    record::install_record_template_bindings(scope, prototype, "IDBRecord");
    finish_template(scope, interfaces::IDBRecord::DESCRIPTOR, template);
    test(scope);
}
#[test]
fn standalone_key_ranges_separate_mutable_cached_bounds_from_query_snapshots() {
    with_values(|scope| {
        eval(
            scope,
            r#"
        const input=[1,'\ud800']; const r=IDBKeyRange.only(input);input[0]=2;
        if(!r.includes([1,'\ud800']) || r.includes(input) || r.lower!==r.lower || r.lower[1].charCodeAt(0)!==0xd800)throw 'key range';
        r.lower[0]=3;if(!r.includes([1,'\ud800']) || r.includes(r.lower))throw 'mutable bound changed query';
        const bytes=new Uint8Array([1,2]);const binary=IDBKeyRange.only(bytes);bytes[0]=9;new Uint8Array(binary.lower)[0]=8;
        if(!binary.includes(new Uint8Array([1,2])))throw 'binary snapshot';
        if(!Object.is(IDBKeyRange.only(-0).lower,-0))throw 'negative zero';
        const native=IDBKeyRange, only=IDBKeyRange.only;globalThis.IDBKeyRange=function(){throw 'author constructor'};
        if(!(only(4) instanceof native))throw 'intrinsic';
        "#,
        );
    });
}
#[test]
fn standalone_key_conversion_preserves_abrupt_completion_and_native_range_receivers() {
    with_values(|scope| {
        eval(
            scope,
            r#"
        const sentinel={},key=[];Object.defineProperty(key,0,{get(){throw sentinel}});
        try{IDBKeyRange.only(key);throw 'getter accepted'}catch(e){if(e!==sentinel)throw e;}
        const cycle=[];cycle.push(cycle);try{IDBKeyRange.only(cycle);throw 'cycle accepted'}catch(e){if(e.name!=='DataError')throw e;}
        try{IDBKeyRange.only(new Uint8Array(new SharedArrayBuffer(2)));throw 'shared key accepted'}catch(e){if(!(e instanceof TypeError))throw e;}
        try{IDBKeyRange.prototype.includes.call(Object.create(IDBKeyRange.prototype),1);throw 'forged'}catch(e){if(!(e instanceof TypeError))throw e;}
        "#,
        );
    });
}
#[test]
fn standalone_records_keep_key_and_value_identity_without_a_transaction_host() {
    with_values(|scope| {
        let value = v8::Object::new(scope);
        publish(scope, "value", value);
        let record = record::create_record(
            scope,
            &Key::Array(vec![Key::from(1)]),
            &Key::Binary(vec![2, 3]),
            value.into(),
        )
        .unwrap();
        publish(scope, "record", record);
        eval(
            scope,
            "if(!(record instanceof IDBRecord) || record.key!==record.key || record.value!==value || new Uint8Array(record.primaryKey).join()!=='2,3')throw 'record';record.key.push(2);if(record.key.join()!=='1,2')throw 'cached key';",
        );
        let input = eval(scope, "[1,2]");
        let key = indexed_db_values::require_idb_key(scope, input).unwrap();
        assert_eq!(key, Key::Array(vec![Key::from(1), Key::from(2)]));
    });
}
#[test]
fn standalone_indexeddb_error_projection_keeps_quota_details() {
    with_values(|scope| {
        let error = errors::request_error_object(
            scope,
            &IndexedDbError::QuotaExceeded {
                quota: 3,
                requested: 5,
            },
        );
        let error = v8::Local::<v8::Object>::try_from(error).unwrap();
        publish(scope, "error", error);
        eval(
            scope,
            "if(!(error instanceof QuotaExceededError) || error.name!=='QuotaExceededError' || error.quota!==3 || error.requested!==5)throw 'quota error';",
        );
    });
}
