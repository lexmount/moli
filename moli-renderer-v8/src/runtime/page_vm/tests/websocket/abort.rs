use super::*;

const ABORT_PROBE: &str =
    include_str!("../../../../script_vm/tests/fixtures/websocket-stream-native-abort.js");

#[tokio::test]
async fn websocket_stream_abort_algorithms_ignore_author_hooks_across_realms() {
    run_page_vm_async_test(async move {
        for child in [false, true] {
            for foreign_signal in [false, true] {
                for mode in ["getter", "noop", "stopping", "prototype"] {
                    let (url, server) = spawn_sleeping_handshake_websocket_server().await;
                    let mut page = test_page_vm();
                    let executor = page.local_executor.clone();
                    executor
                        .run(async move {
                            let realm = if child { "childRealm" } else { "window" };
                            let signal_realm = if foreign_signal { "childRealm" } else { "window" };
                            page.vm_mut().eval(ABORT_PROBE)?;
                            page.vm_mut().eval(&format!(
                                r#"
                          const childRealm=document.body.appendChild(document.createElement('iframe')).contentWindow;
                          const realm={realm};const signalRealm={signal_realm};
                          globalThis.__wsStreamDone=false;
                          probeWebSocketStreamNativeAbort(realm,signalRealm,{url:?},{mode:?}).then(
                            result=>globalThis.__abortResult=result,error=>globalThis.__abortResult=String(error.stack||error)
                          ).finally(()=>globalThis.__wsStreamDone=true);
                        "#
                            ))?;
                            drive_websocket_until_done(
                                &mut page,
                                "String(__wsStreamDone)",
                                "native abort hooks",
                            )
                            .await?;
                            assert_eq!(
                                page.vm_mut().eval("JSON.stringify(__abortResult)")?,
                                r#"{"gets":0,"calls":0,"opened":"AbortError","closed":"AbortError"}"#
                            );
                            Ok::<_, anyhow::Error>(())
                        })
                        .await
                        .unwrap();
                    server.abort();
                    let _ = server.await;
                }
            }
        }
    })
    .await;
}

