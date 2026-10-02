use super::*;
use crate::native_bridge::element::{self, TouchEventPoint};

const PRODUCER_PROBE: &str = include_str!("native_event_construction.js");

#[test]
fn native_keyboard_materialization_preserves_unread_author_global_binding() {
    let mut vm = new_storage_html_test_vm("https://native-keyboard-lazy.test/");
    vm.eval(r#"
        document.body.innerHTML='<input id=target>';
        const target=document.getElementById('target');
        target.focus();
        globalThis.rows=[];
        globalThis.constructorReads=0;
        globalThis.authorKeyboardGetter=()=>{constructorReads++;throw Error('author KeyboardEvent');};
        Object.defineProperty(globalThis,'KeyboardEvent',{configurable:false,get:authorKeyboardGetter});
        target.addEventListener('keydown',event=>rows.push({
            key:event.key,code:event.code,view:event.view===window,trusted:event.isTrusted,
            tag:Object.prototype.toString.call(event)}));
        'ready'
    "#).unwrap();
    vm.dispatch_key_event("keydown", "k", "KeyK", "", 0, false, false)
        .unwrap();
    let result=vm.eval(r#"JSON.stringify({reads:constructorReads,rows,
        binding:Object.getOwnPropertyDescriptor(globalThis,'KeyboardEvent').get===authorKeyboardGetter,
        configurable:Object.getOwnPropertyDescriptor(globalThis,'KeyboardEvent').configurable})"#).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({"reads":0,"binding":true,"configurable":false,
        "rows":[{"key":"k","code":"KeyK","view":true,"trusted":true,"tag":"[object KeyboardEvent]"}]})
    );
}

#[test]
fn native_event_producers_bypass_author_constructors_dictionaries_and_indexed_setters() {
    for mode in ["ordinary", "deleted", "replaced", "getter"] {
        let mut vm = new_storage_html_test_vm("https://native-event-construction.test/");
        vm.eval(PRODUCER_PROBE).unwrap();
        vm.eval(&format!("poisonNativeEventInputs('{mode}'); 'ready'"))
            .unwrap();
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime_ptr| {
            let global = scope.get_current_context().global(scope);
            let [target, peer, transfer, track, list] = [
                "nativeTarget",
                "nativePeer",
                "nativeTransfer",
                "nativeTrack",
                "nativeTrackList",
            ]
            .map(|name| {
                global
                    .get(scope, crate::util::v8str(scope, name).into())
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                    .unwrap()
            });
            let (_, handle) =
                crate::native_bridge::node_runtime_and_handle_from_object(scope, target)
                    .expect("native event target");
            let pointer = crate::runtime::RendererPointerEventProperties::default();
            let active = [
                TouchEventPoint {
                    identifier: 11,
                    x: 11.0,
                    y: 12.0,
                    target,
                    is_target_touch: true,
                },
                TouchEventPoint {
                    identifier: 12,
                    x: 21.0,
                    y: 22.0,
                    target: peer,
                    is_target_touch: false,
                },
            ];
            let events = [
                element::construct_clipboard_event(scope, "copy", transfer),
                element::construct_input_event(
                    scope,
                    "beforeinput",
                    element::TextEditInputType::InsertText,
                    Some("k"),
                ),
                element::construct_mouse_event_with_detail_and_modifiers(
                    scope,
                    "mousemove",
                    11.0,
                    12.0,
                    2,
                    0,
                    1,
                    0,
                ),
                element::construct_pointer_event(scope, "pointerdown", 11.0, 12.0, 0, 1, &pointer),
                element::construct_drag_event(scope, "dragstart", 11.0, 12.0, transfer.into(), 0),
                element::construct_wheel_event(scope, "wheel", 11.0, 12.0, 4.0, 5.0, 0, 1, 0),
                element::construct_touch_event_with_points(
                    scope,
                    "touchstart",
                    &active,
                    &active[..1],
                ),
                element::construct_keyboard_event(
                    scope, "keydown", "k", "KeyK", false, false, false, false, false,
                ),
                element::construct_focus_event(scope, "focusin", Some(peer.into()), true),
                element::construct_simple_event(scope, "change", true, false, false),
                element::construct_submit_event(
                    scope,
                    runtime_ptr,
                    handle,
                    Some(target.into()),
                    true,
                    true,
                ),
                element::construct_command_event(scope, "--test", peer.into()),
                element::construct_toggle_event(
                    scope,
                    "beforetoggle",
                    "closed",
                    "open",
                    true,
                    peer.into(),
                ),
                element::construct_interest_event(scope, "interest", peer.into()),
            ];
            for event in events {
                let event = event.expect("native event should construct without author code");
                assert!(
                    !element::dispatch_public_event(scope, runtime_ptr, handle, event)
                        .had_exception()
                );
            }
            assert!(element::dispatch_text_track_list_event(
                scope, list, track, "addtrack"
            ));
            Ok(())
        })
        .unwrap();
        let facts: serde_json::Value = serde_json::from_str(
            &vm.eval("JSON.stringify(readNativeEventProducerProbe())")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(facts["reads"], 0, "{mode}: {facts}");
        let rows = facts["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 15, "{mode}: {facts}");
        for row in rows {
            for (name, value) in row["checks"].as_object().unwrap() {
                assert_eq!(value, true, "{mode} {name}: {row}");
            }
        }
    }
}

#[tokio::test]
async fn native_ui_events_use_target_document_window_and_leave_synthetic_views_unchanged() {
    let mut vm = native_ui_test_vm().await;
    assert_eq!(
        vm.eval(include_str!("native_ui_view.js")).unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults)").unwrap()
    );
}

