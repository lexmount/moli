use super::*;

#[test]
fn handler_parse_errors_report_native_details_without_author_stack_hooks() {
    let mut vm = new_parsed_test_vm(
        "https://handler-compilation-stack.test/",
        "<!doctype html><body></body>",
    );
    let result = vm.eval(r#"
(() => {
  const frame = document.body.appendChild(document.createElement('iframe'));
  const results = [];
  for (const w of [window, frame.contentWindow]) {
    let reads = 0;
    const errors = [];
    const previous = Object.getOwnPropertyDescriptor(w.Error, 'prepareStackTrace');
    const onError = event => {
      event.preventDefault();
      errors.push([event.error instanceof w.SyntaxError, typeof event.message === 'string', event.message.length > 0]);
    };
    w.addEventListener('error', onError);
    Object.defineProperty(w.Error, 'prepareStackTrace', {
      configurable: true,
      get() { reads++; throw new Error('author stack getter must not run'); }
    });
    try {
      const element = w.document.body.appendChild(w.document.createElement('button'));
      element.setAttribute('onclick', '}');
      const handler = element.onclick;
      results.push({isNull: handler === null, reads, errors});
      element.remove();
    } finally {
      w.removeEventListener('error', onError);
      if (previous) Object.defineProperty(w.Error, 'prepareStackTrace', previous);
      else delete w.Error.prepareStackTrace;
    }
  }
  frame.remove();
  return JSON.stringify(results);
})()
"#).expect("parse error diagnostics should not execute author stack hooks");
    assert_eq!(
        result,
        r#"[{"isNull":true,"reads":0,"errors":[[true,true,true]]},{"isNull":true,"reads":0,"errors":[[true,true,true]]}]"#
    );
}
