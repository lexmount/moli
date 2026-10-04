//! Declarative tool regressions and Chromium form schema cases.

use super::*;

#[test]
fn web_mcp_declarative_fills_current_choices_after_earlier_field_events() {
    for (control, input, mutation, expected) in [
        (
            "<select name=choice><option value=a>A</option><option value=b>B</option></select>",
            "'b'",
            "form.elements.choice.innerHTML='<option value=a>A</option><option value=b>B</option>'",
            r#"{"selected":["b"],"events":["input","change"]}"#,
        ),
        (
            "<select name=choice multiple><option value=a>A</option><option value=b>B</option></select>",
            "['b']",
            "form.elements.choice.innerHTML='<option value=a>A</option><option value=b>B</option>'",
            r#"{"selected":["b"],"events":["input","change"]}"#,
        ),
        (
            "<input name=choice type=checkbox value=a><input name=choice type=checkbox value=b>",
            "['b']",
            "form.elements.choice[1].value='c'",
            r#"{"selected":[],"events":["change","change"]}"#,
        ),
        (
            "<input name=choice type=radio value=a checked><input name=choice type=radio value=b>",
            "'b'",
            "form.elements.choice[1].value='c'",
            r#"{"selected":["a"],"events":[]}"#,
        ),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(r#"
            document.body.innerHTML='<form toolname=fill tooldescription=Fill toolautosubmit><input name=trigger>{control}</form>';
            globalThis.form=document.querySelector('form');
            globalThis.probe='pending';
            const events=[];
            form.elements.trigger.addEventListener('input', () => {{{mutation}}});
            for (const type of ['input','change']) form.addEventListener(type, event => {{
                if (event.target.name==='choice') events.push(type);
            }});
            form.addEventListener('submit', event => {{
                event.preventDefault();
                const controls=Array.from(form.elements).filter(control=>control.name==='choice');
                const selected=controls[0].tagName==='SELECT'
                    ? Array.from(controls[0].selectedOptions, option=>option.value)
                    : controls.filter(control=>control.checked).map(control=>control.value);
                event.respondWith({{selected,events}});
            }});
        "#)).unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval(&format!("document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{{trigger:'changed',choice:{input}}})).then(value=>probe=value,error=>probe=error.name)")).unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("probe").unwrap(),
            expected,
            "{control}"
        );
    }
}

#[test]
fn web_mcp_declarative_rejects_invalid_tool_names_on_discovery_and_mutation() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        for (const name of ['valid','', 'bad name','non_ascii_😀','x'.repeat(129)]) {
            const form=document.createElement('form');form.setAttribute('toolname',name);form.setAttribute('tooldescription','Tool');document.body.append(form);
        }
        globalThis.result='pending';
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(
        "document.modelContext.getTools().then(tools=>result=tools.map(tool=>tool.name).join('|'))",
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("result").unwrap(),
        "valid"
    );
    vm.eval("document.querySelector('form').setAttribute('toolname','invalid name')")
        .unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("document.modelContext.getTools().then(tools=>result=tools.length)")
        .unwrap();
    assert_eq!(vm.eval_after_selected_page_tasks("result").unwrap(), "0");
}

#[test]
fn web_mcp_declarative_numeric_schema_uses_effective_bounds_and_decimal_steps() {
    for (attributes, expected) in [
        (
            "type=number min=0.3 step=0.1",
            serde_json::json!({"type":"number","minimum":0.3,"multipleOf":0.1}),
        ),
        (
            "type=range min=200 max=10",
            serde_json::json!({"type":"number","minimum":200,"maximum":200,"multipleOf":1}),
        ),
        (
            "type=range value=0.5",
            serde_json::json!({"type":"number","minimum":0,"maximum":100}),
        ),
        ("type=number step=AnY", serde_json::json!({"type":"number"})),
        (
            "type=range step=AnY",
            serde_json::json!({"type":"number","minimum":0,"maximum":100}),
        ),
        (
            "type=number min=+2 max=+3 step=+0.1",
            serde_json::json!({"type":"number","multipleOf":1}),
        ),
        (
            "type=number min=0.0000000001 step=1",
            serde_json::json!({"type":"number","minimum":0.0000000001}),
        ),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!("document.body.innerHTML='<form toolname=schema tooldescription=Schema><input name=value {attributes}></form>';globalThis.result=null;")).unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("document.modelContext.getTools().then(([tool])=>result=tool.inputSchema.properties.value)").unwrap();
        let actual = vm
            .eval_after_selected_page_tasks("JSON.stringify(result)")
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            expected,
            "{attributes}"
        );
    }
}

