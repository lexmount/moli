use super::*;

#[tokio::test]
async fn element_navigation_targets_follow_owning_document_and_attribute_presence() {
    let loader = static_http_loader([]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://target-resolution.test/source",
        &loader,
    );
    vm.eval(include_str!(
        "../../../../../../tests/fixtures/navigation-target-resolution.js"
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm, &loader,
        "String(targetResolutionFrames.every(f => f.contentDocument.URL === 'about:srcdoc' && f.contentDocument.readyState === 'complete'))",
        "true", "target resolution source and named destination frames",
    ).await;
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
        .await
        .unwrap();
    vm.eval("installTargetResolutionListeners()").unwrap();
    for child in [false, true] {
        let source = if child { "source-document" } else { "source" };
        let base = if child {
            "child-base"
        } else {
            "base-destination"
        };
        for kind in ["form", "button", "input", "a", "area"] {
            let mut cases = vec![
                (None, None, "none", base),
                (
                    Some(""),
                    None,
                    "none",
                    if matches!(kind, "a" | "area") {
                        base
                    } else {
                        source
                    },
                ),
                (Some("form-destination"), None, "none", "form-destination"),
                (None, None, "base-change", "submitter-destination"),
                (
                    Some("form-destination"),
                    None,
                    "target-change",
                    "submitter-destination",
                ),
                (None, None, "first-base-empty", source),
                (Some("raw<name"), None, "none", "raw<name"),
                (Some("raw\nname"), None, "none", "raw\nname"),
            ];
            if matches!(kind, "button" | "input") {
                cases.extend([
                    (Some("form-destination"), Some(""), "none", source),
                    (None, Some(""), "none", source),
                    (
                        Some("form-destination"),
                        Some("submitter-destination"),
                        "none",
                        "submitter-destination",
                    ),
                    (
                        Some("form-destination"),
                        Some("submitter-destination"),
                        "submitter-empty",
                        source,
                    ),
                ]);
            }
            for (target, override_target, mutation, expected) in cases {
                let description = format!(
                    "child={child}, kind={kind}, target={target:?}, override={override_target:?}, mutation={mutation}"
                );
                vm.eval(&format!(
                    "submitTargetResolutionCase({child},{kind:?},{},{},{mutation:?})",
                    serde_json::json!(target),
                    serde_json::json!(override_target)
                ))
                .unwrap();
                advance_page_task_executor_until_eval_equals(
                    &mut vm,
                    &loader,
                    "String(targetResolutionLog.length)",
                    "1",
                    &description,
                )
                .await;
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(
                        &vm.eval("JSON.stringify(targetResolutionLog)").unwrap()
                    )
                    .unwrap(),
                    serde_json::json!([expected]),
                    "{description}",
                );
            }
        }
    }
}

#[test]
fn empty_targets_preserve_link_fallback_and_form_overrides() {
    for kind in ["a", "area", "form", "button", "input"] {
        let mut vm = new_storage_test_vm("https://target-presence.test/source");
        vm.eval(&format!(r#"
            const html = document.createElement('html');
            const head = document.createElement('head'), body = document.createElement('body');
            html.append(head, body); document.append(html);
            const base = document.createElement('base'); base.target='_blank'; head.append(base);
            const kind={kind:?}, link=kind==='a'||kind==='area';
            const element=document.createElement(link?kind:'form');
            element[link?'href':'action']='about:blank';
            element.target=link||kind==='form'?'':'_blank'; body.append(element);
            if(link) element.click();
            else if(kind==='form') element.submit();
            else {{const control=document.createElement(kind);control.type='submit';control.formTarget='';element.append(control);control.click()}}
        "#)).unwrap();
        let activations = vm.take_pending_popup_activations();
        if matches!(kind, "a" | "area") {
            assert_eq!(
                activations.len(),
                1,
                "{kind}: an empty link target inherits base target"
            );
            assert_eq!(activations[0].target_name(), "_blank");
        } else {
            assert!(
                activations.is_empty(),
                "{kind}: an explicitly empty target selects the current navigable"
            );
        }
    }
}

#[test]
fn element_navigation_target_sanitizes_only_tab_or_newline_with_less_than() {
    for kind in ["a", "area", "form", "button", "input", "base"] {
        for (value, expected) in [
            ("named<window", "named<window"),
            ("named\nwindow", "named\nwindow"),
            ("named\twindow", "named\twindow"),
            ("named<\nwindow", "_blank"),
            ("named<\rwindow", "_blank"),
            ("named<\twindow", "_blank"),
            ("named<\u{c}window", "named<\u{c}window"),
        ] {
            let mut vm = new_storage_test_vm("https://target-sanitization.test/source");
            vm.eval(&format!(r#"
                const html=document.createElement('html'),head=document.createElement('head'),body=document.createElement('body');
                html.append(head,body);document.append(html);
                const kind={kind:?},value={value:?},link=kind==='a'||kind==='area';
                const element=document.createElement(link?kind:'form');
                element[link?'href':'action']='about:blank';body.append(element);
                if(kind==='base'){{const base=document.createElement('base');base.target=value;head.append(base);element.submit()}}
                else if(link){{element.target=value;element.click()}}
                else if(kind==='form'){{element.target=value;element.submit()}}
                else{{const control=document.createElement(kind);control.type='submit';control.formTarget=value;element.append(control);control.click()}}
            "#)).unwrap();
            let activations = vm.take_pending_popup_activations();
            assert_eq!(activations.len(), 1, "{kind}, {value:?}");
            assert_eq!(activations[0].target_name(), expected, "{kind}, {value:?}");
        }
    }
}

#[tokio::test]
async fn formdata_target_changes_keep_the_original_document_ancestor_permissions() {
    for (initial, final_target, allowed) in [("_top", "_self", true), ("_self", "_top", false)] {
        let loader = static_http_loader([]);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            "https://target-permissions.test/source",
            &loader,
        );
        load_form_navigation_frames_with_sandbox(
            &mut vm,
            &loader,
            1,
            Some("allow-scripts allow-same-origin allow-forms"),
        )
        .await;
        vm.eval(&format!(r#"
            globalThis.targetPermissionLog=[];
            navigation.onnavigate=event=>{{targetPermissionLog.push('top');event.preventDefault()}};
            const owner=formNavigationSource;
            owner.defaultView.navigation.onnavigate=event=>{{targetPermissionLog.push('child');event.preventDefault()}};
            const base=owner.createElement('base');base.target={initial:?};owner.head.append(base);
            const form=owner.createElement('form');form.action='https://target-permissions.test/submitted';owner.body.append(form);
            form.onformdata=()=>{{base.target={final_target:?};formNavigationFrames[0].removeAttribute('sandbox')}};
            form.submit();
        "#)).unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 100)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &vm.eval("JSON.stringify(targetPermissionLog)").unwrap()
            )
            .unwrap(),
            if allowed {
                serde_json::json!(["child"])
            } else {
                serde_json::json!([])
            },
            "base target {initial} to {final_target}; changing the container attribute does not lift the active Document sandbox"
        );
    }
}
