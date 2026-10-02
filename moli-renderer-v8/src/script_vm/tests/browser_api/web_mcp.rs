use super::*;
mod declarative;
mod review_regressions;

#[test]
fn web_mcp_get_tools_completes_after_the_current_microtasks() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(
        r#"
        globalThis.probe=[];
        document.modelContext.getTools().then(()=>probe.push('tools'));
        queueMicrotask(()=>probe.push('microtask'));
        probe.push('sync');
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe.join('|')")
            .unwrap(),
        "sync|microtask|tools"
    );
}

#[test]
fn web_mcp_schema_serialization_cannot_replace_a_reentrant_registration() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        globalThis.inner='pending';globalThis.outer='pending';globalThis.result='pending';
        document.modelContext.registerTool({name:'same',description:'Outer',execute:()=> 'outer',inputSchema:{toJSON(){
            document.modelContext.registerTool({name:'same',description:'Inner',execute:()=> 'inner'}).then(()=>inner='registered',error=>inner=error.name);
            return {};
        }}}).then(()=>outer='registered',error=>outer=error.name);
        document.modelContext.getTools().then(([tool])=>document.modelContext.executeTool(tool)).then(value=>result=value);
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("inner+'|'+outer+'|'+result")
            .unwrap(),
        "registered|InvalidStateError|inner"
    );
}

#[test]
fn web_mcp_interface_descriptors_inheritance_and_receivers_match_idl() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    let failures = vm.eval(r#"
        const failures=[];
        function check(value,label) {if (!value) failures.push(label);}
        function typeError(callback,label) {
            try {callback();failures.push(label+' did not throw');}
            catch (error) {check(error instanceof TypeError,label+' error type');}
        }
        const model=document.modelContext;
        check(model===document.modelContext,'SameObject');
        check(model instanceof ModelContext && model instanceof EventTarget,'ModelContext inheritance');
        check(Object.getPrototypeOf(ModelContext.prototype)===EventTarget.prototype,'prototype parent');
        check(Object.getPrototypeOf(ModelContext)===EventTarget,'constructor parent');
        check(Object.prototype.toString.call(model)==='[object ModelContext]','object brand');
        typeError(()=>new ModelContext(),'illegal constructor');
        const documentAttribute=Object.getOwnPropertyDescriptor(Document.prototype,'modelContext');
        check(documentAttribute.enumerable && documentAttribute.configurable && !documentAttribute.set,'document attribute');
        typeError(()=>documentAttribute.get.call({}),'document attribute receiver');
        for (const [name,length] of [['registerTool',1],['getTools',0],['executeTool',1]]) {
            const property=Object.getOwnPropertyDescriptor(ModelContext.prototype,name);
            check(property.enumerable && property.configurable && property.writable,name+' descriptor');
            check(property.value.length===length,name+' length');
        }
        for (const name of ['ontoolchange','ontoolactivated','ontoolcancel']) {
            const property=Object.getOwnPropertyDescriptor(ModelContext.prototype,name);
            check(property.enumerable && property.configurable && typeof property.get==='function' && typeof property.set==='function',name+' descriptor');
            check(model[name]===null,name+' default');
            typeError(()=>property.get.call({}),name+' receiver');
        }
        for (const constructor of [ToolActivatedEvent,ToolCancelEvent]) {
            const event=new constructor('test',{toolName:'echo'});
            check(event instanceof Event && event.toolName==='echo',constructor.name+' event');
            check(Object.getPrototypeOf(constructor.prototype)===Event.prototype,constructor.name+' prototype parent');
            const property=Object.getOwnPropertyDescriptor(constructor.prototype,'toolName');
            check(property.enumerable && property.configurable && !property.set,constructor.name+' readonly attribute');
            typeError(()=>property.get.call({}),constructor.name+' receiver');
        }
        JSON.stringify(failures);
    "#).unwrap();
    assert_eq!(failures, "[]", "IDL contract failures: {failures}");
}

#[test]
fn web_mcp_get_tools_omits_schemas_that_serialize_to_json_primitives() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        globalThis.result='pending';
        Promise.all([null,3,true,'text',[],{}].map((value,index)=>
            document.modelContext.registerTool({name:'tool_'+index,description:'JSON schema',inputSchema:{toJSON:()=>value},execute:()=>42})
        )).then(()=>document.modelContext.getTools()).then(tools=>{
            result=tools.map(tool=>tool.name).join('|')+'|'+Array.isArray(tools[0].inputSchema);
        });
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("result").unwrap(),
        "tool_4|tool_5|true"
    );
    // Filtering discovery must not delete the actual registration.
    vm.eval(r#"document.modelContext.executeTool({name:'tool_0',description:'JSON schema',window,origin:location.origin}).then(value=>result=value,error=>result=error.name)"#).unwrap();
    assert_eq!(vm.eval_after_selected_page_tasks("result").unwrap(), "42");
}

