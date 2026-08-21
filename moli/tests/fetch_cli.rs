use moli_test_support as support;

use anyhow::Result;
use axum::{
    Router,
    extract::State,
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use clap::Parser;
use moli::{
    app,
    cli::{Cli, normalize_args_for_compat},
    config::AppConfig,
};
use moli_browser_profile::{
    BrowserProfilePaths, load_cookie_cache as load_profile_cookie_cache, load_profile_manifest,
};
use parking_lot::Mutex;
use serde_json::Value;
use std::{
    io::Write,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use strip_ansi_escapes::strip;
use support::FixtureServer;
use tokio::{net::TcpListener, task::JoinHandle};
use tracing_subscriber::fmt::MakeWriter;

struct Output {
    status: OutputStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct OutputStatus {
    success: bool,
}

impl OutputStatus {
    fn success(&self) -> bool {
        self.success
    }
}

#[derive(Clone)]
struct CapturedStderr {
    buffer: Arc<Mutex<Vec<u8>>>,
}

struct CapturedStderrWriter {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl<'a> MakeWriter<'a> for CapturedStderr {
    type Writer = CapturedStderrWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CapturedStderrWriter {
            buffer: Arc::clone(&self.buffer),
        }
    }
}

impl Write for CapturedStderrWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.buffer.lock().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run_fetch_cli(url: &str) -> Result<Output> {
    run_fetch_cli_with_wait_until(url, "load")
}

fn run_fetch_cli_with_args(url: &str, args: &[&str]) -> Result<Output> {
    run_fetch_cli_with_dump_and_args(url, "html", args)
}

fn run_fetch_cli_with_log_level(url: &str, log_level: &str) -> Result<Output> {
    run_fetch_cli_with_log_level_and_args(url, log_level, &[])
}

fn run_fetch_cli_with_log_level_and_args(
    url: &str,
    log_level: &str,
    args: &[&str],
) -> Result<Output> {
    let mut cli_args = vec![
        "moli",
        "fetch",
        "--log-level",
        log_level,
        "--http-no-proxy",
        "*",
        "--wait-until",
        "load",
        "--dump",
        "html",
    ];
    cli_args.extend_from_slice(args);
    cli_args.push(url);
    run_moli(cli_args)
}

fn run_fetch_cli_with_dump_and_args(url: &str, dump: &str, args: &[&str]) -> Result<Output> {
    let mut cli_args = vec![
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--wait-until",
        "load",
        "--dump",
        dump,
    ];
    cli_args.extend_from_slice(args);
    cli_args.push(url);
    run_moli(cli_args)
}

fn run_fetch_cli_with_default_wait_and_dump(url: &str, dump: &str) -> Result<Output> {
    run_moli([
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--dump",
        dump,
        url,
    ])
}

fn run_moli<I, S>(args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(normalize_args_for_compat(args))?;
    let config = AppConfig::from_cli(&cli)?;
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(config.log_filter.clone())
        .with_target(false)
        .with_writer(CapturedStderr {
            buffer: Arc::clone(&stderr),
        })
        .finish();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut stdout = Vec::new();
    let _ = tracing::subscriber::set_global_default(subscriber);
    let result = runtime.block_on(app::run_cli_with_config(cli, config, &mut stdout));
    if let Err(error) = &result {
        let mut stderr = stderr.lock();
        writeln!(&mut *stderr, "{error:?}")?;
    }
    let stderr = stderr.lock().clone();
    Ok(Output {
        status: OutputStatus {
            success: result.is_ok(),
        },
        stdout,
        stderr,
    })
}

fn run_fetch_cli_with_wait_until(url: &str, wait_until: &str) -> Result<Output> {
    run_moli([
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--wait-until",
        wait_until,
        "--dump",
        "html",
        url,
    ])
}

fn clean_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&strip(bytes)).into_owned()
}

fn assert_json_dump_shape(payload: &Value, expected_url: &str, expected_status: u64) {
    let Some(object) = payload.as_object() else {
        panic!("expected top-level JSON object, got {payload}");
    };

    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    assert_eq!(keys, vec!["final_url", "html", "status"]);
    assert_eq!(payload["final_url"], expected_url);
    assert_eq!(payload["status"], expected_status);
    assert!(payload["html"].is_string(), "payload={payload}");
}

fn unique_temp_dir(name: &str) -> Result<PathBuf> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let dir = std::env::temp_dir().join(format!("moli-{name}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn unique_temp_file_path(name: &str, file_name: &str) -> Result<PathBuf> {
    Ok(unique_temp_dir(name)?.join(file_name))
}

struct CacheableFixtureServer {
    url: String,
    hits: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

const PDF_FIXTURE_BODY: &[u8] =
    b"%PDF-1.7\n% moli raw document fixture\n1 0 obj\n<<>>\nendobj\n%%EOF\n";

struct BinaryDocumentFixtureServer {
    base_url: String,
    task: JoinHandle<()>,
}

const OPTIONAL_RESOURCE_PATHS: [&str; 5] = [
    "/optional-audio.mp3",
    "/optional-font.woff2",
    "/optional-image.png",
    "/optional-track.vtt",
    "/optional-video.mp4",
];
const OPTIONAL_RESOURCE_FLAG_CASES: [(&str, Option<&str>); 6] = [
    ("--image", Some("/optional-image.png")),
    ("--font", Some("/optional-font.woff2")),
    ("--audio", Some("/optional-audio.mp3")),
    ("--video", Some("/optional-video.mp4")),
    ("--media", None),
    ("--text-track", Some("/optional-track.vtt")),
];
const TRACK_TERMINAL_WAIT_SCRIPT: &str = "document.getElementById('track').readyState === 2";

struct OptionalResourceFixtureServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl OptionalResourceFixtureServer {
    async fn spawn() -> Result<Self> {
        async fn preload_page() -> Html<&'static str> {
            Html(
                r#"<!doctype html>
<html>
  <head>
    <link rel="preload" as="image" href="/optional-image.png">
    <link rel="preload" as="font" href="/optional-font.woff2" crossorigin="anonymous">
    <link rel="preload" as="audio" href="/optional-audio.mp3">
    <link rel="preload" as="video" href="/optional-video.mp4">
    <link rel="preload" as="track" href="/optional-track.vtt">
  </head>
  <body><main id="ready">preload fixture ready</main></body>
</html>"#,
            )
        }

        async fn element_page() -> Html<&'static str> {
            Html(
                r#"<!doctype html>
<html>
  <head>
    <link rel="stylesheet" href="/optional-resources.css">
  </head>
  <body>
    <img id="image" src="/optional-image.png">
    <audio id="audio" preload="auto" src="/optional-audio.mp3"></audio>
    <video id="video" preload="auto" src="/optional-video.mp4">
      <track id="track" default src="/optional-track.vtt">
    </video>
  </body>
</html>"#,
            )
        }

        async fn resource(State(requests): State<Arc<Mutex<Vec<String>>>>, uri: Uri) -> Response {
            let path = uri.path().to_owned();
            requests.lock().push(path.clone());
            let (content_type, body): (&str, Vec<u8>) = match path.as_str() {
                "/optional-resources.css" => (
                    "text/css; charset=utf-8",
                    br#"
                        @font-face {
                            font-family: OptionalFixture;
                            src: url("/optional-font.woff2") format("woff2");
                        }
                        body { font-family: OptionalFixture; }
                    "#
                    .to_vec(),
                ),
                "/optional-image.png" => (
                    "image/png",
                    vec![
                        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
                        0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
                        0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c, 0x02, 0x00, 0x00, 0x00,
                        0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc, 0xff, 0x1f, 0x00,
                        0x03, 0x03, 0x02, 0x00, 0xef, 0xbf, 0x6b, 0xab, 0x00, 0x00, 0x00, 0x00,
                        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
                    ],
                ),
                "/optional-font.woff2" => ("font/woff2", b"fixture-font".to_vec()),
                "/optional-audio.mp3" => ("audio/mpeg", b"fixture-audio".to_vec()),
                "/optional-video.mp4" => ("video/mp4", b"fixture-video".to_vec()),
                "/optional-track.vtt" => (
                    "text/vtt; charset=utf-8",
                    b"WEBVTT\n\n00:00.000 --> 00:01.000\nfixture\n".to_vec(),
                ),
                _ => {
                    return (
                        StatusCode::NOT_FOUND,
                        [("content-type", "text/plain; charset=utf-8")],
                        b"not found".to_vec(),
                    )
                        .into_response();
                }
            };
            (
                StatusCode::OK,
                [
                    ("content-type", content_type),
                    ("cache-control", "no-store"),
                ],
                body,
            )
                .into_response()
        }

        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .route("/preloads.html", get(preload_page))
            .route("/elements.html", get(element_page))
            .route("/optional-resources.css", get(resource))
            .route("/optional-image.png", get(resource))
            .route("/optional-font.woff2", get(resource))
            .route("/optional-audio.mp3", get(resource))
            .route("/optional-video.mp4", get(resource))
            .route("/optional-track.vtt", get(resource))
            .with_state(Arc::clone(&requests));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("optional resource fixture server should serve");
        });
        Ok(Self {
            base_url: format!("http://{addr}"),
            requests,
            task,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn clear_requests(&self) {
        self.requests.lock().clear();
    }

    fn optional_requests(&self) -> Vec<String> {
        let mut requests = self
            .requests
            .lock()
            .iter()
            .filter(|path| OPTIONAL_RESOURCE_PATHS.contains(&path.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        requests.sort();
        requests
    }

    fn request_count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .iter()
            .filter(|request| request.as_str() == path)
            .count()
    }

    async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

impl BinaryDocumentFixtureServer {
    async fn spawn() -> Result<Self> {
        async fn inline_pdf() -> impl IntoResponse {
            (
                [("content-type", "application/pdf")],
                PDF_FIXTURE_BODY.to_vec(),
            )
        }

        async fn attachment_pdf() -> impl IntoResponse {
            (
                [
                    ("content-type", "text/html; charset=utf-8"),
                    ("content-disposition", "attachment; filename=report.html"),
                ],
                PDF_FIXTURE_BODY.to_vec(),
            )
        }

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .route("/inline.pdf", get(inline_pdf))
            .route("/attachment.html", get(attachment_pdf));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("binary fixture server should serve");
        });
        Ok(Self {
            base_url: format!("http://{addr}"),
            task,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

impl CacheableFixtureServer {
    async fn spawn() -> Result<Self> {
        async fn cacheable(
            State(hits): State<Arc<AtomicUsize>>,
        ) -> ([(&'static str, &'static str); 1], Html<String>) {
            let hit = hits.fetch_add(1, Ordering::SeqCst) + 1;
            (
                [("cache-control", "max-age=60")],
                Html(format!(
                    "<!doctype html><html><body><main>http-cache-hit={hit}</main></body></html>"
                )),
            )
        }

        let hits = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .route("/cacheable", get(cacheable))
            .with_state(hits.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("cacheable fixture server should serve");
        });
        Ok(Self {
            url: format!("http://{addr}/cacheable"),
            hits,
            task,
        })
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

fn assert_cli_dump_html_callback_error_contract(output: &Output, expected_message: &str) {
    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);

    assert!(stdout.contains("<main id=\"after\">after</main>"));
    assert!(!stdout.contains("host callback threw"));
    assert!(!stdout.contains("Uncaught Error:"));
    assert!(!stdout.contains("<unknown>:"));
    assert!(
        !stderr.contains(expected_message),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("host callback threw"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("v8 message listener"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("source_line="),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("backtrace:"),
        "stderr start: {stderr} stderr end"
    );
}

fn assert_cli_dump_html_debug_callback_error_contract(output: &Output, expected_message: &str) {
    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);

    assert!(stdout.contains("<main id=\"after\">after</main>"));
    assert!(!stdout.contains("host callback threw"));
    assert!(!stdout.contains("Uncaught Error:"));
    assert!(
        stderr.contains(expected_message),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("host callback threw"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("v8 message listener"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("source_line="),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("backtrace:"),
        "stderr start: {stderr} stderr end"
    );
}

fn assert_callback_error_page(path: &str, expected_message: &str, wait_until: &str) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url(path);
    let output = run_fetch_cli_with_wait_until(&url, wait_until)?;
    runtime.block_on(server.shutdown());
    assert_cli_dump_html_callback_error_contract(&output, expected_message);
    Ok(())
}

fn assert_debug_callback_error_page_after_script(
    path: &str,
    expected_message: &str,
    completion_script: &str,
) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url(path);
    let output = run_fetch_cli_with_log_level_and_args(
        &url,
        "debug",
        &["--wait-script", completion_script],
    )?;
    runtime.block_on(server.shutdown());
    assert_cli_dump_html_debug_callback_error_contract(&output, expected_message);
    Ok(())
}

fn assert_optional_resource_fetch_succeeded(output: &Output, scenario: &str) {
    assert!(
        output.status.success(),
        "{scenario} failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_optional_resource_request_case(
    server: &OptionalResourceFixtureServer,
    page_path: &str,
    flags: &[&str],
    expected_paths: &[&str],
    scenario: &str,
) -> Result<()> {
    server.clear_requests();
    let mut args = flags.to_vec();
    if page_path == "/elements.html" {
        args.extend(["--wait-script", TRACK_TERMINAL_WAIT_SCRIPT]);
    }
    let output = run_fetch_cli_with_args(&server.url(page_path), &args)?;
    assert_optional_resource_fetch_succeeded(&output, scenario);

    let mut expected = expected_paths
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(
        server.optional_requests(),
        expected,
        "{scenario} produced the wrong physical request union"
    );
    for path in OPTIONAL_RESOURCE_PATHS {
        assert_eq!(
            server.request_count(path),
            usize::from(expected.iter().any(|expected| expected == path)),
            "{scenario} produced the wrong request count for {path}"
        );
    }
    if page_path == "/elements.html" {
        assert_eq!(
            server.request_count("/optional-resources.css"),
            1,
            "{scenario} must still fetch the required stylesheet exactly once"
        );
    }
    Ok(())
}

#[test]
fn fetch_cli_default_sends_no_optional_resource_requests_to_a_real_web_server() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(OptionalResourceFixtureServer::spawn())?;

    let preload_output = run_fetch_cli_with_args(&server.url("/preloads.html"), &[])?;
    assert_optional_resource_fetch_succeeded(&preload_output, "default preload page");
    assert!(
        server.optional_requests().is_empty(),
        "default preload page sent optional requests: {:?}",
        server.optional_requests()
    );

    server.clear_requests();
    let element_output = run_fetch_cli_with_args(
        &server.url("/elements.html"),
        &["--wait-script", TRACK_TERMINAL_WAIT_SCRIPT],
    )?;
    assert_optional_resource_fetch_succeeded(&element_output, "default element page");
    assert!(
        server.optional_requests().is_empty(),
        "default element page sent optional requests: {:?}",
        server.optional_requests()
    );
    assert_eq!(
        server.request_count("/optional-resources.css"),
        1,
        "the required stylesheet must still be fetched so the negative assertion is meaningful"
    );

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn fetch_cli_individual_flags_send_only_matching_real_preload_and_element_requests() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(OptionalResourceFixtureServer::spawn())?;

    for page_path in ["/preloads.html", "/elements.html"] {
        for (flag, expected_path) in OPTIONAL_RESOURCE_FLAG_CASES {
            let expected = expected_path.map_or_else(Vec::new, |path| vec![path]);
            assert_optional_resource_request_case(
                &server,
                page_path,
                &[flag],
                &expected,
                &format!("{page_path} {flag}"),
            )?;
        }
    }

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn fetch_cli_representative_flag_subsets_send_exact_real_request_unions() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(OptionalResourceFixtureServer::spawn())?;
    let cases: [(&str, &[&str], &[&str]); 4] = [
        (
            "image+font",
            &["--image", "--font"],
            &["/optional-image.png", "/optional-font.woff2"],
        ),
        (
            "audio+video",
            &["--audio", "--video"],
            &["/optional-audio.mp3", "/optional-video.mp4"],
        ),
        (
            "font+media+track",
            &["--font", "--media", "--text-track"],
            &["/optional-font.woff2", "/optional-track.vtt"],
        ),
        (
            "image+audio+video+track",
            &["--image", "--audio", "--video", "--text-track"],
            &[
                "/optional-image.png",
                "/optional-audio.mp3",
                "/optional-video.mp4",
                "/optional-track.vtt",
            ],
        ),
    ];

    for page_path in ["/preloads.html", "/elements.html"] {
        for (name, flags, expected_paths) in cases {
            assert_optional_resource_request_case(
                &server,
                page_path,
                flags,
                expected_paths,
                &format!("{page_path} {name}"),
            )?;
        }
    }

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn fetch_cli_all_resource_flag_enables_real_preload_and_element_resource_paths() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(OptionalResourceFixtureServer::spawn())?;

    for page_path in ["/preloads.html", "/elements.html"] {
        assert_optional_resource_request_case(
            &server,
            page_path,
            &["--resource"],
            &OPTIONAL_RESOURCE_PATHS,
            &format!("all-resource {page_path}"),
        )?;
    }

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_fetch_about_blank_materializes_empty_document() -> Result<()> {
    let output = run_fetch_cli_with_dump_and_args("about:blank", "json", &[])?;
    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout)?;
    assert_json_dump_shape(&payload, "about:blank", 200);
    assert_eq!(payload["html"], "<html><head></head><body></body></html>");
    Ok(())
}

#[test]
fn cli_dump_screenshot_writes_png_bytes() -> Result<()> {
    let output = run_fetch_cli_with_dump_and_args("about:blank", "screenshot", &["--layout"])?;
    assert!(
        output.status.success(),
        "moli fetch failed: stdout_bytes={}\nstderr={}",
        output.stdout.len(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(output.stdout.ends_with(b"IEND\xaeB`\x82"));
    Ok(())
}

#[test]
fn cli_dump_full_screenshot_writes_png_bytes() -> Result<()> {
    let output = run_fetch_cli_with_dump_and_args("about:blank", "screenshot_full", &["--layout"])?;
    assert!(
        output.status.success(),
        "moli fetch failed: stdout_bytes={}\nstderr={}",
        output.stdout.len(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(output.stdout.ends_with(b"IEND\xaeB`\x82"));
    Ok(())
}

#[test]
fn cli_dump_pdf_writes_pdf_bytes() -> Result<()> {
    let output = run_fetch_cli_with_dump_and_args("about:blank", "pdf", &["--layout"])?;
    assert!(
        output.status.success(),
        "moli fetch failed: stdout_bytes={}\nstderr={}",
        output.stdout.len(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.starts_with(b"%PDF-1.7\n"));
    assert!(
        output
            .stdout
            .windows(b"/Type /Pages".len())
            .any(|window| window == b"/Type /Pages")
    );
    assert!(output.stdout.ends_with(b"%%EOF\n"));
    Ok(())
}

#[test]
fn binary_dump_modes_require_layout() -> Result<()> {
    for dump in ["screenshot", "screenshot_full", "pdf"] {
        let output = run_fetch_cli_with_dump_and_args("about:blank", dump, &[])?;
        assert!(
            !output.status.success(),
            "--dump {dump} unexpectedly worked"
        );
        assert!(
            output.stdout.is_empty(),
            "--dump {dump} wrote partial output"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!("--dump {dump} requires --layout")),
            "stderr={stderr}"
        );
    }
    Ok(())
}

#[test]
fn cli_fetch_inline_pdf_dump_html_bypasses_dcl_page_lifecycle() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(BinaryDocumentFixtureServer::spawn())?;
    let url = server.url("/inline.pdf");
    let output = run_fetch_cli_with_wait_until(&url, "domcontentloaded")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, PDF_FIXTURE_BODY);
    Ok(())
}

#[test]
fn cli_fetch_attachment_dump_html_bypasses_dcl_page_lifecycle() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(BinaryDocumentFixtureServer::spawn())?;
    let url = server.url("/attachment.html");
    let output = run_fetch_cli_with_wait_until(&url, "domcontentloaded")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, PDF_FIXTURE_BODY);
    Ok(())
}

#[test]
fn cli_dump_html_keeps_uncaught_script_errors_out_of_stdout() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/compat/uncaught-script-error");
    let output = run_fetch_cli(&url)?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.contains("<main id=\"after\">after</main>"));
    assert!(!stdout.contains("Uncaught Error: boom"));
    assert!(!stdout.contains("<unknown>:"));
    assert!(
        stderr.contains("Uncaught Error: boom"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("source_line="),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("backtrace:"),
        "stderr start: {stderr} stderr end"
    );
    Ok(())
}

#[test]
fn cli_http_cache_dir_reuses_cached_response_across_processes() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(CacheableFixtureServer::spawn())?;
    let url = server.url.clone();
    let cache_dir = unique_temp_dir("http-cache-cli-e2e")?;
    let cache_dir_arg = cache_dir.to_string_lossy().into_owned();

    let first =
        run_fetch_cli_with_dump_and_args(&url, "json", &["--http-cache-dir", &cache_dir_arg])?;
    assert!(
        first.status.success(),
        "first moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_payload: Value = serde_json::from_slice(&first.stdout)?;
    assert_json_dump_shape(&first_payload, &url, 200);
    assert!(
        first_payload["html"]
            .as_str()
            .is_some_and(|html| html.contains("http-cache-hit=1")),
        "payload={first_payload}"
    );
    assert_eq!(server.hits(), 1);

    let second =
        run_fetch_cli_with_dump_and_args(&url, "json", &["--http-cache-dir", &cache_dir_arg])?;
    assert!(
        second.status.success(),
        "second moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_payload: Value = serde_json::from_slice(&second.stdout)?;
    assert_json_dump_shape(&second_payload, &url, 200);
    assert!(
        second_payload["html"]
            .as_str()
            .is_some_and(|html| html.contains("http-cache-hit=1")),
        "payload={second_payload}"
    );
    assert_eq!(server.hits(), 1);
    assert!(
        std::fs::read_dir(&cache_dir)?.next().is_some(),
        "cache dir should contain persisted response files"
    );
    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_http_cache_dir_skips_cache_when_request_headers_are_present() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(CacheableFixtureServer::spawn())?;
    let cache_dir = unique_temp_dir("http-cache-cli-headers")?;
    let cache_dir_arg = cache_dir.to_string_lossy().into_owned();

    let first = run_fetch_cli_with_dump_and_args(
        &server.url,
        "json",
        &["--http-cache-dir", &cache_dir_arg, "-H", "X-Test: one"],
    )?;
    assert!(
        first.status.success(),
        "first moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_payload: Value = serde_json::from_slice(&first.stdout)?;
    assert_json_dump_shape(&first_payload, &server.url, 200);
    assert!(
        first_payload["html"]
            .as_str()
            .is_some_and(|html| html.contains("http-cache-hit=1")),
        "payload={first_payload}"
    );

    let second = run_fetch_cli_with_dump_and_args(
        &server.url,
        "json",
        &["--http-cache-dir", &cache_dir_arg, "-H", "X-Test: one"],
    )?;
    assert!(
        second.status.success(),
        "second moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_payload: Value = serde_json::from_slice(&second.stdout)?;
    assert_json_dump_shape(&second_payload, &server.url, 200);
    assert!(
        second_payload["html"]
            .as_str()
            .is_some_and(|html| html.contains("http-cache-hit=2")),
        "payload={second_payload}"
    );
    assert_eq!(server.hits(), 2);
    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_cookie_file_imports_netscape_cookie_before_fetch() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/cookie");
    let cookie_file = unique_temp_file_path("cookie-file-import", "cookies.txt")?;
    let cookie_file_arg = cookie_file.to_string_lossy().into_owned();
    let host = url::Url::parse(&url)?
        .host_str()
        .expect("fixture URL should have host")
        .to_owned();
    std::fs::write(
        &cookie_file,
        format!(
            "# Netscape HTTP Cookie File\n{host}\tFALSE\t/\tFALSE\t2147483647\tsession\tfixture\n"
        ),
    )?;

    let output =
        run_fetch_cli_with_dump_and_args(&url, "html", &["--cookie-file", &cookie_file_arg])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("<main>cookie=seen</main>"),
        "stdout={stdout}"
    );
    assert!(
        !stdout.contains("<main>cookie=missing</main>"),
        "stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_profile_dir_persists_cookies_after_successful_fetch() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/cookie");
    let profile_dir = unique_temp_dir("profile-cookie-persist")?;
    let profile_dir_arg = profile_dir.to_string_lossy().into_owned();

    let first =
        run_fetch_cli_with_dump_and_args(&url, "html", &["--profile-dir", &profile_dir_arg])?;
    assert!(
        first.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_stdout = clean_output(&first.stdout);
    assert!(
        first_stdout.contains("<main>cookie=missing</main>"),
        "stdout={first_stdout}"
    );

    let second =
        run_fetch_cli_with_dump_and_args(&url, "html", &["--profile-dir", &profile_dir_arg])?;
    runtime.block_on(server.shutdown());

    assert!(
        second.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout = clean_output(&second.stdout);
    assert!(
        second_stdout.contains("<main>cookie=seen</main>"),
        "stdout={second_stdout}"
    );
    let profile_paths = BrowserProfilePaths::new(&profile_dir);
    let manifest = load_profile_manifest(&profile_paths)?;
    assert_eq!(
        manifest.version,
        moli_browser_profile::PROFILE_MANIFEST_VERSION
    );
    assert_eq!(manifest.partitions.len(), 1);
    Ok(())
}

#[test]
fn cli_cookie_file_with_profile_dir_imports_then_persists_to_profile() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/cookie");
    let profile_dir = unique_temp_dir("cookie-file-profile-import")?;
    let profile_dir_arg = profile_dir.to_string_lossy().into_owned();
    let cookie_file = unique_temp_file_path("cookie-file-profile-import-source", "cookies.txt")?;
    let cookie_file_arg = cookie_file.to_string_lossy().into_owned();
    let host = url::Url::parse(&url)?
        .host_str()
        .expect("fixture URL should have host")
        .to_owned();
    let cookie_file_contents = format!(
        "# Netscape HTTP Cookie File\n{host}\tFALSE\t/\tFALSE\t2147483647\tsession\tfixture\n"
    );
    std::fs::write(&cookie_file, &cookie_file_contents)?;

    let first = run_fetch_cli_with_dump_and_args(
        &url,
        "html",
        &[
            "--profile-dir",
            &profile_dir_arg,
            "--cookie-file",
            &cookie_file_arg,
        ],
    )?;
    assert!(
        first.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_stdout = clean_output(&first.stdout);
    assert!(
        first_stdout.contains("<main>cookie=seen</main>"),
        "stdout={first_stdout}"
    );
    assert_eq!(std::fs::read_to_string(&cookie_file)?, cookie_file_contents);

    let profile_paths = BrowserProfilePaths::new(&profile_dir);
    let profile_cookies = load_profile_cookie_cache(&profile_paths.cookies_path)?;
    assert!(
        profile_cookies
            .iter()
            .any(|cookie| cookie.name == "session" && cookie.value == "fixture"),
        "profile cookies={profile_cookies:?}"
    );

    let second =
        run_fetch_cli_with_dump_and_args(&url, "html", &["--profile-dir", &profile_dir_arg])?;
    runtime.block_on(server.shutdown());

    assert!(
        second.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout = clean_output(&second.stdout);
    assert!(
        second_stdout.contains("<main>cookie=seen</main>"),
        "stdout={second_stdout}"
    );
    assert_eq!(std::fs::read_to_string(&cookie_file)?, cookie_file_contents);
    Ok(())
}

#[test]
fn cli_dump_html_keeps_load_listener_errors_out_of_stdout() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/compat/load-listener-error");
    let output = run_fetch_cli(&url)?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.contains("<main id=\"after\">after</main>"));
    assert!(stdout.contains("data-load-first=\"before-throw\""));
    assert!(stdout.contains("data-load-second=\"after-throw\""));
    assert!(!stdout.contains("<unknown>:"));
    assert!(!stdout.contains("host event listener threw"));
    assert!(!stdout.contains("Uncaught Error: load boom"));
    assert!(
        !stderr.contains("load boom"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("host event listener threw"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("v8 message listener"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("source_line="),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("backtrace:"),
        "stderr start: {stderr} stderr end"
    );
    Ok(())
}

#[test]
fn cli_dump_html_can_debug_log_load_listener_errors() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/compat/load-listener-error");
    let output = run_fetch_cli_with_log_level(&url, "debug")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.contains("<main id=\"after\">after</main>"));
    assert!(!stdout.contains("host event listener threw"));
    assert!(
        stderr.contains("load boom"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("host event listener threw"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("source_line="),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("backtrace:"),
        "stderr start: {stderr} stderr end"
    );
    Ok(())
}

#[test]
fn cli_dump_html_keeps_handled_promise_rejections_out_of_stderr() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/compat/handled-promise-rejection");
    let output = run_fetch_cli(&url)?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.contains("<main id=\"status\">handled</main>"));
    assert!(
        !stderr.contains("handled boom"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("unhandled promise rejection"),
        "stderr start: {stderr} stderr end"
    );
    Ok(())
}

#[test]
fn cli_dump_html_keeps_unhandled_promise_rejections_out_of_stdout() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/compat/unhandled-promise-rejection");
    let output = run_fetch_cli(&url)?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.contains("<main id=\"after\">after</main>"));
    assert!(!stdout.contains("unhandled promise rejection"));
    assert!(!stdout.contains("<unknown>:"));
    assert!(
        stderr.contains("promise boom"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("unhandled promise rejection"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("source_line="),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("backtrace:"),
        "stderr start: {stderr} stderr end"
    );
    Ok(())
}

#[test]
fn cli_dump_html_keeps_caught_dynamic_bare_import_rejections_out_of_stderr() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/compat/caught-dynamic-bare-import");
    let output = run_fetch_cli(&url)?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.contains("<main id=\"status\">caught</main>"));
    assert!(
        !stderr.contains("failed to resolve bare module specifier `_`"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        !stderr.contains("unhandled promise rejection"),
        "stderr start: {stderr} stderr end"
    );
    Ok(())
}

#[test]
fn cli_dump_html_keeps_message_port_callback_errors_out_of_stdout() -> Result<()> {
    assert_callback_error_page(
        "/compat/message-port-callback-error",
        "message port boom",
        "load",
    )
}

#[test]
fn cli_dump_html_can_debug_log_callback_errors() -> Result<()> {
    assert_debug_callback_error_page_after_script(
        "/compat/message-port-callback-error",
        "message port boom",
        "window.__messagePortCallbackCompleted === true",
    )
}

#[test]
fn cli_dump_html_keeps_file_reader_callback_errors_out_of_stdout() -> Result<()> {
    assert_callback_error_page(
        "/compat/file-reader-callback-error",
        "file reader boom",
        "load",
    )
}

#[test]
fn cli_dump_html_keeps_mutation_observer_callback_errors_out_of_stdout() -> Result<()> {
    assert_callback_error_page(
        "/compat/mutation-observer-callback-error",
        "mutation observer boom",
        "load",
    )
}

#[test]
fn cli_dump_html_keeps_resize_observer_callback_errors_out_of_stdout() -> Result<()> {
    assert_callback_error_page(
        "/compat/resize-observer-callback-error",
        "resize observer boom",
        "load",
    )
}

#[test]
fn cli_dump_html_keeps_xhr_callback_errors_out_of_stdout() -> Result<()> {
    assert_callback_error_page(
        "/compat/xhr-callback-error",
        "xhr readyState boom",
        "networkidle",
    )
}

#[test]
fn cli_dump_html_keeps_abort_signal_callback_errors_out_of_stdout() -> Result<()> {
    assert_callback_error_page(
        "/compat/abort-signal-callback-error",
        "abort signal boom",
        "load",
    )
}

#[test]
fn cli_dump_html_serializes_runtime_dom_snapshot_contract() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/compat/dump-dom-snapshot");
    let output = run_fetch_cli(&url)?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    assert!(stdout.starts_with("<!DOCTYPE html>"), "stdout={stdout}");
    assert!(stdout.contains("data-snapshot=\"yes\""), "stdout={stdout}");
    assert!(
        stdout.contains("<main id=\"before\">after-script</main>"),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("<section id=\"after\">runtime</section>"),
        "stdout={stdout}"
    );
    assert!(
        !stdout.to_ascii_lowercase().contains("<meta charset"),
        "stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_fetch_repeats_custom_request_headers() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/net/echo");
    let output = run_fetch_cli_with_args(&url, &["--header", "X-Test: one", "-H", "X-Trace: two"])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    assert!(stdout.contains(r#""x-test":"one""#), "stdout={stdout}");
    assert!(stdout.contains(r#""x-trace":"two""#), "stdout={stdout}");
    Ok(())
}

#[test]
fn cli_fetch_preserves_header_values_with_embedded_colons_and_empty_values() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/net/echo");
    let output = run_fetch_cli_with_args(
        &url,
        &[
            "--header",
            "Authorization: Bearer a:b:c",
            "--header",
            "X-Empty:",
        ],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains(r#""authorization":"Bearer a:b:c""#),
        "stdout={stdout}"
    );
    assert!(stdout.contains(r#""x-empty":"""#), "stdout={stdout}");
    Ok(())
}

#[test]
fn cli_fetch_scopes_custom_headers_to_main_document_request() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/net/header-scope");
    let output = run_fetch_cli_with_args(
        &url,
        &[
            "--header",
            "X-Test: one",
            "--header",
            "X-Trace: two",
            "--wait-selector",
            "#subrequest",
        ],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains(r#"data-nav-x-test="one""#),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains(r#"data-nav-x-trace="two""#),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains(
            r#"<pre id="subrequest">{"method":"GET","received":true,"x-test":"","x-trace":"","authorization":"","x-empty":""}</pre>"#
        ),
        "stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_dump_json_emits_final_url_status_and_html() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/static");
    let output = run_fetch_cli_with_dump_and_args(&url, "json", &[])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout)?;
    assert_json_dump_shape(&payload, &url, 200);
    assert!(
        payload["html"]
            .as_str()
            .is_some_and(|html| html.contains("<main>fixture static</main>"))
    );
    Ok(())
}

#[test]
fn cli_dump_json_trace_network_includes_subresource_summary() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-delayed-fetch");
    let output = run_moli([
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--wait-until",
        "networkidle",
        "--dump",
        "json",
        "--trace-network",
        &url,
    ])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout)?;
    assert_eq!(payload["final_url"], url);
    assert_eq!(payload["status"], 200);
    assert_eq!(payload["network"]["main_document"]["status"], 200);
    let subresources = payload["network"]["subresources"]
        .as_array()
        .expect("network.subresources should be an array");
    let record = subresources
        .iter()
        .find(|record| {
            record["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/wait-until-data"))
        })
        .unwrap_or_else(|| panic!("missing delayed fetch record: {subresources:?}"));
    assert_eq!(record["resource_type"], "Fetch");
    assert_eq!(record["method"], "GET");
    assert_eq!(record["ok"], true);
    assert_eq!(record["status"], 200);
    assert_eq!(record["content_type"], "text/plain; charset=utf-8");
    assert_eq!(record["body_length"], "settled".len());
    Ok(())
}

#[test]
fn cli_wait_response_regexes_match_url_body_and_json_without_networkidle() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-delayed-json-fetch");
    let output = run_moli([
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--wait-until",
        "load",
        "--wait-response-url-regex",
        r"/wait-until-(json|xml)-data$",
        "--wait-response-body-regex",
        "SUCC[A-Z]+",
        "--wait-response-json-regex",
        r"data.url=^/item/\d+$",
        "--dump",
        "json",
        "--trace-network",
        &url,
    ])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout)?;
    assert!(
        payload["html"]
            .as_str()
            .is_some_and(|html| html.contains(r#"id="late-json""#) && html.contains("/item/42")),
        "stdout={stdout}"
    );
    let subresources = payload["network"]["subresources"]
        .as_array()
        .expect("network.subresources should be an array");
    let record = subresources
        .iter()
        .find(|record| {
            record["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/wait-until-json-data"))
        })
        .unwrap_or_else(|| panic!("missing JSON fetch record: {subresources:?}"));
    assert_eq!(record["resource_type"], "Fetch");
    assert_eq!(record["ok"], true);
    assert_eq!(record["content_type"], "application/json");
    assert_eq!(record["json_summary"]["api"], "fixture.detail");
    assert_eq!(record["json_summary"]["data.url"], "/item/42");
    assert_eq!(
        payload["network"]["matched_response"]["url"], record["url"],
        "matched_response should identify the response that satisfied wait-response criteria"
    );
    assert_eq!(
        payload["network"]["matched_response"]["json_summary"]["data.url"],
        "/item/42"
    );
    assert!(
        payload["network"]["matched_response"]
            .get("body_text")
            .is_none(),
        "matched response body should be omitted unless explicitly requested"
    );
    Ok(())
}

#[test]
fn cli_readiness_plan_combines_early_response_selector_and_script() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-readiness-plan");
    let output = run_fetch_cli_with_dump_and_args(
        &url,
        "html",
        &[
            "--timeout",
            "1600",
            "--wait-response-url",
            "/wait-until-json-data",
            "--wait-response-body",
            r#""ret":["SUCCESS"]"#,
            "--wait-response-json",
            "data.url=/item/42",
            "--wait-selector",
            "#readiness-selector",
            "--wait-script",
            "globalThis.readinessScriptReady === true",
        ],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "combined readiness plan failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("data-readiness-response=\"SUCCESS\""),
        "the response completed while the parser-blocking script delayed Load and must remain matchable: {stdout}"
    );
    assert!(
        stdout.contains("id=\"readiness-selector\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("data-readiness-script=\"true\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("data-readiness-order=\"response,selector,script\""),
        "stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_readiness_plan_does_not_restart_timeout_for_script() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-readiness-plan");
    let output = run_fetch_cli_with_dump_and_args(
        &url,
        "html",
        &[
            "--timeout",
            "700",
            "--wait-response-url",
            "/wait-until-json-data",
            "--wait-selector",
            "#readiness-selector",
            "--wait-script",
            "globalThis.readinessScriptReady === true",
        ],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        !output.status.success(),
        "a fresh 700 ms script timeout would incorrectly reach the 500 ms post-Load flag: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("failed while waiting for script to become truthy"),
        "the shared deadline should expire during the final plan phase: stderr={stderr}"
    );
    Ok(())
}

#[test]
fn cli_readiness_timeout_identifies_response_selector_and_script_phases() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/static");
    let cases: [(&[&str], &str); 3] = [
        (
            &["--wait-response-url", "/response-that-never-arrives"],
            "failed while waiting for subresource response",
        ),
        (
            &["--wait-selector", "#selector-that-never-appears"],
            "failed while waiting for selector `#selector-that-never-appears`",
        ),
        (
            &["--wait-script", "globalThis.scriptThatNeverBecomesTruthy"],
            "failed while waiting for script to become truthy",
        ),
    ];

    for (args, expected_phase) in cases {
        let mut args = args.to_vec();
        args.extend(["--timeout", "350"]);
        let output = run_fetch_cli_with_dump_and_args(&url, "html", &args)?;
        let stdout = clean_output(&output.stdout);
        let stderr = clean_output(&output.stderr);
        assert!(
            !output.status.success(),
            "phase={expected_phase} stdout={stdout}"
        );
        assert!(stdout.is_empty(), "phase={expected_phase} stdout={stdout}");
        assert!(
            stderr.contains(expected_phase),
            "expected phase `{expected_phase}` in stderr={stderr}"
        );
    }

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_trace_network_can_include_matched_response_body_text() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-delayed-json-fetch");
    let output = run_moli([
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--wait-until",
        "load",
        "--wait-response-url",
        "/wait-until-json-data",
        "--wait-response-body",
        "SUCCESS",
        "--dump",
        "json",
        "--trace-network",
        "--trace-matched-response-body",
        &url,
    ])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout)?;
    let matched = &payload["network"]["matched_response"];
    assert_eq!(
        matched["url"]
            .as_str()
            .map(|url| url.ends_with("/wait-until-json-data")),
        Some(true)
    );
    assert!(
        matched["body_text"]
            .as_str()
            .is_some_and(|body| body.contains("SUCCESS") && body.contains("/item/42")),
        "matched response body text missing from payload: {matched}"
    );
    let subresources = payload["network"]["subresources"]
        .as_array()
        .expect("network.subresources should be an array");
    assert!(
        subresources
            .iter()
            .all(|record| record.get("body_text").is_none()),
        "body_text should only be included on matched_response: {subresources:?}"
    );
    Ok(())
}

#[test]
fn cli_wait_response_survives_xhr_callback_location_replace() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-xhr-location-replace");
    let output = run_moli([
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--wait-until",
        "load",
        "--wait-response-url",
        "/wait-until-data",
        "--dump",
        "json",
        "--trace-network",
        &url,
    ])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout)?;
    let subresources = payload["network"]["subresources"]
        .as_array()
        .expect("network.subresources should be an array");
    let record = subresources
        .iter()
        .find(|record| {
            record["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/wait-until-data"))
        })
        .unwrap_or_else(|| panic!("missing XHR record: {subresources:?}"));
    assert_eq!(record["resource_type"], "XHR");
    assert_eq!(
        payload["network"]["matched_response"]["url"], record["url"],
        "matched_response should remain available when the response callback queues navigation"
    );
    Ok(())
}

#[test]
fn cli_trace_network_reports_cookie_summary_for_matched_response() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-cookie-fetch");
    let output = run_moli([
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--wait-until",
        "load",
        "--wait-response-url",
        "/wait-until-cookie-data",
        "--wait-response-json",
        "cookie=present",
        "--dump",
        "json",
        "--trace-network",
        &url,
    ])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout)?;
    assert!(
        payload["html"]
            .as_str()
            .is_some_and(|html| html.contains(r#"id="late-cookie""#) && html.contains("present")),
        "stdout={stdout}"
    );
    let matched = &payload["network"]["matched_response"];
    assert_eq!(
        matched["url"]
            .as_str()
            .map(|url| url.ends_with("/wait-until-cookie-data")),
        Some(true)
    );
    assert_eq!(matched["cookies"]["request"]["included"], 1);
    assert_eq!(matched["cookies"]["request"]["excluded"], 0);
    assert_eq!(matched["cookies"]["request"]["access_enabled"], true);
    assert_eq!(matched["cookies"]["request"]["store_available"], true);
    assert_eq!(matched["cookies"]["response"]["accepted"], 1);
    assert_eq!(matched["cookies"]["response"]["rejected"], 0);
    assert!(
        matched["cookies"]["request"]
            .get("included_names")
            .is_none(),
        "trace must not expose cookie names or values: {matched}"
    );
    Ok(())
}

#[test]
fn cli_dump_json_reports_redirected_final_url() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/location-nav/assign-source");
    let final_url = server.url("/location-nav/target?from=assign");
    let output = run_fetch_cli_with_dump_and_args(&url, "json", &[])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout)?;
    assert_json_dump_shape(&payload, &final_url, 200);
    assert!(
        payload["final_url"]
            .as_str()
            .is_some_and(|final_url| final_url.ends_with("/location-nav/target?from=assign")),
        "stdout={stdout}"
    );
    assert!(
        payload["html"]
            .as_str()
            .is_some_and(|html| html.contains("location-target=assign")),
        "stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_domstable_waits_for_late_content() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-delayed-fetch");
    let output = run_fetch_cli_with_wait_until(&url, "domstable")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    assert!(stdout.contains("id=\"late\""), "stdout={stdout}");
    assert!(stdout.contains(">settled<"), "stdout={stdout}");
    Ok(())
}

