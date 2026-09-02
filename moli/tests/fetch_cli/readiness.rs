use super::clean_output;
use anyhow::Result;
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn run_script_stdin(
    url: &str,
    script: &[u8],
    source_args: &[&str],
) -> Result<std::process::Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_moli"));
    command
        .args([
            "fetch",
            "--log-level",
            "error",
            "--http-no-proxy",
            "*",
            "--wait-until",
            "load",
        ])
        .args(source_args)
        .arg(url);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped stdin must be available")
        .write_all(script)?;
    Ok(child.wait_with_output()?)
}

#[test]
fn wait_script_file_dash_reads_predicate_from_stdin() -> Result<()> {
    let output = run_script_stdin(
        "data:text/html,<main>pending</main><script>setTimeout(()=>document.querySelector('main').textContent='ready',20)</script>",
        b"() => document.querySelector('main').textContent === 'ready'",
        &["--wait-script-file", "-", "--dump", "html"],
    )?;

    assert!(
        output.status.success(),
        "stderr={}",
        clean_output(&output.stderr)
    );
    assert!(
        clean_output(&output.stdout).contains("<main>ready</main>"),
        "stdout={}",
        clean_output(&output.stdout)
    );
    Ok(())
}

#[test]
fn stdin_cannot_supply_both_eval_and_wait_scripts() -> Result<()> {
    let output = run_script_stdin(
        "https://stdin-script-conflict-must-not-fetch.invalid/",
        b"() => true",
        &["--eval-file", "-", "--wait-script-file", "-"],
    )?;

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(!output.status.success(), "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("`--eval-file -` and `--wait-script-file -` cannot both read from stdin"),
        "stderr={stderr}"
    );
    assert!(
        !stderr.contains("failed to fetch"),
        "the ambiguous stdin sources must be rejected before fetching: stderr={stderr}"
    );
    Ok(())
}

#[test]
fn wait_script_file_dash_rejects_invalid_utf8_before_fetching() -> Result<()> {
    let output = run_script_stdin(
        "https://wait-script-file-stdin-must-not-fetch.invalid/",
        b"() => true\xff",
        &["--wait-script-file", "-", "--dump", "html"],
    )?;

    let stdout = clean_output(&output.stdout);
    let stderr = clean_output(&output.stderr);
    assert!(!output.status.success(), "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.is_empty(), "stdout={stdout}");
    assert!(
        stderr.contains("failed to read --wait-script-file `-` from stdin"),
        "stderr={stderr}"
    );
    assert!(
        !stderr.contains("failed to fetch"),
        "stdin must be read before starting a fetch: stderr={stderr}"
    );
    Ok(())
}