#[test]
fn web_mcp_execution_creates_input_and_options_in_the_callback_realm() {
    for (callback_window, tool_window) in [
        ("frame.contentWindow", "parent"),
        ("window", "parent.frame.contentWindow"),
    ] {
        for callback in ["execute", "execute.bind(null)", "new Proxy(execute, {})"] {
            let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
            vm.eval(&format!(
                r#"
                globalThis.frame = document.createElement('iframe');
                document.body.append(frame);
                globalThis.probe = 'pending';
                const execute = {callback_window}.eval(`(() => {{
                    const execute = (input, options) => {{
                        const nested = Array.isArray(input) ? input[0] : input.nested;
                        const array = Array.isArray(input) ? input[1] : input.items;
                        return {{
                            root: Object.getPrototypeOf(input) === (Array.isArray(input) ? Array.prototype : Object.prototype),
                            nested: Object.getPrototypeOf(nested) === Object.prototype,
                            array: Object.getPrototypeOf(array) === Array.prototype,
                            element: Object.getPrototypeOf(array[0]) === Object.prototype,
                            options: Object.getPrototypeOf(options) === Object.prototype,
                            signal: options.signal instanceof {tool_window}.AbortSignal
                        }};
                    }};
                    return {callback};
                }})()`);
                {tool_window}.document.modelContext.registerTool({{
                    name: 'realm', description: 'Cross-realm arguments', execute
                }})
                    .then(() => document.modelContext.getTools())
                    .then(([tool]) => Promise.all([
                        {{nested: {{value: 42}}, items: [{{value: 42}}]}},
                        [{{value: 42}}, [{{value: 42}}]]
                    ].map(input => document.modelContext.executeTool(tool, input))))
                    .then(results => probe = results.join('|'), error => probe = error.name);
            "#
            ))
            .unwrap();
            let expected = r#"{"root":true,"nested":true,"array":true,"element":true,"options":true,"signal":true}"#;
            assert_eq!(
                vm.eval_after_selected_page_tasks("probe").unwrap(),
                format!("{expected}|{expected}"),
                "callback {callback} in {callback_window}, tool in {tool_window}"
            );
        }
    }
}

#[test]
fn web_mcp_execution_checks_origin_and_preaborted_signal_before_lookup() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        globalThis.probe=[];
        globalThis.serialized=0;
        const reason={aborted:true};
        const signal=AbortSignal.abort(reason);
        const input={toJSON(){serialized++;throw new Error('must not serialize invalid origins')}};
        const tool={name:'missing',description:'Missing',window,origin:location.origin};
        document.modelContext.executeTool({...tool,origin:'invalid'},input).catch(error=>probe.push(error.name));
        document.modelContext.executeTool({...tool,origin:'data:text/plain,opaque'},input).catch(error=>probe.push(error.name));
        document.modelContext.executeTool(tool,{}, {signal}).catch(error=>probe.push(error===reason));
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe.join('|')+'|'+serialized")
            .unwrap(),
        "NotSupportedError|NotSupportedError|true|0"
    );
}

#[test]
fn web_mcp_registration_rechecks_document_after_schema_serialization() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        globalThis.result='pending';
        document.modelContext.registerTool({name:'fresh',description:'Fresh document',execute:()=>42,inputSchema:{toJSON(){
            document.open();document.write('<!doctype html><body>replacement');document.close();return {type:'object'};
        }}}).then(()=>document.modelContext.getTools()).then(([tool])=>document.modelContext.executeTool(tool)).then(value=>result=value,error=>result=error.name);
    "#).unwrap();
    assert_eq!(vm.eval_after_selected_page_tasks("result").unwrap(), "42");
}

#[test]
fn web_mcp_execution_rechecks_caller_detached_during_input_serialization() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        globalThis.frame=document.createElement('iframe');document.body.append(frame);
        globalThis.calls=0;
        globalThis.result='pending';
        const context=frame.contentDocument.modelContext;
        document.modelContext.registerTool({name:'echo',description:'Echo',execute:()=>{calls++;return 42}})
            .then(()=>context.executeTool({name:'echo',description:'Echo',window,origin:location.origin},{toJSON(){frame.remove();return {}}}))
            .then(value=>result=value,error=>result=error.name);
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("result+'|'+calls")
            .unwrap(),
        "InvalidStateError|0"
    );
}