#[tokio::test]
async fn native_activation_events_follow_target_documents_across_realms_and_adoption() {
    let mut vm = native_ui_test_vm().await;
    assert_eq!(
        vm.eval(include_str!("native_activation_realms.js"))
            .unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults)").unwrap()
    );
}

#[tokio::test]
async fn native_form_events_use_target_realms_and_bypass_author_construction_hooks() {
    let mut vm = native_ui_test_vm().await;
    assert_eq!(
        vm.eval(include_str!("native_form_event_realms.js"))
            .unwrap(),
        "true",
        "{}",
        vm.eval("JSON.stringify(__uiEventResults)").unwrap()
    );
}

#[test]
fn native_form_data_events_preserve_unread_author_constructor_bindings() {
    let mut vm = new_storage_html_test_vm("https://native-form-data-lazy.test/");
    let result = vm
        .eval(
            r#"(() => {
        const Data=FormData;
        const form=document.body.appendChild(document.createElement('form'));
        form.innerHTML='<input name=field value=value>';
        let reads=0, captured;
        const getter=()=>{reads++;throw Error('author FormDataEvent');};
        Object.defineProperty(globalThis,'FormDataEvent',{configurable:false,get:getter});
        form.addEventListener('formdata',event=>{
          captured=event;
          event.formData.append('listener','updated');
        });
        const result=new Data(form);
        const binding=Object.getOwnPropertyDescriptor(globalThis,'FormDataEvent');
        return reads===0 && binding.get===getter && !binding.configurable &&
          Object.prototype.toString.call(captured)==='[object FormDataEvent]' &&
          captured.isTrusted && captured.target===form && captured.formData instanceof Data &&
          captured.formData!==result && result.get('listener')==='updated';
    })()"#,
        )
        .unwrap();
    assert_eq!(result, "true");
}

async fn native_ui_test_vm() -> crate::runtime::PageVmTaskExecutorTestHarness {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://native-ui-target-window.test/",
        &loader,
    );
    vm.eval(
        r#"
        if (!document.documentElement) document.appendChild(document.createElement('html'));
        if (!document.body) document.documentElement.appendChild(document.createElement('body'));
        const frame=document.body.appendChild(document.createElement('iframe'));
        frame.id='child';
        frame.srcdoc='<body></body>';
        void frame.contentWindow;
        'ready'
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            ChildFrameSemanticTurnKind::RealmMaterialization,
            &loader,
        )
        .await
        .unwrap()
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    vm
}

