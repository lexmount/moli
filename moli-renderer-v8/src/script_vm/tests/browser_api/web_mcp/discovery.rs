use super::*;

#[test]
fn web_mcp_form_execution_enumerates_controls_once_before_author_events() {
    use crate::native_bridge::element::take_form_lookup_work_for_test;

    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        document.body.innerHTML='<form toolname=confirm tooldescription=Confirm><input name=value><button>Submit</button></form>';
        globalThis.form=document.querySelector('form');
        globalThis.probe='pending';
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    take_form_lookup_work_for_test();
    vm.eval("document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{value:'filled'})).catch(error=>probe=error.name)").unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    let work = take_form_lookup_work_for_test();
    assert_eq!(work.traversals, 1, "{work:?}");
    assert_eq!(
        vm.eval("form.querySelector('input').value+'|'+(document.activeElement===form.querySelector('button'))+'|'+probe")
            .unwrap(),
        "filled|true|pending"
    );
}

#[test]
fn web_mcp_adopted_external_controls_update_both_document_schemas() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        document.body.innerHTML='<form id=shared toolname=outer tooldescription=Outer></form><input name=query form=shared><iframe></iframe>';
        globalThis.input=document.querySelector('input');
        globalThis.childDocument=document.querySelector('iframe').contentDocument;
        childDocument.body.innerHTML='<form id=shared toolname=inner tooldescription=Inner></form>';
        globalThis.probe='pending';
        globalThis.capture=()=>document.modelContext.getTools().then(tools=>{
            probe=tools.map(tool=>tool.name+':'+Object.keys(tool.inputSchema.properties).sort().join(',')).sort().join('|');
        });
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("capture()").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe").unwrap(),
        "inner:|outer:query"
    );
    vm.eval("childDocument.body.appendChild(input)").unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("capture()").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe").unwrap(),
        "inner:query|outer:"
    );
    vm.eval("document.body.appendChild(input)").unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("capture()").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe").unwrap(),
        "inner:|outer:query"
    );
}

#[test]
fn web_mcp_unrelated_dom_mutations_do_not_rebuild_form_schemas() {
    use crate::native_bridge::element::take_form_lookup_work_for_test;

    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(
        r#"
        for (let i=0;i<32;i++) {
            const form=document.createElement('form');
            form.setAttribute('toolname','form_'+i);
            form.setAttribute('tooldescription','Form '+i);
            form.innerHTML='<input name=value>';
            document.body.appendChild(form);
        }
        globalThis.input=document.querySelector('input');
        globalThis.root=document.body.appendChild(document.createElement('section'));
        globalThis.probe='pending';
    "#,
    )
    .unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    take_form_lookup_work_for_test();
    vm.eval(
        r#"
        for (let i=0;i<8000;i++) root.appendChild(document.createElement('div'));
        root.setAttribute('value','unrelated');
        root.firstChild.textContent='unrelated';
        root.lastChild.remove();
    "#,
    )
    .unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    assert_eq!(take_form_lookup_work_for_test().traversals, 0);

    vm.eval("input.name='changed'").unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    let work = take_form_lookup_work_for_test();
    assert_eq!(work.traversals, 1, "one affected form among 32: {work:?}");
    vm.eval("document.modelContext.getTools().then(tools=>probe=tools.length+'|'+Object.keys(tools.find(tool=>tool.name==='form_0').inputSchema.properties).join(','))").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe").unwrap(),
        "32|changed"
    );
}

