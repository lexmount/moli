use anyhow::Result;
use axum::{
    Router,
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::Parser;
use moli::{app, cli::Cli};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    process::{Command, ExitCode, Output},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    task::JoinHandle,
};

const HTML: &str =
    "<!doctype html><title>HTTP fixture</title><h1>HTTP fixture</h1><p>response body</p>";
const RAW_BODY: &[u8] = b"\0\xffraw HTTP error body";

struct HttpFailureFixture {
    base_url: String,
    task: JoinHandle<()>,
}

impl HttpFailureFixture {
    async fn spawn() -> Result<Self> {
        let app = Router::new()
            .route(
                "/status/{status}",
                get(|Path(status): Path<u16>| async move {
                    (StatusCode::from_u16(status).unwrap(), Html(HTML))
                }),
            )
            .route(
                "/raw",
                get(|| async {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [("content-type", "application/octet-stream")],
                        RAW_BODY,
                    )
                }),
            )
            .route(
                "/raw/{status}",
                get(|Path(status): Path<u16>| async move {
                    (
                        StatusCode::from_u16(status).unwrap(),
                        [("content-type", "application/octet-stream")],
                        RAW_BODY,
                    )
                }),
            )
            .route(
                "/pdf/{status}",
                get(|Path(status): Path<u16>| async move {
                    (
                        StatusCode::from_u16(status).unwrap(),
                        [("content-type", "application/pdf")],
                        b"%PDF-1.7\nHTTP fixture".as_slice(),
                    )
                }),
            )
            .route(
                "/empty",
                get(|| async { Html("<!doctype html><body></body>") }),
            )
            .route(
                "/empty-error",
                get(|| async { (StatusCode::FORBIDDEN, Html("<!doctype html><body></body>")) }),
            )
            .route(
                "/redirect/{status}",
                get(|Path(status): Path<u16>| async move {
                    Redirect::temporary(&format!("/status/{status}"))
                }),
            )
            .route(
                "/challenge/{status}",
                get(|Path(status): Path<u16>| async move {
                    (
                        StatusCode::FORBIDDEN,
                        Html(format!(
                            "<!doctype html><script>location.replace('/status/{status}')</script>"
                        )),
                    )
                        .into_response()
                }),
            )
            .layer(axum::middleware::map_response(
                |mut response: Response| async move {
                    // Separate requests must have identical fixture headers when
                    // the test compares their complete selected-format output.
                    response.headers_mut().insert(
                        header::DATE,
                        HeaderValue::from_static("Thu, 01 Jan 1970 00:00:00 GMT"),
                    );
                    response
                },
            ));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Ok(Self { base_url, task })
    }

    async fn spawn_stalled_raw(status: u16, content_type: &str) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let headers = format!(
            "HTTP/1.1 {status} Error\r\nContent-Type: {content_type}\r\nContent-Length: 1048576\r\n\r\n"
        );
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
            }
            stream
                .get_mut()
                .write_all(headers.as_bytes())
                .await
                .unwrap();
            // Send no body. The client must cancel rather than wait for it.
            match stream.read(&mut [0_u8; 1]).await {
                Ok(0) => {}
                Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                result => panic!("expected download cancellation, got {result:?}"),
            }
        });
        Ok(Self { base_url, task })
    }

    fn run(&self, path: &str, args: &[&str]) -> Result<Output> {
        self.run_with_timeout(path, args, 5000)
    }

    fn run_with_timeout(&self, path: &str, args: &[&str], timeout_ms: u64) -> Result<Output> {
        Ok(Command::new(env!("CARGO_BIN_EXE_moli"))
            .env("MOLI_LAYOUT", "false")
            .args(["fetch", "--http-no-proxy", "*", "--redirect-wait-ms", "0"])
            .arg("--timeout")
            .arg(timeout_ms.to_string())
            .args(args)
            .arg(format!("{}{path}", self.base_url))
            .output()?)
    }

    fn assert_http_failure(&self, output: &Output, status: u16, final_path: &str) {
        assert_eq!(output.status.code(), Some(22), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!("HTTP status {status}")),
            "{stderr}"
        );
        assert!(
            stderr.contains(&format!("{}{final_path}", self.base_url)),
            "{stderr}"
        );
        assert_eq!(stderr.lines().count(), 2, "{stderr}");
    }
}

