use super::*;

#[test]
fn child_joint_history_pushes_accumulate_across_distinct_frames() {
    let mut vm = new_storage_test_vm("https://joint-child-length.test/page.html");

    vm.exec(
        r#"
const first = document.createElement('iframe');
first.srcdoc = '<p>first</p>';
(document.body || document.documentElement || document).appendChild(first);
globalThis.__firstJointLengthFrame = first;
"#,
        None,
    )
    .expect("first child setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const child = __firstJointLengthFrame.contentWindow;
  const before = history.length;
  child.history.pushState(null, '', '#first');
  return [before, history.length, child.history.length].join('|');
})()
"#,
        )
        .expect("first child history push should evaluate"),
        "1|2|2"
    );

    vm.exec(
        r#"
__firstJointLengthFrame.remove();
const second = document.createElement('iframe');
second.srcdoc = '<p>second</p>';
(document.body || document.documentElement || document).appendChild(second);
globalThis.__secondJointLengthFrame = second;
"#,
        None,
    )
    .expect("second child setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const child = __secondJointLengthFrame.contentWindow;
  const before = history.length;
  child.history.pushState(null, '', '#second');
  return [before, history.length, child.history.length].join('|');
})()
"#,
        )
        .expect("second child history push should evaluate"),
        "2|3|3"
    );
}
