use std::{
    io::{BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use clap::Parser;
use moli::{app, cli::Cli, config::AppConfig};
use serde_json::Value;

const CHILD_MODE: &str = "MOLI_TEST_PIPE_LIBRARY_OWNERSHIP";
const TIMEOUT: Duration = Duration::from_secs(15);

struct ServingChild(Child);

impl ServingChild {
    fn wait(&mut self) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.0.try_wait().expect("read child exit status") {
                return status;
            }
            assert!(started.elapsed() < TIMEOUT, "pipe child did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for ServingChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn duplicate_for_child(fd: BorrowedFd<'_>) -> OwnedFd {
    // SAFETY: `fd` remains open while fcntl creates a new descriptor. Keeping
    // both sources above 4 prevents either dup2 from overwriting the other.
    let raw = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 5) };
    assert!(
        raw >= 5,
        "duplicate child pipe: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: fcntl returned a fresh descriptor with no existing Rust owner.
    unsafe { OwnedFd::from_raw_fd(raw) }
}

fn spawn_with_pipe(mut command: Command, input: UnixStream, output: UnixStream) -> ServingChild {
    let input_source = duplicate_for_child(input.as_fd());
    let output_source = duplicate_for_child(output.as_fd());
    // SAFETY: the child callback only performs async-signal-safe dup2 calls.
    // The owned sources stay open through spawn and cannot alias FD 3 or 4.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(input_source.as_raw_fd(), 3) < 0
                || libc::dup2(output_source.as_raw_fd(), 4) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    ServingChild(command.spawn().expect("spawn pipe child"))
}

fn read_reply(output: &mut BufReader<UnixStream>, id: u64) -> Value {
    loop {
        let mut frame = Vec::new();
        output.read_until(0, &mut frame).expect("read CDP reply");
        assert_eq!(frame.pop(), Some(0), "CDP reply must be terminated");
        let message: Value = serde_json::from_slice(&frame).expect("CDP reply JSON");
        if message["id"] == id {
            assert!(message.get("error").is_none(), "{message}");
            return message;
        }
    }
}

#[test]
fn executable_hands_inherited_pipes_to_the_server() {
    let (mut input, server_input) = UnixStream::pair().unwrap();
    let (server_output, output) = UnixStream::pair().unwrap();
    output.set_read_timeout(Some(TIMEOUT)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_moli"));
    command
        .args(["serve", "--remote-debugging-pipe"])
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    let mut child = spawn_with_pipe(command, server_input, server_output);
    let mut output = BufReader::new(output);

    input.write_all(b"{\"id\":1,\"method\":\"Browser.").unwrap();
    input.write_all(b"getVersion\"}\0").unwrap();
    assert!(read_reply(&mut output, 1)["result"]["product"].is_string());
    input
        .write_all(b"{\"id\":2,\"method\":\"Browser.close\"}\0")
        .unwrap();
    assert_eq!(read_reply(&mut output, 2)["result"], serde_json::json!({}));
    // Keep the input open: successful startup and ownership transfer must also
    // preserve Browser.close shutdown, without relying on input EOF.
    assert!(child.wait().success());
}

#[test]
fn library_commands_leave_caller_owned_descriptors_untouched() {
    let (mut input_peer, server_input) = UnixStream::pair().unwrap();
    let (server_output, mut output_peer) = UnixStream::pair().unwrap();
    input_peer.set_read_timeout(Some(TIMEOUT)).unwrap();
    output_peer.set_read_timeout(Some(TIMEOUT)).unwrap();
    // The old implementation must reach EOF rather than waiting forever if
    // it incorrectly starts serving on descriptors already owned by the caller.
    input_peer.shutdown(Shutdown::Write).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "remote_debugging_pipe::library_pipe_child",
            "--nocapture",
        ])
        .env(CHILD_MODE, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    let mut child = spawn_with_pipe(command, server_input, server_output);
    assert!(child.wait().success(), "library ownership probe failed");
    for peer in [&mut input_peer, &mut output_peer] {
        let mut bytes = [0; 12];
        peer.read_exact(&mut bytes)
            .expect("caller endpoint is still usable");
        assert_eq!(&bytes, b"caller-owned");
    }
}

#[test]
fn library_pipe_child() {
    if std::env::var(CHILD_MODE).as_deref() != Ok("1") {
        return;
    }
    // SAFETY: only the parent probe enables this mode, and it transfers these
    // two socket endpoints into the child without any other Rust owner.
    let (mut input, mut output) =
        unsafe { (UnixStream::from_raw_fd(3), UnixStream::from_raw_fd(4)) };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for with_config in [true, false, true, false] {
        let cli = Cli::try_parse_from(["moli", "serve", "--remote-debugging-pipe"]).unwrap();
        let result = if with_config {
            let config = AppConfig::from_cli(&cli).unwrap();
            runtime.block_on(app::run_cli_with_config(cli, config, &mut Vec::new()))
        } else {
            runtime.block_on(app::run_cli(&mut Vec::new(), cli))
        };
        for fd in [input.as_raw_fd(), output.as_raw_fd()] {
            // SAFETY: F_GETFD only queries the descriptor. If a regression
            // closed it, exit without dropping the caller's now-invalid owner.
            if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
                eprintln!("library command closed caller-owned descriptor {fd}");
                std::process::exit(1);
            }
        }
        assert!(
            result.is_err(),
            "library command must require owned pipe inputs"
        );
    }
    input.write_all(b"caller-owned").unwrap();
    output.write_all(b"caller-owned").unwrap();
}