impl Drop for HttpFailureFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct FailingOutput {
    fail_on_flush: bool,
    written: Vec<u8>,
}

impl Write for FailingOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.fail_on_flush {
            return Err(io::Error::other("fixture write failure"));
        }
        self.written.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("fixture flush failure"))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_http_errors_keep_selected_output_and_exit_zero() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for status in [403, 500] {
        for dump in ["html", "markdown", "json"] {
            let output = server.run(&format!("/status/{status}"), &["--dump", dump])?;
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            match dump {
                "markdown" => assert_eq!(output.stdout, b"# HTTP fixture\n\nresponse body"),
                "json" => {
                    let value: Value = serde_json::from_slice(&output.stdout)?;
                    assert_eq!(value["status"], status);
                    assert!(value["html"].as_str().unwrap().contains("response body"));
                }
                _ => assert!(
                    String::from_utf8_lossy(&output.stdout).contains("<h1>HTTP fixture</h1>")
                ),
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fail_suppresses_output_and_exits_22_for_4xx_and_5xx() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for status in [400, 403, 500, 599] {
        let path = format!("/status/{status}");
        let output = server.run(&path, &["--fail", "--dump", "markdown"])?;
        server.assert_http_failure(&output, status, &path);
        assert!(output.stdout.is_empty(), "{output:?}");
    }
    // Suppression also avoids evaluating an expression on the failed page.
    let output = server.run(
        "/status/403",
        &[
            "--fail",
            "--eval",
            "(() => { throw new Error('must not run'); })()",
        ],
    )?;
    server.assert_http_failure(&output, 403, "/status/403");
    assert!(output.stdout.is_empty(), "{output:?}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fail_with_body_preserves_selected_output_format() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for args in [
        vec![],
        vec!["--dump", "html"],
        vec!["--dump", "markdown"],
        vec!["--dump", "json"],
        vec!["--dump", "semantic_tree"],
        vec!["--dump", "semantic_tree_text"],
        vec!["--eval", "document.querySelector('h1').textContent"],
    ] {
        let expected = server.run("/status/500", &args)?;
        assert_eq!(expected.status.code(), Some(0), "{expected:?}");
        assert!(!expected.stdout.is_empty(), "{args:?}");
        let mut fail_args = args.clone();
        fail_args.push("--fail-with-body");
        let output = server.run("/status/500", &fail_args)?;
        server.assert_http_failure(&output, 500, "/status/500");
        assert_eq!(output.stdout, expected.stdout, "{args:?}");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fail_with_body_keeps_output_generation_errors_and_http_diagnostics() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for (args, reason) in [
        (
            vec!["--eval", "document.querySelector('#app').textContent"],
            "TypeError",
        ),
        (vec!["--dump", "screenshot"], "requires --layout"),
        (vec!["--dump", "pdf"], "requires --layout"),
    ] {
        for status in [200, 403] {
            let path = format!("/status/{status}");
            for flag in [None, Some("--fail-with-body")] {
                let mut fail_args = args.clone();
                fail_args.extend(flag);
                let output = server.run(&path, &fail_args)?;
                assert_eq!(output.status.code(), Some(1), "{output:?}");
                assert!(output.stdout.is_empty(), "{output:?}");
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(stderr.contains(reason), "{stderr}");
                assert_eq!(
                    stderr.contains("HTTP status 403"),
                    status == 403 && flag.is_some(),
                    "{stderr}"
                );
                assert_eq!(stderr.lines().count(), 2, "{stderr}");
            }
            let output = server.run(&path, &[args.as_slice(), &["--fail"]].concat())?;
            if status == 403 {
                server.assert_http_failure(&output, status, &path);
                assert!(output.stdout.is_empty(), "{output:?}");
            } else {
                assert_eq!(output.status.code(), Some(1), "{output:?}");
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fail_with_body_reports_http_status_when_raw_output_is_unsupported() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for prefix in ["raw", "pdf"] {
        for status in [403, 503] {
            let path = format!("/{prefix}/{status}");
            for (args, reason) in [
                (
                    vec!["--dump", "markdown"],
                    "raw download output only supports",
                ),
                (vec!["--dump", "pdf"], "raw download output only supports"),
                (vec!["--eval", "1"], "does not support --eval"),
                (
                    vec!["--wait-selector", "#app"],
                    "does not support page wait options",
                ),
                (
                    vec!["--delay-ms", "1"],
                    "does not support page wait options",
                ),
            ] {
                let mut fail_args = args;
                fail_args.push("--fail-with-body");
                let output = server.run(&path, &fail_args)?;
                assert_eq!(output.status.code(), Some(1), "{output:?}");
                assert!(output.stdout.is_empty(), "{output:?}");
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(stderr.contains(reason), "{stderr}");
                assert!(
                    stderr.contains(&format!("HTTP status {status}")),
                    "{stderr}"
                );
                assert!(
                    stderr.contains(&format!("{}{path}", server.base_url)),
                    "{stderr}"
                );
                assert_eq!(stderr.lines().count(), 2, "{stderr}");
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failure_modes_preserve_write_and_flush_errors() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for prefix in ["status", "raw"] {
        for status in [200, 403] {
            let url = format!("{}/{prefix}/{status}", server.base_url);
            for flag in [None, Some("--fail"), Some("--fail-with-body")] {
                for fail_on_flush in [false, true] {
                    let mut args = vec![
                        "moli",
                        "fetch",
                        "--http-no-proxy",
                        "*",
                        "--redirect-wait-ms",
                        "0",
                    ];
                    args.extend(flag);
                    args.push(&url);
                    let cli = Cli::try_parse_from(args)?;
                    let mut stdout = FailingOutput {
                        fail_on_flush,
                        written: Vec::new(),
                    };
                    let error = app::run_cli(&mut stdout, cli)
                        .await
                        .expect_err("HTTP or output error should fail");
                    let mut report = Vec::new();
                    app::write_error_report(&mut report, &error)?;
                    let report = String::from_utf8(report)?;
                    if status == 403 && flag == Some("--fail") {
                        assert_eq!(app::error_exit_code(&error), ExitCode::from(22), "{report}");
                        assert!(stdout.written.is_empty(), "{report}");
                        assert!(!report.contains("fixture"), "{report}");
                    } else {
                        assert_eq!(app::error_exit_code(&error), ExitCode::FAILURE, "{report}");
                        assert!(error.downcast_ref::<io::Error>().is_some(), "{error:?}");
                        let phase = if fail_on_flush { "flush" } else { "write" };
                        assert!(
                            report.contains(&format!("fixture {phase} failure")),
                            "{report}"
                        );
                        if fail_on_flush {
                            assert!(!stdout.written.is_empty(), "{report}");
                        }
                    }
                    assert_eq!(
                        report.contains("HTTP status 403"),
                        status == 403 && flag.is_some(),
                        "{report}"
                    );
                    assert_eq!(report.lines().count(), 2, "{report}");
                }
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failure_modes_handle_raw_binary_and_json_output() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for args in [vec![], vec!["--dump", "json"]] {
        let expected = server.run("/raw", &args)?;
        assert_eq!(expected.status.code(), Some(0), "{expected:?}");
        if args.is_empty() {
            assert_eq!(expected.stdout, RAW_BODY);
        } else {
            let value: Value = serde_json::from_slice(&expected.stdout)?;
            assert_eq!(value["status"], 503);
            assert_eq!(
                STANDARD.decode(value["body_base64"].as_str().unwrap())?,
                RAW_BODY
            );
        }
        for flag in ["--fail", "--fail-with-body"] {
            let mut fail_args = args.clone();
            fail_args.push(flag);
            let output = server.run("/raw", &fail_args)?;
            server.assert_http_failure(&output, 503, "/raw");
            if flag == "--fail" {
                assert!(output.stdout.is_empty(), "{output:?}");
            } else {
                assert_eq!(output.stdout, expected.stdout);
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fail_checks_raw_http_status_before_output_format_and_page_options() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for prefix in ["raw", "pdf"] {
        for status in [403, 503] {
            let path = format!("/{prefix}/{status}");
            for args in [
                vec!["--dump", "markdown"],
                vec!["--dump", "pdf"],
                vec!["--dump", "html"],
                vec!["--dump", "markdown", "--wait-selector", "#app"],
                vec!["--eval", "document.querySelector('#app').textContent"],
            ] {
                let mut fail_args = args;
                fail_args.push("--fail");
                let output = server.run(&path, &fail_args)?;
                server.assert_http_failure(&output, status, &path);
                assert!(output.stdout.is_empty(), "{output:?}");
            }
        }
        // Successful raw responses still have to satisfy output constraints.
        for flag in [None, Some("--fail"), Some("--fail-with-body")] {
            let mut args = vec!["--dump", "markdown"];
            args.extend(flag);
            let output = server.run(&format!("/{prefix}/200"), &args)?;
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stdout.is_empty(), "{output:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("raw download output only supports"),
                "{stderr}"
            );
            assert!(!stderr.contains("HTTP status"), "{stderr}");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fail_cancels_raw_http_errors_without_waiting_for_the_body() -> Result<()> {
    for (status, content_type) in [(403, "application/octet-stream"), (503, "application/pdf")] {
        for wait_until in ["done", "networkidle"] {
            let mut server = HttpFailureFixture::spawn_stalled_raw(status, content_type).await?;
            let mut command = Command::new(env!("CARGO_BIN_EXE_moli"));
            command
                .args([
                    "fetch",
                    "--fail",
                    "--dump",
                    "markdown",
                    "--http-no-proxy",
                    "*",
                    "--redirect-wait-ms",
                    "0",
                    "--timeout",
                    "5000",
                    "--wait-until",
                    wait_until,
                ])
                .arg(format!("{}/raw", server.base_url));
            let output = tokio::time::timeout(
                Duration::from_secs(2),
                tokio::task::spawn_blocking(move || command.output()),
            )
            .await
            .expect("--fail must return before the raw body timeout")??;
            server.assert_http_failure(&output, status, "/raw");
            assert!(output.stdout.is_empty(), "{output:?}");
            tokio::time::timeout(Duration::from_secs(2), &mut server.task)
                .await
                .expect("the HTTP download must be cancelled")?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failure_modes_accept_success_3xx_and_empty_markdown() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for flag in [None, Some("--fail"), Some("--fail-with-body")] {
        let mut args = vec!["--dump", "markdown"];
        args.extend(flag);
        let output = server.run("/empty-error", &args)?;
        assert!(output.stdout.is_empty(), "{output:?}");
        if flag.is_some() {
            server.assert_http_failure(&output, 403, "/empty-error");
        } else {
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
        }
    }
    for flag in ["--fail", "--fail-with-body"] {
        for path in ["/status/200", "/status/399", "/empty"] {
            let output = server.run(path, &[flag, "--dump", "markdown"])?;
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            if path == "/empty" {
                assert!(output.stdout.is_empty(), "{output:?}");
            } else {
                assert_eq!(output.stdout, b"# HTTP fixture\n\nresponse body");
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failure_modes_preserve_binary_dump_formats() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for dump in ["screenshot", "screenshot_full", "pdf"] {
        for flag in ["--fail", "--fail-with-body"] {
            let output = server.run("/status/403", &[flag, "--layout", "--dump", dump])?;
            server.assert_http_failure(&output, 403, "/status/403");
            if flag == "--fail" {
                assert!(output.stdout.is_empty(), "{dump}: {output:?}");
            } else {
                let signature: &[u8] = if dump == "pdf" {
                    b"%PDF-"
                } else {
                    b"\x89PNG\r\n\x1a\n"
                };
                assert!(output.stdout.starts_with(signature), "{dump}: {output:?}");
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failure_modes_use_final_status_after_http_and_script_redirects() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for flag in ["--fail", "--fail-with-body"] {
        for prefix in ["redirect", "challenge"] {
            for status in [200, 500] {
                let output = server.run(
                    &format!("/{prefix}/{status}"),
                    &[flag, "--dump", "json", "--wait-selector", "h1"],
                )?;
                if status == 200 {
                    assert_eq!(output.status.code(), Some(0), "{output:?}");
                    assert!(output.stderr.is_empty(), "{output:?}");
                } else {
                    server.assert_http_failure(&output, 500, "/status/500");
                    if flag == "--fail" {
                        assert!(output.stdout.is_empty(), "{output:?}");
                        continue;
                    }
                }
                let value: Value = serde_json::from_slice(&output.stdout)?;
                assert_eq!(value["status"], status);
                assert_eq!(
                    value["final_url"],
                    format!("{}/status/{status}", server.base_url)
                );
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failure_modes_report_status_when_page_readiness_times_out() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for status in [403, 503] {
        let path = format!("/status/{status}");
        for (wait_option, condition, phase) in [
            ("--wait-selector", "#app", "waiting for a selector"),
            ("--wait-script", "false", "waiting for a script"),
            (
                "--wait-response-url",
                "/missing-api",
                "waiting for a subresource response",
            ),
        ] {
            for flag in ["--fail", "--fail-with-body"] {
                let output = server.run_with_timeout(
                    &path,
                    &[flag, "--dump", "markdown", wait_option, condition],
                    1000,
                )?;
                if flag == "--fail" {
                    server.assert_http_failure(&output, status, &path);
                } else {
                    assert_eq!(output.status.code(), Some(1), "{output:?}");
                }
                assert!(output.stdout.is_empty(), "{output:?}");
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(stderr.contains("fetch readiness timed out"), "{stderr}");
                assert!(stderr.contains(phase), "{stderr}");
                assert!(
                    stderr.contains(&format!("HTTP status {status}")),
                    "{stderr}"
                );
                assert!(
                    stderr.contains(&format!("{}{path}", server.base_url)),
                    "{stderr}"
                );
                assert_eq!(stderr.lines().count(), 2, "{stderr}");
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failure_modes_use_redirect_status_when_readiness_fails() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for flag in ["--fail", "--fail-with-body"] {
        for prefix in ["redirect", "challenge"] {
            for status in [200, 500] {
                let output = server.run_with_timeout(
                    &format!("/{prefix}/{status}"),
                    &[flag, "--dump", "markdown", "--wait-selector", "#app"],
                    1000,
                )?;
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(stderr.contains("waiting for a selector"), "{stderr}");
                assert!(output.stdout.is_empty(), "{output:?}");
                if status == 500 && flag == "--fail" {
                    server.assert_http_failure(&output, status, "/status/500");
                } else {
                    assert_eq!(output.status.code(), Some(1), "{output:?}");
                }
                if status == 200 {
                    assert!(!stderr.contains("HTTP status"), "{stderr}");
                } else {
                    assert!(stderr.contains("HTTP status 500"), "{stderr}");
                    assert!(
                        stderr.contains(&format!("{}/status/500", server.base_url)),
                        "{stderr}"
                    );
                }
                assert!(!stderr.contains("HTTP status 403"), "{stderr}");
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failure_modes_report_status_when_readiness_script_throws() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    for status in [200, 403] {
        let path = format!("/status/{status}");
        for flag in ["--fail", "--fail-with-body"] {
            let output = server.run(
                &path,
                &[
                    flag,
                    "--wait-script",
                    "(() => { throw new Error('readiness failed'); })()",
                ],
            )?;
            if status == 403 && flag == "--fail" {
                server.assert_http_failure(&output, status, &path);
            } else {
                assert_eq!(output.status.code(), Some(1), "{output:?}");
            }
            assert!(output.stdout.is_empty(), "{output:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("readiness failed"), "{stderr}");
            assert_eq!(
                stderr.contains("HTTP status 403"),
                status == 403,
                "{stderr}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_failure_modes_preserve_fetch_and_xhr_error_responses() -> Result<()> {
    let server = HttpFailureFixture::spawn().await?;
    let expression = r#"(async () => {
        const response = await fetch('/status/403');
        const text = await response.text();
        const xhr = await new Promise((resolve, reject) => {
            const request = new XMLHttpRequest();
            request.open('GET', '/status/500');
            request.onload = () => resolve({status: request.status, body: request.responseText.includes('response body')});
            request.onerror = reject;
            request.send();
        });
        return {status: response.status, ok: response.ok, body: text.includes('response body'), xhr};
    })()"#;
    for flag in ["--fail", "--fail-with-body"] {
        let output = server.run("/status/200", &[flag, "--eval", expression])?;
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            value,
            json!({
                "status": 403,
                "ok": false,
                "body": true,
                "xhr": {"status": 500, "body": true}
            })
        );
    }
    Ok(())
}

#[test]
fn http_failure_modes_keep_exit_code_one_for_non_http_errors() -> Result<()> {
    for flag in ["--fail", "--fail-with-body"] {
        let output = Command::new(env!("CARGO_BIN_EXE_moli"))
            .args(["fetch", flag, "https://["])
            .output()?;
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(!output.stderr.is_empty(), "{output:?}");
    }
    Ok(())
}
