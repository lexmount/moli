use super::*;
#[tokio::test(flavor = "current_thread")]
async fn window_message_dictionary_getter_exception_never_admits_a_payload_task() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let document_url = Url::parse("https://example.com/window-message-conversion")?;
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let setup = page_vm.vm_mut().eval(
            r#"
globalThis.conversionMessages = [];
globalThis.conversionClones = 0;
onmessage = event => conversionMessages.push(event.data);
const conversionSentinel = {};
let conversionException = false;
try {
  postMessage({get value() { conversionClones++; return 'rejected'; }}, {
    get targetOrigin() { throw conversionSentinel; }
  });
} catch (error) {
  conversionException = error === conversionSentinel;
}
postMessage('fence', '*');
String(conversionException && conversionClones === 0);
"#,
        )?;
        assert_eq!(setup, "true");
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("JSON.stringify(conversionMessages)")?,
            r#"["fence"]"#
        );
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WindowMessage,
                    &loader,
                )
                .await?
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("a throwing options getter must not create a posted-message task");
}