#[test]
fn web_mcp_incremental_schemas_track_external_controls_labels_and_disabled_fieldsets() {
    for mode in ["light", "open", "closed"] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(r#"
            globalThis.root=document.body;
            if ('{mode}' !== 'light') {{
                const host=document.body.appendChild(document.createElement('section'));
                root=host.attachShadow({{mode:'{mode}'}});
            }}
            root.innerHTML='<form id=one toolname=one tooldescription=One></form><form id=two toolname=two tooldescription=Two></form><label for=query>Old label</label><input id=query name=query form=one><fieldset><input name=outside form=one></fieldset>';
            globalThis.input=root.querySelector('input');
            globalThis.label=root.querySelector('label');
            globalThis.fieldset=root.querySelector('fieldset');
            globalThis.probe='pending';
            globalThis.capture=()=>document.modelContext.getTools().then(tools=>{{
                probe=tools.map(tool=>tool.name+':'+Object.keys(tool.inputSchema.properties).sort().join(',')).sort().join('|');
            }});
        "#)).unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("capture()").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "one:outside,query|two:",
            "{mode}"
        );
        vm.eval("label.firstChild.data='New label'").unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("document.modelContext.getTools().then(tools=>probe=tools.find(tool=>tool.name==='one').inputSchema.properties.query.description)").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "New label",
            "{mode}"
        );
        vm.eval("input.setAttribute('form','two'); fieldset.disabled=true")
            .unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("capture()").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "one:|two:query",
            "{mode}"
        );
        vm.eval("globalThis.wrapper=document.createElement('div'); wrapper.innerHTML='<input name=bulk form=one>'; root.appendChild(wrapper)").unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("capture()").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "one:bulk|two:query",
            "{mode}"
        );
        vm.eval("wrapper.remove(); fieldset.disabled=false")
            .unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("capture()").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "one:outside|two:query",
            "{mode}"
        );
    }
}

#[test]
fn web_mcp_indexed_discovery_preserves_shadow_including_duplicate_name_order() {
    for mode in ["open", "closed"] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(r#"
            const host=document.body.appendChild(document.createElement('section'));
            const light=document.createElement('form');
            light.setAttribute('toolname','duplicate'); light.setAttribute('tooldescription','Light');
            host.appendChild(light);
            const shadow=host.attachShadow({{mode:'{mode}'}});
            shadow.innerHTML='<form toolname=duplicate tooldescription=Shadow></form>';
            globalThis.probe='pending';
        "#)).unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("document.modelContext.getTools().then(([tool])=>probe=tool.description)")
            .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "Shadow",
            "{mode}"
        );
    }
}

#[test]
fn web_mcp_aborted_callback_registration_cannot_remove_its_form_replacement() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        globalThis.controller=new AbortController();
        globalThis.reason={removed:true};
        globalThis.probe='pending';
        document.modelContext.registerTool({name:'replace',description:'Imperative',execute:()=> 'old'},
            {signal:controller.signal}).then(()=>probe='unexpected',error=>probe=error===reason);
        controller.abort(reason);
        document.body.innerHTML='<form toolname=replace tooldescription=Declarative toolautosubmit><input name=value></form>';
        const form=document.querySelector('form');
        form.addEventListener('submit',event=>{event.preventDefault();event.respondWith(form.elements.value.value)});
    "#).unwrap();
    assert_eq!(vm.eval_after_selected_page_tasks("probe").unwrap(), "true");
    vm.eval("document.modelContext.getTools().then(([tool])=>{probe=tool.description; return document.modelContext.executeTool(tool,{value:'new'})}).then(value=>probe+='|'+value,error=>probe=error.name)").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe").unwrap(),
        "Declarative|new"
    );
}

