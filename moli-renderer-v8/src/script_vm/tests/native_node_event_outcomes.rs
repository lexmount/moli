use super::window_node_worlds::child_node_world;
use super::*;

async fn native_outcome_vm(policy: moli_page_types::LayoutPolicy) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://native-node-outcomes.test/");
    vm.set_layout_policy(policy);
    vm.eval("if(!document.body){const root=document.documentElement||document.appendChild(document.createElement('html'));root.appendChild(document.createElement('body'));}'ready'").unwrap();
    child_node_world(&mut vm, "native-node-outcomes").await;
    vm
}

#[tokio::test(flavor = "current_thread")]
async fn retired_form_reset_and_native_click_preserve_control_state() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = native_outcome_vm(policy).await;
        assert_eq!(vm.eval(r#"(()=>{
            frame.contentDocument.body.innerHTML='<form id=form><input id=value value=initial></form><input id=check type=checkbox>';
            globalThis.oldForm=frame.contentDocument.getElementById('form');
            globalThis.oldInput=frame.contentDocument.getElementById('value');
            globalThis.oldCheck=frame.contentDocument.getElementById('check');
            globalThis.resetCalls=0;globalThis.clickCalls=0;
            oldForm.addEventListener('reset',()=>resetCalls++);
            oldCheck.addEventListener('click',()=>clickCalls++);
            oldInput.value='changed';oldForm.reset();
            const result=[oldInput.value,resetCalls];
            oldInput.value='changed-again';oldCheck.indeterminate=true;
            return JSON.stringify(result);
        })()"#).unwrap(),r#"["initial",1]"#,"live reset control: {policy:?}");
        vm.eval("frame.srcdoc='<body>replacement</body>'; 'navigate'")
            .unwrap();
        run_child_navigation_commit_and_host_load_for_test(
            &mut vm,
            "native actions after frame retirement",
        )
        .await;
        assert_eq!(
            vm.eval(
                r#"(()=>{
            oldForm.reset();oldCheck.click();
            const event=new MouseEvent('click',{cancelable:true});
            const result=oldCheck.dispatchEvent(event);
            return JSON.stringify([oldInput.value,resetCalls,clickCalls,oldCheck.checked,
                oldCheck.indeterminate,result,event.defaultPrevented,event.target===null,
                frame.contentDocument.body.textContent]);
        })()"#
            )
            .unwrap(),
            r#"["changed-again",1,0,false,true,false,false,true,"replacement"]"#,
            "retired controls must not execute native defaults: {policy:?}"
        );
    }
}

fn node_handle(vm: &mut ScriptVm, name: &'static str) -> DomHandle {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let node = global
            .get(scope, crate::util::v8str(scope, name).into())
            .unwrap();
        let node = v8::Local::<v8::Object>::try_from(node).unwrap();
        Ok(crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, node)?.1)
    })
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn native_beforeinput_cancellation_does_not_forge_event_default_prevented() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = native_outcome_vm(policy).await;
        vm.eval(r#"
            globalThis.retiredTarget=frame.contentDocument.getElementById('childTarget');
            globalThis.liveTarget=document.createElement('div');document.body.append(liveTarget);
            globalThis.inertTarget=document.implementation.createHTMLDocument('inert').createElement('div');
            globalThis.preventedTarget=document.createElement('div');document.body.append(preventedTarget);
            globalThis.calls=[];
            for(const [target,name] of [[retiredTarget,'retired'],[liveTarget,'live'],[inertTarget,'inert'],[preventedTarget,'prevented']]){
                target.addEventListener('beforeinput',e=>{calls.push(name);if(name==='prevented')e.preventDefault();});
            }'ready'
        "#).unwrap();
        let handles = [
            "retiredTarget",
            "liveTarget",
            "inertTarget",
            "preventedTarget",
        ]
        .map(|name| node_handle(&mut vm, name));
        vm.eval("frame.srcdoc='<body>replacement</body>'; 'navigate'")
            .unwrap();
        run_child_navigation_commit_and_host_load_for_test(
            &mut vm,
            "beforeinput target retirement",
        )
        .await;
        let results = vm
            .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
                Ok(handles.map(|handle| {
                    let event = crate::native_bridge::element::construct_input_event(
                        scope,
                        "beforeinput",
                        crate::native_bridge::element::TextEditInputType::InsertText,
                        Some("X"),
                    )
                    .unwrap();
                    let outcome = crate::native_bridge::element::dispatch_public_event(
                        scope, host_ptr, handle, event,
                    );
                    let prevented = crate::context_bootstrap::event_bool_attribute(
                        scope,
                        event,
                        "defaultPrevented",
                    );
                    let target_null = event
                        .get(scope, crate::util::v8str(scope, "target").into())
                        .unwrap()
                        .is_null();
                    let current_target_null = event
                        .get(scope, crate::util::v8str(scope, "currentTarget").into())
                        .unwrap()
                        .is_null();
                    (
                        outcome.allows_default(),
                        outcome.had_exception(),
                        prevented,
                        target_null,
                        current_target_null,
                    )
                }))
            })
            .unwrap();
        assert_eq!(
            results,
            [
                (false, false, false, true, true),
                (true, false, false, false, true),
                (true, false, false, false, true),
                (false, false, true, false, true)
            ],
            "{policy:?}"
        );
        assert_eq!(
            vm.eval("JSON.stringify(calls)").unwrap(),
            r#"["live","inert","prevented"]"#
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn native_contenteditable_edit_refuses_a_retired_document_before_mutation() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = native_outcome_vm(policy).await;
        vm.eval(r#"
            globalThis.retiredTarget=frame.contentDocument.getElementById('childTarget');retiredTarget.contentEditable='true';
            globalThis.liveTarget=document.createElement('div');liveTarget.contentEditable='true';
            liveTarget.textContent='live';document.body.append(liveTarget);
            globalThis.calls=[];
            for(const [target,name] of [[retiredTarget,'retired'],[liveTarget,'live']]){
                for(const type of ['beforeinput','input'])target.addEventListener(type,()=>calls.push(name+':'+type));
            }'ready'
        "#).unwrap();
        let retired = node_handle(&mut vm, "retiredTarget");
        let live = node_handle(&mut vm, "liveTarget");
        vm.eval("frame.srcdoc='<body>replacement</body>'; 'navigate'")
            .unwrap();
        run_child_navigation_commit_and_host_load_for_test(
            &mut vm,
            "native editor target retirement",
        )
        .await;
        let results = vm
            .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
                Ok([retired, live].map(|handle| {
                    crate::native_bridge::element::replace_contenteditable_selection(
                        scope,
                        host_ptr,
                        handle,
                        "X",
                        crate::native_bridge::element::TextEditInputType::InsertText,
                    )
                }))
            })
            .unwrap();
        assert_eq!(results, [false, true], "{policy:?}");
        assert_eq!(
            vm.eval("JSON.stringify([retiredTarget.textContent,liveTarget.textContent,calls])")
                .unwrap(),
            r#"["child","liveX",["live:beforeinput","live:input"]]"#,
            "{policy:?}"
        );
    }
}
