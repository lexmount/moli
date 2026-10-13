use super::*;

#[test]
fn document_exec_command_gates_only_insert_html_values() {
    let mut vm = new_storage_test_vm("https://exec-command-trusted-types.test/");
    vm.set_response_content_security_policies(&["require-trusted-types-for 'script'".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error && error.name;
    }
  };
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const input = document.createElement("input");
  body.append(input);
  input.focus();

  const blocked = [
    errorName(() => document.execCommand("insertHTML", false, "<b>blocked</b>")),
    errorName(() => document.execCommand("InSeRtHtMl", false, null))
  ];
  const unprotected = [
    errorName(() => document.execCommand("insertHTML")),
    errorName(() => document.execCommand("insertHTML", false, undefined)),
    errorName(() => document.execCommand("paste", false, "<b>plain</b>")),
    errorName(() => document.execCommand("insertText", false, "plain"))
  ];

  input.value = "";
  const custom = trustedTypes.createPolicy("exec-command-custom", {
    createHTML: value => value
  });
  const trustedReturned = document.execCommand(
    "insertHTML",
    false,
    custom.createHTML("<b>trusted</b>")
  );
  const trustedValue = input.value;

  let sourceConversions = 0;
  const defaultCalls = [];
  trustedTypes.createPolicy("default", {
    createHTML: (value, type, sink) => {
      defaultCalls.push([value, type, sink]);
      return "<b>default</b>";
    }
  });
  input.value = "";
  const defaultReturned = document.execCommand("insertHTML", false, {
    toString() {
      sourceConversions += 1;
      return "<i>original</i>";
    }
  });

  return JSON.stringify({
    blocked,
    unprotected,
    trusted: [trustedReturned, trustedValue],
    defaulted: [defaultReturned, input.value, sourceConversions],
    defaultCalls
  });
})()
"#,
        )
        .expect("Document.execCommand TrustedHTML sink probe should evaluate");

    assert_eq!(
        result,
        r#"{"blocked":["TypeError","TypeError"],"unprotected":["none","none","none","none"],"trusted":[true,"trusted"],"defaulted":[true,"default",1],"defaultCalls":[["<i>original</i>","TrustedHTML","Document execCommand"]]}"#
    );
}

#[tokio::test]
async fn borrowed_exec_command_uses_its_document_policy_and_callee_error_realm() {
    let mut vm = new_storage_test_vm("https://borrowed-exec-command-trusted.test/");
    vm.exec(r#"
      const root = document.documentElement || document.appendChild(document.createElement('html'));
      const body = document.body || root.appendChild(document.createElement('body'));
      const frame = document.createElement('iframe'); frame.id = 'enforced';
      frame.srcdoc = `<meta http-equiv="Content-Security-Policy" content="require-trusted-types-for 'script'"><body>`;
      body.append(frame);
    "#, None).unwrap();
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "enforced editing child").await;
    assert_eq!(vm.eval(r#"(() => {
      const child = document.getElementById('enforced').contentWindow;
      let blocked = false;
      try { Document.prototype.execCommand.call(child.document, 'insertHTML', false, 'blocked'); }
      catch (error) { blocked = error instanceof TypeError && !(error instanceof child.TypeError); }
      if (!blocked) throw Error('borrowed command must enforce receiving Document policy in callee realm');
      const calls = [];
      trustedTypes.createPolicy('default', {createHTML(value) {calls.push('top'); return value;}});
      child.trustedTypes.createPolicy('default', {createHTML(value, type, sink) {
        calls.push([value, type, sink]); return value;
      }});
      Document.prototype.execCommand.call(child.document, 'insertHTML', false, '<b>child</b>');
      child.Document.prototype.execCommand.call(document, 'insertHTML', false, '<b>plain</b>');
      if (JSON.stringify(calls) !== JSON.stringify([['<b>child</b>', 'TrustedHTML', 'Document execCommand']]))
        throw Error('borrowed command used the caller default policy');
      return true;
    })()"#).unwrap(), "true");
}
