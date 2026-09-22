//! Real macOS trust evaluation. Run only on an ephemeral GitHub Actions runner:
//! this test installs a temporary keychain and administrator trust setting.

use std::{
    fs,
    io::{self, Read, Seek, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime},
};

use curl::easy::{Easy2, Handler, InfoType, List, WriteError};
use moli_curl::CurlTlsConfig;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer},
};

const DEADLINE: Duration = Duration::from_secs(10);
const COMMAND_DEADLINE: Duration = Duration::from_secs(15);

#[test]
#[ignore = "requires an ephemeral macOS GitHub Actions runner and modifies its trust settings"]
fn apple_sectrust() {
    assert_eq!(std::env::consts::OS, "macos");
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    if let Ok(case) = std::env::var("MOLI_SECTRUST_CASE") {
        run_case(
            &case,
            &PathBuf::from(std::env::var_os("MOLI_SECTRUST_FIXTURES").unwrap()),
        );
        return;
    }

    let fixtures = tempfile::tempdir().unwrap();
    for name in ["trusted", "untrusted"] {
        generate_certificates(fixtures.path(), name);
        create_ca_directory(fixtures.path(), name);
    }
    let _keychain = Keychain::install(fixtures.path());

    // Separate processes keep certificate environment variables isolated from
    // other tests and exercise the binding's fresh-handle initialization.
    for route in ["origin", "proxy", "wss"] {
        for policy in [
            "native",
            "untrusted",
            "hostname",
            "ca",
            "ca-untrusted",
            "env-file",
            "env-file-untrusted",
            "env-dir",
            "env-dir-untrusted",
            "ca-over-env",
            "ca-untrusted-over-env",
            "insecure",
        ] {
            let case = format!("{route}/{policy}");
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--ignored", "--exact", "apple_sectrust", "--nocapture"])
                .env("MOLI_SECTRUST_CASE", &case)
                .env("MOLI_SECTRUST_FIXTURES", fixtures.path())
                .env_remove("SSL_CERT_FILE")
                .env_remove("SSL_CERT_DIR");
            match policy {
                "env-file" | "env-file-untrusted" => {
                    let ca = if policy == "env-file" {
                        "trusted"
                    } else {
                        "untrusted"
                    };
                    command.env("SSL_CERT_FILE", fixtures.path().join(format!("{ca}.pem")));
                }
                "env-dir" | "env-dir-untrusted" => {
                    let ca = if policy == "env-dir" {
                        "trusted"
                    } else {
                        "untrusted"
                    };
                    command.env("SSL_CERT_DIR", fixtures.path().join(ca));
                }
                "ca-over-env" | "ca-untrusted-over-env" => {
                    command.env("SSL_CERT_FILE", fixtures.path().join("untrusted.pem"));
                    // If CAPATH leaks through the explicit CA override, the
                    // negative case would unexpectedly trust this certificate.
                    command.env("SSL_CERT_DIR", fixtures.path().join("trusted"));
                }
                _ => {}
            }
            let output = command_output(&mut command, COMMAND_DEADLINE).unwrap();
            assert!(
                output.status.success(),
                "{case}:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            println!("passed {case}");
        }
    }
}

fn generate_certificates(directory: &Path, name: &str) -> CertificateDer<'static> {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, format!("Moli {name} test CA"));
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let root_key = KeyPair::generate().unwrap();
    let root = params.self_signed(&root_key).unwrap();
    let root_path = directory.join(format!("{name}.pem"));
    fs::write(&root_path, root.pem()).unwrap();

    let mut params = CertificateParams::new(vec!["fixture.test".to_owned()]).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, "fixture.test");
    params.use_authority_key_identifier_extension = true;
    params.not_before = (SystemTime::now() - Duration::from_secs(86400)).into();
    params.not_after = (SystemTime::now() + Duration::from_secs(86400)).into();
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate().unwrap();
    let certificate = params.signed_by(&key, &root, &root_key).unwrap();
    fs::write(directory.join(format!("{name}.der")), certificate.der()).unwrap();
    fs::write(directory.join(format!("{name}.key")), key.serialize_der()).unwrap();
    root.der().clone()
}

fn create_ca_directory(directory: &Path, name: &str) {
    let root_path = directory.join(format!("{name}.pem"));
    let hash = command_output(
        Command::new("openssl")
            .args(["x509", "-hash", "-noout", "-in"])
            .arg(&root_path),
        COMMAND_DEADLINE,
    )
    .unwrap();
    assert!(hash.status.success());
    let ca_directory = directory.join(name);
    fs::create_dir(&ca_directory).unwrap();
    fs::copy(
        root_path,
        ca_directory.join(format!(
            "{}.0",
            String::from_utf8(hash.stdout).unwrap().trim()
        )),
    )
    .unwrap();
}

struct Keychain {
    path: PathBuf,
    root: PathBuf,
    previous: Vec<String>,
}