#[test]
fn native_click_projects_target_document_events_into_isolated_listener_worlds() {
    let mut vm = new_storage_html_test_vm("https://native-click-worlds.test/");
    vm.eval("document.body.innerHTML='<button id=target>click</button>'; 'ready'")
        .unwrap();
    let isolated = vm.create_isolated_world("native-click", false).unwrap();
    let observe = r#"
        globalThis.nativeClickRows=[];
        const Ui=UIEvent, Mouse=MouseEvent, Pointer=PointerEvent, target=document.getElementById('target');
        target.addEventListener('click',event=>nativeClickRows.push({
            ui:event instanceof Ui,mouse:event instanceof Mouse,view:event.view===window,
            pointer:event instanceof Pointer,pointerId:event.pointerId===-1,
            pointerType:event.pointerType==='',primary:event.isPrimary===false,
            offsets:event.offsetX===0 && event.offsetY===0,detail:event.detail===0,
            target:event.target===target,current:event.currentTarget===target,
            currentEvent:window.event===event,trusted:event.isTrusted===false}));
        'ready'
    "#;
    vm.eval(observe).unwrap();
    vm.eval_in_isolated_context(isolated, observe).unwrap();
    vm.eval("target.click(); 'clicked'").unwrap();
    vm.eval_in_isolated_context(isolated, "target.click(); 'clicked'")
        .unwrap();
    for result in [
        vm.eval("JSON.stringify(nativeClickRows)").unwrap(),
        vm.eval_in_isolated_context(isolated, "JSON.stringify(nativeClickRows)")
            .unwrap(),
    ] {
        let rows: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 2, "{rows}");
        for row in rows.as_array().unwrap() {
            for (field, value) in row.as_object().unwrap() {
                assert_eq!(value, true, "{field}: {row}");
            }
        }
    }
}

#[test]
fn native_pointer_click_preserves_identity_without_copying_contact_properties() {
    let mut vm = new_storage_html_test_vm("https://native-pointer-click.test/");
    vm.eval(r#"
        document.body.innerHTML='<button id=target>click</button>';
        const target=document.getElementById('target'), NativePointer=PointerEvent;
        globalThis.nativePointerClicks=[];
        target.addEventListener('click',event=>nativePointerClicks.push({
            pointer:event instanceof NativePointer,view:event.view===window,
            id:event.pointerId===17,type:event.pointerType==='pen',
            pressure:event.pressure===0 && event.tangentialPressure===0,
            tilt:event.tiltX===0 && event.tiltY===0 && event.twist===0,
            contact:event.width===1 && event.height===1 && event.isPrimary===false,
            coordinates:event.clientX===31 && event.clientY===42,
            detail:event.detail===2,modifiers:event.ctrlKey && event.shiftKey,
            trusted:event.isTrusted,target:event.target===target,
            sequences:event.getCoalescedEvents().length===0 && event.getPredictedEvents().length===0}));
        'ready'
    "#).unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime_ptr| {
        let global = scope.get_current_context().global(scope);
        let target = global
            .get(scope, crate::util::v8str(scope, "target").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .unwrap();
        let (_, handle) =
            crate::native_bridge::node_runtime_and_handle_from_object(scope, target).unwrap();
        let pointer = crate::runtime::RendererPointerEventProperties {
            pointer_id: 17,
            pointer_type: "pen".to_owned(),
            pressure: 0.75,
            tangential_pressure: 0.25,
            tilt_x: 23.0,
            tilt_y: -17.0,
            twist: 31.0,
        };
        let outcome = element::activate_handle_after_pointer_release(
            scope,
            runtime_ptr,
            handle,
            31.0,
            42.0,
            0,
            0,
            2,
            10,
            &pointer,
        );
        assert!(outcome.handled);
        Ok(())
    })
    .unwrap();
    let facts: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(nativePointerClicks)").unwrap()).unwrap();
    let rows = facts.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{facts}");
    for (field, value) in rows[0].as_object().unwrap() {
        assert_eq!(value, true, "{field}: {facts}");
    }
}

