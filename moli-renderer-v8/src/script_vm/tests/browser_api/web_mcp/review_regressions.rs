//! Regressions for policy, form lifecycle, task delivery and cancellation.
use super::*;

#[test]
fn web_mcp_cancellation_rechecks_document_after_the_target_abort_listener() {
    for caller_retires in [false, true] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(r#"
            globalThis.log=[];
            globalThis.controller=new AbortController();
            globalThis.frame=document.createElement('iframe'); document.body.append(frame);
            const context=document.modelContext;
            context.addEventListener('toolcancel',()=>log.push('old toolcancel'));
            context.registerTool({{name:'wait',description:'Wait',execute(input,{{signal}}) {{
                globalThis.savedSignal=signal;
                signal.addEventListener('abort',()=>{{
                    log.push('abort');
                    document.open(); document.write('<!doctype html><body>new</body>'); document.close();
                    document.modelContext.addEventListener('toolcancel',()=>log.push('new toolcancel'));
                }});
                return new Promise(()=>{{}});
            }}}}).then(()=>context.getTools()).then(([tool])=>{{
                const caller={caller_retires} ? frame.contentDocument.modelContext : context;
                return caller.executeTool(tool,{{}},{{signal:controller.signal}});
            }}).catch(()=>{{}});
        "#)).unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("savedSignal.aborted")
                .unwrap(),
            "false"
        );
        vm.eval(if caller_retires {
            "frame.remove()"
        } else {
            "controller.abort()"
        })
        .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("savedSignal.aborted+'|'+log.join(',')")
                .unwrap(),
            "true|abort",
            "caller retirement: {caller_retires}"
        );
    }
}

#[test]
fn web_mcp_target_can_complete_before_requested_cancellation_arrives() {
    for (result, expected) in [
        (
            "return {toJSON() {log.push('serialize'); return {ok:true}}}",
            "true|serialize",
        ),
        ("throw new Error('failure')", "true|"),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(
            r#"
            globalThis.controller = new AbortController();
            globalThis.reason = {{canceled: true}};
            globalThis.probe = 'pending';
            globalThis.log = [];
            const context = document.modelContext;
            context.addEventListener('toolcancel', () => log.push('toolcancel'));
            context.registerTool({{name:'race', description:'Race', execute(_, {{signal}}) {{
                signal.addEventListener('abort', () => log.push('target abort'));
                queueMicrotask(() => controller.abort(reason));
                {result}
            }}}}).then(() => context.getTools())
                .then(([tool]) => context.executeTool(tool, {{}}, {{signal:controller.signal}}))
                .then(() => probe=false, error => probe=error===reason);
        "#
        ))
        .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe+'|'+log.join(',')")
                .unwrap(),
            expected,
            "{result}"
        );
    }
}

#[test]
fn web_mcp_result_delivery_remains_abortable_until_the_caller_task() {
    for execute in [
        "queueMicrotask(() => queueMicrotask(() => controller.abort(reason))); return 'ok'",
        "queueMicrotask(() => queueMicrotask(() => controller.abort(reason))); throw new Error('failure')",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(r#"
            globalThis.controller = new AbortController();
            globalThis.reason = {{canceled: true}};
            globalThis.probe = 'pending';
            document.modelContext.registerTool({{name: 'race', description: 'Race', execute() {{{execute}}}}})
                .then(() => document.modelContext.getTools())
                .then(([tool]) => document.modelContext.executeTool(tool, {{}}, {{signal: controller.signal}}))
                .then(value => probe = value, error => probe = error === reason);
        "#)).unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "true",
            "{execute}"
        );
    }
}

