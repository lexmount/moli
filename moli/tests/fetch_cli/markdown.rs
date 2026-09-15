use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::process::Command;

const HTML: &str = include_str!("../../../moli-html2md/tests/fixtures/hacker-news-layout.html");
const MARKDOWN: &str = include_str!("../../../moli-html2md/tests/fixtures/hacker-news-layout.md");

fn assert_layout_table_dump(args: &[&str]) -> Result<()> {
    let url = format!("data:text/html;base64,{}", STANDARD.encode(HTML));
    let output = Command::new(env!("CARGO_BIN_EXE_moli"))
        .args([
            "fetch",
            "--dump",
            "markdown",
            "--wait-until",
            "load",
            "--timeout",
            "10000",
        ])
        .args(args)
        .arg(url)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout)?, MARKDOWN.trim_end());
    Ok(())
}

#[test]
fn hacker_news_layout_tables_dump_markdown() -> Result<()> {
    assert_layout_table_dump(&[])
}

#[test]
fn hacker_news_layout_tables_dump_markdown_with_layout() -> Result<()> {
    assert_layout_table_dump(&["--layout"])
}
