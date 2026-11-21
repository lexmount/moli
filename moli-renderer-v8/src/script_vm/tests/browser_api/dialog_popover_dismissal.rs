use super::*;

#[test]
fn opening_dialog_dismisses_unrelated_auto_popovers() {
    let mut vm = new_storage_html_test_vm("https://dialog-popover.test/");
    assert_eq!(
        vm.eval(
            r#"
(() => {
  for (const method of ['show', 'showModal']) {
    const container = document.createElement('div');
    document.body.append(container);
    container.innerHTML = '<div popover></div><div popover="manual"></div><dialog></dialog>';
    const [automatic, manual, dialog] = container.children;
    automatic.showPopover();
    manual.showPopover();
    dialog[method]();
    if (automatic.matches(':popover-open') || !manual.matches(':popover-open')) {
      throw Error(method + ' dismisses only the unrelated auto popover');
    }
    dialog.close();
    manual.hidePopover();
    container.remove();
  }
  const outer = document.createElement('div');
  outer.popover = 'auto';
  outer.innerHTML = '<dialog></dialog>';
  document.body.append(outer);
  outer.showPopover();
  outer.firstChild.showModal();
  return outer.matches(':popover-open');
})()
"#
        )
        .unwrap(),
        "true"
    );
}
