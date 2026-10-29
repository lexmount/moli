use super::*;

fn beforeinput_target_change(mutation: &str, should_edit: bool) {
    for policy in [
        moli_page_types::LayoutPolicy::Mock,
        moli_page_types::LayoutPolicy::OnDemand,
    ] {
        let mut vm = new_storage_html_test_vm("https://beforeinput-target.test/");
        vm.set_layout_policy(policy);
        vm.eval(&r#"
            globalThis.editor=document.createElement('div');editor.contentEditable='true';
            editor.textContent='original';document.body.append(editor);editor.focus();
            const selection=getSelection();selection.selectAllChildren(editor);selection.collapseToEnd();
            globalThis.calls=[];
            editor.addEventListener('beforeinput',event=>{
                calls.push([event.type,event.data,event.defaultPrevented]);
                MUTATION
            });
            editor.addEventListener('input',event=>calls.push([event.type,event.data]));'ready'
        "#.replace("MUTATION", mutation)).unwrap();
        let builds = vm.layout_pass_observability_for_test().1;
        let edited = vm
            .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
                let global = scope.get_current_context().global(scope);
                let editor = global
                    .get(scope, crate::util::v8str(scope, "editor").into())
                    .unwrap();
                let object = v8::Local::<v8::Object>::try_from(editor).unwrap();
                let handle = crate::native_bridge::node_runtime_and_handle_from_object_or_detached(
                    scope, object,
                )?
                .1;
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
            .eval("JSON.stringify([editor.textContent,calls])")
            .unwrap();
        let expected = if should_edit {
            serde_json::json!(["originalX", [["beforeinput", "X", false], ["input", "X"]]])
        } else {
            serde_json::json!(["original", [["beforeinput", "X", false]]])
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&facts).unwrap(),
            expected,
            "{policy:?}: {mutation}"
        );
        assert_eq!(vm.layout_pass_observability_for_test().1, builds);
    }
}

#[test]
fn native_edit_stops_when_beforeinput_disconnects_the_editor() {
    beforeinput_target_change("editor.remove();", false);
}

#[test]
fn native_edit_stops_when_beforeinput_adopts_the_editor() {
    beforeinput_target_change(
        "document.implementation.createHTMLDocument('other').adoptNode(editor);",
        false,
    );
}

#[test]
fn native_edit_stops_when_beforeinput_disables_editing() {
    beforeinput_target_change("editor.contentEditable='false';", false);
}

#[test]
fn native_edit_continues_after_beforeinput_with_a_valid_target() {
    beforeinput_target_change("void 0;", true);
    beforeinput_target_change("throw new Error('beforeinput-control');", true);
}
