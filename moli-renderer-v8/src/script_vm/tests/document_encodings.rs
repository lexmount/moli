use super::*;

#[test]
fn native_document_encodings_survive_live_and_detached_clones() {
    let mut vm = new_storage_test_vm("https://document-encodings.test/");
    vm.document_runtime.set_document_character_set("GBK");
    assert_eq!(
        vm.eval(
            r#"
        globalThis.shallow = document.cloneNode(false);
        globalThis.deep = document.cloneNode(true);
        JSON.stringify([document.characterSet, document.charset, document.inputEncoding,
            shallow.characterSet, deep.characterSet,
            document.implementation.createHTMLDocument('').characterSet,
            new DOMParser().parseFromString('<root/>', 'application/xml').characterSet])
    "#
        )
        .unwrap(),
        r#"["GBK","GBK","GBK","GBK","GBK","UTF-8","UTF-8"]"#
    );
    vm.document_runtime
        .set_document_character_set("windows-1252");
    assert_eq!(
        vm.eval(
            r#"
        JSON.stringify([document.characterSet, shallow.characterSet, deep.characterSet,
            shallow.cloneNode(true).characterSet, deep.cloneNode(false).characterSet])
    "#
        )
        .unwrap(),
        r#"["windows-1252","GBK","GBK","GBK","GBK"]"#
    );
}

#[tokio::test]
async fn native_document_encodings_follow_child_commit_and_stay_document_local() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://document-encodings.test/",
        &loader,
    );
    vm.document_runtime
        .set_document_character_set("windows-1252");
    vm.exec(
        r#"
        const frame = document.body.appendChild(document.createElement('iframe'));
        frame.id = 'encoding-frame'; frame.srcdoc = '<!doctype html><p>child</p>';
    "#,
        None,
    )
    .unwrap();
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    let frame = vm
        .document_runtime
        .get_element_by_id("encoding-frame")
        .unwrap();
    let child_document = vm
        ._context_host
        .borrow()
        .child_browsing_context_document_handle(frame)
        .unwrap();
    assert_eq!(
        vm.document_runtime
            .document_character_set_for_handle(child_document),
        // srcdoc supplies Unicode markup and is parsed as UTF-8.
        Some("UTF-8")
    );
    vm.document_runtime
        .dom_host_mut()
        .set_document_character_set_for_handle(child_document, "GBK");
    assert_eq!(vm.eval(r#"
        const child = document.getElementById('encoding-frame').contentDocument;
        globalThis.childClone = child.cloneNode(true);
        JSON.stringify([document.characterSet, child.characterSet, child.charset, child.inputEncoding,
            childClone.characterSet, childClone.cloneNode(false).characterSet])
    "#).unwrap(), r#"["windows-1252","GBK","GBK","GBK","GBK","GBK"]"#);
    vm.document_runtime.set_document_character_set("UTF-8");
    assert_eq!(
        vm.eval(
            r#"
        JSON.stringify([document.characterSet,
            document.getElementById('encoding-frame').contentDocument.characterSet,
            childClone.characterSet])
    "#
        )
        .unwrap(),
        r#"["UTF-8","GBK","GBK"]"#
    );
}