#[test]
fn web_mcp_root_devtools_cannot_invoke_tools_from_a_popup_frame_tree() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
        globalThis.popup = window.open('about:blank');
        globalThis.registration = 'pending';
        popup.document.modelContext.registerTool({name:'popup_only',description:'Popup tool',execute:()=>42})
            .then(()=>registration='registered',error=>registration=error.name);
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("registration").unwrap(),
        "registered"
    );
    vm.dispatch_web_mcp_command(
        moli_page_types::DevToolsSessionKey::from_wire_session_id(None),
        moli_page_types::RendererWebMcpCommand::Enable,
    )
    .unwrap()
    .unwrap();
    let result = vm
        .dispatch_web_mcp_command(
            moli_page_types::DevToolsSessionKey::from_wire_session_id(None),
            moli_page_types::RendererWebMcpCommand::InvokeTool {
                frame_id: None,
                name: "popup_only".into(),
                input: "{}".into(),
            },
        )
        .unwrap();
    assert_eq!(
        result,
        Err(moli_page_types::RendererWebMcpError::InvalidParams(
            "Tool not found"
        ))
    );
}

#[test]
fn web_mcp_is_exposed_only_in_secure_window_contexts() {
    for (url, expected) in [
        ("https://tools.test/", "true"),
        ("http://tools.test/", "false"),
        ("http://localhost/", "true"),
    ] {
        let mut vm = new_storage_test_vm(url);
        assert_eq!(vm.eval("'modelContext' in Document.prototype && typeof ModelContext === 'function' && typeof ToolActivatedEvent === 'function' && typeof ToolCancelEvent === 'function'").unwrap(), expected);
    }
}

#[test]
fn web_mcp_rejects_argument_exceptions_without_a_synchronous_throw() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(
        r#"
      globalThis.probe = [];
      const reason = new Error('getter failed');
      document.modelContext.registerTool({ get description() { throw reason; } })
        .catch(e => probe.push(e === reason));
      ModelContext.prototype.getTools.call({}).catch(e => probe.push(e instanceof TypeError));
      document.modelContext.getTools({fromOrigins: ['http://untrusted.test']})
        .catch(e => probe.push(e.name === 'SecurityError'));
      document.modelContext.getTools({fromOrigins: ['not a URL']})
        .catch(e => probe.push(e.name === 'SecurityError'));
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe.join('|')")
            .unwrap(),
        "true|true|true|true"
    );
}

#[test]
fn web_mcp_browser_tasks_cannot_be_canceled_with_js_timer_handles() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
      globalThis.probe = 'pending';
      document.modelContext.registerTool({name: 'echo', description: 'echo', execute: input => input})
        .then(() => document.modelContext.getTools())
        .then(([tool]) => document.modelContext.executeTool(tool, {value: 42}))
        .then(result => probe = result);
      for (let id = 1; id < 32; id++) clearTimeout(id);
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe").unwrap(),
        r#"{"value":42}"#
    );
}

#[test]
fn web_mcp_browser_tasks_ignore_indexed_prototype_accessors() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
      globalThis.probe = 'pending';
      globalThis.interceptions = 0;
      for (let index = 0; index < 3; index++) {
        Object.defineProperty(Array.prototype, index, {
          configurable: true,
          get() { interceptions++; throw new Error('indexed getter'); },
          set() { interceptions++; throw new Error('indexed setter'); }
        });
      }
      const controller = new AbortController();
      document.modelContext.registerTool({name: 'echo', description: 'echo', execute: input => input}, {signal: controller.signal})
        .then(() => document.modelContext.getTools())
        .then(([tool]) => document.modelContext.executeTool(tool, {value: 42}))
        .then(result => { probe = result; controller.abort(); return document.modelContext.getTools(); })
        .then(tools => probe += '|' + tools.length + '|' + interceptions);
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe").unwrap(),
        r#"{"value":42}|0|0"#
    );
}

#[test]
fn web_mcp_document_open_retires_old_tools_and_reuses_the_model_context() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
      globalThis.oldContext = document.modelContext;
      globalThis.oldController = new AbortController();
      document.modelContext.registerTool({name: 'echo', description: 'old', execute: () => 'old'}, {signal: oldController.signal});
    "#).unwrap();
    vm.eval_after_selected_page_tasks("undefined").unwrap();
    vm.eval(
        r#"
      document.open(); document.write('<!doctype html><body>new</body>'); document.close();
      globalThis.probe = [];
      probe.push(oldContext === document.modelContext);
      document.modelContext.getTools().then(tools => probe.push(tools.length === 0));
      document.modelContext.registerTool({name: 'echo', description: 'new', execute: () => 'new'})
        .then(() => { oldController.abort(); return document.modelContext.getTools(); })
        .then(([tool]) => document.modelContext.executeTool(tool))
        .then(result => probe.push(result === 'new'));
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe.join('|')")
            .unwrap(),
        "true|true|true"
    );
}