#[tokio::test]
async fn websocket_stream_synthetic_abort_keeps_post_open_io_alive() {
    run_page_vm_async_test(async move {
        for child in [false, true] {
            let (url, server) = spawn_text_echo_websocket_server().await;
            let mut page = test_page_vm();
            let executor = page.local_executor.clone();
            executor
                .run(async move {
                    let realm = if child { "childRealm" } else { "window" };
                    page.vm_mut().eval(ABORT_PROBE)?;
                    page.vm_mut().eval(&format!(
                        r#"
                  const childRealm=document.body.appendChild(document.createElement('iframe')).contentWindow;
                  globalThis.__wsStreamDone=false;
                  probeWebSocketStreamSyntheticAbort({realm},window,{url:?}).then(
                    result=>globalThis.__abortResult=result,error=>globalThis.__abortResult=String(error.stack||error)
                  ).finally(()=>globalThis.__wsStreamDone=true);
                "#
                    ))?;
                    drive_websocket_until_done(
                        &mut page,
                        "String(__wsStreamDone)",
                        "synthetic abort and real echo",
                    )
                    .await?;
                    assert_eq!(page.vm_mut().eval("String(__abortResult)")?, "true");
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .unwrap();
            server.await.unwrap();
        }
    })
    .await;
}

#[tokio::test]
async fn websocket_stream_abort_options_validate_native_signals_before_connecting() {
    run_page_vm_async_test(async move {
        let mut page = test_page_vm();
        let executor = page.local_executor.clone();
        executor
            .run(async move {
                page.vm_mut().eval(ABORT_PROBE)?;
                assert_eq!(
                    page.vm_mut().eval(
                        r#"(() => {
              const child=document.body.appendChild(document.createElement('iframe')).contentWindow;
              for(const realm of [window,child])for(const signalRealm of [window,child])
                if(!probeWebSocketStreamAbortOptions(realm,signalRealm))return false;
              return true;
            })()"#
                    )?,
                    "true"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn websocket_stream_abort_registration_releases_roots_at_terminal_or_retirement() {
    run_page_vm_async_test(async move {
        for ending in ["open", "error", "close", "retire"] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/pending", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (_socket, _) = listener.accept().await.unwrap();
                std::future::pending::<()>().await;
            });
            let mut page = test_page_vm();
            let executor = page.local_executor.clone();
            executor
                .run(async move {
                    page.vm_mut().eval(&format!(
                        r#"
                  globalThis.__controller=new AbortController();globalThis.__gcSignal=__controller.signal;
                  globalThis.__stream=new WebSocketStream({url:?},{{signal:__gcSignal}});
                  globalThis.__settlements=[];
                  __stream.opened.then(()=>__settlements.push('open'),()=>__settlements.push('error'));
                  __stream.closed.then(()=>__settlements.push('close'),()=>__settlements.push('error'));
                "#
                    ))?;
                    let (weak_signal, weak_algorithm, socket_id) = page
                        .vm_mut()
                        .with_default_context_scope_for_test(|scope, host| {
                            let global = scope.get_current_context().global(scope);
                            let key = crate::util::v8str(scope, "__gcSignal");
                            let signal = v8::Local::<v8::Object>::try_from(
                                global.get(scope, key.into()).unwrap(),
                            )
                            .unwrap();
                            let key = crate::util::v8str(scope, "__stream");
                            let stream = v8::Local::<v8::Object>::try_from(
                                global.get(scope, key.into()).unwrap(),
                            )
                            .unwrap();
                            let id = crate::util::get_private_value(
                                scope,
                                stream,
                                "__moliWebSocketId",
                            )
                            .unwrap()
                            .number_value(scope)
                            .unwrap() as u64;
                            let registration = unsafe { &mut *host }
                                .take_websocket_stream_abort_registration(id)
                                .expect("handshake registration");
                            let algorithm = registration.weak_algorithm_for_test(scope);
                            assert!(unsafe { &mut *host }
                                .set_websocket_stream_abort_registration(id, registration));
                            Ok((v8::Weak::new(scope, signal), algorithm, id))
                        })?;
                    page.vm_mut().eval("__gcSignal=undefined;__controller=undefined")?;
                    page.vm_mut().with_default_context_scope_for_test(|scope, _| {
                        scope.low_memory_notification();
                        assert!(
                            weak_algorithm.to_local(scope).is_some(),
                            "pending handshake retains native callback"
                        );
                        Ok(())
                    })?;
                    if ending == "retire" {
                        page.vm_mut().with_default_context_scope_for_test(|_, host| {
                            assert!(unsafe { &mut *host }.retire_websocket(socket_id));
                            Ok(())
                        })?;
                        page.vm_mut().with_default_context_scope_for_test(|scope, _| {
                            let signal = weak_signal.to_local(scope).expect("signal before collection");
                            let signal = crate::abort_signal_route::ResolvedAbortSignal::resolve(scope, signal).unwrap();
                            signal.abort(scope, v8::undefined(scope).into());
                            Ok(())
                        })?;
                        assert_eq!(
                            page.vm_mut().eval("__settlements.length")?,
                            "0",
                            "a stale native algorithm cannot settle a retired stream"
                        );
                    } else {
                        let event = match ending {
                            "open" => moli_websocket::Event::Open {
                                socket_id,
                                protocol: String::new(),
                                extensions: String::new(),
                                request_headers: Default::default(),
                                response_status: 101,
                                response_headers: Default::default(),
                            },
                            "error" => moli_websocket::Event::Error {
                                socket_id,
                                message: "test handshake failure".into(),
                            },
                            "close" => moli_websocket::Event::Close {
                                socket_id,
                                code: 1000,
                                reason: String::new(),
                                was_clean: true,
                            },
                            _ => unreachable!(),
                        };
                        assert!(page.vm().websocket_sender_for_test().event_sender().send(event).await);
                        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                        assert!(page.run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::WebSocket,
                            &loader,
                        ).await?);
                    }
                    page.vm_mut().with_default_context_scope_for_test(|scope, _| {
                        scope.low_memory_notification();
                        scope.low_memory_notification();
                        assert!(
                            weak_algorithm.to_local(scope).is_none(),
                            "{ending} must release the native callback root"
                        );
                        Ok(())
                    })?;
                    assert_eq!(
                        page.vm_mut().eval("typeof __stream")?,
                        "object",
                        "the stream itself stays reachable"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .unwrap();
            server.abort();
            let _ = server.await;
        }
    })
    .await;
}