#[test]
fn web_mcp_declarative_unchanged_values_do_not_dispatch_input_events() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        document.body.innerHTML='<form toolname=fill tooldescription=Fill toolautosubmit><input name=text value=original><textarea name=area>original</textarea><input name=check type=checkbox checked><input name=radio type=radio value=on checked><select name=choice><option value=a selected>A</option><option value=b>B</option></select><input name=number type=number value=2></form>';
        const form=document.querySelector('form');const events=[];
        for (const type of ['input','change']) form.addEventListener(type,event=>events.push(event.target.name+':'+type));
        form.addEventListener('submit',event=>{event.preventDefault();event.respondWith(events)});
        globalThis.result='pending';
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{text:'original',area:'original',check:true,radio:'on',choice:'a',number:2})).then(value=>result=value,error=>result=error.name)"#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("result").unwrap(),
        r#"["check:change","radio:change"]"#
    );
}

#[test]
fn web_mcp_declarative_converts_numbers_and_rejects_invalid_booleans_before_filling() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        document.body.innerHTML='<form toolname=fill tooldescription=Fill toolautosubmit><input name=value><input name=checked type=checkbox></form>';
        globalThis.form=document.querySelector('form');
        form.addEventListener('submit',event=>{event.preventDefault();event.respondWith(form.elements.value.value)});
        globalThis.result='pending';
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{value:0.001953125})).then(value=>result=value,error=>result=error.name)").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("result").unwrap(),
        "0.00195313"
    );
    vm.eval("document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{value:'changed',checked:0.5})).then(value=>result=value,error=>result=error.name)").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("result+'|'+form.elements.value.value")
            .unwrap(),
        "UnknownError|0.00195313"
    );
}

#[test]
fn web_mcp_declarative_document_open_discards_stale_registration_tasks() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(
        r#"document.body.innerHTML = '<form toolname="old" tooldescription="Old form"></form>'"#,
    )
    .unwrap();
    // Replace the document before its initial form registration task runs.
    vm.eval(r#"
      document.open();
      document.write('<!doctype html><body><form toolname="new" tooldescription="New form" toolautosubmit></form>');
      document.close();
      globalThis.result = 'pending';
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"document.modelContext.getTools().then(tools => result = tools.map(tool => tool.name).join('|'))"#).unwrap();
    assert_eq!(vm.eval_after_selected_page_tasks("result").unwrap(), "new");
}

#[test]
fn web_mcp_declarative_fills_controls_and_responds_in_the_form_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
      globalThis.frame = document.createElement('iframe');
      document.body.append(frame);
      frame.contentWindow.eval(`
        document.body.innerHTML = '<form toolname="fill" tooldescription="Fill controls" toolautosubmit><input name="text"><input name="number" type="number"><input name="checked" type="checkbox"><select name="selected"><option>a</option><option>b</option></select><textarea name="area"></textarea></form>';
        const form = document.querySelector('form');
        const events = [];
        for (const type of ['input','change']) form.addEventListener(type,e=>events.push(e.target.name+':'+type));
        form.addEventListener('submit',e=>{
          e.preventDefault();
          e.respondWith(Promise.resolve({agent:e.agentInvoked,realm:e instanceof SubmitEvent,
            text:form.elements.text.value,number:form.elements.number.value,
            checked:form.elements.checked.checked,selected:form.elements.selected.value,
            area:form.elements.area.value,events}));
        });
      `);
      globalThis.result = 'pending';
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,
        {text:'hello',number:42,checked:true,selected:'b',area:'world'})).then(value=>result=value,e=>result=e.name)"#).unwrap();
    let result = vm.eval_after_selected_page_tasks("result").unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["agent"], true);
    assert_eq!(result["realm"], true);
    assert_eq!(result["text"], "hello");
    assert_eq!(result["number"], "42");
    assert_eq!(result["checked"], true);
    assert_eq!(result["selected"], "b");
    assert_eq!(result["area"], "world");
    assert_eq!(result["events"].as_array().unwrap().len(), 10);
}

