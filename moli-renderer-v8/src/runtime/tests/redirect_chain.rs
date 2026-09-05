use super::*;
use crate::protocol_types::NavigationRedirect;

#[tokio::test(flavor = "multi_thread")]
async fn committed_streaming_page_preserves_redirect_chain_before_and_after_parser_resume() {
    assert_streaming_redirect_chain(RendererReplyBoundary::DocumentCommit).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn completed_streaming_page_preserves_redirect_chain() {
    assert_streaming_redirect_chain(RendererReplyBoundary::Stage).await;
}

async fn assert_streaming_redirect_chain(reply_boundary: RendererReplyBoundary) {
    for (content_type, body) in [
        (
            "text/html",
            "<!doctype html><title>destination</title><main>body</main>",
        ),
        ("application/json", r#"{"result":"destination"}"#),
        ("text/plain", "destination"),
    ] {
        let runtime = JsRuntime::initialize();
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("default loader");
        let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
        runtime.set_renderer_output_transport_sender(output_tx);
        let requested_url = url::Url::parse("https://example.test/redirect/3").unwrap();
        let mut from_url = requested_url.clone();
        let chain: Vec<_> = ["/relative-redirect/2", "/relative-redirect/1", "/get"]
            .into_iter()
            .map(|location| {
                let to_url = requested_url.join(location).unwrap();
                let redirect = NavigationRedirect {
                    from_url: from_url.clone(),
                    to_url: to_url.clone(),
                    status: 302,
                    headers: vec![("location".to_owned(), location.to_owned())],
                    network_extra_info_available: false,
                    request_extra_info: None,
                    response_extra_info: None,
                    redirect_has_extra_info: false,
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                };
                from_url = to_url;
                redirect
            })
            .collect();
        let final_url = from_url;
        let headers = vec![("content-type".to_owned(), content_type.to_owned())];
        // DocumentCommit forces a pending phase-one installation, even when
        // the body is already buffered. No network timing or sleep is needed.
        let (mut page, initial, _, artifacts, download) = runtime
            .create_streaming_raw_page_from_external_body_with_inspector_session_restores(
                requested_url.clone(),
                final_url.clone(),
                None,
                true,
                chain.len(),
                chain.clone(),
                200,
                headers.clone(),
                &loader,
                crate::RendererWebStorageHandles::ephemeral(),
                ExternalRawDocumentBodyStream::from_bytes(body.as_bytes().to_vec()),
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
                false,
                1.0,
                Default::default(),
                None,
                false,
                Vec::new(),
                false,
                None,
                Vec::new(),
                false,
                PageVmInitStage::Load,
                reply_boundary,
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
                RendererNavigationReplyPolicy::FollowBeforeReply,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("redirected document should attach");
        assert!(download.is_none());
        assert_eq!(initial.navigation_redirect_count, chain.len());
        assert!(initial.navigation_redirected);
        assert_eq!(initial.navigation_redirect_chain, chain, "{content_type}");
        assert_eq!(initial.requested_url, requested_url);
        assert_eq!(initial.final_url(), &final_url);
        assert_eq!(initial.headers, headers);

        if matches!(reply_boundary, RendererReplyBoundary::DocumentCommit) {
            assert!(artifacts.lifecycle_snapshot.load.is_none());
            page.take_committed_document_post_response_continuation()
                .expect("commit must precede parser continuation")
                .release();
            tokio::time::timeout(
                Duration::from_secs(2),
                recv_page_lifecycle_until(
                    &mut output_rx,
                    &page,
                    RendererDocumentLifecycleMilestone::Load,
                ),
            )
            .await
            .expect("resumed parser should reach load");
        }
        for _ in 0..2 {
            assert!(
                serialize_html_for_renderer_page(&page)
                    .await
                    .contains("destination")
            );
            let observed = RendererPageTestingHandle::new_for_testing(&page)
                .current_page_state_async()
                .await
                .expect("post-resume state should be readable");
            assert_eq!(observed.navigation_redirect_chain, chain, "{content_type}");
            assert_eq!(observed.navigation_redirect_count, chain.len());
            assert_eq!(observed.requested_url, requested_url);
            assert_eq!(observed.final_url(), &final_url);
        }
        page.close_async()
            .await
            .expect("redirect test page should close");
    }
}