#[test]
fn cli_networkidle_timeout_logs_warning_and_returns_best_effort_output() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-interval-fetch");
    let output = run_moli([
        "moli",
        "fetch",
        "--log-level",
        "warn",
        "--http-no-proxy",
        "*",
        "--wait-until",
        "networkidle",
        "--timeout",
        "1200",
        "--dump",
        "html",
        &url,
    ])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.contains("data-ping="), "stdout={stdout}");
    assert!(
        stderr.contains("fetch readiness wait timed out; returning best-effort page"),
        "stderr={stderr}"
    );
    assert!(stderr.contains("wait_until=NetworkIdle"), "stderr={stderr}");
    assert!(stderr.contains("timeout_ms=1200"), "stderr={stderr}");
    Ok(())
}

fn assert_cli_best_effort_stage_uses_remaining_deadline(
    wait_until: &str,
    path: &str,
    expected_html: &str,
) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    // The main response consumes 500 ms. A fresh one-second stability timer
    // would make this take roughly 1.5 s; one shared deadline must return a
    // usable best-effort Page at roughly the one-second mark.
    let started = std::time::Instant::now();
    let output = run_moli([
        "moli",
        "fetch",
        "--log-level",
        "warn",
        "--http-no-proxy",
        "*",
        "--wait-until",
        wait_until,
        "--timeout",
        "1000",
        "--dump",
        "html",
        &server.url(path),
    ])?;
    let elapsed = started.elapsed();

    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "wait_until={wait_until} stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.contains(expected_html), "stdout={stdout}");
    assert!(
        stderr.contains("fetch readiness wait timed out; returning best-effort page"),
        "stderr={stderr}"
    );
    assert!(stderr.contains("timeout_ms=1000"), "stderr={stderr}");
    assert!(stderr.contains("remaining_ms="), "stderr={stderr}");
    assert!(
        elapsed < std::time::Duration::from_millis(1_300),
        "wait_until={wait_until} appears to have restarted its timeout: {elapsed:?}"
    );
    Ok(())
}