#[test]
fn web_mcp_about_blank_tools_inherit_origin_and_execute_in_the_child_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
      globalThis.frame = document.createElement('iframe');
      document.body.append(frame);
      frame.contentWindow.eval(`document.modelContext.registerTool({
        name: 'child', description: 'child realm',
        execute: (input, {signal}) => ({value: input.value, correctRealm: signal instanceof AbortSignal})
      })`);
      globalThis.probe = 'pending';
      document.modelContext.getTools()
        .then(([tool]) => document.modelContext.executeTool(tool, {value: 42}))
        .then(result => probe = result);
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe").unwrap(),
        r#"{"value":42,"correctRealm":true}"#
    );
}

#[test]
fn web_mcp_detached_child_rejects_pending_execution_and_aborts_its_signal() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(
        r#"
      globalThis.frame = document.createElement('iframe');
      document.body.append(frame);
      frame.contentWindow.eval(`document.modelContext.registerTool({
        name: 'wait', description: 'pending child', execute: (input, {signal}) => {
          parent.savedSignal = signal;
          return new Promise(() => {});
        }
      })`);
      globalThis.probe = [];
      globalThis.childContext = frame.contentDocument.modelContext;
      globalThis.retainedWindow = frame.contentWindow;
      retainedWindow.sentinel = 42;
      document.modelContext.getTools()
        .then(([tool]) => document.modelContext.executeTool(tool))
        .catch(error => probe.push(error.name));
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("savedSignal.aborted")
            .unwrap(),
        "false"
    );
    vm.eval(
        r#"
      frame.remove();
      document.modelContext.getTools().then(tools => probe.push(tools.length === 0));
      childContext.getTools().catch(error => probe.push(error.name));
    "#,
    )
    .unwrap();
    assert_eq!(vm.eval("retainedWindow.sentinel").unwrap(), "42");
    let result = vm
        .eval_after_selected_page_tasks("JSON.stringify({probe, aborted: savedSignal.aborted})")
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["aborted"], true, "{result}");
    let probe = result["probe"].as_array().unwrap();
    assert_eq!(probe.len(), 3, "{result}");
    assert!(
        probe.contains(&serde_json::json!("UnknownError")),
        "{result}"
    );
    assert!(
        probe.contains(&serde_json::json!("InvalidStateError")),
        "{result}"
    );
    assert!(probe.contains(&serde_json::json!(true)), "{result}");
}

#[test]
fn web_mcp_document_open_rejects_unacknowledged_registration() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(
        r#"
      globalThis.probe = [];
      document.modelContext.registerTool({name: 'echo', description: 'old', execute: () => 'old'})
        .then(() => probe.push('old accepted'), error => probe.push(error.name));
      document.open(); document.write('<!doctype html><body>new</body>'); document.close();
      document.modelContext.registerTool({name: 'echo', description: 'new', execute: () => 'new'})
        .then(() => probe.push('new accepted'));
    "#,
    )
    .unwrap();
    let result = vm
        .eval_after_selected_page_tasks("probe.sort().join('|')")
        .unwrap();
    assert_eq!(result, "AbortError|new accepted");
}

#[test]
fn web_mcp_document_open_rejects_started_execution_and_ignores_late_result() {
    let mut vm = new_storage_page_task_executor_test_vm("https://tools.test/");
    vm.eval(r#"
      globalThis.probe = [];
      document.modelContext.registerTool({name: 'wait', description: 'old', execute: (input, {signal}) => {
        globalThis.savedSignal = signal;
        return new Promise(resolve => signal.addEventListener('abort', () => resolve('late')));
      }}).then(() => document.modelContext.getTools())
        .then(([tool]) => document.modelContext.executeTool(tool))
        .then(result => probe.push(result), error => probe.push(error.name));
    "#).unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("savedSignal.aborted")
            .unwrap(),
        "false"
    );
    vm.eval(
        "document.open(); document.write('<!doctype html><body>new</body>'); document.close();",
    )
    .unwrap();
    assert_eq!(
        vm.eval_after_selected_page_tasks("probe.join('|') + '|' + savedSignal.aborted")
            .unwrap(),
        "UnknownError|true"
    );
}
