use super::clean_output;
use anyhow::{Context, Result};
use std::{
    fmt,
    io::{self, Read},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

struct KillOnDropChild(Child);

impl Drop for KillOnDropChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Debug)]
struct ProcessTimeout {
    child_id: u32,
    timeout: Duration,
    output: Output,
}

fn output_preview(bytes: &[u8]) -> String {
    const LIMIT: usize = 4096;
    let mut preview = clean_output(&bytes[..bytes.len().min(LIMIT)]);
    if bytes.len() > LIMIT {
        preview.push_str("... (truncated)");
    }
    preview
}

impl fmt::Display for ProcessTimeout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CLI subprocess {} did not exit within {} ms; status={}; stdout={:?}; stderr={:?}",
            self.child_id,
            self.timeout.as_millis(),
            self.output.status,
            output_preview(&self.output.stdout),
            output_preview(&self.output.stderr)
        )
    }
}

impl std::error::Error for ProcessTimeout {}

fn read_process_pipe(mut pipe: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn run_command_with_process_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<Output> {
    std::thread::scope(|scope| {
        let mut child = KillOnDropChild(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?,
        );
        let child_id = child.0.id();
        let stdout = child.0.stdout.take().expect("CLI stdout must be piped");
        let stderr = child.0.stderr.take().expect("CLI stderr must be piped");
        // Read both streams while the child runs. Waiting for exit first can
        // deadlock even after layout finishes if either pipe fills up.
        let stdout = scope.spawn(move || read_process_pipe(stdout));
        let stderr = scope.spawn(move || read_process_pipe(stderr));
        let started = Instant::now();
        let timed_out = loop {
            if child.0.try_wait()?.is_some() {
                break false;
            }
            if started.elapsed() >= timeout {
                child
                    .0
                    .kill()
                    .context("failed to kill timed-out CLI child")?;
                break true;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        // Reap before joining readers, including after a timeout. The guard
        // does the same on early errors so a blocked child cannot outlive us.
        let output = Output {
            status: child.0.wait()?,
            stdout: stdout.join().expect("CLI stdout reader must not panic")?,
            stderr: stderr.join().expect("CLI stderr reader must not panic")?,
        };
        if timed_out {
            return Err(ProcessTimeout {
                child_id,
                timeout,
                output,
            }
            .into());
        }
        Ok(output)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const CHILD_MODE: &str = "MOLI_CLI_TEST_SUBPROCESS_MODE";

    #[test]
    fn timeout_kills_and_reaps_child_and_preserves_both_output_streams() -> Result<()> {
        if std::env::var(CHILD_MODE).as_deref() == Ok("hang") {
            std::io::stdout().write_all(b"stdout-before-hang\n")?;
            std::io::stdout().flush()?;
            std::io::stderr().write_all(b"stderr-before-hang\n")?;
            std::io::stderr().flush()?;
            loop {
                std::thread::park();
            }
        }
        let current = std::thread::current();
        let test_name = current
            .name()
            .context("libtest must name the test thread")?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--exact", test_name, "--nocapture"])
            .env(CHILD_MODE, "hang");
        let error = run_command_with_process_timeout(command, Duration::from_secs(2))
            .expect_err("the child deliberately never exits");
        let message = error.to_string();
        assert!(message.contains("stdout-before-hang"));
        assert!(message.contains("stderr-before-hang"));
        let timeout = error.downcast::<ProcessTimeout>()?;
        assert_eq!(timeout.timeout, Duration::from_secs(2));
        assert!(!timeout.output.status.success());
        #[cfg(target_os = "linux")]
        assert!(
            !std::path::Path::new(&format!("/proc/{}", timeout.child_id)).exists(),
            "timed-out child must be reaped"
        );
        Ok(())
    }
}
