use super::*;
use moli_websocket::test_support::TlsWebSocketFixture;

#[derive(Clone, Copy)]
enum Client {
    Page,
    Stream,
}

async fn tls_credentials_reach_client(client: Client) {
    let fixture = TlsWebSocketFixture::new();
    let (url, server) = fixture.spawn(true).await;
    let tls = fixture.tls_config();
    let mut config = FetchConfig::default();
    config.set_http_proxy(Some(String::new()));
    config.set_tls_credentials(
        tls.ca_cert,
        tls.client_cert,
        tls.client_key,
        tls.client_cert_password,
    );
    // The document is https://example.com, so this also verifies that WSS
    // sends its configured identity across origins (credentials=include).
    let mut page_vm = test_page_vm_with_config(config, Vec::new());
    let local_executor = page_vm.local_executor.clone();
    let result = local_executor.run(Box::pin(async move {
            let socket_script = format!(r#"
                const socket = new WebSocket({url:?});
                let echo;
                socket.onopen = () => socket.send('tls-echo');
                socket.onmessage = event => {{ echo = event.data; socket.close(1000, 'done'); }};
                socket.onclose = event => finish(`${{echo}}:${{event.code}}:${{event.wasClean}}`);
                socket.onerror = () => finish('error');
            "#);
            let script = match client {
                Client::Page => socket_script,
                Client::Stream => format!(r#"
                    (async () => {{
                        try {{
                            const stream = new WebSocketStream({url:?});
                            stream.closed.catch(() => {{}});
                            const opened = await stream.opened;
                            const writer = opened.writable.getWriter();
                            const reader = opened.readable.getReader();
                            await writer.write('tls-echo');
                            const {{value}} = await reader.read();
                            await writer.close();
                            const info = await stream.closed;
                            finish(`${{value}}:${{info.closeCode}}`);
                        }} catch (error) {{ finish('error'); }}
                    }})();
                "#),

            };
            page_vm.vm_mut().eval(&format!(r#"
                globalThis.__tlsDone = false;
                (() => {{
                    const finish = value => {{ globalThis.__tlsResult = value; globalThis.__tlsDone = true; }};
                    {script}
                }})();
            "#))?;
            drive_websocket_until_done(&mut page_vm, "String(globalThis.__tlsDone === true)", "TLS round trip").await?;
            page_vm.vm_mut().eval("String(globalThis.__tlsResult)")
        })).await.expect("TLS test runs on the page owner lane");
    let peer = server.await.expect("TLS fixture task");
    assert_eq!(
        result,
        if matches!(client, Client::Stream) {
            "tls-echo:1005"
        } else {
            "tls-echo:1000:true"
        },
        "server result: {peer:?}"
    );
    assert_eq!(
        peer.expect("mTLS WebSocket handshake and close"),
        std::slice::from_ref(&fixture.client_certificate)
    );
}

// V8's debug stack budget is 4 MiB; match the renderer's 8 MiB native
// stack when driving Promise/Stream callbacks from these PageVm fixtures.
#[test]
fn websocket_tls_credentials_reach_page() {
    run_page_vm_large_stack_async_test("tls-page", || {
        Box::pin(tls_credentials_reach_client(Client::Page))
    });
}

#[test]
fn websocket_tls_credentials_reach_stream() {
    run_page_vm_large_stack_async_test("tls-stream", || {
        Box::pin(tls_credentials_reach_client(Client::Stream))
    });
}