#[test]
fn cli_networkidle_uses_only_the_deadline_left_after_a_slow_main_response() -> Result<()> {
    assert_cli_best_effort_stage_uses_remaining_deadline(
        "networkidle",
        "/wait-until-slow-interval-fetch",
        "data-state=\"init\"",
    )
}

#[test]
fn cli_domstable_uses_only_the_deadline_left_after_a_slow_main_response() -> Result<()> {
    assert_cli_best_effort_stage_uses_remaining_deadline(
        "domstable",
        "/wait-until-slow-interval-dom-mutation",
        "id=\"mutation-count\"",
    )
}

#[test]
fn cli_best_effort_stage_does_not_soften_a_slow_base_lifecycle_timeout() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-slow-static");

    for wait_until in ["networkidle", "domstable"] {
        // The response takes 500 ms, so at 200 ms no live Page exists yet.
        // This is a hard base-stage failure, not a best-effort stability exit.
        let started = std::time::Instant::now();
        let output = run_moli([
            "moli",
            "fetch",
            "--log-level",
            "warn",
            "--http-no-proxy",
            "*",
            "--wait-until",
            wait_until,
            "--timeout",
            "200",
            "--dump",
            "html",
            &url,
        ])?;
        let elapsed = started.elapsed();

        let stdout = clean_output(&output.stdout);
        let stderr = clean_output(&output.stderr);
        assert!(!output.status.success(), "stdout={stdout}\nstderr={stderr}");
        assert!(stdout.is_empty(), "stdout={stdout}");
        assert!(stderr.contains("timed out after 200 ms"), "stderr={stderr}");
        assert!(
            !stderr.contains("returning best-effort page"),
            "a base lifecycle timeout must not be softened: stderr={stderr}"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(400),
            "wait_until={wait_until} extended the hard base-stage timeout: {elapsed:?}"
        );
    }

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_zero_deadline_cannot_enter_a_best_effort_page_stage() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-slow-static");

    for wait_until in ["networkidle", "domstable"] {
        let started = std::time::Instant::now();
        let output = run_moli([
            "moli",
            "fetch",
            "--log-level",
            "warn",
            "--http-no-proxy",
            "*",
            "--wait-until",
            wait_until,
            "--timeout",
            "0",
            "--dump",
            "html",
            &url,
        ])?;

        let stdout = clean_output(&output.stdout);
        let stderr = clean_output(&output.stderr);
        assert!(!output.status.success(), "stdout={stdout}\nstderr={stderr}");
        assert!(stdout.is_empty(), "stdout={stdout}");
        assert!(stderr.contains("timed out after 0 ms"), "stderr={stderr}");
        assert!(
            !stderr.contains("returning best-effort page"),
            "zero budget cannot produce a Page to return: stderr={stderr}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "wait_until={wait_until} did not fail its zero deadline immediately"
        );
    }

    runtime.block_on(server.shutdown());
    Ok(())
}

