use super::*;

#[test]
fn worker_constructors_convert_inherited_options_before_launch() {
    let mut vm = new_storage_html_test_vm("https://worker-options.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../tests/fixtures/worker-options-dictionary.js"
        ))
        .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
                const trace = [], sentinel = {};
                let caught;
                try { new SharedWorker('http://[', new Proxy({}, {get(_target, key) {
                    trace.push(key);
                    if (key === 'extendedLifetime') throw sentinel;
                }})); } catch (error) { caught = error; }
                return caught === sentinel && trace.join() === 'credentials,name,type,extendedLifetime';
            })()"#,
        )
        .unwrap(),
        "true"
    );
}
