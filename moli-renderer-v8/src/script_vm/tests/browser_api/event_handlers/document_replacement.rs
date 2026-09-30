use super::*;

#[test]
fn document_replacement_clears_native_handlers_without_changing_public_descriptors() {
    let mut vm = new_storage_html_test_vm("https://document-handler-cleanup.test/");
    vm.eval(include_str!("document_replacement.js")).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
}

#[test]
fn native_document_content_replacement_does_not_invoke_author_handler_setters() {
    let mut vm = new_storage_html_test_vm("https://native-handler-cleanup.test/");
    vm.root_frame_id = Some("handler-cleanup-root".to_owned());
    vm.eval(
        r#"
(() => {
  let callbacks = 0, writes = 0;
  const descriptor = Object.getOwnPropertyDescriptor(window, 'onresize');
  const setter = () => { ++writes; throw new Error('author setter ran'); };
  window.onresize = () => { ++callbacks; };
  window.addEventListener('resize', () => { ++callbacks; });
  Object.defineProperty(window, 'onresize', {set: setter});
  for (const name of ['open', 'write', 'close']) {
    document[name] = () => { throw new Error('author document method ran'); };
  }
  globalThis.verifyNativeCleanup = () => {
    window.dispatchEvent(new Event('resize'));
    return callbacks === 0 && writes === 0 && descriptor.get.call(window) === null &&
      Object.getOwnPropertyDescriptor(window, 'onresize').set === setter &&
      document.getElementById('native-replacement').textContent === 'native';
  };
})()
"#,
    )
    .unwrap();
    assert_eq!(
        vm.set_document_content_for_frame(
            "handler-cleanup-root",
            "<!doctype html><body><p id=native-replacement>native</p>",
        )
        .unwrap(),
        crate::runtime::RendererSetDocumentContentResult::Updated
    );
    assert_eq!(vm.eval("verifyNativeCleanup()").unwrap(), "true");
}
