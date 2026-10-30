use super::*;
#[tokio::test(flavor = "current_thread")]
async fn retired_target_window_messages_prepare_without_admitting_a_task() {
    run_page_vm_async_test(async {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let document_url = Url::parse("https://example.com/retired-window-message-prepare")?;
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(r#"
globalThis.targetFrame = document.createElement('iframe');
targetFrame.id = 'retired-message-target'; document.body.appendChild(targetFrame);
"#)?;
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "retired-message-target")?;
        let result = page_vm.vm_mut().eval(r#"
const oldTarget = targetFrame.contentWindow;
const retainedPostMessage = oldTarget.postMessage;
targetFrame.remove();
const buffer = new ArrayBuffer(8), channel = new MessageChannel();
const log = [];
let error = null;
try {
  Reflect.apply(retainedPostMessage, oldTarget, [{get value() {log.push('payload'); return 7;}}, {
    get transfer() {log.push('transfer'); return [channel.port2, buffer];},
    get targetOrigin() {log.push('origin'); return 'not a URL';}
  }]);
} catch (caught) {error = caught.name;}
const marker = {};
let sameException = false;
try {Reflect.apply(retainedPostMessage, oldTarget, [0, {get transfer() {throw marker;}}]);}
catch (caught) {sameException = caught === marker;}
let portUsable = false;
try {const moved = structuredClone(channel.port2, {transfer:[channel.port2]}); moved.close(); portUsable = true;}
catch (_) {}
channel.port1.close(); channel.port2.close();
globalThis.preparedMessages = [];
onmessage = event => preparedMessages.push(event.data);
postMessage('fence', '*');
JSON.stringify({log, error, bytes:buffer.byteLength, portUsable, sameException});
"#)?;
        assert_eq!(result, r#"{"log":["transfer","origin","payload"],"error":null,"bytes":0,"portUsable":true,"sameException":true}"#);
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::WindowMessage, &loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify(preparedMessages)")?, r#"["fence"]"#);
        assert!(!page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::WindowMessage, &loader,
        ).await?);
        Ok::<_, anyhow::Error>(())
    }).await.expect("a retired target must still prepare data without a message task");
}