fn assert_cli_quiet_page_uses_remaining_best_effort_budget(wait_until: &str) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-slow-static");
    // After the 500 ms response, 250 ms remains. That is shorter than either
    // stability window, although a freshly started timer would let this
    // otherwise quiet page settle successfully.
    let output = run_moli([
        "moli",
        "fetch",
        "--log-level",
        "warn",
        "--http-no-proxy",
        "*",
        "--wait-until",
        wait_until,
        "--timeout",
        "750",
        "--dump",
        "html",
        &url,
    ])?;

    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "wait_until={wait_until} stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.contains("slow-main=ready"), "stdout={stdout}");
    assert!(
        stderr.contains("fetch readiness wait timed out; returning best-effort page"),
        "wait_until={wait_until} should use only the remaining budget: stderr={stderr}"
    );
    Ok(())
}

#[test]
fn cli_quiet_page_networkidle_uses_remaining_best_effort_budget() -> Result<()> {
    assert_cli_quiet_page_uses_remaining_best_effort_budget("networkidle")
}

#[test]
fn cli_quiet_page_domstable_uses_remaining_best_effort_budget() -> Result<()> {
    assert_cli_quiet_page_uses_remaining_best_effort_budget("domstable")
}

#[test]
fn cli_post_readiness_wait_cannot_restart_an_exhausted_best_effort_deadline() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-slow-interval-fetch");
    let started = std::time::Instant::now();
    let output = run_moli([
        "moli",
        "fetch",
        "--log-level",
        "warn",
        "--http-no-proxy",
        "*",
        "--wait-until",
        "networkidle",
        "--wait-selector",
        "#never-appears",
        "--timeout",
        "1000",
        "--dump",
        "html",
        &url,
    ])?;
    let elapsed = started.elapsed();
    runtime.block_on(server.shutdown());

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(!output.status.success(), "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("returning best-effort page"),
        "stderr={stderr}"
    );
    assert!(
        stderr.contains("failed while waiting for selector `#never-appears`"),
        "stderr={stderr}"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(1_300),
        "the selector appears to have received a fresh timeout: {elapsed:?}"
    );
    Ok(())
}

