use super::*;

async fn run_integrity_probe(
    page_url: &str,
    worker: bool,
    expression: &str,
    expected_checks: usize,
) {
    let loader = static_http_loader([]);
    let mut vm = new_page_task_executor_test_vm_with_loader(page_url, &loader);
    vm.eval("globalThis.integrityResult = null;").unwrap();
    let script = if worker {
        let source = format!(
            "{expression}.then(postMessage, error => postMessage({{error:String(error.stack || error)}}));"
        );
        format!(
            "const workerUrl = URL.createObjectURL(new Blob([{}], {{type:'text/javascript'}})); const worker = new Worker(workerUrl); worker.onmessage = event => {{ integrityResult = event.data; worker.terminate(); URL.revokeObjectURL(workerUrl); }}; worker.onerror = event => {{ integrityResult = {{error:event.message}}; event.preventDefault(); }};",
            serde_json::to_string(&source).unwrap()
        )
    } else {
        format!(
            "{expression}.then(value => {{ integrityResult = value; }}, error => {{ integrityResult = {{error:String(error.stack || error)}}; }});"
        )
    };
    vm.eval(&script).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(integrityResult !== null)",
        "true",
        "Fetch integrity probe should finish",
    )
    .await;
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(integrityResult)").unwrap()).unwrap();
    let checks = result["checks"]
        .as_array()
        .unwrap_or_else(|| panic!("worker={worker}: {result}"));
    let failures: Vec<_> = checks
        .iter()
        .filter(|check| check["pass"] != true)
        .collect();
    assert_eq!(result["state"], "pass", "worker={worker}: {failures:?}");
    assert_eq!(checks.len(), expected_checks, "worker={worker}");
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_integrity_validates_local_bytes_in_window_and_worker() {
    let fixture = include_str!("../../../tests/fixtures/fetch-integrity.js");
    let expression =
        format!("(async () => {{ {fixture}\nreturn fetchIntegrityProbe('', true); }})()");
    for worker in [false, true] {
        run_integrity_probe("https://fetch-integrity.test/", worker, &expression, 296).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_integrity_validates_network_responses_in_window_and_worker() {
    for worker in [false, true] {
        let server =
            StaticHttpServer::spawn_with_bodies(vec!["hello integrity".to_owned(); 8]).await;
        let expression = format!(
            r#"(async () => {{
              const checks = [];
              const check = (label, actual, wanted) => checks.push({{label, actual, wanted, pass:actual === wanted}});
              const valid = 'sha256-9pyxsrnsacWVDvpoeZHDpEDkdnPx2ySEsclLyHWyL6A=';
              const wrong = 'sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=';
              for (const [label, integrity, method, accepted] of [
                ['valid', valid, 'GET', true],
                ['invalid', wrong, 'GET', false],
                ['stronger-invalid', valid + ' sha512-AA==', 'GET', false],
                ['same-level-valid', wrong + ' ' + valid, 'GET', true],
                ['ignored', 'sha1-ignored', 'GET', true],
                ['whitespace', ' ', 'GET', true],
                ['no-integrity', '', 'GET', true],
                ['null-body', 'sha1-ignored', 'HEAD', false],
              ]) {{
                try {{
                  const response = await fetch({} + label, {{integrity, method}});
                  check(label + '/accepted', true, accepted);
                  if (accepted) check(label + '/body', await response.text(), 'hello integrity');
                }} catch (error) {{ check(label + '/accepted', error instanceof TypeError ? false : String(error), accepted); }}
              }}
              return {{state:checks.every(check => check.pass) ? 'pass' : 'fail', checks}};
            }})()"#,
            serde_json::to_string(server.base_url().as_str()).unwrap(),
        );
        run_integrity_probe(server.base_url().as_str(), worker, &expression, 13).await;
        assert_eq!(server.finish().await.len(), 8);
    }
}
