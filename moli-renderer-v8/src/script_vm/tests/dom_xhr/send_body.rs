use super::*;

const BODY_CONVERSION_PROBE: &str =
    include_str!("../../../../tests/fixtures/xhr-send-body-conversion.js");

fn assert_body_conversion_result(observed: &str) {
    let result: serde_json::Value = serde_json::from_str(observed).expect("body conversion result");
    let checks = result["checks"].as_array().expect("body conversion checks");
    let failures: Vec<_> = checks
        .iter()
        .filter(|check| check["pass"] != true)
        .collect();
    assert!(
        failures.is_empty(),
        "body conversion failures: {failures:#?}"
    );
    assert_eq!(checks.len(), 1451);
    assert_eq!(result["state"], "pass");
}

#[test]
fn window_xhr_send_body_validates_buffers_before_state_and_method() {
    let mut vm = new_storage_test_vm("https://xhr-body-conversion.test/");
    let observed = vm
        .eval(&format!(
            "{BODY_CONVERSION_PROBE}\nJSON.stringify(xhrSendBodyConversionProbe())"
        ))
        .expect("Window body conversion probe");
    assert_body_conversion_result(&observed);
}

#[tokio::test(flavor = "current_thread")]
async fn worker_xhr_send_body_validates_buffers_before_state_and_method() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://xhr-body-conversion.test/", &loader);
    let worker = format!(
        "{BODY_CONVERSION_PROBE}\ntry {{ postMessage(JSON.stringify(xhrSendBodyConversionProbe())); }} catch (error) {{ postMessage(JSON.stringify({{error: String(error.stack || error)}})); }} close();"
    );
    vm.eval(&format!(
        r#"
        globalThis.bodyConversionResult = null;
        const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}})));
        worker.onmessage = event => {{ bodyConversionResult = event.data; }};
        worker.onerror = event => {{ bodyConversionResult = JSON.stringify({{error: event.message}}); event.preventDefault(); }};
        "#,
        serde_json::to_string(&worker).unwrap()
    ))
    .expect("start Worker body conversion probe");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "bodyConversionResult !== null",
        "true",
        "Worker body conversion probe",
    )
    .await;
    let observed = vm
        .eval("bodyConversionResult")
        .expect("Worker conversion result");
    assert_body_conversion_result(&observed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xhr_send_body_reentrant_conversion_controls_request() {
    for worker in [false, true] {
        let server = StaticHttpServer::spawn_with_bodies(vec!["ok".to_owned(); 6]).await;
        let base = server.base_url();
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm = new_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
        let probe = r#"
        (async () => {
          const cases = [
            {initial: 'POST', method: 'GET'},
            {initial: 'GET', method: 'POST'},
            {initial: 'POST', method: 'HEAD', sync: true},
            {initial: null, method: 'POST'},
            {initial: 'POST', method: 'POST', sync: true},
            {initial: 'POST', method: 'POST', nested: true},
          ];
          for (const [index, c] of cases.entries()) {
            const xhr = new XMLHttpRequest();
            const url = BASE + 'echo?case=' + index;
            let returned = false, early = false, conversions = 0;
            const done = new Promise(resolve => xhr.onloadend = () => {
              early = !returned; resolve();
            });
            if (c.initial) xhr.open(c.initial, url, true);
            let caught;
            try {
              xhr.send({toString() {
                conversions++;
                if (c.nested) xhr.send('inner');
                else xhr.open(c.method, url, !c.sync);
                return 'payload';
              }});
            } catch (error) { caught = error; }
            returned = true;
            if ((caught && caught.name || null) !== (c.nested ? 'InvalidStateError' : null))
              throw new Error('send exception: ' + index + ': ' + caught);
            await done;
            if (conversions !== 1 || xhr.status !== 200 || early !== !!c.sync)
              throw new Error('conversion or completion order: ' + index);
          }
          return 'pass';
        })()
        "#
        .replace("BASE", &serde_json::to_string(base.as_str()).unwrap());
        let source = if worker {
            let worker_source = format!(
                "{probe}.then(value => {{postMessage(value); close();}}, error => {{postMessage(String(error.stack || error)); close();}});"
            );
            format!(
                r#"
                globalThis.sendReentryResult = 'pending';
                const worker = new Worker(URL.createObjectURL(new Blob([{}], {{type: 'text/javascript'}})));
                worker.onmessage = event => {{ sendReentryResult = event.data; }};
                worker.onerror = event => {{ sendReentryResult = event.message; event.preventDefault(); }};
                "#,
                serde_json::to_string(&worker_source).unwrap()
            )
        } else {
            format!(
                "globalThis.sendReentryResult = 'pending'; {probe}.then(value => {{sendReentryResult = value;}}, error => {{sendReentryResult = String(error.stack || error);}});"
            )
        };
        vm.eval(&source).expect("start reentrant send probe");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "sendReentryResult !== 'pending'",
            "true",
            "reentrant send probe",
        )
        .await;
        assert_eq!(vm.eval("sendReentryResult").unwrap(), "pass");
        let requests = server.finish().await;
        assert_eq!(requests.len(), 6);
        for (index, (request, (method, length))) in requests
            .iter()
            .zip([
                ("GET", None),
                ("POST", Some("7")),
                ("HEAD", None),
                ("POST", Some("7")),
                ("POST", Some("7")),
                ("POST", Some("5")),
            ])
            .enumerate()
        {
            assert_eq!(request.target, format!("/echo?case={index}"));
            assert_eq!(request.method, method);
            assert_eq!(request.header_value("content-length"), length);
            assert_eq!(
                request.header_value("content-type"),
                length.map(|_| "text/plain;charset=UTF-8")
            );
        }
    }
}
