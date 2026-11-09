#![cfg(unix)]

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

fn moli_cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_moli"))
}

#[test]
fn cli_serve_exits_promptly_when_sigterm_is_delivered() {
    assert_serve_exits_on_signal(libc::SIGTERM);
}

#[test]
fn cli_serve_exits_promptly_when_sigint_is_delivered() {
    assert_serve_exits_on_signal(libc::SIGINT);
}

#[test]
fn cli_serve_exits_promptly_when_sighup_is_delivered() {
    assert_serve_exits_on_signal(libc::SIGHUP);
}

fn assert_serve_exits_on_signal(signal: libc::c_int) {
    let mut child = Command::new(moli_cli_path())
        .args([
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--log-level",
            "info",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("moli serve should spawn");
    let child_pid = child.id();

    let stderr = child.stderr.take().expect("child stderr should be piped");
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let stderr_reader = thread::spawn(move || {
        let mut ready_sender = Some(ready_sender);
        for line in BufReader::new(stderr).lines() {
            let line = line.expect("moli stderr should be readable");
            if line.contains("protocol server listening")
                && let Some(sender) = ready_sender.take()
            {
                let _ = sender.send(());
            }
        }
    });

    if let Err(error) = ready_receiver.recv_timeout(Duration::from_secs(10)) {
        // SAFETY: child_pid identifies the child process started above.
        unsafe {
            libc::kill(child_pid as libc::pid_t, libc::SIGKILL);
        }
        let _ = child.wait();
        let _ = stderr_reader.join();
        panic!("moli serve did not become ready: {error}");
    }

    // SAFETY: child_pid identifies the live server process.
    assert_eq!(unsafe { libc::kill(child_pid as libc::pid_t, signal) }, 0);

    let (status_sender, status_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = status_sender.send(child.wait());
    });
    let status = match status_receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(status) => status.expect("moli serve wait should succeed"),
        Err(error) => {
            // SAFETY: child_pid still identifies the timed-out server. SIGKILL
            // is used only to avoid leaking a failed regression-test process.
            unsafe {
                libc::kill(child_pid as libc::pid_t, libc::SIGKILL);
            }
            panic!("moli serve did not exit promptly: {error}");
        }
    };

    stderr_reader
        .join()
        .expect("stderr reader should finish after server exit");
    assert!(status.success(), "graceful server shutdown: {status}");
}
