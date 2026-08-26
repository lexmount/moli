use super::{BinaryDocumentFixtureServer, clean_output, run_moli};
use anyhow::Result;
use moli_test_support::FixtureServer;

fn run_eval(url: &str, expression: &str, extra_args: &[&str]) -> Result<super::Output> {
    let mut args = vec![
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--wait-until",
        "load",
        "--eval",
        expression,
    ];
    args.extend_from_slice(extra_args);
    args.push(url);
    run_moli(args)
}

#[test]
fn eval_uses_standard_document_apis_and_writes_text() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/static");
    let output = run_eval(
        &url,
        r#"document.querySelector("main").id = "target"; document.getElementById("target").outerHTML"#,
        &[],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "stderr={}",
        clean_output(&output.stderr)
    );
    assert_eq!(
        clean_output(&output.stdout),
        "<main id=\"target\">fixture static</main>\n"
    );
    Ok(())
}

#[test]
fn eval_writes_objects_as_compact_json() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/static");
    let output = run_eval(
        &url,
        r#"({ tag: document.querySelector("main").tagName.toLowerCase(), text: document.querySelector("main").textContent.trim() })"#,
        &[],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "stderr={}",
        clean_output(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        value,
        serde_json::json!({ "tag": "main", "text": "fixture static" })
    );
    assert!(output.stdout.ends_with(b"\n"));
    Ok(())
}

#[test]
fn eval_awaits_a_promise_result() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/static");
    let output = run_eval(
        &url,
        r#"new Promise(resolve => setTimeout(() => resolve([...document.querySelectorAll("main")].map(node => node.textContent.trim())), 10))"#,
        &[],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "stderr={}",
        clean_output(&output.stderr)
    );
    assert_eq!(clean_output(&output.stdout), "[\"fixture static\"]\n");
    Ok(())
}

#[test]
fn eval_remains_available_when_page_javascript_is_disabled() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/static");
    let output = run_eval(
        &url,
        r#"document.querySelector("main").textContent.trim()"#,
        &["--disable-js"],
    )?;
    runtime.block_on(server.shutdown());

    assert!(
        output.status.success(),
        "stderr={}",
        clean_output(&output.stderr)
    );
    assert_eq!(clean_output(&output.stdout), "fixture static\n");
    Ok(())
}

#[test]
fn eval_reports_javascript_exceptions_as_command_failures() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(FixtureServer::spawn())?;
    let url = server.url("/static");
    let output = run_eval(&url, r#"throw new Error("extraction failed")"#, &[])?;
    runtime.block_on(server.shutdown());

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(!output.status.success(), "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("JavaScript evaluation failed: Error: extraction failed"),
        "stderr={stderr}"
    );
    Ok(())
}

#[test]
fn eval_rejects_raw_non_html_documents() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let server = runtime.block_on(BinaryDocumentFixtureServer::spawn())?;
    let url = server.url("/inline.pdf");
    let output = run_eval(&url, "document.title", &[])?;
    runtime.block_on(server.shutdown());

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(!output.status.success(), "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("raw non-HTML document fetch does not support --eval"),
        "stderr={stderr}"
    );
    Ok(())
}
