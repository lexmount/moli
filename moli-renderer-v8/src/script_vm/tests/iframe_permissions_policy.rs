use super::*;

#[test]
fn fullscreen_enabled_reflects_committed_document_policy() {
    let mut vm = new_parsed_test_vm(
        "https://fullscreen-policy.test/",
        "<!doctype html><iframe id='child'></iframe>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const child = document.getElementById("child");
  child.allow = "sync-xhr 'none'";
  return JSON.stringify({
    enabled: [
      document.fullscreenEnabled,
      child.contentDocument.fullscreenEnabled,
      new Document().fullscreenEnabled
    ],
    allow: [child.allow, child.getAttribute("allow")]
  });
})()
"#,
        )
        .expect("fullscreen policy probe should evaluate");

    assert_eq!(
        result,
        r#"{"enabled":[true,true,false],"allow":["sync-xhr 'none'","sync-xhr 'none'"]}"#
    );
}