impl Keychain {
    fn install(directory: &Path) -> Self {
        let previous = security(&["list-keychains", "-d", "user"]);
        let keychain = Self {
            path: directory.join("test.keychain-db"),
            root: directory.join("trusted.pem"),
            previous: previous
                .lines()
                .map(|line| line.trim().trim_matches('"').to_owned())
                .collect(),
        };
        let path = keychain.path.to_str().unwrap();
        security(&["create-keychain", "-p", "moli-test", path]);
        security(&["unlock-keychain", "-p", "moli-test", path]);
        let mut search_list = vec!["list-keychains", "-d", "user", "-s"];
        search_list.extend(keychain.previous.iter().map(String::as_str));
        search_list.push(path);
        security(&search_list);
        let output = command_output(
            Command::new("sudo")
                .args([
                    "-n",
                    "security",
                    "add-trusted-cert",
                    "-d",
                    "-r",
                    "trustRoot",
                    "-p",
                    "ssl",
                    "-k",
                    path,
                ])
                .arg(&keychain.root),
            COMMAND_DEADLINE,
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        keychain
    }
}

impl Drop for Keychain {
    fn drop(&mut self) {
        cleanup(
            Command::new("sudo")
                .args(["-n", "security", "remove-trusted-cert", "-d"])
                .arg(&self.root),
        );
        cleanup(
            Command::new("security")
                .args(["list-keychains", "-d", "user", "-s"])
                .args(&self.previous),
        );
        cleanup(
            Command::new("security")
                .arg("delete-keychain")
                .arg(&self.path),
        );
    }
}

fn security(arguments: &[&str]) -> String {
    let output =
        command_output(Command::new("security").args(arguments), COMMAND_DEADLINE).unwrap();
    assert!(
        output.status.success(),
        "security {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn cleanup(command: &mut Command) {
    match command_output(command, COMMAND_DEADLINE) {
        Ok(output) if output.status.success() => {}
        Ok(output) => eprintln!(
            "cleanup {command:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => eprintln!("cleanup {command:?}: {error}"),
    }
}

fn command_output(command: &mut Command, deadline: Duration) -> io::Result<Output> {
    // Files keep verbose child output from blocking on a full pipe while the
    // parent enforces the deadline, including during panic cleanup.
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    command
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?);
    let mut child = command.spawn()?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= deadline {
            if let Err(error) = child.kill() {
                // `sudo security` runs as root; the runner user cannot kill it
                // directly. Restrict escalation to this still-owned child PID.
                if error.kind() != io::ErrorKind::PermissionDenied
                    || !Command::new("sudo")
                        .args(["-n", "/bin/kill", "-KILL", &child.id().to_string()])
                        .status()?
                        .success()
                {
                    return Err(error);
                }
            }
            child.wait()?;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{command:?} exceeded {deadline:?}"),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };
    stdout.rewind()?;
    stderr.rewind()?;
    let mut output = Output {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    stdout.read_to_end(&mut output.stdout)?;
    stderr.read_to_end(&mut output.stderr)?;
    Ok(output)
}

#[derive(Default)]
struct Response {
    body: Vec<u8>,
    debug: String,
}

impl Handler for Response {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        self.body.extend_from_slice(data);
        Ok(data.len())
    }

    fn debug(&mut self, kind: InfoType, data: &[u8]) {
        if matches!(kind, InfoType::Text) {
            self.debug.push_str(&String::from_utf8_lossy(data));
        }
    }
}

fn run_case(case: &str, directory: &Path) {
    let (route, policy) = case.split_once('/').unwrap();
    let certificate = if matches!(policy, "untrusted" | "insecure") {
        "untrusted"
    } else {
        "trusted"
    };
    let (port, server) = server(directory, certificate, route == "wss");
    let host = if policy == "hostname" {
        "wrong.test"
    } else {
        "fixture.test"
    };
    let mut easy = Easy2::new(Response::default());
    easy.timeout(DEADLINE).unwrap();
    easy.verbose(true).unwrap();
    easy.noproxy("").unwrap();
    let mut resolve = List::new();
    resolve.append(&format!("{host}:{port}:127.0.0.1")).unwrap();
    easy.resolve(resolve).unwrap();
    let mut tls = CurlTlsConfig {
        verify: policy != "insecure",
        ..Default::default()
    };
    if policy.starts_with("ca") {
        let ca = if policy.starts_with("ca-untrusted") {
            "untrusted"
        } else {
            "trusted"
        };
        tls.ca_cert = Some(directory.join(format!("{ca}.pem")));
    }
    if route == "proxy" {
        easy.url("http://target.invalid/resource").unwrap();
        easy.proxy(&format!("https://{host}:{port}")).unwrap();
        tls.configure_https_proxy(&mut easy).unwrap();
    } else {
        let scheme = if route == "wss" { "wss" } else { "https" };
        easy.url(&format!("{scheme}://{host}:{port}/resource"))
            .unwrap();
        easy.proxy("").unwrap();
        tls.configure(&mut easy, false).unwrap();
        if route == "wss" {
            easy.ws_connect_only(true).unwrap();
        }
    }
    let result = easy.perform();
    let Response { body, debug } = std::mem::take(easy.get_mut());
    drop(easy);
    let server_result = server.join().expect("TLS fixture panicked");
    if let Err(error) = &server_result {
        eprintln!("{case}: TLS fixture: {error}");
    }
    let failure = policy.contains("untrusted") || policy == "hostname";
    if failure {
        assert_eq!(result.unwrap_err().code(), 60, "{case}: {debug}");
    } else {
        result.unwrap_or_else(|error| panic!("{case}: {error}: {debug}"));
        server_result.unwrap();
        if route != "wss" {
            assert_eq!(body, b"ok");
        }
        if policy != "insecure" {
            let verifier = if policy == "native" {
                "Apple SecTrust"
            } else {
                "OpenSSL"
            };
            assert!(
                debug.contains(&format!("SSL certificate verified via {verifier}.")),
                "{case}: {debug}"
            );
        }
    }
}

fn server_config(directory: &Path, certificate: &str) -> ServerConfig {
    let certificate_der = fs::read(directory.join(format!("{certificate}.der")))
        .unwrap()
        .into();
    let key =
        PrivatePkcs8KeyDer::from(fs::read(directory.join(format!("{certificate}.key"))).unwrap());
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate_der], key.into())
        .unwrap()
}

fn server(
    directory: &Path,
    certificate: &str,
    websocket: bool,
) -> (u16, thread::JoinHandle<io::Result<()>>) {
    let config = server_config(directory, certificate);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let task = thread::spawn(move || {
        let start = Instant::now();
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && start.elapsed() < DEADLINE =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("TLS fixture accept: {error}"),
            }
        };
        serve_connection(stream, config, websocket)
    });
    (port, task)
}

