use super::*;

#[test]
fn web_mcp_declarative_candidates_retry_registration_when_their_controls_change() {
    for mode in ["light", "open", "closed"] {
        for (markup, mutation) in [
            (
                "",
                "const input=document.createElement('input'); input.name='query'; form.appendChild(input)",
            ),
            (
                "<div></div>",
                "const input=document.createElement('input'); input.name='query'; form.firstChild.appendChild(input)",
            ),
            (
                "<div><input name=discard><input name=query></div>",
                "form.elements.discard.remove()",
            ),
            ("<input name=old>", "form.elements.old.name='query'"),
            (
                "",
                "const input=document.createElement('input'); input.name='query'; input.setAttribute('form',form.id); root.appendChild(input)",
            ),
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
                    event.preventDefault(); event.respondWith(form.elements.query.value);
                }});
                root.appendChild(form);
            "#
            ))
            .unwrap();
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
            vm.eval_after_selected_page_tasks("undefined").unwrap();
            vm.eval(r#"
                mc.getTools().then(tools=>{
                    const tool=tools.find(tool=>tool.name==='retry');
                    probe=tool ? tool.description+'|'+Object.keys(tool.inputSchema.properties).sort().join(',') : 'missing';
                });
            "#).unwrap();
            assert_eq!(
                vm.eval_after_selected_page_tasks("probe").unwrap(),
                "Declarative|query",
                "{mode}: {mutation}"
            );
            vm.eval("mc.getTools().then(([tool])=>mc.executeTool(tool,{query:'filled'})).then(value=>probe=value,error=>probe=error.name)")
                .unwrap();
            assert_eq!(
                vm.eval_after_selected_page_tasks("probe").unwrap(),
                "filled",
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
