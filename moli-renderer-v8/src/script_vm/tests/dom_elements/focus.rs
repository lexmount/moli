use super::*;

#[test]
fn focus_prevent_scroll_controls_real_nested_scroll_container_reveal() {
    let mut vm = new_storage_test_vm("https://focus-prevent-scroll.test/");
    vm.force_fresh_layout_reads_for_test();

    vm.eval(
        r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = `
    <button id="first">first</button>
    <div id="scroller" style="width:100px;height:100px;overflow:auto">
      <div style="width:500px;height:400px"></div>
      <button id="target" style="margin-left:400px">target</button>
    </div>`;
  return 'installed';
})()
"#,
    )
    .expect("focus scroll fixture should initialize");
    refresh_layout_for_test(&mut vm);

    let result = vm
        .eval(
            r#"
(() => {
  const first = document.getElementById('first');
  const scroller = document.getElementById('scroller');
  const target = document.getElementById('target');

  target.focus({ preventScroll: true });
  const prevented = scroller.scrollLeft === 0 && scroller.scrollTop === 0;
  const focused = document.activeElement === target;
  first.focus();
  target.focus();
  return [prevented, focused, scroller.scrollLeft > 0, scroller.scrollTop > 0].join('|');
})()
"#,
        )
        .expect("focus preventScroll probe should evaluate");

    assert_eq!(result, "true|true|true|true");
}

#[test]
fn document_has_focus_projects_the_page_focused_frame_ancestry() {
    let mut vm = new_storage_test_vm("https://document-focus-tree.test/");

    let focused_tree = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const first = body.appendChild(document.createElement('iframe'));
  const second = body.appendChild(document.createElement('iframe'));
  const firstInput = first.contentDocument.body.appendChild(
    first.contentDocument.createElement('input'));
  second.contentDocument.body.appendChild(second.contentDocument.createElement('input'));
  firstInput.focus();
  globalThis.__focusTree = { first, second, firstInput };
  return JSON.stringify([
    document.hasFocus(),
    first.contentDocument.hasFocus(),
    second.contentDocument.hasFocus(),
    document.activeElement === first,
    first.contentDocument.activeElement === firstInput
  ]);
})()
"#,
        )
        .expect("focused-frame ancestry should evaluate");
    assert_eq!(focused_tree, "[true,true,false,true,true]");

    assert!(
        vm.set_top_level_page_focus(false, false)
            .expect("Page blur should complete")
    );
    let background = vm
        .eval(
            "JSON.stringify([document.hasFocus(), __focusTree.first.contentDocument.hasFocus(), __focusTree.second.contentDocument.hasFocus(), document.activeElement === __focusTree.first])",
        )
        .expect("background frame focus should evaluate");
    assert_eq!(background, "[false,false,false,true]");

    assert!(
        vm.set_top_level_page_focus(true, true)
            .expect("Page refocus should complete")
    );
    let restored = vm
        .eval(
            "JSON.stringify([document.hasFocus(), __focusTree.first.contentDocument.hasFocus(), __focusTree.second.contentDocument.hasFocus(), __focusTree.first.contentDocument.activeElement === __focusTree.firstInput])",
        )
        .expect("restored frame focus should evaluate");
    assert_eq!(restored, "[true,true,false,true]");
}

#[test]
fn focusing_contenteditable_in_child_frame_reveals_authored_frame_position() {
    let mut vm = new_storage_test_vm("https://focus-scroll.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    iframe { position: absolute; left: 250vw; }
    .spacer { width: 100vw; height: 250vh; }
  `;
  head.appendChild(style);

  const first = document.createElement('div');
  first.contentEditable = 'true';
  const spacer = document.createElement('div');
  spacer.className = 'spacer';
  const frame = document.createElement('iframe');
  body.append(first, spacer, frame);

  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.write('<div id="target" contenteditable="true">target</div>');
  childDocument.close();
  const target = childDocument.getElementById('target');

  first.focus();
  target.focus();
  const firstX = window.scrollX;
  const firstY = window.scrollY;

  window.scroll(0, 0);
  first.focus();
  target.focus();
  return JSON.stringify({
    beyondViewport: firstX > window.innerWidth && firstY > window.innerHeight,
    repeated: firstX === window.scrollX && firstY === window.scrollY,
    parentRetargeted: document.activeElement === frame,
    childFocused: childDocument.activeElement === target
  });
})()
"#,
        )
        .expect("child contenteditable focus scroll probe should evaluate");

    assert_eq!(
        result,
        r#"{"beyondViewport":true,"repeated":true,"parentRetargeted":true,"childFocused":true}"#
    );
}

#[test]
fn focusing_frame_owner_then_input_dispatches_child_window_focus_and_blur() {
    let mut vm = new_storage_test_vm("https://focus-frame-window.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const input = document.createElement('input');
  const frame = document.createElement('iframe');
  body.append(input, frame);
  const log = [];
  window.onblur = () => log.push('top-window-blur');
  window.onfocus = () => log.push('top-window-focus');
  frame.onfocus = () => log.push('frame-focus');
  frame.onblur = () => log.push('frame-blur');
  frame.contentWindow.onfocus = () => log.push('child-window-focus');
  frame.contentWindow.onblur = () => log.push('child-window-blur');
  input.onfocus = () => log.push('input-focus');

  frame.focus();
  input.focus();
  return `${log.join(',')}|${document.activeElement === input}`;
})()
"#,
        )
        .expect("frame window focus transition probe should evaluate");

    assert_eq!(
        result,
        "top-window-blur,frame-focus,child-window-focus,frame-blur,child-window-blur,input-focus,top-window-focus|true"
    );
}
