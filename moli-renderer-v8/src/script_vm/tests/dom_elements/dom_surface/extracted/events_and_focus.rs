use super::*;

#[test]
fn element_event_handler_honors_document_parent_node_unscopables() {
    let mut vm = new_storage_test_vm("https://event-handler-unscopables.test/");

    let result = vm
        .eval(
            r#"
(() => {
  window.prepend = "global prepend";
  window.append = "global append";
  const element = document.createElement("div");
  element.setAttribute("onclick", `
    window.__unscopablesResult = [
      typeof prepend,
      typeof append,
      typeof this.prepend,
      typeof this.append
    ].join("|");
  `);
  element.dispatchEvent(new Event("click"));
  return window.__unscopablesResult;
})()
"#,
        )
        .expect("event handler unscopables probe should evaluate");

    assert_eq!(result, "string|string|function|function");
}
#[test]
fn anchor_click_to_identical_url_dispatches_replace_navigate_event() {
    let mut vm = new_storage_test_vm("https://anchor-same-url.test/page.html");

    let result = vm
        .eval(
            r#"
const root = document.body || document.documentElement || document;
const link = document.createElement('a');
link.href = '/page.html';
root.appendChild(link);
let seen = [];
navigation.onnavigate = e => {
  seen.push([
    e.navigationType,
    e.destination.url,
    e.sourceElement === link
  ].join(','));
  e.intercept({ handler: () => {} });
};
link.click();
seen.join('|')
"#,
        )
        .expect("same-url anchor click should dispatch navigate");

    assert_eq!(
        result,
        "replace,https://anchor-same-url.test/page.html,true"
    );
}
