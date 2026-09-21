use super::*;

const JOINT_HISTORY_TRAVERSAL: &str =
    include_str!("../../../tests/fixtures/joint-history-traversal.js");

#[tokio::test]
async fn joint_history_traversal_uses_commit_order_and_shared_positions() {
    for mode in [
        "back",
        "multi",
        "child",
        "queued",
        "fork",
        "cross-document",
        "cross-document-queued",
        "navigation",
        "navigation-cross-document",
        "navigation-cross-document-queued",
    ] {
        let request_count = match mode {
            "cross-document" => 10,
            "cross-document-queued"
            | "navigation-cross-document"
            | "navigation-cross-document-queued" => 9,
            _ => 2,
        };
        let server = StaticHttpServer::spawn(request_count).await;
        let base = server.base_url().origin().ascii_serialization();
        let loader = static_http_loader([]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(&format!("{base}/parent"), &loader);
        vm.eval(&format!(
            "{JOINT_HISTORY_TRAVERSAL}\n\
             globalThis.jointHistoryResult = 'pending';\n\
             jointHistoryTraversal({base:?}, {mode:?}).then(\n\
               value => jointHistoryResult = value,\n\
               error => jointHistoryResult = String(error));"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(jointHistoryResult !== 'pending')",
            "true",
            mode,
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(jointHistoryResult)").unwrap()).unwrap();
        let steps = result
            .as_array()
            .unwrap_or_else(|| panic!("{mode}: {result}"));
        for step in steps {
            let length = if step["label"] == "fork" { 2 } else { 3 };
            assert_eq!(step["length"], length, "{mode}: {step}");
            assert_eq!(
                step["childLengths"],
                serde_json::json!([length, length]),
                "{mode}: {step}"
            );
        }
        let expected = match mode {
            "back" => serde_json::json!(["/a0#a1", "/b0#b1"]),
            "fork" => serde_json::json!(["/a0#fork", "/b0"]),
            "cross-document"
            | "cross-document-queued"
            | "navigation-cross-document"
            | "navigation-cross-document-queued" => {
                serde_json::json!(["/a2", "/b1"])
            }
            _ => serde_json::json!(["/a0#a2", "/b0#b1"]),
        };
        let last = steps.last().unwrap();
        assert_eq!(last["paths"], expected, "{mode}: {result}");
        if mode == "fork" {
            assert_eq!(
                last["entries"],
                serde_json::json!([["/a0", "/a0#a1", "/a0#fork"], ["/b0"]])
            );
        }
        assert_eq!(server.finish_targets().await.len(), request_count, "{mode}");
    }
}

#[tokio::test]
async fn joint_history_waits_for_network_commits_before_releasing_mutations_and_deltas() {
    for mode in ["race", "api-pending"] {
        let server = JointHistoryGateServer::spawn(mode).await;
        let base = format!("http://{}", server.address);
        let loader = static_http_loader([]);
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(&format!("{base}/parent"), &loader);
        let script = include_str!("../../../tests/fixtures/joint-history-pending.js");
        vm.eval(&format!(
            "{script}\nglobalThis.pendingJointResult = 'pending';\n\
             jointHistoryPending({base:?}, {mode:?}).then(value => pendingJointResult = value,\n\
             error => pendingJointResult = String(error));"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(pendingJointResult !== 'pending')",
            "true",
            mode,
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(pendingJointResult)").unwrap()).unwrap();
        let steps = result
            .as_array()
            .unwrap_or_else(|| panic!("{mode}: {result}"));
        if mode == "race" {
            assert_eq!(
                steps[1]["paths"],
                serde_json::json!(["/race/a0", "/race/b1"])
            );
            assert_eq!(steps[2]["length"], 1, "{result}");
            let complete = steps.last().unwrap();
            assert_eq!(
                complete["paths"],
                serde_json::json!(["/race/a0#during", "/race/b0"])
            );
            assert_eq!(complete["length"], 1);
            assert_eq!(
                complete["entries"],
                serde_json::json!([["/race/a0", "/race/a0#during"], ["/race/b0"]])
            );
        } else {
            let complete = steps.last().unwrap();
            assert_eq!(
                complete["paths"],
                serde_json::json!(["/api-pending/a1", "/api-pending/b0"])
            );
            assert_eq!(complete["length"], 3);
        }
        let requests = server.finish().await;
        let gate = if mode == "race" {
            "/race/b0"
        } else {
            "/api-pending/a1"
        };
        assert_eq!(
            requests.iter().filter(|path| path.as_str() == gate).count(),
            2
        );
        assert_eq!(
            requests
                .iter()
                .filter(|path| path.as_str() == "/release")
                .count(),
            1
        );
    }
}

struct JointHistoryGateServer {
    address: std::net::SocketAddr,
    task: Option<tokio::task::JoinHandle<Vec<String>>>,
}

impl JointHistoryGateServer {
    async fn spawn(mode: &str) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (gate, count) = if mode == "race" {
            ("/race/b0", 7)
        } else {
            ("/api-pending/a1", 9)
        };
        let task = tokio::spawn(async move {
            let started = std::sync::Arc::new(tokio::sync::Notify::new());
            let released = std::sync::Arc::new(tokio::sync::Notify::new());
            let mut gate_visits = 0;
            let mut requests = Vec::new();
            let mut responses = tokio::task::JoinSet::new();
            for _ in 0..count {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut header = Vec::new();
                let mut buffer = [0; 2048];
                while !header.windows(4).any(|part| part == b"\r\n\r\n") {
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(read, 0);
                    header.extend_from_slice(&buffer[..read]);
                }
                let path = String::from_utf8_lossy(&header)
                    .lines()
                    .next()
                    .unwrap()
                    .split_ascii_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned();
                if path == gate {
                    gate_visits += 1;
                }
                let hold = path == gate && gate_visits == 2;
                requests.push(path.clone());
                let started = std::sync::Arc::clone(&started);
                let released = std::sync::Arc::clone(&released);
                responses.spawn(async move {
                    if hold {
                        started.notify_one();
                        released.notified().await;
                    } else if path == "/wait" {
                        started.notified().await;
                    } else if path == "/release" {
                        released.notify_one();
                    }
                    let body = "<!doctype html><body>joint history fixture</body>";
                    socket.write_all(format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                    ).as_bytes()).await.unwrap();
                });
            }
            while let Some(result) = responses.join_next().await {
                result.unwrap();
            }
            requests
        });
        Self {
            address,
            task: Some(task),
        }
    }

    async fn finish(mut self) -> Vec<String> {
        self.task.take().unwrap().await.unwrap()
    }
}

impl Drop for JointHistoryGateServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