#[test]
fn web_mcp_missing_tool_and_invalid_input_delivery_remain_abortable() {
    for invocation in [
        "const invocation = context.executeTool({...tool, name: 'missing'}, {}, {signal: controller.signal}); controller.abort(reason); return invocation",
        "context.addEventListener('toolactivated', () => controller.abort(reason), {once: true}); return context.executeTool(tool, {toJSON: () => null}, {signal: controller.signal})",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(
            r#"
            globalThis.controller = new AbortController();
            globalThis.reason = {{canceled: true}};
            globalThis.probe = 'pending';
            const context = document.modelContext;
            context.registerTool({{name: 'race', description: 'Race', execute: () => 'ok'}})
                .then(() => context.getTools()).then(([tool]) => {{{invocation}}})
                .then(value => probe = value, error => probe = error === reason);
        "#
        ))
        .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "true",
            "{invocation}"
        );
    }
}

#[test]
fn web_mcp_declarative_invalidation_is_not_undone_by_restoring_the_form() {
    for mutation in [
        "form.remove(); document.body.append(form)",
        "form.removeAttribute('tooldescription'); form.setAttribute('tooldescription', 'Confirm')",
        "other.appendChild(form)",
        "other.insertBefore(form, null)",
        "other.appendChild(original)",
        "other.attachShadow({mode:'open'}).appendChild(form)",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(r#"
            document.body.innerHTML = '<section id=original><form toolname=confirm tooldescription=Confirm><input name=value><button>Submit</button></form></section><section id=other></section>';
            globalThis.form = document.querySelector('form');
            globalThis.probe = 'pending';
            globalThis.submissions = [];
            form.addEventListener('submit', event => {submissions.push(event.agentInvoked); event.preventDefault()});
        "#).unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("document.modelContext.getTools().then(([tool]) => document.modelContext.executeTool(tool, {value: 'filled'})).then(value => probe = value, error => probe = error.name)").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("form.matches(':tool-form-active')")
                .unwrap(),
            "true"
        );
        vm.eval(mutation).unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe+'|'+form.matches(':tool-form-active')")
                .unwrap(),
            "UnknownError|false",
            "{mutation}"
        );
        vm.eval("form.requestSubmit()").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("JSON.stringify(submissions)")
                .unwrap(),
            "[false]"
        );
    }
}

#[test]
fn web_mcp_captured_form_response_survives_removal_and_unrelated_mutations() {
    for mutation in [
        "",
        "document.body.appendChild(document.createElement('div'))",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(r#"
            document.body.innerHTML='<form toolname=respond tooldescription=Respond toolautosubmit><input name=value></form>';
            globalThis.form=document.querySelector('form');
            globalThis.probe='pending';
            form.addEventListener('submit', event=>{{
                event.preventDefault();
                event.respondWith(Promise.resolve('captured'));
                form.remove();
            }});
            document.modelContext.addEventListener('toolactivated',()=>{{{mutation}}});
        "#)).unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{value:'filled'})).then(value=>probe=value,error=>probe=error.name)").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe+'|'+form.matches(':tool-form-active')")
                .unwrap(),
            "captured|false",
            "{mutation}"
        );
    }
}

#[test]
fn web_mcp_removed_form_releases_its_registration_synchronously() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval("document.body.innerHTML = '<form toolname=replace tooldescription=Replace><input name=value></form>'; globalThis.probe='pending'").unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"
        document.querySelector('form').remove();
        document.modelContext.registerTool({name: 'replace', description: 'Replacement', execute: () => 'ok'})
            .then(() => document.modelContext.getTools())
            .then(([tool]) => document.modelContext.executeTool(tool))
            .then(value => probe=value, error => probe=error.name);
    "#).unwrap();
    assert_eq!(vm.eval_after_selected_page_tasks("probe").unwrap(), "ok");
}

#[test]
fn web_mcp_discovery_from_toolchange_observes_the_reregistered_form() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval("document.body.innerHTML = '<form toolname=confirm tooldescription=Confirm></form>'; globalThis.form=document.querySelector('form')").unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"
        globalThis.snapshots=[];
        document.modelContext.addEventListener('toolchange', () => {
            document.modelContext.getTools().then(tools => snapshots.push(tools.map(tool => tool.name).join('|')));
        });
        form.remove(); document.body.append(form);
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("JSON.stringify(snapshots)")
            .unwrap(),
        "[\"confirm\",\"confirm\"]"
    );
}

