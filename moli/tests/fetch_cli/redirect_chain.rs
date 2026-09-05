use super::{clean_output, run_moli, unique_temp_dir};
use anyhow::Result;
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::Notify,
    task::{JoinHandle, JoinSet},
};

const JSON_BODY: &str = r#"{"result":"redirect-destination"}"#;

struct RedirectFixture {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl RedirectFixture {
    async fn spawn_streaming(html: bool) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let release = Arc::new(Notify::new());
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.expect("accept streaming fixture request");
                        connections.spawn(serve_streaming_request(
                            stream, seen.clone(), release.clone(), html,
                        ));
                    }
                    result = connections.join_next(), if !connections.is_empty() => {
                        result.unwrap().expect("fixture task").expect("fixture response");
                    }
                }
            }
        });
        Ok(Self {
            base,
            requests,
            task,
        })
    }

    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/redirect/{count}", get(redirect))
            .route("/relative-redirect/{count}", get(redirect))
            .route("/absolute-redirect/{count}", get(redirect))
            .route("/get", get(destination))
            .with_state(requests.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("HTTP redirect fixture should serve");
        });
        Ok(Self {
            base,
            requests,
            task,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn fetch(&self, path: &str, args: &[&str]) -> Result<Value> {
        let url = self.url(path);
        let mut command = vec![
            "moli",
            "fetch",
            "--log-level",
            "error",
            "--http-no-proxy",
            "*",
            "--dump",
            "json",
        ];
        command.extend_from_slice(args);
        command.push(&url);
        let output = run_moli(command)?;
        assert!(
            output.status.success(),
            "path={path} args={args:?}\nstdout={}\nstderr={}",
            clean_output(&output.stdout),
            clean_output(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn assert_chain(&self, payload: &Value, start: &str, count: usize) {
        let suffix = start.find('?').map_or("", |index| &start[index..]);
        let absolute = start.starts_with("/absolute-redirect/");
        let prefix = if absolute {
            "/absolute-redirect"
        } else {
            "/relative-redirect"
        };
        assert_eq!(payload["status"], 200);
        assert_eq!(payload["final_url"], self.url(&format!("/get{suffix}")));
        let redirects = payload["redirect_chain"]
            .as_array()
            .expect("redirect array");
        assert_eq!(redirects.len(), count, "payload={payload}");
        let mut from = start.to_owned();
        for (index, redirect) in redirects.iter().enumerate() {
            let to = if index + 1 == count {
                format!("/get{suffix}")
            } else {
                format!("{prefix}/{}{suffix}", count - index - 1)
            };
            assert_eq!(redirect["from_url"], self.url(&from));
            assert_eq!(redirect["to_url"], self.url(&to));
            assert_eq!(redirect["status"], 302);
            let headers = redirect["headers"].as_array().expect("redirect headers");
            // Hop URLs are always resolved, but Location retains the original
            // header value, including its absolute/relative form.
            let location = if absolute { self.url(&to) } else { to.clone() };
            assert!(headers.iter().any(|header| {
                header["name"]
                    .as_str()
                    .unwrap()
                    .eq_ignore_ascii_case("location")
                    && header["value"] == location
            }));
            let markers: Vec<_> = headers
                .iter()
                .filter(|header| {
                    header["name"]
                        .as_str()
                        .unwrap()
                        .eq_ignore_ascii_case("x-hop")
                })
                .map(|header| header["value"].clone())
                .collect();
            assert_eq!(markers, [json!("first"), json!("second")]);
            from = to;
        }
        if suffix == "?html" {
            assert_eq!(payload["title"], "destination");
            assert!(
                payload["html"]
                    .as_str()
                    .unwrap()
                    .contains("redirect-destination")
            );
            assert!(payload.get("body_base64").is_none());
        } else if suffix == "?download" {
            assert_eq!(payload["title"], Value::Null);
            assert_eq!(payload["html"], Value::Null);
            assert_eq!(
                STANDARD
                    .decode(payload["body_base64"].as_str().unwrap())
                    .unwrap(),
                JSON_BODY.as_bytes()
            );
        } else {
            // JSON is a renderable text Document, not a raw download, unless
            // Content-Disposition explicitly makes it an attachment.
            assert_eq!(payload["title"], "");
            assert!(payload["html"].as_str().unwrap().contains(JSON_BODY));
            assert!(payload.get("body_base64").is_none());
        }
    }
}

impl Drop for RedirectFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// Mirror httpbin's relative and absolute redirect endpoints: each response is
// a 302, ending at /get with application/json.
// ?html and ?download cover HTML and attachment output with the same hops.
async fn redirect(
    Path(count): Path<usize>,
    State(requests): State<Arc<Mutex<Vec<String>>>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    requests.lock().push(uri.to_string());
    let suffix = uri
        .query()
        .map_or(String::new(), |query| format!("?{query}"));
    let absolute = uri.path().starts_with("/absolute-redirect/");
    let prefix = if absolute {
        "/absolute-redirect"
    } else {
        "/relative-redirect"
    };
    let path = if count <= 1 {
        format!("/get{suffix}")
    } else {
        format!("{prefix}/{}{suffix}", count - 1)
    };
    let target = if absolute {
        let host = headers[header::HOST].to_str().unwrap();
        format!("http://{host}{path}")
    } else {
        path
    };
    let mut response = (StatusCode::FOUND, Html("<p>Redirecting...</p>")).into_response();
    response
        .headers_mut()
        .insert(header::LOCATION, HeaderValue::from_str(&target).unwrap());
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    response
        .headers_mut()
        .append("x-hop", HeaderValue::from_static("first"));
    response
        .headers_mut()
        .append("x-hop", HeaderValue::from_static("second"));
    response
}

async fn destination(State(requests): State<Arc<Mutex<Vec<String>>>>, uri: Uri) -> Response {
    requests.lock().push(uri.to_string());
    let mut response = if uri.query() == Some("html") {
        Html("<!doctype html><title>destination</title><main>redirect-destination</main>")
            .into_response()
    } else {
        ([(header::CONTENT_TYPE, "application/json")], JSON_BODY).into_response()
    };
    if uri.query() == Some("download") {
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=response.json"),
        );
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    response
}

async fn serve_streaming_request(
    stream: TcpStream,
    requests: Arc<Mutex<Vec<String>>>,
    release: Arc<Notify>,
    html: bool,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let path = line
        .split_whitespace()
        .nth(1)
        .expect("request target")
        .to_owned();
    requests.lock().push(path.clone());
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 || line == "\r\n" {
            break;
        }
    }
    let mut stream = reader.into_inner();
    let suffix = if html { "?html" } else { "" };
    if path.starts_with("/redirect/") || path.starts_with("/relative-redirect/") {
        let count: usize = path
            .trim_end_matches(suffix)
            .rsplit('/')
            .next()
            .unwrap()
            .parse()?;
        let location = if count > 1 {
            format!("/relative-redirect/{}{suffix}", count - 1)
        } else {
            format!("/get{suffix}")
        };
        stream.write_all(format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nX-Hop: first\r\nX-Hop: second\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ).as_bytes()).await?;
    } else if path.starts_with("/get") {
        let (content_type, body) = if html {
            (
                "text/html",
                "<!doctype html><title>destination</title><main>redirect-destination</main>",
            )
        } else {
            ("application/json", JSON_BODY)
        };
        stream.write_all(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len(),
        ).as_bytes()).await?;
        stream.write_all(&body.as_bytes()[..12]).await?;
        // Keep the main response open until a document-start script has
        // consumed one fetch response and issued a second request. This
        // requires the Page owner to run while its parser is suspended.
        release.notified().await;
        stream.write_all(&body.as_bytes()[12..]).await?;
    } else {
        assert!(matches!(path.as_str(), "/gate" | "/release"));
        if path == "/release" {
            release.notify_one();
        }
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .await?;
    }
    stream.shutdown().await?;
    Ok(())
}

#[test]
fn cli_redirect_chain_httpbin_json_destinations() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let fixture = runtime.block_on(RedirectFixture::spawn())?;
    for (path, count) in [
        ("/redirect/3", 3),
        ("/redirect/1", 1),
        ("/relative-redirect/2", 2),
        ("/absolute-redirect/2", 2),
    ] {
        let payload = fixture.fetch(path, &[])?;
        fixture.assert_chain(&payload, path, count);
    }
    let payload = fixture.fetch("/get", &[])?;
    fixture.assert_chain(&payload, "/get", 0);
    Ok(())
}