#[test]
fn cli_domstable_waits_for_inflight_slow_fetch_content() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-complete-slow-fetch");
    let output = run_fetch_cli_with_wait_until(&url, "domstable")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    assert!(stdout.contains("id=\"late-slow-fetch\""), "stdout={stdout}");
    assert!(stdout.contains(">settled-very-slow<"), "stdout={stdout}");
    Ok(())
}

#[test]
fn cli_domstable_waits_for_slow_post_domcontentloaded_runtime_script() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-domcontentloaded-runtime-script-very-slow");
    let output = run_fetch_cli_with_wait_until(&url, "domstable")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("id=\"late-dcl-script-very-slow\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains(">script-loaded-very-slow<"),
        "stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_load_waits_for_slow_post_domcontentloaded_runtime_script() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-domcontentloaded-runtime-script-slow");
    let output = run_fetch_cli_with_wait_until(&url, "load")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("id=\"late-dcl-script-slow\""),
        "stdout={stdout}"
    );
    assert!(stdout.contains(">script-loaded-slow<"), "stdout={stdout}");
    Ok(())
}

#[test]
fn cli_domcontentloaded_recovers_from_delayed_403_navigation_at_same_stage() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-navigation");
    let output = run_fetch_cli_with_wait_until(&url, "domcontentloaded")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("id=\"http-error-navigation-target\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("data-reached-dcl=\"true\""),
        "stdout={stdout}"
    );
    assert!(!stdout.contains("id=\"challenge\""), "stdout={stdout}");
    Ok(())
}

