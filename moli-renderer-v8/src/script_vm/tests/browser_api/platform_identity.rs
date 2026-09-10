use super::*;

#[test]
fn file_reader_fallback_inherits_event_without_a_public_progress_event_constructor() {
    let mut vm = new_storage_page_task_executor_test_vm("https://fallback-event-parent.test/");
    vm.eval(
        r#"
        // Remove the lazy property before replacing its descriptor.
        delete globalThis.ProgressEvent;
        Object.defineProperty(globalThis, 'ProgressEvent', {
          get() { throw new Error('the public constructor must not be consulted'); }
        });
        const reader = new FileReader();
        reader.onload = event => {
          Object.setPrototypeOf(event, null);
          globalThis.readEvent = event;
        };
        reader.readAsText(new Blob(['identity']));
        "#,
    )
    .expect("start a FileReader read without materializing ProgressEvent");
    assert_eq!(
        vm.eval_after_selected_page_tasks("readEvent.type").unwrap(),
        "load"
    );
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "readEvent").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let event = v8::Local::<v8::Object>::try_from(value).unwrap();
        assert_eq!(
            moli_webapi_declare::web_api_object_type(scope, event)
                .unwrap()
                .name(),
            "ProgressEvent"
        );
        assert!(moli_webapi_declare::implements_interface(
            scope, event, "Event"
        ));
        assert!(!moli_webapi_declare::implements_interface(
            scope,
            event,
            "EventTarget"
        ));
        assert_eq!(
            crate::context_bootstrap::exposed_interfaces::interface_template_build_count(
                scope,
                "ProgressEvent"
            ),
            0
        );
        Ok(())
    })
    .expect("the fallback factory should inherit the shared registry's parent metadata");
}

#[test]
fn native_receivers_survive_prototype_changes_and_reject_author_wrappers() {
    let mut vm = new_storage_test_vm("https://native-receiver-identity.test/");
    let result = vm.eval(r#"
      (() => {
        const width = Object.getOwnPropertyDescriptor(ImageData.prototype, 'width').get;
        const family = Object.getOwnPropertyDescriptor(FontFace.prototype, 'family').get;
        const hidden = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'hidden').get;
        const detached = document.implementation.createHTMLDocument('detached');
        const cases = [
          ['ImageData', new ImageData(2, 1), value => width.call(value) === 2],
          ['PerformanceEntry', new PerformanceMark('identity'), value => PerformanceEntry.prototype.toJSON.call(value).name === 'identity'],
          ['DOMMatrixReadOnly', new DOMMatrix(), value => DOMMatrixReadOnly.prototype.toFloat64Array.call(value).length === 16],
          ['FormData', new FormData(), value => FormData.prototype.has.call(value, 'x') === false],
          ['URLSearchParams', new URLSearchParams('x=1'), value => URLSearchParams.prototype.get.call(value, 'x') === '1'],
          ['Headers', new Headers({x: '1'}), value => Headers.prototype.get.call(value, 'x') === '1'],
          ['FontFace', new FontFace('Identity', 'local(Identity)'), value => family.call(value) === 'Identity'],
          ['EventTarget', new EventTarget(), value => EventTarget.prototype.dispatchEvent.call(value, new Event('test')) === true],
          ['HTMLElement', document.createElement('div'), value => hidden.call(value) === false],
          ['detached HTMLElement', detached.createElement('div'), value => hidden.call(value) === false],
        ];
        const failures = [];
        for (const [name, real, check] of cases) {
          const prototype = Object.getPrototypeOf(real);
          Object.setPrototypeOf(real, null);
          if (!check(real)) failures.push(`${name}:real`);
          for (const fake of [{}, Object.create(prototype), Object.create(real), new Proxy(real, {})]) {
            try { check(fake); failures.push(`${name}:accepted`); }
            catch (error) { if (error.name !== 'TypeError') failures.push(`${name}:${error.name}`); }
          }
        }
        return failures.join('|');
      })()
    "#).expect("native receiver identity matrix should evaluate");
    assert_eq!(result, "");
}

#[test]
fn document_type_conversion_ignores_public_constructor_and_prototype_forgery() {
    let mut vm = new_storage_test_vm("https://document-type-identity.test/");
    let result = vm.eval(r#"
      (() => {
        const prototype = DocumentType.prototype;
        const real = document.implementation.createDocumentType('html', '', '');
        Object.setPrototypeOf(real, null);
        globalThis.DocumentType = function ForgedDocumentType() {};
        Object.defineProperty(DocumentType, Symbol.hasInstance, {value() { throw Error('must not run'); }});
        const created = document.implementation.createDocument(null, '', real);
        const outcomes = [created.doctype === real];
        for (const fake of [Object.create(prototype), Object.create(real), {nodeType: 10}, new Proxy(real, {})]) {
          try { document.implementation.createDocument(null, '', fake); outcomes.push('accepted'); }
          catch (error) { outcomes.push(error.name); }
        }
        return outcomes.join('|');
      })()
    "#).expect("DocumentType conversion should use native identity");
    assert_eq!(result, "true|TypeError|TypeError|TypeError|TypeError");
}

#[test]
fn platform_subtypes_do_not_implicitly_inherit_clone_or_transfer_codecs() {
    let mut vm = new_storage_page_task_executor_test_vm("https://primary-interface-codecs.test/");
    vm.eval(
        "globalThis.futureBlob = new Blob(['x']); globalThis.futureStream = new WritableStream();",
    )
    .expect("create native base payloads");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        moli_webapi_declare::register_web_api_interfaces(
            scope,
            [
                ("FutureBlob", Some("Blob")),
                ("FutureWritableStream", Some("WritableStream")),
            ],
        )?;
        let global = scope.get_current_context().global(scope);
        for (key, interface) in [
            ("futureBlob", "FutureBlob"),
            ("futureStream", "FutureWritableStream"),
        ] {
            let key = v8::String::new(scope, key).unwrap();
            let value = global.get(scope, key.into()).unwrap();
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            moli_webapi_declare::initialize_web_api_object(scope, object, interface)?;
        }
        Ok(())
    })
    .expect("model native subinterfaces with inherited state but no codecs");
    let result = vm.eval(r#"
      (() => {
        const probe = fn => { try { fn(); return 'accepted'; } catch (error) { return error.name; } };
        const size = Object.getOwnPropertyDescriptor(Blob.prototype, 'size').get.call(futureBlob);
        const locked = Object.getOwnPropertyDescriptor(WritableStream.prototype, 'locked').get;
        return [size, probe(() => structuredClone({value: futureBlob})),
          probe(() => structuredClone(futureStream, {transfer: [futureStream]})),
          probe(() => structuredClone({}, {transfer: [futureStream]})), locked.call(futureStream)].join('|');
      })()
    "#).expect("receiver inheritance must not grant serialization capability");
    assert_eq!(
        result,
        "1|DataCloneError|DataCloneError|DataCloneError|false"
    );
    vm.eval(
        r#"
      globalThis.futureStored = 'pending';
      const open = indexedDB.open('future-interface', 1);
      open.onupgradeneeded = () => {
        const store = open.result.createObjectStore('values');
        try { store.put({value: futureBlob}, 1); futureStored = 'accepted'; }
        catch (error) { futureStored = error.name; }
      };
    "#,
    )
    .expect("schedule future native interface storage");
    assert_eq!(
        vm.eval_after_selected_page_tasks("futureStored").unwrap(),
        "DataCloneError"
    );
}