#[test]
fn web_mcp_declarative_candidates_retry_registration_when_their_controls_change() {
    for mode in ["light", "open", "closed"] {
        for (markup, external_control, mutation) in [
            (
                "",
                false,
                "const input=document.createElement('input'); input.name='query'; form.appendChild(input)",
            ),
            (
                "<div></div>",
                false,
                "const input=document.createElement('input'); input.name='query'; form.firstChild.appendChild(input)",
            ),
            (
                "<div><input name=discard><input name=query></div>",
                false,
                "form.elements.discard.remove()",
            ),
            ("<input name=old>", false, "form.elements.old.name='query'"),
            (
                "",
                false,
                "const input=document.createElement('input'); input.name='query'; input.setAttribute('form',form.id); root.appendChild(input)",
            ),
            ("", true, "external.remove()"),
            ("", true, "external.removeAttribute('form')"),
        ] {
            let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
            vm.eval(
                r#"
                globalThis.mc=document.modelContext;
                globalThis.controller=new AbortController();
                globalThis.probe='pending';
                mc.registerTool({name:'retry',description:'Imperative',execute:()=> 'imperative'},
                    {signal:controller.signal}).then(()=>probe='registered');
            "#,
            )
            .unwrap();
            assert_eq!(
                vm.eval_after_selected_page_tasks("probe").unwrap(),
                "registered"
            );
            vm.eval(&format!(
                r#"
                globalThis.root=document.body;
                if ('{mode}' !== 'light') {{
                    const host=document.body.appendChild(document.createElement('div'));
                    root=host.attachShadow({{mode:'{mode}'}});
                }}
                globalThis.form=document.createElement('form');
                form.id='candidate';
                form.setAttribute('toolname','retry');
                form.setAttribute('tooldescription','Declarative');
                form.setAttribute('toolautosubmit','');
                form.innerHTML='{markup}';
                form.addEventListener('submit',event=>{{
                    event.preventDefault();
                    const query=form.elements.namedItem('query');
                    event.respondWith(query ? query.value : 'empty');
                }});
                root.appendChild(form);
                if ({external_control}) {{
                    globalThis.external=document.createElement('input');
                    external.name='query';
                    external.setAttribute('form',form.id);
                    root.appendChild(external);
                }}
            "#
            ))
            .unwrap();
            if external_control {
                assert_eq!(vm.eval("external.form===form").unwrap(), "true", "{mode}");
            }
            // Complete the conflicting registration attempt before freeing the
            // name, so a coalesced initial scan cannot hide a missing retry.
            vm.eval_after_selected_page_tasks("undefined").unwrap();
            vm.eval("mc.getTools().then(tools=>probe=tools.map(tool=>tool.description).join('|'))")
                .unwrap();
            assert_eq!(
                vm.eval_after_selected_page_tasks("probe").unwrap(),
                "Imperative"
            );
            vm.eval("controller.abort()").unwrap();
            vm.eval_after_selected_page_tasks("undefined").unwrap();
            vm.eval("mc.getTools().then(tools=>probe=JSON.stringify(tools))")
                .unwrap();
            assert_eq!(vm.eval_after_selected_page_tasks("probe").unwrap(), "[]");
            // An unrelated insertion must not retry this form's registration.
            vm.eval("document.body.appendChild(document.createElement('section'))")
                .unwrap();
            vm.eval_after_selected_page_tasks("undefined").unwrap();
            vm.eval("mc.getTools().then(tools=>probe=JSON.stringify(tools))")
                .unwrap();
            assert_eq!(vm.eval_after_selected_page_tasks("probe").unwrap(), "[]");

            vm.eval(mutation).unwrap();
            if external_control {
                assert_eq!(
                    vm.eval("external.form===null").unwrap(),
                    "true",
                    "{mode}: {mutation}"
                );
            }
            vm.eval_after_selected_page_tasks("undefined").unwrap();
            vm.eval(r#"
                mc.getTools().then(tools=>{
                    const tool=tools.find(tool=>tool.name==='retry');
                    probe=tool ? tool.description+'|'+Object.keys(tool.inputSchema.properties).sort().join(',') : 'missing';
                });
            "#).unwrap();
            assert_eq!(
                vm.eval_after_selected_page_tasks("probe").unwrap(),
                if external_control {
                    "Declarative|"
                } else {
                    "Declarative|query"
                },
                "{mode}: {mutation}"
            );
            let input = if external_control {
                "{}"
            } else {
                "{query:'filled'}"
            };
            vm.eval(&format!("mc.getTools().then(([tool])=>mc.executeTool(tool,{input})).then(value=>probe=value,error=>probe=error.name)"))
                .unwrap();
            assert_eq!(
                vm.eval_after_selected_page_tasks("probe").unwrap(),
                if external_control { "empty" } else { "filled" },
                "{mode}: {mutation}"
            );
        }
    }
}

#[test]
fn web_mcp_declarative_discovery_after_unrelated_dom_growth_requires_no_api_access() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        globalThis.root=document.body.appendChild(document.createElement('section'));
        for (let i=0;i<8000;i++) root.appendChild(document.createElement('div'));
        const detached=document.createElement('section');
        const shadow=detached.attachShadow({mode:'closed'});
        shadow.innerHTML='<form toolname=late tooldescription=Late toolautosubmit><input name=value></form>';
        globalThis.form=shadow.querySelector('form');
        form.addEventListener('submit',event=>{event.preventDefault();event.respondWith(form.elements.value.value)});
        root.appendChild(detached);
        globalThis.probe='pending';
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{value:'filled'})).then(value=>probe=value,error=>probe=error.name)").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe").unwrap(),
        "filled"
    );
}