#[test]
fn cli_load_recovers_from_delayed_403_navigation_at_same_stage() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-navigation");
    let output = run_fetch_cli_with_wait_until(&url, "load")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("id=\"http-error-navigation-target\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("data-reached-load=\"true\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("id=\"http-error-navigation-load-tail\""),
        "load must include the replacement load tail: stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_load_follows_five_same_url_replacements_after_403() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-five-navigations");
    let output = run_fetch_cli_with_wait_until(&url, "load")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("id=\"http-error-five-navigation-target\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("id=\"http-error-five-navigation-load-tail\""),
        "the fifth replacement must still reach Load: stdout={stdout}"
    );
    assert!(!stdout.contains("id=\"challenge\""), "stdout={stdout}");
    Ok(())
}

#[test]
fn cli_load_uses_one_timeout_across_403_replacement_navigation() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-navigation");
    let output = run_fetch_cli_with_dump_and_args(&url, "html", &["--timeout", "250"])?;
    runtime.block_on(server.shutdown());

    assert!(!output.status.success());
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("allow-http-error wait_until Load timed out after 250 ms"),
        "stderr={stderr}"
    );
    Ok(())
}

#[test]
fn cli_default_done_recovers_from_delayed_403_navigation_at_load() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-navigation");
    let output = run_fetch_cli_with_default_wait_and_dump(&url, "json")?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    let payload: Value = serde_json::from_str(&stdout)?;
    assert_json_dump_shape(&payload, &url, 200);
    let html = payload["html"].as_str().unwrap_or_default();
    assert!(
        html.contains("id=\"http-error-navigation-target\""),
        "stdout={stdout}"
    );
    assert!(
        html.contains("id=\"http-error-navigation-load-tail\""),
        "stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_explicit_load_fails_when_404_does_not_navigate_and_does_not_retry() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/net/upstream/xhr/404-then-200");
    let output = run_fetch_cli_with_dump_and_args(&url, "json", &[])?;
    runtime.block_on(server.shutdown());

    assert!(
        !output.status.success(),
        "an HTTP error document without navigation must fail: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(stderr.contains("404 Not Found"), "stderr={stderr}");
    assert!(stderr.contains("1000 ms grace period"), "stderr={stderr}");
    Ok(())
}

