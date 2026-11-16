mod support;
use moli_v8_test_util::ensure_v8;
use moli_webapi::{
    crypto_key, file_list, idle_deadline, interfaces, intersection_observer_entry,
    media_capabilities, mutation_record, touch, webidl_iterator,
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
        interfaces::FileList::DESCRIPTOR,
        interfaces::Touch::DESCRIPTOR,
        interfaces::TouchList::DESCRIPTOR,
        interfaces::TouchEvent::DESCRIPTOR,
        interfaces::CryptoKey::DESCRIPTOR,
        interfaces::IdleDeadline::DESCRIPTOR,
        interfaces::MediaCapabilities::DESCRIPTOR,
        interfaces::IntersectionObserverEntry::DESCRIPTOR,
        interfaces::MutationRecord::DESCRIPTOR,
    ] {
        let template = match interface.name() {
            "Touch" => v8::FunctionTemplate::new(scope, touch::touch_constructor_callback),
            "IntersectionObserverEntry" => v8::FunctionTemplate::new(
                scope,
                intersection_observer_entry::intersection_observer_entry_constructor_callback,
            ),
            _ => v8::FunctionTemplate::new(scope, illegal_constructor),
        };
        file_list::install_file_list_template_bindings(scope, template, interface.name());
        touch::install_touch_template_bindings(scope, template, interface.name());
        media_capabilities::install_media_capabilities_template_bindings(
            scope,
            template,
            interface.name(),
        );
        match interface.name() {
            "CryptoKey" => crypto_key::install_crypto_key_template_bindings(scope, template),
            "IdleDeadline" => {
                idle_deadline::install_idle_deadline_template_bindings(scope, template)
            }
            "IntersectionObserverEntry" => intersection_observer_entry::install(scope, template),
            "MutationRecord" => mutation_record::install_template_bindings(scope, template),
            _ => {}
        }
        finish_template(scope, interface, template);
    }
    test(scope);
}
#[test]
fn standalone_file_list_sync_preserves_native_contents_and_prototype() {
    with_values(|scope| {
        let first = v8::Object::new(scope);
        let second = v8::Object::new(scope);
        publish(scope, "first", first);
        publish(scope, "second", second);
        let list = file_list::build_file_list_object(scope, &[first]).unwrap();
        publish(scope, "files", list);
        eval(
            scope,
            "if(files.length!==1 || files.item(0)!==first || [...files][0]!==first || files.item(8)!==null) throw 'FileList'; globalThis.FileList=function(){throw 'author constructor'};",
        );
        file_list::sync_file_list_contents(scope, list, &[second, first]);
        eval(
            scope,
            "if(files.length!==2 || files[0]!==second || files.item(1)!==first || [...files].length!==2) throw 'live FileList';",
        );
        file_list::sync_file_list_contents(scope, list, &[]);
        eval(
            scope,
            "if(files.length!==0 || files[0]!==undefined || files.item(0)!==null) throw 'stale FileList';",
        );
    });
}
#[test]
fn standalone_native_iterator_retains_live_set_and_captured_intrinsics() {
    with_values(|scope| {
        let global = scope.get_current_context().global(scope);
        webidl_iterator::install_webidl_collection_iterator_intrinsics(scope, global).unwrap();
        let set = v8::Set::new(scope);
        set.add(scope, v8::Integer::new(scope, 1).into()).unwrap();
        publish(scope, "values", set.into());
        let iterator = webidl_iterator::new_setlike_webidl_iterator(
            scope,
            set,
            webidl_iterator::SetlikeWebIdlIteratorKind::CustomStateSet,
            webidl_iterator::SetlikeWebIdlIteratorMethod::Values,
        )
        .unwrap();
        publish(scope, "iterator", iterator);
        eval(
            scope,
            "if(iterator.next().value!==1) throw 'first'; values.add(2); Set.prototype.values=function(){throw 'changed intrinsic'}; Reflect.apply=function(){throw 'changed apply'}; if(iterator.next().value!==2 || !iterator.next().done || Object.getPrototypeOf(iterator).hasOwnProperty('constructor')) throw 'iterator state';",
        );
    });
}
#[test]
fn standalone_crypto_key_keeps_visible_snapshots_separate_from_native_material() {
    with_values(|scope| {
        let algorithm = crypto_key::build_symmetric_algorithm_object(scope, "AES-GCM", 128);
        let key = crypto_key::new_crypto_key_object(
            scope,
            "secret",
            algorithm,
            true,
            &["decrypt".into(), "encrypt".into()],
            &[7; 16],
        )
        .unwrap();
        publish(scope, "key", key);
        eval(
            scope,
            "if(key.algorithm!==key.algorithm || key.usages!==key.usages || key.type!=='secret' || key.algorithm.name!=='AES-GCM') throw 'key'; key.algorithm.name='tampered'; key.usages.length=0; globalThis.CryptoKey=function(){throw 'public constructor'};",
        );
        let payload = crypto_key::crypto_key_clone_payload_from_object(scope, key).unwrap();
        assert_eq!(payload.algorithm.name, "AES-GCM");
        assert_eq!(payload.usages, vec!["encrypt", "decrypt"]);
        assert_eq!(payload.key_bytes, vec![7; 16]);
        let cloned = crypto_key::crypto_key_object_from_clone_payload(scope, payload).unwrap();
        publish(scope, "cloned", cloned);
        eval(
            scope,
            "if(cloned===key || cloned.algorithm.name!=='AES-GCM' || cloned.usages.join()!=='encrypt,decrypt') throw 'clone';",
        );
    });
}
#[test]
fn standalone_observer_entry_and_idle_deadline_need_no_document_host() {
    with_values(|scope| {
        eval(
            scope,
            "const target={}; const rect={width:4}; const entry=new IntersectionObserverEntry({time:3,target,rootBounds:null,boundingClientRect:rect,intersectionRect:rect,isIntersecting:true,intersectionRatio:0.5}); if(entry.target!==target || entry.boundingClientRect!==rect || entry.intersectionRatio!==0.5 || entry.time!==3) throw 'entry';",
        );
        let deadline = idle_deadline::build(scope, 0., 0., true).unwrap();
        publish(scope, "deadline", deadline);
        eval(
            scope,
            "if(!deadline.didTimeout || deadline.timeRemaining()!==0) throw 'deadline';",
        );
        let capabilities =
            media_capabilities::build_media_capabilities_object(scope, true, false).unwrap();
        publish(scope, "capabilities", capabilities);
        eval(
            scope,
            "globalThis.outcome='pending'; capabilities.decodingInfo({type:'invalid'}).then(()=>outcome='unexpected',e=>outcome=e instanceof TypeError?'pass':String(e));",
        );
        scope.perform_microtask_checkpoint();
        assert_eq!(eval(scope, "outcome").to_rust_string_lossy(scope), "pass");
    });
}
