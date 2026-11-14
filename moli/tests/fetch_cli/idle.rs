use super::{Output, clean_output, fixture_runtime, run_moli, unique_temp_file_path};
use anyhow::Result;
use axum::{
    Router,
    extract::ws::{Message, WebSocketUpgrade},
    response::{Html, IntoResponse},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{process::Command, time::Duration};

fn document(script: &str) -> String {
    format!(
        "data:text/html;base64,{}",
        STANDARD.encode(format!("<main>pending</main><script>{script}</script>"))
    )
}

fn run(url: &str, timeout_ms: &str, extra: &[&str]) -> Result<Output> {
    let mut args = vec![
        "moli",
        "fetch",
        "--http-no-proxy",
        "*",
        "--log-level",
        "error",
        "--wait",
        "idle",
        "--timeout-ms",
        timeout_ms,
    ];
    args.extend_from_slice(extra);
    args.push(url);
    run_moli(args)
}

fn assert_done(output: Output) {
    let stderr = clean_output(&output.stderr);
    let stdout = clean_output(&output.stdout);
    assert!(output.status.success(), "stderr={stderr}");
    assert!(stdout.contains("<main>done</main>"), "stdout={stdout}");
}

struct Server {
    _runtime: tokio::runtime::Runtime,
    task: tokio::task::JoinHandle<()>,
    url: String,
}

impl Server {
    fn new(script: &str) -> Result<Self> {
        let runtime = fixture_runtime()?;
        let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))?;
        let url = format!("http://{}/", listener.local_addr()?);
        let html = format!("<main>pending</main><script>{script}</script>");
        let app = Router::new()
            .route("/", get(move || { let html = html.clone(); async move { Html(html) } }))
            .route("/data", get(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                "payload"
            }))
            .route("/worker.js", get(|| async {
                ([("content-type", "text/javascript")],
                 "setTimeout(() => { postMessage('payload'); close(); }, 100)")
            }))
            .route("/waiting-worker.js", get(|| async {
                ([("content-type", "text/javascript")],
                 "postMessage('ready'); onmessage = () => {}")
            }))
            .route("/burst-worker.js", get(|| async {
                ([("content-type", "text/javascript")],
                 "for (let i = 0; i < 16; ++i) postMessage(i); close()")
            }))
            .route("/module.js", get(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                ([("content-type", "text/javascript")], "export default 'payload'")
            }))
            .route("/next", get(|| async { Html(
                "<main>pending</main><script>setTimeout(() => document.querySelector('main').textContent='done',100)</script>"
            ) }))
            .route("/socket", get(|upgrade: WebSocketUpgrade| async {
                upgrade.on_upgrade(|mut socket| async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if socket.send(Message::Text("payload".into())).await.is_ok() {
                        // Stay open until the client closes, so the test also
                        // exercises the connection's active lifetime.
                        while socket.recv().await.is_some() {}
                    }
                }).into_response()
            }));
        let task = runtime.spawn(async move { axum::serve(listener, app).await.unwrap() });
        Ok(Self {
            _runtime: runtime,
            task,
            url,
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[test]
fn idle_accepts_documents_without_a_javascript_realm() -> Result<()> {
    for extra in [
        vec!["--dump", "html"],
        vec!["--dump", "html", "--disable-js"],
    ] {
        assert_done(run("data:text/html,<main>done</main>", "3000", &extra)?);
    }
    Ok(())
}

#[test]
fn idle_waits_for_future_timers_and_their_microtask_followups() -> Result<()> {
    let url = document(
        r#"
        Promise.resolve().then(() => queueMicrotask(() => {
          setTimeout(() => {
            document.querySelector('main').textContent = 'first';
            Promise.resolve().then(() => setTimeout(() => {
              document.querySelector('main').textContent = 'done';
            }, 30));
          }, 650);
        }));
    "#,
    );
    assert_done(run(&url, "3000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_waits_until_an_interval_is_cleared() -> Result<()> {
    let url = document(
        r#"
        let count = 0;
        const id = setInterval(() => {
          if (++count === 3) {
            clearInterval(id);
            queueMicrotask(() => document.querySelector('main').textContent = 'done');
          }
        }, 40);
    "#,
    );
    assert_done(run(&url, "3000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_ignores_canceled_timers_internal_timeouts_and_unbacked_promises() -> Result<()> {
    let url = document(
        r#"
        clearTimeout(setTimeout(() => {}, 60000));
        clearInterval(setInterval(() => {}, 60000));
        globalThis.pending = new Promise(() => {});
        globalThis.signal = AbortSignal.timeout(60000);
        queueMicrotask(() => document.querySelector('main').textContent = 'done');
    "#,
    );
    assert_done(run(&url, "2000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_waits_for_fetch_and_xhr_callbacks_and_their_timers() -> Result<()> {
    let server = Server::new(
        r#"
        const done = [];
        function finish(value) {
          done.push(value);
          if (done.length === 2) setTimeout(() => {
            document.querySelector('main').textContent =
              done.every(x => x === 'payload') ? 'done' : 'wrong payload';
          }, 30);
        }
        fetch('/data').then(r => r.text()).then(finish);
        const xhr = new XMLHttpRequest();
        xhr.open('GET', '/data');
        xhr.onloadend = () => Promise.resolve().then(() => finish(xhr.responseText));
        xhr.send();
    "#,
    )?;
    assert_done(run(&server.url, "5000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_waits_for_indexed_db_requests_transactions_and_their_followups() -> Result<()> {
    let server = Server::new(
        r#"
        const open = indexedDB.open('idle-requests', 1);
        open.onupgradeneeded = () => open.result.createObjectStore('kv');
        open.onsuccess = () => {
          const db = open.result;
          const tx = db.transaction('kv', 'readwrite');
          const write = tx.objectStore('kv').put('payload', 1);
          let received = false;
          write.onsuccess = () => queueMicrotask(() => received = true);
          tx.oncomplete = () => {
            db.close();
            queueMicrotask(() => {
              document.querySelector('main').textContent = received ? 'done' : 'missing request';
            });
          };
        };
    "#,
    )?;
    assert_done(run(&server.url, "5000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_keeps_fetch_alive_until_an_internal_abort_timeout_fires() -> Result<()> {
    let server = Server::new(
        r#"
        fetch('/data', {signal: AbortSignal.timeout(20)}).then(
          () => document.querySelector('main').textContent = 'not aborted',
          error => setTimeout(() => {
            document.querySelector('main').textContent =
              error.name === 'TimeoutError' ? 'done' : error.name;
          }, 30)
        );
    "#,
    )?;
    assert_done(run(&server.url, "5000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_waits_for_dynamic_module_evaluation_and_its_callbacks() -> Result<()> {
    let server = Server::new(
        r#"
        import('/module.js').then(module => setTimeout(() => {
          document.querySelector('main').textContent = module.default === 'payload' ? 'done' : 'wrong payload';
        }, 30));
    "#,
    )?;
    assert_done(run(&server.url, "5000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_drains_worker_messages_before_the_worker_closes() -> Result<()> {
    let server = Server::new(
        r#"
        const worker = new Worker('/worker.js');
        worker.onmessage = event => queueMicrotask(() => {
          document.querySelector('main').textContent = event.data === 'payload' ? 'done' : 'wrong payload';
        });
    "#,
    )?;
    assert_done(run(&server.url, "5000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_stops_waiting_after_a_worker_is_terminated() -> Result<()> {
    let server = Server::new(
        r#"
        const worker = new Worker('/waiting-worker.js');
        worker.onmessage = event => {
          worker.terminate();
          queueMicrotask(() => {
            document.querySelector('main').textContent = event.data === 'ready' ? 'done' : 'wrong payload';
          });
        };
    "#,
    )?;
    assert_done(run(&server.url, "5000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_drains_all_worker_messages_after_immediate_close() -> Result<()> {
    let server = Server::new(
        r#"
        const worker = new Worker('/burst-worker.js');
        const messages = [];
        worker.onmessage = event => {
          messages.push(event.data);
          if (messages.length === 16) queueMicrotask(() => {
            document.querySelector('main').textContent =
              messages.every((value, index) => value === index) ? 'done' : 'out of order';
          });
        };
    "#,
    )?;
    assert_done(run(&server.url, "5000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_ignores_a_worker_terminated_while_its_script_is_loading() -> Result<()> {
    let server = Server::new(
        "const worker = new Worker('/waiting-worker.js'); worker.terminate(); document.querySelector('main').textContent = 'done'",
    )?;
    assert_done(run(&server.url, "3000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_waits_for_websocket_close_and_its_callback() -> Result<()> {
    let server = Server::new(
        r#"
        const socket = new WebSocket(location.origin.replace('http:', 'ws:') + '/socket');
        let received = false;
        socket.onmessage = event => { received = event.data === 'payload'; socket.close(); };
        socket.onclose = () => setTimeout(() => {
          document.querySelector('main').textContent = received ? 'done' : 'no message';
        }, 30);
    "#,
    )?;
    assert_done(run(&server.url, "5000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_drains_started_message_ports_and_observes_close() -> Result<()> {
    let url = document(
        r#"
        const channel = new MessageChannel();
        channel.port1.onmessage = event => {
          channel.port1.close();
          channel.port2.close();
          queueMicrotask(() => document.querySelector('main').textContent = event.data);
        };
        channel.port2.postMessage('done');
    "#,
    );
    assert_done(run(&url, "3000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_observes_native_message_port_listeners_without_calling_author_getters() -> Result<()> {
    let url = document(
        r#"
        const c = new MessageChannel();
        c.port1.addEventListener('message', e => {
          c.port1.close(); c.port2.close();
          document.querySelector('main').textContent = e.data;
        });
        c.port1.start();
        Object.defineProperty(c.port1, 'onmessage', {
          get() { throw new Error('must not inspect public properties'); }
        });
        c.port2.postMessage('done');
    "#,
    );
    assert_done(run(&url, "3000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_ignores_internal_stream_transfer_ports_without_backing_work() -> Result<()> {
    let url = document(
        r#"
        const stream = new ReadableStream();
        globalThis.copy = structuredClone(stream, {transfer: [stream]});
        globalThis.pending = copy.getReader().read();
        queueMicrotask(() => document.querySelector('main').textContent = 'done');
    "#,
    );
    assert_done(run(&url, "2000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_ignores_unstarted_ports_and_ports_without_receivers() -> Result<()> {
    let url = document(
        r#"
        const a = new MessageChannel();
        a.port1.addEventListener('message', () => { throw Error('not started'); });
        a.port2.postMessage('queued');
        const b = new MessageChannel();
        b.port1.start();
        b.port2.postMessage('no receiver');
        queueMicrotask(() => document.querySelector('main').textContent = 'done');
    "#,
    );
    assert_done(run(&url, "2000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_has_a_strict_timeout_for_resources_that_stay_alive() -> Result<()> {
    for script in [
        "setInterval(() => {}, 30)",
        "const c = new MessageChannel(); c.port1.onmessage = () => {}",
    ] {
        let output = run(&document(script), "800", &["--dump", "html"])?;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            clean_output(&output.stderr).contains("waiting for idle"),
            "{}",
            clean_output(&output.stderr)
        );
    }
    Ok(())
}

#[test]
fn idle_has_a_strict_timeout_for_live_workers_and_websockets() -> Result<()> {
    for script in [
        "globalThis.worker = new Worker('/waiting-worker.js')",
        "globalThis.socket = new WebSocket(location.origin.replace('http:', 'ws:') + '/socket')",
    ] {
        let server = Server::new(script)?;
        let output = run(&server.url, "1000", &["--dump", "html"])?;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            clean_output(&output.stderr).contains("waiting for idle"),
            "{}",
            clean_output(&output.stderr)
        );
    }
    Ok(())
}

#[test]
fn idle_waits_for_work_started_by_eval_before_exporting() -> Result<()> {
    let script =
        "setTimeout(() => document.querySelector('main').textContent = 'done', 100); 'value'";
    assert_done(run(
        &document(""),
        "3000",
        &["--dump", "html", "--eval", script],
    )?);
    let output = run(&document(""), "3000", &["--eval", script])?;
    assert!(output.status.success(), "{}", clean_output(&output.stderr));
    assert_eq!(clean_output(&output.stdout), "value\n");
    Ok(())
}

#[test]
fn idle_waits_for_work_started_by_eval_file() -> Result<()> {
    let path = unique_temp_file_path("wait-idle", "eval.js")?;
    std::fs::write(
        &path,
        "setTimeout(() => document.querySelector('main').textContent = 'done', 100)",
    )?;
    let output = run(
        &document(""),
        "3000",
        &["--dump", "html", "--eval-file", path.to_str().unwrap()],
    );
    std::fs::remove_file(path)?;
    assert_done(output?);
    Ok(())
}

#[test]
fn idle_also_drains_work_created_by_the_readiness_script() -> Result<()> {
    assert_done(run(
        &document(""),
        "3000",
        &[
            "--dump",
            "html",
            "--wait-script",
            "(setTimeout(() => document.querySelector('main').textContent='done', 100), true)",
        ],
    )?);
    Ok(())
}

#[test]
fn idle_waits_for_child_window_timers_and_releases_removed_windows() -> Result<()> {
    let url = document(
        r#"
        const frame = document.createElement('iframe');
        document.body.append(frame);
        const child = frame.contentWindow;
        child.setInterval(() => {}, 60000);
        child.setTimeout(() => {
          frame.remove();
          queueMicrotask(() => document.querySelector('main').textContent = 'done');
        }, 100);
    "#,
    );
    assert_done(run(&url, "3000", &["--dump", "html"])?);
    Ok(())
}

#[test]
fn idle_keeps_one_deadline_for_work_started_after_initial_readiness() -> Result<()> {
    let output = run(
        &document("setTimeout(() => {}, 300)"),
        "1000",
        &["--dump", "html", "--eval", "setTimeout(() => {}, 800)"],
    )?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        clean_output(&output.stderr).contains("timed out after 1000 ms while waiting for idle"),
        "{}",
        clean_output(&output.stderr)
    );
    Ok(())
}

fn assert_idle_execution_times_out(extra: &[&str], phase: &str) -> Result<()> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_moli"));
    command
        .args([
            "fetch",
            "--wait",
            "idle",
            "--timeout-ms",
            "1000",
            "--http-no-proxy",
            "*",
            "--log-level",
            "error",
        ])
        .args(extra)
        .arg(document(""));
    // Also bound teardown: dropping only the caller future would leave an
    // awaited Promise in the renderer and hang browser shutdown.
    let output =
        super::subprocess::run_command_with_process_timeout(command, Duration::from_secs(5))?;
    let stderr = clean_output(&output.stderr);
    assert!(
        !output.status.success(),
        "stdout={}",
        clean_output(&output.stdout)
    );
    assert!(output.stdout.is_empty());
    assert!(
        stderr.contains(&format!("timed out after 1000 ms while {phase}")),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn idle_deadline_interrupts_async_eval_and_acknowledges_page_cleanup() -> Result<()> {
    for expression in [
        "new Promise(resolve => setTimeout(() => resolve('late'), 10000))",
        "new Promise(() => {})",
    ] {
        assert_idle_execution_times_out(&["--eval", expression], "evaluating JavaScript")?;
        assert_idle_execution_times_out(
            &["--dump", "html", "--eval", expression],
            "evaluating JavaScript",
        )?;
    }
    Ok(())
}

#[test]
fn idle_deadline_interrupts_eval_file() -> Result<()> {
    let path = unique_temp_file_path("idle-eval-timeout", "eval.js")?;
    std::fs::write(&path, "new Promise(resolve => setTimeout(resolve, 10000))")?;
    let result = assert_idle_execution_times_out(
        &["--dump", "html", "--eval-file", path.to_str().unwrap()],
        "evaluating JavaScript",
    );
    std::fs::remove_file(path)?;
    result
}

#[test]
fn idle_deadline_interrupts_page_delay() -> Result<()> {
    assert_idle_execution_times_out(
        &["--dump", "html", "--delay-ms", "10000"],
        "waiting for page delay",
    )
}

#[test]
fn idle_waits_for_async_eval_and_followup_work_that_fit_the_deadline() -> Result<()> {
    assert_done(run(
        &document(""),
        "3000",
        &[
            "--dump",
            "html",
            "--eval",
            "new Promise(resolve => setTimeout(() => { setTimeout(() => document.querySelector('main').textContent = 'done', 100); resolve('value'); }, 100))",
        ],
    )?);
    Ok(())
}

#[test]
fn idle_follows_navigation_and_keeps_waiting_on_the_replacement_document() -> Result<()> {
    let server = Server::new("setTimeout(() => location.href = '/next', 40)")?;
    assert_done(run(&server.url, "5000", &["--dump", "html"])?);
    Ok(())
}
