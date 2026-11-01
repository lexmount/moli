use std::time::Duration;

use super::*;

#[tokio::test(flavor = "current_thread")]
async fn window_fetch_projects_cache_modes_to_native_wire_requests() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..7 {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                assert!(head.len() < 16384);
                head.push(
                    tokio::time::timeout(Duration::from_secs(10), stream.read_u8())
                        .await
                        .unwrap()
                        .unwrap(),
                );
            }
            requests.push(String::from_utf8(head).unwrap());
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nok").await.unwrap();
        }
        requests
    });
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_page_task_executor_test_vm_with_loader(&format!("http://{address}/page"), &loader);
    vm.eval("globalThis.cacheModeResult=null").unwrap();
    vm.eval(&format!("({}).then(value => cacheModeResult=value,error => cacheModeResult=String(error))", "(async () => {\n    for (const cache of ['default','no-cache','reload','no-store','force-cache']) {\n        const response = await fetch('/wire/' + cache, {cache});\n        if (await response.text() !== 'ok') throw Error(cache);\n    }\n    await (await fetch('/wire/author', {cache:'reload', headers:{'Cache-Control':'private',Pragma:'author'}})).text();\n    await (await fetch('/wire/conditional', {headers:{'If-None-Match':'\"author\"'}})).text();\n    let rejected = false;\n    try { await fetch('/wire/miss', {mode:'same-origin',cache:'only-if-cached'}); }\n    catch (error) { rejected = error instanceof TypeError; }\n    if (!rejected) throw Error('cache-only miss must reject');\n    return 'passed';\n})()")).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(cacheModeResult !== null)",
        "true",
        "Window cache modes",
    )
    .await;
    assert_eq!(vm.eval("cacheModeResult").unwrap(), "passed");
    let requests = server.await.unwrap();
    for (head, (path, control, pragma)) in requests.iter().zip([
        ("default", None, None),
        ("no-cache", Some("max-age=0"), None),
        ("reload", Some("no-cache"), Some("no-cache")),
        ("no-store", Some("no-cache"), Some("no-cache")),
        ("force-cache", None, None),
        ("author", Some("private"), Some("author")),
        ("conditional", Some("no-cache"), Some("no-cache")),
    ]) {
        assert!(
            head.starts_with(&format!("GET /wire/{path} HTTP/1.1\r\n")),
            "{head}"
        );
        let field = |name: &str| {
            head.split("\r\n")
                .filter_map(|line| line.split_once(':'))
                .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(value.trim()))
        };
        assert_eq!(field("cache-control"), control, "{path}: {head}");
        assert_eq!(field("pragma"), pragma, "{path}: {head}");
    }
}