#[test]
fn web_mcp_native_forms_in_shadow_trees_are_discovered_and_updated() {
    for mode in ["open", "closed"] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(r#"
            globalThis.host = document.body.appendChild(document.createElement('div'));
            globalThis.shadow = host.attachShadow({{mode: '{mode}'}});
            shadow.innerHTML = '<form toolname=shadow_query tooldescription=Query toolautosubmit><input name=query><button>Submit</button></form>';
            globalThis.form = shadow.querySelector('form');
            form.addEventListener('submit', event => {{event.preventDefault(); event.respondWith(Promise.resolve(form.elements.query.value))}});
            globalThis.probe = 'pending';
        "#)).unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("document.modelContext.getTools().then(tools => probe=tools.map(tool => tool.name).join('|'))").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "shadow_query",
            "{mode}"
        );
        vm.eval("form.setAttribute('tooldescription', 'Updated')")
            .unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("document.modelContext.getTools().then(([tool]) => {probe=tool.description; return document.modelContext.executeTool(tool, {query: 'filled'})}).then(value => probe += '|'+value)").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "Updated|filled",
            "{mode}"
        );
        vm.eval("host.remove()").unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("document.modelContext.getTools().then(tools => probe=tools.length)")
            .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            "0",
            "{mode}"
        );
    }
}

#[test]
fn web_mcp_readonly_applies_only_to_controls_that_support_it() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        document.body.innerHTML = '<form toolname=readonly tooldescription=Readonly toolautosubmit><input name=text readonly><textarea name=area readonly></textarea><select name=choice readonly><option value=a>A</option><option value=b>B</option></select><input name=checked type=checkbox readonly><input name=range type=range readonly></form>';
        globalThis.form = document.querySelector('form');
        globalThis.probe = 'pending';
        form.addEventListener('submit', event => {event.preventDefault(); event.respondWith(Promise.resolve('filled'))});
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("document.modelContext.getTools().then(([tool]) => {probe=Object.keys(tool.inputSchema.properties).sort().join('|'); return document.modelContext.executeTool(tool, {choice: 'b', checked: true, range: 42})}).then(value => probe += '|'+value, error => probe += '|'+error.name)").unwrap();
    assert_eq!(vm.eval_after_selected_page_tasks("probe+'|'+form.elements.choice.value+'|'+form.elements.checked.checked+'|'+form.elements.range.value").unwrap(), "checked|choice|range|filled|b|true|42");
}

#[test]
fn web_mcp_caller_retirement_cancels_the_live_target_with_an_event() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        globalThis.frame=document.createElement('iframe'); document.body.append(frame);
        globalThis.probe=[];
        document.modelContext.addEventListener('toolcancel', event => probe.push(event.toolName));
        document.modelContext.registerTool({name:'wait',description:'Wait',execute(input,{signal}) {signal.addEventListener('abort',()=>probe.push('abort')); return new Promise(()=>{})}})
            .then(()=>frame.contentDocument.modelContext.executeTool({name:'wait',description:'Wait',window,origin:location.origin}));
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("frame.remove()").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe.join('|')")
            .unwrap(),
        "abort|wait"
    );
}

#[test]
fn web_mcp_toolchange_is_dispatched_for_each_registration() {
    for registration in [
        "Promise.all(['one','two','three'].map(name=>document.modelContext.registerTool({name,description:name,execute:()=>42})))",
        "document.body.innerHTML=['one','two','three'].map(name=>`<form toolname=${name} tooldescription=${name}></form>`).join('')",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(
            r#"
            globalThis.changes=0;
            document.modelContext.addEventListener('toolchange',()=>changes++);
            {registration};
        "#
        ))
        .unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("changes").unwrap(),
            "3",
            "{registration}"
        );
    }
}
