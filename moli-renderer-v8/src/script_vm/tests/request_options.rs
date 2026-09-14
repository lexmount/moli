use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[derive(Clone, Copy)]
enum RequestCase {
    RedirectError,
    NoReferrer,
    OriginReferrer,
    DocumentReferrer,
    CspReport,
}

#[tokio::test]
async fn continued_window_fetch_retains_redirect_error() {
    request_option(RequestCase::RedirectError, true, false).await;
}

#[tokio::test]
async fn authenticated_window_fetch_retains_redirect_error() {
    request_option(RequestCase::RedirectError, true, true).await;
}

#[tokio::test]
async fn continued_window_fetch_retains_no_referrer() {
    request_option(RequestCase::NoReferrer, true, false).await;
}

#[tokio::test]
async fn authenticated_window_fetch_retains_no_referrer() {
    request_option(RequestCase::NoReferrer, true, true).await;
}

#[tokio::test]
async fn continued_window_fetch_retains_origin_referrer() {
    request_option(RequestCase::OriginReferrer, true, false).await;
}

#[tokio::test]
async fn authenticated_window_fetch_retains_origin_referrer() {
    request_option(RequestCase::OriginReferrer, true, true).await;
}

#[tokio::test]
async fn continued_window_fetch_retains_original_document_referrer_policy() {
    request_option(RequestCase::DocumentReferrer, true, false).await;
}

#[tokio::test]
async fn authenticated_window_fetch_retains_original_document_referrer_policy() {
    request_option(RequestCase::DocumentReferrer, true, true).await;
}

#[tokio::test]
async fn ordinary_window_fetch_applies_original_options() {
    for option in [
        RequestCase::RedirectError,
        RequestCase::NoReferrer,
        RequestCase::OriginReferrer,
        RequestCase::DocumentReferrer,
    ] {
        request_option(option, false, false).await;
    }
}

#[tokio::test]
async fn ordinary_csp_report_does_not_follow_redirects() {
    request_option(RequestCase::CspReport, false, false).await;
}

#[tokio::test]
async fn continued_csp_report_does_not_follow_redirects() {
    request_option(RequestCase::CspReport, true, false).await;
}

#[tokio::test]
async fn authenticated_csp_report_does_not_follow_redirects() {
    request_option(RequestCase::CspReport, true, true).await;
}

