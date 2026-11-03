use super::*;

#[tokio::test]
async fn bar_props_follow_shared_popup_state_across_independent_pages() {
    let (fixture_addr, fixture_server) = spawn_local_storage_fixture_server().await;
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(
        discover_browser_websocket_url(cdp_addr)
            .await
            .expect("discover browser endpoint"),
    )
    .await
    .expect("connect to browser websocket");
    let session_id = cdp_create_default_session_and_navigate(
        &mut socket,
        &format!("http://{fixture_addr}/page"),
    )
    .await;
    let expression = r#"(async () => {
      const names = ['locationbar', 'menubar', 'personalbar', 'scrollbars', 'statusbar', 'toolbar'];
      const readVisible = Object.getOwnPropertyDescriptor(BarProp.prototype, 'visible').get;
      const failures = [];
      let total = 0;
      const check = (name, pass) => {total++; if (!pass) failures.push(name);};
      for (const [features, expected] of [['', true], ['popup', false], ['popup=0,width=300', true]]) {
        const popup = window.open('about:blank', '_blank', features);
        for (let attempt = 0; attempt < 100 && (!popup.EventTarget || popup.Function === Function || !popup.document.body); attempt++)
          await new Promise(resolve => setTimeout(resolve, 10));
        check(features + ': independent realm', typeof popup.EventTarget === 'function' && popup.Function !== Function);
        const bars = names.map(name => popup[name]);
        check(features + ': own getters', bars.every(bar => bar.visible === expected));
        check(features + ': borrowed getters', bars.every(bar => readVisible.call(bar) === expected));
        const child = popup.document.createElement('iframe');
        popup.document.body.appendChild(child);
        check(features + ': child', names.every(name => child.contentWindow[name].visible === expected));
        popup.close();
        check(features + ': retired', bars.every(bar => readVisible.call(bar) === false));
      }
      return {total, failures};
    })()"#;
    let messages = send_cdp_command(
        &mut socket,
        10,
        "Runtime.evaluate",
        Some(&session_id),
        json!({"expression":expression, "awaitPromise":true, "returnByValue":true}),
    )
    .await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(10_u64))
        .expect("probe response");
    assert!(
        response["error"].is_null() && response["result"]["exceptionDetails"].is_null(),
        "probe must complete without exceptions: {response:#}"
    );
    assert_eq!(
        response["result"]["result"]["value"],
        json!({"total":15,"failures":[]})
    );
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