#[test]
fn cli_explicit_domcontentloaded_fails_when_500_does_not_navigate() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/net/upstream/xhr/500");
    let output = run_fetch_cli_with_wait_until(&url, "domcontentloaded")?;
    runtime.block_on(server.shutdown());

    assert!(!output.status.success());
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("500 Internal Server Error"),
        "stderr={stderr}"
    );
    assert!(stderr.contains("1000 ms grace period"), "stderr={stderr}");
    Ok(())
}

#[test]
fn cli_explicit_load_rejects_navigation_after_the_one_second_grace() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-late-navigation");
    let output = run_fetch_cli_with_wait_until(&url, "load")?;
    runtime.block_on(server.shutdown());

    assert!(!output.status.success());
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(stderr.contains("403 Forbidden"), "stderr={stderr}");
    assert!(stderr.contains("1000 ms grace period"), "stderr={stderr}");
    Ok(())
}

#[test]
fn cli_redirect_wait_can_extend_http_error_navigation_grace() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-late-navigation");
    let output = run_fetch_cli_with_dump_and_args(&url, "html", &["--redirect-wait-ms", "1500"])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("id=\"http-error-navigation-target\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("id=\"http-error-navigation-load-tail\""),
        "the configured grace must still preserve the requested Load stage: stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_readiness_plan_continues_after_http_error_replacement_navigation() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-readiness-http-error-navigation");
    let output = run_fetch_cli_with_dump_and_args(
        &url,
        "html",
        &[
            "--timeout",
            "1800",
            "--wait-response-url",
            "/wait-until-json-data",
            "--wait-response-json",
            "data.url=/item/42",
            "--wait-selector",
            "#readiness-navigation-selector",
            "--wait-script",
            "globalThis.readinessNavigationScriptReady === true",
        ],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "replacement readiness plan failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("id=\"readiness-navigation-target\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("data-readiness-navigation-response=\"SUCCESS\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("id=\"readiness-navigation-selector\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("data-readiness-navigation-script=\"true\""),
        "stdout={stdout}"
    );
    assert!(!stdout.contains("id=\"readiness-challenge\""));
    Ok(())
}

#[test]
fn cli_http_error_replacement_and_post_waits_share_one_deadline() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-readiness-http-error-navigation");
    let output = run_fetch_cli_with_dump_and_args(
        &url,
        "html",
        &[
            "--timeout",
            "850",
            "--wait-response-url",
            "/wait-until-json-data",
            "--wait-selector",
            "#readiness-navigation-selector",
            "--wait-script",
            "globalThis.readinessNavigationScriptReady === true",
        ],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        !output.status.success(),
        "a fresh post-navigation script timeout would incorrectly succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("failed while waiting for script to become truthy"),
        "the replacement and earlier post waits should leave the script phase with only the remaining budget: stderr={stderr}"
    );
    Ok(())
}

#[test]
fn cli_zero_redirect_wait_accepts_navigation_already_pending_at_stage() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-immediate-navigation");
    let output = run_fetch_cli_with_dump_and_args(&url, "html", &["--redirect-wait-ms", "0"])?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "a replacement already pending at the 403 Load stage must not need a grace timer: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = clean_output(&output.stdout);
    assert!(
        stdout.contains("id=\"http-error-navigation-target\""),
        "stdout={stdout}"
    );
    assert!(
        stdout.contains("id=\"http-error-navigation-load-tail\""),
        "stdout={stdout}"
    );
    Ok(())
}

#[test]
fn cli_zero_redirect_wait_rejects_navigation_that_starts_later() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-navigation");
    let output = run_fetch_cli_with_dump_and_args(&url, "html", &["--redirect-wait-ms", "0"])?;
    runtime.block_on(server.shutdown());

    assert!(!output.status.success());
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(stderr.contains("403 Forbidden"), "stderr={stderr}");
    assert!(stderr.contains("0 ms grace period"), "stderr={stderr}");
    Ok(())
}

#[test]
fn cli_explicit_load_rejects_http_error_navigation_to_another_error() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-http-error-navigation-to-error");
    let output = run_fetch_cli_with_wait_until(&url, "load")?;
    runtime.block_on(server.shutdown());

    assert!(!output.status.success());
    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("500 Internal Server Error"),
        "stderr={stderr}"
    );
    assert!(
        stderr.contains("reached another HTTP error document"),
        "stderr={stderr}"
    );
    Ok(())
}

#[test]
fn cli_non_lifecycle_wait_modes_keep_http_error_dump_behavior() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/net/upstream/xhr/404");

    for wait_until in ["networkidle", "domstable"] {
        let output = run_fetch_cli_with_wait_until(&url, wait_until)?;
        assert!(
            output.status.success(),
            "wait_until={wait_until} stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            clean_output(&output.stdout).contains("Not Found"),
            "wait_until={wait_until} stdout={}",
            clean_output(&output.stdout)
        );
    }

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_default_done_fails_when_http_error_document_does_not_navigate() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/net/upstream/xhr/404");
    let output = run_fetch_cli_with_default_wait_and_dump(&url, "json")?;
    runtime.block_on(server.shutdown());

    let stdout = clean_output(&output.stdout);
    assert!(
        !output.status.success(),
        "default done must fail an HTTP error document without navigation: stdout={stdout}"
    );
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(stderr.contains("404 Not Found"), "stderr={stderr}");
    assert!(stderr.contains("1000 ms grace period"), "stderr={stderr}");
    Ok(())
}

#[test]
fn cli_default_done_http_error_wait_does_not_retry_request() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/net/upstream/xhr/404-then-200");
    let output = run_fetch_cli_with_default_wait_and_dump(&url, "json")?;
    runtime.block_on(server.shutdown());

    let stdout = clean_output(&output.stdout);
    assert!(
        !output.status.success(),
        "a hidden HTTP retry would incorrectly receive the fixture's second 200 response"
    );
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(stderr.contains("404 Not Found"), "stderr={stderr}");
    Ok(())
}

#[test]
fn cli_dump_json_keeps_transport_failures_as_process_errors() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0"))?;
    let addr = listener.local_addr()?;
    let disconnect = runtime.spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("transport failure fixture should accept");
        drop(stream);
    });
    let url = format!("http://{addr}/transport-failure");

    let output = run_fetch_cli_with_dump_and_args(&url, "json", &[])?;
    runtime.block_on(disconnect)?;

    assert!(
        !output.status.success(),
        "expected transport failure to keep non-zero exit: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(stderr.contains("failed to fetch"), "stderr={stderr}");
    Ok(())
}