#[test]
fn native_click_materialization_preserves_unread_author_pointer_binding() {
    let mut vm = new_storage_html_test_vm("https://native-click-lazy.test/");
    assert_eq!(
        vm.eval(r#"(() => {
            const target=document.body.appendChild(document.createElement('button'));
            let captured, reads=0;
            const getter=()=>{reads++;throw Error('author PointerEvent');};
            Object.defineProperty(globalThis,'PointerEvent',{configurable:false,get:getter});
            target.addEventListener('click',event=>captured=event);
            target.click();
            const binding=Object.getOwnPropertyDescriptor(globalThis,'PointerEvent');
            captured.initMouseEvent('probe',false,false,null,0,0,0,31,42,false,false,false,false,0,null);
            const author=new MouseEvent('probe',{clientX:31,clientY:42});
            return reads===0 && binding.get===getter && !binding.configurable &&
                Object.prototype.toString.call(captured)==='[object PointerEvent]' &&
                captured.pointerId===-1 && captured.pointerType==='' && !captured.isTrusted &&
                captured.offsetX===0 && captured.offsetY===0 &&
                captured.clientX===31 && captured.clientY===42 && author.offsetX===31 && author.offsetY===42;
        })()"#).unwrap(),
        "true"
    );
}

#[test]
fn native_form_events_share_mutations_with_isolated_listener_worlds() {
    let mut vm = new_storage_html_test_vm("https://native-form-worlds.test/");
    vm.eval("document.body.innerHTML='<form id=form><input id=input name=field value=initial><button id=submit type=submit></button></form>'; 'ready'")
        .unwrap();
    let isolated = vm.create_isolated_world("native-form", false).unwrap();
    let observe = r#"
        globalThis.nativeFormRows=[];
        const E=Event, S=SubmitEvent, D=FormDataEvent, F=FormData;
        const form=document.getElementById('form'), input=document.getElementById('input'),
          submit=document.getElementById('submit');
        for (const type of ['submit','reset','invalid','formdata']) {
          const target=type==='invalid'?input:form;
          target.addEventListener(type,event=> {
            const Constructor=type==='submit'?S:type==='formdata'?D:E;
            const checks={realm:event instanceof E,typed:event instanceof Constructor,
              prototype:Object.getPrototypeOf(event)===Constructor.prototype,
              constructor:event.constructor===Constructor,
              target:event.target===target,current:event.currentTarget===target,trusted:event.isTrusted};
            if (type==='submit') checks.submitter=event.submitter===submit;
            if (type==='formdata') {
              checks.payload=event.formData instanceof F;
              checks.payloadIdentity=event.formData===event.formData;
              event.formData.append(globalThis.formWorld,'updated');
            } else event.preventDefault();
            nativeFormRows.push(checks);
          });
        }
        'ready'
    "#;
    vm.eval("globalThis.formWorld='main'; 'ready'").unwrap();
    vm.eval_in_isolated_context(isolated, "globalThis.formWorld='isolated'; 'ready'")
        .unwrap();
    vm.eval(observe).unwrap();
    vm.eval_in_isolated_context(isolated, observe).unwrap();
    let actions = r#"(() => {
        input.value='edited'; form.requestSubmit(submit); form.reset();
        const resetCanceled=input.value==='edited';
        input.required=true; input.value=''; const valid=input.checkValidity();
        input.required=false; input.value='edited'; const result=new FormData(form);
        return resetCanceled && !valid && result instanceof FormData &&
          result.get('field')==='edited' && result.get('main')==='updated' && result.get('isolated')==='updated';
    })()"#;
    assert_eq!(vm.eval(actions).unwrap(), "true");
    assert_eq!(
        vm.eval_in_isolated_context(isolated, actions).unwrap(),
        "true"
    );
    for result in [
        vm.eval("JSON.stringify(nativeFormRows)").unwrap(),
        vm.eval_in_isolated_context(isolated, "JSON.stringify(nativeFormRows)")
            .unwrap(),
    ] {
        let rows: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 8, "{rows}");
        for row in rows.as_array().unwrap() {
            for (field, value) in row.as_object().unwrap() {
                assert_eq!(value, true, "{field}: {row}");
            }
        }
    }
}