fn serve_connection(stream: TcpStream, config: ServerConfig, websocket: bool) -> io::Result<()> {
    // Darwin can inherit O_NONBLOCK from the listener. StreamOwned and the
    // synchronous WebSocket handshake need blocking I/O on the accepted socket.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(DEADLINE))?;
    stream.set_write_timeout(Some(DEADLINE))?;
    let mut tls = StreamOwned::new(ServerConnection::new(Arc::new(config)).unwrap(), stream);
    if websocket {
        let mut socket = tokio_tungstenite::tungstenite::accept(tls)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let _ = socket.close(None);
        return Ok(());
    }
    let mut request = Vec::new();
    let mut byte = [0];
    while request.len() < 16 * 1024 && !request.ends_with(b"\r\n\r\n") {
        tls.read_exact(&mut byte)?;
        request.push(byte[0]);
    }
    tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
}

#[test]
fn fixture_completes_tls_on_an_initially_nonblocking_socket() {
    use rustls::{ClientConfig, ClientConnection, RootCertStore};

    let fixtures = tempfile::tempdir().unwrap();
    let root = generate_certificates(fixtures.path(), "trusted");
    let mut roots = RootCertStore::empty();
    roots.add(root).unwrap();
    let client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client.set_read_timeout(Some(DEADLINE)).unwrap();
    client.set_write_timeout(Some(DEADLINE)).unwrap();
    let (accepted, _) = listener.accept().unwrap();
    accepted.set_nonblocking(true).unwrap();
    let server_config = server_config(fixtures.path(), "trusted");
    let server = thread::spawn(move || serve_connection(accepted, server_config, false));
    // The fixture must wait for ClientHello rather than treating WouldBlock as
    // rejection and closing the connection before the client starts TLS.
    thread::sleep(Duration::from_millis(50));
    let mut tls = StreamOwned::new(
        ClientConnection::new(Arc::new(client_config), "fixture.test".try_into().unwrap()).unwrap(),
        client,
    );
    tls.write_all(b"GET / HTTP/1.1\r\nHost: fixture.test\r\n\r\n")
        .unwrap();
    let expected = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
    let mut response = vec![0; expected.len()];
    tls.read_exact(&mut response).unwrap();
    assert_eq!(response, expected);
    server.join().unwrap().unwrap();
}

#[test]
fn fixture_certificates_work_with_explicit_ca_files() {
    let fixtures = tempfile::tempdir().unwrap();
    generate_certificates(fixtures.path(), "trusted");
    generate_certificates(fixtures.path(), "untrusted");
    for route in ["origin", "wss", "proxy"] {
        for policy in ["ca", "ca-untrusted"] {
            run_case(&format!("{route}/{policy}"), fixtures.path());
        }
    }
}

#[cfg(unix)]
#[test]
fn command_deadline_terminates_a_stalled_process() {
    let error =
        command_output(Command::new("sleep").arg("30"), Duration::from_millis(50)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}