#[test]
fn web_mcp_declarative_uses_native_range_bounds_and_text_sanitization() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        document.body.innerHTML='<form toolname=fill tooldescription=Fill toolautosubmit><input name=amount type=range min=100 max=200 step=10 value=150><input name=text><textarea name=area></textarea></form>';
        const form=document.querySelector('form');
        form.addEventListener('submit',event=>{
            event.preventDefault();event.respondWith({amount:Number(form.elements.amount.value),text:form.elements.text.value,area:form.elements.area.value});
        });
        globalThis.result='pending';
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{amount:180,text:'x\ny',area:'a\r\nb\rc'})).then(value=>result=value,error=>result=error.name)"#).unwrap();
    let result = vm.eval_after_selected_page_tasks("result").unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({"amount":180,"text":"xy","area":"a\nb\nc"})
    );
}

#[test]
fn web_mcp_declarative_rejects_sanitized_empty_values_before_filling() {
    // Chromium's ValidateTextData/ValidateNumberData check the native input
    // sanitizer before applying any of the supplied parameters.
    for (kind, initial, invalid) in [
        ("text", "original", "\r\n"),
        ("search", "original", "\n"),
        ("password", "original", "\r"),
        ("tel", "original", "\n"),
        ("url", "https://tools.test/", " \t\n"),
        ("email", "a@tools.test", " \t\n"),
        ("number", "12", "not a number"),
        ("date", "2024-02-29", "2025-02-29"),
        ("datetime-local", "2024-02-29T12:30", "2024-02-29T25:00"),
        ("month", "2024-02", "2024-13"),
        ("week", "2024-W01", "2024-W00"),
        ("time", "12:30", "25:00"),
    ] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(&format!(
            r#"
            document.body.innerHTML='<form toolname=atomic tooldescription=Atomic toolautosubmit><input name=first value=original><input name=typed type={kind} value="{initial}"></form>';
            globalThis.form=document.querySelector('form');
            globalThis.events=[];
            for (const type of ['input','change','submit']) form.addEventListener(type,event=>{{
                events.push(type);
                if (type==='submit') {{event.preventDefault();event.respondWith('submitted');}}
            }});
            globalThis.result='pending';
            "#
        ))
        .unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval(&format!(
            r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{{first:'changed',typed:{}}})).then(value=>result=value,error=>result=error.name)"#,
            serde_json::to_string(invalid).unwrap()
        ))
        .unwrap();
        let result = vm
            .eval_after_selected_page_tasks(
                "JSON.stringify([result,form.elements.first.value,form.elements.typed.value,events])",
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            serde_json::json!(["UnknownError", "original", initial, []]),
            "input type={kind}"
        );
    }
}

#[test]
fn web_mcp_declarative_range_accepts_nonempty_values_using_native_sanitization() {
    for value in [serde_json::json!("invalid"), serde_json::json!(true)] {
        let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
        vm.eval(r#"
            document.body.innerHTML='<form toolname=fill tooldescription=Fill toolautosubmit><input name=text value=original><input name=amount type=range min=100 max=200 step=10 value=100></form>';
            globalThis.form=document.querySelector('form');
            form.addEventListener('submit',event=>{event.preventDefault();event.respondWith(form.elements.amount.value)});
            globalThis.result='pending';
        "#).unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval(&format!(
            r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{{amount:{value}}})).then(value=>result=value,error=>result=error.name)"#
        )).unwrap();
        assert_eq!(vm.eval_after_selected_page_tasks("result").unwrap(), "150");
        vm.eval(r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{text:'changed',amount:''})).then(value=>result=value,error=>result=error.name)"#).unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks(
                "result+'|'+form.elements.text.value+'|'+form.elements.amount.value"
            )
            .unwrap(),
            "UnknownError|original|150"
        );
    }
}

