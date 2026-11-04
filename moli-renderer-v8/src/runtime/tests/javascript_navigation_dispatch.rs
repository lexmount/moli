use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn javascript_url_followup_delegates_network_navigation_on_browser_pages() {
    for during_dcl in [false, true] {
        for action in ["assign", "replace", "nested"] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let destination = format!("http://{address}/destination");
            let (fetched_tx, mut fetched_rx) = oneshot::channel();
            let fixture = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(count, 0, "HTTP request headers must be complete");
                    request.extend_from_slice(&buffer[..count]);
                }
                let body = "<!doctype html><title>unexpected renderer fetch</title>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                let _ = fetched_tx.send(());
            });

            let destination_js = serde_json::to_string(&destination).unwrap();
            let assignment = format!(
                "globalThis.navigationReadyState=document.readyState; location.href={destination_js}; void 0"
            );
            let source = match action {
                "assign" => assignment.clone(),
                "replace" => format!(
                    "globalThis.navigationReadyState=document.readyState; location.replace({destination_js}); void 0"
                ),
                "nested" => format!(
                    "location.href={}; void 0",
                    serde_json::to_string(&format!("javascript:{assignment}")).unwrap()
                ),
                _ => unreachable!(),
            };
            let initiate = format!(
                "location.href={}",
                serde_json::to_string(&format!("javascript:{source}")).unwrap()
            );
            let dcl_script = if during_dcl {
                format!("addEventListener('DOMContentLoaded',()=>{{{initiate}}});")
            } else {
                String::new()
            };
            let runtime = JsRuntime::initialize();
            let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
            runtime.set_renderer_output_transport_sender(output_tx);
            let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
                .expect("default loader");
            let mut page = create_test_html_page_with_navigation_dispatch(
                &runtime,
                &loader,
                url::Url::parse(&format!("http://{address}/source")).unwrap(),
                &format!(
                    "<!doctype html><title>source</title><script>globalThis.sourceDocument=document;{dcl_script}</script>"
                ),
                RendererTopLevelNavigationDispatch::DelegateToBrowser,
            )
            .await;
            if !during_dcl {
                page.run_async_command(RendererPageCommand::EvaluateExpression {
                    expression: initiate,
                    await_promise: false,
                })
                .await
                .expect("javascript URL should be queued");
            }

            // The browser consumes the request and owns its fetch/commit.
            // Return an HTTP response even on the broken renderer route so
            // failure cleanup can always close the Page and its owner lane.
            let navigation = tokio::select! {
                navigation = output_rx.recv_top_level_location_navigation() => navigation,
                _ = &mut fetched_rx => None,
                _ = tokio::time::sleep(Duration::from_secs(3)) => None,
            };
            let (source_reply, _) = page
                .run_async_command(RendererPageCommand::EvaluateExpression {
                    expression: "document===globalThis.sourceDocument && document.title==='source'"
                        .to_owned(),
                    await_promise: false,
                })
                .await
                .expect("source Document should remain accessible before browser commit");
            let (phase_reply, _) = page
                .run_async_command(RendererPageCommand::EvaluateExpression {
                    expression: "globalThis.navigationReadyState".to_owned(),
                    await_promise: false,
                })
                .await
                .expect("capture the source phase at the HTTP navigation");
            let remaining = output_rx.drain();
            page.close_async().await.expect("close navigation source");
            let renderer_did_not_fetch = matches!(
                fetched_rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            );
            fixture.abort();

            let navigation = navigation.unwrap_or_else(|| {
                panic!("{action}, during_dcl={during_dcl}: renderer followed a browser-owned URL instead of publishing its request")
            });
            assert_eq!(navigation.url(), destination);
            assert!(
                renderer_did_not_fetch,
                "the browser must own the HTTP fetch"
            );
            assert_eq!(navigation.request_method(), "GET");
            assert_eq!(
                renderer_json_value(phase_reply),
                Some(serde_json::json!(if during_dcl {
                    "interactive"
                } else {
                    "complete"
                })),
                "{action}, during_dcl={during_dcl}"
            );
            // Location navigation without transient activation replaces the
            // current entry while the source Document is still loading.
            assert_eq!(
                navigation.history_mutation(),
                if action == "replace" || during_dcl {
                    moli_page_types::NavigationHistoryMutation::Replace
                } else {
                    moli_page_types::NavigationHistoryMutation::Push
                },
                "{action}, during_dcl={during_dcl}"
            );
            assert_eq!(
                renderer_json_value(source_reply),
                Some(serde_json::json!(true))
            );
            assert!(
                remaining
                    .iter()
                    .flat_map(RendererOutputPublication::records)
                    .all(|record| !matches!(
                        record.item(),
                        RendererOutputItem::OwnerAction(
                            RendererOwnerAction::TopLevelLocationNavigation(_)
                        )
                    )),
                "one HTTP navigation must publish exactly one browser request"
            );
        }
    }
}