#[test]
fn native_keyboard_editing_preserves_listener_worlds_defaults_and_cancellation() {
    for cancel in [false, true] {
        let mut vm = new_storage_html_test_vm("https://native-keyboard.test/");
        vm.eval(
            r#"
            document.body.innerHTML = '<input id=target>';
            document.getElementById('target').focus();
            globalThis.keyboardRows = [];
            function observeNativeKeyboard(cancel) {
              const Keyboard = KeyboardEvent, Input = InputEvent, Ui = UIEvent;
              const target = document.getElementById('target');
              for (const type of ['keydown','keypress','keyup','beforeinput','input']) {
                target.addEventListener(type, e => {
                  keyboardRows.push({type, view:e.view===window, target:e.target===target,
                    current:e.currentTarget===target, currentEvent:window.event===e,
                    ui:e instanceof Ui, typed:e instanceof (type.includes('input') ? Input : Keyboard),
                    trusted:e.isTrusted});
                  if (cancel && type==='keydown') e.preventDefault();
                });
              }
              Object.defineProperty(globalThis,'KeyboardEvent',{configurable:true,get(){throw Error('author KeyboardEvent')}});
              Object.defineProperty(globalThis,'InputEvent',{configurable:true,get(){throw Error('author InputEvent')}});
              Object.defineProperty(Object.prototype,'location',{configurable:true,get(){throw Error('author location')}});
              Object.defineProperty(Object.prototype,'view',{configurable:true,get(){throw Error('author view')}});
            }
            'ready'
        "#,
        )
        .unwrap();
        let isolated = vm.create_isolated_world("native-keyboard", false).unwrap();
        vm.eval_in_isolated_context(
            isolated,
            r#"
            globalThis.keyboardRows=[];
            const Keyboard=KeyboardEvent, Input=InputEvent, Ui=UIEvent;
            const target=document.getElementById('target');
            for (const type of ['keydown','keypress','keyup','beforeinput','input']) {
              target.addEventListener(type,e=>keyboardRows.push({type,view:e.view===window,
                target:e.target===target,current:e.currentTarget===target,currentEvent:window.event===e,
                ui:e instanceof Ui,typed:e instanceof (type.includes('input')?Input:Keyboard),trusted:e.isTrusted}));
            }
            'ready'
        "#,
        )
        .unwrap();
        vm.eval(&format!("observeNativeKeyboard({cancel}); 'installed'"))
            .unwrap();
        vm.dispatch_key_event("keydown", "k", "KeyK", "k", 0, false, true)
            .unwrap();
        vm.dispatch_key_event("keyup", "k", "KeyK", "", 0, false, false)
            .unwrap();
        let expected = if cancel {
            vec!["keydown", "keyup"]
        } else {
            vec!["keydown", "keypress", "beforeinput", "input", "keyup"]
        };
        for result in [
            vm.eval("JSON.stringify(keyboardRows)").unwrap(),
            vm.eval_in_isolated_context(isolated, "JSON.stringify(keyboardRows)")
                .unwrap(),
        ] {
            let rows: serde_json::Value = serde_json::from_str(&result).unwrap();
            let types: Vec<_> = rows
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["type"].as_str().unwrap())
                .collect();
            assert_eq!(types, expected, "{rows}");
            for row in rows.as_array().unwrap() {
                for field in [
                    "view",
                    "target",
                    "current",
                    "currentEvent",
                    "ui",
                    "typed",
                    "trusted",
                ] {
                    assert_eq!(row[field], true, "{cancel}: {row}");
                }
            }
        }
        assert_eq!(
            vm.eval("document.getElementById('target').value").unwrap(),
            if cancel { "" } else { "k" }
        );
    }
}

#[test]
fn touch_constructor_retains_subclass_prototype_and_native_receiver_identity() {
    let mut vm = new_storage_html_test_vm("https://native-touch.test/");
    assert_eq!(vm.eval(r#"
        (() => {
          const TouchConstructor=Touch;
          class ExtendedTouch extends TouchConstructor {}
          Object.defineProperty(globalThis,'Touch',{configurable:true,get(){throw Error('author Touch')}});
          const touch=new ExtendedTouch({identifier:7,target:document.body,clientX:11});
          return Object.getPrototypeOf(touch)===ExtendedTouch.prototype &&
            touch instanceof ExtendedTouch && touch instanceof TouchConstructor &&
            touch.identifier===7 && touch.target===document.body && touch.clientX===11;
        })()
    "#).unwrap(), "true");
}