#[test]
fn web_mcp_declarative_select_uses_native_option_values_and_first_duplicate() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        document.body.innerHTML='<form toolname=fill tooldescription=Fill toolautosubmit><select name=single><option value=dup>first</option><option value=dup selected>second</option><option value=empty></option><option>  a\n \t b  </option></select><select name=multiple multiple><option value=dup>first</option><option value=dup>second</option></select></form>';
        globalThis.form=document.querySelector('form');
        globalThis.schema=null;
        globalThis.result='pending';
        form.addEventListener('submit',event=>{event.preventDefault();event.respondWith({index:form.elements.single.selectedIndex,multiple:Array.from(form.elements.multiple.options,option=>option.selected)})});
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"document.modelContext.getTools().then(([tool])=>{schema=tool.inputSchema;return document.modelContext.executeTool(tool,{single:'dup',multiple:['dup']})}).then(value=>result=value,error=>result=error.name)"#).unwrap();
    let result = vm.eval_after_selected_page_tasks("result").unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({"index":0,"multiple":[true,true]})
    );
    let schema: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(schema)").unwrap()).unwrap();
    assert_eq!(
        schema["properties"]["single"]["enum"],
        serde_json::json!(["dup", "dup", "empty", "a b"])
    );
    assert_eq!(schema["properties"]["single"]["anyOf"][2]["title"], "");
    assert_eq!(
        schema["properties"]["single"]["anyOf"][3]["title"],
        "  a\n \t b  "
    );
    vm.eval(r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{single:'a b'})).then(value=>result=value,error=>result=error.name)"#).unwrap();
    let result = vm.eval_after_selected_page_tasks("result").unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap()["index"],
        3
    );
}

#[test]
fn web_mcp_declarative_manual_confirmation_reset_and_response_guards() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
      document.body.innerHTML = '<form toolname="confirm" tooldescription="Confirmation"><input name="query"><button>Submit</button></form>';
      globalThis.form = document.querySelector('form');
      globalThis.probe = [];
      form.addEventListener('submit',e=>{
        probe.push(e.agentInvoked);
        try {e.respondWith('before prevention')} catch(error) {probe.push(error.name)}
        e.preventDefault();
        if (e.agentInvoked) {e.respondWith('confirmed');globalThis.savedEvent=e;}
      });
      form.requestSubmit();
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{query:'hello'})).then(value=>probe.push(value));"#).unwrap();
    assert_eq!(vm.eval_after_selected_page_tasks("form.querySelector('input').value+'|'+(document.activeElement===form.querySelector('button'))").unwrap(),"hello|true");
    vm.eval("form.querySelector('button').click()").unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"
      try {savedEvent.respondWith('late')} catch(error) {probe.push(error.name)}
      document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{query:'reset'})).catch(error=>probe.push(error.name));
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("form.reset()").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe.join('|')")
            .unwrap(),
        "false|InvalidStateError|true|InvalidStateError|confirmed|InvalidStateError|UnknownError"
    );
}

#[test]
fn web_mcp_declarative_validates_all_arguments_before_mutation_and_preserves_response_after_unregister()
 {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
      document.body.innerHTML = '<form toolname="atomic" tooldescription="Atomic fill" toolautosubmit><input name="text" value="original"><select name="choice"><option>a</option></select></form>';
      globalThis.form = document.querySelector('form');
      globalThis.probe = [];
      form.addEventListener('submit',e=>{e.preventDefault();e.respondWith(new Promise(resolve=>globalThis.finish=resolve))});
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{text:'changed',choice:'invalid'})).catch(error=>probe.push(error.name));"#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe.join('|')+'|'+form.elements.text.value")
            .unwrap(),
        "UnknownError|original"
    );
    vm.eval(r#"document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool,{text:'changed'})).then(()=>probe.push('late'),error=>probe.push(error.name));"#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("form.remove()").unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval("finish('late result')").unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe.join('|')")
            .unwrap(),
        "UnknownError|late"
    );
}

#[test]
fn web_mcp_chromium_form_schema_cases() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-wpt-compat/fixtures/wpt/upstream/webmcp/declarative/chromium-form-schema-cases.json"
    ))).unwrap();
    let mut failures = Vec::new();
    for case in fixtures["cases"].as_array().unwrap() {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://tools.test/",
            case["html"].as_str().unwrap(),
            &loader,
        );
        vm.eval("undefined").unwrap();
        vm.eval_after_selected_page_tasks("undefined").unwrap();
        vm.eval("globalThis.schema = null; document.modelContext.getTools().then(tools => schema = JSON.stringify(tools[0].inputSchema))").unwrap();
        let actual = vm.eval_after_selected_page_tasks("schema").unwrap();
        let expected = vm
            .eval(&format!(
                "JSON.stringify(JSON.parse({}))",
                serde_json::to_string(&case["expected_json"]).unwrap()
            ))
            .unwrap();
        if actual != expected {
            failures.push(format!(
                "{}\nactual: {actual}\nexpected: {expected}",
                case["name"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
