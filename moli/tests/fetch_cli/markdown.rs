use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::process::Command;

const HTML: &str = include_str!("../../../moli-html2md/tests/fixtures/hacker-news-layout.html");
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
    let markdown = String::from_utf8(output.stdout)?;
    for content in [
        "First story",
        "42 points by",
        "8 comments",
        "Second story",
        "7 points by",
        "discuss",
        "More",
    ] {
        assert!(markdown.contains(content), "missing {content}: {markdown}");
    }
    let positions = [
        "First story",
        "42 points by",
        "Second story",
        "7 points by",
        "More",
    ]
    .map(|content| markdown.find(content).expect("checked above"));
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{markdown}"
    );
    assert!(markdown.contains("<table"), "{markdown}");
    Ok(())
}

fn assert_markdown_dump(html: &str, markdown: &str, args: &[&str]) -> Result<()> {
    let url = format!("data:text/html;base64,{}", STANDARD.encode(html));
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
    assert_eq!(String::from_utf8(output.stdout)?, markdown.trim_end());
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

#[test]
fn data_tables_dump_gfm_markdown_with_and_without_layout() -> Result<()> {
    for args in [&[][..], &["--layout"][..]] {
        assert_markdown_dump(
            include_str!("../../../moli-html2md/tests/fixtures/data-table.html"),
            include_str!("../../../moli-html2md/tests/fixtures/data-table.md"),
            args,
        )?;
    }
    Ok(())
}

#[test]
fn headerless_tables_dump_gfm_markdown_with_and_without_layout() -> Result<()> {
    for args in [&[][..], &["--layout"][..]] {
        assert_markdown_dump(
            include_str!("../../../moli-html2md/tests/fixtures/headerless-data-table.html"),
            include_str!("../../../moli-html2md/tests/fixtures/headerless-data-table.md"),
            args,
        )?;
    }
    Ok(())
}
