use super::*;

#[tokio::test]
async fn worker_importscripts_rejects_missing_or_invalid_http_mime_before_execution() {
    ensure_v8();
    for content_type in [
        None,
        Some(""),
        Some("not a mime type"),
        Some("text/javascript"),
    ] {
        let headers = content_type
            .map(|mime| format!("Content-Type: {mime}\r\n"))
            .unwrap_or_default();
        let body = "self.executed = true;";
        let (base_url, server) = spawn_raw_path_response_http_server(vec![(
            "/dep.js",
            format!(
                "HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
            Duration::ZERO,
        )])
        .await;
        let mut handle = spawn_test_worker_with_options(WorkerSpawnOptions::new(
            r#"
            let result = 'ok';
            try { importScripts('./dep.js'); }
            catch (error) { result = error.name; }
            postMessage([result, self.executed === true]); close();
            "#
            .into(),
            format!("{base_url}/worker.js"),
        ));
        assert_eq!(
            recv_post_json(&mut handle).await,
            if content_type == Some("text/javascript") {
                "[\"ok\",true]"
            } else {
                "[\"NetworkError\",false]"
            },
            "{content_type:?}"
        );
        handle.terminate_and_join();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn worker_importscripts_rejects_missing_or_invalid_blob_mime() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(WorkerSpawnOptions::new(
        r#"
        const results = [];
        for (const type of ['text/plain', '', 'not a mime type']) {
            self.second = false;
            const badMime = URL.createObjectURL(new Blob(['self.second = true'], {type}));
            let result = 'ok';
            try { importScripts(badMime); }
            catch (error) { result = error.name; }
            URL.revokeObjectURL(badMime);
            results.push([self.second === true, result]);
        }
        postMessage(results); close();
        "#
        .into(),
        "https://example.test/worker.js".into(),
    ));
    assert_eq!(
        recv_post_json(&mut handle).await,
        "[[false,\"NetworkError\"],[false,\"NetworkError\"],[false,\"NetworkError\"]]"
    );
}
