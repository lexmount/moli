use super::native_node_event_outcomes::{native_outcome_vm, node_handle};

async fn beforeinput_target_change(mutation: &str, should_edit: bool) {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = native_outcome_vm(policy).await;
        let script=r#"
            globalThis.editor=frame.contentDocument.getElementById('childTarget');editor.contentEditable='true';
            globalThis.editCalls=[];
            editor.addEventListener('beforeinput',e=>{
                editCalls.push([e.type,e.data,e.defaultPrevented]);
                MUTATION
            });
            editor.addEventListener('input',e=>editCalls.push([e.type,e.data]));'ready'
        "#.replace("MUTATION",mutation);
        vm.eval(&script).unwrap();
        let handle = node_handle(&mut vm, "editor");
        let edited = vm
            .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
                Ok(
                    crate::native_bridge::element::replace_contenteditable_selection(
                        scope,
                        host_ptr,
                        handle,
                        "X",
                        crate::native_bridge::element::TextEditInputType::InsertText,
                    ),
                )
            })
            .unwrap();
        assert_eq!(edited, should_edit, "{policy:?}: {mutation}");
        let facts = vm
            .eval("JSON.stringify([editor.textContent,editCalls])")
            .unwrap();
        let expected = if should_edit {
            serde_json::json!(["childX", [["beforeinput", "X", false], ["input", "X"]]])
        } else {
            serde_json::json!(["child", [["beforeinput", "X", false]]])
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&facts).unwrap(),
            expected,
            "{policy:?}: {mutation}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn native_edit_stops_when_beforeinput_removes_its_frame() {
    beforeinput_target_change("frame.remove();", false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_edit_stops_when_beforeinput_adopts_the_editor_into_another_document() {
    beforeinput_target_change("document.adoptNode(editor);document.body.append(editor);editor.focus();const s=getSelection();s.selectAllChildren(editor);s.collapseToEnd();",false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_edit_stops_when_beforeinput_disables_editing() {
    beforeinput_target_change("editor.contentEditable='false';", false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_edit_continues_with_a_valid_target_and_after_a_callback_exception() {
    beforeinput_target_change("void 0;", true).await;
    beforeinput_target_change("throw new Error('beforeinput-control');", true).await;
}

#[tokio::test(flavor = "current_thread")]
async fn native_click_revalidates_the_target_after_focus_before_preactivation() {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
        moli_page_types::LayoutPolicy::Realtime,
    ] {
        let mut vm = native_outcome_vm(policy).await;
        vm.eval(
            r#"
            globalThis.check=frame.contentDocument.createElement('input');check.type='checkbox';
            check.indeterminate=true;frame.contentDocument.body.append(check);
            globalThis.focusCalls=0;globalThis.clickCalls=0;
            check.addEventListener('focus',()=>{focusCalls++;frame.remove();});
            check.addEventListener('click',()=>clickCalls++);'ready'
        "#,
        )
        .unwrap();
        let handle = node_handle(&mut vm, "check");
        let handled = vm
            .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
                Ok(crate::native_bridge::element::activate_handle_via_click(
                    scope, host_ptr, handle, 0.0, 0.0, 0, 0,
                )
                .handled)
            })
            .unwrap();
        assert!(!handled, "retired focus target: {policy:?}");
        assert_eq!(vm.eval("check.defaultChecked=true;JSON.stringify([focusCalls,clickCalls,check.checked,check.indeterminate,document.getElementById('frame')===null])").unwrap(),
            "[1,0,true,true,true]","focus callback must not allow hidden preactivation mutations: {policy:?}");
    }
}