async fn request_option(option: RequestCase, intercepted: bool, authenticate: bool) {
    let csp_report = matches!(option, RequestCase::CspReport);
    let redirect_error = matches!(option, RequestCase::RedirectError | RequestCase::CspReport);
    let method = if csp_report { "POST" } else { "GET" };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        loop {
            let (mut stream, _) = tokio::select! {
                accepted = listener.accept() => accepted.unwrap(),
                _ = &mut stopped => return requests,
            };
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(stream.read_u8().await.unwrap());
            }
            let head = String::from_utf8(bytes).unwrap();
            let length =
                request_header(&head, "content-length").map_or(0, |length| length.parse().unwrap());
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            if csp_report {
                let report: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(report["csp-report"]["effective-directive"], "connect-src");
            }
            let authorized = request_header(&head, "authorization") == Some("Basic dXNlcjpwYXNz");
            let response = if authenticate && !authorized {
                "401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test\""
            } else if redirect_error && head.starts_with(&format!("{method} /probe ")) {
                "307 Temporary Redirect\r\nLocation: /must-not-follow"
            } else {
                "200 OK"
            };
            requests.push(head);
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 {response}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (mut vm, mut completions) = new_storage_test_vm_with_loader_and_resource_completion_queue(
        &format!("{origin}/source/path?private=value"),
        &loader,
    );
    let output = NativeResourceOutput::observe(&vm);
    if matches!(option, RequestCase::DocumentReferrer) {
        vm.set_response_referrer_policy(Some("origin".into()));
    }
    vm.set_fetch_subresource_interception(intercepted, None);
    if csp_report {
        let document_url = Url::parse(&format!("{origin}/source/path?private=value")).unwrap();
        let violation = test_window_csp_report_violation(
            &document_url,
            &Url::parse(&format!("{origin}/probe")).unwrap(),
        );
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let context = crate::network_host::capture_window_csp_report_request_context(
                scope,
                host,
                crate::native_bridge::OwnerDispatchScope::Top,
            )
            .unwrap();
            crate::network_host::send_content_security_policy_violation_report_from_window_context(
                host, &context, &violation,
            );
            Ok(())
        })
        .unwrap();
    } else {
        let options = match option {
            RequestCase::RedirectError => "{redirect:'error'}",
            RequestCase::NoReferrer => "{referrer:''}",
            RequestCase::OriginReferrer => "{referrerPolicy:'origin'}",
            RequestCase::DocumentReferrer => "{}",
            RequestCase::CspReport => unreachable!(),
        };
        vm.exec(&format!("globalThis.result='pending'; fetch('/probe',{options}).then(()=>result='resolved',error=>result=error.name);"), None).unwrap();
    }
    let request_id = if intercepted {
        let pending = vm.take_pending_subresource_fetch_infos();
        assert_eq!(pending.len(), 1);
        let id = pending[0].internal_id;
        if matches!(option, RequestCase::DocumentReferrer) {
            vm.set_response_referrer_policy(Some("unsafe-url".into()));
        }
        vm.continue_pending_subresource_fetch(id, None, None, None, None, false, authenticate)
            .unwrap();
        Some(id)
    } else {
        None
    };
    let mut challenges = 0;
    let mut observations = Vec::new();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            assert!(completions.wait_for_arrival_without_timeout().await);
            while let Some(event) = completions.pop_next_async_subresource_event() {
                let _ = vm
                    .complete_async_subresource_fetch_event_body(event)
                    .unwrap();
            }
            for event in vm.take_pending_subresource_continue_events() {
                if let crate::types::PendingSubresourceContinueEvent::AuthRequired(info) = event {
                    assert!(authenticate);
                    assert_eq!(Some(info.internal_id), request_id);
                    challenges += 1;
                    assert_eq!(challenges, 1, "credentials must settle the challenge");
                    let _ = vm
                        .continue_pending_subresource_auth_body(
                            info.internal_id,
                            crate::SubresourceAuthCredentials {
                                target: crate::types::SubresourceAuthTarget::Server,
                                scheme: crate::types::SubresourceAuthScheme::Basic,
                                username: "user".into(),
                                password: "pass".into(),
                            },
                        )
                        .unwrap();
                }
            }
            vm.with_default_context_scope_and_checkpoint_for_test(|_, _| Ok(()))
                .unwrap();
            observations.extend(output.take());
            let result = (!csp_report).then(|| vm.eval("globalThis.result").unwrap());
            if result.as_deref() != Some("pending")
                && observations.iter().any(|item| {
                    matches!(
                        item,
                        crate::types::ScriptNetworkOutputItem::SubresourceBodyFinished(_)
                    )
                })
            {
                break result;
            }
        }
    })
    .await;
    stop.send(()).unwrap();
    let requests = tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    let result = result.expect("request and promise must finish");
    assert_eq!(challenges, usize::from(authenticate));
    assert!(requests.len() > usize::from(authenticate));
    for head in &requests {
        assert!(
            head.starts_with(&format!("{method} /probe HTTP/1.1\r\n")),
            "redirect:error must never follow: {head}"
        );
        match option {
            RequestCase::RedirectError | RequestCase::CspReport => {}
            RequestCase::NoReferrer => assert_eq!(request_header(head, "referer"), None),
            RequestCase::OriginReferrer | RequestCase::DocumentReferrer => assert_eq!(
                request_header(head, "referer"),
                Some(format!("{origin}/").as_str())
            ),
        }
    }
    assert_eq!(
        result.as_deref(),
        (!csp_report).then_some(if redirect_error {
            "TypeError"
        } else {
            "resolved"
        })
    );
    if redirect_error {
        let heads: Vec<_> = observations
            .iter()
            .filter_map(|item| match item {
                crate::types::ScriptNetworkOutputItem::SubresourceResponseStarted(head) => {
                    Some(head)
                }
                _ => None,
            })
            .collect();
        assert_eq!(heads.len(), 1, "retain the original redirect response");
        assert_eq!(heads[0].status(), 307);
    }
    assert_eq!(
        observations
            .iter()
            .filter(|item| matches!(
                item,
                crate::types::ScriptNetworkOutputItem::SubresourceBodyFinished(_)
            ))
            .count(),
        1,
        "all physical attempts retain a single original request terminal"
    );
}

fn request_header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (header, value) = line.split_once(':')?;
        header.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}
