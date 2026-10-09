use super::*;

async fn wait_for_storage_value(
    page: &mut TestCdpSocket,
    expression: &str,
    expected: serde_json::Value,
) {
    timeout(Duration::from_secs(5), async {
        loop {
            if evaluate_window_name_probe(page, 900, expression).await == expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{expression} did not become {expected}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_area_subscriptions_include_listener_only_iframes_on_other_pages() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async {
            axum::response::Html("<!doctype html><p>storage</p>")
        })),
        "window-storage-subscriptions",
    );
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let receiver_id = create_dynamic_target(&mut browser, 1).await;
    let source_id = create_dynamic_target(&mut browser, 2).await;
    let mut receiver = connect_dynamic_page(addr, &receiver_id).await;
    let mut source = connect_dynamic_page(addr, &source_id).await;
    for page in [&mut receiver, &mut source] {
        send_cdp_command(page, 1, "Page.enable", None, json!({})).await;
        navigate_dynamic_page_and_wait_for_load(page, 2, &format!("http://{fixture_addr}/storage"))
            .await;
    }
    evaluate_window_name_probe(&mut receiver, 3, r#"
window.events = [];
window.frameReady = false;
addEventListener('storage', e => events.push([e.key, e.oldValue, e.newValue, e.storageArea === localStorage]));
const frame = document.createElement('iframe');
frame.id = 'listener';
frame.srcdoc = `<script>
window.events = [];
onstorage = e => events.push([e.key, e.oldValue, e.newValue, e.storageArea === localStorage]);
parent.frameReady = true;
</script>`;
document.body.appendChild(frame);
true
"#).await;
    wait_for_storage_value(&mut receiver, "frameReady", json!(true)).await;
    evaluate_window_name_probe(
        &mut source,
        3,
        "window.events=[];onstorage=e=>events.push(e.key);true",
    )
    .await;
    evaluate_window_name_probe(
        &mut source,
        4,
        r#"
localStorage.setItem('shared', 'one');
localStorage.setItem('shared', 'one');
localStorage.setItem('shared', 'two');
localStorage.removeItem('missing');
localStorage.removeItem('shared');
localStorage.clear();
localStorage.setItem('clear', 'yes');
localStorage.clear();
sessionStorage.setItem('session', 'other page');
true
"#,
    )
    .await;
    let expected = json!([
        ["shared", null, "one", true],
        ["shared", "one", "two", true],
        ["shared", "two", null, true],
        ["clear", null, "yes", true],
        [null, null, null, true],
    ]);
    wait_for_storage_value(&mut receiver, "events", expected.clone()).await;
    wait_for_storage_value(
        &mut receiver,
        "document.getElementById('listener').contentWindow.events",
        expected,
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut source, 5, "events").await,
        json!([])
    );

    evaluate_window_name_probe(
        &mut receiver,
        5,
        "sessionStorage.setItem('session','same page');true",
    )
    .await;
    wait_for_storage_value(
        &mut receiver,
        "document.getElementById('listener').contentWindow.events.slice(5)",
        json!([["session", null, "same page", false]]),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut receiver, 6, "events.length").await,
        json!(5)
    );
    assert_eq!(
        evaluate_window_name_probe(&mut source, 6, "events").await,
        json!([])
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_subscriptions_survive_document_open_without_rejoining_the_area() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async {
            axum::response::Html("<!doctype html><p>storage</p>")
        })),
        "window-storage-document-open",
    );
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let source_id = create_dynamic_target(&mut browser, 2).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    let mut source = connect_dynamic_page(addr, &source_id).await;
    for page in [&mut opener, &mut source] {
        send_cdp_command(page, 1, "Page.enable", None, json!({})).await;
        navigate_dynamic_page_and_wait_for_load(page, 2, &format!("http://{fixture_addr}/storage"))
            .await;
    }
    let (_, mut related) =
        super::auxiliary_page_identity::open_auxiliary(addr, &mut opener, "").await;
    evaluate_window_name_probe(
        &mut related,
        3,
        "window.events=[];onstorage=e=>events.push(e.newValue);true",
    )
    .await;
    evaluate_window_name_probe(
        &mut source,
        3,
        "localStorage.setItem('preserved','before');true",
    )
    .await;
    wait_for_storage_value(&mut related, "events", json!(["before"])).await;
    evaluate_window_name_probe(
        &mut related,
        4,
        "document.open();document.close();onstorage=e=>events.push(e.newValue);true",
    )
    .await;
    evaluate_window_name_probe(
        &mut source,
        4,
        "localStorage.setItem('preserved','after');true",
    )
    .await;
    wait_for_storage_value(&mut related, "events", json!(["before", "after"])).await;
    abort_test_cdp_server(server).await;
}
