use crate::dom::native::DomHost;
use crate::parser::HtmlParser;
use crate::script_vm::{ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness};
use url::Url;

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
