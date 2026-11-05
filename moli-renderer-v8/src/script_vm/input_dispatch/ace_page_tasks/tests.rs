use crate::dom::native::DomHost;
use crate::parser::HtmlParser;
use crate::script_vm::{ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness};
use url::Url;

fn handle(vm: &StandaloneScriptVmHarness, id: &str) -> crate::dom::NodeId {
    vm._context_host
        .borrow()
        .dom_host()
        .element_handle_by_id(id)
        .unwrap()
}

#[test]
fn select_validates_target_and_all_values_before_changing_selection() {
    let mut vm = new_parsed_test_vm(
        "https://select.test/",
        "<div id=other></div><select id=s><option value=a>A</option><option selected value=b>B</option></select>",
    );
    vm.eval("globalThis.events = []; const s = document.getElementById('s'); for (const type of ['input','change']) s.addEventListener(type, e => events.push(e.type))").unwrap();
    let other = handle(&vm, "other");
    assert!(vm.select_control_values(other, &["a".into()]).is_err());
    let select = handle(&vm, "s");
    for values in [
        vec!["missing".into()],
        vec!["a".into(), "missing".into()],
        vec!["a".into(), "b".into()],
    ] {
        assert!(vm.select_control_values(select, &values).is_err());
        assert_eq!(
            vm.eval("JSON.stringify([s.value, events])").unwrap(),
            r#"["b",[]]"#
        );
    }
}

#[test]
fn select_emits_events_only_for_changes_to_the_actual_selected_options() {
    let mut vm = new_parsed_test_vm(
        "https://select.test/",
        "<select id=s><option selected value=a>A</option><option value=b>B</option></select>",
    );
    vm.eval("globalThis.events = []; const s = document.getElementById('s'); for (const type of ['input','change']) s.addEventListener(type, e => events.push([e.type,e.isTrusted,s.value]))").unwrap();
    let select = handle(&vm, "s");
    assert!(vm.select_control_values(select, &["a".into()]).unwrap());
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        "[]",
        "dirty state alone is not a selection change"
    );
    assert!(
        vm.select_control_values(select, &["b".into(), "b".into()])
            .unwrap()
    );
    assert_eq!(
        vm.eval("JSON.stringify(events)").unwrap(),
        r#"[["input",true,"b"],["change",true,"b"]]"#
    );
    vm.eval("events.length = 0").unwrap();
    assert!(vm.select_control_values(select, &[]).unwrap());
    assert_eq!(
        vm.eval("JSON.stringify([s.selectedIndex, events])")
            .unwrap(),
        r#"[-1,[["input",true,""],["change",true,""]]]"#
    );
    vm.eval("events.length = 0").unwrap();
    assert!(vm.select_control_values(select, &[]).unwrap());
    assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), "[]");
}

#[test]
fn select_uses_the_live_multiple_attribute_and_option_identities() {
    let mut vm = new_parsed_test_vm(
        "https://select.test/",
        "<select id=s multiple><option value=a>A1</option><option value=a>A2</option><option value=b>B</option></select>",
    );
    let select = handle(&vm, "s");
    assert!(
        vm.select_control_values(select, &["a".into(), "b".into()])
            .unwrap()
    );
    assert_eq!(
        vm.eval("JSON.stringify(Array.from(document.getElementById('s').options, o=>o.selected))")
            .unwrap(),
        "[true,true,true]"
    );
    vm.eval("document.getElementById('s').multiple = false")
        .unwrap();
    assert!(
        vm.select_control_values(select, &["a".into(), "b".into()])
            .is_err()
    );
    assert!(vm.select_control_values(select, &["a".into()]).unwrap());
    assert_eq!(
        vm.eval("JSON.stringify(Array.from(document.getElementById('s').options, o=>o.selected))")
            .unwrap(),
        "[true,false,false]"
    );
}

#[test]
fn explicit_submit_does_not_click_a_default_button_and_enter_keeps_implicit_rules() {
    for buttons in ["", "<button disabled>first</button><button>second</button>"] {
        let mut vm = new_parsed_test_vm(
            "https://submit.test/",
            &format!("<form id=f><input id=query><input>{buttons}</form>"),
        );
        vm.eval("globalThis.events = []; const f = document.getElementById('f'); f.addEventListener('click', e=>events.push('click')); f.addEventListener('submit', e=>{events.push(e.submitter ? e.submitter.id : null); e.preventDefault()}); document.getElementById('query').focus()").unwrap();
        vm.dispatch_key_event("keydown", "Enter", "Enter", "\r", 0, false, true)
            .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            "[]",
            "implicit submission is blocked"
        );
        for id in ["query", "f"] {
            let target = handle(&vm, id);
            vm.submit_control_form(target).unwrap();
        }
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            "[null,null]",
            "explicit submission must not search for or click a button"
        );
    }
}

#[test]
fn explicit_submit_reuses_validation_and_submitter_overrides() {
    let mut vm = new_parsed_test_vm(
        "https://submit.test/",
        "<form id=f action='/default'><input required name=q><button id=send name=button value=send formaction='/override' formnovalidate>send</button></form>",
    );
    vm.eval("globalThis.events = []; const f=document.getElementById('f'); f.addEventListener('click', ()=>events.push('click')); f.addEventListener('submit', e=>events.push(e.submitter.id))").unwrap();
    let form = handle(&vm, "f");
    vm.submit_control_form(form).unwrap();
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "invalid form is not submitted"
    );
    assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), "[]");
    let button = handle(&vm, "send");
    vm.submit_control_form(button).unwrap();
    assert_eq!(vm.eval("JSON.stringify(events)").unwrap(), r#"["send"]"#);
    let pending = vm
        .take_pending_location_navigation_with_seed()
        .expect("submitter skips validation and chooses action");
    assert_eq!(
        pending.url.as_str(),
        "https://submit.test/override?q=&button=send"
    );
}

#[test]
fn replace_control_text_replaces_entire_value_with_surrogate_pairs() {
    let mut vm = new_parsed_test_vm(
        "https://replace-control-text.test/",
        "<html><body><input id='input'><textarea id='textarea'></textarea></body></html>",
    );

    for control in ["input", "textarea"] {
        vm.eval(&format!(
            "(() => {{ const control = document.getElementById('{control}'); \
             control.value = 'A😀B'; control.focus(); }})()"
        ))
        .expect("text control should be focused");
        let handle = vm
            ._context_host
            .borrow()
            .active_element_handle()
            .expect("text control should have an active handle");

        assert!(
            vm.replace_control_text(handle, "done")
                .expect("complete text replacement should succeed")
        );
        assert_eq!(
            vm.eval(&format!(
                "(() => {{ const control = document.getElementById('{control}'); \
                 return JSON.stringify([control.value, control.selectionStart, \
                 control.selectionEnd]); }})()"
            ))
            .expect("replacement value and selection should be readable"),
            r#"["done",4,4]"#,
            "{control} must not retain text after a surrogate pair"
        );
    }
}

fn new_parsed_test_vm(url: &str, markup: &str) -> StandaloneScriptVmHarness {
    let _js_runtime = crate::JsRuntime::initialize();
    let document =
        HtmlParser::SCRIPTING_ENABLED.parse(Url::parse(url).expect("test url"), markup.to_owned());
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
    let page_runtime_task_source = page_task_queue.residence();
    ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(document),
        post_domcontentloaded_page_task_sender,
        page_task_front_injection_tx,
    )
    .expect("script vm bootstrap should succeed")
    .finish()
    .map(|mut vm| {
        vm.install_page_task_residence_for_executor_test(page_runtime_task_source);
        vm
    })
    .expect("script vm finish should succeed")
}
