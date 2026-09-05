use super::*;

#[test]
fn bootstrap_eval_remains_intrinsic_with_dom_named_candidates() {
    let mut vm = new_parsed_test_vm(
        "https://bootstrap-intrinsic-eval.test/",
        "<!doctype html><body><div id=eval></div><form name=eval></form></body>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
                globalThis.__indirectEvalValue = 11;
                function direct() {
                    const lexicalValue = 7;
                    return eval('lexicalValue + 1');
                }
                const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'eval');
                return JSON.stringify([
                    typeof eval, eval.name, eval.length, descriptor.value === eval,
                    direct(), (0, eval)('__indirectEvalValue')
                ]);
            })()"#,
        )
        .expect("direct and indirect builtin eval with DOM named properties"),
        r#"["function","eval",1,true,8,11]"#
    );
}
