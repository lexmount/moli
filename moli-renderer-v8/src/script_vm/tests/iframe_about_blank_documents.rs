use super::*;

#[tokio::test]
async fn explicit_about_blank_iframe_variants_preserve_the_initial_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://iframe-initial-blank.test/page.html",
        &loader,
    );
    assert_eq!(vm.eval(r##"(() => {
        const sources = ['about:blank', 'about:blank#fragment', 'about:blank?query', 'about:blank?query#fragment'];
        globalThis.blankDocuments = sources.map(src => {
            const frame = document.createElement('iframe');
            frame.src = src;
            document.body.appendChild(frame);
            const child = frame.contentWindow, childDocument = child.document;
            childDocument.body.textContent = src;
            return {frame, child, document: childDocument, src};
        });
        globalThis.blankDocumentsMatch = () => blankDocuments.every(({frame, child, document, src}) =>
            frame.contentWindow === child && frame.contentDocument === document &&
            child.document === document && child.location.href === src && document.URL === src &&
            document.body.textContent === src);
        globalThis.afterBlankTask = 'pending';
        setTimeout(() => afterBlankTask = String(blankDocumentsMatch()), 0);
        return blankDocumentsMatch();
    })()"##).unwrap(), "true");
    assert!(
        !vm.has_pending_child_navigation_commit_for_test(),
        "initial about:blank variants must not enqueue a replacement document"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "afterBlankTask",
        "true",
        "about:blank documents after queued tasks",
    )
    .await;
    assert_eq!(vm.eval("String(blankDocumentsMatch())").unwrap(), "true");
}
