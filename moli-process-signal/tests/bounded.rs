#![cfg(unix)]

use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[test]
fn bounded_relay_notifies_cleanup_and_enforces_deadline_and_second_signal() {
    for mode in ["cleanup", "deadline", "second"] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "bounded_child", "--nocapture"])
            .env("MOLI_SIGNAL_TEST_MODE", mode)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line == "ready" {
                    let _ = ready_tx.send(());
                }
            }
        });
        if let Err(error) = ready_rx.recv_timeout(Duration::from_secs(5)) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("relay child {mode} did not start: {error}");
        }
        let started = Instant::now();
        // SAFETY: child.id identifies the live process just started above.
        assert_eq!(
            unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) },
            0
        );
        if mode == "second" {
            thread::sleep(Duration::from_millis(30));
            // SAFETY: this child stays blocked until a second signal/deadline.
            assert_eq!(
                unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
                0
            );
        }
        let deadline = started + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("relay child {mode} exceeded hard deadline");
            }
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(
            status.code(),
            Some(if mode == "second" { 130 } else { 143 }),
            "{mode}"
        );
        if mode == "deadline" {
            assert!(started.elapsed() >= Duration::from_millis(150));
        }
    }
}

#[test]
fn bounded_child() {
    let Ok(mode) = std::env::var("MOLI_SIGNAL_TEST_MODE") else {
        return;
    };
    let mut relay = moli_process_signal::BoundedTermination::install(if mode == "deadline" {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(5)
    })
    .unwrap();
    println!("ready");
    std::io::stdout().flush().unwrap();
    if mode == "cleanup" {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let notice = runtime.block_on(relay.notified()).unwrap();
        assert_eq!(notice.signal, libc::SIGTERM);
        assert!(notice.deadline > Instant::now());
        std::process::exit(128 + notice.signal);
    }
    loop {
        thread::park();
    }
}