#[test]
fn cli_streaming_page_creation_timeout_exits_cleanly_for_every_wait_mode() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/streaming/slow-html-tail");

    // Run a real child process so this regression distinguishes the expected
    // exit code from SIGABRT. The response has committed its headers and first
    // HTML chunk, but page creation remains parked on the delayed parser tail
    // when the shared readiness deadline expires.
    for wait_until in [
        "domcontentloaded",
        "load",
        "networkidle",
        "domstable",
        "done",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_moli"))
            .args([
                "fetch",
                "--log-level",
                "error",
                "--http-no-proxy",
                "*",
                "--wait-until",
                wait_until,
                "--timeout",
                "400",
                "--dump",
                "html",
                &url,
            ])
            .output()?;
        let stdout = clean_output(&output.stdout);
        let stderr = clean_output(&output.stderr);

        assert_eq!(
            output.status.code(),
            Some(1),
            "wait_until={wait_until} must report a normal fetch failure instead of aborting: stdout={stdout}\nstderr={stderr}"
        );
        assert!(stdout.is_empty(), "wait_until={wait_until} stdout={stdout}");
        assert!(
            stderr.contains("timed out after 400 ms"),
            "wait_until={wait_until} stderr={stderr}"
        );
        assert!(
            !stderr.contains("resident renderer page entry must retain an active PageVm"),
            "wait_until={wait_until} leaked the renderer teardown invariant: stderr={stderr}"
        );
    }

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_domcontentloaded_exit_stays_stable_with_inflight_background_fetch_tail() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/wait-until-domcontentloaded-runtime-script-slow");

    // This reproduces the CLI shape that used to be risky: return at DCL while
    // a post-DCL external script fetch is still in flight, then let the fetch
    // subprocess tear down the page and browser normally.
    for attempt in 0..12 {
        let output = run_fetch_cli_with_wait_until(&url, "domcontentloaded")?;
        assert!(
            output.status.success(),
            "attempt={attempt} moli fetch failed: stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = clean_output(&output.stdout);
        assert!(
            stdout.contains("runtimeOwnedDclInjectedDclOrder"),
            "attempt={attempt} stdout={stdout}"
        );
        assert!(
            !stdout.contains("id=\"late-dcl-script-slow\""),
            "attempt={attempt} stdout should not include post-DCL slow script load tail: {stdout}"
        );
    }

    runtime.block_on(server.shutdown());
    Ok(())
}

const ROBOTS_FIXTURE_PAGE: &str =
    "<!doctype html><html><body><main id=\"ready\">robots fixture page</main></body></html>";

#[derive(Clone)]
struct RobotsFixtureState {
    robots_status: StatusCode,
    robots_body: &'static str,
    robots_hits: Arc<AtomicUsize>,
    page_hits: Arc<AtomicUsize>,
}

struct RobotsFixtureServer {
    base_url: String,
    robots_hits: Arc<AtomicUsize>,
    page_hits: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl RobotsFixtureServer {
    async fn spawn(robots_status: StatusCode, robots_body: &'static str) -> Result<Self> {
        async fn robots_txt(State(state): State<RobotsFixtureState>) -> impl IntoResponse {
            state.robots_hits.fetch_add(1, Ordering::SeqCst);
            (
                state.robots_status,
                [("content-type", "text/plain; charset=utf-8")],
                state.robots_body,
            )
        }

        async fn page(State(state): State<RobotsFixtureState>) -> Html<&'static str> {
            state.page_hits.fetch_add(1, Ordering::SeqCst);
            Html(ROBOTS_FIXTURE_PAGE)
        }

        let robots_hits = Arc::new(AtomicUsize::new(0));
        let page_hits = Arc::new(AtomicUsize::new(0));
        let state = RobotsFixtureState {
            robots_status,
            robots_body,
            robots_hits: Arc::clone(&robots_hits),
            page_hits: Arc::clone(&page_hits),
        };

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let app = Router::new()
            .route("/robots.txt", get(robots_txt))
            .route("/allowed", get(page))
            .route("/private/secret", get(page))
            .with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("robots fixture server should serve");
        });

        Ok(Self {
            base_url: format!("http://{addr}"),
            robots_hits,
            page_hits,
            task,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn robots_hits(&self) -> usize {
        self.robots_hits.load(Ordering::SeqCst)
    }

    fn page_hits(&self) -> usize {
        self.page_hits.load(Ordering::SeqCst)
    }

    async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

const ROBOTS_DISALLOWING_PRIVATE: &str = "User-agent: *\nDisallow: /private\n";

#[test]
fn cli_obey_robots_refuses_a_disallowed_url_without_requesting_it() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(RobotsFixtureServer::spawn(
        StatusCode::OK,
        ROBOTS_DISALLOWING_PRIVATE,
    ))?;
    let url = server.url("/private/secret");

    let output = run_fetch_cli_with_args(&url, &["--obey-robots"])?;

    assert!(
        !output.status.success(),
        "moli fetch should have been refused: stdout={}",
        clean_output(&output.stdout)
    );
    let stderr = clean_output(&output.stderr);
    assert!(stderr.contains("is disallowed by"), "stderr={stderr}");
    assert!(stderr.contains("/robots.txt"), "stderr={stderr}");
    assert_eq!(server.robots_hits(), 1, "robots.txt should be read once");
    assert_eq!(
        server.page_hits(),
        0,
        "a disallowed URL must never be requested"
    );

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_obey_robots_allows_a_permitted_url() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(RobotsFixtureServer::spawn(
        StatusCode::OK,
        ROBOTS_DISALLOWING_PRIVATE,
    ))?;
    let url = server.url("/allowed");

    let output = run_fetch_cli_with_args(&url, &["--obey-robots"])?;

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    assert!(
        clean_output(&output.stdout).contains("robots fixture page"),
        "stdout={}",
        clean_output(&output.stdout)
    );
    assert_eq!(server.robots_hits(), 1);
    assert_eq!(server.page_hits(), 1);

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_without_obey_robots_never_reads_robots_txt() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(RobotsFixtureServer::spawn(
        StatusCode::OK,
        ROBOTS_DISALLOWING_PRIVATE,
    ))?;
    let url = server.url("/private/secret");

    let output = run_fetch_cli_with_args(&url, &[])?;

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    assert_eq!(
        server.robots_hits(),
        0,
        "the default fetch must not pay for a robots.txt request"
    );
    assert_eq!(server.page_hits(), 1);

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_obey_robots_allows_everything_when_robots_txt_is_absent() -> Result<()> {
    // RFC 9309 §2.3.1.3: a 4xx means the origin published no rules.
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(RobotsFixtureServer::spawn(StatusCode::NOT_FOUND, ""))?;
    let url = server.url("/private/secret");

    let output = run_fetch_cli_with_args(&url, &["--obey-robots"])?;

    assert!(
        output.status.success(),
        "moli fetch failed: stdout={}\nstderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    assert_eq!(server.page_hits(), 1);

    runtime.block_on(server.shutdown());
    Ok(())
}

#[test]
fn cli_obey_robots_refuses_everything_when_robots_txt_is_unreachable() -> Result<()> {
    // RFC 9309 §2.3.1.4: a 5xx hides rules that may exist, so nothing is safe.
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(RobotsFixtureServer::spawn(
        StatusCode::SERVICE_UNAVAILABLE,
        "",
    ))?;
    let url = server.url("/allowed");

    let output = run_fetch_cli_with_args(&url, &["--obey-robots"])?;

    assert!(
        !output.status.success(),
        "moli fetch should have been refused: stdout={}",
        clean_output(&output.stdout)
    );
    let stderr = clean_output(&output.stderr);
    assert!(stderr.contains("could not be read"), "stderr={stderr}");
    assert_eq!(server.page_hits(), 0);

    runtime.block_on(server.shutdown());
    Ok(())
}

const ROBOTS_NAMING_ONE_AGENT: &str =
    "User-agent: MoliTestBot\nDisallow: /private\n\nUser-agent: *\nAllow: /\n";

#[test]
fn cli_obey_robots_applies_the_group_naming_the_configured_user_agent() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(RobotsFixtureServer::spawn(
        StatusCode::OK,
        ROBOTS_NAMING_ONE_AGENT,
    ))?;
    let url = server.url("/private/secret");

    let named =
        run_fetch_cli_with_args(&url, &["--obey-robots", "--user-agent", "MoliTestBot/1.0"])?;
    assert!(
        !named.status.success(),
        "the named group must refuse: stdout={}",
        clean_output(&named.stdout)
    );
    assert_eq!(server.page_hits(), 0);

    // The same URL is fine for a user agent the file does not name, which is
    // what proves the group was selected rather than merged.
    let unnamed = run_fetch_cli_with_args(&url, &["--obey-robots"])?;
    assert!(
        unnamed.status.success(),
        "the wildcard group must allow: stdout={}\nstderr={}",
        clean_output(&unnamed.stdout),
        clean_output(&unnamed.stderr)
    );
    assert_eq!(server.page_hits(), 1);

    runtime.block_on(server.shutdown());
    Ok(())
}