#[test]
fn cli_redirect_chain_is_independent_of_trace_and_readiness() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let fixture = runtime.block_on(RedirectFixture::spawn())?;
    for path in [
        "/redirect/3",
        "/relative-redirect/3",
        "/absolute-redirect/3",
        "/redirect/3?html",
        "/redirect/3?download",
    ] {
        for wait in [
            "domcontentloaded",
            "load",
            "done",
            "networkidle",
            "domstable",
        ] {
            for trace in [false, true] {
                let mut args = vec!["--wait-until", wait];
                if trace {
                    args.push("--trace-network");
                }
                let payload = fixture.fetch(path, &args)?;
                fixture.assert_chain(&payload, path, 3);
                if path != "/redirect/3?download" && trace {
                    assert_eq!(payload["network"]["main_document"]["redirected"], true);
                    assert_eq!(payload["network"]["main_document"]["redirect_count"], 3);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn cli_redirect_chain_survives_cached_hops_and_cached_final_response() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let fixture = runtime.block_on(RedirectFixture::spawn())?;
    let cache = unique_temp_dir("redirect-chain-cache")?;
    let cache_arg = cache.to_string_lossy();
    for path in ["/redirect/3", "/redirect/3?html", "/redirect/3?download"] {
        let first = fixture.fetch(path, &["--http-cache-dir", &cache_arg])?;
        fixture.assert_chain(&first, path, 3);
        let requests = fixture.requests.lock().clone();
        for _ in 0..2 {
            let cached = fixture.fetch(path, &["--http-cache-dir", &cache_arg])?;
            fixture.assert_chain(&cached, path, 3);
            assert_eq!(cached["redirect_chain"], first["redirect_chain"]);
            assert_eq!(
                *fixture.requests.lock(),
                requests,
                "warm fetch must hit the cache"
            );
        }
    }
    Ok(())
}

#[test]
fn cli_redirect_chain_process_stdout_reports_all_httpbin_hops() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let fixture = runtime.block_on(RedirectFixture::spawn())?;
    for (path, count) in [
        ("/redirect/3", 3),
        ("/redirect/1", 1),
        ("/relative-redirect/2", 2),
        ("/absolute-redirect/2", 2),
    ] {
        for trace in [false, true] {
            let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_moli"));
            command.args(["fetch", "--http-no-proxy", "*", "--dump", "json"]);
            if trace {
                command.arg("--trace-network");
            }
            let output = command.arg(fixture.url(path)).output()?;
            assert!(
                output.status.success(),
                "path={path} trace={trace}\nstderr={}",
                clean_output(&output.stderr)
            );
            let payload = serde_json::from_slice(&output.stdout)?;
            fixture.assert_chain(&payload, path, count);
        }
    }
    Ok(())
}

#[test]
fn cli_redirect_chain_survives_streaming_parser_suspension() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    for html in [false, true] {
        let fixture = runtime.block_on(RedirectFixture::spawn_streaming(html))?;
        let path = if html {
            "/redirect/3?html"
        } else {
            "/redirect/3"
        };
        for wait in ["domcontentloaded", "load", "done"] {
            for trace in [false, true] {
                let mut args = vec![
                    "--wait-until",
                    wait,
                    "--document-start-script",
                    "fetch('/gate').then(() => fetch('/release'))",
                ];
                if trace {
                    args.push("--trace-network");
                }
                let payload = fixture.fetch(path, &args)?;
                fixture.assert_chain(&payload, path, 3);
                if trace {
                    assert_eq!(payload["network"]["main_document"]["redirect_count"], 3);
                }
                assert!(
                    fixture
                        .requests
                        .lock()
                        .iter()
                        .any(|request| request == "/release")
                );
            }
        }
    }
    Ok(())
}
